// Port of Go `execute/watcher.go`: `tsc --watch` for one project. Each cycle drains the file system events, decides
// whether they matter to the program (the paths the last build read, wildcard matches, watched directories), and
// rebuilds: in place when exactly one already-parsed file changed (`Program::reuse_program`), otherwise with a full
// `new_program` over a source file cache keyed by modification time. Either way the incremental program carries the
// old one's diagnostics and signatures, so only affected files are checked again.

use std::sync::Arc;
use std::time::SystemTime;

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{new_compiler_diagnostic, SourceFile, SourceFileParseOptions};
use tsrs_compiler::{new_compiler_host, new_program, CompilerHost, Program, ProgramOptions};
use tsrs_contentmapper as contentmapper;
use tsrs_core::collections::SyncMap;
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{CompilerOptions, ScriptKind, P};
use tsrs_diagnostics as diagnostics;
use tsrs_fswatch::EventKind;
use tsrs_tsoptions::{self as tsoptions, CompilerOptionsValue, Mapper, ParsedCommandLine};
use tsrs_vfs::{cachedvfs, trackingvfs, FS};

use crate::tsc::{self, CommandLineTesting, DiagnosticReporter, DiagnosticsReporter, EmitInput, ExtendedConfigCache, System};
use crate::watchmanager::{self, DirWatchSet, WatchManager, WatchState};

#[derive(Clone, Copy)]
struct cachedSourceFile {
    file: P<SourceFile>,
    mod_time: Option<SystemTime>,
}

type SourceFileCache = Arc<SyncMap<Path, cachedSourceFile>>;

/// Go `watchCompilerHost`: the compiler host, with parsed files reused while their modification time is unchanged.
struct watchCompilerHost {
    inner: Arc<dyn CompilerHost>,
    cache: SourceFileCache,
}

impl CompilerHost for watchCompilerHost {
    fn fs(&self) -> &dyn FS {
        self.inner.fs()
    }
    fn default_library_path(&self) -> &str {
        self.inner.default_library_path()
    }
    fn get_current_directory(&self) -> &str {
        self.inner.get_current_directory()
    }
    fn trace(&self, msg: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]) {
        self.inner.trace(msg, args);
    }
    fn get_source_file(&self, opts: SourceFileParseOptions) -> Option<P<SourceFile>> {
        let info = self.inner.fs().stat(&opts.file_name);
        if let Some(cached) = self.cache.load(&opts.path)
            && let Some(info) = &info
            && info.mod_time() == cached.mod_time
        {
            return Some(cached.file);
        }
        let path = opts.path.clone();
        let file = self.inner.get_source_file(opts);
        match file {
            Some(file) => {
                if let Some(info) = info {
                    self.cache.store(path, cachedSourceFile { file, mod_time: info.mod_time() });
                }
            }
            None => self.cache.delete(&path),
        }
        file
    }
    fn get_content_mapped_source_files(
        &self,
        parse_options: SourceFileParseOptions,
        mapper: &Mapper,
    ) -> Result<contentmapper::SourceFiles, contentmapper::Error> {
        self.inner.get_content_mapped_source_files(parse_options, mapper)
    }
    fn content_mapper_project(&self) -> Option<Arc<dyn contentmapper::Project>> {
        self.inner.content_mapper_project()
    }
    fn get_resolved_project_reference(&self, file_name: &str, path: Path) -> Option<P<ParsedCommandLine>> {
        self.inner.get_resolved_project_reference(file_name, path)
    }
}

pub struct Watcher {
    sys: &'static dyn System,
    // tsrs-only: what the programs since the last full build share (`install_program`).
    shared_program_data: Option<tsrs_compiler::SharedProgramData>,
    // Frees what the last `install_program` replaced, once the cycle has reported (`free_replaced_program`).
    free_replaced: Option<Box<dyn FnOnce() + Send>>,
    config_file_name: String,
    config: P<ParsedCommandLine>,
    compiler_options_from_command_line: P<CompilerOptions>,
    command_line_raw: Option<CompilerOptionsValue>,
    report_diagnostic: DiagnosticReporter<'static>,
    report_error_summary: DiagnosticsReporter<'static>,
    report_watch_status: DiagnosticReporter<'static>,
    testing: Option<&'static dyn CommandLineTesting>,

