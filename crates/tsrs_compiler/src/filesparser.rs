use indexmap::IndexMap;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{self as ast, Node, SourceFile, SourceFileMetaData, SourceFileParseOptions};
use tsrs_core::tspath::{self, Path};
use tsrs_core::{ModuleKind, Tristate, P};
use tsrs_diagnostics as diagnostics;
use tsrs_module::{DiagAndArgs, ModeAwareCache, PackageId, ResolvedModule, ResolvedTypeReferenceDirective};
use tsrs_tsoptions as tsoptions;

use crate::file_include::{fileIncludeKind, FileIncludeReason};
use crate::fileloader::{
    fileLoader, jsxRuntimeImportSpecifier, parse_options_for, prefetch_resolutions, prefetchedResolutions, processedFiles, redirectsFile,
    source_file_meta_data, DuplicateSourceFile, LibFile,
};
use crate::includeprocessor::fileIncludeData;
use crate::processing_diagnostic::{includeExplainingDiagnostic, processingDiagnostic};

pub(crate) type TaskId = usize;
type DataId = usize;

pub(crate) struct parseTask {
    // Shared (like `path`) by the tasks of every reference to the same file name (`add_sub_task_normalized`).
    pub(crate) normalized_file_path: std::sync::Arc<str>,
    pub(crate) path: Path,
    pub(crate) file: Option<P<SourceFile>>,
    pub(crate) lib_file: Option<P<LibFile>>,
    pub(crate) redirected_parse_task: Option<TaskId>,
    pub(crate) sub_tasks: Vec<TaskId>,
    pub(crate) loaded: bool,
    pub(crate) started_sub_tasks: bool,
    pub(crate) is_for_automatic_type_directive: bool,
    pub(crate) failed_lookup: bool,
    // Set when the metadata was computed ahead of time for a parallel parse.
    pub(crate) metadata_loaded: bool,
    pub(crate) increase_depth: bool,
    pub(crate) elide_on_depth: bool,
    pub(crate) include_reason: Option<P<FileIncludeReason>>,
    pub(crate) package_id: PackageId,
    pub(crate) loaded_task: Option<TaskId>,
    // Everything only a loaded (or metadata-prefetched) task fills, allocated on the first write: most tasks are
    // one per import edge and only point at the task that loads their file (`loaded_task`, filesParser::start).
    data: Option<Box<parseTaskLoaded>>,
}

#[derive(Default)]
pub(crate) struct parseTaskLoaded {
    pub(crate) metadata: SourceFileMetaData,
    pub(crate) prefetched_resolutions: Option<Box<prefetchedResolutions>>,
    pub(crate) resolutions_in_file: ModeAwareCache<P<ResolvedModule>>,
    pub(crate) resolutions_trace: Vec<DiagAndArgs>,
    pub(crate) type_resolutions_in_file: ModeAwareCache<P<ResolvedTypeReferenceDirective>>,
    pub(crate) type_resolutions_trace: Vec<DiagAndArgs>,
    pub(crate) processing_diagnostics: Vec<processingDiagnostic>,
    pub(crate) import_helpers_import_specifier: Option<P<Node>>,
    pub(crate) jsx_runtime_import_specifier: Option<P<jsxRuntimeImportSpecifier>>,
    // The task's path as the `FileIncludeReason`s of its references store it: one copy per file, not per reference.
    pub(crate) reason_path: Option<&'static str>,
}

impl parseTask {
    pub(crate) fn new(normalized_file_path: impl Into<std::sync::Arc<str>>) -> parseTask {
        parseTask {
            normalized_file_path: normalized_file_path.into(),
            path: Path::default(),
            file: None,
            lib_file: None,
            redirected_parse_task: None,
            sub_tasks: Vec::new(),
            loaded: false,
            started_sub_tasks: false,
            is_for_automatic_type_directive: false,
            failed_lookup: false,
            metadata_loaded: false,
            increase_depth: false,
            elide_on_depth: false,
            include_reason: None,
            package_id: PackageId::default(),
            loaded_task: None,
            data: None,
        }
    }

    /// The loaded-task data, created on first use.
    pub(crate) fn data(&mut self) -> &mut parseTaskLoaded {
        self.data.get_or_insert_with(Default::default)
    }

    pub(crate) fn data_ref(&self) -> Option<&parseTaskLoaded> {
        self.data.as_deref()
    }

    /// The file's metadata (the default before `load_metadata` or the prefetch set it).
    pub(crate) fn metadata(&self) -> SourceFileMetaData {
        self.data_ref().map(|d| d.metadata.clone()).unwrap_or_default()
    }

