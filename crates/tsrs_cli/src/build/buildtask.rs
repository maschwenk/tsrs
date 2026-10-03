// Port of execute/build/buildtask.go (the non-watch parts; content mappers are not supported by tsrs).

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tsrs_ast::{new_compiler_diagnostic, Diagnostic};
use tsrs_compiler::{new_program, ProgramOptions, WriteFileData};
use tsrs_core::collections::{new_set_from_items, Set};
use tsrs_core::tspath::{self, Path};
use tsrs_core::P;
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_incremental::{is_build_info_file_name_default_library, BuildInfo, BuildInfoRootInfoReader};
use tsrs_tsoptions::ParsedCommandLine;

use super::compilerhost::compilerHost;
use super::host::incrementalHost;
use super::orchestrator::{Orchestrator, OrchestratorResult};
use super::uptodatestatus::*;
use crate::tsc::{self, emit_and_report_statistics, CompileTimes, DiagnosticReporter, EmitInput, ExitStatus, Statistics};

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

pub struct BuildTask {
    pub(crate) config: String,
    pub(crate) resolved: Cell<Option<P<ParsedCommandLine>>>,
    pub(crate) up_stream: RefCell<Vec<upstreamTask>>,
    pub(crate) status: RefCell<Option<upToDateStatus>>,

    // task reporting
    pub(crate) result: RefCell<Option<taskResult>>,

    build_info_entry: Mutex<Option<buildInfoEntry>>,
    package_jsons: RefCell<Vec<String>>,

    errors: RefCell<Vec<P<Diagnostic>>>,
    pub(crate) pending: Cell<bool>,
    is_initial_cycle: Cell<bool>,
}

fn diag(message: &'static Message, args: &[&dyn std::fmt::Display]) -> P<Diagnostic> {
    new_compiler_diagnostic(message, args)
}

impl BuildTask {
    pub(crate) fn new(config: String, is_initial_cycle: bool) -> BuildTask {
        BuildTask {
            config,
            resolved: Cell::new(None),
            up_stream: RefCell::new(Vec::new()),
            status: RefCell::new(None),
            result: RefCell::new(None),
            build_info_entry: Mutex::new(None),
            package_jsons: RefCell::new(Vec::new()),
            errors: RefCell::new(Vec::new()),
            pending: Cell::new(false),
            is_initial_cycle: Cell::new(is_initial_cycle),
        }
    }

    fn resolved(&self) -> P<ParsedCommandLine> {
        self.resolved.get().unwrap()
    }

    fn status(&self) -> upToDateStatus {
        self.status.borrow().clone().unwrap()
    }

    fn set_status(&self, status: upToDateStatus) {
        *self.status.borrow_mut() = Some(status);
    }

    fn report_status(&self, d: P<Diagnostic>) {
        let result = self.result.borrow();
        (result.as_ref().unwrap().report_status.as_ref().unwrap())(d);
    }

    fn set_exit_status(&self, status: ExitStatus) {
        self.result.borrow_mut().as_mut().unwrap().exit_status = status;
    }

    // buildtask.go:115
    fn unblock_downstream(&self) {
        self.pending.set(false);
        self.is_initial_cycle.set(false);
    }

    // buildtask.go:121
    fn report_diagnostic(&self, err: P<Diagnostic>) {
        self.errors.borrow_mut().push(err);
        let result = self.result.borrow();
        (result.as_ref().unwrap().diagnostic_reporter.as_ref().unwrap())(err);
    }

    // buildtask.go:126
    pub(crate) fn report(&self, orchestrator: &Orchestrator, _config_path: &Path, build_result: &mut OrchestratorResult) {
        let errors = self.errors.borrow();
        if !errors.is_empty() {
            build_result.errors.get_or_insert_with(Vec::new).extend(errors.iter().copied());
        }
        let result = self.result.borrow_mut().take().unwrap();
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
        // Upstream tasks are complete (tasks run in build order).
        if self.pending.get() {
            self.set_status(self.get_up_to_date_status(orchestrator, path));
            self.report_up_to_date_status(orchestrator);
            if !self.handle_status_that_doesnt_require_build(orchestrator) {
                self.compile_and_emit(orchestrator, path);
                self.update_downstream(orchestrator, path);
            } else {
                if let Some(resolved) = self.resolved.get() {
                    for diagnostic in resolved.get_config_file_parsing_diagnostics() {
                        self.report_diagnostic(diagnostic);
                    }
                }
                if !self.errors.borrow().is_empty() {
                    self.set_exit_status(ExitStatus::DiagnosticsPresent_OutputsSkipped);
                }
            }
        } else if !self.errors.borrow().is_empty() {
            self.report_up_to_date_status(orchestrator);
            let errors = self.errors.borrow().clone();
            let result = self.result.borrow();
            for err in errors {
                // Should not add the diagnostics so just reporting
                (result.as_ref().unwrap().diagnostic_reporter.as_ref().unwrap())(err);
            }
        }
        self.unblock_downstream();
    }

