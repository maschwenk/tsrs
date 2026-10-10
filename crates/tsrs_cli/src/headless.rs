use std::io::{Read, Write};
use std::sync::Arc;
use std::time::SystemTime;

use rustc_hash::FxHashMap;
use serde::Serialize;
use tsrs_core::tspath;
use tsrs_linter::{
    Fixes, HeadlessConfig as Payload, InternalDiagnostic, LintConfig, RuleDiagnostic,
    RunLinterOptions, TypeErrors, Workload,
};
use tsrs_project::TsConfigResolver;
use tsrs_vfs::{Entries, FS, FileInfo, FileMode, bundled, osvfs};

#[derive(Clone, Copy, Default)]
struct HeadlessOptions {
    fix: bool,
    fix_suggestions: bool,
    timings: bool,
}

fn parse_options(args: &[String]) -> Result<HeadlessOptions, String> {
    let mut options = HeadlessOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-fix" | "--fix" => options.fix = true,
            "-fix-suggestions" | "--fix-suggestions" => options.fix_suggestions = true,
            "-debug" | "--debug" => {
                index += 1;
                if args.get(index).map(String::as_str) != Some("timings") {
                    return Err("-debug only supports `timings`".to_string());
                }
                options.timings = true;
            }
            "-debug=timings" | "--debug=timings" => options.timings = true,
            value => return Err(format!("unknown headless option: {value}")),
        }
        index += 1;
    }
    Ok(options)
}

pub(crate) struct OverlayFs {
    base: Arc<dyn FS>,
    files: FxHashMap<String, String>,
}

impl OverlayFs {
    pub(crate) fn new(base: Arc<dyn FS>, files: FxHashMap<String, String>) -> OverlayFs {
        let case_sensitive = base.use_case_sensitive_file_names();
        OverlayFs {
            base,
            files: files
                .into_iter()
                .map(|(path, text)| {
                    (
                        tspath::get_canonical_file_name(
                            &tspath::normalize_path(&path),
                            case_sensitive,
                        ),
                        text,
                    )
                })
                .collect(),
        }
    }

    fn key(&self, path: &str) -> String {
        tspath::get_canonical_file_name(
            &tspath::normalize_path(path),
            self.use_case_sensitive_file_names(),
        )
    }
}

impl FS for OverlayFs {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.base.use_case_sensitive_file_names()
    }
    fn file_exists(&self, path: &str) -> bool {
        self.files.contains_key(&self.key(path)) || self.base.file_exists(path)
    }
    fn read_file(&self, path: &str) -> Option<String> {
        self.files
            .get(&self.key(path))
            .cloned()
            .or_else(|| self.base.read_file(path))
    }
    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.base.write_file(path, data)
    }
    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.base.append_file(path, data)
    }
    fn remove(&self, path: &str) -> Result<(), String> {
        self.base.remove(path)
    }
    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        self.base.chtimes(path, a_time, m_time)
    }
    fn directory_exists(&self, path: &str) -> bool {
        let prefix = format!("{}/", self.key(path).trim_end_matches('/'));
        self.files.keys().any(|file| file.starts_with(&prefix)) || self.base.directory_exists(path)
    }
    fn get_accessible_entries(&self, path: &str) -> Entries {
        let mut entries = self.base.get_accessible_entries(path);
        let prefix = format!("{}/", self.key(path).trim_end_matches('/'));
        for file in self
            .files
            .keys()
            .filter_map(|file| file.strip_prefix(&prefix))
        {
            if let Some((directory, _)) = file.split_once('/') {
                if !entries.directories.iter().any(|entry| entry == directory) {
                    entries.directories.push(directory.to_string());
                }
            } else if !entries.files.iter().any(|entry| entry == file) {
                entries.files.push(file.to_string());
            }
        }
        entries
    }
    fn stat(&self, path: &str) -> Option<FileInfo> {
        self.files
            .get(&self.key(path))
            .map(|text| FileInfo {
                name: path.to_string(),
                size: text.len() as i64,
                mode: FileMode::None,
                mod_time: None,
            })
            .or_else(|| self.base.stat(path))
    }
    fn realpath(&self, path: &str) -> String {
        if self.files.contains_key(&self.key(path)) {
            self.key(path)
        } else {
            self.base.realpath(path)
        }
    }
}

