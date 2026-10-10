use std::sync::Arc;

use rustc_hash::FxHashMap;
use tsrs_ast::{self as ast, FileReference, Kind, Node, NodeFactory, NodeFlags, SourceFile, SourceFileMetaData, SourceFileParseOptions, TokenFlags};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{alloc_str, CompilerOptions, ModuleKind, ModuleResolutionKind, ResolutionMode, ScriptKind, P};
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_module::{self as module, DiagAndArgs, ModeAwareCache, ModeAwareCacheKey, ResolvedModule, ResolvedTypeReferenceDirective, Resolver};
use tsrs_tsoptions::{self as tsoptions};

use crate::file_include::{fileIncludeKind, automaticTypeDirectiveFileData, FileIncludeReason};
use crate::filesparser::{filesParser, parseTask, resolvedRef, TaskId};
use crate::host::CompilerHost;
use crate::processing_diagnostic::{includeExplainingDiagnostic, processingDiagnostic};
use crate::program::{ProgramConfig, ProgramOptions};
use crate::projectreferencefilemapper::{projectReferenceFileMapper, projectReferenceFileMapperBuilder, projectReferenceRedirects, resolution_host_for};
use crate::projectreferenceparser::projectReferenceParser;
use crate::includeprocessor::fileIncludeData;

pub(crate) struct libResolution {
    pub(crate) library_name: String,
    pub(crate) resolution: Option<P<ResolvedModule>>,
    pub(crate) trace: Vec<DiagAndArgs>,
}

pub struct LibFile {
    pub name: String,
    pub(crate) path: String,
    pub replaced: bool,
}

pub(crate) struct sourceFileFromReferenceDiagnostic {
    pub(crate) message: &'static Message,
    pub(crate) args: Vec<String>,
}

pub(crate) struct fileLoader {
    pub(crate) opts: ProgramConfig,
    pub(crate) host: std::sync::Arc<dyn CompilerHost>,
    pub(crate) resolver: Box<dyn Resolver>,
    pub(crate) default_library_path: String,
    pub(crate) compare_paths_options: ComparePathsOptions,
    pub(crate) supported_extensions: Vec<Vec<String>>,
    pub(crate) supported_extensions_with_json_if_resolve_json_module: Vec<Vec<String>>,

    pub(crate) files_parser: filesParser,
    pub(crate) root_tasks: Vec<TaskId>,
    pub(crate) tasks: Vec<parseTask>,

    pub(crate) total_file_count: i32,
    pub(crate) lib_file_count: i32,

    pub(crate) factory: NodeFactory,

    pub(crate) project_references: projectReferenceFileMapperBuilder,

    pub(crate) path_for_lib_file_cache: FxHashMap<String, P<LibFile>>,
    pub(crate) path_for_lib_file_resolutions: FxHashMap<Path, libResolution>,

    pub(crate) module_resolution_error: Option<String>,
}

pub(crate) struct redirectsFile {
    // Index of file at which this redirect file needs to be iterated
    pub(crate) index: usize,
    pub(crate) file_name: String,
    pub(crate) path: Path,
    pub(crate) target: Path,
}

// fileloader.go:90 (content mappers are not ported: no content-mapper fields)
#[derive(Clone, Debug)]
pub struct DuplicateSourceFile {
    pub parse_options: SourceFileParseOptions,
    pub hash: u128,
    pub script_kind: ScriptKind,
}

#[derive(Default)]
pub struct processedFiles {
    pub(crate) files: Arc<[P<SourceFile>]>,
    // duplicateSourceFiles tracks parsed files loaded during program construction
    // that were later dropped from the final program, such as losing filename
    // casing variants for the same path or files hidden behind package redirect
    // deduplication. Their parse-cache acquires still need to be balanced when
    // the program is disposed.
    pub(crate) duplicate_source_files: Vec<DuplicateSourceFile>,
    pub(crate) files_by_path: FxHashMap<Path, P<SourceFile>>,
    pub(crate) project_reference_file_mapper: Option<Arc<projectReferenceFileMapper>>,
    pub(crate) missing_files: Vec<String>,
    pub(crate) resolved_modules: FxHashMap<Path, ModeAwareCache<P<ResolvedModule>>>,
    pub(crate) type_resolutions_in_file: FxHashMap<Path, ModeAwareCache<P<ResolvedTypeReferenceDirective>>>,
    pub(crate) source_file_meta_datas: FxHashMap<Path, SourceFileMetaData>,
    pub(crate) jsx_runtime_import_specifiers: FxHashMap<Path, P<jsxRuntimeImportSpecifier>>,
    pub(crate) import_helpers_import_specifiers: FxHashMap<Path, P<Node>>,
    pub(crate) lib_files: FxHashMap<Path, P<LibFile>>,
    // List of present unsupported extensions
    pub(crate) source_files_found_searching_node_modules: rustc_hash::FxHashSet<Path>,
    pub(crate) file_include_data: fileIncludeData,
    // if file was included using source file and its output is actually part of program
    // this contains mapping from output to source file
    pub(crate) output_file_to_project_reference_source: FxHashMap<Path, String>,
    // Key is a file path. Value is the list of files that redirect to it (same package, different install location)
    pub(crate) redirect_targets_map: FxHashMap<Path, Vec<String>>,
    // filesByPath for redirect files
    pub(crate) redirect_files_by_path: FxHashMap<Path, redirectsFile>,
    pub(crate) finished_processing: bool,
}

pub(crate) struct jsxRuntimeImportSpecifier {
    pub(crate) module_reference: String,
    pub(crate) specifier: P<Node>,
}

// fileloader.go:337 (Go runs it on the loader before creating the resolver; it only needs the options and the host,
// and returns the builder the loader keeps)
fn add_project_reference_tasks(opts: &ProgramConfig, host: &Arc<dyn CompilerHost>, _single_threaded: bool) -> projectReferenceFileMapperBuilder {
    let mut redirects = projectReferenceRedirects::new(opts.config, opts.can_use_project_reference_source());
    let resolution_host = resolution_host_for(Arc::clone(host));
    let project_references = opts.config.resolved_project_reference_paths();
    if !project_references.is_empty() {
        let mut parser = projectReferenceParser::new(&**host);
        let mut root_tasks = parser.create_project_reference_parse_tasks(project_references);
        parser.parse(&mut root_tasks, &mut redirects);
    }
    let mapper = Arc::new(projectReferenceFileMapper::new(redirects));
    let host = mapper.resolution_host(resolution_host);
    projectReferenceFileMapperBuilder { mapper, host }
}

