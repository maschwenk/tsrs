// Port of execute/build/buildtask.go (the non-watch parts).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use tsrs_ast::{new_compiler_diagnostic, Diagnostic};
use tsrs_compiler::{new_program, ProgramOptions, WriteFileData};
use tsrs_core::arena::Region;
use tsrs_core::collections::{new_set_from_items, Set};
use tsrs_core::tspath::{self, Path};
use tsrs_core::P;
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_incremental::{is_build_info_file_name_default_library, BuildInfo, BuildInfoRootInfoReader};
use tsrs_tsoptions::ParsedCommandLine;
use tsrs_vfs::FS;

use super::compilerhost::compilerHost;
use super::host::incrementalHost;
use super::orchestrator::{Orchestrator, OrchestratorResult};
use super::uptodatestatus::*;
use crate::tsc::{self, emit_and_report_statistics, CompileTimes, DiagnosticReporter, EmitInput, ExitStatus, Statistics};

// tsrs-only (watch mode): the most changed paths a cycle can have and still reparse one file (`single_changed_file`).
const MAX_ONE_FILE_CHANGES: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum buildKind {
    #[default]
    None,
    Pseudo,
    Program,
}

pub(crate) struct upstreamTask {
    pub(crate) task: P<BuildTask>,
    pub(crate) ref_index: usize,
}

pub(crate) struct buildInfoEntry {
    build_info: Option<Arc<BuildInfo>>,
    path: Path,
    m_time: Option<SystemTime>,
    dts_time: Option<Option<SystemTime>>,
}

pub(crate) struct taskResult {
    pub(crate) builder: Arc<Mutex<String>>,
    pub(crate) report_status: Option<DiagnosticReporter<'static>>,
    pub(crate) diagnostic_reporter: Option<DiagnosticReporter<'static>>,
    pub(crate) exit_status: ExitStatus,
    pub(crate) statistics: Option<Statistics>,
    pub(crate) program: Option<P<tsrs_incremental::Program>>,
    pub(crate) build_kind: buildKind,
    pub(crate) files_to_delete: Vec<String>,
}

impl taskResult {
    pub(crate) fn new(builder: Arc<Mutex<String>>) -> taskResult {
        taskResult {
            builder,
            report_status: None,
            diagnostic_reporter: None,
            exit_status: ExitStatus::Success,
            statistics: None,
            program: None,
            build_kind: buildKind::None,
            files_to_delete: Vec::new(),
        }
    }
}

// A Go `chan struct{}` that is only ever closed (orchestrator.go `task.done`, `task.built`): `wait` blocks until
// `close`. `abort` also releases waiters (a builder panicked; Go would take the process down instead of hanging).
#[derive(Default)]
pub(crate) struct closeSignal {
    closed: Mutex<bool>,
    cond: std::sync::Condvar,
}

impl closeSignal {
    pub(crate) fn reset(&self) {
        *self.closed.lock().unwrap() = false;
    }

    pub(crate) fn close(&self) {
        *self.closed.lock().unwrap() = true;
        self.cond.notify_all();
    }

    // Returns false if released by an abort rather than a close.
    pub(crate) fn wait(&self, aborted: &AtomicBool) -> bool {
        let mut closed = self.closed.lock().unwrap();
        while !*closed && !aborted.load(Ordering::SeqCst) {
            closed = self.cond.wait(closed).unwrap();
        }
        *closed
    }

    pub(crate) fn wake(&self) {
        let _guard = self.closed.lock().unwrap();
        self.cond.notify_all();
    }
}

pub struct BuildTask {
    pub(crate) config: String,
    pub(crate) resolved: Mutex<Option<P<ParsedCommandLine>>>,
    pub(crate) up_stream: Mutex<Vec<upstreamTask>>,
    pub(crate) down_stream: Mutex<Vec<P<BuildTask>>>, // Only set and used in watch mode
    pub(crate) status: Mutex<Option<upToDateStatus>>,
    pub(crate) done: closeSignal,

    // task reporting
    pub(crate) result: Mutex<Option<taskResult>>,
    pub(crate) built: closeSignal, // closed when result is ready to be reported

    build_info_entry: Mutex<Option<buildInfoEntry>>,
    package_jsons: Mutex<Vec<String>>,

    errors: Mutex<Vec<P<Diagnostic>>>,
    pub(crate) pending: AtomicBool,
    is_initial_cycle: AtomicBool,
    // Go `dirty`: the config changed since the task was created (API rebuilds and watch mode).
    pub(crate) dirty: AtomicBool,
    down_stream_update_mu: Mutex<()>,

    // Go's contentMapperProjectOnce and the contentMapperProject it sets.
    content_mapper_project: OnceLock<Option<Arc<dyn tsrs_contentmapper::Project>>>,
    content_mapper_project_err: Mutex<Option<tsrs_contentmapper::Error>>,

    // tsrs-only, watch mode: the program of the task's last build, freed when the task builds again (Go's GC).
    watch_program: Mutex<Option<&'static tsrs_compiler::Program>>,
    // tsrs-only, watch mode: what the task's last full build allocated beside its program, which the programs
    // `reuse_program` made since share (`replace_watch_program`).
    watch_full_build: Mutex<Option<watchFullBuild>>,
    // tsrs-only, watch mode: the incremental program of the task's last build, the old state of its next build
    // (`old_watch_state`), and the region of the diagnostics its snapshot keeps (None under the test harness).
    watch_incremental: Mutex<Option<(P<tsrs_incremental::Program>, Option<Region>)>>,
    // tsrs-only, watch mode: the changed paths that made the task's inputs out of date this cycle, or None when
    // something else did (`single_changed_file`).
    watch_changes: Mutex<Option<Vec<(Path, tsrs_fswatch::EventKind)>>>,
    // tsrs-only, watch mode: the task built a program this cycle (reset by the orchestrator each cycle).
    pub(crate) built_program: AtomicBool,
}

// tsrs-only (watch mode): what a task's full build allocated beside its program, which the programs `reuse_program`
// made from it share (`BuildTask::replace_watch_program`): its processed-file data, the regions of what its file loader
// allocated (resolutions, include reasons, the processed-file data), and its files, which the processed-file data refers
// to (`BuildTask::add_live_watch_files`).
struct watchFullBuild {
    shared: tsrs_compiler::SharedProgramData,
    regions: Arc<tsrs_compiler::ProgramRegions>,
    files: Vec<P<tsrs_ast::SourceFile>>,
}

fn diag(message: &'static Message, args: &[&dyn std::fmt::Display]) -> P<Diagnostic> {
    new_compiler_diagnostic(message, args)
}

impl BuildTask {
    pub(crate) fn new(config: String, is_initial_cycle: bool) -> BuildTask {
        BuildTask {
            config,
            resolved: Mutex::new(None),
            up_stream: Mutex::new(Vec::new()),
            down_stream: Mutex::new(Vec::new()),
            status: Mutex::new(None),
            done: closeSignal::default(),
            result: Mutex::new(None),
            built: closeSignal::default(),
            build_info_entry: Mutex::new(None),
            package_jsons: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            pending: AtomicBool::new(false),
            is_initial_cycle: AtomicBool::new(is_initial_cycle),
            dirty: AtomicBool::new(false),
            down_stream_update_mu: Mutex::new(()),
            content_mapper_project: OnceLock::new(),
            content_mapper_project_err: Mutex::new(None),
            watch_program: Mutex::new(None),
            watch_full_build: Mutex::new(None),
            watch_incremental: Mutex::new(None),
            watch_changes: Mutex::new(None),
            built_program: AtomicBool::new(false),
        }
    }

    // buildtask.go:82
    fn get_content_mapper_project(
        &self,
        orchestrator: &Orchestrator,
    ) -> (Option<Arc<dyn tsrs_contentmapper::Project>>, Option<tsrs_contentmapper::Error>) {
        let project = self
            .content_mapper_project
            .get_or_init(|| {
                let content_mapper_host = orchestrator.content_mapper_host.lock().unwrap().clone()?;
                let resolved: &'static ParsedCommandLine = self.resolved_opt()?.get();
                if resolved.content_mappers().is_empty() {
                    return None;
                }
                content_mapper_host.project(tsrs_contentmapper::ProjectSpec {
                    config_file_name: resolved.config_name().to_string(),
                    mappers: resolved.content_mappers().iter().collect(),
                    compiler_options: resolved.compiler_options(),
                })
            })
            .clone();
        (project, self.content_mapper_project_err.lock().unwrap().clone())
    }