    // Created once per watch session (when enabled) and reused across cycles.
    content_mapper_host: Option<Arc<dyn contentmapper::Host>>,
    content_mapper_project: Option<Arc<dyn contentmapper::Project>>,

    program: Option<P<tsrs_incremental::Program>>,
    extended_config_cache: Arc<ExtendedConfigCache>,
    config_modified: bool,
    config_has_errors: bool,
    config_file_paths: Vec<String>,

    source_file_cache: SourceFileCache,

    wm: Arc<WatchManager>,
    seen_files: FxHashSet<Path>, // all build dependencies (for event filtering)
    config_mtimes: FxHashMap<String, Option<SystemTime>>,
    watch_set_dirty: bool,
    // A reason that requires a full `new_program` rebuild (an overflow, a mid-cycle watch failure, a newly appeared
    // project file, a changed non-source dependency). Unlike `watch_set_dirty` it is kept until a full rebuild runs, so
    // the single-file fast path cannot reuse a stale program.
    force_full_rebuild: bool,
    program_ready: bool,

    // Test-only observability of which build path was taken.
    pub fast_path_builds: usize,
    pub full_builds: usize,
}

impl tsc::Watcher for Watcher {
    fn do_cycle(&mut self) {
        Watcher::do_cycle(self);
    }
}

/// Go `createWatcher` + `start`. Without the test harness this blocks, running cycles until the process ends.
#[expect(clippy::too_many_arguments, reason = "Go's createWatcher parameters")]
pub(crate) fn run_watcher(
    sys: &'static dyn System,
    config: P<ParsedCommandLine>,
    compiler_options_from_command_line: P<CompilerOptions>,
    command_line_raw: Option<CompilerOptionsValue>,
    report_diagnostic: DiagnosticReporter<'static>,
    report_error_summary: DiagnosticsReporter<'static>,
    testing: Option<&'static dyn CommandLineTesting>,
    backend: Option<Arc<dyn watchmanager::WatchBackend>>,
) -> Option<Box<Watcher>> {
    let writer: tsc::SyncWriter = Arc::new(move |t: &str| sys.write(t));
    let fs = sys.fs();
    let wm = WatchManager::new(Arc::clone(&writer), Box::new(move |d: &str| fs.directory_exists(d)));
    if testing.is_none() && sys.get_environment_variable("TS_WATCH_DEBUG").is_some_and(|v| !v.is_empty()) {
        wm.set_debug_log(Arc::clone(&writer));
    }
    if let Some(backend) = backend {
        WatchManager::set_backend(&mut wm.lock(), backend);
    }
    let options = config.compiler_options().unwrap();
    let mut w = Box::new(Watcher {
        sys,
        shared_program_data: None,
        free_replaced: None,
        config_file_name: config.config_file.map(|f| f.source_file.file_name().to_string()).unwrap_or_default(),
        config,
        compiler_options_from_command_line,
        command_line_raw,
        report_diagnostic,
        report_error_summary,
        report_watch_status: tsc::create_watch_status_reporter(sys, &options, testing.map(|t| t as &dyn CommandLineTesting)),
        testing,
        content_mapper_host: None,
        content_mapper_project: None,
        program: None,
        extended_config_cache: Arc::new(ExtendedConfigCache::default()),
        config_modified: false,
        config_has_errors: false,
        config_file_paths: Vec::new(),
        source_file_cache: Arc::default(),
        wm,
        seen_files: FxHashSet::default(),
        config_mtimes: FxHashMap::default(),
        watch_set_dirty: false,
        force_full_rebuild: false,
        program_ready: false,
        fast_path_builds: 0,
        full_builds: 0,
    });
    w.start();
    if testing.is_some() {
        return Some(w);
    }
    let wm = Arc::clone(&w.wm);
    wm.run_loop(|| w.do_cycle());
    None
}

