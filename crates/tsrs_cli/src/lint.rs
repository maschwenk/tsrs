//! Native compiler entry point for `--lint <headless-config.json>`.
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;
use tsrs_core::tspath;
use tsrs_execute::{execute, tsc};
use tsrs_linter::{Fixes, HeadlessConfig as Payload, LintConfig};
use tsrs_scanner::get_ecma_line_and_utf16_character_of_position;
use tsrs_vfs::{FS, bundled};

use crate::headless::OverlayFs;

struct LintSystem {
    base: &'static dyn tsc::System,
    fs: Arc<dyn FS>,
    lint: Arc<LintConfig>,
}

impl tsc::System for LintSystem {
    fn fs(&self) -> Arc<dyn FS> {
        Arc::clone(&self.fs)
    }
    fn default_library_path(&self) -> &str {
        self.base.default_library_path()
    }
    fn get_current_directory(&self) -> &str {
        self.base.get_current_directory()
    }
    fn write(&self, text: &str) {
        self.base.write(text);
    }
    fn flush(&self) {
        self.base.flush();
    }
    fn write_output_is_tty(&self) -> bool {
        self.base.write_output_is_tty()
    }
    fn get_environment_variable(&self, name: &str) -> Option<String> {
        self.base.get_environment_variable(name)
    }
    fn now(&self) -> Instant {
        self.base.now()
    }
    fn since_start(&self) -> Duration {
        self.base.since_start()
    }
    fn lint_config(&self) -> Option<&Arc<LintConfig>> {
        Some(&self.lint)
    }
}

fn error(sys: &dyn tsc::System, message: &str) -> tsc::CommandLineResult {
    sys.write(&format!("error: {message}\n"));
    tsc::CommandLineResult {
        status: tsc::ExitStatus::DiagnosticsPresent_OutputsSkipped,
    }
}

pub(crate) fn command_line(
    sys: &'static dyn tsc::System,
    mut args: Vec<String>,
) -> tsc::CommandLineResult {
    let Some(index) = args
        .iter()
        .position(|arg| arg.eq_ignore_ascii_case("--lint"))
    else {
        return execute::command_line(sys, args);
    };
    let Some(path) = args.get(index + 1).filter(|arg| !arg.starts_with('-')) else {
        return error(sys, "--lint expects a headless JSON config file path");
    };
    let path = tspath::get_normalized_absolute_path(path, sys.get_current_directory());
    let Some(text) = sys.fs().read_file(&path) else {
        return error(sys, &format!("cannot read lint config {path}"));
    };
    let payload: Payload = match serde_json::from_str(&text) {
        Ok(payload) => payload,
        Err(err) => return error(sys, &format!("invalid lint config: {err}")),
    };
    if payload.version != 2 {
        return error(sys, "lint config must use headless payload version 2");
    }
    args.drain(index..index + 2);
    let base_fs = sys.fs();
    let cwd = sys.get_current_directory();
    // The compiler resolves its working directory physically (e.g. macOS /var -> /private/var).
    // Keep both spellings so payload paths and overlays also match programs loaded through symlinks.
    let mut overlays = FxHashMap::default();
    let mut overrides: Vec<_> = payload
        .source_overrides
        .unwrap_or_default()
        .into_iter()
        .collect();
    overrides.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, text) in overrides {
        let path = tspath::get_normalized_absolute_path(&name, cwd);
        overlays.insert(base_fs.realpath(&path), text.clone());
        overlays.insert(path, text);
    }
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(OverlayFs::new(
        Arc::clone(&base_fs),
        overlays,
    )));
    let mut lint = match LintConfig::new(
        &payload.configs,
        cwd,
        fs.use_case_sensitive_file_names(),
        Fixes::default(),
        false,
    ) {
        Ok(lint) => lint,
        Err(message) => return error(sys, &message),
    };
    for config in &payload.configs {
        for file in &config.file_paths {
            let file = tspath::get_normalized_absolute_path(file, cwd);
            lint.add_path_alias(
                &tspath::to_path(&file, cwd, fs.use_case_sensitive_file_names()),
                tspath::to_path(
                    &base_fs.realpath(&file),
                    cwd,
                    fs.use_case_sensitive_file_names(),
                ),
            );
        }
    }
    let lint_sys: &'static LintSystem = Box::leak(Box::new(LintSystem {
        base: sys,
        fs,
        lint: Arc::new(lint),
    }));
    let mut result = execute::command_line(lint_sys, args);
    let output = lint_sys.lint.take_output();
    let diagnostics = output.diagnostics;
    for diagnostic in diagnostics.iter() {
        let (line, column) = get_ecma_line_and_utf16_character_of_position(
            diagnostic.source_file.get(),
            diagnostic.range.pos(),
        );
        sys.write(&format!(
            "{}({},{}): error {}: {}\n",
            diagnostic.source_file.file_name(),
            line + 1,
            column + 1,
            diagnostic.rule_name,
            diagnostic.message.description
        ));
    }
    if !diagnostics.is_empty() && result.status == tsc::ExitStatus::Success {
        result.status = if output.no_emit {
            tsc::ExitStatus::DiagnosticsPresent_OutputsSkipped
        } else {
            tsc::ExitStatus::DiagnosticsPresent_OutputsGenerated
        };
    }
    result
}