pub(crate) fn process_all_program_files(opts: &ProgramOptions, single_threaded: bool) -> (processedFiles, P<module::ResolutionData>, Option<String>) {
    let compiler_options = opts.config.compiler_options().unwrap();
    let root_files = opts.config.file_names();
    let supported_extensions = tsoptions::get_supported_extensions(Some(&compiler_options), &[]);
    let supported_extensions_with_json_if_resolve_json_module =
        tsoptions::get_supported_extensions_with_json_if_resolve_json_module(Some(&compiler_options), &supported_extensions);
    // Go `int`. It is only compared with node_modules depths (small non-negative counts), so saturating to i32 keeps
    // every comparison's result.
    let max_node_module_js_depth = compiler_options.max_node_module_js_depth.unwrap_or(0).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let host = Arc::clone(&opts.host);
    let project_references = add_project_reference_tasks(&opts.program_config(), &host, single_threaded);
    let resolver_options = module::ResolverOptions {
        host: Arc::clone(&project_references.host),
        compiler_options,
        typings_location: opts.typings_location.clone(),
        project_name: opts.project_name.clone(),
        extra_extensions: Vec::new(),
        package_json_cache: None,
    };
    let resolver: Box<dyn Resolver> = match &opts.create_module_resolver {
        Some(create) => create(resolver_options),
        None => Box::new(module::new_resolver(resolver_options)),
    };
    let mut loader = fileLoader {
        opts: opts.program_config(),
        host: Arc::clone(&host),
        resolver,
        default_library_path: tspath::get_normalized_absolute_path(host.default_library_path(), host.get_current_directory()),
        compare_paths_options: ComparePathsOptions {
            use_case_sensitive_file_names: host.fs().use_case_sensitive_file_names(),
            current_directory: host.get_current_directory().to_string(),
        },
        files_parser: filesParser::new(single_threaded, max_node_module_js_depth),
        root_tasks: Vec::with_capacity(root_files.len() + compiler_options.lib.as_ref().map_or(0, |l| l.len())),
        tasks: Vec::new(),
        total_file_count: 0,
        lib_file_count: 0,
        factory: NodeFactory::default(),
        project_references,
        path_for_lib_file_cache: FxHashMap::default(),
        path_for_lib_file_resolutions: FxHashMap::default(),
        supported_extensions,
        supported_extensions_with_json_if_resolve_json_module,
        module_resolution_error: None,
    };
    let roots_start = std::time::Instant::now();
    // The lookups (a file_exists per root file) are independent; tasks are still created in root order.
    let curr_dir = host.get_current_directory().to_string();
    let containing_file = match &loader.opts.config.config_file {
        Some(config_file) => tspath::get_normalized_absolute_path(config_file.source_file.file_name(), &curr_dir),
        None => curr_dir.clone(),
    };
    let lookup = |root_file: &String| {
        let abs_path = tspath::get_normalized_absolute_path(root_file, &curr_dir);
        // A root file reason is never a referenced-file reason, which is all the lookup asks of it.
        let looked_up = source_file_from_reference(
            &compiler_options,
            host.fs(),
            &loader.supported_extensions,
            &loader.supported_extensions_with_json_if_resolve_json_module,
            &abs_path,
            root_file,
            &containing_file,
            false,
        );
        // The task's path, which `filesParser::start` would otherwise compute from the task's file name on this thread.
        let name = if looked_up.1.is_some() { &abs_path } else { &looked_up.0 };
        let path = tspath::to_path(name, host.get_current_directory(), host.fs().use_case_sensitive_file_names());
        (abs_path, looked_up, path)
    };
    let looked_up: Vec<(String, (String, Option<sourceFileFromReferenceDiagnostic>), Path)> = if single_threaded {
        root_files.iter().map(lookup).collect()
    } else {
        use rayon::prelude::*;
        crate::program::worker_pool().install(|| root_files.par_iter().map(lookup).collect())
    };
    for (index, (abs_path, (resolved_file, diagnostic), path)) in looked_up.into_iter().enumerate() {
        let mut reason = FileIncludeReason::new(fileIncludeKind::RootFile);
        reason.index = index;
        loader.add_root_file_task_with(abs_path, resolved_file, diagnostic, None, P::new(reason), path);
    }
    tsrs_core::phases::record("Program: root file lookups", roots_start.elapsed());
    if !root_files.is_empty() && compiler_options.no_lib.is_false_or_unknown() {
        if compiler_options.lib.is_none() {
            let name = tsoptions::get_default_lib_file_name(&compiler_options);
            let lib_file = loader.path_for_lib_file(name);
            let mut reason = FileIncludeReason::new(fileIncludeKind::LibFile);
            reason.is_default_lib = true;
            loader.add_root_task(&lib_file.path.clone(), Some(lib_file), P::new(reason));
        } else {
            for (index, lib) in compiler_options.lib.as_deref().unwrap_or(&[]).iter().enumerate() {
                if let Some(name) = tsoptions::get_lib_file_name(lib) {
                    let lib_file = loader.path_for_lib_file(&name);
                    let mut reason = FileIncludeReason::new(fileIncludeKind::LibFile);
                    reason.index = index;
                    loader.add_root_task(&lib_file.path.clone(), Some(lib_file), P::new(reason));
                }
                // !!! error on unknown name
            }
        }
    }

    if !root_files.is_empty() && !opts.skip_module_resolution {
        loader.add_automatic_type_directive_tasks();
    }

    let root_tasks = loader.root_tasks.clone();
    tsrs_core::phases::time("Program: file graph (parse + resolve)", || filesParser::parse(&mut loader, &root_tasks));

    let processed = tsrs_core::phases::time("Program: collect files", || filesParser::get_processed_files(&mut loader));
    let resolution_data = loader.resolver.get_resolution_data();
    let module_resolution_error = loader.module_resolution_error.take();
    // tsrs-only: the loader's tasks and maps and the resolver's caches (heap only; nothing the program refers to) are
    // freed on a helper thread while the program is set up, not on this thread (vscode: 3-4 ms of frees).
    if !single_threaded {
        let fileLoader { tasks, files_parser, resolver, .. } = loader;
        std::thread::spawn(move || drop((tasks, files_parser, resolver)));
    }
    (processed, resolution_data, module_resolution_error)
}

impl fileLoader {
    pub(crate) fn to_path(&self, file: &str) -> Path {
        tspath::to_path(file, self.host.get_current_directory(), self.host.fs().use_case_sensitive_file_names())
    }

    fn new_task(&mut self, task: parseTask) -> TaskId {
        self.tasks.push(task);
        self.tasks.len() - 1
    }

    fn add_root_task(&mut self, file_name: &str, lib_file: Option<P<LibFile>>, include_reason: P<FileIncludeReason>) {
        let abs_path = tspath::get_normalized_absolute_path(file_name, self.host.get_current_directory());
        if self.opts.config.compiler_options().unwrap().allow_non_ts_extensions.is_true() || tspath::has_extension(&abs_path) {
            let mut task = parseTask::new(abs_path);
            task.lib_file = lib_file;
            task.include_reason = Some(include_reason);
            let id = self.new_task(task);
            self.root_tasks.push(id);
        }
    }

    fn add_root_file_task_with(
        &mut self,
        abs_path: String,
        resolved_file: String,
        diagnostic: Option<sourceFileFromReferenceDiagnostic>,
        lib_file: Option<P<LibFile>>,
        include_reason: P<FileIncludeReason>,
        path: Path,
    ) {
        let mut root_task = parseTask::new(resolved_file);
        root_task.path = path;
        root_task.lib_file = lib_file;
        root_task.include_reason = Some(include_reason);
        if let Some(diagnostic) = diagnostic {
            root_task.normalized_file_path = abs_path.into();
            root_task.failed_lookup = true;
            root_task.data().processing_diagnostics = vec![processingDiagnostic::explaining(includeExplainingDiagnostic {
                file: None,
                diagnostic_reason: Some(include_reason),
                message: diagnostic.message,
                args: diagnostic.args,
            })];
        }
        let id = self.new_task(root_task);
        self.root_tasks.push(id);
    }

