use std::fmt::Display;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{
    self as ast, compare_diagnostics, equal_diagnostics, equal_diagnostics_no_related_info, new_compiler_diagnostic, new_diagnostic,
    CommentDirectiveKind, Diagnostic, DiagnosticExt, FileReference, Kind, Node, SourceFile, SourceFileMetaData,
};
use crate::checkerpool::{Checker, CheckerHandle, CheckerPool, Context};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{CompilerOptions, JsxEmit, ModuleKind, ModuleResolutionKind, ResolutionMode, ScriptKind, ScriptTarget, Tristate, P};
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_module::{self as module, ModeAwareCache, ModeAwareCacheKey, ResolvedModule, ResolvedTypeReferenceDirective, Resolver, ResolverOptions};
use tsrs_tsoptions::{self as tsoptions, ParsedCommandLine};

use crate::checkerpool::checkerPool;
use crate::emitter::source_file_may_be_emitted;
use crate::file_include::{self, FileIncludeReason};
use crate::fileloader::{
    self, get_default_resolution_mode_for_file, get_emit_syntax_for_usage_location_worker, get_mode_for_usage_location,
    process_all_program_files, processedFiles, DuplicateSourceFile, LibFile,
};
use tsrs_core::collections::Set;
use crate::host::CompilerHost;
use crate::includeprocessor::includeProcessor;
use crate::outputpaths;
use crate::processing_diagnostic::{includeExplainingDiagnostic, processingDiagnostic};
use crate::projectreferencefilemapper::projectReferenceFileMapper;
use tsrs_tsoptions::SourceOutputAndProjectReference;
use crate::fileloader::str_slice;
use tsrs_module::symlinks::{self, KnownSymlinks};
use tsrs_module::{ResolutionHost, ResolvedProjectReference};

pub type CreateModuleResolver = Arc<dyn Fn(ResolverOptions) -> Box<dyn Resolver> + Send + Sync>;

// Go `ProgramFactories.CreateCheckerPool func(*Program) CheckerPool`.
pub type CreateCheckerPool = Arc<dyn Fn(&'static Program) -> Box<dyn CheckerPool> + Send + Sync>;

// Go `ProgramOptions` (= ProgramConfig + ProgramHosts + ProgramFactories, flattened; tracing is not ported).
pub struct ProgramOptions {
    pub config: P<ParsedCommandLine>,
    pub use_source_of_project_reference: bool,
    pub single_threaded: Tristate,
    pub typings_location: String,
    pub project_name: String,
    // SkipModuleResolution avoids all module and type reference resolution while
    // still collecting import metadata needed for emit.
    pub skip_module_resolution: bool,
    pub host: Arc<dyn CompilerHost>,
    pub create_checker_pool: Option<CreateCheckerPool>,
    pub create_module_resolver: Option<CreateModuleResolver>,
}

impl ProgramOptions {
    pub fn new(config: P<ParsedCommandLine>, host: Arc<dyn CompilerHost>) -> ProgramOptions {
        ProgramOptions {
            config,
            use_source_of_project_reference: false,
            single_threaded: Tristate::Unknown,
            typings_location: String::new(),
            project_name: String::new(),
            skip_module_resolution: false,
            host,
            create_checker_pool: None,
            create_module_resolver: None,
        }
    }

    // Go `ProgramOptions{ProgramConfig: config, Host: host, CreateCheckerPool: ..., CreateModuleResolver: ...}`.
    pub fn from_config(
        config: ProgramConfig,
        host: Arc<dyn CompilerHost>,
        create_checker_pool: Option<CreateCheckerPool>,
        create_module_resolver: Option<CreateModuleResolver>,
    ) -> ProgramOptions {
        ProgramOptions {
            config: config.config,
            use_source_of_project_reference: config.use_source_of_project_reference,
            single_threaded: config.single_threaded,
            typings_location: config.typings_location,
            project_name: config.project_name,
            skip_module_resolution: config.skip_module_resolution,
            host,
            create_checker_pool,
            create_module_resolver,
        }
    }

    pub(crate) fn program_config(&self) -> ProgramConfig {
        ProgramConfig {
            config: self.config,
            use_source_of_project_reference: self.use_source_of_project_reference,
            single_threaded: self.single_threaded,
            typings_location: self.typings_location.clone(),
            project_name: self.project_name.clone(),
            skip_module_resolution: self.skip_module_resolution,
        }
    }
}

#[derive(Clone)]
pub struct ProgramConfig {
    pub config: P<ParsedCommandLine>,
    pub use_source_of_project_reference: bool,
    pub single_threaded: Tristate,
    pub typings_location: String,
    pub project_name: String,
    pub skip_module_resolution: bool,
}

impl ProgramConfig {
    // program.go:64
    pub(crate) fn can_use_project_reference_source(&self) -> bool {
        self.use_source_of_project_reference && !self.config.compiler_options().unwrap().disable_source_of_project_reference_redirect.is_true()
    }
}

// program.go:91
pub(crate) struct packageNamesInfo {
    resolved: Set<String>,
    unresolved: Set<String>,
    deep_import_packages: Set<String>,
}

// Go `checkerPool CheckerPool` + `compilerCheckerPool *checkerPool`: the built-in pool is set only when
// `CreateCheckerPool` was not provided; it enables grouped parallel iteration, non-exclusive access for emit,
// and direct global diagnostics collection.
enum programCheckerPool {
    Compiler(checkerPool),
    External(Box<dyn CheckerPool>),
}

pub struct Program {
    pub(crate) opts: ProgramConfig,
    host: Arc<dyn CompilerHost>,
    resolution_host: &'static dyn ResolutionHost,
    pub(crate) resolution_data: P<module::ResolutionData>,
    // Always set once the program is constructed (see `init_checker_pool`).
    checker_pool: OnceLock<programCheckerPool>,
    pub(crate) include_processor: includeProcessor,
    module_resolution_error: Option<String>,

    pub(crate) compare_paths_options: ComparePathsOptions,

    // Go's embedded `processedFiles`: `files` and `filesByPath` are per program (ReuseProgram replaces one file);
    // the rest is shared by the programs ReuseProgram derives from this one, as Go's shallow struct copy shares it.
    pub(crate) files: &'static [P<SourceFile>],
    pub(crate) files_by_path: FxHashMap<Path, P<SourceFile>>,
    processed: &'static processedFiles,
    project_reference_file_mapper: &'static projectReferenceFileMapper,
    processing_diagnostics: Mutex<Vec<processingDiagnostic>>,

    uses_uri_style_node_core_modules: Tristate,

    common_source_directory: OnceLock<String>,

    declaration_diagnostic_cache: Mutex<FxHashMap<P<SourceFile>, Vec<P<Diagnostic>>>>,

    program_diagnostics: Vec<P<Diagnostic>>,
    has_emit_blocking_diagnostics: FxHashSet<Path>,

    // Cached unresolved imports for ATA
    unresolved_imports: OnceLock<Arc<Set<String>>>,
    known_symlinks: OnceLock<P<KnownSymlinks>>,

    // Used by auto-imports
    package_names: OnceLock<Arc<packageNamesInfo>>,

    // Used by workspace/symbol
    has_ts_file: OnceLock<bool>,

    // Cached map of package names to whether they bundle types
    packages_map: OnceLock<FxHashMap<String, bool>>,
}

impl std::ops::Deref for Program {
    type Target = processedFiles;
    fn deref(&self) -> &processedFiles {
        self.processed
    }
}

impl Program {
    pub fn file_exists(&self, path: &str) -> bool {
        self.host.fs().file_exists(path)
    }

    pub fn get_current_directory(&self) -> &str {
        self.host.get_current_directory()
    }

    pub fn get_global_typings_cache_location(&self) -> &str {
        &self.opts.typings_location
    }

    pub fn get_nearest_ancestor_directory_with_package_json(&self, dirname: &str) -> String {
        match self.new_resolver().get_package_scope_for_path(dirname) {
            Some(scoped) if scoped.exists() => scoped.package_directory.to_string(),
            _ => String::new(),
        }
    }

    pub fn get_package_json_info(&self, pkg_json_path: &str) -> Option<P<tsrs_module::packagejson::InfoCacheEntry>> {
        let directory = tspath::get_directory_path(pkg_json_path);
        match self.new_resolver().get_package_scope_for_path(&directory) {
            Some(scoped) if scoped.exists() && scoped.package_directory == directory => Some(scoped),
            _ => None,
        }
    }

    fn new_resolver(&self) -> module::DefaultResolver {
        self.resolution_data.get().new_resolver(self.project_reference_file_mapper.resolution_host(self.resolution_host))
    }

    // GetRedirectTargets returns the list of file paths that redirect to the given path.
    // These are files from the same package (same name@version) installed in different locations.
    pub fn get_redirect_targets(&self, path: &Path) -> &[String] {
        self.redirect_targets_map.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }

    // gets the original file that was included in program
    // this returns original source file name when including output of project reference
    // otherwise same name
    pub fn get_source_of_project_reference_if_output_included(&self, file_name: &str, path: &Path) -> String {
        if let Some(source) = self.output_file_to_project_reference_source.get(path) {
            return source.clone();
        }
        file_name.to_string()
    }

    // program.go:209
    pub fn get_project_reference_from_source(&self, path: &Path) -> Option<P<SourceOutputAndProjectReference>> {
        self.project_reference_file_mapper.get_project_reference_from_source(path)
    }

    // program.go:214
    pub fn is_source_from_project_reference(&self, path: &Path) -> bool {
        self.project_reference_file_mapper.is_source_from_project_reference(path)
    }

    // program.go:218
    pub fn get_project_reference_from_output_dts(&self, path: &Path) -> Option<P<SourceOutputAndProjectReference>> {
        self.project_reference_file_mapper.get_project_reference_from_output_dts(path)
    }

    // program.go:222
    pub fn get_resolved_project_reference_for(&self, path: &Path) -> (Option<P<ParsedCommandLine>>, bool) {
        self.project_reference_file_mapper.get_resolved_reference_for(path)
    }

    // program.go:226
    pub fn get_redirect_for_resolution(&self, file: P<SourceFile>) -> Option<P<ParsedCommandLine>> {
        self.project_reference_file_mapper.get_redirect_for_resolution(file.file_name(), &file.path()).0
    }

    // program.go:231
    pub fn get_parse_file_redirect(&self, file_name: &str) -> String {
        self.project_reference_file_mapper.get_parse_file_redirect(file_name, &self.to_path(file_name))
    }

    // program.go:235
    pub fn get_resolved_project_references(&self) -> Vec<Option<P<ParsedCommandLine>>> {
        self.project_reference_file_mapper.get_resolved_project_references()
    }

    // program.go:239
    pub fn range_resolved_project_reference(
        &self,
        f: impl FnMut(&Path, Option<P<ParsedCommandLine>>, P<ParsedCommandLine>, usize) -> bool,
    ) -> bool {
        self.project_reference_file_mapper.range_resolved_project_reference(f)
    }

    // program.go:243
    pub fn range_resolved_project_reference_in_child_config(
        &self,
        child_config: Option<P<ParsedCommandLine>>,
        f: impl FnMut(&Path, Option<P<ParsedCommandLine>>, P<ParsedCommandLine>, usize) -> bool,
    ) -> bool {
        self.project_reference_file_mapper.range_resolved_project_reference_in_child_config(child_config, f)
    }

    // program.go:183
    pub fn package_json_cache_entries(&self, f: impl FnMut(&Path, &P<tsrs_module::packagejson::InfoCacheEntry>) -> bool) {
        self.resolution_data.package_json_cache_entries(f)
    }

    pub fn use_case_sensitive_file_names(&self) -> bool {
        self.host.fs().use_case_sensitive_file_names()
    }

    pub fn uses_uri_style_node_core_modules(&self) -> Tristate {
        self.uses_uri_style_node_core_modules
    }

    /** This should have similar behavior to 'processSourceFile' without diagnostics or mutation. */
    pub fn get_source_file_from_reference(&self, origin: P<SourceFile>, ref_: P<FileReference>) -> Option<P<SourceFile>> {
        let file_name = tspath::resolve_path(&tspath::get_directory_path(origin.file_name()), &[&ref_.file_name]);
        let supported_extensions_base = tsoptions::get_supported_extensions(Some(&self.options()), &[]);
        let supported_extensions = tsoptions::get_supported_extensions_with_json_if_resolve_json_module(Some(&self.options()), &supported_extensions_base);
        let allow_non_ts_extensions = self.options().allow_non_ts_extensions.is_true();
        if tspath::has_extension(&file_name) {
            if !allow_non_ts_extensions {
                let canonical_file_name = tspath::get_canonical_file_name(&file_name, self.use_case_sensitive_file_names());
                let supported = supported_extensions.iter().any(|group| tspath::file_extension_is_one_of(&canonical_file_name, &str_slice(group)));
                if !supported {
                    return None; // unsupported extensions are forced to fail
                }
            }

            return self.get_source_file_for_resolved_module(&file_name);
        }
        if allow_non_ts_extensions {
            if let Some(extensionless) = self.get_source_file_for_resolved_module(&file_name) {
                return Some(extensionless);
            }
        }

        // Only try adding extensions from the first supported group (which should be .ts/.tsx/.d.ts)
        for ext in &supported_extensions[0] {
            if let Some(result) = self.get_source_file_for_resolved_module(&format!("{}{}", file_name, ext)) {
                return Some(result);
            }
        }
        None
    }
}

// Parsing and binding recurse deeply on large files; Go's goroutine stacks grow on demand, so the
// worker threads get large stacks.
pub(crate) fn worker_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| rayon::ThreadPoolBuilder::new().stack_size(256 << 20).build().unwrap())
}

// program.go:305
pub fn new_program(opts: ProgramOptions) -> &'static Program {
    let single_threaded = opts.single_threaded.default_if_unknown(opts.config.compiler_options().unwrap().single_threaded).is_true();
    let (mut processed, resolution_data, module_resolution_error) = process_all_program_files(&opts, single_threaded);
    let project_reference_file_mapper: &'static projectReferenceFileMapper = processed.project_reference_file_mapper.take().unwrap();
    let processing_diagnostics = std::mem::take(&mut processed.file_include_data.processing_diagnostics);
    let files = std::mem::take(&mut processed.files);
    let files_by_path = std::mem::take(&mut processed.files_by_path);
    let host = opts.host.clone();
    let mut p = Program {
        opts: opts.program_config(),
        compare_paths_options: ComparePathsOptions {
            use_case_sensitive_file_names: host.fs().use_case_sensitive_file_names(),
            current_directory: host.get_current_directory().to_string(),
        },
        resolution_host: crate::projectreferencefilemapper::resolution_host_for(host.clone()),
        host,
        resolution_data,
        checker_pool: OnceLock::new(),
        include_processor: includeProcessor::default(),
        module_resolution_error,
        files,
        files_by_path,
        processed: Box::leak(Box::new(processed)),
        project_reference_file_mapper,
        processing_diagnostics: Mutex::new(processing_diagnostics),
        uses_uri_style_node_core_modules: Tristate::Unknown,
        common_source_directory: OnceLock::new(),
        declaration_diagnostic_cache: Mutex::new(FxHashMap::default()),
        program_diagnostics: Vec::new(),
        has_emit_blocking_diagnostics: FxHashSet::default(),
        unresolved_imports: OnceLock::new(),
        known_symlinks: OnceLock::new(),
        package_names: OnceLock::new(),
        has_ts_file: OnceLock::new(),
        packages_map: OnceLock::new(),
    };
    // Go initializes the checker pool before verifying options; the pool factory takes the program by
    // `&'static`, so here it runs after verification, once the program is leaked. Neither pool reads anything
    // verification writes (checkers are created lazily).
    tsrs_core::phases::time("Program: verify options", || p.verify_compiler_options());
    let p: &'static Program = Box::leak(Box::new(p));
    p.init_checker_pool(opts.create_checker_pool.as_ref());
    p
}

