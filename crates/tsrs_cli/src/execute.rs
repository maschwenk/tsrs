// Port of the type-check-only subset of Go's execute/tsc.go.

use std::sync::Arc;

use tsrs_ast::new_compiler_diagnostic;
use tsrs_compiler::{new_cached_fs_compiler_host, new_program, ProgramOptions};
use tsrs_core::tspath;
use tsrs_core::P;
use tsrs_diagnostics as diagnostics;
use tsrs_tsoptions::{self as tsoptions, CompilerOptionsValue, ParseConfigHost, ParsedCommandLine};
use tsrs_vfs::FS;

use crate::tsc::{
    self, create_diagnostic_reporter, create_report_error_summary, emit_and_report_statistics, CommandLineResult, CompileTimes, EmitInput,
    ExitStatus, ExtendedConfigCache, System,
};

// The system doubles as the config-parsing host (Go's tsc.System satisfies tsoptions.ParseConfigHost).
struct sysParseConfigHost {
    sys: &'static dyn System,
    fs: Arc<dyn FS>,
}

impl ParseConfigHost for sysParseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }

    fn get_current_directory(&self) -> &str {
        self.sys.get_current_directory()
    }
}

/// docs/EMIT.md section 6: emit runs only when the environment variable `TSRS_EMIT` is exactly `1`. Without it tsrs
/// keeps behaving like `tsc --noEmit` and never writes a file. Read once; the library crates never read it.
pub(crate) fn emit_enabled() -> bool {
    static EMIT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *EMIT.get_or_init(|| std::env::var("TSRS_EMIT").as_deref() == Ok("1"))
}

fn not_supported(sys: &dyn System, what: &str) -> CommandLineResult {
    sys.write(&format!("error: {} is not supported by tsrs (type checking only).\n", what));
    CommandLineResult { status: ExitStatus::NotImplemented }
}

pub fn command_line(sys: &'static dyn System, command_line_args: Vec<String>) -> CommandLineResult {
    if let Some(first) = command_line_args.first() {
        match first.to_lowercase().as_str() {
            "-b" | "--b" | "-build" | "--build" => return not_supported(sys, "build mode (--build)"),
            _ => {}
        }
    }

    let mut args = command_line_args;
    // tsrs-only: `--noLazyMembers` turns off the port of the lazy member resolution PRs (tsrs_core::lazymembers).
    for (flag, on) in [("--lazyMembers", true), ("--noLazyMembers", false)] {
        if args.iter().any(|a| a.eq_ignore_ascii_case(flag)) {
            tsrs_core::lazymembers::set_from_cli(on);
            args.retain(|a| !a.eq_ignore_ascii_case(flag));
        }
    }
    // tsrs-only: `--checkerAssignment <locality|go>` picks how files are assigned to checkers
    // (tsrs_compiler checkerpool.rs; `go` is Go's FENNEL assignment).
    if let Some(pos) = args.iter().position(|a| a.eq_ignore_ascii_case("--checkerAssignment")) {
        let name = args.get(pos + 1).cloned().unwrap_or_default();
        if !tsrs_compiler::set_checker_assignment_from_cli(&name) {
            sys.write(&format!("error: unknown --checkerAssignment {name:?} (expected locality or go).\n"));
            return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsSkipped };
        }
        args.drain(pos..(pos + 2).min(args.len()));
    }
    // tsrs-only, opt-in: `--checkerCostCache <file>` balances the checkers on per-file check times measured by
    // the previous run that used the same file, and writes this run's times to it (checkerpool.rs).
    if let Some(pos) = args.iter().position(|a| a.eq_ignore_ascii_case("--checkerCostCache")) {
        let Some(path) = args.get(pos + 1).cloned().filter(|p| !p.starts_with('-')) else {
            sys.write("error: --checkerCostCache expects a file path.\n");
            return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsSkipped };
        };
        tsrs_compiler::set_checker_cost_cache_from_cli(&path);
        args.drain(pos..pos + 2);
    }
    // Without TSRS_EMIT=1 tsrs always behaves like `tsc --noEmit`; with it, the project's own noEmit /
    // emitDeclarationOnly / noEmitOnError decide, as in tsc.
    if !emit_enabled() && !args.iter().any(|a| a.eq_ignore_ascii_case("--noEmit") || a.eq_ignore_ascii_case("-noEmit")) {
        args.push("--noEmit".to_string());
    }

    let host: &'static sysParseConfigHost = Box::leak(Box::new(sysParseConfigHost { sys, fs: sys.fs() }));
    tsc_compilation(sys, host, P::new(tsoptions::parse_command_line(&args, host)))
}