    fn add_automatic_type_directive_tasks(&mut self) {
        let compiler_options = self.opts.config.compiler_options().unwrap();
        let containing_directory = if !compiler_options.config_file_path.is_empty() {
            tspath::get_directory_path(&compiler_options.config_file_path)
        } else {
            self.host.get_current_directory().to_string()
        };
        let containing_file_name = tspath::combine_paths(&containing_directory, &[module::INFERRED_TYPES_CONTAINING_FILE]);
        let mut task = parseTask::new(containing_file_name);
        task.is_for_automatic_type_directive = true;
        let id = self.new_task(task);
        self.root_tasks.push(id);
    }

    pub(crate) fn resolve_automatic_type_directives(
        &mut self,
        containing_file_name: &str,
    ) -> (Vec<resolvedRef>, ModeAwareCache<P<ResolvedTypeReferenceDirective>>, Vec<DiagAndArgs>, Vec<processingDiagnostic>) {
        let mut to_parse = Vec::new();
        let mut type_resolutions_in_file = ModeAwareCache::default();
        let mut type_resolutions_trace = Vec::new();
        let mut p_diagnostics = Vec::new();
        let automatic_type_directive_names =
            module::get_automatic_type_directive_names(&self.opts.config.compiler_options().unwrap(), &*self.project_references.host);
        for name in automatic_type_directive_names {
            // Under node16/nodenext module resolution, load `types`/ata include names as cjs resolution results by passing an `undefined` mode.
            // Under bundler module resolution, this also triggers the "import" condition to be used.
            let resolution_mode = ModuleKind::None;
            let (resolved, trace) = self.resolver.resolve_type_reference_directive(&name, containing_file_name, resolution_mode, None);
            type_resolutions_in_file.insert(ModeAwareCacheKey { name: alloc_str(&name), mode: resolution_mode }, resolved);
            type_resolutions_trace.extend(trace);
            if resolved.is_resolved() {
                let mut reason = FileIncludeReason::new(fileIncludeKind::AutomaticTypeDirectiveFile);
                reason.automatic_type_directive =
                    Some(P::new(automaticTypeDirectiveFileData { type_reference: name.clone(), package_id: resolved.package_id }));
                to_parse.push(resolvedRef {
                    file_name: std::borrow::Cow::Borrowed(resolved.resolved_file_name),
                    increase_depth: resolved.is_external_library_import,
                    elide_on_depth: false,
                    include_reason: P::new(reason),
                    package_id: resolved.package_id,
                });
            } else {
                let mut reason = FileIncludeReason::new(fileIncludeKind::AutomaticTypeDirectiveFile);
                reason.automatic_type_directive =
                    Some(P::new(automaticTypeDirectiveFileData { type_reference: name.clone(), package_id: Default::default() }));
                p_diagnostics.push(processingDiagnostic::explaining(includeExplainingDiagnostic {
                    file: None,
                    diagnostic_reason: Some(P::new(reason)),
                    message: &diagnostics::Cannot_find_type_definition_file_for_0,
                    args: vec![name.clone()],
                }));
            }
        }
        (to_parse, type_resolutions_in_file, type_resolutions_trace, p_diagnostics)
    }

    pub(crate) fn sort_libs(&self, lib_files: &mut [P<SourceFile>]) {
        lib_files.sort_by_key(|f| self.get_default_lib_file_priority(*f));
    }

    fn get_default_lib_file_priority(&self, a: P<SourceFile>) -> usize {
        // defaultLibraryPath and a.FileName() are absolute and normalized; a prefix check should suffice.
        let default_library_path = tspath::remove_trailing_directory_separator(&self.default_library_path);
        let a_file_name = a.file_name();

        if a_file_name.starts_with(default_library_path)
            && a_file_name.len() > default_library_path.len()
            && a_file_name.as_bytes()[default_library_path.len()] == b'/'
        {
            // avoid tspath.GetBaseFileName; we know these paths are already absolute and normalized.
            let basename = &a_file_name[a_file_name.rfind('/').unwrap() + 1..];
            if basename == "lib.d.ts" || basename == "lib.es6.d.ts" {
                return 0;
            }
            let name = basename.strip_prefix("lib.").unwrap_or(basename);
            let name = name.strip_suffix(".d.ts").unwrap_or(name);
            if let Some(index) = tsoptions::LIBS.iter().position(|l| *l == name) {
                return index + 1;
            }
        }
        tsoptions::LIBS.len() + 2
    }

    pub(crate) fn load_source_file_meta_data(&self, file_name: &str) -> SourceFileMetaData {
        source_file_meta_data(&self.opts, &*self.resolver, &self.project_references, file_name)
    }

    pub(crate) fn parse_options_for_task(&self, t: TaskId) -> SourceFileParseOptions {
        let task = &self.tasks[t];
        parse_options_for(&*self.host, &self.project_references, &task.normalized_file_path, &task.path, &task.metadata())
    }

    pub(crate) fn parse_source_file(&self, t: TaskId) -> Option<P<SourceFile>> {
        let is_root = self.tasks[t].include_reason.is_some_and(|r| r.is_root_file());
        crate::fileregions::parse_task(is_root, || self.host.get_source_file(self.parse_options_for_task(t)))
    }
}

pub(crate) fn source_file_meta_data(
    opts: &ProgramConfig,
    resolver: &dyn Resolver,
    project_references: &projectReferenceFileMapperBuilder,
    file_name: &str,
) -> SourceFileMetaData {
    if opts.skip_module_resolution {
        return SourceFileMetaData {
            implied_node_format: ast::get_implied_node_format_for_file(file_name, ""),
            ..Default::default()
        };
    }

    let package_json_scope = resolver
        .get_resolution_data()
        .get()
        .new_resolver(Arc::clone(&project_references.host))
        .get_package_scope_for_path(&tspath::get_directory_path(file_name));
    let module_resolution_kind = opts.config.compiler_options().unwrap().get_module_resolution_kind();

    let mut package_json_type = String::new();
    let mut package_json_directory = String::new();
    if let Some(scope) = package_json_scope.filter(|s| s.exists()) {
        package_json_directory = scope.package_directory.to_string();
        if let Some(value) = scope.contents.unwrap().fields.type_.get_value() {
            if !tspath::file_extension_is_one_of(
                file_name,
                &[tspath::EXTENSION_MTS, tspath::EXTENSION_CTS, tspath::EXTENSION_MJS, tspath::EXTENSION_CJS],
            ) && ModuleResolutionKind::Node16 <= module_resolution_kind
                && module_resolution_kind <= ModuleResolutionKind::NodeNext
                || file_name.contains("/node_modules/")
            {
                package_json_type.clone_from(value);
            }
        }
    }

    let implied_node_format = ast::get_implied_node_format_for_file(file_name, &package_json_type);
    SourceFileMetaData { package_json_type, package_json_directory, implied_node_format }
}

pub(crate) fn parse_options_for(
    host: &dyn CompilerHost,
    project_references: &projectReferenceFileMapperBuilder,
    normalized_file_path: &str,
    // The task's path when it is already known (`to_path(normalized_file_path)`, filesParser::start): sharing it
    // keeps one copy of the string per file.
    path: &Path,
    metadata: &SourceFileMetaData,
) -> SourceFileParseOptions {
    let path = if path.is_empty() {
        tspath::to_path(normalized_file_path, host.get_current_directory(), host.fs().use_case_sensitive_file_names())
    } else {
        path.clone()
    };
    let options = project_references.get_compiler_options_for_file(normalized_file_path, &path);
    SourceFileParseOptions {
        file_name: normalized_file_path.to_string(),
        path,
        external_module_indicator_options: ast::get_external_module_indicator_options(normalized_file_path, &options, metadata),
    }
}