    pub(crate) fn take_processing_diagnostics(&mut self) -> Vec<processingDiagnostic> {
        self.data.as_mut().map(|d| std::mem::take(&mut d.processing_diagnostics)).unwrap_or_default()
    }

    /// `self.path` as an arena string shared by every include reason this task creates.
    pub(crate) fn reason_path(&mut self) -> &'static str {
        let path = &self.path;
        let data = self.data.get_or_insert_with(Default::default);
        *data.reason_path.get_or_insert_with(|| tsrs_core::alloc_str(path.as_str()))
    }
}

// Whether a task's file would go through the parser when loaded (as opposed to being skipped
// because of an unsupported extension, a failed lookup, or being the automatic type directive task).
fn task_needs_parse(loader: &fileLoader, t: TaskId) -> bool {
    let task = &loader.tasks[t];
    if task.loaded || task.is_for_automatic_type_directive || task.failed_lookup || task.file.is_some() {
        return false;
    }
    if !loader.project_references.get_parse_file_redirect(&task.normalized_file_path, &task.path).is_empty() {
        return false;
    }
    if tspath::has_extension(&task.normalized_file_path) {
        let allow_non_ts_extensions = loader.opts.config.compiler_options().unwrap().allow_non_ts_extensions.is_true();
        if !allow_non_ts_extensions {
            let canonical_file_name =
                tspath::get_canonical_file_name(&task.normalized_file_path, loader.host.fs().use_case_sensitive_file_names());
            if !loader.is_supported_extension(&canonical_file_name) {
                return false;
            }
        }
    }
    true
}

fn load(t: TaskId, loader: &mut fileLoader) {
    loader.tasks[t].loaded = true;
    if loader.tasks[t].is_for_automatic_type_directive {
        load_automatic_type_directives(t, loader);
        return;
    }
    if loader.tasks[t].failed_lookup {
        // The root file name did not resolve to a supported extension; the task
        // exists only to carry its processing diagnostic, so nothing is parsed.
        return;
    }
    let redirect = {
        let task = &loader.tasks[t];
        loader.project_references.get_parse_file_redirect(&task.normalized_file_path, &task.path)
    };
    if !redirect.is_empty() {
        redirect_task(t, loader, &redirect);
        return;
    }

    let normalized_file_path = loader.tasks[t].normalized_file_path.to_string();
    if tspath::has_extension(&normalized_file_path) {
        let compiler_options = loader.opts.config.compiler_options().unwrap();
        let allow_non_ts_extensions = compiler_options.allow_non_ts_extensions.is_true();
        if !allow_non_ts_extensions {
            let canonical_file_name =
                tspath::get_canonical_file_name(&normalized_file_path, loader.host.fs().use_case_sensitive_file_names());
            if !loader.is_supported_extension(&canonical_file_name) {
                let include_reason = loader.tasks[t].include_reason;
                if tspath::has_js_file_extension(&canonical_file_name) {
                    loader.tasks[t].data().processing_diagnostics.push(processingDiagnostic::explaining(includeExplainingDiagnostic {
                        file: None,
                        diagnostic_reason: include_reason,
                        message: &diagnostics::File_0_is_a_JavaScript_file_Did_you_mean_to_enable_the_allowJs_option,
                        args: vec![normalized_file_path.to_string()],
                    }));
                } else {
                    let flat: Vec<&str> = loader.supported_extensions.iter().flatten().map(|s| s.as_str()).collect();
                    loader.tasks[t].data().processing_diagnostics.push(processingDiagnostic::explaining(includeExplainingDiagnostic {
                        file: None,
                        diagnostic_reason: include_reason,
                        message: &diagnostics::File_0_has_an_unsupported_extension_The_only_supported_extensions_are_1,
                        args: vec![normalized_file_path.to_string(), format!("'{}'", flat.join("', '"))],
                    }));
                }
                return;
            }
        }
    }

    loader.total_file_count += 1;
    load_metadata(t, loader);

    let mut file = loader.tasks[t].file;
    if file.is_none() {
        file = loader.parse_source_file(t);
    }
    let Some(file) = file else {
        return;
    };

    loader.tasks[t].file = Some(file);
    loader.tasks[t].sub_tasks =
        Vec::with_capacity(file.referenced_files.get().len() + file.imports().len() + file.module_augmentations.get().len());

    let compiler_options = loader.opts.config.compiler_options().unwrap();
    if !compiler_options.no_resolve.is_true() && !loader.opts.skip_module_resolution {
        let mut prefetched = loader.tasks[t].data().prefetched_resolutions.as_mut().map(|p| std::mem::take(&mut p.referenced_files).into_iter());
        for (index, ref_) in file.referenced_files.get().iter().enumerate() {
            let prefetched_lookup = prefetched.as_mut().map(|p| p.next().expect("prefetched triple-slash reference lookup"));
            match loader.resolve_tripleslash_path_reference(&ref_.file_name, file.file_name(), index, prefetched_lookup) {
                Err(processing_diagnostic) => {
                    loader.tasks[t].data().processing_diagnostics.push(processing_diagnostic);
                    continue;
                }
                Ok(resolved_ref) => loader.add_sub_task(t, resolved_ref, None),
            }
        }

        loader.resolve_type_reference_directives(t);
    }

    if compiler_options.no_lib != Tristate::True && !loader.opts.skip_module_resolution {
        for (index, lib) in file.lib_reference_directives.get().iter().enumerate() {
            let include_reason = FileIncludeReason::new_referenced(
                fileIncludeKind::LibReferenceDirective,
                loader.tasks[t].reason_path(),
                index as i32,
                None,
            );
            if let Some(name) = tsoptions::get_lib_file_name(&lib.file_name) {
                let lib_file = loader.path_for_lib_file(&name);
                loader.add_sub_task(
                    t,
                    resolvedRef {
                        file_name: lib_file.path.clone(),
                        increase_depth: false,
                        elide_on_depth: false,
                        include_reason,
                        package_id: PackageId::default(),
                    },
                    Some(lib_file),
                );
            } else {
                loader.tasks[t].data().processing_diagnostics.push(processingDiagnostic::unknown_reference(include_reason));
            }
        }
    }

    loader.resolve_imports_and_module_augmentations(t);
}

