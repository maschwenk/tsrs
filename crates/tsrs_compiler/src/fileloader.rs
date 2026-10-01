use std::sync::Mutex;

use rustc_hash::FxHashMap;
use tsrs_ast::{self as ast, FileReference, Kind, Node, NodeFactory, NodeFlags, SourceFile, SourceFileMetaData, SourceFileParseOptions, TokenFlags};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{alloc_str, CompilerOptions, ModuleKind, ModuleResolutionKind, ResolutionMode, P};
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_module::{self as module, DiagAndArgs, ModeAwareCache, ModeAwareCacheKey, ResolvedModule, ResolvedTypeReferenceDirective, Resolver};
use tsrs_tsoptions::{self as tsoptions, ParsedCommandLine};

use crate::file_include::{fileIncludeKind, automaticTypeDirectiveFileData, FileIncludeReason};
use crate::filesparser::{filesParser, parseTask, resolvedRef, TaskId};
use crate::host::CompilerHost;
use crate::processing_diagnostic::{includeExplainingDiagnostic, processingDiagnostic, processingDiagnosticKind};
use crate::program::{ProgramConfig, ProgramOptions};
use crate::projectreferencefilemapper::{projectReferenceFileMapper, projectReferenceFileMapperBuilder};
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

#[derive(Default)]
pub struct processedFiles {
    pub(crate) files: &'static [P<SourceFile>],
    pub(crate) files_by_path: FxHashMap<Path, P<SourceFile>>,
    pub(crate) project_reference_file_mapper: Option<projectReferenceFileMapper>,
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

pub(crate) fn process_all_program_files(opts: &ProgramOptions, single_threaded: bool) -> (processedFiles, P<module::ResolutionData>, Option<String>) {
    let compiler_options = opts.config.compiler_options().unwrap();
    let root_files = opts.config.file_names();
    let supported_extensions = tsoptions::get_supported_extensions(Some(&compiler_options), &[]);
    let supported_extensions_with_json_if_resolve_json_module =
        tsoptions::get_supported_extensions_with_json_if_resolve_json_module(Some(&compiler_options), &supported_extensions);
    let max_node_module_js_depth = compiler_options.max_node_module_js_depth.unwrap_or(0);
    let host = opts.host.clone();
    let project_references = projectReferenceFileMapperBuilder::new(&opts.program_config(), host.clone());
    let resolver_options = module::ResolverOptions {
        host: project_references.host,
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
        host: host.clone(),
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
    loader.add_project_reference_tasks(single_threaded);
    let roots_start = std::time::Instant::now();
    for (index, root_file) in root_files.iter().enumerate() {
        let mut reason = FileIncludeReason::new(fileIncludeKind::RootFile);
        reason.index = index;
        loader.add_root_file_task(root_file, None, P::new(reason));
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
    (processed, resolution_data, loader.module_resolution_error.take())
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

    fn add_root_file_task(&mut self, file_name: &str, lib_file: Option<P<LibFile>>, include_reason: P<FileIncludeReason>) {
        let curr_dir = self.host.get_current_directory().to_string();
        let abs_path = tspath::get_normalized_absolute_path(file_name, &curr_dir);
        let mut containing_file = curr_dir.clone();
        if let Some(config_file) = &self.opts.config.config_file {
            containing_file = tspath::get_normalized_absolute_path(config_file.source_file.file_name(), &curr_dir);
        }
        let (resolved_file, diagnostic) = self.get_source_file_from_reference(&abs_path, file_name, &containing_file, Some(include_reason));
        let mut root_task = parseTask::new(resolved_file);
        root_task.lib_file = lib_file;
        root_task.include_reason = Some(include_reason);
        if let Some(diagnostic) = diagnostic {
            root_task.normalized_file_path = abs_path;
            root_task.failed_lookup = true;
            root_task.processing_diagnostics = vec![processingDiagnostic::explaining(includeExplainingDiagnostic {
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
            module::get_automatic_type_directive_names(&self.opts.config.compiler_options().unwrap(), self.project_references.host);
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
                    Some(automaticTypeDirectiveFileData { type_reference: name.clone(), package_id: resolved.package_id });
                to_parse.push(resolvedRef {
                    file_name: resolved.resolved_file_name.to_string(),
                    increase_depth: resolved.is_external_library_import,
                    elide_on_depth: false,
                    include_reason: P::new(reason),
                    package_id: resolved.package_id,
                });
            } else {
                let mut reason = FileIncludeReason::new(fileIncludeKind::AutomaticTypeDirectiveFile);
                reason.automatic_type_directive =
                    Some(automaticTypeDirectiveFileData { type_reference: name.clone(), package_id: Default::default() });
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

    fn add_project_reference_tasks(&mut self, _single_threaded: bool) {
        // Project references are not supported yet: ResolvedProjectReferencePaths is never consulted,
        // so the mapper behaves as if the config had no references.
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
        if self.opts.skip_module_resolution {
            return SourceFileMetaData {
                implied_node_format: ast::get_implied_node_format_for_file(file_name, ""),
                ..Default::default()
            };
        }

        let package_json_scope = self
            .resolver
            .get_resolution_data()
            .get()
            .new_resolver(self.project_references.host)
            .get_package_scope_for_path(&tspath::get_directory_path(file_name));
        let module_resolution_kind = self.opts.config.compiler_options().unwrap().get_module_resolution_kind();

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
                    package_json_type = value.to_string();
                }
            }
        }

        let implied_node_format = ast::get_implied_node_format_for_file(file_name, &package_json_type);
        SourceFileMetaData { package_json_type, package_json_directory, implied_node_format }
    }

    pub(crate) fn parse_options_for_task(&self, t: TaskId) -> SourceFileParseOptions {
        let task = &self.tasks[t];
        let path = self.to_path(&task.normalized_file_path);
        let options = self.project_references.get_compiler_options_for_file(&task.normalized_file_path, &path);
        SourceFileParseOptions {
            file_name: task.normalized_file_path.clone(),
            path,
            external_module_indicator_options: ast::get_external_module_indicator_options(
                &task.normalized_file_path,
                &options,
                &task.metadata,
            ),
        }
    }

    pub(crate) fn parse_source_file(&self, t: TaskId) -> Option<P<SourceFile>> {
        self.host.get_source_file(self.parse_options_for_task(t))
    }

    pub(crate) fn is_supported_extension(&self, canonical_file_name: &str) -> bool {
        self.supported_extensions_with_json_if_resolve_json_module
            .iter()
            .any(|group| tspath::file_extension_is_one_of(canonical_file_name, &str_slice(group)))
    }

    fn supported_extensions_display(&self) -> String {
        let flat: Vec<&str> = self.supported_extensions.iter().flatten().map(|s| s.as_str()).collect();
        format!("'{}'", flat.join("', '"))
    }

    pub(crate) fn get_source_file_from_reference(
        &self,
        file_name: &str,
        reference_text: &str,
        containing_file: &str,
        include_reason: Option<P<FileIncludeReason>>,
    ) -> (String, Option<sourceFileFromReferenceDiagnostic>) {
        let options = self.opts.config.compiler_options().unwrap();
        let allow_non_ts_extensions = options.allow_non_ts_extensions.is_true();
        let diagnostic_file_name = tspath::normalize_slashes(reference_text);
        let fs = self.host.fs();

        if tspath::has_extension(file_name) {
            let canonical_file_name = tspath::get_canonical_file_name(file_name, fs.use_case_sensitive_file_names());
            if !allow_non_ts_extensions && !self.is_supported_extension(&canonical_file_name) {
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
                        args: vec![diagnostic_file_name, self.supported_extensions_display()],
                    }),
                );
            }

            if !fs.file_exists(file_name) {
                return (
                    String::new(),
                    Some(sourceFileFromReferenceDiagnostic { message: &diagnostics::File_0_not_found, args: vec![diagnostic_file_name] }),
                );
            }

            if include_reason.is_some_and(|r| r.is_referenced_file())
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

        for ext in &self.supported_extensions[0] {
            let candidate = format!("{}{}", file_name, ext);
            if fs.file_exists(&candidate) {
                return (candidate, None);
            }
        }

        (
            String::new(),
            Some(sourceFileFromReferenceDiagnostic {
                message: &diagnostics::Could_not_resolve_the_path_0_with_the_extensions_Colon_1,
                args: vec![diagnostic_file_name, self.supported_extensions_display()],
            }),
        )
    }

    pub(crate) fn resolve_tripleslash_path_reference(
        &self,
        module_name: &str,
        containing_file: &str,
        index: usize,
    ) -> Result<resolvedRef, processingDiagnostic> {
        let base_path = tspath::get_directory_path(containing_file);
        let mut referenced_file_name = module_name.to_string();

        if !tspath::is_rooted_disk_path(module_name) {
            referenced_file_name = tspath::combine_paths(&base_path, &[module_name]);
        }
        let normalized_file_name = tspath::normalize_path(&referenced_file_name);
        let include_reason =
            FileIncludeReason::new_referenced(fileIncludeKind::ReferenceFile, self.to_path(containing_file), index as i32, None);

        let (resolved_file_name, diagnostic) =
            self.get_source_file_from_reference(&normalized_file_name, module_name, containing_file, Some(include_reason));
        if let Some(diagnostic) = diagnostic {
            return Err(processingDiagnostic::explaining(includeExplainingDiagnostic {
                file: None,
                diagnostic_reason: Some(include_reason),
                message: diagnostic.message,
                args: diagnostic.args,
            }));
        }

        Ok(resolvedRef {
            file_name: resolved_file_name,
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
        let meta = self.tasks[t].metadata.clone();
        let task_path = self.tasks[t].path.clone();

        let mut type_resolutions_in_file = ModeAwareCache::default();
        let mut type_resolutions_trace = Vec::new();
        for (index, ref_) in type_reference_directives.iter().enumerate() {
            let (redirect, file_name) = self.project_references.get_redirect_for_resolution(file.file_name(), &file.path());
            let resolution_mode = get_mode_for_type_reference_directive_in_file(
                *ref_,
                file,
                &meta,
                &module::get_compiler_options_with_redirect(self.opts.config.compiler_options().unwrap(), redirect),
            );
            let (resolved, trace) = self.resolver.resolve_type_reference_directive(&ref_.file_name, &file_name, resolution_mode, redirect);
            type_resolutions_in_file.insert(ModeAwareCacheKey { name: alloc_str(&ref_.file_name), mode: resolution_mode }, resolved);
            let include_reason =
                FileIncludeReason::new_referenced(fileIncludeKind::TypeReferenceDirective, task_path.clone(), index as i32, None);
            type_resolutions_trace.extend(trace);

            if resolved.is_resolved() {
                self.add_sub_task(
                    t,
                    resolvedRef {
                        file_name: resolved.resolved_file_name.to_string(),
                        increase_depth: resolved.is_external_library_import,
                        elide_on_depth: false,
                        include_reason,
                        package_id: resolved.package_id,
                    },
                    None,
                );
            } else {
                self.tasks[t].processing_diagnostics.push(processingDiagnostic::unknown_reference(include_reason));
            }
        }

        self.tasks[t].type_resolutions_in_file = type_resolutions_in_file;
        self.tasks[t].type_resolutions_trace = type_resolutions_trace;
    }

    pub(crate) fn resolve_imports_and_module_augmentations(&mut self, t: TaskId) {
        let start = std::time::Instant::now();
        self.resolve_imports_and_module_augmentations_worker(t);
        tsrs_core::phases::record("Program:   module resolution", start.elapsed());
    }

    fn resolve_imports_and_module_augmentations_worker(&mut self, t: TaskId) {
        let file = self.tasks[t].file.unwrap();
        let meta = self.tasks[t].metadata.clone();
        let task_path = self.tasks[t].path.clone();

        let imports = file.imports();
        let mut module_names: Vec<P<Node>> = Vec::with_capacity(imports.len() + file.module_augmentations.get().len() + 2);

        let is_java_script_file = ast::is_source_file_js(file);
        let is_external_module_file = ast::is_external_module(file);

        let (redirect, file_name) = self.project_references.get_redirect_for_resolution(file.file_name(), &file.path());
        let options_for_file = module::get_compiler_options_with_redirect(self.opts.config.compiler_options().unwrap(), redirect);
        if is_java_script_file || (!file.is_declaration_file.get() && (options_for_file.get_isolated_modules() || is_external_module_file)) {
            if options_for_file.import_helpers.is_true() {
                let specifier = self.create_synthetic_import(EXTERNAL_HELPERS_MODULE_NAME_TEXT, file);
                module_names.push(specifier);
                self.tasks[t].import_helpers_import_specifier = Some(specifier);
            }
        }

        if is_java_script_file || file.script_kind.get() == tsrs_core::ScriptKind::TSX {
            let jsx_import = ast::get_jsx_runtime_import(&ast::get_jsx_implicit_import_base(&options_for_file, Some(file)), &options_for_file);
            if !jsx_import.is_empty() {
                let specifier = self.create_synthetic_import(&jsx_import, file);
                module_names.push(specifier);
                self.tasks[t].jsx_runtime_import_specifier =
                    Some(P::new(jsxRuntimeImportSpecifier { module_reference: jsx_import.to_string(), specifier }));
            }
        }

        let imports_start = module_names.len() as i32;

        module_names.extend_from_slice(imports);
        for &imp in file.module_augmentations.get() {
            if imp.kind == Kind::StringLiteral {
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

            for (index, &entry) in module_names.iter().enumerate() {
                let module_name = entry.text();
                if module_name.is_empty() {
                    continue;
                }

                let mode = get_mode_for_usage_location(file.file_name(), &meta, entry, Some(&options_for_file));
                let (resolved_module, trace) = match self.resolver.resolve_module_name(module_name, &file_name, mode, redirect) {
                    Ok((resolved_module, trace)) => (resolved_module, trace),
                    Err(err) => {
                        if self.module_resolution_error.is_none() {
                            self.module_resolution_error = Some(err);
                        }
                        (P::new(ResolvedModule::default()), Vec::new())
                    }
                };
                resolutions_in_file.insert(ModeAwareCacheKey { name: module_name, mode }, resolved_module);
                resolutions_trace.extend(trace);

                if !resolved_module.is_resolved() {
                    continue;
                }

                let resolved_file_name = resolved_module.resolved_file_name;
                let is_from_node_modules_search = resolved_module.is_external_library_import;
                // Don't treat redirected files as JS files.
                let is_js_file = !resolved_module.resolved_using_extra_extensions
                    && !tspath::file_extension_is_one_of(resolved_file_name, tspath::SUPPORTED_TS_EXTENSIONS_WITH_JSON_FLAT)
                    && self
                        .project_references
                        .get_redirect_parsed_command_line_for_resolution(resolved_file_name, &self.to_path(resolved_file_name))
                        .is_none();
                let is_js_file_from_node_modules = is_from_node_modules_search && is_js_file && resolved_file_name.contains("/node_modules/");

                // add file to program only if:
                // - resolution was successful
                // - noResolve is falsy
                // - module name comes from the list of imports
                // - it's not a top level JavaScript module that exceeded the search max

                let import_index = index as i32 - imports_start;

                let should_add_file = !module_name.is_empty()
                    && module::get_resolution_diagnostic(&options_for_file, &resolved_module, file).is_none()
                    && !options_for_file.no_resolve.is_true()
                    && !(is_js_file && !options_for_file.get_allow_js())
                    && (import_index < 0
                        || ((import_index as usize) < imports.len()
                            && (ast::is_in_js_file(imports[import_index as usize])
                                || !imports[import_index as usize].flags().intersects(NodeFlags::JSDoc))));

                if should_add_file {
                    let include_reason = FileIncludeReason::new_referenced(
                        fileIncludeKind::Import,
                        task_path.clone(),
                        import_index,
                        if import_index < 0 { Some(entry) } else { None },
                    );
                    self.add_sub_task(
                        t,
                        resolvedRef {
                            file_name: resolved_file_name.to_string(),
                            increase_depth: resolved_module.is_external_library_import,
                            elide_on_depth: is_js_file_from_node_modules,
                            include_reason,
                            package_id: resolved_module.package_id,
                        },
                        None,
                    );
                }
            }

            self.tasks[t].resolutions_in_file = resolutions_in_file;
            self.tasks[t].resolutions_trace = resolutions_trace;
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

    pub(crate) fn add_sub_task(&mut self, t: TaskId, ref_: resolvedRef, lib_file: Option<P<LibFile>>) {
        let normalized_file_path = tspath::normalize_path(&ref_.file_name);
        let mut sub_task = parseTask::new(normalized_file_path);
        sub_task.lib_file = lib_file;
        sub_task.increase_depth = ref_.increase_depth;
        sub_task.elide_on_depth = ref_.elide_on_depth;
        sub_task.include_reason = Some(ref_.include_reason);
        sub_task.package_id = ref_.package_id;
        let id = self.new_task(sub_task);
        self.tasks[t].sub_tasks.push(id);
    }
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
        || parent.kind == Kind::JSImportDeclaration
        || ast::is_export_declaration(parent)
        || ast::is_jsdoc_import_tag(parent)
    {
        let is_type_only = ast::is_exclusively_type_only_import_or_export(parent);
        if is_type_only {
            let attributes = match parent.kind {
                Kind::ImportDeclaration | Kind::JSImportDeclaration => parent.as_import_declaration().attributes,
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