// The module and type reference resolutions of one file, computed on a worker thread ahead of the sequential
// load (filesparser.rs prefetch): one entry per import and string-literal module augmentation (None for an empty
// name), and one per type reference directive, in source order. Resolution results do not depend on the order
// in which files are resolved (the resolver caches are keyed by name, directory, mode and redirect), so the
// sequential load consumes them exactly as if it had resolved on the spot.
pub(crate) struct prefetchedResolutions {
    pub(crate) referenced_files: Vec<(String, Option<sourceFileFromReferenceDiagnostic>)>,
    pub(crate) imports: Vec<Option<prefetchedImport>>,
    pub(crate) type_references: Vec<(P<ResolvedTypeReferenceDirective>, Vec<DiagAndArgs>)>,
    // The task's path as its include reasons store it (`parseTask::reason_path`), when a prepared sub task needed one.
    pub(crate) reason_path: Option<&'static str>,
    // The file's resolutions keyed by module name and mode, inserted in import order: what the sequential load builds
    // for a file without synthetic imports, so it takes this one instead (the same hashing and insertion sequence).
    pub(crate) resolutions_in_file: ModeAwareCache<P<ResolvedModule>>,
}

impl prefetchedResolutions {
    /// At most the number of sub tasks that loading the file adds from these resolutions.
    pub(crate) fn sub_task_bound(&self) -> usize {
        self.referenced_files.len() + self.imports.len() + self.type_references.len()
    }
}

pub(crate) struct prefetchedImport {
    pub(crate) resolution: Result<(P<ResolvedModule>, Vec<DiagAndArgs>), String>,
    // The import's resolution mode.
    pub(crate) mode: ResolutionMode,
    // For a resolved module that `resolved_import_sub_task` adds to the program: the sub task's fields, so the
    // sequential load only appends the task (vscode: 110k import edges; computing this there was most of it).
    pub(crate) prepared: Option<preparedSubTask>,
}

// A sub task of an import, as `add_sub_task_normalized` would build it, computed on the worker pool from what the task
// map held when the round's prefetch started. Everything depends only on the file, its metadata, the resolution and
// the tasks that exist before the round; `data_id` is `NO_DATA` for a path first seen in this round, and the
// sequential load then looks it up again (an earlier file of the round may have added it).
pub(crate) struct preparedSubTask {
    pub(crate) normalized_file_path: Arc<str>,
    pub(crate) path: Path,
    pub(crate) data_id: u32,
    pub(crate) include_reason: P<FileIncludeReason>,
    pub(crate) increase_depth: bool,
    pub(crate) elide_on_depth: bool,
    pub(crate) package_id: tsrs_module::PackageId,
}

// The parts of the file loader that the parallel prefetch reads (the loader itself is not Sync).
pub(crate) struct prefetchContext<'a> {
    pub(crate) opts: &'a ProgramConfig,
    pub(crate) host: &'a dyn CompilerHost,
    pub(crate) resolver: &'a dyn Resolver,
    pub(crate) project_references: &'a projectReferenceFileMapperBuilder,
    pub(crate) supported_extensions: &'a [Vec<String>],
    pub(crate) supported_extensions_with_json: &'a [Vec<String>],
    // The tasks that exist when the round starts (read-only until its sequential load).
    pub(crate) task_data_by_path: &'a FxHashMap<Path, crate::filesparser::DataId>,
    pub(crate) datas: &'a [crate::filesparser::parseTaskData],
}

impl fileLoader {
    pub(crate) fn prefetch_context(&self) -> prefetchContext<'_> {
        prefetchContext {
            opts: &self.opts,
            host: &*self.host,
            resolver: &*self.resolver,
            project_references: &self.project_references,
            supported_extensions: &self.supported_extensions,
            supported_extensions_with_json: &self.supported_extensions_with_json_if_resolve_json_module,
            task_data_by_path: &self.files_parser.task_data_by_path,
            datas: &self.files_parser.datas,
        }
    }
}

// The lookup half of resolveTripleslashPathReference: the normalized referenced file name and the result of
// getSourceFileFromReference for it.
fn tripleslash_reference_lookup(
    ctx: &prefetchContext,
    module_name: &str,
    containing_file: &str,
) -> (String, (String, Option<sourceFileFromReferenceDiagnostic>)) {
    let base_path = tspath::get_directory_path(containing_file);
    let mut referenced_file_name = module_name.to_string();

    if !tspath::is_rooted_disk_path(module_name) {
        referenced_file_name = tspath::combine_paths(&base_path, &[module_name]);
    }
    let normalized_file_name = tspath::normalize_path(&referenced_file_name);
    // The include reason of a triple-slash reference is a referenced-file reason.
    let looked_up = source_file_from_reference(
        &ctx.opts.config.compiler_options().unwrap(),
        ctx.host.fs(),
        ctx.supported_extensions,
        ctx.supported_extensions_with_json,
        &normalized_file_name,
        module_name,
        containing_file,
        true,
    );
    (normalized_file_name, looked_up)
}

pub(crate) fn prefetch_resolutions(ctx: &prefetchContext, file: P<SourceFile>, meta: &SourceFileMetaData) -> prefetchedResolutions {
    let (opts, resolver, project_references) = (ctx.opts, ctx.resolver, ctx.project_references);
    let compiler_options = opts.config.compiler_options().unwrap();
    let (redirect, file_name) = project_references.get_redirect_for_resolution(file.file_name(), &file.path());
    let redirect = redirect.map(crate::projectreferencefilemapper::as_resolved_project_reference);
    let options_for_file = module::get_compiler_options_with_redirect(compiler_options, redirect);
    let mut referenced_files = Vec::new();
    let mut type_references = Vec::new();
    if !compiler_options.no_resolve.is_true() && !opts.skip_module_resolution {
        for ref_ in file.referenced_files.get().iter() {
            referenced_files.push(tripleslash_reference_lookup(ctx, &ref_.file_name, file.file_name()).1);
        }
        for ref_ in file.type_reference_directives.get().iter() {
            let resolution_mode = get_mode_for_type_reference_directive_in_file(*ref_, file, meta, &options_for_file);
            type_references.push(resolver.resolve_type_reference_directive(&ref_.file_name, &file_name, resolution_mode, redirect));
        }
    }
    let mut imports = Vec::new();
    let mut resolutions_in_file: ModeAwareCache<P<ResolvedModule>> = ModeAwareCache::default();
    let mut reason_path: Option<&'static str> = None;
    if !opts.skip_module_resolution {
        let imports_of_file = file.imports();
        let augmentations = file.module_augmentations.get().iter().copied().filter(|imp| imp.kind() == Kind::StringLiteral);
        for (import_index, entry) in imports_of_file.iter().copied().chain(augmentations).enumerate() {
            let module_name = entry.text();
            if module_name.is_empty() {
                imports.push(None);
                continue;
            }
            let mode = get_mode_for_usage_location(file.file_name(), meta, entry, Some(&options_for_file));
            let resolution = resolver.resolve_module_name(module_name, &file_name, mode, redirect);
            // As resolve_imports_and_module_augmentations inserts them, in the same order (a failed resolution gets a
            // default module).
            let resolved_module = match &resolution {
                Ok((resolved, _)) => *resolved,
                Err(_) => P::new(ResolvedModule::default()),
            };
            resolutions_in_file.insert(ModeAwareCacheKey { name: alloc_str(module_name), mode }, resolved_module);
            let mut prepared = None;
            if let Ok((resolved, _)) = &resolution {
                if resolved.is_resolved() {
                    let (should_add_file, is_js_file_from_node_modules) =
                        resolved_import_sub_task(project_references, ctx.host, &options_for_file, file, module_name, import_index as i32, resolved);
                    if should_add_file {
                        let reason_path = *reason_path.get_or_insert_with(|| tsrs_core::alloc_str(file.path().as_str()));
                        let include_reason = FileIncludeReason::new_referenced(fileIncludeKind::Import, reason_path, import_index as i32, None);
                        prepared = Some(prepare_sub_task(ctx, resolved, include_reason, is_js_file_from_node_modules));
                    }
                }
            }
            imports.push(Some(prefetchedImport { resolution, mode, prepared }));
        }
    }
    prefetchedResolutions { referenced_files, imports, type_references, reason_path, resolutions_in_file }
}