fn load_metadata(t: TaskId, loader: &mut fileLoader) {
    if loader.tasks[t].metadata_loaded {
        return;
    }
    loader.tasks[t].metadata_loaded = true;
    if loader.tasks[t].lib_file.is_some() {
        loader.lib_file_count += 1;
        // Default lib files are all scripts; we can safely skip looking up their package.json
        // to avoid adding spurious lookups to file watcher tracking.
        loader.tasks[t].data().metadata = SourceFileMetaData { implied_node_format: ModuleKind::CommonJS, ..Default::default() };
    } else {
        let metadata = loader.load_source_file_meta_data(&loader.tasks[t].normalized_file_path);
        loader.tasks[t].data().metadata = metadata;
    }
}

fn redirect_task(t: TaskId, loader: &mut fileLoader, file_name: &str) {
    let mut redirected = parseTask::new(tspath::normalize_path(file_name));
    redirected.lib_file = loader.tasks[t].lib_file;
    redirected.include_reason = loader.tasks[t].include_reason;
    loader.tasks.push(redirected);
    let id = loader.tasks.len() - 1;
    loader.tasks[t].redirected_parse_task = Some(id);
    // increaseDepth and elideOnDepth are not copied to redirects, otherwise their depth would be double counted.
    loader.tasks[t].sub_tasks = vec![id];
}

fn load_automatic_type_directives(t: TaskId, loader: &mut fileLoader) {
    let normalized_file_path = loader.tasks[t].normalized_file_path.to_string();
    let (to_parse_type_refs, type_resolutions_in_file, type_resolutions_trace, p_diagnostics) =
        loader.resolve_automatic_type_directives(&normalized_file_path);
    let data = loader.tasks[t].data();
    data.type_resolutions_in_file = type_resolutions_in_file;
    data.type_resolutions_trace = type_resolutions_trace;
    data.processing_diagnostics.extend(p_diagnostics);
    for type_resolution in to_parse_type_refs {
        loader.add_sub_task(t, type_resolution, None);
    }
}

pub(crate) struct resolvedRef {
    pub(crate) file_name: String,
    pub(crate) increase_depth: bool,
    pub(crate) elide_on_depth: bool,
    pub(crate) include_reason: P<FileIncludeReason>,
    pub(crate) package_id: PackageId,
}

pub(crate) struct parseTaskData {
    // map of tasks by file casing
    pub(crate) tasks: IndexMap<std::sync::Arc<str>, TaskId>,
    lowest_depth: i32,
    started_sub_tasks: bool,
    package_id: PackageId,
}

