use tsrs_ast::Diagnostic;
use tsrs_compiler::diagnosticwriter::{self, FormattingOptions};
use tsrs_core::tspath::ComparePathsOptions;
use tsrs_core::{CompilerOptions, P};

use super::System;

fn get_format_opts_of_sys(sys: &dyn System) -> FormattingOptions {
    FormattingOptions {
        new_line: "\n".to_string(),
        compare_paths_options: ComparePathsOptions {
            current_directory: sys.get_current_directory().to_string(),
            use_case_sensitive_file_names: sys.fs().use_case_sensitive_file_names(),
        },
    }
}

// Send + Sync: build mode reports from its builder threads (orchestrator.go rangeTasks).
pub type DiagnosticReporter<'a> = Box<dyn Fn(P<Diagnostic>) + Send + Sync + 'a>;

// Go `io.Writer` for the reporters: `sys.Writer()` or a build task's output buffer.
pub type Writer<'a> = std::sync::Arc<dyn Fn(&str) + Send + Sync + 'a>;

pub fn create_diagnostic_reporter<'a>(sys: &'a dyn System, options: Option<&CompilerOptions>) -> DiagnosticReporter<'a> {
    create_diagnostic_reporter_with_writer(sys, std::sync::Arc::new(move |t: &str| sys.write(t)), options)
}

// diagnostics.go:27 CreateDiagnosticReporter(sys, w, locale, options)
pub fn create_diagnostic_reporter_with_writer<'a>(sys: &'a dyn System, w: Writer<'a>, options: Option<&CompilerOptions>) -> DiagnosticReporter<'a> {
    if options.is_some_and(|o| o.quiet.is_true()) {
        return Box::new(|_| {});
    }
    if let Some(sink) = sys.diagnostic_sink() {
        return Box::new(sink);
    }
    let format_opts = get_format_opts_of_sys(sys);
    if should_be_pretty(sys, options) {
        return Box::new(move |diagnostic| {
            let mut out = Vec::new();
            diagnosticwriter::format_diagnostic_with_color_and_context(&mut out, diagnostic, &format_opts);
            out.extend_from_slice(format_opts.new_line.as_bytes());
            w(&tsrs_core::utf8::from_utf8_lossy(&out));
        });
    }
    Box::new(move |diagnostic| {
        let mut out = Vec::new();
        diagnosticwriter::write_format_diagnostic(&mut out, diagnostic, &format_opts);
        w(&tsrs_core::utf8::from_utf8_lossy(&out));
    })
}

// diagnostics.go:155
pub fn create_builder_status_reporter<'a>(
    sys: &'a dyn System,
    w: Writer<'a>,
    options: &CompilerOptions,
    testing: Option<&'a dyn super::CommandLineTesting>,
) -> DiagnosticReporter<'a> {
    if options.quiet.is_true() {
        return Box::new(|_| {});
    }

    let format_opts = get_format_opts_of_sys(sys);
    let pretty = should_be_pretty(sys, Some(options));
    Box::new(move |diagnostic| {
        if let Some(testing) = testing {
            testing.on_build_status_report_start(&*w);
        }
        let mut out = Vec::new();
        let time = sys.format_time_now();
        if pretty {
            diagnosticwriter::format_diagnostics_status_with_color_and_time(&mut out, &time, diagnostic, &format_opts);
        } else {
            diagnosticwriter::format_diagnostics_status_and_time(&mut out, &time, diagnostic, &format_opts);
        }
        out.extend_from_slice(format_opts.new_line.as_bytes());
        out.extend_from_slice(format_opts.new_line.as_bytes());
        w(&tsrs_core::utf8::from_utf8_lossy(&out));
        if let Some(testing) = testing {
            testing.on_build_status_report_end(&*w);
        }
    })
}

// diagnostics.go:149
pub fn quiet_diagnostics_reporter<'a>() -> DiagnosticsReporter<'a> {
    Box::new(|_| {})
}

fn default_is_pretty(sys: &dyn System) -> bool {
    if let Some(force_color) = sys.get_environment_variable("FORCE_COLOR") {
        return matches!(force_color.as_str(), "" | "1" | "2" | "3" | "true");
    }
    if sys.get_environment_variable("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if sys.get_environment_variable("TERM").as_deref() == Some("dumb") {
        return false;
    }
    sys.write_output_is_tty()
}

fn should_be_pretty(sys: &dyn System, options: Option<&CompilerOptions>) -> bool {
    match options {
        Some(options) if !options.pretty.is_unknown() => options.pretty.is_true(),
        _ => default_is_pretty(sys),
    }
}

pub type DiagnosticsReporter<'a> = Box<dyn Fn(&[P<Diagnostic>]) + Send + Sync + 'a>;

pub fn create_report_error_summary<'a>(sys: &'a dyn System, options: &CompilerOptions) -> DiagnosticsReporter<'a> {
    if sys.diagnostic_sink().is_none() && should_be_pretty(sys, Some(options)) {
        let format_opts = get_format_opts_of_sys(sys);
        return Box::new(move |diagnostics| {
            let mut out = Vec::new();
            diagnosticwriter::write_error_summary_text(&mut out, diagnostics, &format_opts);
            sys.write(&tsrs_core::utf8::from_utf8_lossy(&out));
        });
    }
    Box::new(|_| {})
}