    // The task's content mapper project, if it was created.
    pub(crate) fn content_mapper_project_opt(&self) -> Option<Arc<dyn tsrs_contentmapper::Project>> {
        self.content_mapper_project.get().cloned().flatten()
    }

    pub(crate) fn set_content_mapper_project_err(&self, err: tsrs_contentmapper::Error) {
        *self.content_mapper_project_err.lock().unwrap() = Some(err);
    }

    // The build info file name and its parsed contents, if the task read or wrote one.
    pub(crate) fn build_info_snapshot(&self) -> Option<(Path, Arc<BuildInfo>)> {
        let entry = self.build_info_entry.lock().unwrap();
        let entry = entry.as_ref()?;
        Some((entry.path.clone(), Arc::clone(entry.build_info.as_ref()?)))
    }

    pub(crate) fn package_jsons(&self) -> Vec<String> {
        self.package_jsons.lock().unwrap().clone()
    }

    // buildtask.go:96
    pub(crate) fn refresh_content_mapper_project(&self, _orchestrator: &Orchestrator) {
        if let Some(Some(project)) = self.content_mapper_project.get() {
            *self.content_mapper_project_err.lock().unwrap() = project.refresh().err();
        }
    }

    // Go `existing.contentMapperProject.Close()` (orchestrator.go:186, 280), when the task's project was created.
    pub(crate) fn close_content_mapper_project(&self) {
        if let Some(Some(project)) = self.content_mapper_project.get() {
            let _ = project.close();
        }
    }

    // buildtask.go:831
    pub(crate) fn reset_status(&self) {
        *self.status.lock().unwrap() = None;
        self.pending.store(true, Ordering::SeqCst);
        self.errors.lock().unwrap().clear();
    }

    // buildtask.go:837
    pub(crate) fn reset_config(&self, orchestrator: &Orchestrator, path: &Path) {
        self.dirty.store(true, Ordering::SeqCst);
        orchestrator.host().resolved_references.delete(path);
    }

    // orchestrator.go createBuildTasks / clean: a dirty task's build info entry carries over to its replacement.
    pub(crate) fn take_build_info_entry(&self) -> Option<buildInfoEntry> {
        self.build_info_entry.lock().unwrap().take()
    }

    pub(crate) fn set_build_info_entry(&self, entry: Option<buildInfoEntry>) {
        *self.build_info_entry.lock().unwrap() = entry;
    }

    pub(crate) fn resolved_opt(&self) -> Option<P<ParsedCommandLine>> {
        *self.resolved.lock().unwrap()
    }

    fn resolved(&self) -> P<ParsedCommandLine> {
        self.resolved_opt().unwrap()
    }

    fn status(&self) -> upToDateStatus {
        self.status.lock().unwrap().clone().unwrap()
    }

    fn set_status(&self, status: upToDateStatus) {
        *self.status.lock().unwrap() = Some(status);
    }

    fn report_status(&self, d: P<Diagnostic>) {
        let result = self.result.lock().unwrap();
        (result.as_ref().unwrap().report_status.as_ref().unwrap())(d);
    }

    fn set_exit_status(&self, status: ExitStatus) {
        self.result.lock().unwrap().as_mut().unwrap().exit_status = status;
    }

    // buildtask.go:109
    fn wait_on_upstream(&self, orchestrator: &Orchestrator) {
        let upstream: Vec<P<BuildTask>> = self.up_stream.lock().unwrap().iter().map(|u| u.task).collect();
        for task in upstream {
            assert!(task.done.wait(&orchestrator.aborted), "build aborted: a builder thread panicked");
        }
    }

    // buildtask.go:115
    fn unblock_downstream(&self) {
        self.pending.store(false, Ordering::SeqCst);
        self.is_initial_cycle.store(false, Ordering::SeqCst);
        self.done.close();
    }

    // buildtask.go:121
    fn report_diagnostic(&self, err: P<Diagnostic>) {
        self.errors.lock().unwrap().push(err);
        let result = self.result.lock().unwrap();
        (result.as_ref().unwrap().diagnostic_reporter.as_ref().unwrap())(err);
    }

    // buildtask.go:126
    pub(crate) fn report(&self, orchestrator: &Orchestrator, _config_path: &Path, build_result: &mut OrchestratorResult) {
        let errors = self.errors.lock().unwrap();
        if !errors.is_empty() {
            build_result.errors.get_or_insert_with(Vec::new).extend(errors.iter().copied());
        }
        let result = self.result.lock().unwrap().take().unwrap();
        orchestrator.opts.sys.write(&result.builder.lock().unwrap());
        if result.exit_status as i32 > build_result.status() as i32 {
            build_result.status = Some(result.exit_status);
        }
        if let Some(statistics) = &result.statistics {
            build_result.statistics.aggregate(statistics);
        }
        // If we built the program, or updated timestamps, or had errors, we need to
        // delete files that are no longer needed
        match result.build_kind {
            buildKind::Program => {
                if let (Some(testing), Some(program)) = (orchestrator.opts.testing, result.program) {
                    testing.on_program(program);
                }
                build_result.statistics.projects_built += 1;
            }
            buildKind::Pseudo => build_result.statistics.timestamp_updates += 1,
            buildKind::None => {}
        }
        if !result.files_to_delete.is_empty() {
            build_result.files_to_delete.get_or_insert_with(Vec::new).extend(result.files_to_delete);
        }
    }

    // buildtask.go:154
    pub(crate) fn build_project(&'static self, orchestrator: &'static Orchestrator, path: &Path) {
        // Wait on upstream tasks to complete
        self.wait_on_upstream(orchestrator);
        // API builds: the task allocates in its own collectable region, entered after the upstream wait and left
        // before downstream tasks are released.
        let region = orchestrator.enter_api_region();
        if self.pending.load(Ordering::SeqCst) {
            self.set_status(self.get_up_to_date_status(orchestrator, path));
            self.report_up_to_date_status(orchestrator);
            if !self.handle_status_that_doesnt_require_build(orchestrator) {
                self.compile_and_emit(orchestrator, path);
                self.update_downstream(orchestrator, path);
            } else {
                if let Some(resolved) = self.resolved_opt() {
                    for diagnostic in resolved.get_config_file_parsing_diagnostics() {
                        self.report_diagnostic(diagnostic);
                    }
                }
                if !self.errors.lock().unwrap().is_empty() {
                    self.set_exit_status(ExitStatus::DiagnosticsPresent_OutputsSkipped);
                }
            }
        } else if !self.errors.lock().unwrap().is_empty() {
            self.report_up_to_date_status(orchestrator);
            let errors = self.errors.lock().unwrap().clone();
            let result = self.result.lock().unwrap();
            for err in errors {
                // Should not add the diagnostics so just reporting
                (result.as_ref().unwrap().diagnostic_reporter.as_ref().unwrap())(err);
            }
        }
        drop(region);
        self.unblock_downstream();
    }