impl Watcher {
    fn start(&mut self) {
        self.content_mapper_host = tsc::new_content_mapper_host(self.sys, &self.config.compiler_options().unwrap());
        self.replace_content_mapper_project(self.config);
        let wm = Arc::clone(&self.wm);
        let mut state = wm.lock();
        let host = self.new_compiler_host(self.sys.fs());
        self.program = tsrs_incremental::read_build_info_program(self.config, &*tsrs_incremental::new_build_info_reader(Arc::clone(&host)), &*host);

        if !self.config_file_name.is_empty() {
            self.config_file_paths = std::iter::once(self.config_file_name.clone()).chain(self.config.extended_source_files()).collect();
        }
        if self.testing.is_none() {
            wm.ensure_default_backend(&mut state);
        }
        (self.report_watch_status)(new_compiler_diagnostic(&diagnostics::Starting_compilation_in_watch_mode, &[]));
        self.watch_set_dirty = true;
        if self.do_build(&mut state).is_err() {
            wm.force_overflow();
        }
    }

    fn new_compiler_host(&self, fs: Arc<dyn FS>) -> Arc<dyn CompilerHost> {
        new_compiler_host(
            self.sys.get_current_directory(),
            fs,
            self.sys.default_library_path(),
            Some(Arc::clone(&self.extended_config_cache) as Arc<dyn tsoptions::ExtendedConfigCache + Send>),
            Some(crate::execute::get_trace_from_sys(self.sys, self.testing)),
            self.content_mapper_project.clone(),
        )
    }

    fn replace_content_mapper_project(&mut self, config: P<ParsedCommandLine>) {
        if self.content_mapper_host.is_none() {
            return;
        }
        let project = crate::execute::get_content_mapper_project(self.content_mapper_host.as_ref(), config);
        if let Some(old) = self.content_mapper_project.take() {
            let _ = old.close();
        }
        self.content_mapper_project = project;
    }

    fn content_mapper_watched_files(&self) -> Vec<String> {
        let mut files: Vec<String> = self
            .config
            .content_mappers()
            .iter()
            .filter(|m| !m.package_directory.is_empty() && m.contribution_id.is_empty())
            .map(|m| tspath::combine_paths(&m.package_directory, &["package.json"]))
            .collect();
        if let Some(project) = &self.content_mapper_project {
            match project.watched_files() {
                Ok(dynamic) => files.extend(dynamic),
                Err(err) => {
                    (self.report_diagnostic)(tsrs_compiler::content_mapper_project_diagnostic(&err));
                    return files;
                }
            }
        }
        files.sort();
        files.dedup();
        files
    }

    fn compare_paths_options(&self) -> ComparePathsOptions {
        ComparePathsOptions {
            use_case_sensitive_file_names: self.sys.fs().use_case_sensitive_file_names(),
            current_directory: self.sys.get_current_directory().to_string(),
        }
    }

    fn to_path(&self, file_name: &str) -> Path {
        tspath::to_path(file_name, self.sys.get_current_directory(), self.sys.fs().use_case_sensitive_file_names())
    }