struct queuedTask {
    task: TaskId,
    loaded: bool,
    data: DataId,
    depth: i32,
}

// Go queues one closure per task on a work group and runs them concurrently. Here the queue is
// drained in rounds: every file that a round will load is first parsed in parallel, then the
// round's closures run sequentially. The resulting task graph (and therefore the file order
// computed by getProcessedFiles) is the same fixed point Go reaches.
pub(crate) struct filesParser {
    pub(crate) task_data_by_path: FxHashMap<Path, DataId>,
    pub(crate) datas: Vec<parseTaskData>,
    queue: Vec<queuedTask>,
    max_depth: i32,
    single_threaded: bool,
}

impl filesParser {
    pub(crate) fn new(single_threaded: bool, max_depth: i32) -> filesParser {
        filesParser { task_data_by_path: FxHashMap::default(), datas: Vec::new(), queue: Vec::new(), max_depth, single_threaded }
    }

    pub(crate) fn parse(loader: &mut fileLoader, tasks: &[TaskId]) {
        Self::start(loader, tasks, 0);
        loop {
            let round = std::mem::take(&mut loader.files_parser.queue);
            if round.is_empty() {
                break;
            }
            tsrs_core::phases::count("Program:   loader rounds", 1);
            Self::prefetch(loader, &round);
            let start = std::time::Instant::now();
            for item in round {
                Self::run_queued(loader, item);
            }
            tsrs_core::phases::record("Program:   sequential load", start.elapsed());
        }
    }

    fn start(loader: &mut fileLoader, tasks: &[TaskId], depth: i32) {
        for &task in tasks {
            // A sub task created from a prefetched resolution already carries this path.
            if loader.tasks[task].path.as_str().is_empty() {
                loader.tasks[task].path = loader.to_path(&loader.tasks[task].normalized_file_path);
            }
            let path = loader.tasks[task].path.clone();
            let w = &mut loader.files_parser;
            let (data, loaded) = match w.task_data_by_path.get(&path) {
                Some(&data) => (data, true),
                None => {
                    let mut tasks = IndexMap::new();
                    tasks.insert(loader.tasks[task].normalized_file_path.clone(), task);
                    w.datas.push(parseTaskData { tasks, lowest_depth: i32::MAX, started_sub_tasks: false, package_id: PackageId::default() });
                    let id = w.datas.len() - 1;
                    w.task_data_by_path.insert(path, id);
                    (id, false)
                }
            };
            w.queue.push(queuedTask { task, loaded, data, depth });
        }
    }

    fn run_queued(loader: &mut fileLoader, item: queuedTask) {
        let queuedTask { task, loaded, data, depth } = item;
        let mut start_subtasks = false;
        if loaded {
            let casing = loader.tasks[task].normalized_file_path.clone();
            if let Some(&existing_task) = loader.files_parser.datas[data].tasks.get(&casing) {
                loader.tasks[task].loaded_task = Some(existing_task);
            } else {
                loader.files_parser.datas[data].tasks.insert(casing, task);
                // This is new task for file name - so load subtasks if there was loading for any other casing
                start_subtasks = loader.files_parser.datas[data].started_sub_tasks;
            }
        }

        // Propagate packageId to data if we have one and data doesn't yet
        if loader.files_parser.datas[data].package_id.name.is_empty() && !loader.tasks[task].package_id.name.is_empty() {
            loader.files_parser.datas[data].package_id = loader.tasks[task].package_id.clone();
        }

        let current_depth = if loader.tasks[task].increase_depth { depth + 1 } else { depth };
        if current_depth < loader.files_parser.datas[data].lowest_depth {
            // If we're seeing this task at a lower depth than before,
            // reprocess its subtasks to ensure they are loaded.
            loader.files_parser.datas[data].lowest_depth = current_depth;
            start_subtasks = true;
            loader.files_parser.datas[data].started_sub_tasks = true;
        }

        if loader.tasks[task].elide_on_depth && current_depth > loader.files_parser.max_depth {
            return;
        }

        let tasks_by_file_name: Vec<TaskId> = loader.files_parser.datas[data].tasks.values().copied().collect();
        for task_by_file_name in tasks_by_file_name {
            let mut load_sub_tasks = start_subtasks;
            if !loader.tasks[task_by_file_name].loaded {
                load(task_by_file_name, loader);
                if loader.tasks[task_by_file_name].redirected_parse_task.is_some() {
                    // Always load redirected task
                    load_sub_tasks = true;
                    loader.files_parser.datas[data].started_sub_tasks = true;
                }
            }
            if !loader.tasks[task_by_file_name].started_sub_tasks && load_sub_tasks {
                loader.tasks[task_by_file_name].started_sub_tasks = true;
                let sub_tasks = loader.tasks[task_by_file_name].sub_tasks.clone();
                let lowest_depth = loader.files_parser.datas[data].lowest_depth;
                Self::start(loader, &sub_tasks, lowest_depth);
            }
        }
    }

