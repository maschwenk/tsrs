//! Native compiler entry point for `--lint <headless-config.json>`.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;
use tsrs_ast::SourceFile;
use tsrs_compiler::{CompilerHost, Program};
use tsrs_core::P;
use tsrs_core::tspath;
use tsrs_execute::{execute, tsc};
use tsrs_linter::{ConfiguredRule, Fixes, LintSession, RuleDiagnostic, rule_by_name};
use tsrs_scanner::get_ecma_line_and_utf16_character_of_position;
use tsrs_vfs::{FS, bundled};

use crate::headless::{OverlayFs, Payload};

type ProgramSetup = dyn Fn(&'static Program) -> Result<(), String> + Send + Sync;

struct LintSystem {
    base: &'static dyn tsc::System,
    fs: Arc<dyn FS>,
    setup: Box<ProgramSetup>,
    rules: Arc<dyn Fn(P<SourceFile>) -> Vec<ConfiguredRule> + Send + Sync>,
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
    fn compiler_host(&self, host: Arc<dyn CompilerHost>) -> Arc<dyn CompilerHost> {
        tsrs_linter::linting_host(host, Arc::clone(&self.rules))
    }
    fn program_setup(&self) -> Option<&(dyn Fn(&'static Program) -> Result<(), String> + Sync)> {
        Some(&*self.setup)
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
    let mut rules = FxHashMap::default();
    for config in payload.configs {
        let configured: Vec<_> = config
            .rules
            .into_iter()
            .filter_map(|rule| {
                rule_by_name(&rule.name).map(|definition| ConfiguredRule {
                    definition,
                    options: rule.options,
                })
            })
            .collect();
        for file in config.file_paths {
            let file = tspath::get_normalized_absolute_path(&file, cwd);
            rules.insert(
                tspath::to_path(
                    &base_fs.realpath(&file),
                    cwd,
                    fs.use_case_sensitive_file_names(),
                ),
                configured.clone(),
            );
            rules.insert(
                tspath::to_path(&file, cwd, fs.use_case_sensitive_file_names()),
                configured.clone(),
            );
        }
    }
    let rules = Arc::new(rules);
    let host_rules = Arc::clone(&rules);
    let output = Arc::new(Mutex::new(Vec::<RuleDiagnostic>::new()));
    let diagnostics = Arc::clone(&output);
    let sessions = Arc::new(Mutex::new(Vec::new()));
    let saved_sessions = Arc::clone(&sessions);
    let no_emit = Arc::new(Mutex::new(true));
    let saved_no_emit = Arc::clone(&no_emit);
    let setup = Box::new(move |program: &'static Program| {
        *saved_no_emit.lock().unwrap() = program.options().no_emit.is_true();
        let files: Vec<_> = program
            .source_files()
            .iter()
            .copied()
            .filter(|file| rules.contains_key(file.path()))
            .collect();
        let output = Arc::clone(&diagnostics);
        let session = LintSession::attach(
            program,
            &files,
            &|file| rules[file.path()].clone(),
            Fixes::default(),
            Arc::new(move |diagnostic| output.lock().unwrap().push(diagnostic)),
            false,
        )?;
        saved_sessions.lock().unwrap().push(session);
        Ok(())
    });
    let lint_sys: &'static LintSystem = Box::leak(Box::new(LintSystem {
        base: sys,
        fs,
        setup,
        rules: Arc::new(move |file| host_rules.get(file.path()).cloned().unwrap_or_default()),
    }));
    let mut result = execute::command_line(lint_sys, args);
    for session in sessions.lock().unwrap().iter() {
        if let Err(message) = session.result() {
            return error(sys, &message);
        }
    }
    let mut diagnostics = output.lock().unwrap();
    diagnostics.sort_by(|a, b| {
        a.source_file
            .file_name()
            .cmp(b.source_file.file_name())
            .then(a.range.pos().cmp(&b.range.pos()))
            .then(a.rule_name.cmp(b.rule_name))
    });
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
        result.status = if *no_emit.lock().unwrap() {
            tsc::ExitStatus::DiagnosticsPresent_OutputsSkipped
        } else {
            tsc::ExitStatus::DiagnosticsPresent_OutputsGenerated
        };
    }
    result
}