    fn compiler_program(&self) -> &'static Program {
        self.program.expect("built").get_program()
    }

    fn compute_desired_watches(&self, seen_file_paths: &[String]) -> FxHashMap<String, bool> {
        let fs = self.sys.fs();
        let mut desired_dirs: FxHashMap<String, bool> = FxHashMap::default(); // dir → recursive

        // Wildcard directories from tsconfig (recursive or non-recursive)
        if self.config.config_file.is_some()
            && let Some(wildcards) = self.config.wildcard_directories()
        {
            for (dir, &recursive) in wildcards.iter() {
                desired_dirs.insert(fs.realpath(dir), recursive);
            }
        }
        // For no-config CLI mode, ensure CWD is watched
        if self.config.config_file.is_none() && desired_dirs.is_empty() {
            desired_dirs.insert(fs.realpath(self.sys.get_current_directory()), false);
        }
        // Config file parent directories as non-recursive watches
        for cfg_path in &self.config_file_paths {
            let dir = tspath::get_directory_path(&fs.realpath(cfg_path));
            desired_dirs.entry(dir).or_insert(false);
        }
        // For no-config CLI mode, also watch the CLI-specified files' directories
        if self.config.config_file.is_none() {
            for file_name in self.config.file_names() {
                let abs = tspath::get_normalized_absolute_path(file_name, self.sys.get_current_directory());
                desired_dirs.entry(tspath::get_directory_path(&fs.realpath(&abs))).or_insert(false);
            }
        }

        // Add parent directories for seen files not covered by existing dir watches. Resolve ancestor fallbacks first
        // so coverage checks use final dirs.
        let resolved_dirs = self.wm.resolve_desired_dirs(&desired_dirs);
        let mut coverage = DirWatchSet::new(&self.compare_paths_options());
        let mut resolved_sorted: Vec<(&String, &bool)> = resolved_dirs.iter().collect();
        resolved_sorted.sort();
        for (dir, &recursive) in resolved_sorted {
            coverage.set(dir, recursive);
        }
        let program_files = self.compiler_program().files_by_path();
        let root_files: FxHashSet<Path> = self.config.file_names().iter().map(|f| self.to_path(f)).collect();
        for file_path in seen_file_paths {
            let dir = tspath::get_directory_path(file_path);
            if coverage.covered(&dir) {
                continue;
            }
            // Seen files mix program files with lookup locations. Only lookups keep the depth check, so an imported
            // file outside the tsconfig directory (say /shared next to /app) is still watched. A root file is not in
            // the program while it is missing, but its directory stays watched so that recreating it rebuilds.
            let p = self.to_path(file_path);
            if program_files.contains_key(&p) || root_files.contains(&p) || watchmanager::can_watch_directory(&dir) {
                coverage.set(&dir, false);
            }
        }
        // Re-resolve in case newly added dirs don't exist
        self.wm.resolve_desired_dirs(&coverage.dirs())
    }

    fn reconcile_watches(&self, state: &mut WatchState, seen_file_paths: &[String]) -> Result<(), tsrs_fswatch::Error> {
        let desired_dirs = self.compute_desired_watches(seen_file_paths);
        self.wm.reconcile_watches(state, &desired_dirs)
    }

    /// Go `DoCycle`.
    pub fn do_cycle(&mut self) {
        let wm = Arc::clone(&self.wm);
        let mut state = wm.lock();

        let (changed_paths, overflow) = wm.drain_events();
        let has_events = !changed_paths.is_empty() || overflow;

        let manifest_changed = self.content_mapper_manifest_changed(&changed_paths);
        if self.recheck_ts_config(manifest_changed) {
            return;
        }

        if has_events && !overflow && !self.config_modified {
            // Filter fswatch events against known dependencies
            if !self.is_relevant_change(&state, &changed_paths) {
                if let Some(log) = wm.debug_log() {
                    log(&format!("[watch] DoCycle: {} event(s) not relevant to compilation, skipping rebuild\n", changed_paths.len()));
                }
                if let Some(testing) = self.testing {
                    testing.on_program(self.program.unwrap());
                }
                return;
            }
            self.evict_changed_source_files(&changed_paths);
            let fs = self.sys.fs();
            let program_files = self.compiler_program().files_by_path();
            let content_mapper_watched_files: FxHashSet<Path> = self.content_mapper_watched_files().iter().map(|f| self.to_path(f)).collect();
            let mut content_mapper_config_changed = false;
            for event_path in sorted_keys(&changed_paths) {
                if fs.directory_exists(event_path) {
                    // A watched directory changed: the wildcard file set may have changed, so reload file names on
                    // the next build.
                    self.watch_set_dirty = true;
                    continue;
                }
                let p = self.to_path(event_path);
                if content_mapper_watched_files.contains(&p) {
                    content_mapper_config_changed = true;
                    self.force_full_rebuild = true;
                }
                if self.config.config_file.is_some() && self.config.possibly_matches_file_name(event_path) && !self.seen_files.contains(&p) {
                    // A file that matches the project but was not previously seen appeared: a structural change that
                    // requires a full rebuild, not the single-file fast path.
                    self.watch_set_dirty = true;
                    self.force_full_rebuild = true;
                    continue;
                }
                match program_files.get(&p) {
                    // Canonical mapped files must be transformed again, and supplemental paths are failed physical
                    // lookups reserved for virtual files. Neither can use single-file AST reuse.
                    Some(source_file) if !source_file.content_mapper().is_empty() => self.force_full_rebuild = true,
                    Some(_) => {}
                    // A non-source build dependency changed (a package.json or a previously-missing module path):
                    // module resolution may now differ, so the single-file fast path is unsafe.
                    None if self.seen_files.contains(&p) => self.force_full_rebuild = true,
                    None => {}
                }
            }
            if content_mapper_config_changed
                && let Some(project) = &self.content_mapper_project
                && project.refresh().is_err()
            {
                (self.report_diagnostic)(new_compiler_diagnostic(&diagnostics::The_content_mapper_process_could_not_be_started_or_initialized, &[]));
                return;
            }
        } else if overflow {
            // Overflow: evict the entire source file cache and force a full rebuild. The fast path must not run here:
            // after clearing the cache a one-file program would present exactly one cache miss and be misread as a
            // single-file content edit, silently reusing a stale program instead of rediscovering the file graph.
            self.source_file_cache = Arc::default();
            self.watch_set_dirty = true;
            self.force_full_rebuild = true;
        } else if !has_events && !self.config_modified {
            if let Some(log) = wm.debug_log() {
                log("[watch] DoCycle: no events, skipping\n");
            }
            if let Some(testing) = self.testing {
                testing.on_program(self.program.unwrap());
            }
            return;
        }

        (self.report_watch_status)(new_compiler_diagnostic(&diagnostics::File_change_detected_Starting_incremental_compilation, &[]));
        if self.do_build(&mut state).is_err() {
            // Mid-cycle watch failure; force a full rebuild on the next event
            wm.force_overflow();
        }
    }

    fn is_relevant_change(&self, state: &WatchState, changed_paths: &FxHashMap<String, EventKind>) -> bool {
        let fs = self.sys.fs();
        let opts = self.compare_paths_options();
        let content_mapper_watched_files: FxHashSet<Path> = self.content_mapper_watched_files().iter().map(|f| self.to_path(f)).collect();
        sorted_keys(changed_paths).into_iter().any(|event_path| {
            let p = self.to_path(event_path);
            content_mapper_watched_files.contains(&p)
                || self.seen_files.contains(&p)
                || (self.config.config_file.is_some() && self.config.possibly_matches_file_name(event_path))
                || (self.config.config_file.is_some() && self.config.possibly_matches_directory_name(&p))
                || (fs.directory_exists(event_path) && WatchManager::is_path_under_watch(state, event_path, &opts))
        })
    }

    fn do_build(&mut self, state: &mut WatchState) -> Result<(), tsrs_fswatch::Error> {
        if self.config_modified {
            self.source_file_cache = Arc::default();
            self.watch_set_dirty = true;
        }

        let mut reloaded_file_names = false;
        if self.watch_set_dirty {
            if self.config.config_file.is_some() && self.config.wildcard_directories().is_some_and(|w| !w.is_empty()) {
                let new_config = P::new(self.config.reload_file_names_of_parsed_command_line(&*self.sys.fs()));
                reloaded_file_names = true;
                if self.config.file_names() == new_config.file_names() {
                    self.watch_set_dirty = false;
                }
                self.config = new_config;
            } else if !self.config_modified {
                self.watch_set_dirty = false;
            }
        }

        if self.program.is_some() && self.program_ready && !self.config_modified && !self.watch_set_dirty && !self.force_full_rebuild {
            let cached = Arc::new(cachedvfs::from(self.sys.fs()));
            let inner_host = self.new_compiler_host(Arc::clone(&cached) as Arc<dyn FS>);
            let host: Arc<dyn CompilerHost> = Arc::new(watchCompilerHost { inner: inner_host, cache: Arc::clone(&self.source_file_cache) });
            if self.try_update_program(host) {
                self.fast_path_builds += 1;
                let result = self.compile_and_emit();
                cached.disable_and_clear_cache();
                self.record_config_mtimes();
                self.config_modified = false;
                self.report_error_count(result.diagnostics.len());
                self.free_replaced_program();
                if let Some(testing) = self.testing {
                    testing.on_program(self.program.unwrap());
                }
                return Ok(());
            }
            cached.disable_and_clear_cache();
        }

        let cached = Arc::new(cachedvfs::from(self.sys.fs()));
        let tfs = Arc::new(trackingvfs::FS::new(Arc::clone(&cached)));
        let inner_host = self.new_compiler_host(Arc::clone(&tfs) as Arc<dyn FS>);
        let host: Arc<dyn CompilerHost> = Arc::new(watchCompilerHost { inner: inner_host, cache: Arc::clone(&self.source_file_cache) });

        if self.config.config_file.is_some() {
            if let Some(wildcards) = self.config.wildcard_directories() {
                for dir in wildcards.keys() {
                    tfs.add_seen_file(dir);
                }
            }
            if !reloaded_file_names && !self.watch_set_dirty && self.config.wildcard_directories().is_some_and(|w| !w.is_empty()) {
                self.config = P::new(self.config.reload_file_names_of_parsed_command_line(&*self.sys.fs()));
            }
        }
        for path in &self.config_file_paths {
            tfs.add_seen_file(path);
        }
        for path in self.content_mapper_watched_files() {
            tfs.add_seen_file(&path);
        }

        let mut program_options = ProgramOptions::new(self.config, Arc::clone(&host));
        // Each cycle replaces the program; its checkers' regions are freed with it (`install_program`).
        program_options.checker_regions = true;
        let program = new_program(program_options);
        self.install_program(program, host, true);
        self.program_ready = true;
        self.full_builds += 1;

        let result = self.compile_and_emit();
        cached.disable_and_clear_cache();

        let seen = tfs.seen_files();
        self.seen_files = seen.iter().map(|p| self.to_path(p)).collect();
        self.record_config_mtimes();

        if let Err(err) = self.reconcile_watches(state, &seen) {
            self.sys.write(&format!("{}\n", err.message()));
            return Err(err);
        }
        self.watch_set_dirty = false;
        self.config_modified = false;
        self.force_full_rebuild = false;

        let program_files = self.compiler_program().files_by_path();
        for path in self.source_file_cache.keys() {
            if !program_files.contains_key(&path) {
                self.source_file_cache.delete(&path);
            }
        }

        self.report_error_count(result.diagnostics.len());
        self.free_replaced_program();
        if let Some(testing) = self.testing {
            testing.on_program(self.program.unwrap());
        }
        Ok(())
    }

    fn record_config_mtimes(&mut self) {
        let fs = self.sys.fs();
        self.config_mtimes =
            self.config_file_paths.iter().filter_map(|cfg_path| fs.stat(cfg_path).map(|s| (cfg_path.clone(), s.mod_time()))).collect();
    }

    fn report_error_count(&self, error_count: usize) {
        let diagnostic = if error_count == 1 {
            new_compiler_diagnostic(&diagnostics::Found_1_error_Watching_for_file_changes, &[])
        } else {
            new_compiler_diagnostic(&diagnostics::Found_0_errors_Watching_for_file_changes, &[&error_count])
        };
        (self.report_watch_status)(diagnostic);
    }

    fn try_update_program(&mut self, host: Arc<dyn CompilerHost>) -> bool {
        let old_program = self.compiler_program();
        let mut changed_path: Option<&Path> = None;
        let mut changed_count = 0;
        for file in old_program.get_source_files() {
            if !file.content_mapper().is_empty() {
                continue;
            }
            if self.source_file_cache.load(file.path()).is_none() {
                changed_path = Some(file.path());
                changed_count += 1;
                if changed_count > 1 {
                    return false;
                }
            }
        }
        let Some(changed_path) = changed_path else { return false };

        if let Some(&old_file) = old_program.files_by_path().get(changed_path)
            && let Some(new_file) = host.get_source_file(old_file.parse_options().clone())
            && !equal_jsx_implicit_import(&old_program.options(), old_file, new_file)
        {
            return false;
        }

        let (new_program, _, reused) = old_program.reuse_program(changed_path, Arc::clone(&host), None, None);
        if reused {
            self.install_program(new_program.unwrap(), host, false);
        }
        reused
    }

    // tsrs-only (Go's GC): makes `program` the current one and frees the compiler program it replaces, with its
    // checkers, once the new incremental program has taken what it keeps of the old snapshot (diagnostics, which the
    // checkers allocate outside their regions, and plain data). A full build also frees the data the programs since
    // the previous full build shared (`update_program` hands it on).
    fn install_program(&mut self, program: &'static Program, host: Arc<dyn CompilerHost>, full_build: bool) {
        // Bind before any checker exists: single-threaded, the first checker would bind the files in its region, and
        // later programs reuse the cached files.
        program.bind_source_files();
        let old = self.program.replace(tsrs_incremental::new_program(
            program,
            self.program,
            tsrs_incremental::create_host(host),
            Some(std::time::Instant::now),
            self.testing.is_some(),
        ));
        let old = old.and_then(|old| old.compiler_program());
        let shared = if full_build { self.shared_program_data.replace(tsrs_compiler::shared_program_data(program)) } else { None };
        let free = move || {
            if let Some(old) = old {
                // SAFETY: nothing refers to the old compiler program any more: the watcher's only reference was
                // `self.program`, its checkers are not held (the cycle that used them is over), and the new
                // incremental program read the old snapshot without keeping the program.
                unsafe { tsrs_compiler::free_program(old) };
            }
            if let Some(shared) = shared {
                // SAFETY: every program since the previous full build shared this data, and each was freed when the
                // next one replaced it (above, for the last one); the new full build has its own.
                unsafe { shared.free() };
            }
        };
        // The test harness frees in place, for deterministic runs.
        if self.testing.is_some() {
            free();
        } else {
            // A cycle that ended early (a watch error) left its free pending.
            self.free_replaced_program();
            self.free_replaced = Some(Box::new(free));
        }
    }

    // Freeing a large program's checker regions takes a while (~150 ms on a 9k-file project) and nothing waits for it,
    // so it runs on a thread of its own, started once the cycle has reported instead of beside the rest of the cycle.
    fn free_replaced_program(&mut self) {
        if let Some(free) = self.free_replaced.take() {
            std::thread::Builder::new().name("tsrs-free-program".to_string()).spawn(free).expect("failed to spawn a thread");
        }
    }

    fn evict_changed_source_files(&self, changed_paths: &FxHashMap<String, EventKind>) {
        for event_path in sorted_keys(changed_paths) {
            let p = self.to_path(event_path);
            if self.source_file_cache.load(&p).is_some() {
                if let Some(log) = self.wm.debug_log() {
                    log(&format!("[watch] evicting cached source file: {p}\n"));
                }
                self.source_file_cache.delete(&p);
            }
        }
    }

    fn compile_and_emit(&self) -> tsc::CompileAndEmitResult {
        let program = self.program.unwrap();
        tsc::emit_files_and_report_errors(&EmitInput {
            sys: self.sys,
            program: program.get_program(),
            config: self.config,
            report_diagnostic: &self.report_diagnostic,
            report_error_summary: &self.report_error_summary,
            compile_times: tsc::CompileTimes::default(),
            incremental: Some(program),
            writer: None,
            write_file: None,
            testing: self.testing,
            testing_m_times_cache: None,
        })
    }

    fn content_mapper_manifest_changed(&self, changed_paths: &FxHashMap<String, EventKind>) -> bool {
        self.config
            .content_mappers()
            .iter()
            .filter(|m| !m.package_directory.is_empty() && m.contribution_id.is_empty())
            .any(|m| changed_paths.contains_key(&tspath::combine_paths(&m.package_directory, &["package.json"])))
    }

    // Returns true when the config could not be parsed (its errors were reported and the cycle ends).
    fn recheck_ts_config(&mut self, force: bool) -> bool {
        if self.config_file_name.is_empty() {
            return false;
        }
        if !force && !self.config_has_errors && !self.config_file_paths.is_empty() {
            let fs = self.sys.fs();
            let changed = self.config_file_paths.iter().any(|path| {
                let stat = fs.stat(path);
                match self.config_mtimes.get(path) {
                    None => stat.is_some(),
                    Some(old_mtime) => stat.is_none_or(|s| s.mod_time() != *old_mtime),
                }
            });
            if !changed {
                return false;
            }
        }

        let Some(config_parse_result) = self.parse_config_file() else { return true };
        if self.config_has_errors {
            self.config_modified = true;
        }
        self.config_has_errors = false;
        self.config_file_paths = std::iter::once(self.config_file_name.clone()).chain(config_parse_result.extended_source_files()).collect();
        if !parsed_configs_equal(&self.config, &config_parse_result) {
            self.config_modified = true;
        }
        self.replace_content_mapper_project(config_parse_result);
        self.config = config_parse_result;
        false
    }

    fn parse_config_file(&mut self) -> Option<P<ParsedCommandLine>> {
        let extended_config_cache = Arc::new(ExtendedConfigCache::default());
        let host = crate::execute::parse_config_host(self.sys);
        let (config_parse_result, errors) = tsrs_compiler::worker_pool().install(|| {
            tsoptions::get_parsed_command_line_of_config_file(
                &self.config_file_name,
                Some(&self.compiler_options_from_command_line),
                self.command_line_raw.as_ref(),
                host,
                Some(&*extended_config_cache),
            )
        });
        if !errors.is_empty() {
            let error_count = errors.len();
            for e in errors {
                (self.report_diagnostic)(e);
            }
            self.config_has_errors = true;
            self.report_error_count(error_count);
            return None;
        }
        self.extended_config_cache = extended_config_cache;
        config_parse_result.map(P::new)
    }
}