fn tsc_compilation(sys: &'static dyn System, host: &'static sysParseConfigHost, command_line: P<ParsedCommandLine>) -> CommandLineResult {
    let mut config_file_name = String::new();
    let command_line_options = command_line.compiler_options().unwrap();
    let report_diagnostic = create_diagnostic_reporter(sys, Some(&command_line_options));

    if !command_line.errors.is_empty() {
        for &e in &command_line.errors {
            report_diagnostic(e);
        }
        return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsSkipped };
    }

    if command_line_options.init.is_true() {
        return not_supported(sys, "--init");
    }

    if command_line_options.version.is_true() {
        print_version(sys);
        return CommandLineResult { status: ExitStatus::Success };
    }

    if command_line_options.help.is_true() || command_line_options.all.is_true() {
        print_version(sys);
        sys.write("Usage: tsrs [-p <project>] [options] [files...]\n  Type-checks like `tsc --noEmit`. See `tsc --help` for options.\n");
        return CommandLineResult { status: ExitStatus::Success };
    }

    if command_line_options.watch.is_true() && command_line_options.list_files_only.is_true() {
        report_diagnostic(new_compiler_diagnostic(&diagnostics::Options_0_and_1_cannot_be_combined, &[&"watch", &"listFilesOnly"]));
        return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsSkipped };
    }

    let fs = sys.fs();
    if !command_line_options.project.is_empty() {
        if !command_line.file_names().is_empty() {
            report_diagnostic(new_compiler_diagnostic(&diagnostics::Option_project_cannot_be_mixed_with_source_files_on_a_command_line, &[]));
            return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsSkipped };
        }

        let file_or_directory = tspath::normalize_path(&command_line_options.project);
        if fs.directory_exists(&file_or_directory) {
            config_file_name = tspath::combine_paths(&file_or_directory, &["tsconfig.json"]);
            if !fs.file_exists(&config_file_name) {
                report_diagnostic(new_compiler_diagnostic(
                    &diagnostics::Cannot_find_a_tsconfig_json_file_at_the_current_directory_Colon_0,
                    &[&config_file_name],
                ));
                return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsSkipped };
            }
        } else {
            config_file_name = file_or_directory.clone();
            if !fs.file_exists(&config_file_name) {
                report_diagnostic(new_compiler_diagnostic(&diagnostics::The_specified_path_does_not_exist_Colon_0, &[&file_or_directory]));
                return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsSkipped };
            }
        }
    } else if !command_line_options.ignore_config.is_true() || command_line.file_names().is_empty() {
        let search_path = tspath::normalize_path(sys.get_current_directory());
        config_file_name = find_config_file(&search_path, |f| fs.file_exists(f), "tsconfig.json");
        if !command_line.file_names().is_empty() {
            if !config_file_name.is_empty() {
                // Error to not specify config file
                report_diagnostic(new_compiler_diagnostic(
                    &diagnostics::X_tsconfig_json_is_present_but_will_not_be_loaded_if_files_are_specified_on_commandline_Use_ignoreConfig_to_skip_this_error,
                    &[],
                ));
                return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsSkipped };
            }
        } else if config_file_name.is_empty() {
            if command_line_options.show_config.is_true() {
                report_diagnostic(new_compiler_diagnostic(
                    &diagnostics::Cannot_find_a_tsconfig_json_file_at_the_current_directory_Colon_0,
                    &[&tspath::normalize_path(sys.get_current_directory())],
                ));
            } else {
                print_version(sys);
                sys.write("Usage: tsrs [-p <project>] [options] [files...]\n");
            }
            return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsSkipped };
        }
    }

    // !!! convert to options with absolute paths is usually done here, but for ease of implementation, it's done in `tsoptions.ParseCommandLine()`
    let compiler_options_from_command_line = command_line_options;
    let mut config_for_compilation = command_line;
    let extended_config_cache: Arc<ExtendedConfigCache> = Arc::new(ExtendedConfigCache::default());
    let mut compile_times = CompileTimes::default();
    let mut report_diagnostic = report_diagnostic;
    if !config_file_name.is_empty() {
        let config_start = sys.now();
        // Wrap command line options in a "compilerOptions" key to match tsconfig.json structure
        let command_line_raw = match &command_line.raw {
            CompilerOptionsValue::Object(_) => {
                let mut wrapped = tsrs_core::collections::OrderedMap::default();
                wrapped.insert("compilerOptions".to_string(), command_line.raw.clone());
                Some(CompilerOptionsValue::Object(wrapped))
            }
            _ => None,
        };
        let (config_parse_result, errors) = tsoptions::get_parsed_command_line_of_config_file(
            &config_file_name,
            Some(&compiler_options_from_command_line),
            command_line_raw.as_ref(),
            host,
            Some(&*extended_config_cache),
        );
        compile_times.config_time = sys.now() - config_start;
        if !errors.is_empty() {
            // these are unrecoverable errors--exit to report them as diagnostics
            for e in errors {
                report_diagnostic(e);
            }
            return CommandLineResult { status: ExitStatus::DiagnosticsPresent_OutputsGenerated };
        }
        config_for_compilation = P::new(config_parse_result.unwrap());
        // Updater to reflect pretty
        report_diagnostic = create_diagnostic_reporter(sys, Some(&command_line.compiler_options().unwrap()));
    }

    let config_options = config_for_compilation.compiler_options().unwrap();
    let report_error_summary = create_report_error_summary(sys, &config_options);
    if compiler_options_from_command_line.show_config.is_true() {
        return not_supported(sys, "--showConfig");
    }
    if config_options.watch.is_true() {
        return not_supported(sys, "watch mode (--watch)");
    }
    // Without TSRS_EMIT=1, incremental compilation (tsbuildinfo) is off: incremental projects are checked from
    // scratch and nothing is written.
    if emit_enabled() && config_for_compilation.compiler_options().unwrap().is_incremental() {
        return perform_incremental_compilation(sys, config_for_compilation, &report_diagnostic, &report_error_summary, extended_config_cache, compile_times);
    }
    perform_compilation(sys, config_for_compilation, &report_diagnostic, &report_error_summary, extended_config_cache, compile_times)
}