    // buildtask.go:184
    fn update_downstream(&self, orchestrator: &Orchestrator, _path: &Path) {
        if self.is_initial_cycle.get() {
            return;
        }
        if orchestrator.opts.command.build_options.stop_build_on_errors.is_true() && self.status().is_error() {
            return;
        }
        // downStream is only set in watch mode, which is not ported.
    }

    // buildtask.go:225
    fn compile_and_emit(&'static self, orchestrator: &'static Orchestrator, path: &Path) {
        self.errors.borrow_mut().clear();
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
        let builder = self.result.borrow().as_ref().unwrap().builder.clone();
        let trace_builder = builder.clone();
        let compiler_host: Arc<dyn tsrs_compiler::CompilerHost> = Arc::new(compilerHost {
            host,
            trace: Box::new(move |msg: &'static Message, args: &[&dyn std::fmt::Display]| {
                let mut b = trace_builder.lock().unwrap();
                b.push_str(&msg.localize(args));
                b.push('\n');
            }),
        });
        let mut old_program = None;
        if !build_options.force.is_true() {
            old_program = tsrs_incremental::read_build_info_program(self.resolved(), host, &*compiler_host);
        }
        compile_times.build_info_read_time = sys.now() - build_info_read_start;
        let parse_start = sys.now();
        let program = new_program(ProgramOptions::new(self.resolved(), compiler_host));
        compile_times.parse_time = sys.now() - parse_start;
        let changes_compute_start = sys.now();
        let incremental_program = tsrs_incremental::new_program(
            program,
            old_program,
            Box::new(incrementalHost(host)),
            Some(std::time::Instant::now),
            orchestrator.opts.testing.is_some(),
        );
        self.result.borrow_mut().as_mut().unwrap().program = Some(incremental_program);
        compile_times.changes_compute_time = sys.now() - changes_compute_start;

        let this: &'static BuildTask = self;
        let report_diagnostic: tsc::DiagnosticReporter = Box::new(move |d| this.report_diagnostic(d));
        let report_error_summary = tsc::quiet_diagnostics_reporter();
        let writer_builder = builder.clone();
        let writer = move |t: &str| writer_builder.lock().unwrap().push_str(t);
        // Called from the checker threads: capture arena handles (P is Send + Sync).
        let (task_p, orchestrator_p) = (P::from_static(this), P::from_static(orchestrator));
        let write_file = move |file_name: &str, text: &str, data: &mut WriteFileData| task_p.write_file(&orchestrator_p, file_name, text, data);
        let (result, statistics) = emit_and_report_statistics(EmitInput {
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
        });
        {
            let mut r = self.result.borrow_mut();
            let r = r.as_mut().unwrap();
            r.exit_status = result.status;
            r.statistics = statistics;
        }
        *self.package_jsons.borrow_mut() = incremental_program.package_json_lookup_paths();
        let emitted_files = result.emitted_files.clone();
        if (!program.options().no_emit_on_error.is_true() || result.diagnostics.is_empty())
            && (!emitted_files.is_empty() || self.status().kind != upToDateStatusType::OutOfDateBuildInfoWithErrors)
        {
            // Update time stamps for rest of the outputs
            self.update_time_stamps(orchestrator, &emitted_files, &diagnostics::Updating_unchanged_output_timestamps_of_project_0);
        }
        self.result.borrow_mut().as_mut().unwrap().build_kind = buildKind::Program;
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
            self.set_status(upToDateStatus::with(upToDateStatusType::UpToDate, status.data.clone()));
            self.result.borrow_mut().as_mut().unwrap().build_kind = buildKind::Pseudo;
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
        if let Some(status) = self.status.borrow().clone() {
            return status;
        }
        // Config file not found
        let Some(resolved) = self.resolved.get() else {
            return upToDateStatus::new(upToDateStatusType::ConfigFileNotFound);
        };

        // Solution - nothing to build
        // Go tests `ProjectReferences() != nil`; tsrs does not keep an absent "references" apart from an empty one.
        if resolved.file_names().is_empty() && !resolved.project_references().is_empty() {
            return upToDateStatus::new(upToDateStatusType::Solution);
        }