// Frees a program made by `new_program` or `update_program` (language server; Go's GC). The caller guarantees that
// nothing uses it any more: no checker of its pool is held, and no snapshot or language service refers to it. What
// it shares with other versions (`processed`, the project reference file mapper) stays.
//
// # Safety
// `program` came from `new_program` / `update_program` and is not used afterwards.
pub unsafe fn free_program(program: &'static Program) {
    let resolution_host: *const dyn ResolutionHost = program.resolution_host;
    drop(Box::from_raw(program as *const Program as *mut Program));
    // Per program (`resolution_host_for`); it keeps the compiler host alive.
    drop(Box::from_raw(resolution_host as *mut dyn ResolutionHost));
}

impl Program {
    // program.go:323
    // Return an updated program for which it is known that only the file with the given path has changed.
    // In addition to a new program, return a boolean indicating whether the data of the old program was reused.
    // The returned *ast.SourceFile is the changed file as acquired through newHost; it is nil
    // only if the host cannot locate the file (e.g. it was deleted). Callers that manage
    // host-side parse caches must release this exact pointer when the old program could not be
    // reused, since it was acquired speculatively before that decision was made.
    pub fn update_program(
        &'static self,
        changed_file_path: &Path,
        new_host: Arc<dyn CompilerHost>,
        create_checker_pool: Option<CreateCheckerPool>,
        create_module_resolver: Option<CreateModuleResolver>,
    ) -> (&'static Program, Option<P<SourceFile>>, bool) {
        let (result, new_file, reused) = self.reuse_program(changed_file_path, new_host.clone(), create_checker_pool.clone(), create_module_resolver.clone());
        if reused {
            (result.unwrap(), new_file, true)
        } else {
            (new_program(ProgramOptions::from_config(self.opts.clone(), new_host, create_checker_pool, create_module_resolver)), new_file, false)
        }
    }

    // program.go:347
    // ReuseProgram attempts to produce a new program by replacing only
    // changedFilePath in place, reusing the rest of p. It returns
    // (newProgram, newFile, true) on success, or (nil, newFile, false) when the
    // file cannot be replaced in place. Unlike UpdateProgram, it never constructs a
    // full fallback program, so callers that build their own fallback (e.g. with a
    // different host) do not pay for a discarded program build.
    pub fn reuse_program(
        &'static self,
        changed_file_path: &Path,
        new_host: Arc<dyn CompilerHost>,
        create_checker_pool: Option<CreateCheckerPool>,
        _create_module_resolver: Option<CreateModuleResolver>,
    ) -> (Option<&'static Program>, Option<P<SourceFile>>, bool) {
        let old_file = self.files_by_path[changed_file_path];
        // Content mappers are not ported: no file is content-mapped (`oldFile.ContentMapper() == ""`), so the
        // supplemental file lists are always empty.
        let old_supplemental_files: &[P<SourceFile>] = &[];
        let new_supplemental_files: &[P<SourceFile>] = &[];
        let new_file = new_host.get_source_file(old_file.parse_options().clone());

        // If this file is part of a package redirect group (same package installed in multiple
        // node_modules locations), we need to rebuild the program because the redirect targets
        // might need recalculation.
        let in_redirect_files = self.redirect_files_by_path.contains_key(changed_file_path);
        let is_redirect_target = self.redirect_targets_map.contains_key(changed_file_path);
        if in_redirect_files || is_redirect_target {
            return (None, new_file, false);
        }

        if self.module_resolution_error.is_some() || !self.can_replace_file_in_program(old_file, new_file) {
            return (None, new_file, false);
        }
        let new_file_some = new_file.unwrap();
        // Cloning does not recompute synthetic helper or JSX-runtime import bookkeeping. Fall back to a full
        // build whenever either version requires those imports.
        if self.import_helpers_import_specifiers.contains_key(old_file.path()) || self.needs_import_helpers_import_specifier(new_file_some) {
            return (None, new_file, false);
        }
        if self.jsx_runtime_import_specifiers.contains_key(old_file.path()) || !self.jsx_runtime_import_specifier(new_file_some).is_empty() {
            return (None, new_file, false);
        }
        if old_supplemental_files.len() != new_supplemental_files.len() {
            return (None, new_file, false);
        }
        for (i, &old_supplemental) in old_supplemental_files.iter().enumerate() {
            let new_supplemental = new_supplemental_files[i];
            if old_supplemental.path() != new_supplemental.path() || !self.can_replace_file_in_program(old_supplemental, Some(new_supplemental)) {
                return (None, new_file, false);
            }
            if self.import_helpers_import_specifiers.contains_key(old_supplemental.path()) || self.needs_import_helpers_import_specifier(new_supplemental) {
                return (None, new_file, false);
            }
            if self.jsx_runtime_import_specifiers.contains_key(old_supplemental.path()) || !self.jsx_runtime_import_specifier(new_supplemental).is_empty() {
                return (None, new_file, false);
            }
        }
        // TODO: reverify compiler options when config has changed?
        let mut result = Program {
            opts: self.opts.clone(),
            resolution_host: crate::projectreferencefilemapper::resolution_host_for(new_host.clone()),
            host: new_host,
            resolution_data: self.resolution_data.clone_data(),
            checker_pool: OnceLock::new(),
            include_processor: includeProcessor::default(),
            module_resolution_error: None,
            compare_paths_options: self.compare_paths_options.clone(),
            files: self.files,
            files_by_path: FxHashMap::default(),
            processed: self.processed,
            project_reference_file_mapper: self.project_reference_file_mapper,
            processing_diagnostics: Mutex::new(self.processing_diagnostics().clone()),
            uses_uri_style_node_core_modules: self.uses_uri_style_node_core_modules,
            common_source_directory: OnceLock::new(),
            declaration_diagnostic_cache: Mutex::new(FxHashMap::default()),
            program_diagnostics: self.program_diagnostics.clone(),
            has_emit_blocking_diagnostics: self.has_emit_blocking_diagnostics.clone(),
            unresolved_imports: OnceLock::new(),
            known_symlinks: OnceLock::new(),
            package_names: OnceLock::new(),
            has_ts_file: OnceLock::new(),
            packages_map: OnceLock::new(),
        };
        try_reuse(&result.unresolved_imports, &self.unresolved_imports);
        try_reuse(&result.known_symlinks, &self.known_symlinks);
        try_reuse(&result.package_names, &self.package_names);
        let index = result.files.iter().position(|file| file.path() == new_file_some.path()).unwrap();
        let mut files = result.files.to_vec();
        files[index] = new_file_some;
        result.files_by_path = self.files_by_path.clone();
        result.files_by_path.insert(new_file_some.path().clone(), new_file_some);
        for (i, &old_supplemental) in old_supplemental_files.iter().enumerate() {
            let new_supplemental = new_supplemental_files[i];
            let supplemental_index = files.iter().position(|&file| file == old_supplemental).unwrap();
            files[supplemental_index] = new_supplemental;
            result.files_by_path.insert(new_supplemental.path().clone(), new_supplemental);
        }
        result.files = tsrs_core::alloc_vec(files);
        let result: &'static Program = Box::leak(Box::new(result));
        result.init_checker_pool(create_checker_pool.as_ref());
        (Some(result), new_file, true)
    }

    // program.go:443
    fn init_checker_pool(&'static self, create: Option<&CreateCheckerPool>) {
        if !self.finished_processing {
            panic!("Program must finish processing files before initializing checker pool");
        }

        let pool = match create {
            Some(create) => programCheckerPool::External(create(self)),
            None => programCheckerPool::Compiler(checkerPool::new(self)),
        };
        if self.checker_pool.set(pool).is_err() {
            panic!("checker pool initialized twice");
        }
    }

    // program.go:458
    // GetCheckerPool returns the checker pool associated with this program.
    pub fn get_checker_pool(&self) -> &dyn CheckerPool {
        match self.checker_pool.get().unwrap() {
            programCheckerPool::Compiler(pool) => pool,
            programCheckerPool::External(pool) => &**pool,
        }
    }

    // Go `p.compilerCheckerPool` (nil when the program was created with `CreateCheckerPool`).
    pub(crate) fn compiler_checker_pool(&self) -> Option<&checkerPool> {
        match self.checker_pool.get() {
            Some(programCheckerPool::Compiler(pool)) => Some(pool),
            _ => None,
        }
    }

    // The built-in pool, for the CLI's statistics; panics on a program with an external pool.
    pub(crate) fn pool(&self) -> &checkerPool {
        self.compiler_checker_pool().expect("program uses an external checker pool")
    }

    // program.go:462
    fn can_replace_file_in_program(&self, file1: P<SourceFile>, file2: Option<P<SourceFile>>) -> bool {
        let Some(file2) = file2 else {
            return false;
        };
        file1.parse_options() == file2.parse_options()
            && file1.script_kind.get() == file2.script_kind.get()
            && ast::is_external_or_common_js_module(file1) == ast::is_external_or_common_js_module(file2)
            && file1.uses_uri_style_node_core_modules() == file2.uses_uri_style_node_core_modules()
            && equal_func(file1.imports(), file2.imports(), |n1, n2| {
                equal_module_specifiers(n1, n2) && self.get_mode_for_usage_location(file1, n1) == self.get_mode_for_usage_location(file2, n2)
            })
            && equal_func(file1.module_augmentations(), file2.module_augmentations(), |n1, n2| equal_module_augmentation_names(n1, n2))
            && file1.ambient_module_names() == file2.ambient_module_names()
            && equal_func(file1.referenced_files(), file2.referenced_files(), |f1, f2| equal_file_references(f1, f2))
            && equal_func(file1.type_reference_directives.get(), file2.type_reference_directives.get(), |f1, f2| equal_file_references(f1, f2))
            && equal_func(file1.lib_reference_directives.get(), file2.lib_reference_directives.get(), |f1, f2| equal_file_references(f1, f2))
            && equal_check_js_directives(file1.check_js_directive.get(), file2.check_js_directive.get())
    }

    // program.go:480
    fn needs_import_helpers_import_specifier(&self, file: P<SourceFile>) -> bool {
        let (redirect, _) = self.project_reference_file_mapper.get_redirect_for_resolution(file.file_name(), file.path());
        let redirect = redirect.map(crate::projectreferencefilemapper::as_resolved_project_reference);
        let options_for_file = module::get_compiler_options_with_redirect(self.opts.config.compiler_options().unwrap(), redirect);
        if !options_for_file.import_helpers.is_true() {
            return false;
        }
        let is_java_script_file = ast::is_source_file_js(file);
        let is_external_module_file = ast::is_external_module(file);
        if !is_java_script_file && (file.is_declaration_file.get() || (!options_for_file.get_isolated_modules() && !is_external_module_file)) {
            return false;
        }
        true
    }

    // program.go:494
    fn jsx_runtime_import_specifier(&self, file: P<SourceFile>) -> String {
        if !ast::is_source_file_js(file) && file.script_kind.get() != ScriptKind::TSX {
            return String::new();
        }
        let (redirect, _) = self.project_reference_file_mapper.get_redirect_for_resolution(file.file_name(), file.path());
        let redirect = redirect.map(crate::projectreferencefilemapper::as_resolved_project_reference);
        let options_for_file = module::get_compiler_options_with_redirect(self.opts.config.compiler_options().unwrap(), redirect);
        ast::get_jsx_runtime_import(&ast::get_jsx_implicit_import_base(&options_for_file, Some(file)), &options_for_file)
    }

