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

pub type DiagnosticReporter<'a> = Box<dyn Fn(P<Diagnostic>) + 'a>;

pub fn create_diagnostic_reporter<'a>(sys: &'a dyn System, options: Option<&CompilerOptions>) -> DiagnosticReporter<'a> {
    if options.is_some_and(|o| o.quiet.is_true()) {
        return Box::new(|_| {});
    }
    let format_opts = get_format_opts_of_sys(sys);
    if should_be_pretty(sys, options) {
        return Box::new(move |diagnostic| {
            let mut out = Vec::new();
            diagnosticwriter::format_diagnostic_with_color_and_context(&mut out, diagnostic, &format_opts);
            out.extend_from_slice(format_opts.new_line.as_bytes());
            sys.write(&String::from_utf8_lossy(&out));
        });
    }
    Box::new(move |diagnostic| {
        let mut out = Vec::new();
        diagnosticwriter::write_format_diagnostic(&mut out, diagnostic, &format_opts);
        sys.write(&String::from_utf8_lossy(&out));
    })
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

pub type DiagnosticsReporter<'a> = Box<dyn Fn(&[P<Diagnostic>]) + 'a>;

pub fn create_report_error_summary<'a>(sys: &'a dyn System, options: &CompilerOptions) -> DiagnosticsReporter<'a> {
    if should_be_pretty(sys, Some(options)) {
        let format_opts = get_format_opts_of_sys(sys);
        return Box::new(move |diagnostics| {
            let mut out = Vec::new();
            diagnosticwriter::write_error_summary_text(&mut out, diagnostics, &format_opts);
            sys.write(&String::from_utf8_lossy(&out));
        });
    }
    Box::new(|_| {})
}
