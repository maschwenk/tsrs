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
        let stats = tsrs_core::phases::time("Statistics", || statistics_from_program(&input, &result.times));
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

    let ctx = tsrs_compiler::Context::default();
    let mut all_diagnostics = get_diagnostics_of_any_program(
        &ctx,
        program,
        None,
        false,
        &mut |ctx, file| {
            // Options diagnostics include global diagnostics (even though we collect them separately),
            // and global diagnostics create checkers, which then bind all of the files. Do this binding
            // early so we can track the time.
            let bind_start = input.sys.now();
            let diags = program.get_bind_diagnostics(ctx, file);
            bind_time.set(input.sys.now() - bind_start);
            diags
        },
        &mut |ctx, file| {
            let check_start = input.sys.now();
            let diags = program.get_semantic_diagnostics(ctx, file);
            check_time.set(input.sys.now() - check_start);
            diags
        },
    );
    times.bind_time = bind_time.get();
    times.check_time = check_time.get();

    // Without TSRS_EMIT=1 (docs/EMIT.md section 6) emit is never called: under --listFilesOnly Go skips emit
    // (EmitSkipped); otherwise the program always has noEmit set, and HandleNoEmitOptions returns an empty,
    // non-skipped result.
    let mut emit_result = tsrs_compiler::EmitResult { emit_skipped: true, ..Default::default() };
    if crate::execute::emit_enabled() {
        // emit.go:115
        if !program.options().list_files_only.is_true() {
            let emit_start = input.sys.now();
            emit_result = tsrs_core::phases::time("Emit", || program.emit(&ctx, tsrs_compiler::EmitOptions::default()));
            times.emit_time += input.sys.now() - emit_start;
        }
    } else {
        emit_result.emit_skipped = program.options().list_files_only.is_true() || !program.options().no_emit.is_true();
    }
    let emit_skipped = emit_result.emit_skipped;
    all_diagnostics.extend(emit_result.diagnostics.iter().copied());

    let all_diagnostics = tsrs_core::phases::time("Diagnostics: sort", || sort_and_deduplicate_diagnostics(&all_diagnostics));
    tsrs_core::phases::time("Diagnostics: report", || {
        for &diagnostic in &all_diagnostics {
            (input.report_diagnostic)(diagnostic);
        }
    });

    tsrs_core::phases::time("List files", || list_files(input, &emit_result));

    tsrs_core::phases::time("Error summary", || (input.report_error_summary)(&all_diagnostics));
    CompileAndEmitResult { diagnostics: all_diagnostics, emit_skipped, status: ExitStatus::Success, times }
}

// emit.go:142
fn list_files(input: &EmitInput, emit_result: &tsrs_compiler::EmitResult) {
    let options = input.program.options();
    if options.list_emitted_files.is_true() {
        let mut out = String::new();
        for file in &emit_result.emitted_files {
            out.push_str("TSFILE: ");
            out.push_str(&tsrs_core::tspath::get_normalized_absolute_path(file, input.program.get_current_directory()));
            out.push('\n');
        }
        input.sys.write(&out);
    }
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