// The sub task for a resolved import (what `add_sub_task_normalized` builds), from the tasks known before the round.
fn prepare_sub_task(
    ctx: &prefetchContext,
    resolved: &ResolvedModule,
    include_reason: P<FileIncludeReason>,
    elide_on_depth: bool,
) -> preparedSubTask {
    let normalized_file_path = tspath::normalize_path(resolved.resolved_file_name);
    let mut path = tspath::to_path(&normalized_file_path, ctx.host.get_current_directory(), ctx.host.fs().use_case_sensitive_file_names());
    // A reference to a file that already has a task shares that task's strings (one copy per file name, as Go's
    // strings are shared); the values are equal either way.
    let mut shared_name: Option<Arc<str>> = None;
    let mut data_id = crate::filesparser::NO_DATA;
    if let Some((known_path, &data)) = ctx.task_data_by_path.get_key_value(&path) {
        path = known_path.clone();
        shared_name = ctx.datas[data].tasks.get_key_value(&normalized_file_path).map(|(name, _)| Arc::clone(name));
        data_id = data as u32;
    }
    preparedSubTask {
        normalized_file_path: shared_name.unwrap_or_else(|| Arc::from(normalized_file_path)),
        path,
        data_id,
        include_reason,
        increase_depth: resolved.is_external_library_import,
        elide_on_depth,
        package_id: resolved.package_id,
    }
}

impl fileLoader {
    pub(crate) fn is_supported_extension(&self, canonical_file_name: &str) -> bool {
        is_supported_extension(&self.supported_extensions_with_json_if_resolve_json_module, canonical_file_name)
    }

    // `prefetched` is the lookup computed ahead by the prefetch (tripleslash_reference_lookup), if any.
    pub(crate) fn resolve_tripleslash_path_reference(
        &self,
        module_name: &str,
        containing_file: &str,
        index: usize,
        prefetched: Option<(String, Option<sourceFileFromReferenceDiagnostic>)>,
    ) -> Result<resolvedRef, processingDiagnostic> {
        let include_reason =
            FileIncludeReason::new_referenced(
            fileIncludeKind::ReferenceFile,
            tsrs_core::alloc_str(self.to_path(containing_file).as_str()),
            index as i32,
            None,
        );

        let (resolved_file_name, diagnostic) = match prefetched {
            Some(looked_up) => looked_up,
            None => tripleslash_reference_lookup(&self.prefetch_context(), module_name, containing_file).1,
        };
        if let Some(diagnostic) = diagnostic {
            return Err(processingDiagnostic::explaining(includeExplainingDiagnostic {
                file: None,
                diagnostic_reason: Some(include_reason),
                message: diagnostic.message,
                args: diagnostic.args,
            }));
        }

        Ok(resolvedRef {
            file_name: std::borrow::Cow::Owned(resolved_file_name),
            increase_depth: false,
            elide_on_depth: false,
            include_reason,
            package_id: Default::default(),
        })
    }

    pub(crate) fn resolve_type_reference_directives(&mut self, t: TaskId) {
        let file = self.tasks[t].file.unwrap();
        let type_reference_directives = file.type_reference_directives.get();
        if type_reference_directives.is_empty() {
            return;
        }
        let meta = self.tasks[t].metadata();
        let task_path = self.tasks[t].reason_path();
        let mut prefetched =
            self.tasks[t].data().prefetched_resolutions.as_mut().map(|p| std::mem::take(&mut p.type_references).into_iter());

        let mut type_resolutions_in_file = ModeAwareCache::default();
        let mut type_resolutions_trace = Vec::new();
        for (index, ref_) in type_reference_directives.iter().enumerate() {
            let (redirect, file_name) = self.project_references.get_redirect_for_resolution(file.file_name(), &file.path());
            let redirect = redirect.map(crate::projectreferencefilemapper::as_resolved_project_reference);
            let resolution_mode = get_mode_for_type_reference_directive_in_file(
                *ref_,
                file,
                &meta,
                &module::get_compiler_options_with_redirect(self.opts.config.compiler_options().unwrap(), redirect),
            );
            let (resolved, trace) = match prefetched.as_mut() {
                Some(prefetched) => prefetched.next().expect("prefetched type reference resolution"),
                None => self.resolver.resolve_type_reference_directive(&ref_.file_name, &file_name, resolution_mode, redirect),
            };
            type_resolutions_in_file.insert(ModeAwareCacheKey { name: alloc_str(&ref_.file_name), mode: resolution_mode }, resolved);
            let include_reason =
                FileIncludeReason::new_referenced(fileIncludeKind::TypeReferenceDirective, task_path, index as i32, None);
            type_resolutions_trace.extend(trace);

            if resolved.is_resolved() {
                self.add_sub_task(
                    t,
                    &resolvedRef {
                        file_name: std::borrow::Cow::Borrowed(resolved.resolved_file_name),
                        increase_depth: resolved.is_external_library_import,
                        elide_on_depth: false,
                        include_reason,
                        package_id: resolved.package_id,
                    },
                    None,
                );
            } else {
                self.tasks[t].data().processing_diagnostics.push(processingDiagnostic::unknown_reference(include_reason));
            }
        }

        let data = self.tasks[t].data();
        data.type_resolutions_in_file = type_resolutions_in_file;
        data.type_resolutions_trace = type_resolutions_trace;
    }