    // program.go:519
    pub fn source_files(&self) -> &'static [P<SourceFile>] {
        self.files
    }

    // program.go:520
    pub fn duplicate_source_files(&self) -> &[DuplicateSourceFile] {
        &self.processed.duplicate_source_files
    }

    // program.go:521
    pub fn options(&self) -> P<CompilerOptions> {
        self.opts.config.compiler_options().unwrap()
    }

    // program.go:536 (content mappers are not ported)
    pub fn content_mapper_extensions(&self) -> Vec<String> {
        Vec::new()
    }

    // program.go:537
    pub fn command_line(&self) -> P<ParsedCommandLine> {
        self.opts.config
    }

    // program.go:538
    pub fn host(&self) -> &Arc<dyn CompilerHost> {
        &self.host
    }

    // program.go:540
    pub fn get_config_file_parsing_diagnostics(&self) -> Vec<P<Diagnostic>> {
        self.opts.config.get_config_file_parsing_diagnostics().to_vec()
    }

    // program.go:546
    // GetUnresolvedImports returns the unresolved imports for this program.
    // The result is cached and computed only once.
    pub fn get_unresolved_imports(&self) -> &Set<String> {
        self.unresolved_imports.get_or_init(|| Arc::new(self.extract_unresolved_imports()))
    }

    // program.go:550
    fn extract_unresolved_imports(&self) -> Set<String> {
        let mut unresolved_set = Set::default();

        for &source_file in self.files {
            let unresolved_imports = self.extract_unresolved_imports_from_source_file(source_file);
            for imp in unresolved_imports {
                unresolved_set.add(imp);
            }
        }

        unresolved_set
    }

    // program.go:563
    fn extract_unresolved_imports_from_source_file(&self, file: P<SourceFile>) -> Vec<String> {
        let mut unresolved_imports = Vec::new();

        if let Some(resolved_modules) = self.resolved_modules.get(file.path()) {
            for (cache_key, resolution) in resolved_modules {
                let resolved = resolution.is_resolved();
                if (!resolved || !tspath::extension_is_one_of(resolution.extension, tspath::SUPPORTED_TS_EXTENSIONS_WITH_JSON_FLAT))
                    && !tspath::is_external_module_name_relative(cache_key.name)
                {
                    unresolved_imports.push(cache_key.name.to_string());
                }
            }
        }

        unresolved_imports
    }

    pub fn checker_count(&self) -> usize {
        self.pool().checker_count()
    }

    // TSRS_ASSIGNMENT_STATS report (checkerpool_stats.rs).
    #[cfg(feature = "checker")]
    pub fn checker_assignment_report(&'static self) -> String {
        crate::checkerpool_stats::report(self, self.pool())
    }

    pub(crate) fn processing_diagnostics(&self) -> std::sync::MutexGuard<'_, Vec<processingDiagnostic>> {
        self.processing_diagnostics.lock().unwrap()
    }

    pub(crate) fn add_processing_diagnostic(&self, d: processingDiagnostic) {
        self.processing_diagnostics.lock().unwrap().push(d);
    }

    // program.go:578
    pub fn single_threaded(&self) -> bool {
        self.opts.single_threaded.default_if_unknown(self.options().single_threaded).is_true()
    }

    // program.go:582
    pub fn bind_source_files(&self) {
        let unbound: Vec<P<SourceFile>> = self.files.iter().copied().filter(|f| !f.is_bound()).collect();
        if self.single_threaded() {
            // Go's single-threaded work group runs queued functions last-queued first, so files bind in reverse
            // program order. The order is observable: the binder assigns symbol ids (private names).
            unbound.into_iter().rev().for_each(tsrs_binder::bind_source_file);
        } else {
            worker_pool().install(|| unbound.into_par_iter().for_each(tsrs_binder::bind_source_file));
        }
    }

    // program.go:598
    // Return the type checker associated with the program.
    pub fn get_type_checker(&self, ctx: &Context) -> CheckerHandle {
        if let Some(pool) = self.compiler_checker_pool() {
            return pool.get_checker_non_exclusive();
        }
        self.get_checker_pool().get_checker(ctx, None)
    }

    // program.go:605
    pub fn for_each_checker_parallel(&self, cb: impl Fn(usize, &mut Checker) + Sync) {
        if let Some(pool) = self.compiler_checker_pool() {
            pool.for_each_checker_parallel(cb);
        }
    }

    // program.go:615
    // Return a checker for the given file. We may have multiple checkers in concurrent scenarios and this
    // method returns the checker that was tasked with checking the file. Note that it isn't possible to mix
    // types obtained from different checkers, so only non-type data (such as diagnostics or string
    // representations of types) should be obtained from checkers returned by this method.
    pub fn get_type_checker_for_file(&self, ctx: &Context, file: P<SourceFile>) -> CheckerHandle {
        if let Some(pool) = self.compiler_checker_pool() {
            return pool.get_checker_for_file_non_exclusive(file);
        }
        self.get_checker_pool().get_checker(ctx, Some(file))
    }

    // program.go:624
    // Return a checker for the given file, locked to the current thread to prevent data races from multiple threads
    // accessing the same checker. The lock will be released when the `done` function is called.
    pub fn get_type_checker_for_file_exclusive(&self, ctx: &Context, file: P<SourceFile>) -> CheckerHandle {
        if let Some(pool) = self.compiler_checker_pool() {
            return pool.get_checker_for_file_exclusive(file);
        }
        self.get_checker_pool().get_checker(ctx, Some(file))
    }

    pub fn get_resolved_module(&self, file: P<SourceFile>, module_reference: &str, mode: ResolutionMode) -> Option<P<ResolvedModule>> {
        self.get_resolved_module_by_path(&file.path(), module_reference, mode)
    }

    pub(crate) fn get_resolved_module_by_path(&self, path: &Path, module_reference: &str, mode: ResolutionMode) -> Option<P<ResolvedModule>> {
        let resolutions = self.resolved_modules.get(path)?;
        // Keys hold `&'static str`; a borrowed name cannot build a lookup key, and per-file caches are small.
        resolutions.iter().find(|(k, _)| k.name == module_reference && k.mode == mode).map(|(_, v)| *v)
    }

    pub fn get_resolved_module_from_module_specifier(&self, file: P<SourceFile>, module_specifier: P<Node>) -> Option<P<ResolvedModule>> {
        if !ast::is_string_literal_like(module_specifier) {
            panic!("moduleSpecifier must be a StringLiteralLike");
        }
        let mode = self.get_mode_for_usage_location(file, module_specifier);
        self.get_resolved_module(file, module_specifier.text(), mode)
    }

    pub fn get_resolved_modules(&self) -> &FxHashMap<Path, ModeAwareCache<P<ResolvedModule>>> {
        &self.resolved_modules
    }

    pub fn module_resolution_error(&self) -> Option<&str> {
        self.module_resolution_error.as_deref()
    }

    // GetPackagesMap returns a lazily-cached map of package names to whether they bundle types.
    pub fn get_packages_map(&self) -> &FxHashMap<String, bool> {
        self.packages_map.get_or_init(|| {
            let mut packages_map: FxHashMap<String, bool> = FxHashMap::default();
            for resolved_modules_in_file in self.resolved_modules.values() {
                for module in resolved_modules_in_file.values() {
                    if !module.package_id.name.is_empty() {
                        let entry = packages_map.entry(module.package_id.name.to_string()).or_insert(false);
                        *entry = *entry || module.extension == tspath::EXTENSION_DTS;
                    }
                }
            }
            packages_map
        })
    }

    // program.go:675
    // collectDiagnostics collects diagnostics from a single file or all files.
    // If sourceFile is non-nil, returns diagnostics for just that file.
    // If sourceFile is nil, returns diagnostics for all files in the program.
    fn collect_diagnostics(
        &self,
        ctx: &Context,
        source_file: Option<P<SourceFile>>,
        concurrent: bool,
        collect: impl Fn(&Context, P<SourceFile>) -> Vec<P<Diagnostic>> + Sync,
    ) -> Vec<P<Diagnostic>> {
        let result = match source_file {
            Some(file) => collect(ctx, file),
            None => {
                let diagnostics = self.collect_diagnostics_from_files(ctx, self.files, concurrent, &collect);
                diagnostics.concat()
            }
        };
        filter_and_sort_diagnostics(result)
    }

    // program.go:686
    fn collect_diagnostics_from_files(
        &self,
        ctx: &Context,
        source_files: &[P<SourceFile>],
        concurrent: bool,
        collect: &(impl Fn(&Context, P<SourceFile>) -> Vec<P<Diagnostic>> + Sync),
    ) -> Vec<Vec<P<Diagnostic>>> {
        if !concurrent || self.single_threaded() {
            source_files.iter().map(|&f| collect(ctx, f)).collect()
        } else {
            source_files.par_iter().map(|&f| collect(ctx, f)).collect()
        }
    }

    // program.go:703
    // collectCheckerDiagnostics collects diagnostics from a single file or all files,
    // using a callback that receives the checker for each file. When the checker pool
    // supports grouped iteration (compiler pool), files are grouped by checker and
    // processed in parallel with one task per checker, reducing contention and improving
    // cache locality. Otherwise, falls back to per-file concurrent collection.
    fn collect_checker_diagnostics(
        &'static self,
        ctx: &Context,
        source_file: Option<P<SourceFile>>,
        collect: impl Fn(&Context, &mut Checker, P<SourceFile>) -> Vec<P<Diagnostic>> + Sync,
    ) -> Vec<P<Diagnostic>> {
        if let Some(source_file) = source_file {
            if self.skip_type_checking(source_file, false) {
                return Vec::new();
            }
            let mut c = self.get_type_checker_for_file_exclusive(ctx, source_file);
            let result = collect(ctx, &mut c, source_file);
            drop(c);
            return filter_and_sort_diagnostics(result);
        }
        filter_and_sort_diagnostics(self.collect_checker_diagnostics_from_files(ctx, self.files, &collect).concat())
    }

    // program.go:728
    // collectCheckerDiagnosticsFromFiles collects checker diagnostics for a list of files.
    fn collect_checker_diagnostics_from_files(
        &'static self,
        ctx: &Context,
        source_files: &[P<SourceFile>],
        collect: &(impl Fn(&Context, &mut Checker, P<SourceFile>) -> Vec<P<Diagnostic>> + Sync),
    ) -> Vec<Vec<P<Diagnostic>>> {
        let diagnostics: Vec<Mutex<Vec<P<Diagnostic>>>> = source_files.iter().map(|_| Mutex::new(Vec::new())).collect();
        if let Some(pool) = self.compiler_checker_pool() {
            pool.for_each_checker_group_do(source_files, self.single_threaded(), |c, file_index, file| {
                *diagnostics[file_index].lock().unwrap() = collect(ctx, c, file);
            });
        } else {
            let run = |i: usize, file: P<SourceFile>| {
                if self.skip_type_checking(file, false) {
                    return;
                }
                let mut c = self.get_checker_pool().get_checker(ctx, Some(file));
                *diagnostics[i].lock().unwrap() = collect(ctx, &mut c, file);
                drop(c);
            };
            if self.single_threaded() {
                // Go's single-threaded work group runs queued functions last-queued first.
                source_files.iter().enumerate().rev().for_each(|(i, &file)| run(i, file));
            } else {
                worker_pool().install(|| source_files.par_iter().enumerate().for_each(|(i, &file)| run(i, file)));
            }
        }
        diagnostics.into_iter().map(|d| d.into_inner().unwrap()).collect()
    }

    // program.go:751
    pub fn get_syntactic_diagnostics(&self, ctx: &Context, source_file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.collect_diagnostics(ctx, source_file, false /*concurrent*/, |_, file| {
            let mut diags: Vec<P<Diagnostic>> = file.diagnostics().to_vec();
            diags.extend_from_slice(file.js_diagnostics());
            // For JS files that won't be checked by the checker (no checkJs/ts-check), we need
            // program-level syntactic checks that require compiler options. This mirrors Strada's
            // getJSSyntacticDiagnosticsForFile in program.ts.
            if ast::is_source_file_js(file) && !ast::is_check_js_enabled_for_file(file, &self.options()) {
                diags.extend(get_additional_js_syntactic_diagnostics(file, &self.options()));
            }
            diags
        })
    }

    // program.go:795
    pub fn get_bind_diagnostics(&self, ctx: &Context, source_file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        match source_file {
            Some(file) => tsrs_binder::bind_source_file(file),
            None => self.bind_source_files(),
        }
        self.collect_diagnostics(ctx, source_file, false /*concurrent*/, |_, file| file.bind_diagnostics().to_vec())
    }

    // program.go:806
    pub fn get_semantic_diagnostics(&'static self, ctx: &Context, source_file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.collect_checker_diagnostics(ctx, source_file, |ctx, c, file| self.get_semantic_diagnostics_with_checker(ctx, c, file))
    }

    // program.go:812
    // GetSemanticDiagnosticsForIncremental includes newly discovered globals in each
    // file's cached diagnostics and leaves noEmit filtering to the builder.
    pub fn get_semantic_diagnostics_for_incremental(
        &'static self,
        ctx: &Context,
        source_files: &[P<SourceFile>],
    ) -> FxHashMap<P<SourceFile>, Vec<P<Diagnostic>>> {
        let all_diags = self.collect_checker_diagnostics_from_files(ctx, source_files, &|ctx: &Context, c: &mut Checker, file: P<SourceFile>| {
            self.get_bind_and_check_diagnostics_with_checker(ctx, c, file, true /*includeDeferredGlobals*/)
        });
        let mut result = FxHashMap::default();
        for (i, diags) in all_diags.into_iter().enumerate() {
            result.insert(source_files[i], filter_and_sort_diagnostics(diags));
        }
        result
    }

    // program.go:823
    pub fn get_suggestion_diagnostics(&'static self, ctx: &Context, source_file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.collect_checker_diagnostics(ctx, source_file, |ctx, c, file| self.get_suggestion_diagnostics_with_checker(ctx, c, file))
    }

    // program.go:827
    pub fn get_program_diagnostics(&'static self) -> Vec<P<Diagnostic>> {
        let mut all = self.program_diagnostics.clone();
        all.extend(self.include_processor.get_diagnostics(self).lock().unwrap().get_global_diagnostics());
        sort_and_deduplicate_diagnostics(&all)
    }

    pub fn get_include_processor_diagnostics(&'static self, source_file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        if self.skip_type_checking(source_file, false) {
            return Vec::new();
        }
        let diags = self.include_processor.get_diagnostics(self).lock().unwrap().get_diagnostics_for_file(source_file);
        let (filtered, _) = self.get_diagnostics_with_preceding_directives(source_file, diags);
        filtered
    }

    pub fn skip_type_checking(&self, source_file: P<SourceFile>, ignore_no_check: bool) -> bool {
        (!ignore_no_check && self.options().no_check.is_true())
            || self.options().skip_lib_check.is_true() && source_file.is_declaration_file.get()
            || self.options().skip_default_lib_check.is_true() && self.is_source_file_default_library(&source_file.path())
            || self.is_source_from_project_reference(&source_file.path())
            || !self.can_include_bind_and_check_diagnostics(source_file)
    }

    fn can_include_bind_and_check_diagnostics(&self, source_file: P<SourceFile>) -> bool {
        if let Some(directive) = source_file.check_js_directive.get() {
            if !directive.enabled {
                return false;
            }
        }

        if source_file.script_kind.get() == ScriptKind::TS || source_file.script_kind.get() == ScriptKind::TSX {
            return true;
        }

        let is_js = source_file.script_kind.get() == ScriptKind::JS || source_file.script_kind.get() == ScriptKind::JSX;
        let is_check_js = is_js && ast::is_check_js_enabled_for_file(source_file, &self.options());
        let is_plain_js = ast::is_plain_js_file(Some(source_file), self.options().check_js);

        // By default, only type-check .ts, .tsx, plain JS, and checked JS
        // - plain JS: .js files with no // ts-check and checkJs: undefined
        // - check JS: .js files with either // ts-check or checkJs: true
        is_plain_js || is_check_js
    }

    fn verify_compiler_options(&mut self) {
        let options = self.options();

        let source_file = self.opts.config.config_file.as_ref().map(|c| c.source_file);
        let config_file_path = source_file.map(|f| f.file_name().to_string()).unwrap_or_default();

        let compiler_options_property_syntax: Option<P<Node>> =
            tsoptions::for_each_ts_config_prop_array(source_file, "compilerOptions", |p| Some(p));

        let compiler_options_object_literal_syntax: Option<P<Node>> = compiler_options_property_syntax
            .and_then(|p| p.initializer())
            .filter(|init| ast::is_object_literal_expression(*init));

        let mut diags: Vec<P<Diagnostic>> = Vec::new();

        macro_rules! create_option_diagnostic_in_object_literal_syntax {
            ($object_literal:expr, $on_key:expr, $key1:expr, $key2:expr, $message:expr, $args:expr) => {{
                let key2: &str = $key2;
                let keys2: Option<&str> = if key2.is_empty() { None } else { Some(key2) };
                let diag = tsoptions::for_each_property_assignment(
                    $object_literal,
                    $key1,
                    |property: P<Node>| {
                        Some(tsoptions::create_diagnostic_for_node_in_source_file(
                            source_file.unwrap(),
                            if $on_key { property.name().unwrap() } else { property.initializer().unwrap() },
                            $message,
                            $args,
                        ))
                    },
                    keys2,
                );
                if let Some(diag) = diag {
                    diags.push(diag);
                }
                diag
            }};
        }

        let create_compiler_options_diagnostic = |diags: &mut Vec<P<Diagnostic>>, message: &'static Message, args: &[&dyn Display]| {
            let diag = if let Some(compiler_options_property) = compiler_options_property_syntax {
                tsoptions::create_diagnostic_for_node_in_source_file(source_file.unwrap(), compiler_options_property.name().unwrap(), message, args)
            } else {
                new_compiler_diagnostic(message, args)
            };
            diags.push(diag);
            diag
        };

        macro_rules! create_diagnostic_for_option {
            ($on_key:expr, $option1:expr, $option2:expr, $message:expr, $args:expr) => {{
                let diag = create_option_diagnostic_in_object_literal_syntax!(
                    compiler_options_object_literal_syntax,
                    $on_key,
                    $option1,
                    $option2,
                    $message,
                    $args
                );
                match diag {
                    Some(d) => d,
                    None => create_compiler_options_diagnostic(&mut diags, $message, $args),
                }
            }};
        }

        macro_rules! create_diagnostic_for_option_name {
            ($message:expr, $option1:expr, $option2:expr) => {{
                let o1: &str = $option1;
                let o2: &str = $option2;
                create_diagnostic_for_option!(true, o1, o2, $message, &[&o1, &o2]);
            }};
            ($message:expr, $option1:expr, $option2:expr, $($arg:expr),+) => {{
                let o1: &str = $option1;
                let o2: &str = $option2;
                create_diagnostic_for_option!(true, o1, o2, $message, &[&o1, &o2, $(&$arg),+]);
            }};
        }

        macro_rules! create_option_value_diagnostic {
            ($option1:expr, $message:expr, $args:expr) => {{
                create_diagnostic_for_option!(false, $option1, "", $message, $args);
            }};
        }

        macro_rules! create_removed_option_diagnostic {
            ($name:expr, $value:expr, $use_instead:expr) => {{
                let name: &str = $name;
                let value: &str = $value;
                let use_instead: String = $use_instead;
                let diag = if value.is_empty() {
                    create_diagnostic_for_option!(
                        true,
                        name,
                        "",
                        &diagnostics::Option_0_has_been_removed_Please_remove_it_from_your_configuration,
                        &[&name]
                    )
                } else {
                    create_diagnostic_for_option!(
                        false,
                        name,
                        "",
                        &diagnostics::Option_0_1_has_been_removed_Please_remove_it_from_your_configuration,
                        &[&name, &value]
                    )
                };
                if !use_instead.is_empty() {
                    diag.add_message_chain(new_compiler_diagnostic(&diagnostics::Use_0_instead, &[&use_instead]));
                }
            }};
        }

        // Removed in TS7

        if !options.base_url.is_empty() {
            // BaseUrl will have been turned absolute by this point.
            let mut use_instead = String::new();
            if !config_file_path.is_empty() {
                let mut relative = tspath::get_relative_path_from_file(&config_file_path, &options.base_url, &self.compare_paths_options);
                if !(relative.starts_with("./") || relative.starts_with("../")) {
                    relative = format!("./{}", relative);
                }
                let suggestion = tspath::combine_paths(&relative, &["*"]);
                use_instead = format!("\"paths\": {{\"*\": [{}]}}", tsrs_core::json::marshal_string(&suggestion));
            }
            create_removed_option_diagnostic!("baseUrl", "", use_instead);
        }

        if !options.out_file.is_empty() {
            create_removed_option_diagnostic!("outFile", "", String::new());
        }

        if options.target == ScriptTarget::ES5 {
            create_removed_option_diagnostic!("target", "ES5", String::new());
        }

        if options.module == ModuleKind::AMD {
            create_removed_option_diagnostic!("module", "AMD", String::new());
        }
        if options.module == ModuleKind::System {
            create_removed_option_diagnostic!("module", "System", String::new());
        }
        if options.module == ModuleKind::UMD {
            create_removed_option_diagnostic!("module", "UMD", String::new());
        }

        if options.module_resolution == ModuleResolutionKind::Classic {
            create_removed_option_diagnostic!("moduleResolution", "Classic", String::new());
        }

        if options.always_strict.is_false() {
            create_removed_option_diagnostic!("alwaysStrict", "false", String::new());
        }

        if options.es_module_interop.is_false() {
            create_removed_option_diagnostic!("esModuleInterop", "false", String::new());
        }

        if options.allow_synthetic_default_imports.is_false() {
            create_removed_option_diagnostic!("allowSyntheticDefaultImports", "false", String::new());
        }

        if options.module_resolution == ModuleResolutionKind::Node10 {
            create_removed_option_diagnostic!("moduleResolution", "node10", String::new());
        }

        if !options.downlevel_iteration.is_unknown() {
            create_removed_option_diagnostic!("downlevelIteration", "", String::new());
        }

        if options.strict_property_initialization.is_true() && !options.get_strict_option_value(options.strict_null_checks) {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_cannot_be_specified_without_specifying_option_1,
                "strictPropertyInitialization",
                "strictNullChecks"
            );
        }
        if options.exact_optional_property_types.is_true() && !options.get_strict_option_value(options.strict_null_checks) {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_cannot_be_specified_without_specifying_option_1,
                "exactOptionalPropertyTypes",
                "strictNullChecks"
            );
        }

        if options.isolated_declarations.is_true() {
            if options.get_allow_js() {
                create_diagnostic_for_option_name!(&diagnostics::Option_0_cannot_be_specified_with_option_1, "allowJs", "isolatedDeclarations");
            }
            if !options.get_emit_declarations() {
                create_diagnostic_for_option_name!(
                    &diagnostics::Option_0_cannot_be_specified_without_specifying_option_1_or_option_2,
                    "isolatedDeclarations",
                    "declaration",
                    "composite"
                );
            }
        }

        if options.inline_source_map.is_true() {
            if options.source_map.is_true() {
                create_diagnostic_for_option_name!(&diagnostics::Option_0_cannot_be_specified_with_option_1, "sourceMap", "inlineSourceMap");
            }
            if !options.map_root.is_empty() {
                create_diagnostic_for_option_name!(&diagnostics::Option_0_cannot_be_specified_with_option_1, "mapRoot", "inlineSourceMap");
            }
        }

        if options.composite.is_true() {
            if options.declaration.is_false() {
                create_diagnostic_for_option_name!(&diagnostics::Composite_projects_may_not_disable_declaration_emit, "declaration", "");
            }
            if options.incremental.is_false() {
                create_diagnostic_for_option_name!(&diagnostics::Composite_projects_may_not_disable_incremental_compilation, "declaration", "");
            }
        }

        if options.ts_build_info_file.is_empty() && options.incremental.is_true() && options.config_file_path.is_empty() {
            create_compiler_options_diagnostic(
                &mut diags,
                &diagnostics::Option_incremental_is_only_valid_with_a_known_configuration_file_like_tsconfig_json_or_when_tsBuildInfoFile_is_explicitly_provided,
                &[],
            );
        }

        self.program_diagnostics.append(&mut diags);
        self.verify_project_references();

        if options.composite.is_true() {
            let mut root_paths: FxHashSet<Path> = FxHashSet::default();
            for file_name in self.opts.config.file_names() {
                root_paths.insert(self.to_path(file_name));
            }

            for &file in self.files.iter() {
                let root_path = file.path();
                if source_file_may_be_emitted(file, self, false, false) && !root_paths.contains(root_path) {
                    self.add_processing_diagnostic(processingDiagnostic::explaining(includeExplainingDiagnostic {
                        file: Some(file.path().clone()),
                        diagnostic_reason: None,
                        message: &diagnostics::File_0_is_not_listed_within_the_file_list_of_project_1_Projects_must_list_all_files_or_use_an_include_pattern,
                        args: vec![file.file_name().to_string(), config_file_path.clone()],
                    }));
                }
            }
        }

        let paths_object_literals: Vec<P<Node>> = {
            // forEachOptionPathsSyntax visits every "paths" property; collect their object literal initializers.
            let mut result = Vec::new();
            tsoptions::for_each_property_assignment(
                compiler_options_object_literal_syntax,
                "paths",
                |path_prop: P<Node>| -> Option<P<Diagnostic>> {
                    result.push(path_prop);
                    None
                },
                None,
            );
            result
        };

        let create_diagnostic_for_option_paths =
            |diags: &mut Vec<P<Diagnostic>>, on_key: bool, key: &str, message: &'static Message, args: &[&dyn Display]| {
                let mut diag = None;
                for &path_prop in &paths_object_literals {
                    let initializer = path_prop.initializer().unwrap();
                    if ast::is_object_literal_expression(initializer) {
                        let d = tsoptions::for_each_property_assignment(
                            Some(initializer),
                            key,
                            |property: P<Node>| {
                                Some(tsoptions::create_diagnostic_for_node_in_source_file(
                                    source_file.unwrap(),
                                    if on_key { property.name().unwrap() } else { property.initializer().unwrap() },
                                    message,
                                    args,
                                ))
                            },
                            None,
                        );
                        if let Some(d) = d {
                            diags.push(d);
                            diag = Some(d);
                            break;
                        }
                    }
                }
                if diag.is_none() {
                    create_compiler_options_diagnostic(diags, message, args);
                }
            };

        let create_diagnostic_for_option_path_key_value =
            |diags: &mut Vec<P<Diagnostic>>, key: &str, value_index: usize, message: &'static Message, args: &[&dyn Display]| {
                let mut diag = None;
                for &path_prop in &paths_object_literals {
                    let initializer = path_prop.initializer().unwrap();
                    if ast::is_object_literal_expression(initializer) {
                        let d = tsoptions::for_each_property_assignment(
                            Some(initializer),
                            key,
                            |key_props: P<Node>| {
                                let initializer = key_props.initializer().unwrap();
                                if ast::is_array_literal_expression(initializer) {
                                    let elements = initializer.elements();
                                    if elements.len() > value_index {
                                        return Some(tsoptions::create_diagnostic_for_node_in_source_file(
                                            source_file.unwrap(),
                                            elements[value_index],
                                            message,
                                            args,
                                        ));
                                    }
                                }
                                None
                            },
                            None,
                        );
                        if let Some(d) = d {
                            diags.push(d);
                            diag = Some(d);
                            break;
                        }
                    }
                }
                if diag.is_none() {
                    create_compiler_options_diagnostic(diags, message, args);
                }
            };

        if let Some(paths) = &options.paths {
            for (key, value) in paths.iter() {
                // !!! This code does not handle cases where where the path mappings have the wrong types,
                // as that information is mostly lost during the parsing process.
                if !has_zero_or_one_asterisk_character(key) {
                    create_diagnostic_for_option_paths(&mut diags, true, key, &diagnostics::Pattern_0_can_have_at_most_one_Asterisk_character, &[&key]);
                }
                if value.is_empty() {
                    create_diagnostic_for_option_paths(
                        &mut diags,
                        false,
                        key,
                        &diagnostics::Substitutions_for_pattern_0_shouldn_t_be_an_empty_array,
                        &[&key],
                    );
                }
                for (i, subst) in value.iter().enumerate() {
                    if !has_zero_or_one_asterisk_character(subst) {
                        create_diagnostic_for_option_path_key_value(
                            &mut diags,
                            key,
                            i,
                            &diagnostics::Substitution_0_in_pattern_1_can_have_at_most_one_Asterisk_character,
                            &[&subst, &key],
                        );
                    }
                    if !tspath::path_is_relative(subst) && !tspath::path_is_absolute(subst) {
                        create_diagnostic_for_option_path_key_value(
                            &mut diags,
                            key,
                            i,
                            &diagnostics::Non_relative_paths_are_not_allowed_Did_you_forget_a_leading_Slash,
                            &[],
                        );
                    }
                }
            }
        }

        if options.source_map.is_false_or_unknown() && options.inline_source_map.is_false_or_unknown() {
            if options.inline_sources.is_true() {
                create_diagnostic_for_option_name!(
                    &diagnostics::Option_0_can_only_be_used_when_either_option_inlineSourceMap_or_option_sourceMap_is_provided,
                    "inlineSources",
                    ""
                );
            }
            if !options.source_root.is_empty() {
                create_diagnostic_for_option_name!(
                    &diagnostics::Option_0_can_only_be_used_when_either_option_inlineSourceMap_or_option_sourceMap_is_provided,
                    "sourceRoot",
                    ""
                );
            }
        }

        if !options.map_root.is_empty() && !(options.source_map.is_true() || options.declaration_map.is_true()) {
            // Error to specify --mapRoot without --sourcemap
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_cannot_be_specified_without_specifying_option_1_or_option_2,
                "mapRoot",
                "sourceMap",
                "declarationMap"
            );
        }

        if !options.declaration_dir.is_empty() && !options.get_emit_declarations() {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_cannot_be_specified_without_specifying_option_1_or_option_2,
                "declarationDir",
                "declaration",
                "composite"
            );
        }

        if options.declaration_map.is_true() && !options.get_emit_declarations() {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_cannot_be_specified_without_specifying_option_1_or_option_2,
                "declarationMap",
                "declaration",
                "composite"
            );
        }

        if options.lib.is_some() && options.no_lib.is_true() {
            create_diagnostic_for_option_name!(&diagnostics::Option_0_cannot_be_specified_with_option_1, "lib", "noLib");
        }

        if options.isolated_modules.is_true() || options.verbatim_module_syntax.is_true() {
            if options.preserve_const_enums.is_false() {
                create_diagnostic_for_option_name!(
                    &diagnostics::Option_preserveConstEnums_cannot_be_disabled_when_0_is_enabled,
                    if options.verbatim_module_syntax.is_true() { "verbatimModuleSyntax" } else { "isolatedModules" },
                    "preserveConstEnums"
                );
            }
        }

        self.program_diagnostics.append(&mut diags);

        if !options.out_dir.is_empty()
            || !options.root_dir.is_empty()
            || !options.source_root.is_empty()
            || !options.map_root.is_empty()
            || (options.get_emit_declarations() && !options.declaration_dir.is_empty())
        {
            // !!! sheetal checkSourceFilesBelongToPath - for root Dir and configFile - explaining why file is in the program
            let dir = self.common_source_directory();
            if !options.out_dir.is_empty() && dir.is_empty() && self.files.iter().any(|f| tspath::get_root_length(f.file_name()) > 1) {
                create_diagnostic_for_option_name!(&diagnostics::Cannot_find_the_common_subdirectory_path_for_the_input_files, "outDir", "");
            }
        }

        if !options.no_emit.is_true()
            && !options.composite.is_true()
            && options.root_dir.is_empty()
            && !options.config_file_path.is_empty()
            && (!options.out_dir.is_empty() || (options.get_emit_declarations() && !options.declaration_dir.is_empty()) || !options.out_file.is_empty())
        {
            // Check if rootDir inferred changed and issue diagnostic
            let dir = self.common_source_directory().to_string();
            let emitted_files: Vec<String> = self
                .files
                .iter()
                .filter(|f| !f.is_declaration_file.get() && source_file_may_be_emitted(**f, self, false, false))
                .map(|f| f.file_name().to_string())
                .collect();
            let dir59 = outputpaths::get_computed_common_source_directory(&emitted_files, self.get_current_directory(), self.use_case_sensitive_file_names());
            if !dir59.is_empty()
                && tspath::get_canonical_file_name(&dir, self.use_case_sensitive_file_names())
                    != tspath::get_canonical_file_name(&dir59, self.use_case_sensitive_file_names())
            {
                // change in layout
                let option1 = if !options.out_file.is_empty() {
                    "outFile"
                } else if !options.out_dir.is_empty() {
                    "outDir"
                } else {
                    "declarationDir"
                };
                let option2 = if options.out_file.is_empty() && !options.out_dir.is_empty() { "declarationDir" } else { "" };
                let base = tspath::get_base_file_name(&options.config_file_path);
                let relative = tspath::get_relative_path_from_file(&options.config_file_path, &dir59, &self.compare_paths_options);
                let diag = create_diagnostic_for_option!(
                    true,
                    option1,
                    option2,
                    &diagnostics::The_common_source_directory_of_0_is_1_The_rootDir_setting_must_be_explicitly_set_to_this_or_another_path_to_adjust_your_output_s_file_layout,
                    &[&base, &relative]
                );
                diag.add_message_chain(new_compiler_diagnostic(&diagnostics::Visit_https_Colon_Slash_Slashaka_ms_Slashts6_for_migration_information, &[]));
            }
        }

        if options.check_js.is_true() && !options.get_allow_js() {
            create_diagnostic_for_option_name!(&diagnostics::Option_0_cannot_be_specified_without_specifying_option_1, "checkJs", "allowJs");
        }

        if options.emit_declaration_only.is_true() && !options.get_emit_declarations() {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_cannot_be_specified_without_specifying_option_1_or_option_2,
                "emitDeclarationOnly",
                "declaration",
                "composite"
            );
        }

        if options.emit_decorator_metadata.is_true() && options.experimental_decorators.is_false_or_unknown() {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_cannot_be_specified_without_specifying_option_1,
                "emitDecoratorMetadata",
                "experimentalDecorators"
            );
        }

        let jsx_is_react_jsx = options.jsx == JsxEmit::ReactJSX || options.jsx == JsxEmit::ReactJSXDev;
        if !options.jsx_factory.is_empty() {
            if !options.react_namespace.is_empty() {
                create_diagnostic_for_option_name!(&diagnostics::Option_0_cannot_be_specified_with_option_1, "reactNamespace", "jsxFactory");
            }
            if jsx_is_react_jsx {
                create_diagnostic_for_option_name!(
                    &diagnostics::Option_0_cannot_be_specified_when_option_jsx_is_1,
                    "jsxFactory",
                    options.jsx.string()
                );
            }
            if tsrs_parser::parse_isolated_entity_name(&options.jsx_factory).is_none() {
                create_option_value_diagnostic!(
                    "jsxFactory",
                    &diagnostics::Invalid_value_for_jsxFactory_0_is_not_a_valid_identifier_or_qualified_name,
                    &[&options.jsx_factory]
                );
            }
        } else if !options.react_namespace.is_empty()
            && !tsrs_scanner::is_identifier_text(&options.react_namespace, tsrs_core::LanguageVariant::Standard)
        {
            create_option_value_diagnostic!(
                "reactNamespace",
                &diagnostics::Invalid_value_for_reactNamespace_0_is_not_a_valid_identifier,
                &[&options.react_namespace]
            );
        }

        if !options.jsx_fragment_factory.is_empty() {
            if options.jsx_factory.is_empty() {
                create_diagnostic_for_option_name!(
                    &diagnostics::Option_0_cannot_be_specified_without_specifying_option_1,
                    "jsxFragmentFactory",
                    "jsxFactory"
                );
            }
            if jsx_is_react_jsx {
                create_diagnostic_for_option_name!(
                    &diagnostics::Option_0_cannot_be_specified_when_option_jsx_is_1,
                    "jsxFragmentFactory",
                    options.jsx.string()
                );
            }
            if tsrs_parser::parse_isolated_entity_name(&options.jsx_fragment_factory).is_none() {
                create_option_value_diagnostic!(
                    "jsxFragmentFactory",
                    &diagnostics::Invalid_value_for_jsxFragmentFactory_0_is_not_a_valid_identifier_or_qualified_name,
                    &[&options.jsx_fragment_factory]
                );
            }
        }

        if !options.react_namespace.is_empty() && jsx_is_react_jsx {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_cannot_be_specified_when_option_jsx_is_1,
                "reactNamespace",
                options.jsx.string()
            );
        }

        if !options.jsx_import_source.is_empty() && options.jsx == JsxEmit::React {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_cannot_be_specified_when_option_jsx_is_1,
                "jsxImportSource",
                options.jsx.string()
            );
        }

        let module_kind = options.get_emit_module_kind();

        if options.allow_importing_ts_extensions.is_true()
            && !(options.no_emit.is_true() || options.emit_declaration_only.is_true() || options.rewrite_relative_import_extensions.is_true())
        {
            create_option_value_diagnostic!(
                "allowImportingTsExtensions",
                &diagnostics::Option_allowImportingTsExtensions_can_only_be_used_when_one_of_noEmit_emitDeclarationOnly_or_rewriteRelativeImportExtensions_is_set,
                &[]
            );
        }

        let module_resolution = options.get_module_resolution_kind();
        if options.resolve_package_json_exports.is_true() && !module_resolution_supports_package_json_exports_and_imports(module_resolution) {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_can_only_be_used_when_moduleResolution_is_set_to_node16_nodenext_or_bundler,
                "resolvePackageJsonExports",
                ""
            );
        }
        if options.resolve_package_json_imports.is_true() && !module_resolution_supports_package_json_exports_and_imports(module_resolution) {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_can_only_be_used_when_moduleResolution_is_set_to_node16_nodenext_or_bundler,
                "resolvePackageJsonImports",
                ""
            );
        }
        if options.custom_conditions.is_some() && !module_resolution_supports_package_json_exports_and_imports(module_resolution) {
            create_diagnostic_for_option_name!(
                &diagnostics::Option_0_can_only_be_used_when_moduleResolution_is_set_to_node16_nodenext_or_bundler,
                "customConditions",
                ""
            );
        }

        if module_resolution == ModuleResolutionKind::Bundler
            && !emit_module_kind_is_non_node_esm(module_kind)
            && module_kind != ModuleKind::Preserve
            && module_kind != ModuleKind::CommonJS
        {
            create_option_value_diagnostic!(
                "moduleResolution",
                &diagnostics::Option_0_can_only_be_used_when_module_is_set_to_preserve_commonjs_or_es2015_or_later,
                &[&"bundler"]
            );
        }

        if ModuleKind::Node16 <= module_kind
            && module_kind <= ModuleKind::NodeNext
            && !(ModuleResolutionKind::Node16 <= module_resolution && module_resolution <= ModuleResolutionKind::NodeNext)
        {
            let module_kind_name = module_kind.string();
            let module_resolution_name = match tsrs_core::MODULE_KIND_TO_MODULE_RESOLUTION_KIND.get(&module_kind) {
                Some(v) => v.string().to_string(),
                None => "Node16".to_string(),
            };
            create_option_value_diagnostic!(
                "moduleResolution",
                &diagnostics::Option_moduleResolution_must_be_set_to_0_or_left_unspecified_when_option_module_is_set_to_1,
                &[&module_resolution_name, &module_kind_name]
            );
        } else if ModuleResolutionKind::Node16 <= module_resolution
            && module_resolution <= ModuleResolutionKind::NodeNext
            && !(ModuleKind::Node16 <= module_kind && module_kind <= ModuleKind::NodeNext)
        {
            let module_resolution_name = module_resolution.string();
            create_option_value_diagnostic!(
                "module",
                &diagnostics::Option_module_must_be_set_to_0_when_option_moduleResolution_is_set_to_1,
                &[&module_resolution_name, &module_resolution_name]
            );
        }

        self.program_diagnostics.append(&mut diags);

        // !!! The below needs filesByName, which is not equivalent to p.filesByPath.

        // If the emit is enabled make sure that every output file is unique and not overwriting any of the input files
        if !options.no_emit.is_true() && !options.suppress_output_path_check.is_true() {
            let mut emit_files_seen: FxHashSet<String> = FxHashSet::default();
            let mut emit_file_names: Vec<String> = Vec::new();
            tsoptions::outputpaths::for_each_emitted_file(
                &*self,
                &options,
                |paths, _| {
                    emit_file_names.push(paths.js_file_path().to_string());
                    emit_file_names.push(paths.source_map_file_path().to_string());
                    emit_file_names.push(paths.declaration_file_path().to_string());
                    emit_file_names.push(paths.declaration_map_path().to_string());
                    false
                },
                &self.get_source_files_to_emit(None, false, false),
                false,
            );
            emit_file_names.push(self.opts.config.get_build_info_file_name());

            // Verify that all the emit files are unique and don't overwrite input files
            for emit_file_name in &emit_file_names {
                if emit_file_name.is_empty() {
                    continue;
                }
                let emit_file_path = self.to_path(emit_file_name);
                // Report error if the output overwrites input file
                if self.files_by_path().contains_key(&emit_file_path) {
                    let diag = new_compiler_diagnostic(&diagnostics::Cannot_write_file_0_because_it_would_overwrite_input_file, &[emit_file_name]);
                    if config_file_path.is_empty() {
                        // The program is from either an inferred project or an external project
                        diag.add_message_chain(new_compiler_diagnostic(
                            &diagnostics::Adding_a_tsconfig_json_file_will_help_organize_projects_that_contain_both_TypeScript_and_JavaScript_files_Learn_more_at_https_Colon_Slash_Slashaka_ms_Slashtsconfig,
                            &[],
                        ));
                    }
                    self.block_emitting_of_file(emit_file_name, diag);
                }

                let emit_file_key = if !self.host().fs().use_case_sensitive_file_names() {
                    tspath::to_file_name_lower_case(&emit_file_path)
                } else {
                    emit_file_path.0.clone()
                };

                // Report error if multiple files write into same file
                if emit_files_seen.contains(&emit_file_key) {
                    // Already seen the same emit file - report error
                    self.block_emitting_of_file(
                        emit_file_name,
                        new_compiler_diagnostic(&diagnostics::Cannot_write_file_0_because_it_would_be_overwritten_by_multiple_input_files, &[emit_file_name]),
                    );
                } else {
                    emit_files_seen.insert(emit_file_key);
                }
            }
        }
    }

    fn get_source_files_to_emit(&self, target_source_files: Option<&[P<SourceFile>]>, force_dts_emit: bool, force_js_emit: bool) -> Vec<P<SourceFile>> {
        crate::emitter::get_source_files_to_emit(self, target_source_files, force_dts_emit, force_js_emit)
    }

    fn block_emitting_of_file(&mut self, emit_file_name: &str, diag: P<Diagnostic>) {
        let path = self.to_path(emit_file_name);
        self.has_emit_blocking_diagnostics.insert(path);
        self.program_diagnostics.push(diag);
    }

    pub fn is_emit_blocked(&self, emit_file_name: &str) -> bool {
        self.has_emit_blocking_diagnostics.contains(&self.to_path(emit_file_name))
    }

    // program.go:1402
    fn verify_project_references(&mut self) {
        let build_info_file_name =
            if !self.options().suppress_output_path_check.is_true() { self.opts.config.get_build_info_file_name() } else { String::new() };
        let mut program_diagnostics = Vec::new();
        let mut emit_blocking = Vec::new();
        let mut create_diagnostic_for_reference = |config: P<ParsedCommandLine>, index: usize, message: &'static Message, args: &[&dyn Display]| {
            let diag = tsoptions::create_diagnostic_at_reference_syntax(&config, index, message, args)
                .unwrap_or_else(|| new_compiler_diagnostic(message, args));
            program_diagnostics.push(diag);
        };

        self.project_reference_file_mapper.range_resolved_project_reference(|_path, config, parent, index| {
            let ref_ = &parent.project_references()[index];
            // !!! Deprecated in 5.0 and removed since 5.5
            // verifyRemovedProjectReference(ref, parent, index);
            let Some(config) = config else {
                create_diagnostic_for_reference(parent, index, &diagnostics::File_0_not_found, &[&ref_.path]);
                return true;
            };
            let ref_options = config.compiler_options().unwrap();
            if !ref_options.composite.is_true() || ref_options.no_emit.is_true() {
                if !parent.file_names().is_empty() {
                    if !ref_options.composite.is_true() {
                        create_diagnostic_for_reference(parent, index, &diagnostics::Referenced_project_0_must_have_setting_composite_Colon_true, &[&ref_.path]);
                    }
                    if ref_options.no_emit.is_true() {
                        create_diagnostic_for_reference(parent, index, &diagnostics::Referenced_project_0_may_not_disable_emit, &[&ref_.path]);
                    }
                }
            }
            if !build_info_file_name.is_empty() && build_info_file_name == config.get_build_info_file_name() {
                create_diagnostic_for_reference(
                    parent,
                    index,
                    &diagnostics::Cannot_write_file_0_because_it_will_overwrite_tsbuildinfo_file_generated_by_referenced_project_1,
                    &[&build_info_file_name, &ref_.path],
                );
                emit_blocking.push(build_info_file_name.clone());
            }
            true
        });
        self.program_diagnostics.extend(program_diagnostics);
        for file_name in emit_blocking {
            let path = self.to_path(&file_name);
            self.has_emit_blocking_diagnostics.insert(path);
        }
    }

    // program.go:1463
    pub fn get_global_diagnostics(&'static self, _ctx: &Context) -> Vec<P<Diagnostic>> {
        if self.files.is_empty() {
            return Vec::new();
        }
        if let Some(pool) = self.compiler_checker_pool() {
            return pool.get_global_diagnostics();
        }
        // For external pools (project system), global diagnostics are collected
        // incrementally as checkers are used, not via a bulk query.
        Vec::new()
    }

    // program.go:1475. Go's `collectDiagnostics(ctx, sourceFile, true /*concurrent*/, p.getDeclarationDiagnosticsForFile)`
    // runs one task per file, each taking the file's checker from the pool; with the built-in pool the files run
    // grouped by checker on the checker threads (in file order within a checker), like the other checker-backed
    // diagnostics. With an external pool each file takes its checker from the pool, as in Go.
    #[cfg(feature = "checker")]
    pub fn get_declaration_diagnostics(&'static self, ctx: &Context, source_file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        let result = match (source_file, self.compiler_checker_pool()) {
            (Some(file), _) => self.get_declaration_diagnostics_for_file(ctx, None, file),
            (None, Some(pool)) => {
                let diagnostics: Vec<Mutex<Vec<P<Diagnostic>>>> = self.files.iter().map(|_| Mutex::new(Vec::new())).collect();
                pool.for_each_checker_group_do(self.files, self.single_threaded(), |c, file_index, file| {
                    *diagnostics[file_index].lock().unwrap() = self.get_declaration_diagnostics_for_file(ctx, Some(c), file);
                });
                diagnostics.into_iter().flat_map(|d| d.into_inner().unwrap()).collect()
            }
            (None, None) => {
                self.collect_diagnostics_from_files(ctx, self.files, true /*concurrent*/, &|ctx: &Context, file: P<SourceFile>| {
                    self.get_declaration_diagnostics_for_file(ctx, None, file)
                })
                .concat()
            }
        };
        filter_and_sort_diagnostics(result)
    }

    // Declaration emit needs the checker; without it declaration diagnostics are empty.
    #[cfg(not(feature = "checker"))]
    pub fn get_declaration_diagnostics(&'static self, _ctx: &Context, _source_file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        Vec::new()
    }

    // program.go:1626. `c` is the file's checker when the caller already holds it (Go `newEmitHost` takes it from
    // the pool); with `None` it is taken here.
    #[cfg(feature = "checker")]
    fn get_declaration_diagnostics_for_file(&'static self, ctx: &Context, c: Option<&mut Checker>, source_file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        if source_file.is_declaration_file.get() {
            return Vec::new();
        }

        if let Some(cached) = self.declaration_diagnostic_cache.lock().unwrap().get(&source_file) {
            return cached.clone();
        }

        let mut guard;
        let c = match c {
            Some(c) => c,
            None => {
                guard = self.get_type_checker_for_file(ctx, source_file);
                &mut *guard
            }
        };
        let checker_slot = P::new(tsrs_checker::CheckerSlot::default());
        let host = crate::emithost::new_emit_host(self, c.get_emit_resolver(), checker_slot);
        let diagnostics = checker_slot.lend(c, || crate::emitter::get_declaration_diagnostics(host, self, source_file));
        self.declaration_diagnostic_cache.lock().unwrap().entry(source_file).or_insert(diagnostics).clone()
    }

    // program.go:1488
    fn get_semantic_diagnostics_with_checker(&'static self, ctx: &Context, c: &mut Checker, source_file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        let mut result = filter_no_emit_semantic_diagnostics(
            self.get_bind_and_check_diagnostics_with_checker(ctx, c, source_file, false /*includeDeferredGlobals*/),
            &self.options(),
        );
        result.extend(self.get_include_processor_diagnostics(source_file));
        result
    }

    // getBindAndCheckDiagnosticsWithChecker gets semantic diagnostics for a single file using a
    // caller-provided checker, including bind diagnostics, checker diagnostics, and handling
    // of @ts-ignore/@ts-expect-error directives.
    // program.go:1498
    fn get_bind_and_check_diagnostics_with_checker(
        &self,
        ctx: &Context,
        file_checker: &mut Checker,
        source_file: P<SourceFile>,
        include_deferred_globals: bool,
    ) -> Vec<P<Diagnostic>> {
        let compiler_options = self.options();
        if self.skip_type_checking(source_file, false) {
            return Vec::new();
        }
        let mut previous_globals = Vec::new();
        if include_deferred_globals {
            previous_globals = file_checker.get_global_diagnostics();
        }

        // Checker creation forces binding, so bind diagnostics will be populated.
        let mut diags: Vec<P<Diagnostic>> = source_file.bind_diagnostics().to_vec();
        diags.extend(file_checker.get_diagnostics_exported(ctx, source_file));

        if include_deferred_globals {
            if file_checker.was_canceled() {
                return Vec::new();
            }
            let current_globals = file_checker.get_global_diagnostics();
            if current_globals.len() > previous_globals.len() {
                for diagnostic in current_globals {
                    let found = previous_globals.binary_search_by(|d| compare_diagnostics(*d, diagnostic).cmp(&0)).is_ok();
                    if !found {
                        diags.push(diagnostic);
                    }
                }
            }
        }

        let is_plain_js = ast::is_plain_js_file(Some(source_file), compiler_options.check_js);
        if is_plain_js {
            return diags.into_iter().filter(|d| plain_js_errors().contains(&d.code())).collect();
        }

        let is_js = source_file.script_kind.get() == ScriptKind::JS || source_file.script_kind.get() == ScriptKind::JSX;
        let is_check_js = is_js && ast::is_check_js_enabled_for_file(source_file, &compiler_options);
        if is_check_js {
            diags.extend_from_slice(source_file.jsdoc_diagnostics());
        }

        let (mut filtered, directives_by_line) = self.get_diagnostics_with_preceding_directives(source_file, diags);
        if let Some(directives_by_line) = directives_by_line {
            // Go iterates this map in random order; the result is sorted by the caller.
            for directive in directives_by_line.values() {
                // Above we changed all used directive kinds to @ts-ignore, so any @ts-expect-error directives that
                // remain are unused and thus errors.
                if directive.kind == CommentDirectiveKind::ExpectError {
                    filtered.push(new_diagnostic(Some(source_file), directive.loc, &diagnostics::Unused_ts_expect_error_directive, &[]));
                }
            }
        }
        filtered
    }

    // program.go:1642
    fn get_suggestion_diagnostics_with_checker(&self, ctx: &Context, file_checker: &mut Checker, source_file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        if self.skip_type_checking(source_file, false) {
            return Vec::new();
        }
        file_checker.get_suggestion_diagnostics(ctx, source_file)
    }

    fn get_diagnostics_with_preceding_directives(
        &self,
        source_file: P<SourceFile>,
        diags: Vec<P<Diagnostic>>,
    ) -> (Vec<P<Diagnostic>>, Option<FxHashMap<usize, ast::CommentDirective>>) {
        let comment_directives = source_file.comment_directives.get();
        if comment_directives.is_empty() {
            return (diags, None);
        }
        // Build map of directives by line number
        let mut directives_by_line: FxHashMap<usize, ast::CommentDirective> = FxHashMap::default();
        for directive in comment_directives.iter() {
            let line = tsrs_scanner::get_ecma_line_of_position(&*source_file, directive.loc.pos());
            directives_by_line.insert(line as usize, directive.clone());
        }
        let line_starts = tsrs_scanner::get_ecma_line_starts(&*source_file);
        let text = source_file.text();
        let mut filtered = Vec::with_capacity(diags.len());
        for diagnostic in diags {
            let mut ignore_diagnostic = false;
            if diagnostic.file() != Some(source_file) {
                filtered.push(diagnostic);
                continue;
            }
            let mut line = tsrs_scanner::compute_line_of_position(&line_starts, diagnostic.pos()) as i64 - 1;
            while line >= 0 {
                // If line contains a @ts-ignore or @ts-expect-error directive, ignore this diagnostic and change
                // the directive kind to @ts-ignore to indicate it was used.
                if let Some(directive) = directives_by_line.get_mut(&(line as usize)) {
                    ignore_diagnostic = true;
                    directive.kind = CommentDirectiveKind::Ignore;
                    break;
                }
                // Stop searching backwards when we encounter a line that isn't blank or a comment.
                if !is_comment_or_blank_line(text, line_starts[line as usize] as usize) {
                    break;
                }
                line -= 1;
            }
            if !ignore_diagnostic {
                filtered.push(diagnostic);
            }
        }
        (filtered, Some(directives_by_line))
    }

    pub fn line_count(&self) -> usize {
        // Most line maps are first computed here (--diagnostics / --extendedDiagnostics); they are independent.
        if self.single_threaded() {
            return self.files.iter().map(|f| f.ecma_line_map().len()).sum();
        }
        worker_pool().install(|| self.files.par_iter().map(|f| f.ecma_line_map().len()).sum())
    }

    pub fn identifier_count(&self) -> usize {
        self.files.iter().map(|f| f.identifier_count.get() as usize).sum()
    }

    pub fn symbol_count(&'static self) -> usize {
        let count: usize = self.files.iter().map(|f| f.symbol_count.get() as usize).sum();
        let val = std::sync::atomic::AtomicUsize::new(count);
        self.for_each_checker_parallel(|_, c| {
            val.fetch_add(c.symbol_count as usize, std::sync::atomic::Ordering::Relaxed);
        });
        val.into_inner()
    }

    pub fn type_count(&'static self) -> usize {
        let val = std::sync::atomic::AtomicUsize::new(0);
        self.for_each_checker_parallel(|_, c| {
            val.fetch_add(c.type_count as usize, std::sync::atomic::Ordering::Relaxed);
        });
        val.into_inner()
    }

    pub fn instantiation_count(&'static self) -> usize {
        let val = std::sync::atomic::AtomicUsize::new(0);
        self.for_each_checker_parallel(|_, c| {
            val.fetch_add(c.total_instantiation_count as usize, std::sync::atomic::Ordering::Relaxed);
        });
        val.into_inner()
    }

    pub fn lazy_member_stats(&'static self) -> tsrs_core::lazymembers::LazyMemberStats {
        let total = std::sync::Mutex::new(tsrs_core::lazymembers::LazyMemberStats::default());
        self.for_each_checker_parallel(|_, c| {
            total.lock().unwrap().add(&c.lazy_member_stats);
        });
        total.into_inner().unwrap()
    }

    // program.go:1743
    pub fn program(&self) -> &Program {
        self
    }

    pub fn get_source_file_meta_data(&self, path: &Path) -> SourceFileMetaData {
        self.source_file_meta_datas.get(path).cloned().unwrap_or_default()
    }

    pub fn get_emit_module_format_of_file(&self, source_file: P<SourceFile>) -> ModuleKind {
        ast::get_emit_module_format_of_file_worker(
            source_file.file_name(),
            &self.project_reference_file_mapper.get_compiler_options_for_file(source_file.file_name(), &source_file.path()),
            &self.get_source_file_meta_data(&source_file.path()),
        )
    }

    pub fn get_emit_syntax_for_usage_location(&self, source_file: P<SourceFile>, location: P<Node>) -> ResolutionMode {
        get_emit_syntax_for_usage_location_worker(
            source_file.file_name(),
            &self.get_source_file_meta_data(&source_file.path()),
            location,
            &self.project_reference_file_mapper.get_compiler_options_for_file(source_file.file_name(), &source_file.path()),
        )
    }

    pub fn get_implied_node_format_for_emit(&self, source_file: P<SourceFile>) -> ResolutionMode {
        ast::get_implied_node_format_for_emit_worker(
            source_file.file_name(),
            self.project_reference_file_mapper
                .get_compiler_options_for_file(source_file.file_name(), &source_file.path())
                .get_emit_module_kind(),
            &self.get_source_file_meta_data(&source_file.path()),
        )
    }

    pub fn get_mode_for_usage_location(&self, source_file: P<SourceFile>, location: P<Node>) -> ResolutionMode {
        get_mode_for_usage_location(
            source_file.file_name(),
            &self.get_source_file_meta_data(&source_file.path()),
            location,
            Some(&self.project_reference_file_mapper.get_compiler_options_for_file(source_file.file_name(), &source_file.path())),
        )
    }

    pub fn get_mode_for_resolution_at_index(&self, source_file: P<SourceFile>, index: usize) -> ResolutionMode {
        let imports = source_file.imports();
        if index < imports.len() {
            return self.get_mode_for_usage_location(source_file, imports[index]);
        }
        let mut index = index - imports.len();
        for &augmentation in source_file.module_augmentations.get() {
            if augmentation.kind() == Kind::StringLiteral {
                if index == 0 {
                    return self.get_mode_for_usage_location(source_file, augmentation);
                }
                index -= 1;
            }
        }
        panic!("resolution index out of range");
    }

    pub fn get_default_resolution_mode_for_file(&self, source_file: P<SourceFile>) -> ResolutionMode {
        get_default_resolution_mode_for_file(
            source_file.file_name(),
            &self.get_source_file_meta_data(&source_file.path()),
            &self.project_reference_file_mapper.get_compiler_options_for_file(source_file.file_name(), &source_file.path()),
        )
    }

    pub fn is_source_file_default_library(&self, path: &Path) -> bool {
        self.lib_files.contains_key(path)
    }

    pub fn is_global_typings_file(&self, file_name: &str) -> bool {
        if !tspath::is_declaration_file_name(file_name) {
            return false;
        }
        tspath::contains_path(self.get_global_typings_cache_location(), file_name, &self.compare_paths_options)
    }

    pub fn get_default_lib_file(&self, path: &Path) -> Option<P<LibFile>> {
        self.lib_files.get(path).copied()
    }

    pub fn common_source_directory(&self) -> &str {
        self.common_source_directory.get_or_init(|| {
            outputpaths::get_common_source_directory(
                &self.options(),
                || {
                    self.files
                        .iter()
                        .filter(|f| source_file_may_be_emitted(**f, self, false /*forceDtsEmit*/, false /*forceJsEmit*/) && !f.is_declaration_file.get())
                        .map(|f| f.file_name().to_string())
                        .collect()
                },
                self.get_current_directory(),
                self.use_case_sensitive_file_names(),
                Some(&|source_files: &[String], root_directory: &str| self.check_source_files_belong_to_path(source_files, root_directory)),
            )
        })
    }

    fn check_source_files_belong_to_path(&self, source_files: &[String], root_directory: &str) -> bool {
        let mut all_files_belong_to_path = true;
        for file in source_files {
            let absolute_source_file_path = tspath::get_canonical_file_name(
                &tspath::get_normalized_absolute_path(file, self.get_current_directory()),
                self.use_case_sensitive_file_names(),
            );
            if !tspath::contains_path(root_directory, file, &self.compare_paths_options) {
                self.add_processing_diagnostic(processingDiagnostic::explaining(includeExplainingDiagnostic {
                    file: Some(Path::from(absolute_source_file_path)),
                    diagnostic_reason: None,
                    message: &diagnostics::File_0_is_not_under_rootDir_1_rootDir_is_expected_to_contain_all_source_files,
                    args: vec![file.clone(), root_directory.to_string()],
                }));
                all_files_belong_to_path = false;
            }
        }
        all_files_belong_to_path
    }

    pub fn to_path(&self, filename: &str) -> Path {
        tspath::to_path(filename, self.get_current_directory(), self.use_case_sensitive_file_names())
    }

    pub fn get_source_file(&self, filename: &str) -> Option<P<SourceFile>> {
        let path = self.to_path(filename);
        self.get_source_file_by_path(&path)
    }

    pub fn get_source_file_for_resolved_module(&self, file_name: &str) -> Option<P<SourceFile>> {
        let file = self.get_source_file(file_name);
        if file.is_none() {
            let filename = self.get_parse_file_redirect(file_name);
            if !filename.is_empty() {
                return self.get_source_file(&filename);
            }
        }
        file
    }

    pub fn files_by_path(&self) -> &FxHashMap<Path, P<SourceFile>> {
        &self.files_by_path
    }

    pub fn get_source_file_by_path(&self, path: &Path) -> Option<P<SourceFile>> {
        self.files_by_path.get(path).copied()
    }

    // program.go:2103
    pub fn has_same_file_names(&self, other: &Program) -> bool {
        // checks for casing differences on case-insensitive file systems
        self.files_by_path.len() == other.files_by_path.len()
            && self.files_by_path.iter().all(|(path, a)| other.files_by_path.get(path).is_some_and(|b| a.file_name() == b.file_name()))
            && self.redirect_files_by_path.len() == other.redirect_files_by_path.len()
            && self
                .redirect_files_by_path
                .iter()
                .all(|(path, a)| other.redirect_files_by_path.get(path).is_some_and(|b| a.file_name == b.file_name))
    }

    // program.go:2112
    pub fn get_source_files(&self) -> &'static [P<SourceFile>] {
        self.files
    }

    // Testing only
    pub fn get_include_reasons(&self) -> &FxHashMap<Path, Vec<P<FileIncludeReason>>> {
        &self.file_include_data.file_include_reasons
    }

    // Testing only
    pub fn is_missing_path(&self, path: &Path) -> bool {
        self.missing_files.iter().any(|missing_path| self.to_path(missing_path) == *path)
    }

    pub fn explain_files(&self, w: &mut dyn Write) {
        let to_relative_file_name =
            |file_name: &str| tspath::get_relative_path_from_directory(self.get_current_directory(), file_name, &self.compare_paths_options);
        let mut files_explained = 0usize;
        let mut explain_file = |file_name: &str, path: &Path, files_explained: &mut usize| {
            let _ = writeln!(w, "{}", to_relative_file_name(file_name));
            for &reason in self.file_include_data.file_include_reasons.get(path).map(|v| v.as_slice()).unwrap_or(&[]) {
                let _ = writeln!(w, "   {}", file_include::to_diagnostic(reason, self, true).localize());
            }
            for diag in self.include_processor.explain_redirect_and_implied_format(self, path, &to_relative_file_name).unwrap_or_default() {
                let _ = writeln!(w, "   {}", diag.localize());
            }
            *files_explained += 1;
        };

        let mut redirect_files: Vec<&crate::fileloader::redirectsFile> = self.redirect_files_by_path.values().collect();
        redirect_files.sort_by_key(|r| r.index);

        let files = self.get_source_files();
        let mut source_file_index = 0;

        for redirect_file in &redirect_files {
            // Explain all sourceFiles till we reach this redirectFile index
            while files_explained < redirect_file.index {
                let f = files[source_file_index];
                explain_file(f.file_name(), &f.path(), &mut files_explained);
                source_file_index += 1;
            }
            explain_file(&redirect_file.file_name, &redirect_file.path, &mut files_explained);
        }

        // Explain any remaining sourceFiles
        while files_explained < files.len() + redirect_files.len() {
            let f = files[source_file_index];
            explain_file(f.file_name(), &f.path(), &mut files_explained);
            source_file_index += 1;
        }
    }

    pub fn get_lib_file_from_reference(&self, ref_: P<FileReference>) -> Option<P<SourceFile>> {
        let path = tsoptions::get_lib_file_name(&ref_.file_name)?;
        self.files_by_path.get(&Path::from(path)).copied()
    }

    pub fn get_resolved_type_reference_directive_from_type_reference_directive(
        &self,
        type_ref: P<FileReference>,
        source_file: P<SourceFile>,
    ) -> Option<P<ResolvedTypeReferenceDirective>> {
        self.get_resolved_type_reference_directive(
            source_file,
            &type_ref.file_name,
            self.get_mode_for_type_reference_directive_in_file(type_ref, source_file),
        )
    }

    pub fn get_resolved_type_reference_directive(
        &self,
        file: P<SourceFile>,
        type_directive_name: &str,
        mode: ResolutionMode,
    ) -> Option<P<ResolvedTypeReferenceDirective>> {
        let resolutions = self.type_resolutions_in_file.get(file.path())?;
        resolutions.iter().find(|(k, _)| k.name == type_directive_name && k.mode == mode).map(|(_, v)| *v)
    }

    pub fn get_resolved_type_reference_directives(&self) -> &FxHashMap<Path, ModeAwareCache<P<ResolvedTypeReferenceDirective>>> {
        &self.type_resolutions_in_file
    }

    fn get_mode_for_type_reference_directive_in_file(&self, ref_: P<FileReference>, source_file: P<SourceFile>) -> ResolutionMode {
        if ref_.resolution_mode != ModuleKind::None {
            return ref_.resolution_mode;
        }
        self.get_default_resolution_mode_for_file(source_file)
    }

    pub fn is_source_file_from_external_library(&self, file: P<SourceFile>) -> bool {
        self.source_files_found_searching_node_modules.contains(file.path())
    }

    pub fn get_jsx_runtime_import_specifier(&self, path: &Path) -> (String, Option<P<Node>>) {
        match self.jsx_runtime_import_specifiers.get(path) {
            Some(result) => (result.module_reference.clone(), Some(result.specifier)),
            None => (String::new(), None),
        }
    }

    pub fn get_import_helpers_import_specifier(&self, path: &Path) -> Option<P<Node>> {
        self.import_helpers_import_specifiers.get(path).copied()
    }

    pub fn source_file_may_be_emitted(&self, source_file: P<SourceFile>, force_dts_emit: bool) -> bool {
        source_file_may_be_emitted(source_file, self, force_dts_emit, false)
    }

    // program.go:2222
    pub fn resolved_package_names(&self) -> &Set<String> {
        &self.collect_package_names().resolved
    }

    // program.go:2226
    pub fn unresolved_package_names(&self) -> &Set<String> {
        &self.collect_package_names().unresolved
    }

    // program.go:2230
    pub fn deep_import_package_names(&self) -> &Set<String> {
        &self.collect_package_names().deep_import_packages
    }

    // program.go:2234
    fn collect_package_names(&self) -> &packageNamesInfo {
        self.package_names.get_or_init(|| {
            let resolver = self.new_resolver();
            let mut package_names = packageNamesInfo { resolved: Set::default(), unresolved: Set::default(), deep_import_packages: Set::default() };
            for &file in self.files {
                if self.is_source_file_default_library(file.path()) || self.is_source_file_from_external_library(file) || file.file_name().contains("/node_modules/") {
                    // Checking for /node_modules/ is a little imprecise, but ATA treats locally installed typings
                    // as root files, which would not pass IsSourceFileFromExternalLibrary.
                    continue;
                }
                'imports: for &imp in file.imports() {
                    if tspath::is_external_module_name_relative(imp.text()) {
                        continue;
                    }
                    if self.resolved_modules.contains_key(file.path()) {
                        let mode = self.get_mode_for_usage_location(file, imp);
                        if let Some(resolved_module) = self.get_resolved_module(file, imp.text(), mode).filter(|r| r.is_resolved()) {
                            if !resolved_module.is_external_library_import {
                                continue 'imports;
                            }
                            // Priority order for getting package name:
                            // 1. PackageId.Name (requires both name and version in package.json)
                            let mut name = resolved_module.package_id.name.to_string();
                            if name.is_empty() {
                                // 2. GetPackageScopeForPath - get name from package.json in the package directory
                                if let Some(package_scope) = resolver.get_package_scope_for_path(&resolved_module.resolved_file_name) {
                                    if package_scope.exists() {
                                        if let Some(scope_name) = package_scope.contents.and_then(|c| c.get().name.get_value()) {
                                            name = scope_name.to_string();
                                        }
                                    }
                                }
                            }
                            if name.is_empty() {
                                // 3. GetPackageNameFromDirectory - extract from node_modules path
                                name = tsrs_modulespecifiers::get_package_name_from_directory(&resolved_module.resolved_file_name);
                            }
                            // 4. If all fail, don't add empty string
                            if !name.is_empty() {
                                package_names.resolved.add(name.clone());
                                // Detect deep imports: subpath imports in packages without exports.
                                // These are imports like "lodash/fp" where the package has no exports
                                // map, so auto-import can only find them via recursive directory search.
                                let (_, rest) = module::parse_package_name(imp.text());
                                if !rest.is_empty() {
                                    if let Some(scope) = resolver.get_package_scope_for_path(&resolved_module.resolved_file_name) {
                                        if scope.exists() && !scope.contents.is_some_and(|c| c.exports.is_present()) {
                                            package_names.deep_import_packages.add(module::get_package_name_from_types_package_name(&name));
                                        }
                                    }
                                }
                            }
                            continue 'imports;
                        }
                    }
                    package_names.unresolved.add(imp.text().to_string());
                }
            }
            Arc::new(package_names)
        })
    }

    pub fn is_lib_file(&self, source_file: P<SourceFile>) -> bool {
        self.lib_files.contains_key(source_file.path())
    }

    // program.go:2297
    pub fn has_ts_file(&self) -> bool {
        *self.has_ts_file.get_or_init(|| self.files.iter().any(|f| tspath::has_implementation_ts_file_extension(f.file_name())))
    }

    pub fn get_symlink_cache(&self) -> P<KnownSymlinks> {
        if let Some(&k) = self.known_symlinks.get() {
            return k;
        }
        // `UpdateProgram` hands the cache to the next program version, which shares `processed`: in the language
        // server it lives in the region that owns `processed` (the full build's), not in this version's.
        let _region = tsrs_core::arena::enter_owner(self.processed as *const processedFiles as usize);
        *self.known_symlinks.get_or_init(|| {
            let resolver = self.new_resolver();
            let known_symlinks = symlinks::new_known_symlink(self.get_current_directory(), self.use_case_sensitive_file_names());

            // Resolved modules store realpath information when they're resolved inside node_modules
            if !self.resolved_modules.is_empty() || !self.type_resolutions_in_file.is_empty() {
                known_symlinks.set_symlinks_from_resolutions(
                    |callback, file| self.for_each_resolved_module(callback, file),
                    |callback, file| self.for_each_resolved_type_reference_directive(callback, file),
                );
            }

            // Check other dependencies for symlinks
            let mut seen_package_jsons: tsrs_core::collections::Set<Path> = tsrs_core::collections::Set::default();
            for (file_path, meta) in &self.source_file_meta_datas {
                if meta.package_json_directory.is_empty()
                    || !self.source_file_may_be_emitted(self.get_source_file_by_path(file_path).unwrap(), false)
                    || !seen_package_jsons.add_if_absent(self.to_path(&meta.package_json_directory))
                {
                    continue;
                }
                let package_json_name = tspath::combine_paths(&meta.package_json_directory, &["package.json"]);
                let info = self.get_package_json_info(&package_json_name);
                let Some(contents) = info.and_then(|info| info.get_contents()) else {
                    continue;
                };

                for dep in contents.get_runtime_dependency_names().keys() {
                    // Skip work in common case: we already saved a symlink for this package directory
                    // in the node_modules adjacent to this package.json
                    let possible_directory_path = self.to_path(&tspath::combine_paths(&meta.package_json_directory, &["node_modules", dep]));
                    if known_symlinks.has_directory(&possible_directory_path) {
                        continue;
                    }
                    if !dep.starts_with("@types") {
                        let possible_types_directory_path = self.to_path(&tspath::combine_paths(
                            &meta.package_json_directory,
                            &["node_modules", &module::get_types_package_name(dep)],
                        ));
                        if known_symlinks.has_directory(&possible_types_directory_path) {
                            continue;
                        }
                    }

                    if let Some(package_resolution) = resolver
                        .resolve_package_directory(dep, &package_json_name, ModuleKind::CommonJS, None)
                        .filter(|r| r.is_resolved() && !r.original_path.is_empty())
                    {
                        known_symlinks.process_resolution(
                            &tspath::combine_paths(package_resolution.original_path, &["package.json"]),
                            &tspath::combine_paths(package_resolution.resolved_file_name, &["package.json"]),
                        );
                    }
                }
            }
            P::new(known_symlinks)
        })
    }

    pub fn for_each_resolved_module(
        &self,
        callback: &mut dyn FnMut(&ResolvedModule, &str, ResolutionMode, &Path),
        file: Option<P<SourceFile>>,
    ) {
        for_each_resolution(&self.resolved_modules, |resolution, module_name, mode, file_path| callback(resolution, module_name, mode, file_path), file);
    }

    pub fn for_each_resolved_type_reference_directive(
        &self,
        callback: &mut dyn FnMut(&ResolvedTypeReferenceDirective, &str, ResolutionMode, &Path),
        file: Option<P<SourceFile>>,
    ) {
        for_each_resolution(
            &self.type_resolutions_in_file,
            |resolution, module_name, mode, file_path| callback(resolution, module_name, mode, file_path),
            file,
        );
    }
}

