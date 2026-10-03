// Port of execute/build/orchestrator.go (the non-watch parts; watch mode is out of scope).
//
// Like Go, the build tasks run on `--builders` threads (default 4, 1 with --singleThreaded; `rangeTasks`) that take
// projects from Order() by an atomic index and block on their upstream tasks' `done` signals, while the calling thread
// (Go: a reporter goroutine) waits on each task's `built` signal in Order() and prints its buffered output. The
// signals are Mutex + Condvar stand-ins for Go's closed channels (buildtask.rs closeSignal). Each task's state is
// Mutex/atomic; the shared host caches are Mutex-guarded. At the pinned commit Go's builders iterate `order`, not
// ScheduleOrder() (whose comment says otherwise); this port does the same.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use rustc_hash::FxHashMap;
use tsrs_ast::{new_compiler_diagnostic, Diagnostic};
use tsrs_compiler::{new_cached_fs_compiler_host, CompilerHost};
use tsrs_core::collections::Set;
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::P;
use tsrs_diagnostics as diagnostics;
use tsrs_tsoptions::ParsedBuildCommandLine;

use super::buildtask::{taskResult, BuildTask};
use super::host::host;
use super::parsecache::parseCache;
use crate::tsc::{
    self, create_builder_status_reporter, create_diagnostic_reporter_with_writer, create_report_error_summary, CommandLineResult, CommandLineTesting,
    DiagnosticReporter, DiagnosticsReporter, ExitStatus, Statistics, System, Writer,
};

pub struct Options {
    pub sys: &'static dyn System,
    pub command: P<ParsedBuildCommandLine>,
    pub testing: Option<&'static dyn CommandLineTesting>,
}

#[derive(Default)]
pub struct OrchestratorResult {
    pub result: Option<CommandLineResult>,
    pub status: Option<ExitStatus>,
    pub errors: Option<Vec<P<Diagnostic>>>,
    pub statistics: Statistics,
    pub files_to_delete: Option<Vec<String>>,
}

impl OrchestratorResult {
    pub(crate) fn status(&self) -> ExitStatus {
        self.status.unwrap_or(ExitStatus::Success)
    }

    // orchestrator.go:39
    fn report(&mut self, o: &Orchestrator) {
        self.report_with_files_to_delete(o, true);
    }

    // orchestrator.go:43
    fn report_with_files_to_delete(&mut self, o: &Orchestrator, report_files_to_delete: bool) {
        (o.error_summary_reporter)(self.errors.as_deref().unwrap_or_default());
        if report_files_to_delete {
            if let Some(files_to_delete) = &self.files_to_delete {
                o.create_builder_status_reporter(None)(new_compiler_diagnostic(
                    &diagnostics::A_non_dry_build_would_delete_the_following_files_Colon_0,
                    &[&files_to_delete.iter().map(|f| format!("\r\n * {f}")).collect::<String>()],
                ));
            }
        }
        let options = &o.opts.command.compiler_options;
        if !options.diagnostics.is_true() && !options.extended_diagnostics.is_true() {
            return;
        }
        self.statistics.set_total_time(o.opts.sys.since_start());
        let sys = o.opts.sys;
        self.statistics.report(&|t: &str| sys.write(t), o.opts.testing);
    }
}

pub struct Orchestrator {
    pub(crate) opts: Options,
    pub(crate) compare_paths_options: ComparePathsOptions,
    host: OnceLock<&'static host>,

    // order generation result
    tasks: Mutex<FxHashMap<Path, P<BuildTask>>>,
    order: Mutex<Vec<String>>,
    errors: Mutex<Vec<P<Diagnostic>>>,
    graph_generated: AtomicBool,
    // Set when a builder thread panicked (see range_tasks).
    pub(crate) aborted: AtomicBool,

    error_summary_reporter: DiagnosticsReporter<'static>,

    // order sorted by dependency depth, to reduce how often builders block on upstream projects
    schedule_order: Mutex<Vec<String>>,
}

