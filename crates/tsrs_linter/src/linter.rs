use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;
use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_compiler::{Context, Program, ProgramOptions, new_cached_fs_compiler_host, new_program};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{
    CompilerOptions, JsxEmit, ModuleKind, ModuleResolutionKind, P, ScriptTarget, Tristate,
};
use tsrs_tsoptions::{
    ParseConfigHost, get_parsed_command_line_of_config_file, new_parsed_command_line,
};
use tsrs_vfs::{FS, bundled};

use crate::{ConfiguredRule, Fixes, InternalDiagnostic, RuleContext, RuleDiagnostic};

#[derive(Clone, Debug, Default)]
pub struct Workload {
    pub programs: BTreeMap<String, Vec<String>>,
    pub unmatched_files: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TypeErrors {
    pub report_syntactic: bool,
    pub report_semantic: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RuleTimingRecord {
    pub rule_name: String,
    pub duration: Duration,
    pub calls: u64,
}

pub struct RunLinterOptions {
    pub current_directory: String,
    pub workload: Workload,
    pub fs: Arc<dyn FS>,
    pub get_rules_for_file: Arc<dyn Fn(P<SourceFile>) -> Vec<ConfiguredRule> + Send + Sync>,
    pub on_rule_diagnostic: Arc<dyn Fn(RuleDiagnostic) + Send + Sync>,
    pub on_internal_diagnostic: Arc<dyn Fn(InternalDiagnostic) + Send + Sync>,
    pub fixes: Fixes,
    pub type_errors: TypeErrors,
    pub suppress_program_diagnostics: bool,
    pub timings: bool,
}

struct ConfigHost {
    fs: Arc<dyn FS>,
    cwd: String,
}

impl ParseConfigHost for ConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

fn diagnostic_to_internal(
    d: P<Diagnostic>,
    fallback_file: Option<&str>,
    config: bool,
) -> InternalDiagnostic {
    let range = d.loc();
    InternalDiagnostic {
        range: range.is_valid().then_some(range),
        id: if config {
            "tsconfig-error".to_string()
        } else {
            format!("TS{}", d.code())
        },
        description: if config {
            "Invalid tsconfig".to_string()
        } else {
            tsrs_compiler::diagnosticwriter::flatten_diagnostic_message(d, "\n")
        },
        help: config.then(|| tsrs_compiler::diagnosticwriter::flatten_diagnostic_message(d, "\n")),
        file_path: d
            .file()
            .map(|f| f.file_name().to_string())
            .or_else(|| fallback_file.map(str::to_string)),
    }
}

fn create_configured_program(
    fs: Arc<dyn FS>,
    config_name: &str,
    suppress_program_diagnostics: bool,
) -> Result<(Option<&'static Program>, Vec<InternalDiagnostic>), String> {
    let cwd = tspath::get_directory_path(config_name);
    let parse_host: &'static ConfigHost = Box::leak(Box::new(ConfigHost {
        fs: Arc::clone(&fs),
        cwd: cwd.clone(),
    }));
    let (parsed, read_errors) =
        get_parsed_command_line_of_config_file(config_name, None, None, parse_host, None);
    if !read_errors.is_empty() {
        return Ok((
            None,
            read_errors
                .into_iter()
                .map(|d| diagnostic_to_internal(d, Some(config_name), true))
                .collect(),
        ));
    }
    let Some(parsed) = parsed else {
        return Err(format!("couldn't parse tsconfig at {config_name}"));
    };
    if !parsed.errors.is_empty() && !suppress_program_diagnostics {
        let errors = parsed
            .errors
            .iter()
            .copied()
            .map(|d| diagnostic_to_internal(d, Some(config_name), true))
            .collect();
        return Ok((None, errors));
    }
    let config = P::new(parsed);
    let host = new_cached_fs_compiler_host(&cwd, fs, &bundled::lib_path(), None, None);
    let mut opts = ProgramOptions::new(config, host);
    opts.use_source_of_project_reference = true;
    opts.single_threaded = Tristate::False;
    let program = new_program(opts);
    let diagnostics = program.get_program_diagnostics();
    if !diagnostics.is_empty() && !suppress_program_diagnostics {
        return Ok((
            None,
            diagnostics
                .into_iter()
                .map(|d| diagnostic_to_internal(d, Some(config_name), true))
                .collect(),
        ));
    }
    program.bind_source_files();
    Ok((Some(program), Vec::new()))
}

fn create_empty_program(fs: Arc<dyn FS>, cwd: &str, files: &[String]) -> &'static Program {
    let options = P::new(CompilerOptions {
        allow_js: Tristate::True,
        module: ModuleKind::ESNext,
        module_resolution: ModuleResolutionKind::Bundler,
        target: ScriptTarget::ES2022,
        jsx: JsxEmit::ReactJSX,
        allow_importing_ts_extensions: Tristate::True,
        strict_null_checks: Tristate::True,
        strict_function_types: Tristate::True,
        source_map: Tristate::True,
        es_module_interop: Tristate::True,
        allow_non_ts_extensions: Tristate::True,
        resolve_json_module: Tristate::True,
        ..Default::default()
    });
    let config = P::new(new_parsed_command_line(
        options,
        files.to_vec(),
        Vec::new(),
        ComparePathsOptions {
            current_directory: cwd.to_string(),
            use_case_sensitive_file_names: fs.use_case_sensitive_file_names(),
        },
    ));
    let host = new_cached_fs_compiler_host(cwd, fs, &bundled::lib_path(), None, None);
    let mut opts = ProgramOptions::new(config, host);
    opts.single_threaded = Tristate::False;
    let program = new_program(opts);
    program.bind_source_files();
    program
}