    // Parse (in parallel) every file that the given round will load. This only warms `task.file`;
    // `load` falls back to parsing on the spot for anything not prefetched.
    fn prefetch(loader: &mut fileLoader, round: &[queuedTask]) {
        let mut planned: FxHashSet<(DataId, std::sync::Arc<str>)> = FxHashSet::default();
        let mut to_parse: Vec<TaskId> = Vec::new();
        for item in round {
            let task = &loader.tasks[item.task];
            let current_depth = if task.increase_depth { item.depth + 1 } else { item.depth };
            if task.elide_on_depth && current_depth > loader.files_parser.max_depth {
                continue;
            }
            let data = &loader.files_parser.datas[item.data];
            let mut candidates: Vec<TaskId> = data.tasks.values().copied().collect();
            if !data.tasks.contains_key(&task.normalized_file_path) {
                candidates.push(item.task);
            }
            for candidate in candidates {
                let key = (item.data, loader.tasks[candidate].normalized_file_path.clone());
                if planned.contains(&key) || !task_needs_parse(loader, candidate) {
                    continue;
                }
                planned.insert(key);
                to_parse.push(candidate);
            }
        }
        if to_parse.len() < 2 || loader.files_parser.single_threaded {
            return;
        }
        // Each job computes the file's metadata, parses it and, unless resolution traces are requested, resolves
        // its imports and type reference directives (Go does all of this per task in parallel). Traces stay
        // sequential: they say whether a lookup was served from a cache, which depends on the resolution order.
        let resolve_ahead = !loader.opts.config.compiler_options().unwrap().trace_resolution.is_true();
        let jobs: Vec<(TaskId, String, Path, bool)> = to_parse
            .into_iter()
            .map(|t| (t, loader.tasks[t].normalized_file_path.to_string(), loader.tasks[t].path.clone(), loader.tasks[t].lib_file.is_some()))
            .collect();
        let ctx = loader.prefetch_context();
        let (opts, host, resolver, project_references) = (ctx.opts, ctx.host, ctx.resolver, ctx.project_references);
        let parse_start = std::time::Instant::now();
        let prefetched: Vec<(TaskId, SourceFileMetaData, Option<P<SourceFile>>, Option<Box<prefetchedResolutions>>)> =
            crate::program::worker_pool().install(|| {
                jobs.into_par_iter()
                    .map(|(t, file_name, path, is_lib)| {
                        let metadata = if is_lib {
                            SourceFileMetaData { implied_node_format: ModuleKind::CommonJS, ..Default::default() }
                        } else {
                            source_file_meta_data(opts, resolver, project_references, &file_name)
                        };
                        let file = host.get_source_file(parse_options_for(host, project_references, &file_name, &path, &metadata));
                        let resolutions = match file {
                            Some(file) if resolve_ahead => {
                                Some(Box::new(prefetch_resolutions(&ctx, file, &metadata)))
                            }
                            _ => None,
                        };
                        (t, metadata, file, resolutions)
                    })
                    .collect()
            });
        tsrs_core::phases::record("Program:   parallel parse + resolve", parse_start.elapsed());
        for (t, metadata, file, resolutions) in prefetched {
            let task = &mut loader.tasks[t];
            task.data().metadata = metadata;
            // Lib files keep metadata_loaded unset so that load_metadata still counts them.
            task.metadata_loaded = task.lib_file.is_none();
            // A missing file stays None; load() asks the host again and records it as missing.
            task.file = file;
            if resolutions.is_some() {
                task.data().prefetched_resolutions = resolutions;
            }
        }
    }