fn for_each_resolution<T>(
    resolution_cache: &FxHashMap<Path, ModeAwareCache<P<T>>>,
    mut callback: impl FnMut(&T, &str, ResolutionMode, &Path),
    file: Option<P<SourceFile>>,
) {
    if let Some(file) = file {
        if let Some(resolutions) = resolution_cache.get(file.path()) {
            for (key, resolution) in resolutions {
                callback(resolution, key.name, key.mode, file.path());
            }
        }
    } else {
        for (file_path, resolutions) in resolution_cache {
            for (key, resolution) in resolutions {
                callback(resolution, key.name, key.mode, file_path);
            }
        }
    }
}

// program.go:84 `lazyValue.tryReuse`.
fn try_reuse<T: Clone>(to: &OnceLock<T>, from: &OnceLock<T>) {
    if let Some(value) = from.get() {
        let _ = to.set(value.clone());
    }
}

// Go `slices.EqualFunc`.
fn equal_func<T: Copy>(a: &[T], b: &[T], mut eq: impl FnMut(T, T) -> bool) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(&x, &y)| eq(x, y))
}

// program.go:503
fn equal_module_specifiers(n1: P<Node>, n2: P<Node>) -> bool {
    n1.kind() == n2.kind() && (!ast::is_string_literal(n1) || n1.text() == n2.text())
}