fn requested_source_files(
    program: &'static Program,
    names: &[String],
    cwd: &str,
) -> Result<Vec<P<SourceFile>>, String> {
    let mut requested: FxHashMap<Path, &str> = names
        .iter()
        .map(|name| {
            (
                tspath::to_path(
                    name,
                    cwd,
                    program.host().fs().use_case_sensitive_file_names(),
                ),
                name.as_str(),
            )
        })
        .collect();
    let mut files = Vec::with_capacity(names.len());
    for &source_file in program.source_files() {
        if requested.remove(source_file.path()).is_some() {
            files.push(source_file);
        }
    }
    if requested.is_empty() {
        return Ok(files);
    }
    Err(format!(
        "requested files are not in their TypeScript program: {}",
        requested.values().copied().collect::<Vec<_>>().join(", ")
    ))
}

fn report_type_errors(
    program: &'static Program,
    files: &[P<SourceFile>],
    options: TypeErrors,
    report: &Arc<dyn Fn(InternalDiagnostic) + Send + Sync>,
) {
    let ctx = Context::default();
    for &file in files {
        if options.report_syntactic {
            for diagnostic in program.get_syntactic_diagnostics(&ctx, Some(file)) {
                if diagnostic
                    .file()
                    .is_some_and(|f| f.file_name() == file.file_name())
                {
                    report(diagnostic_to_internal(
                        diagnostic,
                        Some(file.file_name()),
                        false,
                    ));
                }
            }
        }
        if options.report_semantic {
            for diagnostic in program.get_semantic_diagnostics(&ctx, Some(file)) {
                if diagnostic
                    .file()
                    .is_some_and(|f| f.file_name() == file.file_name())
                {
                    report(diagnostic_to_internal(
                        diagnostic,
                        Some(file.file_name()),
                        false,
                    ));
                }
            }
        }
    }
}

fn run_on_program(
    options: &RunLinterOptions,
    program: &'static Program,
    files: &[P<SourceFile>],
    timings: &Mutex<FxHashMap<&'static str, (Duration, u64)>>,
) -> Result<(), String> {
    report_type_errors(
        program,
        files,
        options.type_errors,
        &options.on_internal_diagnostic,
    );
    let error = Mutex::new(None);
    if !program.for_each_checker_group(files, |checker, _, file| {
        if error.lock().unwrap().is_some() {
            return;
        }
        for configured in (options.get_rules_for_file)(file) {
            let mut context = RuleContext {
                source_file: file,
                program,
                checker,
                rule_name: configured.definition.name,
                fixes: options.fixes,
                on_diagnostic: &options.on_rule_diagnostic,
            };
            let start = Instant::now();
            let result = (configured.definition.run)(&mut context, &configured.options);
            let elapsed = start.elapsed();
            if options.timings {
                let mut all = timings.lock().unwrap();
                let stat = all.entry(configured.definition.name).or_default();
                stat.0 += elapsed;
                if let Ok(calls) = &result {
                    stat.1 += *calls;
                }
            }
            if let Err(err) = result {
                *error.lock().unwrap() = Some(err);
                return;
            }
        }
    }) {
        return Err("program does not expose a checker pool".to_string());
    }
    match error.into_inner().unwrap() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub fn run_linter(options: &RunLinterOptions) -> Result<Vec<RuleTimingRecord>, String> {
    let timings = Mutex::new(FxHashMap::default());
    for (config_name, names) in &options.workload.programs {
        let cwd = tspath::get_directory_path(config_name);
        let (program, diagnostics) = create_configured_program(
            Arc::clone(&options.fs),
            config_name,
            options.suppress_program_diagnostics,
        )?;
        for diagnostic in diagnostics {
            (options.on_internal_diagnostic)(diagnostic);
        }
        let Some(program) = program else { continue };
        let files = requested_source_files(program, names, &cwd)?;
        run_on_program(&options, program, &files, &timings)?;
    }
    if !options.workload.unmatched_files.is_empty() {
        let program = create_empty_program(
            Arc::clone(&options.fs),
            &options.current_directory,
            &options.workload.unmatched_files,
        );
        let files = requested_source_files(
            program,
            &options.workload.unmatched_files,
            &options.current_directory,
        )?;
        run_on_program(&options, program, &files, &timings)?;
    }
    let mut result: Vec<_> = timings
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|(name, (duration, calls))| RuleTimingRecord {
            rule_name: name.to_string(),
            duration,
            calls,
        })
        .collect();
    result.sort_by(|a, b| a.rule_name.cmp(&b.rule_name));
    Ok(result)
}