impl Orchestrator {
    pub(crate) fn host(&self) -> &'static host {
        self.host.get().unwrap()
    }

    // orchestrator.go:94
    pub(crate) fn relative_file_name(&self, file_name: &str) -> String {
        tspath::convert_to_relative_path(file_name, &self.compare_paths_options)
    }

    // orchestrator.go:98
    pub(crate) fn to_path(&self, file_name: &str) -> Path {
        tspath::to_path(file_name, &self.compare_paths_options.current_directory, self.compare_paths_options.use_case_sensitive_file_names)
    }

    pub fn order(&self) -> Vec<String> {
        self.order.lock().unwrap().clone()
    }

    // ScheduleOrder is the order in which builders pick up projects: Order() stably sorted by dependency depth.
    pub fn schedule_order(&self) -> Vec<String> {
        self.schedule_order.lock().unwrap().clone()
    }

    // orchestrator.go:131
    fn compute_schedule_order(&self) -> Vec<String> {
        let order = self.order.lock().unwrap().clone();
        let mut entries: Vec<(String, usize)> = Vec::with_capacity(order.len());
        let mut depths: FxHashMap<P<BuildTask>, usize> = FxHashMap::default();
        for config in &order {
            let task = self.get_task(&self.to_path(config));
            let mut depth = 0;
            for upstream in task.up_stream.lock().unwrap().iter() {
                depth = depth.max(depths.get(&upstream.task).copied().unwrap_or(0) + 1);
            }
            depths.insert(task, depth);
            entries.push((config.clone(), depth));
        }
        entries.sort_by_key(|e| e.1);
        entries.into_iter().map(|e| e.0).collect()
    }

    // orchestrator.go:155
    pub fn upstream(&self, config_name: &str) -> Vec<String> {
        let task = self.get_task(&self.to_path(config_name));
        let upstream = task.up_stream.lock().unwrap();
        upstream.iter().map(|t| t.task.config.clone()).collect()
    }

    // orchestrator.go:171
    pub(crate) fn get_task(&self, path: &Path) -> P<BuildTask> {
        match self.tasks.lock().unwrap().get(path) {
            Some(task) => *task,
            None => panic!("No build task found for {}", path.as_str()),
        }
    }

    // orchestrator.go:179
    fn create_build_tasks(&self, configs: &[String]) {
        for config in configs {
            let path = self.to_path(config);
            let task = P::new(BuildTask::new(config.clone(), true));
            task.pending.store(true, Ordering::SeqCst);
            if self.tasks.lock().unwrap().contains_key(&path) {
                continue;
            }
            self.tasks.lock().unwrap().insert(path.clone(), task);
            *task.resolved.lock().unwrap() = self.host().get_resolved_project_reference(config, path);
            task.up_stream.lock().unwrap().clear();
            if let Some(resolved) = task.resolved_opt() {
                self.create_build_tasks(resolved.resolved_project_reference_paths());
            }
        }
    }

    // orchestrator.go:213
    fn setup_build_task(
        &self,
        config_name: &str,
        _down_stream: Option<P<BuildTask>>,
        in_circular_context: bool,
        completed: &mut Set<Path>,
        analyzing: &mut Set<Path>,
        circularity_stack: &mut Vec<String>,
    ) -> Option<P<BuildTask>> {
        let path = self.to_path(config_name);
        let task = self.get_task(&path);
        if !completed.has(&path) {
            if analyzing.has(&path) {
                if !in_circular_context {
                    self.errors.lock().unwrap().push(new_compiler_diagnostic(
                        &diagnostics::Project_references_may_not_form_a_circular_graph_Cycle_detected_Colon_0,
                        &[&circularity_stack.join("\n")],
                    ));
                }
                return None;
            }
            analyzing.add(path.clone());
            circularity_stack.push(config_name.to_string());
            if let Some(resolved) = task.resolved_opt() {
                for (index, sub_reference) in resolved.resolved_project_reference_paths().iter().enumerate() {
                    let upstream = self.setup_build_task(
                        sub_reference,
                        Some(task),
                        in_circular_context || resolved.project_references()[index].circular,
                        completed,
                        analyzing,
                        circularity_stack,
                    );
                    if let Some(upstream) = upstream {
                        task.up_stream.lock().unwrap().push(super::buildtask::upstreamTask { task: upstream, ref_index: index });
                    }
                }
            }
            circularity_stack.pop();
            completed.add(path);
            task.built.reset();
            task.done.reset();
            self.order.lock().unwrap().push(config_name.to_string());
        }
        // Watch mode only: downStream links.
        Some(task)
    }

    // orchestrator.go:265
    pub fn generate_graph(&self) {
        let projects = self.opts.command.resolved_project_paths().to_vec();
        // Parse all config files (Go: in parallel)
        self.create_build_tasks(&projects);

        // Generate the graph
        let mut completed = Set::default();
        let mut analyzing = Set::default();
        let mut circularity_stack = Vec::new();
        for project in &projects {
            self.setup_build_task(project, None, false, &mut completed, &mut analyzing, &mut circularity_stack);
        }
        *self.schedule_order.lock().unwrap() = self.compute_schedule_order();
        self.graph_generated.store(true, Ordering::SeqCst);
    }

    // tsc -b entrypoint
    // orchestrator.go:295
    pub fn start(&'static self) -> CommandLineResult {
        CommandLineResult { status: self.start_worker("", false /*onlyReferences*/).status() }
    }

    // orchestrator.go:295/301 `Build` / `BuildReferences` entrypoints for the API. The API creates a fresh
    // orchestrator per call (crates/tsrs_cli/src/api.rs), so Go's `recheckAllProjects` (which only resets
    // state of an already generated graph) has nothing to reset here.
    pub fn build_for_api(&'static self, project: &str, only_references: bool) -> OrchestratorResult {
        self.start_worker(project, only_references)
    }

    // orchestrator.go:354-414 `Clean` / `CleanReferences` entrypoints for the API.
    pub fn clean_for_api(&'static self, project: &str, only_references: bool) -> OrchestratorResult {
        if !self.graph_generated.load(Ordering::SeqCst) {
            self.generate_graph();
        }
        let errors = self.errors.lock().unwrap().clone();
        if !errors.is_empty() {
            let mut result = OrchestratorResult { status: Some(ExitStatus::ProjectReferenceCycle_OutputsSkipped), errors: Some(errors), ..Default::default() };
            result.report_with_files_to_delete(self, true);
            return result;
        }
        let Some(mut order) = self.get_build_order_for(project) else {
            return OrchestratorResult { status: Some(ExitStatus::InvalidProject_OutputsSkipped), ..Default::default() };
        };
        if only_references {
            order.pop();
        }
        let mut result = OrchestratorResult::default();
        result.statistics.projects = order.len();
        let dry = self.opts.command.build_options.dry.is_true();
        let report_diagnostic = self.create_diagnostic_reporter(None);
        let mut files_to_delete = Vec::new();
        for config in &order {
            let task = self.get_task(&self.to_path(config));
            let Some(resolved) = task.resolved_opt() else {
                let diagnostic = new_compiler_diagnostic(&diagnostics::File_0_not_found, &[&task.config]);
                report_diagnostic(diagnostic);
                result.errors.get_or_insert_with(Vec::new).push(diagnostic);
                continue;
            };
            let inputs: Set<Path> = tsrs_core::collections::new_set_from_items(resolved.file_names().iter().map(|f| self.to_path(f)));
            let mut outputs: Vec<String> = resolved.get_output_file_names().into_iter().collect();
            outputs.push(resolved.get_build_info_file_name());
            for output_file in outputs {
                self.clean_project_output_for_api(&output_file, &inputs, dry, &mut files_to_delete, &report_diagnostic);
            }
        }
        if !files_to_delete.is_empty() {
            result.files_to_delete = Some(files_to_delete);
        }
        result.report_with_files_to_delete(self, dry);
        result
    }

    // orchestrator.go:452
    fn clean_project_output_for_api(&self, output_file: &str, inputs: &Set<Path>, dry: bool, files_to_delete: &mut Vec<String>, report: &DiagnosticReporter<'static>) -> bool {
        let fs = tsrs_compiler::CompilerHost::fs(self.host());
        if output_file.is_empty() || inputs.has(&self.to_path(output_file)) || !fs.file_exists(output_file) {
            return false;
        }
        files_to_delete.push(output_file.to_string());
        if dry {
            return false;
        }
        if fs.remove(output_file).is_err() {
            report(new_compiler_diagnostic(&diagnostics::Failed_to_delete_file_0, &[&output_file]));
            return false;
        }
        true
    }

    // orchestrator.go:311
    fn start_worker(&'static self, project: &str, only_references: bool) -> OrchestratorResult {
        // Content mappers are not supported by tsrs. Watch mode is not ported.
        self.generate_graph();
        let Some(mut order) = self.get_build_order_for(project) else {
            return OrchestratorResult { status: Some(ExitStatus::InvalidProject_OutputsSkipped), ..Default::default() };
        };
        if only_references && self.errors.lock().unwrap().is_empty() {
            if project.is_empty() {
                return OrchestratorResult { status: Some(ExitStatus::InvalidProject_OutputsSkipped), ..Default::default() };
            }
            order.pop();
        }
        self.build_or_clean_order(&order)
    }

    // orchestrator.go:426
    fn get_build_order_for(&self, project: &str) -> Option<Vec<String>> {
        if project.is_empty() {
            return Some(self.order.lock().unwrap().clone());
        }

        let config = tsrs_core::resolve_config_file_name_of_project_reference(&tspath::resolve_path(self.opts.sys.get_current_directory(), &[project]));
        let target = *self.tasks.lock().unwrap().get(&self.to_path(&config))?;

        let mut projects: Set<Path> = Set::default();
        fn add_project_and_references(o: &Orchestrator, projects: &mut Set<Path>, task: P<BuildTask>) {
            let path = o.to_path(&task.config);
            if projects.has(&path) {
                return;
            }
            projects.add(path);
            for upstream in task.up_stream.lock().unwrap().iter() {
                add_project_and_references(o, projects, upstream.task);
            }
        }
        add_project_and_references(self, &mut projects, target);

        let order: Vec<String> = self.order.lock().unwrap().iter().filter(|config| projects.has(&self.to_path(config))).cloned().collect();
        Some(order)
    }

    // orchestrator.go:798
    fn build_or_clean_order(&'static self, order: &[String]) -> OrchestratorResult {
        let build_options = &self.opts.command.build_options;
        if !build_options.clean.is_true() && build_options.verbose.is_true() {
            self.create_builder_status_reporter(None)(new_compiler_diagnostic(
                &diagnostics::Projects_in_this_build_Colon_0,
                &[&order.iter().map(|p| format!("\r\n    * {}", self.relative_file_name(p))).collect::<String>()],
            ));
        }
        let mut build_result = OrchestratorResult::default();
        if self.errors.lock().unwrap().is_empty() {
            build_result.statistics.projects = order.len();
            // Builders pick up projects in scheduleOrder; results are reported in Order(), waiting for each project to finish
            // (Go: a reporter goroutine; here the calling thread, while rangeTasks runs the builders on their own threads).
            let mut aborted_report = false;
            std::thread::scope(|scope| {
                let builders = scope.spawn(|| self.range_tasks(order, &|path, task| self.build_or_clean_project(task, path)));
                for config in order {
                    let path = self.to_path(config);
                    let task = self.get_task(&path);
                    if !task.built.wait(&self.aborted) {
                        aborted_report = true;
                        break;
                    }
                    task.report(self, &path, &mut build_result);
                }
                if let Err(panic) = builders.join() {
                    std::panic::resume_unwind(panic);
                }
            });
            assert!(!aborted_report, "build aborted");
        } else {
            // Circularity errors prevent any project from being built
            build_result.status = Some(ExitStatus::ProjectReferenceCycle_OutputsSkipped);
            let report_diagnostic = self.create_diagnostic_reporter(None);
            for &err in self.errors.lock().unwrap().iter() {
                report_diagnostic(err);
            }
            build_result.errors = Some(self.errors.lock().unwrap().clone());
        }
        build_result.report(self);
        build_result
    }

    // orchestrator.go:925
    fn range_tasks(&'static self, order: &[String], f: &(dyn Fn(&Path, P<BuildTask>) + Sync)) {
        let mut num_routines = 4;
        if self.opts.command.compiler_options.single_threaded.is_true() {
            num_routines = 1;
        } else if let Some(builders) = self.opts.command.build_options.builders {
            num_routines = builders as usize;
        }

        let current_task_index = std::sync::atomic::AtomicUsize::new(0);
        let get_next_task = || -> Option<(Path, P<BuildTask>)> {
            let index = current_task_index.fetch_add(1, Ordering::SeqCst);
            let config = order.get(index)?;
            let path = self.to_path(config);
            let task = self.get_task(&path);
            Some((path, task))
        };
        let run_task = || {
            while let Some((path, task)) = get_next_task() {
                f(&path, task);
            }
        };

        // Go's goroutines grow their stacks; checking and binding recurse deeply (main.rs runs on 512 MB too).
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..num_routines)
                .map(|i| {
                    std::thread::Builder::new()
                        .name(format!("builder-{i}"))
                        .stack_size(512 << 20)
                        .spawn_scoped(scope, || {
                            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_task));
                            if result.is_err() {
                                self.abort();
                            }
                            result
                        })
                        .unwrap()
                })
                .collect();
            for handle in handles {
                if let Err(panic) | Ok(Err(panic)) = handle.join() {
                    std::panic::resume_unwind(panic);
                }
            }
        });
    }

    // A builder panicked: release every waiter so the panic surfaces instead of a hang.
    fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
        for task in self.tasks.lock().unwrap().values() {
            task.done.wake();
            task.built.wake();
        }
    }

    // orchestrator.go:888
    fn build_or_clean_project(&'static self, task: P<BuildTask>, path: &Path) {
        let builder: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        *task.result.lock().unwrap() = Some(taskResult::new(builder));
        let report_status = self.create_builder_status_reporter(Some(task));
        let diagnostic_reporter = self.create_diagnostic_reporter(Some(task));
        {
            let mut result = task.result.lock().unwrap();
            let result = result.as_mut().unwrap();
            result.report_status = Some(report_status);
            result.diagnostic_reporter = Some(diagnostic_reporter);
        }
        if !self.opts.command.build_options.clean.is_true() {
            task.get().build_project(self, path);
        } else {
            task.clean_project(self, path);
        }
        if self.opts.testing.is_none() {
            // The program is only needed by Testing.OnProgram at report time; drop it now so a task
            // that has finished but is not yet reported does not keep its program alive.
            task.result.lock().unwrap().as_mut().unwrap().program = None;
        }
        task.built.close();
    }

    // orchestrator.go:906
    pub(crate) fn get_writer(&self, task: Option<P<BuildTask>>) -> Writer<'static> {
        let sys = self.opts.sys;
        match task {
            None => std::sync::Arc::new(move |t: &str| sys.write(t)),
            Some(task) => {
                let builder = {
                    let result = task.result.lock().unwrap();
                    result.as_ref().unwrap().builder.clone()
                };
                std::sync::Arc::new(move |t: &str| builder.lock().unwrap().push_str(t))
            }
        }
    }

    // orchestrator.go:913
    pub(crate) fn create_builder_status_reporter(&self, task: Option<P<BuildTask>>) -> DiagnosticReporter<'static> {
        create_builder_status_reporter(self.opts.sys, self.get_writer(task), &self.opts.command.compiler_options, self.opts.testing)
    }

    // orchestrator.go:917
    pub(crate) fn create_diagnostic_reporter(&self, task: Option<P<BuildTask>>) -> DiagnosticReporter<'static> {
        create_diagnostic_reporter_with_writer(self.opts.sys, self.get_writer(task), Some(&self.opts.command.compiler_options))
    }
}