// program.go:507
fn equal_module_augmentation_names(n1: P<Node>, n2: P<Node>) -> bool {
    n1.kind() == n2.kind() && n1.text() == n2.text()
}

// program.go:511
fn equal_file_references(f1: P<FileReference>, f2: P<FileReference>) -> bool {
    f1.file_name == f2.file_name && f1.resolution_mode == f2.resolution_mode && f1.preserve == f2.preserve
}

// program.go:515
fn equal_check_js_directives(d1: Option<P<ast::CheckJsDirective>>, d2: Option<P<ast::CheckJsDirective>>) -> bool {
    match (d1, d2) {
        (None, None) => true,
        (Some(d1), Some(d2)) => d1.enabled == d2.enabled,
        _ => false,
    }
}

fn filter_and_sort_diagnostics(diags: Vec<P<Diagnostic>>) -> Vec<P<Diagnostic>> {
    // Content-mapped files (span maps) are not ported, so no diagnostic is filtered out here.
    sort_and_deduplicate_diagnostics(&diags)
}

// getAdditionalJSSyntacticDiagnostics produces option-dependent syntactic diagnostics for JS files
// that aren't covered by the parser or the checker. The checker handles these for checked files,
// but doesn't run on unchecked JS files (no checkJs/ts-check).
fn get_additional_js_syntactic_diagnostics(file: P<SourceFile>, options: &CompilerOptions) -> Vec<P<Diagnostic>> {
    if options.experimental_decorators.is_true() {
        return Vec::new();
    }
    let mut diags = Vec::new();
    // Parameter decorators are only valid with experimentalDecorators. Without it,
    // the checker would report this, but the checker doesn't run on unchecked JS files.
    fn walk(node: P<Node>, file: P<SourceFile>, diags: &mut Vec<P<Diagnostic>>) -> bool {
        if !node.subtree_facts().intersects(ast::SubtreeFacts::ContainsDecorators) {
            return false;
        }
        if node.kind() == Kind::Parameter && ast::has_decorators(node) {
            if let Some(decorator) = node.modifier_nodes().iter().copied().find(|n| ast::is_decorator(*n)) {
                diags.push(new_diagnostic(Some(file), decorator.loc(), &diagnostics::Decorators_are_not_valid_here, &[]));
            }
        }
        node.for_each_child(&mut |child| walk(child, file, diags));
        false
    }
    file.as_node().for_each_child(&mut |child| walk(child, file, &mut diags));
    diags
}