    pub(crate) fn get_processed_files(loader: &mut fileLoader) -> processedFiles {
        let total_file_count = loader.total_file_count as usize;
        let lib_file_count = loader.lib_file_count as usize;

        let mut missing_files: Vec<String> = Vec::new();
        let mut duplicate_source_files: Vec<DuplicateSourceFile> = Vec::new();
        let mut files: Vec<P<SourceFile>> = Vec::with_capacity(total_file_count.saturating_sub(lib_file_count));
        let mut lib_files: Vec<P<SourceFile>> = Vec::with_capacity(total_file_count);

        let mut files_by_path: FxHashMap<Path, P<SourceFile>> = FxHashMap::default();
        // stores 'filename -> file association' ignoring case
        // used to track cases when two file names differ only in casing
        let mut tasks_seen_by_name_ignore_case: Option<FxHashMap<String, TaskId>> =
            if loader.compare_paths_options.use_case_sensitive_file_names { Some(FxHashMap::default()) } else { None };

        let mut include_data = fileIncludeData::default();
        let can_use_project_reference_source = loader.opts.can_use_project_reference_source();
        let mut output_file_to_project_reference_source: FxHashMap<Path, String> = FxHashMap::default();
        let mut resolved_modules: FxHashMap<Path, ModeAwareCache<P<ResolvedModule>>> = FxHashMap::default();
        let mut type_resolutions_in_file: FxHashMap<Path, ModeAwareCache<P<ResolvedTypeReferenceDirective>>> = FxHashMap::default();
        let mut source_file_meta_datas: FxHashMap<Path, SourceFileMetaData> = FxHashMap::default();
        let mut jsx_runtime_import_specifiers: FxHashMap<Path, P<jsxRuntimeImportSpecifier>> = FxHashMap::default();
        let mut import_helpers_import_specifiers: FxHashMap<Path, P<Node>> = FxHashMap::default();
        let mut source_files_found_searching_node_modules: FxHashSet<Path> = FxHashSet::default();
        let mut lib_files_map: FxHashMap<Path, P<LibFile>> = FxHashMap::default();

        let dedupe = !loader.opts.config.compiler_options().unwrap().deduplicate_packages.is_false();
        let mut redirect_targets_map: FxHashMap<Path, Vec<String>> = FxHashMap::default();
        let mut redirect_files_by_path: FxHashMap<Path, redirectsFile> = FxHashMap::default();
        let mut package_id_to_source_file: FxHashMap<PackageId, P<SourceFile>> = FxHashMap::default();
        let force_consistent_casing = !loader.opts.config.compiler_options().unwrap().force_consistent_casing_in_file_names.is_false();

        struct Collector<'a> {
            loader: &'a mut fileLoader,
            seen: FxHashMap<DataId, std::sync::Arc<str>>,
        }

        // An explicit stack replaces Go's recursive collectFiles (the import graph of a large
        // project is deeper than the default thread stack allows). Each frame is a task list and
        // the index of the next task to visit; the post-subtask work of a task runs when its
        // child frame is popped.
        enum Frame {
            List { tasks: Vec<TaskId>, next: usize },
            After { task: TaskId, data: DataId },
        }

        // recordedDuplicates tracks, per task data, the set of file-name casings that
        // have already been recorded in duplicateSourceFiles. A file that is reached
        // from multiple import sites is walked once per site, but each distinct casing
        // is only parsed and acquired in the parse cache once. Recording the same casing
        // as a duplicate more than once would cause it to be released more times than it
        // was acquired when the snapshot is disposed, leaving a dangling cache entry that
        // panics the next time it is referenced.
        let mut recorded_duplicates: FxHashMap<DataId, FxHashSet<String>> = FxHashMap::default();
        let mut c = Collector { loader, seen: FxHashMap::default() };
        let mut stack: Vec<Frame> = vec![Frame::List { tasks: c.loader.root_tasks.clone(), next: 0 }];