// orchestrator.go:921
pub fn new_orchestrator(opts: Options) -> &'static Orchestrator {
    let sys = opts.sys;
    let compare_paths_options =
        ComparePathsOptions { current_directory: sys.get_current_directory().to_string(), use_case_sensitive_file_names: sys.fs().use_case_sensitive_file_names() };
    let error_summary_reporter = create_report_error_summary(sys, &opts.command.compiler_options);
    let orchestrator: &'static Orchestrator = Box::leak(Box::new(Orchestrator {
        opts,
        compare_paths_options,
        host: OnceLock::new(),
        tasks: Mutex::new(FxHashMap::default()),
        order: Mutex::new(Vec::new()),
        errors: Mutex::new(Vec::new()),
        graph_generated: AtomicBool::new(false),
        aborted: AtomicBool::new(false),
        error_summary_reporter,
        schedule_order: Mutex::new(Vec::new()),
    }));
    let compiler_host: Arc<dyn CompilerHost> = new_cached_fs_compiler_host(sys.get_current_directory(), sys.fs(), sys.default_library_path(), None, None);
    let h: &'static host = Box::leak(Box::new(host {
        orchestrator: OnceLock::new(),
        host: compiler_host,
        extended_config_cache: Mutex::new(Arc::new(tsc::ExtendedConfigCache::default())),
        source_files: parseCache::default(),
        config_times: Mutex::new(FxHashMap::default()),
        resolved_references: parseCache::default(),
        m_times: Mutex::new(Arc::new(Mutex::new(FxHashMap::default()))),
    }));
    let _ = h.orchestrator.set(orchestrator);
    let _ = orchestrator.host.set(h);
    orchestrator
}