pub fn filter_no_emit_semantic_diagnostics(diagnostics: Vec<P<Diagnostic>>, options: &CompilerOptions) -> Vec<P<Diagnostic>> {
    if !options.no_emit.is_true() {
        return diagnostics;
    }
    diagnostics.into_iter().filter(|d| !d.skipped_on_no_emit()).collect()
}

fn has_zero_or_one_asterisk_character(str: &str) -> bool {
    str.bytes().filter(|&c| c == b'*').count() <= 1
}

fn module_resolution_supports_package_json_exports_and_imports(module_resolution: ModuleResolutionKind) -> bool {
    module_resolution >= ModuleResolutionKind::Node16 && module_resolution <= ModuleResolutionKind::NodeNext
        || module_resolution == ModuleResolutionKind::Bundler
}

fn emit_module_kind_is_non_node_esm(module_kind: ModuleKind) -> bool {
    module_kind >= ModuleKind::ES2015 && module_kind <= ModuleKind::ESNext
}

fn is_comment_or_blank_line(text: &str, pos: usize) -> bool {
    let text = text.as_bytes();
    let mut pos = pos;
    while pos < text.len() && (text[pos] == b' ' || text[pos] == b'\t') {
        pos += 1;
    }
    pos == text.len()
        || pos < text.len() && (text[pos] == b'\r' || text[pos] == b'\n')
        || pos + 1 < text.len() && text[pos] == b'/' && text[pos + 1] == b'/'
}