    pub(crate) fn resolve_imports_and_module_augmentations(&mut self, t: TaskId) {
        let file = self.tasks[t].file.unwrap();

        let imports = file.imports();
        let mut module_names: Vec<P<Node>> = Vec::with_capacity(imports.len() + file.module_augmentations.get().len() + 2);

        let is_java_script_file = ast::is_source_file_js(file);
        let is_external_module_file = ast::is_external_module(file);

        let (redirect, file_name) = self.project_references.get_redirect_for_resolution(file.file_name(), &file.path());
        let redirect = redirect.map(crate::projectreferencefilemapper::as_resolved_project_reference);
        let options_for_file = module::get_compiler_options_with_redirect(self.opts.config.compiler_options().unwrap(), redirect);
        if is_java_script_file || (!file.is_declaration_file.get() && (options_for_file.get_isolated_modules() || is_external_module_file)) {
            if options_for_file.import_helpers.is_true() {
                let specifier = self.create_synthetic_import(EXTERNAL_HELPERS_MODULE_NAME_TEXT, file);
                module_names.push(specifier);
                self.tasks[t].data().import_helpers_import_specifier = Some(specifier);
            }
        }

        if is_java_script_file || file.script_kind.get() == tsrs_core::ScriptKind::TSX {
            let jsx_import = ast::get_jsx_runtime_import(&ast::get_jsx_implicit_import_base(&options_for_file, Some(file)), &options_for_file);
            if !jsx_import.is_empty() {
                let specifier = self.create_synthetic_import(&jsx_import, file);
                module_names.push(specifier);
                self.tasks[t].data().jsx_runtime_import_specifier =
                    Some(P::new(jsxRuntimeImportSpecifier { module_reference: jsx_import.clone(), specifier }));
            }
        }

        let imports_start = module_names.len() as i32;

        module_names.extend_from_slice(imports);
        for &imp in file.module_augmentations.get() {
            if imp.kind() == Kind::StringLiteral {
                module_names.push(imp);
            }
            // Do nothing if it's an Identifier; we don't need to do module resolution for `declare global`.
        }

        if self.opts.skip_module_resolution {
            return;
        }

        if !module_names.is_empty() {
            let mut resolutions_in_file: ModeAwareCache<P<ResolvedModule>> = ModeAwareCache::default();
            let mut resolutions_trace = Vec::new();
            let prefetched = self.tasks[t].data().prefetched_resolutions.take();
            if imports_start == 0 {
                if let Some(prefetched) = prefetched {
                    // Without synthetic imports the module names are the prefetch's, in its order: its map of
                    // resolutions is the one this loop would build. Only the sub tasks and the first resolution
                    // error remain.
                    resolutions_in_file = prefetched.resolutions_in_file;
                    for import in prefetched.imports.into_iter().flatten() {
                        match import.resolution {
                            Ok((_, trace)) => resolutions_trace.extend(trace),
                            Err(err) => {
                                if self.module_resolution_error.is_none() {
                                    self.module_resolution_error = Some(err);
                                }
                            }
                        }
                        if let Some(prepared) = import.prepared {
                            self.add_prepared_sub_task(t, prepared);
                        }
                    }
                    let data = self.tasks[t].data();
                    data.resolutions_in_file = resolutions_in_file;
                    data.resolutions_trace = resolutions_trace;
                    return;
                }
            }
            let mut prefetched = prefetched.map(|p| p.imports);
            // The metadata (two strings, cloned) only serves resolution modes the prefetch did not compute.
            let meta = if prefetched.is_none() || imports_start > 0 { Some(self.tasks[t].metadata()) } else { None };

            for (index, &entry) in module_names.iter().enumerate() {
                let prefetched_resolution = match prefetched.as_mut() {
                    Some(prefetched) if index as i32 >= imports_start => prefetched[index - imports_start as usize].take(),
                    _ => None,
                };
                let module_name = entry.text();
                if module_name.is_empty() {
                    continue;
                }

                let (resolution, mode, prepared) = match prefetched_resolution {
                    Some(prefetched) => (prefetched.resolution, prefetched.mode, Some(prefetched.prepared)),
                    None => {
                        let mode = get_mode_for_usage_location(file.file_name(), meta.as_ref().expect("metadata"), entry, Some(&options_for_file));
                        (self.resolver.resolve_module_name(module_name, &file_name, mode, redirect), mode, None)
                    }
                };
                let (resolved_module, trace) = match resolution {
                    Ok((resolved_module, trace)) => (resolved_module, trace),
                    Err(err) => {
                        if self.module_resolution_error.is_none() {
                            self.module_resolution_error = Some(err);
                        }
                        (P::new(ResolvedModule::default()), Vec::new())
                    }
                };
                resolutions_in_file.insert(ModeAwareCacheKey { name: alloc_str(module_name), mode }, resolved_module);
                resolutions_trace.extend(trace);

                if !resolved_module.is_resolved() {
                    continue;
                }

                let resolved_file_name = resolved_module.resolved_file_name;
                let import_index = index as i32 - imports_start;
                let Some(prepared) = prepared else {
                    // Not prefetched (`--traceResolution`, a single file, or a synthetic import).
                    let (should_add_file, is_js_file_from_node_modules) =
                        resolved_import_sub_task(&self.project_references, &*self.host, &options_for_file, file, module_name, import_index, &resolved_module);
                    if should_add_file {
                        let include_reason = FileIncludeReason::new_referenced(
                            fileIncludeKind::Import,
                            self.tasks[t].reason_path(),
                            import_index,
                            if import_index < 0 { Some(entry) } else { None },
                        );
                        self.add_sub_task_normalized(
                            t,
                            &resolvedRef {
                                file_name: std::borrow::Cow::Borrowed(resolved_file_name),
                                increase_depth: resolved_module.is_external_library_import,
                                elide_on_depth: is_js_file_from_node_modules,
                                include_reason,
                                package_id: resolved_module.package_id,
                            },
                            None,
                            None,
                        );
                    }
                    continue;
                };
                if let Some(prepared) = prepared {
                    self.add_prepared_sub_task(t, prepared);
                }
            }

            let data = self.tasks[t].data();
            data.resolutions_in_file = resolutions_in_file;
            data.resolutions_trace = resolutions_trace;
        }
    }

    fn create_synthetic_import(&mut self, text: &str, file: P<SourceFile>) -> P<Node> {
        let external_helpers_module_reference = self.factory.new_string_literal(tsrs_core::alloc_str(text), TokenFlags::None);
        let import_decl = self.factory.new_import_declaration(None, None, external_helpers_module_reference, None);
        external_helpers_module_reference.set_parent(Some(import_decl));
        import_decl.set_parent(Some(file.as_node()));
        external_helpers_module_reference
    }

    pub(crate) fn path_for_lib_file(&mut self, name: &str) -> P<LibFile> {
        if let Some(cached) = self.path_for_lib_file_cache.get(name) {
            return *cached;
        }

        let mut path = tspath::combine_paths(&self.default_library_path, &[name]);
        let mut replaced = false;
        if !self.opts.skip_module_resolution && self.opts.config.compiler_options().unwrap().lib_replacement.is_true() && name != "lib.d.ts" {
            let library_name = get_library_name_from_lib_file_name(name);
            let resolve_from =
                get_inferred_library_name_resolve_from(&self.opts.config.compiler_options().unwrap(), self.host.get_current_directory(), name);
            let (resolution, trace) = self.resolve_library(&library_name, &resolve_from);
            if let Some(resolution) = resolution.filter(|r| r.is_resolved()) {
                path = resolution.resolved_file_name.to_string();
                replaced = true;
            }
            let key = self.to_path(&resolve_from);
            self.path_for_lib_file_resolutions.entry(key).or_insert(libResolution { library_name, resolution, trace });
        }

        let lib_file = P::new(LibFile { name: name.to_string(), path, replaced });
        *self.path_for_lib_file_cache.entry(name.to_string()).or_insert(lib_file)
    }

    fn resolve_library(&mut self, library_name: &str, resolve_from: &str) -> (Option<P<ResolvedModule>>, Vec<DiagAndArgs>) {
        match self.resolver.resolve_module_name(library_name, resolve_from, ModuleKind::CommonJS, None) {
            Ok((resolved, trace)) => (Some(resolved), trace),
            Err(err) => {
                if self.module_resolution_error.is_none() {
                    self.module_resolution_error = Some(err);
                }
                (None, Vec::new())
            }
        }
    }