// Go ranges over the event map; the order only matters for debug logs and the first matching reason.
fn sorted_keys(changed_paths: &FxHashMap<String, EventKind>) -> Vec<&String> {
    let mut keys: Vec<&String> = changed_paths.keys().collect();
    keys.sort();
    keys
}

// Go `reflect.DeepEqual(w.config.ParsedConfig, configParseResult.ParsedConfig)`: the compiler options by value
// (`P`'s equality is identity).
fn parsed_configs_equal(a: &ParsedCommandLine, b: &ParsedCommandLine) -> bool {
    let (a, b) = (&a.parsed_config, &b.parsed_config);
    a.compiler_options.as_ref().map(|o| &**o) == b.compiler_options.as_ref().map(|o| &**o)
        && a.watch_options == b.watch_options
        && a.type_acquisition == b.type_acquisition
        && a.file_names == b.file_names
        && a.project_references == b.project_references
        && a.project_references_is_nil == b.project_references_is_nil
        && a.content_mappers == b.content_mappers
}

fn equal_jsx_implicit_import(options: &CompilerOptions, old_file: P<SourceFile>, new_file: P<SourceFile>) -> bool {
    let is_jsx = |file: P<SourceFile>| matches!(file.script_kind(), ScriptKind::JSX | ScriptKind::TSX);
    if !is_jsx(old_file) && !is_jsx(new_file) {
        return true;
    }
    let old_import = tsrs_ast::get_jsx_runtime_import(&tsrs_ast::get_jsx_implicit_import_base(options, Some(old_file)), options);
    let new_import = tsrs_ast::get_jsx_runtime_import(&tsrs_ast::get_jsx_implicit_import_base(options, Some(new_file)), options);
    old_import == new_import
}