pub fn sort_and_deduplicate_diagnostics(diagnostics: &[P<Diagnostic>]) -> Vec<P<Diagnostic>> {
    let mut diagnostics = diagnostics.to_vec();
    // Go's slices.SortFunc is not stable; equal diagnostics are merged below, so stability does not matter.
    diagnostics.sort_by(|a, b| compare_diagnostics(*a, *b).cmp(&0));
    compact_and_merge_related_infos(diagnostics)
}

// Remove duplicate diagnostics and, for sequences of diagnostics that differ only by related information,
// create a single diagnostic with sorted and deduplicated related information.
fn compact_and_merge_related_infos(diagnostics: Vec<P<Diagnostic>>) -> Vec<P<Diagnostic>> {
    if diagnostics.len() < 2 {
        return diagnostics;
    }
    let mut result = Vec::with_capacity(diagnostics.len());
    let mut i = 0;
    while i < diagnostics.len() {
        let mut d = diagnostics[i];
        let mut n = 1;
        while i + n < diagnostics.len() && equal_diagnostics_no_related_info(d, diagnostics[i + n]) {
            n += 1;
        }
        if n > 1 {
            let mut related_infos: Vec<P<Diagnostic>> = Vec::new();
            let mut any = false;
            for k in 0..n {
                let r = diagnostics[i + k].related_information();
                if !r.is_empty() {
                    any = true;
                }
                related_infos.extend_from_slice(r);
            }
            if any {
                related_infos.sort_by(|a, b| compare_diagnostics(*a, *b).cmp(&0));
                related_infos.dedup_by(|a, b| equal_diagnostics(*a, *b));
                d = d.clone_diagnostic().set_related_info(&related_infos);
            }
        }
        result.push(d);
        i += n;
    }
    result
}

