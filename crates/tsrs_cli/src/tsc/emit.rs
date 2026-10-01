use std::cell::Cell;

use tsrs_ast::Diagnostic;
use tsrs_compiler::{get_diagnostics_of_any_program, sort_and_deduplicate_diagnostics, Program};
use tsrs_core::P;
use tsrs_tsoptions::ParsedCommandLine;

use super::{statistics_from_program, CompileAndEmitResult, CompileTimes, DiagnosticReporter, DiagnosticsReporter, ExitStatus, Statistics, System};

pub struct EmitInput<'a> {
    pub sys: &'a dyn System,
    pub program: &'static Program,
    pub config: P<ParsedCommandLine>,
    pub report_diagnostic: &'a DiagnosticReporter<'a>,
    pub report_error_summary: &'a DiagnosticsReporter<'a>,
    pub compile_times: CompileTimes,
}

pub fn emit_and_report_statistics(input: EmitInput) -> (CompileAndEmitResult, Option<Statistics>) {
    let mut statistics = None;
    let mut result = emit_files_and_report_errors(&input);
    if result.status != ExitStatus::Success {
        // compile exited early
        return (result, None);
    }
    result.times.total_time = input.sys.since_start();

    let options = input.config.compiler_options().unwrap();
    if options.diagnostics.is_true() || options.extended_diagnostics.is_true() {
        let stats = statistics_from_program(&input, &result.times);
        stats.report(input.sys);
        if tsrs_compiler::assignment_stats_enabled() {
            input.sys.write(&input.program.checker_assignment_report());
        }
        statistics = Some(stats);
    }

    if result.emit_skipped && !result.diagnostics.is_empty() {
        result.status = ExitStatus::DiagnosticsPresent_OutputsSkipped;
    } else if !result.diagnostics.is_empty() {
        result.status = ExitStatus::DiagnosticsPresent_OutputsGenerated;
    }
    (result, statistics)
}

pub fn emit_files_and_report_errors(input: &EmitInput) -> CompileAndEmitResult {
    let mut times = input.compile_times;
    let bind_time = Cell::new(times.bind_time);
    let check_time = Cell::new(times.check_time);
    let program = input.program;

    let mut all_diagnostics = get_diagnostics_of_any_program(
        program,
        None,
        false,
        &mut |file| {
            // Options diagnostics include global diagnostics (even though we collect them separately),
            // and global diagnostics create checkers, which then bind all of the files. Do this binding
            // early so we can track the time.
            let bind_start = input.sys.now();
            let diags = program.get_bind_diagnostics(file);
            bind_time.set(input.sys.now() - bind_start);
            diags
        },
        &mut |file| {
            let check_start = input.sys.now();
            let diags = program.get_semantic_diagnostics(file);
            check_time.set(input.sys.now() - check_start);
            diags
        },
    );
    times.bind_time = bind_time.get();
    times.check_time = check_time.get();

    // Emit is not supported. Under --listFilesOnly Go skips emit (EmitSkipped); otherwise the
    // program always has noEmit set, and HandleNoEmitOptions returns an empty, non-skipped result.
    let emit_skipped = program.options().list_files_only.is_true() || !program.options().no_emit.is_true();
    let emit_diagnostics: Vec<P<Diagnostic>> = Vec::new();
    all_diagnostics.extend(emit_diagnostics);

    let all_diagnostics = sort_and_deduplicate_diagnostics(&all_diagnostics);
    for &diagnostic in &all_diagnostics {
        (input.report_diagnostic)(diagnostic);
    }

    list_files(input);

    (input.report_error_summary)(&all_diagnostics);
    CompileAndEmitResult { diagnostics: all_diagnostics, emit_skipped, status: ExitStatus::Success, times }
}

fn list_files(input: &EmitInput) {
    let options = input.program.options();
    if options.explain_files.is_true() {
        let mut out = Vec::new();
        input.program.explain_files(&mut out);
        input.sys.write(&String::from_utf8_lossy(&out));
    } else if options.list_files.is_true() || options.list_files_only.is_true() {
        let mut out = String::new();
        for file in input.program.get_source_files() {
            out.push_str(file.file_name());
            out.push('\n');
        }
        input.sys.write(&out);
    }
}