    pub(crate) fn add_sub_task(&mut self, t: TaskId, ref_: &resolvedRef, lib_file: Option<P<LibFile>>) {
        self.add_sub_task_normalized(t, ref_, lib_file, None);
    }

    // Appends a sub task the prefetch prepared (`prepare_sub_task`). A path the prefetch did not know may have got a task
    // from an earlier file of the round: share its strings then, as `add_sub_task_normalized` does.
    pub(crate) fn add_prepared_sub_task(&mut self, t: TaskId, prepared: preparedSubTask) {
        let mut sub_task = parseTask::with_path(prepared.normalized_file_path, prepared.path);
        sub_task.data_id = prepared.data_id;
        if sub_task.data_id == crate::filesparser::NO_DATA {
            if let Some((known_path, &data)) = self.files_parser.task_data_by_path.get_key_value(&sub_task.path) {
                sub_task.path = known_path.clone();
                if let Some((name, _)) = self.files_parser.datas[data].tasks.get_key_value(&sub_task.normalized_file_path) {
                    sub_task.normalized_file_path = Arc::clone(name);
                }
                sub_task.data_id = data as u32;
            }
        }
        sub_task.increase_depth = prepared.increase_depth;
        sub_task.elide_on_depth = prepared.elide_on_depth;
        sub_task.include_reason = Some(prepared.include_reason);
        sub_task.package_id = prepared.package_id;
        let id = self.new_task(sub_task);
        self.tasks[t].sub_tasks.push(id);
    }

    // `normalized` is the normalized file name and path of `ref_.file_name` when the caller already has them.
    pub(crate) fn add_sub_task_normalized(
        &mut self,
        t: TaskId,
        ref_: &resolvedRef,
        lib_file: Option<P<LibFile>>,
        normalized: Option<(String, Path)>,
    ) {
        let (normalized_file_path, mut path) = match normalized {
            Some((normalized_file_path, path)) => (normalized_file_path, path),
            None => (tspath::normalize_path(&ref_.file_name), Path::default()),
        };
        // A reference to a file that already has a task shares that task's strings (one copy per file name, as Go's
        // strings are shared); the values are equal either way.
        let mut shared_name: Option<std::sync::Arc<str>> = None;
        let mut data_id = crate::filesparser::NO_DATA;
        if let Some((known_path, &data)) = self.files_parser.task_data_by_path.get_key_value(&path) {
            path = known_path.clone();
            shared_name = self.files_parser.datas[data].tasks.get_key_value(&normalized_file_path).map(|(name, _)| Arc::clone(name));
            data_id = data as u32;
        }
        let mut sub_task = match shared_name {
            Some(name) => parseTask::with_path(name, path),
            None => parseTask::with_path(normalized_file_path, path),
        };
        sub_task.data_id = data_id;
        sub_task.lib_file = lib_file;
        sub_task.increase_depth = ref_.increase_depth;
        sub_task.elide_on_depth = ref_.elide_on_depth;
        sub_task.include_reason = Some(ref_.include_reason);
        sub_task.package_id = ref_.package_id;
        let id = self.new_task(sub_task);
        self.tasks[t].sub_tasks.push(id);
    }
}

// The part of resolveImportsAndModuleAugmentations that decides whether the file a resolved import names is added
// to the program, and whether that sub task is elided past maxNodeModuleJsDepth (`isJsFileFromNodeModules`).
// The speculative parse (filesParser::prefetch) asks the same question.
pub(crate) fn resolved_import_sub_task(
    project_references: &projectReferenceFileMapperBuilder,
    host: &dyn CompilerHost,
    options_for_file: &CompilerOptions,
    file: P<SourceFile>,
    module_name: &str,
    import_index: i32,
    resolved_module: &ResolvedModule,
) -> (bool, bool) {
    let imports = file.imports();
    let resolved_file_name = resolved_module.resolved_file_name;
    let is_from_node_modules_search = resolved_module.is_external_library_import;
    // Don't treat redirected files as JS files.
    let is_js_file = !resolved_module.resolved_using_extra_extensions
        && !tspath::file_extension_is_one_of(resolved_file_name, tspath::SUPPORTED_TS_EXTENSIONS_WITH_JSON_FLAT)
        && project_references
            .get_redirect_parsed_command_line_for_resolution(
                resolved_file_name,
                &tspath::to_path(resolved_file_name, host.get_current_directory(), host.fs().use_case_sensitive_file_names()),
            )
            .is_none();
    let is_js_file_from_node_modules = is_from_node_modules_search && is_js_file && resolved_file_name.contains("/node_modules/");

    // add file to program only if:
    // - resolution was successful
    // - noResolve is falsy
    // - module name comes from the list of imports
    // - it's not a top level JavaScript module that exceeded the search max

    let should_add_file = !module_name.is_empty()
        && module::get_resolution_diagnostic(options_for_file, resolved_module, file).is_none()
        && !options_for_file.no_resolve.is_true()
        && !(is_js_file && !options_for_file.get_allow_js())
        && (import_index < 0
            || ((import_index as usize) < imports.len()
                && (ast::is_in_js_file(imports[import_index as usize]) || !imports[import_index as usize].flags().intersects(NodeFlags::JSDoc))));
    (should_add_file, is_js_file_from_node_modules)
}

// fileLoader.getSourceFileFromReference; `is_referenced_file_reason` is what it asks of the include reason.
pub(crate) fn source_file_from_reference(
    options: &CompilerOptions,
    fs: &dyn tsrs_vfs::FS,
    supported_extensions: &[Vec<String>],
    supported_extensions_with_json: &[Vec<String>],
    file_name: &str,
    reference_text: &str,
    containing_file: &str,
    is_referenced_file_reason: bool,
) -> (String, Option<sourceFileFromReferenceDiagnostic>) {
    let allow_non_ts_extensions = options.allow_non_ts_extensions.is_true();
    let diagnostic_file_name = tspath::normalize_slashes(reference_text);
    if tspath::has_extension(file_name) {
        let canonical_file_name = tspath::get_canonical_file_name(file_name, fs.use_case_sensitive_file_names());
        if !allow_non_ts_extensions && !is_supported_extension(supported_extensions_with_json, &canonical_file_name) {
            if tspath::has_js_file_extension(&canonical_file_name) {
                return (
                    String::new(),
                    Some(sourceFileFromReferenceDiagnostic {
                        message: &diagnostics::File_0_is_a_JavaScript_file_Did_you_mean_to_enable_the_allowJs_option,
                        args: vec![diagnostic_file_name],
                    }),
                );
            }
            return (
                String::new(),
                Some(sourceFileFromReferenceDiagnostic {
                    message: &diagnostics::File_0_has_an_unsupported_extension_The_only_supported_extensions_are_1,
                    args: vec![diagnostic_file_name, supported_extensions_display(supported_extensions)],
                }),
            );
        }

        if !fs.file_exists(file_name) {
            return (
                String::new(),
                Some(sourceFileFromReferenceDiagnostic { message: &diagnostics::File_0_not_found, args: vec![diagnostic_file_name] }),
            );
        }

        if is_referenced_file_reason
            && tspath::get_canonical_file_name(containing_file, fs.use_case_sensitive_file_names()) == canonical_file_name
        {
            return (
                String::new(),
                Some(sourceFileFromReferenceDiagnostic { message: &diagnostics::A_file_cannot_have_a_reference_to_itself, args: vec![] }),
            );
        }
        return (file_name.to_string(), None);
    }

    if allow_non_ts_extensions && fs.file_exists(file_name) {
        return (file_name.to_string(), None);
    }

    if allow_non_ts_extensions {
        return (
            String::new(),
            Some(sourceFileFromReferenceDiagnostic { message: &diagnostics::File_0_not_found, args: vec![diagnostic_file_name] }),
        );
    }

    for ext in &supported_extensions[0] {
        let candidate = format!("{}{}", file_name, ext);
        if fs.file_exists(&candidate) {
            return (candidate, None);
        }
    }

    (
        String::new(),
        Some(sourceFileFromReferenceDiagnostic {
            message: &diagnostics::Could_not_resolve_the_path_0_with_the_extensions_Colon_1,
            args: vec![diagnostic_file_name, supported_extensions_display(supported_extensions)],
        }),
    )
}