#[derive(Serialize)]
struct Range {
    pos: i32,
    end: i32,
}

#[derive(Serialize)]
struct Message {
    id: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    help: Option<String>,
}

#[derive(Serialize)]
struct Fix {
    text: String,
    range: Range,
}

#[derive(Serialize)]
struct Suggestion {
    message: Message,
    fixes: Vec<Fix>,
}

#[derive(Serialize)]
struct LabeledRange {
    label: String,
    range: Range,
}

#[derive(Serialize)]
struct DiagnosticPayload {
    kind: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    range: Option<Range>,
    message: Message,
    file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fixes: Vec<Fix>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggestions: Vec<Suggestion>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    labeled_ranges: Vec<LabeledRange>,
}

#[derive(Serialize)]
struct ErrorPayload {
    error: String,
}

#[derive(Serialize)]
struct TimingPayload {
    rules: Vec<Timing>,
}

#[derive(Serialize)]
struct Timing {
    rule_name: String,
    duration: u64,
    calls: u64,
}

fn range(range: tsrs_core::TextRange) -> Range {
    Range {
        pos: range.pos(),
        end: range.end(),
    }
}
fn message(message: tsrs_linter::RuleMessage) -> Message {
    Message {
        id: message.id,
        description: message.description,
        help: message.help,
    }
}
fn fixes(fixes: Vec<tsrs_linter::RuleFix>) -> Vec<Fix> {
    fixes
        .into_iter()
        .map(|fix| Fix {
            text: fix.text,
            range: range(fix.range),
        })
        .collect()
}

fn rule_diagnostic(diagnostic: RuleDiagnostic, options: HeadlessOptions) -> DiagnosticPayload {
    DiagnosticPayload {
        kind: 0,
        range: diagnostic.range.is_valid().then(|| range(diagnostic.range)),
        message: message(diagnostic.message),
        file_path: Some(diagnostic.source_file.file_name().to_string()),
        rule: Some(diagnostic.rule_name.to_string()),
        fixes: if options.fix {
            fixes(diagnostic.fixes)
        } else {
            Vec::new()
        },
        suggestions: if options.fix_suggestions {
            diagnostic
                .suggestions
                .into_iter()
                .map(|suggestion| Suggestion {
                    message: message(suggestion.message),
                    fixes: fixes(suggestion.fixes),
                })
                .collect()
        } else {
            Vec::new()
        },
        labeled_ranges: diagnostic
            .labeled_ranges
            .into_iter()
            .map(|label| LabeledRange {
                label: label.label,
                range: range(label.range),
            })
            .collect(),
    }
}

fn internal_diagnostic(diagnostic: InternalDiagnostic) -> DiagnosticPayload {
    DiagnosticPayload {
        kind: 1,
        range: diagnostic.range.map(range),
        message: Message {
            id: diagnostic.id,
            description: diagnostic.description,
            help: diagnostic.help,
        },
        file_path: diagnostic.file_path,
        rule: None,
        fixes: Vec::new(),
        suggestions: Vec::new(),
        labeled_ranges: Vec::new(),
    }
}

fn write_message(writer: &mut dyn Write, kind: u8, payload: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    let length =
        u32::try_from(bytes.len()).map_err(|_| "headless message is too large".to_string())?;
    writer
        .write_all(&length.to_le_bytes())
        .map_err(|error| error.to_string())?;
    writer
        .write_all(&[kind])
        .map_err(|error| error.to_string())?;
    writer.write_all(&bytes).map_err(|error| error.to_string())
}