        let build_options = &orchestrator.opts.command.build_options;
        for upstream in self.up_stream.borrow().iter() {
            if build_options.stop_build_on_errors.is_true() && upstream.task.status().is_error() {
                // Upstream project has errors, so we cannot build this project
                return upToDateStatus::with(
                    upToDateStatusType::UpstreamErrors,
                    statusData::UpstreamErrors(upstreamErrors {
                        ref_: resolved.project_references()[upstream.ref_index].path.clone(),
                        ref_has_upstream_errors: upstream.task.status().kind == upToDateStatusType::UpstreamErrors,
                    }),
                );
            }
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
        match tsrs_incremental::content_mapper_identities() {
            Ok(identities) if build_info.content_mapper_identities_match(identities.as_deref()) => {}
            _ => return upToDateStatus::with(upToDateStatusType::OutOfDateOptions, statusData::String(build_info_path)),
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
                    if let Some(file_info) = build_info_file_info.map(|i| i.get_file_info()) {
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

        for root in get_build_info_root_info_reader().roots() {
            if !seen_roots.has(root) {
                // File was root file when project was built but its not any more
                return upToDateStatus::with(
                    upToDateStatusType::OutOfDateRoots,
                    statusData::InputOutputName(inputOutputName { input: root.to_string(), output: build_info_path }),
                );
            }
        }

        if build_info.is_incremental() {
            let mut resolved_roots: Set<Path> = Set::default();
            for root in get_build_info_root_info_reader().roots() {
                let (_, resolved) = get_build_info_root_info_reader().get_build_info_file_info(root);
                if !resolved.is_empty() {
                    resolved_roots.add(resolved);
                }
            }
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
                // Content-mapper supplemental files: content mappers are not supported by tsrs.
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
                        statusData::InputOutputName(inputOutputName { input: newest_input_file_and_time.file.clone(), output: output_file }),
                    );
                }

                if before(output_time, oldest_output_file_and_time.time) {
                    oldest_output_file_and_time = fileAndTime { file: output_file, time: output_time };
                }
            }
        }

        let mut ref_dts_unchanged = false;
        for upstream in self.up_stream.borrow().iter() {
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
                        output: oldest_output_file_and_time.file.clone(),
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
                    output: oldest_output_file_and_time.file.clone(),
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

        for extended_config in resolved.extended_source_files() {
            if let Some(extended_config_status) = check_input_file_time(&extended_config) {
                return extended_config_status;
            }
        }

        let package_jsons = build_info.get_package_jsons(&get_build_info_directory());
        for package_json in &package_jsons {
            let package_json_time = host.get_m_time(package_json);
            if package_json_time.is_none() {
                return upToDateStatus::with(upToDateStatusType::InputFileMissing, statusData::String(package_json.clone()));
            }
            if after(package_json_time, oldest_output_file_and_time.time) {
                return upToDateStatus::with(
                    upToDateStatusType::InputFileNewer,
                    statusData::InputOutputName(inputOutputName { input: package_json.clone(), output: oldest_output_file_and_time.file.clone() }),
                );
            }
        }
        let missing_package_jsons = build_info.get_missing_package_jsons(&get_build_info_directory());
        for package_json in &missing_package_jsons {
            if host.get_m_time(package_json).is_some() {
                return upToDateStatus::with(
                    upToDateStatusType::InputFileNewer,
                    statusData::InputOutputName(inputOutputName { input: package_json.clone(), output: oldest_output_file_and_time.file.clone() }),
                );
            }
        }
        let mut all = package_jsons;
        all.extend(missing_package_jsons);
        *self.package_jsons.borrow_mut() = all;

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
                build_info: build_info_path,
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
                self.report_status(diag(&diagnostics::Project_0_is_out_of_date_because_config_file_does_not_exist, &[&config]))
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
                ))
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
                self.report_status(diag(&diagnostics::Project_0_is_up_to_date_with_d_ts_files_from_its_dependencies, &[&config]))
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
                ))
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
                ))
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
        let emitted: Set<&str> = new_set_from_items(emitted_files.iter().map(|s| s.as_str()));
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
        let Some(resolved) = self.resolved.get() else {
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
                self.result.borrow_mut().as_mut().unwrap().files_to_delete.push(output_file.to_string());
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
        let build_info = tsrs_incremental::new_build_info_reader(orchestrator.host().host.clone()).read_build_info(&self.resolved()).map(Arc::new);
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
            if let Some(build_info) = data.build_info.clone() {
                let build_info = build_info.downcast::<BuildInfo>().unwrap();
                let has_changed_dts_file = self.result.borrow().as_ref().unwrap().program.unwrap().has_changed_dts_file();
                self.on_build_info_emit(orchestrator, file_name, build_info, has_changed_dts_file);
            } else if self.store_output_time_stamp(orchestrator) {
                // Store time stamps
                host.store_m_time(file_name, orchestrator.opts.sys.now_time());
            }
        }
        err
    }
}