// GetDiagnosticsOfAnyProgram collects config, syntactic, program, global and semantic diagnostics
// in tsc's order, stopping at the first phase that produced diagnostics.
// program.go:2018
pub fn get_diagnostics_of_any_program(
    ctx: &Context,
    program: &'static Program,
    files: Option<&[P<SourceFile>]>,
    skip_no_emit_check_for_dts_diagnostics: bool,
    get_bind_diagnostics: &mut dyn FnMut(&Context, Option<P<SourceFile>>) -> Vec<P<Diagnostic>>,
    get_semantic_diagnostics: &mut dyn FnMut(&Context, Option<P<SourceFile>>) -> Vec<P<Diagnostic>>,
) -> Vec<P<Diagnostic>> {
    let mut all_diagnostics = program.get_config_file_parsing_diagnostics();
    let config_file_parsing_diagnostics_length = all_diagnostics.len();

    let append_diagnostics_for_all_files =
        |diagnostics: &mut Vec<P<Diagnostic>>, get_diagnostics: &mut dyn FnMut(&Context, Option<P<SourceFile>>) -> Vec<P<Diagnostic>>| match files {
            None => diagnostics.extend(get_diagnostics(ctx, None)),
            Some(files) => {
                for &file in files {
                    diagnostics.extend(get_diagnostics(ctx, Some(file)));
                }
            }
        };

    let mut syntactic_diagnostics = Vec::new();
    let start = std::time::Instant::now();
    append_diagnostics_for_all_files(&mut syntactic_diagnostics, &mut |ctx, f| program.get_syntactic_diagnostics(ctx, f));
    tsrs_core::phases::record("Diagnostics: syntactic", start.elapsed());
    all_diagnostics.extend(syntactic_diagnostics);

    // If we didn't have any syntactic errors, then also try getting the program (options),
    // global and semantic errors.
    if all_diagnostics.len() == config_file_parsing_diagnostics_length {
        all_diagnostics.extend(tsrs_core::phases::time("Diagnostics: program", || program.get_program_diagnostics()));

        // Do binding early so we can track the time.
        append_diagnostics_for_all_files(&mut Vec::new(), get_bind_diagnostics);

        if program.options().list_files_only.is_false_or_unknown() {
            all_diagnostics.extend(tsrs_core::phases::time("Diagnostics: global (first)", || program.get_global_diagnostics(ctx)));

            if all_diagnostics.len() == config_file_parsing_diagnostics_length {
                append_diagnostics_for_all_files(&mut all_diagnostics, get_semantic_diagnostics);
                // Incremental programs cache checking globals with file diagnostics;
                // a late sweep would also collect incidental signature-generation globals.
                all_diagnostics.extend(tsrs_core::phases::time("Diagnostics: global (after)", || program.get_global_diagnostics(ctx)));
                #[cfg(feature = "checker")]
                crate::checkerpool::write_file_times(program);
            }

            if (skip_no_emit_check_for_dts_diagnostics || program.options().no_emit.is_true())
                && program.options().get_emit_declarations()
                && all_diagnostics.len() == config_file_parsing_diagnostics_length
            {
                append_diagnostics_for_all_files(&mut all_diagnostics, &mut |ctx, f| program.get_declaration_diagnostics(ctx, f));
            }
        }
    }
    #[cfg(feature = "checker")]
    crate::checkerpool::write_cost_cache(program);
    all_diagnostics
}

fn plain_js_errors() -> &'static FxHashSet<i32> {
    static SET: OnceLock<FxHashSet<i32>> = OnceLock::new();
    SET.get_or_init(|| {
        [
            // binder errors
            &diagnostics::Cannot_redeclare_block_scoped_variable_0,
            &diagnostics::A_module_cannot_have_multiple_default_exports,
            &diagnostics::Another_export_default_is_here,
            &diagnostics::The_first_export_default_is_here,
            &diagnostics::Identifier_expected_0_is_a_reserved_word_at_the_top_level_of_a_module,
            &diagnostics::Identifier_expected_0_is_a_reserved_word_in_strict_mode_Modules_are_automatically_in_strict_mode,
            &diagnostics::Identifier_expected_0_is_a_reserved_word_that_cannot_be_used_here,
            &diagnostics::X_constructor_is_a_reserved_word,
            &diagnostics::X_delete_cannot_be_called_on_an_identifier_in_strict_mode,
            &diagnostics::Code_contained_in_a_class_is_evaluated_in_JavaScript_s_strict_mode_which_does_not_allow_this_use_of_0_For_more_information_see_https_Colon_Slash_Slashdeveloper_mozilla_org_Slashen_US_Slashdocs_SlashWeb_SlashJavaScript_SlashReference_SlashStrict_mode,
            &diagnostics::Invalid_use_of_0_Modules_are_automatically_in_strict_mode,
            &diagnostics::Invalid_use_of_0_in_strict_mode,
            &diagnostics::A_label_is_not_allowed_here,
            &diagnostics::X_with_statements_are_not_allowed_in_strict_mode,
            // grammar errors
            &diagnostics::A_break_statement_can_only_be_used_within_an_enclosing_iteration_or_switch_statement,
            &diagnostics::A_break_statement_can_only_jump_to_a_label_of_an_enclosing_statement,
            &diagnostics::A_class_declaration_without_the_default_modifier_must_have_a_name,
            &diagnostics::A_class_member_cannot_have_the_0_keyword,
            &diagnostics::A_comma_expression_is_not_allowed_in_a_computed_property_name,
            &diagnostics::A_continue_statement_can_only_be_used_within_an_enclosing_iteration_statement,
            &diagnostics::A_continue_statement_can_only_jump_to_a_label_of_an_enclosing_iteration_statement,
            &diagnostics::A_default_clause_cannot_appear_more_than_once_in_a_switch_statement,
            &diagnostics::A_default_export_must_be_at_the_top_level_of_a_file_or_module_declaration,
            &diagnostics::A_definite_assignment_assertion_is_not_permitted_in_this_context,
            &diagnostics::A_destructuring_declaration_must_have_an_initializer,
            &diagnostics::A_get_accessor_cannot_have_parameters,
            &diagnostics::A_rest_element_cannot_contain_a_binding_pattern,
            &diagnostics::A_rest_element_cannot_have_a_property_name,
            &diagnostics::A_rest_element_cannot_have_an_initializer,
            &diagnostics::A_rest_element_must_be_last_in_a_destructuring_pattern,
            &diagnostics::A_rest_parameter_cannot_have_an_initializer,
            &diagnostics::A_rest_parameter_must_be_last_in_a_parameter_list,
            &diagnostics::A_rest_parameter_or_binding_pattern_may_not_have_a_trailing_comma,
            &diagnostics::A_return_statement_cannot_be_used_inside_a_class_static_block,
            &diagnostics::A_set_accessor_cannot_have_rest_parameter,
            &diagnostics::A_set_accessor_must_have_exactly_one_parameter,
            &diagnostics::An_export_declaration_can_only_be_used_at_the_top_level_of_a_module,
            &diagnostics::An_export_declaration_cannot_have_modifiers,
            &diagnostics::An_import_declaration_can_only_be_used_at_the_top_level_of_a_module,
            &diagnostics::An_import_declaration_cannot_have_modifiers,
            &diagnostics::An_object_member_cannot_be_declared_optional,
            &diagnostics::Argument_of_dynamic_import_cannot_be_spread_element,
            &diagnostics::Cannot_assign_to_private_method_0_Private_methods_are_not_writable,
            &diagnostics::Cannot_redeclare_identifier_0_in_catch_clause,
            &diagnostics::Catch_clause_variable_cannot_have_an_initializer,
            &diagnostics::Class_decorators_can_t_be_used_with_static_private_identifier_Consider_removing_the_experimental_decorator,
            &diagnostics::Classes_can_only_extend_a_single_class,
            &diagnostics::Classes_may_not_have_a_field_named_constructor,
            &diagnostics::Did_you_mean_to_use_a_Colon_An_can_only_follow_a_property_name_when_the_containing_object_literal_is_part_of_a_destructuring_pattern,
            &diagnostics::Duplicate_label_0,
            &diagnostics::Dynamic_imports_can_only_accept_a_module_specifier_and_an_optional_set_of_attributes_as_arguments,
            &diagnostics::X_for_await_loops_cannot_be_used_inside_a_class_static_block,
            &diagnostics::JSX_attributes_must_only_be_assigned_a_non_empty_expression,
            &diagnostics::JSX_elements_cannot_have_multiple_attributes_with_the_same_name,
            &diagnostics::JSX_expressions_may_not_use_the_comma_operator_Did_you_mean_to_write_an_array,
            &diagnostics::JSX_property_access_expressions_cannot_include_JSX_namespace_names,
            &diagnostics::Jump_target_cannot_cross_function_boundary,
            &diagnostics::Line_terminator_not_permitted_before_arrow,
            &diagnostics::Modifiers_cannot_appear_here,
            &diagnostics::Only_a_single_variable_declaration_is_allowed_in_a_for_in_statement,
            &diagnostics::Only_a_single_variable_declaration_is_allowed_in_a_for_of_statement,
            &diagnostics::Private_identifiers_are_not_allowed_outside_class_bodies,
            &diagnostics::Private_identifiers_are_only_allowed_in_class_bodies_and_may_only_be_used_as_part_of_a_class_member_declaration_property_access_or_on_the_left_hand_side_of_an_in_expression,
            &diagnostics::Property_0_is_not_accessible_outside_class_1_because_it_has_a_private_identifier,
            &diagnostics::Tagged_template_expressions_are_not_permitted_in_an_optional_chain,
            &diagnostics::The_left_hand_side_of_a_for_of_statement_may_not_be_async,
            &diagnostics::The_variable_declaration_of_a_for_in_statement_cannot_have_an_initializer,
            &diagnostics::The_variable_declaration_of_a_for_of_statement_cannot_have_an_initializer,
            &diagnostics::Trailing_comma_not_allowed,
            &diagnostics::Variable_declaration_list_cannot_be_empty,
            &diagnostics::X_0_and_1_operations_cannot_be_mixed_without_parentheses,
            &diagnostics::X_0_expected,
            &diagnostics::X_0_is_not_a_valid_meta_property_for_keyword_1_Did_you_mean_2,
            &diagnostics::X_0_list_cannot_be_empty,
            &diagnostics::X_0_modifier_already_seen,
            &diagnostics::X_0_modifier_cannot_appear_on_a_constructor_declaration,
            &diagnostics::X_0_modifier_cannot_appear_on_a_module_or_namespace_element,
            &diagnostics::X_0_modifier_cannot_appear_on_a_parameter,
            &diagnostics::X_0_modifier_cannot_appear_on_class_elements_of_this_kind,
            &diagnostics::X_0_modifier_cannot_be_used_here,
            &diagnostics::X_0_modifier_must_precede_1_modifier,
            &diagnostics::X_0_declarations_can_only_be_declared_inside_a_block,
            &diagnostics::X_0_declarations_must_be_initialized,
            &diagnostics::X_extends_clause_already_seen,
            &diagnostics::X_let_is_not_allowed_to_be_used_as_a_name_in_let_or_const_declarations,
            &diagnostics::Class_constructor_may_not_be_a_generator,
            &diagnostics::Class_constructor_may_not_be_an_accessor,
            &diagnostics::X_await_expressions_are_only_allowed_within_async_functions_and_at_the_top_levels_of_modules,
            &diagnostics::X_await_using_statements_are_only_allowed_within_async_functions_and_at_the_top_levels_of_modules,
            &diagnostics::Private_field_0_must_be_declared_in_an_enclosing_class,
            // Type errors
            &diagnostics::This_condition_will_always_return_0_since_JavaScript_compares_objects_by_reference_not_value,
        ]
        .iter()
        .map(|m| m.code())
        .collect()
    })
}

impl tsoptions::outputpaths::OutputPathsHost for Program {
    fn common_source_directory(&self) -> String {
        Program::common_source_directory(self).to_string()
    }

    fn content_mapper_extensions(&self) -> Vec<String> {
        self.opts.config.content_mapper_extensions()
    }

    fn get_current_directory(&self) -> &str {
        Program::get_current_directory(self)
    }

    fn use_case_sensitive_file_names(&self) -> bool {
        Program::use_case_sensitive_file_names(self)
    }
}