fn write_error(error: String) {
    let mut stdout = std::io::stdout().lock();
    let _ = write_message(&mut stdout, 0, &ErrorPayload { error });
    let _ = stdout.flush();
}

pub fn run(args: &[String]) -> i32 {
    let options = match parse_options(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("error parsing options: {error}");
            return 1;
        }
    };
    let mut input = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut input) {
        write_error(format!("error reading from stdin: {error}"));
        return 1;
    }
    let payload: Payload = match serde_json::from_slice(&input) {
        Ok(payload) => payload,
        Err(error) => {
            write_error(format!("error parsing config: {error}"));
            return 1;
        }
    };
    if payload.version != 2 {
        write_error(format!(
            "error parsing config: unsupported version `{}`: expected `2`",
            payload.version
        ));
        return 1;
    }
    let cwd = match std::env::current_dir() {
        Ok(cwd) => tspath::normalize_slashes(&cwd.to_string_lossy()),
        Err(error) => {
            write_error(format!("error getting current directory: {error}"));
            return 1;
        }
    };
    let os: Arc<dyn FS> = Arc::new(osvfs::fs());
    let overlay = OverlayFs::new(os, payload.source_overrides.unwrap_or_default());
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(overlay));
    let resolver = TsConfigResolver::new(Arc::clone(&fs), &cwd);
    let lint = match LintConfig::new(
        &payload.configs,
        &cwd,
        fs.use_case_sensitive_file_names(),
        Fixes {
            fix: options.fix,
            fix_suggestions: options.fix_suggestions,
        },
        options.timings,
    ) {
        Ok(lint) => Arc::new(lint),
        Err(error) => {
            write_error(format!("error parsing config: {error}"));
            return 1;
        }
    };
    let mut files = Vec::new();
    for config in payload.configs {
        for file in config.file_paths {
            let file = tspath::normalize_slashes(&file);
            files.push(file);
        }
    }
    let mut workload = Workload::default();
    for (file, config) in resolver.find_tsconfigs(&files) {
        match config {
            Some(config) => workload.programs.entry(config).or_default().push(file),
            None => workload.unmatched_files.push(file),
        }
    }
    let run_options = RunLinterOptions {
        current_directory: cwd,
        workload,
        fs,
        lint,
        type_errors: TypeErrors {
            report_syntactic: payload.report_syntactic,
            report_semantic: payload.report_semantic,
        },
        suppress_program_diagnostics: std::env::var_os(
            "OXLINT_TSGOLINT_DANGEROUSLY_SUPPRESS_PROGRAM_DIAGNOSTICS",
        )
        .is_some_and(|value| value == "true"),
    };
    let result = tsrs_linter::run_linter(&run_options);
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            write_error(format!("error running linter: {error}"));
            return 1;
        }
    };
    let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());
    // Preserve tsgolint's compiler-diagnostics-before-rule-diagnostics framing even though each
    // file now runs both during the same checker task.
    for diagnostic in result
        .diagnostics
        .into_iter()
        .map(internal_diagnostic)
        .chain(
            result
                .lint
                .diagnostics
                .into_iter()
                .map(|d| rule_diagnostic(d, options)),
        )
    {
        if let Err(error) = write_message(&mut stdout, 1, &diagnostic) {
            eprintln!("error writing diagnostic: {error}");
            return 1;
        }
    }
    if options.timings {
        let rules = result
            .lint
            .timings
            .into_iter()
            .map(|timing| Timing {
                rule_name: timing.rule_name,
                duration: u64::try_from(timing.duration.as_nanos()).unwrap_or(u64::MAX),
                calls: timing.calls,
            })
            .collect();
        if let Err(error) = write_message(&mut stdout, 2, &TimingPayload { rules }) {
            eprintln!("error writing timings: {error}");
            return 1;
        }
    }
    if let Err(error) = stdout.flush() {
        eprintln!("error flushing diagnostics: {error}");
        return 1;
    }
    0
}