        while let Some(frame) = stack.pop() {
            match frame {
                Frame::After { task, data } => {
                    let loader = &mut *c.loader;
                    // Exclude automatic type directive tasks from include reason processing,
                    // as these are internal implementation details and should not contribute
                    // to the reasons for including files.
                    if let Some(redirected) = loader.tasks[task].redirected_parse_task {
                        if !can_use_project_reference_source {
                            output_file_to_project_reference_source
                                .insert(loader.tasks[redirected].path.clone(), loader.tasks[task].normalized_file_path.to_string());
                        }
                        continue;
                    }

                    if loader.tasks[task].is_for_automatic_type_directive {
                        let path = loader.tasks[task].path.clone();
                        type_resolutions_in_file.insert(path, std::mem::take(&mut loader.tasks[task].data().type_resolutions_in_file));
                        let diags = loader.tasks[task].take_processing_diagnostics();
                        include_data.processing_diagnostics.extend(diags);
                        continue;
                    }

                    let path = loader.tasks[task].path.clone();

                    let diags = loader.tasks[task].take_processing_diagnostics();
                    include_data.processing_diagnostics.extend(diags);

                    let Some(file) = loader.tasks[task].file else {
                        missing_files.push(loader.tasks[task].normalized_file_path.to_string());
                        continue;
                    };

                    if let Some(lib_file) = loader.tasks[task].lib_file {
                        lib_files.push(file);
                        lib_files_map.insert(path.clone(), lib_file);
                    } else {
                        files.push(file);
                    }
                    files_by_path.insert(path.clone(), file);
                    let loaded = loader.tasks[task].data();
                    resolved_modules.insert(path.clone(), std::mem::take(&mut loaded.resolutions_in_file));
                    type_resolutions_in_file.insert(path.clone(), std::mem::take(&mut loaded.type_resolutions_in_file));
                    source_file_meta_datas.insert(path.clone(), loaded.metadata.clone());

                    if let Some(jsx) = loaded.jsx_runtime_import_specifier {
                        jsx_runtime_import_specifiers.insert(path.clone(), jsx);
                    }
                    if let Some(specifier) = loaded.import_helpers_import_specifier {
                        import_helpers_import_specifiers.insert(path.clone(), specifier);
                    }
                    if loader.files_parser.datas[data].lowest_depth > 0 {
                        source_files_found_searching_node_modules.insert(path);
                    }
                }
                Frame::List { tasks, next } => {
                    if next >= tasks.len() {
                        continue;
                    }
                    let mut task = tasks[next];
                    stack.push(Frame::List { tasks, next: next + 1 });

                    let loader = &mut *c.loader;
                    let include_reason = loader.tasks[task].include_reason;
                    // Exclude automatic type directive tasks from include reason processing,
                    // as these are internal implementation details and should not contribute
                    // to the reasons for including files.
                    if loader.tasks[task].redirected_parse_task.is_none() && !loader.tasks[task].is_for_automatic_type_directive {
                        if let Some(loaded_task) = loader.tasks[task].loaded_task {
                            task = loaded_task;
                        }
                        add_include_reason(loader, &mut include_data, task, include_reason);
                    }
                    let data = loader.files_parser.task_data_by_path[&loader.tasks[task].path];
                    if !loader.tasks[task].loaded {
                        continue;
                    }

                    // ensure we only walk each task once
                    if let Some(checked_name) = c.seen.get(&data) {
                        if let Some(file) = loader.tasks[task].file {
                            if *checked_name != loader.tasks[task].normalized_file_path
                                && recorded_duplicates.entry(data).or_default().insert(loader.tasks[task].normalized_file_path.to_string())
                            {
                                duplicate_source_files.push(duplicate_source_file(file));
                            }
                        }
                        // Identical names normalize identically; only differing ones need the comparison.
                        if force_consistent_casing && *checked_name != loader.tasks[task].normalized_file_path {
                            // Check if it differs only in drive letters its ok to ignore that error:
                            let checked_absolute_path =
                                tspath::get_normalized_absolute_path_without_root(checked_name, &loader.compare_paths_options.current_directory);
                            let input_absolute_path = tspath::get_normalized_absolute_path_without_root(
                                &loader.tasks[task].normalized_file_path,
                                &loader.compare_paths_options.current_directory,
                            );
                            if checked_absolute_path != input_absolute_path {
                                let checked_name = checked_name.clone();
                                include_data.add_processing_diagnostics_for_file_casing(
                                    loader.tasks[task].path.clone(),
                                    &checked_name,
                                    &loader.tasks[task].normalized_file_path.to_string(),
                                    include_reason,
                                );
                            }
                        }
                        continue;
                    } else {
                        c.seen.insert(data, loader.tasks[task].normalized_file_path.clone());
                    }

                    if let Some(seen_ignore_case) = &mut tasks_seen_by_name_ignore_case {
                        let path_lower_case = tspath::to_file_name_lower_case(loader.tasks[task].path.as_str());
                        if let Some(&task_by_ignore_case) = seen_ignore_case.get(&path_lower_case) {
                            let (p, n) =
                                (loader.tasks[task_by_ignore_case].path.clone(), loader.tasks[task_by_ignore_case].normalized_file_path.to_string());
                            include_data.add_processing_diagnostics_for_file_casing(
                                p,
                                &n,
                                &loader.tasks[task].normalized_file_path.to_string(),
                                include_reason,
                            );
                        } else {
                            seen_ignore_case.insert(path_lower_case, task);
                        }
                    }

                    if let Some(data) = loader.tasks[task].data_ref() {
                        for trace in &data.type_resolutions_trace {
                            loader.host.trace(trace.message, &trace_args(trace));
                        }
                        for trace in &data.resolutions_trace {
                            loader.host.trace(trace.message, &trace_args(trace));
                        }
                    }

                    let file = loader.tasks[task].file;
                    let data_package_id = loader.files_parser.datas[data].package_id.clone();
                    if dedupe && !data_package_id.name.is_empty() {
                        if let Some(&package_id_file) = package_id_to_source_file.get(&data_package_id) {
                            if let Some(file) = file {
                                // Package deduplication keeps the first package instance in the
                                // program, but we still parsed this file and acquired it through
                                // the host, so snapshot disposal must release that extra owner.
                                duplicate_source_files.push(duplicate_source_file(file));
                            }
                            redirect_targets_map
                                .entry(package_id_file.path().clone())
                                .or_default()
                                .push(loader.tasks[task].normalized_file_path.to_string());
                            let index = files.len() + redirect_files_by_path.len();
                            redirect_files_by_path.insert(
                                loader.tasks[task].path.clone(),
                                redirectsFile {
                                    index,
                                    file_name: loader.tasks[task].normalized_file_path.to_string(),
                                    path: loader.tasks[task].path.clone(),
                                    target: package_id_file.path().clone(),
                                },
                            );
                            files_by_path.insert(loader.tasks[task].path.clone(), package_id_file);
                            if loader.files_parser.datas[data].lowest_depth > 0 {
                                source_files_found_searching_node_modules.insert(loader.tasks[task].path.clone());
                            }
                            continue;
                        } else if let Some(file) = file {
                            package_id_to_source_file.insert(data_package_id, file);
                        }
                    }

                    stack.push(Frame::After { task, data });
                    let sub_tasks = loader.tasks[task].sub_tasks.clone();
                    if !sub_tasks.is_empty() {
                        stack.push(Frame::List { tasks: sub_tasks, next: 0 });
                    }
                }
            }
        }