pub(crate) fn is_supported_extension(supported_extensions_with_json: &[Vec<String>], canonical_file_name: &str) -> bool {
    supported_extensions_with_json.iter().any(|group| tspath::file_extension_is_one_of(canonical_file_name, &str_slice(group)))
}

fn supported_extensions_display(supported_extensions: &[Vec<String>]) -> String {
    let flat: Vec<&str> = supported_extensions.iter().flatten().map(|s| s.as_str()).collect();
    format!("'{}'", flat.join("', '"))
}

const EXTERNAL_HELPERS_MODULE_NAME_TEXT: &str = "tslib"; // TODO(jakebailey): dedupe

fn get_library_name_from_lib_file_name(lib_file_name: &str) -> String {
    // Support resolving to lib.dom.d.ts -> @typescript/lib-dom, and
    //                      lib.dom.iterable.d.ts -> @typescript/lib-dom/iterable
    //                      lib.es2015.symbol.wellknown.d.ts -> @typescript/lib-es2015/symbol-wellknown
    let components: Vec<&str> = lib_file_name.split('.').collect();
    let mut path = String::from("@typescript/lib-");
    if components.len() > 1 {
        path.push_str(components[1]);
    }
    let mut i = 2;
    while i < components.len() && !components[i].is_empty() && components[i] != "d" {
        if i == 2 {
            path.push('/');
        } else {
            path.push('-');
        }
        path.push_str(components[i]);
        i += 1;
    }
    path
}

fn get_inferred_library_name_resolve_from(options: &CompilerOptions, current_directory: &str, lib_file_name: &str) -> String {
    let containing_directory = if !options.config_file_path.is_empty() {
        tspath::get_directory_path(&options.config_file_path)
    } else {
        current_directory.to_string()
    };
    tspath::combine_paths(&containing_directory, &[&format!("__lib_node_modules_lookup_{}__.ts", lib_file_name)])
}

pub(crate) fn get_mode_for_type_reference_directive_in_file(
    ref_: P<FileReference>,
    file: P<SourceFile>,
    meta: &SourceFileMetaData,
    options: &CompilerOptions,
) -> ResolutionMode {
    if ref_.resolution_mode != ModuleKind::None {
        ref_.resolution_mode
    } else {
        get_default_resolution_mode_for_file(file.file_name(), meta, options)
    }
}

pub(crate) fn get_default_resolution_mode_for_file(file_name: &str, meta: &SourceFileMetaData, options: &CompilerOptions) -> ResolutionMode {
    if import_syntax_affects_module_resolution(options) {
        ast::get_implied_node_format_for_emit_worker(file_name, options.get_emit_module_kind(), meta)
    } else {
        ModuleKind::None
    }
}

pub(crate) fn get_mode_for_usage_location(
    file_name: &str,
    meta: &SourceFileMetaData,
    usage: P<Node>,
    options: Option<&CompilerOptions>,
) -> ResolutionMode {
    let parent = usage.parent().unwrap();
    if ast::is_import_declaration(parent)
        || parent.kind() == Kind::JSImportDeclaration
        || ast::is_export_declaration(parent)
        || ast::is_jsdoc_import_tag(parent)
    {
        let is_type_only = ast::is_exclusively_type_only_import_or_export(parent);
        if is_type_only {
            let attributes = match parent.kind() {
                Kind::ImportDeclaration | Kind::JSImportDeclaration => parent.as_import_declaration().attributes(),
                Kind::ExportDeclaration => parent.as_export_declaration().attributes,
                Kind::JSDocImportTag => parent.as_jsdoc_import_tag().attributes,
                _ => None,
            };
            if let Some(override_) = ast::get_resolution_mode_override(attributes, None) {
                return override_;
            }
        }
    }
    if ast::is_literal_type_node(parent) && ast::is_import_type_node(parent.parent().unwrap()) {
        if let Some(override_) = ast::get_resolution_mode_override(parent.parent().unwrap().as_import_type_node().attributes, None) {
            return override_;
        }
    }

    if let Some(options) = options {
        if import_syntax_affects_module_resolution(options) {
            return get_emit_syntax_for_usage_location_worker(file_name, meta, usage, options);
        }
    }

    ModuleKind::None
}

fn import_syntax_affects_module_resolution(options: &CompilerOptions) -> bool {
    let module_resolution = options.get_module_resolution_kind();
    ModuleResolutionKind::Node16 <= module_resolution && module_resolution <= ModuleResolutionKind::NodeNext
        || options.get_resolve_package_json_exports()
        || options.get_resolve_package_json_imports()
}

pub(crate) fn get_emit_syntax_for_usage_location_worker(
    file_name: &str,
    meta: &SourceFileMetaData,
    usage: P<Node>,
    options: &CompilerOptions,
) -> ResolutionMode {
    let parent = usage.parent().unwrap();
    if ast::is_require_call(parent, false /*requireStringLiteralLikeArgument*/)
        || ast::is_external_module_reference(parent) && ast::is_import_equals_declaration(parent.parent().unwrap())
    {
        return ModuleKind::CommonJS;
    }
    let file_emit_mode = ast::get_emit_module_format_of_file_worker(file_name, options, meta);
    if ast::walk_up_parenthesized_expressions(parent).is_some_and(ast::is_import_call) {
        return if ast::should_transform_import_call(file_name, options, file_emit_mode) {
            ModuleKind::CommonJS
        } else {
            ModuleKind::ESNext
        };
    }
    // If we're in --module preserve on an input file, we know that an import
    // is an import. But if this is a declaration file, we'd prefer to use the
    // impliedNodeFormat. Since we want things to be consistent between the two,
    // we need to issue errors when the user writes ESM syntax in a definitely-CJS
    // file, until/unless declaration emit can indicate a true ESM import. On the
    // other hand, writing CJS syntax in a definitely-ESM file is fine, since declaration
    // emit preserves the CJS syntax.
    if file_emit_mode == ModuleKind::CommonJS {
        return ModuleKind::CommonJS;
    } else if file_emit_mode.is_non_node_esm() || file_emit_mode == ModuleKind::Preserve {
        return ModuleKind::ESNext;
    }
    ModuleKind::None
}

pub(crate) fn str_slice(group: &[String]) -> Vec<&str> {
    group.iter().map(|s| s.as_str()).collect()
}