fn find_config_file(search_path: &str, file_exists: impl Fn(&str) -> bool, config_name: &str) -> String {
    tspath::for_each_ancestor_directory(search_path, |ancestor| {
        let full_config_name = tspath::combine_paths(ancestor, &[config_name]);
        if file_exists(&full_config_name) {
            Some(full_config_name)
        } else {
            None
        }
    })
    .unwrap_or_default()
}

// tsc's "Version X" first (scripts grep for it), then the tsrs release and the TypeScript commit it ports.
fn print_version(sys: &dyn System) {
    sys.write(&format!(
        "{} (tsrs {}, microsoft/TypeScript@{})\n",
        diagnostics::Version_0.localize(&[&tsrs_core::version()]),
        env!("TSRS_RELEASE_VERSION"),
        env!("TSRS_TYPESCRIPT_COMMIT"),
    ));
}

fn perform_compilation(
    sys: &'static dyn System,
    config: P<ParsedCommandLine>,
    report_diagnostic: &tsc::DiagnosticReporter,
    report_error_summary: &tsc::DiagnosticsReporter,
    extended_config_cache: Arc<ExtendedConfigCache>,
    mut compile_times: CompileTimes,
) -> CommandLineResult {
    let host = new_cached_fs_compiler_host(
        sys.get_current_directory(),
        sys.fs(),
        sys.default_library_path(),
        Some(extended_config_cache),
        Some(Box::new(move |msg: &'static diagnostics::Message, args: &[&dyn std::fmt::Display]| {
            sys.write(&format!("{}\n", msg.localize(args)));
        })),
    );

    let parse_start = sys.now();
    let program = new_program(ProgramOptions::new(config, host));
    compile_times.parse_time = sys.now() - parse_start;
    let (result, _) = emit_and_report_statistics(EmitInput {
        sys,
        program,
        config,
        report_diagnostic,
        report_error_summary,
        compile_times,
        incremental: None,
    });
    #[cfg(feature = "alloc-profile")]
    crate::census::run(program, &[config.addr(), result.diagnostics.as_ptr() as usize]);

    CommandLineResult { status: result.status }
}

// tsc.go:308
fn perform_incremental_compilation(
    sys: &'static dyn System,
    config: P<ParsedCommandLine>,
    report_diagnostic: &tsc::DiagnosticReporter,
    report_error_summary: &tsc::DiagnosticsReporter,
    extended_config_cache: Arc<ExtendedConfigCache>,
    mut compile_times: CompileTimes,
) -> CommandLineResult {
    let host = new_cached_fs_compiler_host(
        sys.get_current_directory(),
        sys.fs(),
        sys.default_library_path(),
        Some(extended_config_cache),
        Some(Box::new(move |msg: &'static diagnostics::Message, args: &[&dyn std::fmt::Display]| {
            sys.write(&format!("{}\n", msg.localize(args)));
        })),
    );
    let build_info_read_start = sys.now();
    let old_program = tsrs_incremental::read_build_info_program(config, &*tsrs_incremental::new_build_info_reader(host.clone()), &*host);
    compile_times.build_info_read_time = sys.now() - build_info_read_start;

    let parse_start = sys.now();
    let program = new_program(ProgramOptions::new(config, host.clone()));
    compile_times.parse_time = sys.now() - parse_start;
    let changes_compute_start = sys.now();
    let incremental_program =
        tsrs_incremental::new_program(program, old_program, tsrs_incremental::create_host(host), Some(std::time::Instant::now), false);
    compile_times.changes_compute_time = sys.now() - changes_compute_start;
    let (result, _) = emit_and_report_statistics(EmitInput {
        sys,
        program: incremental_program.get_program(),
        config,
        report_diagnostic,
        report_error_summary,
        compile_times,
        incremental: Some(incremental_program),
    });

    CommandLineResult { status: result.status }
}