    // buildtask.go:184
    fn update_downstream(&self, orchestrator: &Orchestrator, path: &Path) {
        if self.is_initial_cycle.load(Ordering::SeqCst) {
            return;
        }
        if orchestrator.opts.command.build_options.stop_build_on_errors.is_true() && self.status().is_error() {
            return;
        }
        let program = self.result.lock().unwrap().as_ref().and_then(|r| r.program);
        let down_stream = self.down_stream.lock().unwrap().clone();
        let Some(program) = program else {
            for down_stream in down_stream {
                let _guard = down_stream.down_stream_update_mu.lock().unwrap();
                down_stream.reset_status();
                down_stream.pending.store(true, Ordering::SeqCst);
            }
            return;
        };
        for down_stream in down_stream {
            let _guard = down_stream.down_stream_update_mu.lock().unwrap();
            let mut reset = false;
            {
                let mut status = down_stream.status.lock().unwrap();
                if let Some(current) = status.as_ref() {
                    let mut kind = current.kind;
                    if kind == upToDateStatusType::UpToDate && !program.has_changed_dts_file() {
                        *status = Some(upToDateStatus::with(upToDateStatusType::UpToDateWithUpstreamTypes, current.data.clone()));
                    } else {
                        if kind == upToDateStatusType::UpToDate {
                            kind = upToDateStatusType::UpToDateWithUpstreamTypes; // Go: fallthrough
                        }
                        match kind {
                            upToDateStatusType::UpToDateWithUpstreamTypes | upToDateStatusType::UpToDateWithInputFileText => {
                                if program.has_changed_dts_file() {
                                    let output = current.oldest_output_file_name();
                                    *status = Some(upToDateStatus::with(
                                        upToDateStatusType::InputFileNewer,
                                        statusData::InputOutputName(inputOutputName { input: self.config.clone(), output }),
                                    ));
                                }
                            }
                            upToDateStatusType::UpstreamErrors => {
                                let ref_config = tsrs_core::resolve_config_file_name_of_project_reference(&current.upstream_errors().ref_);
                                if orchestrator.to_path(&ref_config) == *path {
                                    reset = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            if reset {
                down_stream.reset_status();
            }
            down_stream.pending.store(true, Ordering::SeqCst);
        }
    }

    // tsrs-only: the program of the task's last build in watch mode.
    pub(crate) fn watch_program(&self) -> Option<&'static tsrs_compiler::Program> {
        *self.watch_program.lock().unwrap()
    }

    // The incremental program the task's next build starts from (`Orchestrator::start_signature_job`).
    pub(crate) fn watch_incremental_program(&self) -> Option<P<tsrs_incremental::Program>> {
        self.watch_incremental.lock().unwrap().as_ref().map(|&(program, _)| program)
    }

    // tsrs-only (watch mode): makes `program` and `incremental` the task's program and old state, and frees the
    // program it built last, the diagnostics the old state kept (`old_snapshot_diagnostics`) and, after a full build
    // (`full_build_regions`), what the last full build allocated beside its program. Nothing else holds them: the cycle
    // that built the last program has reported, and `incremental` copied the diagnostics it keeps into
    // `snapshot_diagnostics`. A program
    // `reuse_program` made shares the processed-file data and the file loader's regions of the full build it comes
    // from; they go when a full build replaces them, after the programs that shared them (the frees run in order).
    fn replace_watch_program(
        &self,
        orchestrator: &Orchestrator,
        program: &'static tsrs_compiler::Program,
        incremental: P<tsrs_incremental::Program>,
        snapshot_diagnostics: Option<Region>,
        old_snapshot_diagnostics: Option<Region>,
        full_build_regions: Option<Arc<tsrs_compiler::ProgramRegions>>,
    ) {
        *self.watch_incremental.lock().unwrap() = Some((incremental, snapshot_diagnostics));
        let old_full_build = full_build_regions.and_then(|regions| {
            let full_build = watchFullBuild { shared: tsrs_compiler::shared_program_data(program), regions, files: program.get_source_files().to_vec() };
            self.watch_full_build.lock().unwrap().replace(full_build)
        });
        let previous = self.watch_program.lock().unwrap().replace(program);
        orchestrator.freer.run(move || {
            if let Some(previous) = previous {
                // SAFETY: see above.
                unsafe { tsrs_compiler::free_program(previous) };
            }
            drop(old_snapshot_diagnostics);
            if let Some(old_full_build) = old_full_build {
                // SAFETY: every program since that full build shared this data, and each was freed when the next one
                // replaced it (above, for the last one, after the earlier ones); the new full build has its own.
                unsafe { old_full_build.shared.free() };
                // Last: the programs and the shared data above point into them.
                drop(old_full_build.regions);
            }
        });
    }

    // tsrs-only (watch mode): adds the files the task's program, its last full build's processed-file data and the
    // diagnostics its snapshot keeps refer to (`Orchestrator::free_unreachable_watch_files`).
    pub(crate) fn add_live_watch_files(&self, live: &mut rustc_hash::FxHashSet<usize>) {
        if let Some(program) = *self.watch_program.lock().unwrap() {
            live.extend(program.get_source_files().iter().map(|f| f.addr()));
        }
        if let Some(full_build) = &*self.watch_full_build.lock().unwrap() {
            live.extend(full_build.files.iter().map(|f| f.addr()));
        }
        let kept = self.watch_incremental.lock().unwrap().as_ref().map(|&(program, _)| program);
        let program = kept.or_else(|| self.result.lock().unwrap().as_ref().and_then(|r| r.program));
        if let Some(program) = program {
            program.for_each_diagnostic_file(&mut |f| {
                live.insert(f.addr());
            });
        }
    }

    // tsrs-only (watch mode): the program `kept` was made from, when `kept`, the incremental program of the task's last
    // build, is the old state of this build in place of the one read back from the build info that build wrote. Go
    // reads the build info again each cycle; the snapshot it gives is the one the last build left, as rebuilding it
    // took 70 ms of a 380 ms one-file cycle on a 9.4k-file project. A task with content mappers still reads it: the
    // build info read checks that their identities did not change.
    fn old_watch_state(&self, orchestrator: &Orchestrator, kept: P<tsrs_incremental::Program>) -> Option<&'static tsrs_compiler::Program> {
        if orchestrator.opts.command.build_options.force.is_true() || !self.resolved().content_mappers().is_empty() {
            return None;
        }
        let previous = self.watch_program()?;
        kept.compiler_program().is_some_and(|p| std::ptr::eq(p, previous)).then_some(previous)
    }

    // tsrs-only (watch mode): `previous` with the one file that changed reparsed (`reuse_program`), when the change
    // allows it: building the program again resolved and bound every file (130 ms of the 380 ms cycle above).
    fn reuse_watch_program(
        &self,
        orchestrator: &Orchestrator,
        previous: &'static tsrs_compiler::Program,
        compiler_host: &Arc<dyn tsrs_compiler::CompilerHost>,
    ) -> Option<&'static tsrs_compiler::Program> {
        let changed = self.single_changed_file(orchestrator, previous, compiler_host.fs())?;
        match previous.reuse_program(&changed, Arc::clone(compiler_host), None, None) {
            (Some(program), _, true) => Some(program),
            _ => None,
        }
    }

    // The one file of `previous` whose change made this task out of date, if nothing else could change the program:
    // no upstream project built a program this cycle (its declaration outputs are files of this one), and no other
    // changed path could change module resolution (a file module resolution could find, a package.json, or a
    // directory). Paths under the task's output directories are its own outputs, and a directory that contains another
    // changed path is reported for that change (the watch backends report the ancestors of a changed file).
    //
    // A directory that was moved away is not seen: its path no longer exists, and an editor's temporary files without
    // an extension (vim's `4913`) look the same. A module it resolved to stays resolved until the next full build.
    fn single_changed_file(&self, orchestrator: &Orchestrator, previous: &'static tsrs_compiler::Program, fs: &dyn FS) -> Option<Path> {
        let changes = self.watch_changes.lock().unwrap().take()?;
        // A one-file edit changes the file and its ancestors; more is a checkout or a generator, which a full build
        // handles, and which would make the ancestor check below quadratic.
        if changes.len() > MAX_ONE_FILE_CHANGES {
            return None;
        }
        if self.up_stream.lock().unwrap().iter().any(|upstream| upstream.task.built_program.load(Ordering::SeqCst)) {
            return None;
        }
        let options = previous.options();
        let compare = &orchestrator.compare_paths_options;
        let output_dirs: Vec<Path> = [&options.out_dir, &options.declaration_dir]
            .into_iter()
            .filter(|dir| !dir.is_empty())
            .map(|dir| tspath::to_path(dir, &compare.current_directory, compare.use_case_sensitive_file_names))
            .collect();
        let mut changed = None;
        for (path, kind) in &changes {
            if previous.get_source_file_by_path(path).is_some() {
                if *kind == tsrs_fswatch::EventKind::Delete || changed.replace(path.clone()).is_some() {
                    return None;
                }
                continue;
            }
            if output_dirs.iter().any(|dir| tspath::contains_path(dir.as_str(), path.as_str(), compare))
                || changes.iter().any(|(other, _)| other != path && tspath::contains_path(path.as_str(), other.as_str(), compare))
            {
                continue;
            }
            if could_change_resolution(path) || fs.directory_exists(path.as_str()) {
                return None;
            }
        }
        changed
    }

    // tsrs-only (watch mode): the changed paths that made the task's inputs out of date (`single_changed_file`).
    pub(crate) fn set_watch_changes(&self, changes: Option<Vec<(Path, tsrs_fswatch::EventKind)>>) {
        *self.watch_changes.lock().unwrap() = changes;
    }

    // buildtask.go:821
    pub(crate) fn update_watch(&self, orchestrator: &Orchestrator, old_cache: &super::host::mTimeCache) {
        if self.resolved_opt().is_some() && self.can_update_js_dts_output_timestamps() {
            for output_file in self.resolved().get_output_file_names() {
                orchestrator.host().store_m_time_from_old_cache(&output_file, old_cache);
            }
        }
    }

    // buildtask.go:225
    fn compile_and_emit(&'static self, orchestrator: &'static Orchestrator, path: &Path) {
        self.errors.lock().unwrap().clear();
        let build_options = &orchestrator.opts.command.build_options;
        if build_options.verbose.is_true() {
            self.report_status(diag(&diagnostics::Building_project_0, &[&orchestrator.relative_file_name(&self.config)]));
        }

        // Real build
        let mut compile_times = CompileTimes::default();
        let host = orchestrator.host();
        compile_times.config_time = host.config_times.lock().unwrap().get(path).copied().unwrap_or_default();
        let sys = orchestrator.opts.sys;
        let build_info_read_start = sys.now();
        let (content_mapper_project, err) = self.get_content_mapper_project(orchestrator);
        if let Some(err) = err {
            self.report_diagnostic(tsrs_compiler::content_mapper_project_diagnostic(&err));
            self.set_status(upToDateStatus::new(upToDateStatusType::BuildErrors));
            self.set_exit_status(ExitStatus::DiagnosticsPresent_OutputsSkipped);
            return;
        }
        let builder = Arc::clone(&self.result.lock().unwrap().as_ref().unwrap().builder);
        let trace_builder = Arc::clone(&builder);
        let compiler_host: Arc<dyn tsrs_compiler::CompilerHost> = Arc::new(compilerHost {
            host,
            trace: tsc::get_trace_with_writer_from_sys(Arc::new(move |t: &str| trace_builder.lock().unwrap().push_str(t)), false, orchestrator.opts.testing),
            content_mapper_project,
        });
        let watch = orchestrator.opts.command.compiler_options.watch.is_true();
        // The region of the diagnostics the kept snapshot keeps goes after the new snapshot copied those it keeps.
        let (kept, kept_diagnostics) = match watch.then(|| self.watch_incremental.lock().unwrap().take()).flatten() {
            Some((kept, region)) => (Some(kept), region),
            None => (None, None),
        };
        let previous = kept.and_then(|kept| self.old_watch_state(orchestrator, kept));
        let mut old_program = None;
        if !build_options.force.is_true() {
            old_program = match previous {
                Some(_) => kept,
                None => tsrs_incremental::read_build_info_program(self.resolved(), host, &*compiler_host),
            };
        }
        compile_times.build_info_read_time = sys.now() - build_info_read_start;
        let parse_start = sys.now();
        let reused = previous.and_then(|previous| self.reuse_watch_program(orchestrator, previous, &compiler_host));
        let mut regions = None;
        let program = reused.unwrap_or_else(|| {
            let mut program_options = ProgramOptions::new(self.resolved(), compiler_host);
            program_options.canonical_diagnostics = orchestrator.uses_canonical_diagnostics();
            // Watch mode frees the program when the task builds again (`replace_watch_program`), with the diagnostics
            // its checkers make: the task's errors are cleared first, and the snapshot that keeps some copies them.
            program_options.checker_regions = watch;
            // `watch_full_build` keeps the parents of the synthetic imports the reused programs keep.
            program_options.reuse_synthetic_imports = watch;
            program_options.checker_diagnostic_regions = watch && orchestrator.opts.testing.is_none();
            regions = watch.then(|| Arc::new(tsrs_compiler::ProgramRegions::default()));
            program_options.regions.clone_from(&regions);
            new_program(program_options)
        });
        self.built_program.store(true, Ordering::SeqCst);
        if watch {
            // Bind before any checker exists: single-threaded, the first checker would bind the files in its region,
            // and later cycles reuse the parsed files (host.rs `get_watch_source_file`).
            program.bind_source_files();
        }
        compile_times.parse_time = sys.now() - parse_start;
        let changes_compute_start = sys.now();
        // The harness reads the old snapshot's diagnostics, which the new one shares (`copy_diagnostics`).
        let snapshot_diagnostics = (watch && orchestrator.opts.testing.is_none()).then(|| Region::new_scratch(64 << 10));
        let incremental_program = {
            // The diagnostics the snapshot recomputes for the new program (`repopulate_diagnostics_of_file`).
            let _route = snapshot_diagnostics.as_ref().map(Region::route_diagnostics);
            tsrs_incremental::new_program(program, old_program, Box::new(incrementalHost(host)), Some(std::time::Instant::now), orchestrator.opts.testing.is_some())
        };
        if let Some(region) = &snapshot_diagnostics {
            let _scope = region.enter();
            incremental_program.copy_diagnostics();
            if let Some(old_program) = old_program {
                // SAFETY: `incremental_program` was made from it, and nothing else holds it: it is the task's kept
                // state (taken above) or was read from the build info for this build alone.
                unsafe { tsrs_incremental::Program::drop_replaced(old_program) };
            }
            if let Some(kept) = kept
                && old_program != Some(kept)
            {
                // SAFETY: the task's kept state, taken above, which nothing else holds, and which this build did not
                // use.
                unsafe { tsrs_incremental::Program::drop_replaced(kept) };
            }
        }
        self.result.lock().unwrap().as_mut().unwrap().program = Some(incremental_program);
        compile_times.changes_compute_time = sys.now() - changes_compute_start;
        if watch {
            // After the incremental program is made: its snapshot reads the program it replaces.
            self.replace_watch_program(orchestrator, program, incremental_program, snapshot_diagnostics, kept_diagnostics, regions);
        }

        let this: &'static BuildTask = self;
        let report_diagnostic: tsc::DiagnosticReporter = Box::new(move |d| this.report_diagnostic(d));
        let report_error_summary = tsc::quiet_diagnostics_reporter();
        let writer_builder = builder;
        let writer = move |t: &str| writer_builder.lock().unwrap().push_str(t);
        // Called from the checker threads: BuildTask and Orchestrator are Sync (Mutex/atomic state).
        let write_file = move |file_name: &str, text: &str, data: &mut WriteFileData| this.write_file(orchestrator, file_name, text, data);
        // Go passes `orchestrator.host.mTimes` (the map pointer). Take the Arc here: a guard created inside the call's
        // argument list would live until the call returns, and `host.get_m_time` (WriteFile of a `.d.ts` that differs
        // only in its map, emitfileshandler.go:240) locks the same mutex during emit, which deadlocked.
        let testing_m_times_cache = host.m_times.lock().unwrap().clone();
        let (result, statistics) = emit_and_report_statistics(&EmitInput {
            sys,
            program,
            config: self.resolved(),
            report_diagnostic: &report_diagnostic,
            report_error_summary: &report_error_summary,
            compile_times,
            incremental: Some(incremental_program),
            writer: Some(&writer),
            write_file: Some(&write_file),
            testing: orchestrator.opts.testing,
            testing_m_times_cache: Some(testing_m_times_cache),
        });
        {
            let mut r = self.result.lock().unwrap();
            let r = r.as_mut().unwrap();
            r.exit_status = result.status;
            r.statistics = statistics;
        }
        *self.package_jsons.lock().unwrap() = incremental_program.package_json_lookup_paths();
        let emitted_files = result.emitted_files.clone();
        if (!program.options().no_emit_on_error.is_true() || result.diagnostics.is_empty())
            && (!emitted_files.is_empty() || self.status().kind != upToDateStatusType::OutOfDateBuildInfoWithErrors)
        {
            // Update time stamps for rest of the outputs
            self.update_time_stamps(orchestrator, &emitted_files, &diagnostics::Updating_unchanged_output_timestamps_of_project_0);
        }
        self.result.lock().unwrap().as_mut().unwrap().build_kind = buildKind::Program;
        if result.status == ExitStatus::DiagnosticsPresent_OutputsSkipped || result.status == ExitStatus::DiagnosticsPresent_OutputsGenerated {
            self.set_status(upToDateStatus::new(upToDateStatusType::BuildErrors));
        } else {
            let oldest_output_file_name = if !emitted_files.is_empty() {
                emitted_files[0].clone()
            } else {
                self.resolved().get_output_file_names().into_iter().next().unwrap_or_default()
            };
            self.set_status(upToDateStatus::with(upToDateStatusType::UpToDate, statusData::String(oldest_output_file_name)));
        }
    }

    // buildtask.go:296
    fn handle_status_that_doesnt_require_build(&self, orchestrator: &Orchestrator) -> bool {
        let build_options = &orchestrator.opts.command.build_options;
        let status = self.status();
        match status.kind {
            upToDateStatusType::UpToDate => {
                if build_options.dry.is_true() {
                    self.report_status(diag(&diagnostics::Project_0_is_up_to_date, &[&self.config]));
                }
                return true;
            }
            upToDateStatusType::UpstreamErrors => {
                let upstream_status = status.upstream_errors();
                if build_options.verbose.is_true() {
                    self.report_status(diag(
                        if upstream_status.ref_has_upstream_errors {
                            &diagnostics::Skipping_build_of_project_0_because_its_dependency_1_was_not_built
                        } else {
                            &diagnostics::Skipping_build_of_project_0_because_its_dependency_1_has_errors
                        },
                        &[&orchestrator.relative_file_name(&self.config), &orchestrator.relative_file_name(&upstream_status.ref_)],
                    ));
                }
                return true;
            }
            upToDateStatusType::Solution => return true,
            upToDateStatusType::ConfigFileNotFound => {
                self.report_diagnostic(diag(&diagnostics::File_0_not_found, &[&self.config]));
                return true;
            }
            _ => {}
        }

        // update timestamps
        if status.is_pseudo_build() {
            if build_options.dry.is_true() {
                self.report_status(diag(&diagnostics::A_non_dry_build_would_update_timestamps_for_output_of_project_0, &[&self.config]));
                self.set_status(upToDateStatus::new(upToDateStatusType::UpToDate));
                return true;
            }

            self.update_time_stamps(orchestrator, &[], &diagnostics::Updating_output_timestamps_of_project_0);
            self.set_status(upToDateStatus::with(upToDateStatusType::UpToDate, status.data));
            self.result.lock().unwrap().as_mut().unwrap().build_kind = buildKind::Pseudo;
            return true;
        }

        if build_options.dry.is_true() {
            self.report_status(diag(&diagnostics::A_non_dry_build_would_build_project_0, &[&self.config]));
            self.set_status(upToDateStatus::new(upToDateStatusType::UpToDate));
            return true;
        }
        false
    }

    // buildtask.go:353
    fn get_up_to_date_status(&self, orchestrator: &Orchestrator, config_path: &Path) -> upToDateStatus {
        let status = self.status.lock().unwrap().clone();
        if let Some(status) = status {
            return status;
        }
        // Config file not found
        let Some(resolved) = self.resolved_opt() else {
            return upToDateStatus::new(upToDateStatusType::ConfigFileNotFound);
        };

        // Solution - nothing to build
        if resolved.file_names().is_empty() && !resolved.project_references_is_nil() {
            return upToDateStatus::new(upToDateStatusType::Solution);
        }

        let build_options = &orchestrator.opts.command.build_options;
        let upstream_error = self.up_stream.lock().unwrap().iter().find_map(|upstream| {
            if build_options.stop_build_on_errors.is_true() && upstream.task.status().is_error() {
                // Upstream project has errors, so we cannot build this project
                return Some(upToDateStatus::with(
                    upToDateStatusType::UpstreamErrors,
                    statusData::UpstreamErrors(upstreamErrors {
                        ref_: resolved.project_references()[upstream.ref_index].path.clone(),
                        ref_has_upstream_errors: upstream.task.status().kind == upToDateStatusType::UpstreamErrors,
                    }),
                ));
            }

            None
        });
        if let Some(result) = upstream_error {
            return result;
        }

        if build_options.force.is_true() {
            return upToDateStatus::new(upToDateStatusType::ForceBuild);
        }

        // Check the build info
        let build_info_path = resolved.get_build_info_file_name();
        let build_info_directory = std::cell::OnceCell::new();
        let get_build_info_directory = || -> String {
            build_info_directory
                .get_or_init(|| {
                    tspath::get_directory_path(&tspath::get_normalized_absolute_path(
                        &build_info_path,
                        &orchestrator.compare_paths_options.current_directory,
                    ))
                })
                .clone()
        };
        let (build_info, build_info_time) = self.load_or_store_build_info(orchestrator, config_path, &build_info_path);
        let Some(build_info) = build_info else {
            return upToDateStatus::with(upToDateStatusType::OutputMissing, statusData::String(build_info_path));
        };

        // build info version
        if !build_info.is_valid_version() {
            return upToDateStatus::with(upToDateStatusType::TsVersionOutputOfDate, statusData::String(build_info.version.clone()));
        }

        // If a configured content mapper's identity has changed, files it produced may be stale.
        let (content_mapper_project, err) = self.get_content_mapper_project(orchestrator);
        let (content_mapper_identities, identity_err) = match tsrs_incremental::content_mapper_identities(content_mapper_project.as_deref()) {
            Ok(identities) => (identities, None),
            Err(identity_err) => (None, Some(identity_err)),
        };
        if let Some(identity_err) = &identity_err {
            *self.content_mapper_project_err.lock().unwrap() = Some(identity_err.clone());
        }
        if err.is_some() || identity_err.is_some() || !build_info.content_mapper_identities_match(content_mapper_identities.as_deref()) {
            return upToDateStatus::with(upToDateStatusType::OutOfDateOptions, statusData::String(build_info_path));
        }

        let options = resolved.compiler_options().unwrap();
        // Report errors if build info indicates errors
        if build_info.errors || // Errors that need to be reported irrespective of "--noCheck"
            (!options.no_check.is_true() && (build_info.semantic_errors || build_info.check_pending))
        {
            // Errors without --noCheck
            return upToDateStatus::with(upToDateStatusType::OutOfDateBuildInfoWithErrors, statusData::String(build_info_path));
        }

        if options.is_incremental() {
            if !build_info.is_incremental() {
                // Program options out of date
                return upToDateStatus::with(upToDateStatusType::OutOfDateOptions, statusData::String(build_info_path));
            }

            // Errors need to be reported if build info has errors
            // (Go compares the slices with nil: a field absent from the tsbuildinfo is nil.)
            if (options.get_emit_declarations() && !build_info.emit_diagnostics_per_file.is_empty()) || // Always reported errors
                (!options.no_check.is_true() && // Semantic errors if not --noCheck
                    (!build_info.change_file_set.is_empty() || !build_info.semantic_diagnostics_per_file.is_empty()))
            {
                return upToDateStatus::with(upToDateStatusType::OutOfDateBuildInfoWithErrors, statusData::String(build_info_path));
            }

            // Pending emit files
            if !options.no_emit.is_true() && (!build_info.change_file_set.is_empty() || !build_info.affected_files_pending_emit.is_empty()) {
                return upToDateStatus::with(upToDateStatusType::OutOfDateBuildInfoWithPendingEmit, statusData::String(build_info_path));
            }

            // Some of the emit files like source map or dts etc are not yet done
            if build_info.is_emit_pending(&resolved, &get_build_info_directory()) {
                return upToDateStatus::with(upToDateStatusType::OutOfDateOptions, statusData::String(build_info_path));
            }
        }
        let mut input_text_unchanged = false;
        let mut oldest_output_file_and_time = fileAndTime { file: build_info_path.clone(), time: build_info_time };
        let mut newest_input_file_and_time = fileAndTime::default();
        let mut seen_roots: Set<Path> = Set::default();
        let root_info_reader: std::cell::OnceCell<BuildInfoRootInfoReader> = std::cell::OnceCell::new();
        let get_build_info_root_info_reader =
            || root_info_reader.get_or_init(|| build_info.get_build_info_root_info_reader(&get_build_info_directory(), &orchestrator.compare_paths_options));
        let host = orchestrator.host();
        let testing = orchestrator.opts.testing.is_some();
        for input_file in resolved.file_names() {
            let input_time = host.get_m_time(input_file);
            if input_time.is_none() {
                return upToDateStatus::with(upToDateStatusType::InputFileMissing, statusData::String(input_file.clone()));
            }
            let input_path = orchestrator.to_path(input_file);
            if after(input_time, oldest_output_file_and_time.time) {
                let mut version = String::new();
                let mut current_version = String::new();
                if build_info.is_incremental() {
                    let (build_info_file_info, resolved_input_path) = get_build_info_root_info_reader().get_build_info_file_info(&input_path);
                    if let Some(file_info) = build_info_file_info.map(tsrs_incremental::BuildInfoFileInfo::get_file_info) {
                        if !file_info.version().is_empty() {
                            version = file_info.version().to_string();
                            if let Some(text) = tsrs_compiler::CompilerHost::fs(host).read_file(&resolved_input_path) {
                                current_version = tsrs_incremental::compute_hash(&text, testing);
                                if version == current_version {
                                    input_text_unchanged = true;
                                }
                            }
                        }
                    }
                }

                if version.is_empty() || version != current_version {
                    return upToDateStatus::with(
                        upToDateStatusType::InputFileNewer,
                        statusData::InputOutputName(inputOutputName { input: input_file.clone(), output: build_info_path }),
                    );
                }
            }
            if after(input_time, newest_input_file_and_time.time) {
                newest_input_file_and_time = fileAndTime { file: input_file.clone(), time: input_time };
            }
            seen_roots.add(input_path);
        }

        if let Some(root) = get_build_info_root_info_reader().roots().find(|root| !seen_roots.has(root)) {
            // File was root file when project was built but its not any more
            return upToDateStatus::with(
                upToDateStatusType::OutOfDateRoots,
                statusData::InputOutputName(inputOutputName { input: root.to_string(), output: build_info_path }),
            );
        }

        if build_info.is_incremental() {
            let mut resolved_roots: Set<Path> = Set::default();
            resolved_roots.m.extend(get_build_info_root_info_reader().roots().filter_map(|root| {
                let (_, resolved) = get_build_info_root_info_reader().get_build_info_file_info(root);
                if !resolved.is_empty() { Some(resolved) } else { None }
            }));
            for (index, build_info_file_info) in build_info.file_infos.iter().enumerate() {
                let build_info_file_name = &build_info.file_names[index];
                // Lib files bundled with the compiler can change only with the version of the compiler,
                // which is already verified with buildInfo.Version
                if is_build_info_file_name_default_library(build_info_file_name) {
                    continue;
                }
                let input_file = tspath::get_normalized_absolute_path(build_info_file_name, &get_build_info_directory());
                let input_path = orchestrator.to_path(&input_file);
                // Root files are already checked
                if seen_roots.has(&input_path) || resolved_roots.has(&input_path) {
                    continue;
                }
                if is_content_mapper_supplemental_build_info_path(&input_path, get_build_info_root_info_reader().roots())
                    && !tsrs_compiler::CompilerHost::fs(host).file_exists(&input_file)
                {
                    continue;
                }
                let input_time = host.get_m_time(&input_file);
                if input_time.is_none() {
                    // Input file that was part of the program is missing (eg: dependency was removed)
                    return upToDateStatus::with(upToDateStatusType::InputFileMissing, statusData::String(input_file));
                }

                if after(input_time, oldest_output_file_and_time.time) {
                    let mut current_version = String::new();
                    let version = build_info_file_info.get_file_info().version().to_string();
                    if !version.is_empty() {
                        if let Some(text) = tsrs_compiler::CompilerHost::fs(host).read_file(&input_file) {
                            current_version = tsrs_incremental::compute_hash(&text, testing);
                        }
                    }
                    if version.is_empty() || version != current_version {
                        return upToDateStatus::with(
                            upToDateStatusType::InputFileNewer,
                            statusData::InputOutputName(inputOutputName { input: input_file, output: build_info_path }),
                        );
                    }
                    input_text_unchanged = true;
                }
            }
        }

        if !options.is_incremental() {
            // Check output file stamps
            for output_file in resolved.get_output_file_names() {
                let output_time = host.get_m_time(&output_file);
                if output_time.is_none() {
                    // Output file missing
                    return upToDateStatus::with(upToDateStatusType::OutputMissing, statusData::String(output_file));
                }

                if before(output_time, newest_input_file_and_time.time) {
                    // Output file is older than input file
                    return upToDateStatus::with(
                        upToDateStatusType::InputFileNewer,
                        statusData::InputOutputName(inputOutputName { input: newest_input_file_and_time.file, output: output_file }),
                    );
                }

                if before(output_time, oldest_output_file_and_time.time) {
                    oldest_output_file_and_time = fileAndTime { file: output_file, time: output_time };
                }
            }
        }

        let mut ref_dts_unchanged = false;
        for upstream in self.up_stream.lock().unwrap().iter() {
            let upstream_status = upstream.task.status();
            if upstream_status.kind == upToDateStatusType::Solution {
                // Not dependent on the status or this upstream project
                // (eg: expected cycle was detected and hence skipped, or is solution)
                continue;
            }

            // If the upstream project's newest file is older than our oldest output,
            // we can't be out of date because of it
            // inputTime will not be present if we just built this project or updated timestamps
            // - in that case we do want to either build or update timestamps
            if let Some(ref_input_output_file_and_time) = upstream_status.input_output_file_and_time() {
                if ref_input_output_file_and_time.input.time.is_some()
                    && before(ref_input_output_file_and_time.input.time, oldest_output_file_and_time.time)
                {
                    continue;
                }
            }

            // Check if tsbuildinfo path is shared, then we need to rebuild
            if self.has_conflicting_build_info(upstream.task) {
                // We have an output older than an upstream output - we are out of date
                return upToDateStatus::with(
                    upToDateStatusType::InputFileNewer,
                    statusData::InputOutputName(inputOutputName {
                        input: resolved.project_references()[upstream.ref_index].path.clone(),
                        output: oldest_output_file_and_time.file,
                    }),
                );
            }

            // If the upstream project has only change .d.ts files, and we've built
            // *after* those files, then we're "pseudo up to date" and eligible for a fast rebuild
            let newest_dts_change_time = upstream.task.get_latest_changed_dts_m_time(orchestrator);
            if newest_dts_change_time.is_some() && before(newest_dts_change_time, oldest_output_file_and_time.time) {
                ref_dts_unchanged = true;
                continue;
            }

            // We have an output older than an upstream output - we are out of date
            return upToDateStatus::with(
                upToDateStatusType::InputFileNewer,
                statusData::InputOutputName(inputOutputName {
                    input: resolved.project_references()[upstream.ref_index].path.clone(),
                    output: oldest_output_file_and_time.file,
                }),
            );
        }

        let check_input_file_time = |input_file: &str| -> Option<upToDateStatus> {
            let input_time = host.get_m_time(input_file);
            if after(input_time, oldest_output_file_and_time.time) {
                // Output file is older than input file
                return Some(upToDateStatus::with(
                    upToDateStatusType::InputFileNewer,
                    statusData::InputOutputName(inputOutputName { input: input_file.to_string(), output: oldest_output_file_and_time.file.clone() }),
                ));
            }
            None
        };

        if let Some(config_status) = check_input_file_time(&self.config) {
            return config_status;
        }

        if let Some(result) = resolved.extended_source_files().into_iter().find_map(|extended_config| {
            if let Some(extended_config_status) = check_input_file_time(&extended_config) {
                return Some(extended_config_status);
            }

            None
        }) {
            return result;
        }

        let package_jsons = build_info.get_package_jsons(&get_build_info_directory());
        if let Some(result) = package_jsons.iter().find_map(|package_json| {
            let package_json_time = host.get_m_time(package_json);
            if package_json_time.is_none() {
                return Some(upToDateStatus::with(upToDateStatusType::InputFileMissing, statusData::String(package_json.clone())));
            }
            if after(package_json_time, oldest_output_file_and_time.time) {
                return Some(upToDateStatus::with(
                    upToDateStatusType::InputFileNewer,
                    statusData::InputOutputName(inputOutputName { input: package_json.clone(), output: oldest_output_file_and_time.file.clone() }),
                ));
            }

            None
        }) {
            return result;
        }
        let missing_package_jsons = build_info.get_missing_package_jsons(&get_build_info_directory());
        if let Some(result) = missing_package_jsons.iter().find_map(|package_json| {
            if host.get_m_time(package_json).is_some() {
                Some(upToDateStatus::with(
                    upToDateStatusType::InputFileNewer,
                    statusData::InputOutputName(inputOutputName { input: package_json.clone(), output: oldest_output_file_and_time.file.clone() }),
                ))
            } else {
                None
            }
        }) {
            return result;
        }
        let mut all = package_jsons;
        all.extend(missing_package_jsons);
        *self.package_jsons.lock().unwrap() = all;

        upToDateStatus::with(
            if ref_dts_unchanged {
                upToDateStatusType::UpToDateWithUpstreamTypes
            } else if input_text_unchanged {
                upToDateStatusType::UpToDateWithInputFileText
            } else {
                upToDateStatusType::UpToDate
            },
            statusData::InputOutputFileAndTime(inputOutputFileAndTime {
                input: newest_input_file_and_time,
                output: oldest_output_file_and_time,
            }),
        )
    }

    // buildtask.go:611
    fn report_up_to_date_status(&self, orchestrator: &Orchestrator) {
        if !orchestrator.opts.command.build_options.verbose.is_true() {
            return;
        }
        let rel = |f: &str| orchestrator.relative_file_name(f);
        let config = rel(&self.config);
        let status = self.status();
        match status.kind {
            upToDateStatusType::ConfigFileNotFound => {
                self.report_status(diag(&diagnostics::Project_0_is_out_of_date_because_config_file_does_not_exist, &[&config]));
            }
            upToDateStatusType::UpstreamErrors => {
                let upstream_status = status.upstream_errors();
                self.report_status(diag(
                    if upstream_status.ref_has_upstream_errors {
                        &diagnostics::Project_0_can_t_be_built_because_its_dependency_1_was_not_built
                    } else {
                        &diagnostics::Project_0_can_t_be_built_because_its_dependency_1_has_errors
                    },
                    &[&config, &rel(&upstream_status.ref_)],
                ));
            }
            upToDateStatusType::BuildErrors => self.report_status(diag(&diagnostics::Project_0_is_out_of_date_because_it_has_errors, &[&config])),
            upToDateStatusType::UpToDate => {
                // This is to ensure skipping verbose log for projects that were built,
                // and then some other package changed but this package doesnt need update
                if let Some(input_output_file_and_time) = status.input_output_file_and_time() {
                    self.report_status(diag(
                        &diagnostics::Project_0_is_up_to_date_because_newest_input_1_is_older_than_output_2,
                        &[&config, &rel(&input_output_file_and_time.input.file), &rel(&input_output_file_and_time.output.file)],
                    ));
                }
            }
            upToDateStatusType::UpToDateWithUpstreamTypes => {
                self.report_status(diag(&diagnostics::Project_0_is_up_to_date_with_d_ts_files_from_its_dependencies, &[&config]));
            }
            upToDateStatusType::UpToDateWithInputFileText => self.report_status(diag(
                &diagnostics::Project_0_is_up_to_date_but_needs_to_update_timestamps_of_output_files_that_are_older_than_input_files,
                &[&config],
            )),
            upToDateStatusType::InputFileMissing => self.report_status(diag(
                &diagnostics::Project_0_is_out_of_date_because_input_1_does_not_exist,
                &[&config, &rel(status.data_string())],
            )),
            upToDateStatusType::OutputMissing => self.report_status(diag(
                &diagnostics::Project_0_is_out_of_date_because_output_file_1_does_not_exist,
                &[&config, &rel(status.data_string())],
            )),
            upToDateStatusType::InputFileNewer => {
                let input_output = status.input_output_name().unwrap();
                self.report_status(diag(
                    &diagnostics::Project_0_is_out_of_date_because_output_1_is_older_than_input_2,
                    &[&config, &rel(&input_output.output), &rel(&input_output.input)],
                ));
            }
            upToDateStatusType::OutOfDateBuildInfoWithPendingEmit => self.report_status(diag(
                &diagnostics::Project_0_is_out_of_date_because_buildinfo_file_1_indicates_that_some_of_the_changes_were_not_emitted,
                &[&config, &rel(status.data_string())],
            )),
            upToDateStatusType::OutOfDateBuildInfoWithErrors => self.report_status(diag(
                &diagnostics::Project_0_is_out_of_date_because_buildinfo_file_1_indicates_that_program_needs_to_report_errors,
                &[&config, &rel(status.data_string())],
            )),
            upToDateStatusType::OutOfDateOptions => self.report_status(diag(
                &diagnostics::Project_0_is_out_of_date_because_buildinfo_file_1_indicates_there_is_change_in_compilerOptions,
                &[&config, &rel(status.data_string())],
            )),
            upToDateStatusType::OutOfDateRoots => {
                let input_output = status.input_output_name().unwrap();
                self.report_status(diag(
                    &diagnostics::Project_0_is_out_of_date_because_buildinfo_file_1_indicates_that_file_2_was_root_file_of_compilation_but_not_any_more,
                    &[&config, &rel(&input_output.output), &rel(&input_output.input)],
                ));
            }
            upToDateStatusType::TsVersionOutputOfDate => self.report_status(diag(
                &diagnostics::Project_0_is_out_of_date_because_output_for_it_was_generated_with_version_1_that_differs_with_current_version_2,
                &[&config, &rel(status.data_string()), &tsrs_core::version()],
            )),
            upToDateStatusType::ForceBuild => self.report_status(diag(&diagnostics::Project_0_is_being_forcibly_rebuilt, &[&config])),
            upToDateStatusType::Solution => {
                // Does not need to report status
            }
        }
    }

    // buildtask.go:735
    fn can_update_js_dts_output_timestamps(&self) -> bool {
        let options = self.resolved().compiler_options().unwrap();
        !options.no_emit.is_true() && !options.is_incremental()
    }

    // buildtask.go:739
    fn update_time_stamps(&self, orchestrator: &Orchestrator, emitted_files: &[String], verbose_message: &'static Message) {
        let emitted: Set<&str> = new_set_from_items(emitted_files.iter().map(std::string::String::as_str));
        let mut verbose_message_reported = false;
        let build_info_name = self.resolved().get_build_info_file_name();
        let now = orchestrator.opts.sys.now_time();
        let mut update_time_stamp = |file: &str| {
            if emitted.has(&file) {
                return;
            }
            if !verbose_message_reported && orchestrator.opts.command.build_options.verbose.is_true() {
                self.report_status(diag(verbose_message, &[&orchestrator.relative_file_name(&self.config)]));
                verbose_message_reported = true;
            }
            let err = orchestrator.host().set_m_time(file, now);
            if err.is_ok() {
                if file == build_info_name {
                    let mut entry = self.build_info_entry.lock().unwrap();
                    if let Some(entry) = entry.as_mut() {
                        entry.m_time = Some(now);
                    }
                } else if self.store_output_time_stamp(orchestrator) {
                    orchestrator.host().store_m_time(file, now);
                }
            }
        };

        if self.can_update_js_dts_output_timestamps() {
            for output_file in self.resolved().get_output_file_names() {
                update_time_stamp(&output_file);
            }
        }
        update_time_stamp(&self.resolved().get_build_info_file_name());
    }

    // buildtask.go:777
    pub(crate) fn clean_project(&self, orchestrator: &Orchestrator, _path: &Path) {
        let Some(resolved) = self.resolved_opt() else {
            self.report_diagnostic(diag(&diagnostics::File_0_not_found, &[&self.config]));
            self.set_exit_status(ExitStatus::DiagnosticsPresent_OutputsSkipped);
            return;
        };

        let inputs: Set<Path> = new_set_from_items(resolved.file_names().iter().map(|f| orchestrator.to_path(f)));
        for output_file in resolved.get_output_file_names() {
            self.clean_project_output(orchestrator, &output_file, &inputs);
        }
        self.clean_project_output(orchestrator, &resolved.get_build_info_file_name(), &inputs);
    }

    // buildtask.go:791
    fn clean_project_output(&self, orchestrator: &Orchestrator, output_file: &str, inputs: &Set<Path>) {
        let output_path = orchestrator.to_path(output_file);
        // If output name is same as input file name, do not delete and ignore the error
        if inputs.has(&output_path) {
            return;
        }
        let fs = tsrs_compiler::CompilerHost::fs(orchestrator.host());
        if fs.file_exists(output_file) {
            if !orchestrator.opts.command.build_options.dry.is_true() {
                if fs.remove(output_file).is_err() {
                    self.report_diagnostic(diag(&diagnostics::Failed_to_delete_file_0, &[&output_file]));
                }
            } else {
                self.result.lock().unwrap().as_mut().unwrap().files_to_delete.push(output_file.to_string());
            }
        }
    }

    // buildtask.go:833
    pub(crate) fn load_or_store_build_info(
        &self,
        orchestrator: &Orchestrator,
        _config_path: &Path,
        build_info_file_name: &str,
    ) -> (Option<Arc<BuildInfo>>, Option<SystemTime>) {
        let path = orchestrator.to_path(build_info_file_name);
        let mut entry = self.build_info_entry.lock().unwrap();
        if let Some(entry) = entry.as_ref() {
            if entry.path == path {
                return (entry.build_info.clone(), entry.m_time);
            }
        }
        let build_info = tsrs_incremental::new_build_info_reader_with_canonical_diagnostics(
            Arc::clone(&orchestrator.host().host),
            orchestrator.uses_canonical_diagnostics(),
        )
        .read_build_info(&self.resolved())
        .map(Arc::new);
        let m_time = if build_info.is_some() { orchestrator.host().get_m_time(build_info_file_name) } else { None };
        *entry = Some(buildInfoEntry { build_info: build_info.clone(), path, m_time, dts_time: None });
        (build_info, m_time)
    }

    // buildtask.go:852
    fn on_build_info_emit(&self, orchestrator: &Orchestrator, build_info_file_name: &str, build_info: Arc<BuildInfo>, has_changed_dts_file: bool) {
        let mut entry = self.build_info_entry.lock().unwrap();
        let m_time = orchestrator.opts.sys.now_time();
        let dts_time = if has_changed_dts_file {
            Some(Some(m_time))
        } else {
            entry.as_ref().and_then(|e| e.dts_time)
        };
        *entry = Some(buildInfoEntry { build_info: Some(build_info), path: orchestrator.to_path(build_info_file_name), m_time: Some(m_time), dts_time });
    }

    // buildtask.go:870
    fn has_conflicting_build_info(&self, upstream: P<BuildTask>) -> bool {
        let a = self.build_info_entry.lock().unwrap();
        let b = upstream.build_info_entry.lock().unwrap();
        if let (Some(a), Some(b)) = (a.as_ref(), b.as_ref()) {
            return a.path == b.path;
        }
        false
    }

    // buildtask.go:877
    fn get_latest_changed_dts_m_time(&self, orchestrator: &Orchestrator) -> Option<SystemTime> {
        let mut entry = self.build_info_entry.lock().unwrap();
        let entry = entry.as_mut().unwrap();
        if let Some(dts_time) = entry.dts_time {
            return dts_time;
        }
        let dts_time = orchestrator.host().get_m_time(&tspath::get_normalized_absolute_path(
            &entry.build_info.as_ref().unwrap().latest_changed_dts_file,
            &tspath::get_directory_path(&entry.path),
        ));
        entry.dts_time = Some(dts_time);
        dts_time
    }

    // buildtask.go:893
    fn store_output_time_stamp(&self, orchestrator: &Orchestrator) -> bool {
        orchestrator.opts.command.compiler_options.watch.is_true() && !self.resolved().compiler_options().unwrap().is_incremental()
    }

    // buildtask.go:897
    fn write_file(&self, orchestrator: &Orchestrator, file_name: &str, text: &str, data: &mut WriteFileData) -> Result<(), String> {
        let host = orchestrator.host();
        let err = tsrs_compiler::CompilerHost::fs(host).write_file(file_name, text);
        if err.is_ok() {
            match data.build_info.clone() {
                Some(build_info) => {
                    let build_info = build_info.downcast::<BuildInfo>().unwrap();
                    let has_changed_dts_file = self.result.lock().unwrap().as_ref().unwrap().program.unwrap().has_changed_dts_file();
                    self.on_build_info_emit(orchestrator, file_name, build_info, has_changed_dts_file);
                }
                _ => {
                    if self.store_output_time_stamp(orchestrator) {
                        // Store time stamps
                        host.store_m_time(file_name, orchestrator.opts.sys.now_time());
                    }
                }
            }
        }
        err
    }
}

// buildtask.go:621
pub(crate) fn is_content_mapper_supplemental_build_info_path<'a>(input_path: &Path, roots: impl IntoIterator<Item = &'a Path>) -> bool {
    if roots.into_iter().any(|root| {
        let Some(suffix) = input_path.as_str().strip_prefix(root.as_str()).and_then(|rest| rest.strip_prefix('.')) else {
            return false;
        };
        let Some((index, extension)) = suffix.split_once('.') else {
            return false;
        };
        if extension.is_empty() {
            return false;
        }
        // Go `strconv.Atoi`: an optional sign and decimal digits.
        let is_int = index.parse::<i64>().is_ok();
        is_int && tsrs_contentmapper::is_supported_virtual_extension(&format!(".{extension}"))
    }) {
        return true;
    }
    false
}

// Whether a changed file that is not a file of a program could change its module resolution: a file module
// resolution could find, or a package.json.
fn could_change_resolution(path: &Path) -> bool {
    tspath::get_base_file_name(path.as_str()) == "package.json"
        || [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"].iter().any(|ext| tspath::file_extension_is(path.as_str(), ext))
}

#[cfg(test)]
mod buildtask_contentmapper_test {
    use tsrs_core::tspath::Path;

    use super::is_content_mapper_supplemental_build_info_path;

    // buildtask_contentmapper_test.go:11. A build info file entry that names a supplemental output of a root file
    // (`<root>.<n>.<ext>`) is skipped when the file is gone, instead of making the build out of date; anything else
    // must still be checked.
    #[test]
    fn test_is_content_mapper_supplemental_build_info_path() {
        let roots = [Path::new("/src/app.vue"), Path::new("/src/index.ts")];
        let path = Path::new;

        assert!(is_content_mapper_supplemental_build_info_path(&path("/src/app.vue.0.ts"), &roots));
        assert!(is_content_mapper_supplemental_build_info_path(&path("/src/app.vue.12.mts"), &roots));
        assert!(!is_content_mapper_supplemental_build_info_path(&path("/src/app.vue.ts"), &roots));
        assert!(!is_content_mapper_supplemental_build_info_path(&path("/src/app.vue.0.txt"), &roots));
        assert!(!is_content_mapper_supplemental_build_info_path(&path("/src/other.vue.0.ts"), &roots));
    }
}

#[cfg(test)]
mod could_change_resolution_test {
    use tsrs_core::tspath::Path;

    use super::could_change_resolution;

    #[test]
    fn paths_module_resolution_could_find() {
        for path in ["/a/b.ts", "/a/b.d.ts", "/a/b.tsx", "/a/b.mts", "/a/b.cjs", "/a/b.json", "/a/package.json"] {
            assert!(could_change_resolution(&Path::new(path)), "{path}");
        }
        for path in ["/a/b.md", "/a/b.css", "/a/b.tsbuildinfo", "/a/b.js.map", "/a/4913"] {
            assert!(!could_change_resolution(&Path::new(path)), "{path}");
        }
    }
}