        let loader = c.loader;
        loader.sort_libs(&mut lib_files);

        let lib_len = lib_files.len();
        let mut all_files = lib_files;
        all_files.extend(files);
        for redirect_file in redirect_files_by_path.values_mut() {
            redirect_file.index += lib_len;
        }

        let mut keys: Vec<Path> = loader.path_for_lib_file_resolutions.keys().cloned().collect();
        keys.sort();
        for key in keys {
            let value = &loader.path_for_lib_file_resolutions[&key];
            let mut cache = ModeAwareCache::default();
            if let Some(resolution) = value.resolution {
                cache.insert(tsrs_module::ModeAwareCacheKey { name: tsrs_core::alloc_str(&value.library_name), mode: ModuleKind::CommonJS }, resolution);
            }
            resolved_modules.insert(key.clone(), cache);
            for trace in &value.trace {
                loader.host.trace(trace.message, &trace_args(trace));
            }
        }

        processedFiles {
            finished_processing: true,
            files: tsrs_core::alloc_vec(all_files),
            duplicate_source_files,
            files_by_path,
            project_reference_file_mapper: Some(loader.project_references.take_mapper()),
            resolved_modules,
            type_resolutions_in_file,
            source_file_meta_datas,
            jsx_runtime_import_specifiers,
            import_helpers_import_specifiers,
            source_files_found_searching_node_modules,
            lib_files: lib_files_map,
            missing_files,
            file_include_data: include_data,
            output_file_to_project_reference_source,
            redirect_targets_map,
            redirect_files_by_path,
        }
    }
}

fn duplicate_source_file(file: P<SourceFile>) -> DuplicateSourceFile {
    DuplicateSourceFile { parse_options: file.parse_options().clone(), hash: file.hash.get(), script_kind: file.script_kind.get() }
}

fn add_include_reason(loader: &fileLoader, include_processor: &mut fileIncludeData, task: TaskId, reason: Option<P<FileIncludeReason>>) {
    if let Some(redirected) = loader.tasks[task].redirected_parse_task {
        add_include_reason(loader, include_processor, redirected, reason);
    } else if loader.tasks[task].loaded {
        if let Some(reason) = reason {
            include_processor.file_include_reasons.entry(loader.tasks[task].path.clone()).or_default().push(reason);
        }
    }
}

fn trace_args(trace: &DiagAndArgs) -> Vec<&dyn std::fmt::Display> {
    trace.args.iter().map(|a| a as &dyn std::fmt::Display).collect()
}
