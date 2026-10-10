use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;
use tsrs_ast::{Diagnostic, LintNodes, SourceFile};
use tsrs_compiler::{
    CheckFileHook, Checker, CompilerHost, Context, Program, ProgramOptions,
    new_cached_fs_compiler_host, new_program,
};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{
    CompilerOptions, JsxEmit, ModuleKind, ModuleResolutionKind, OwnedCell, P, ScriptTarget,
    Tristate,
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

type RulesForFile = dyn Fn(P<SourceFile>) -> Vec<ConfiguredRule> + Send + Sync;

struct LintHost {
    base: Arc<dyn CompilerHost>,
    get_rules: Arc<RulesForFile>,
}

/// Collect rule candidates in the original bind, including files bound during parallel loading.
pub fn linting_host(
    base: Arc<dyn CompilerHost>,
    get_rules: Arc<RulesForFile>,
) -> Arc<dyn CompilerHost> {
    Arc::new(LintHost { base, get_rules })
}

impl CompilerHost for LintHost {
    fn fs(&self) -> &dyn FS {
        self.base.fs()
    }
    fn default_library_path(&self) -> &str {
        self.base.default_library_path()
    }
    fn get_current_directory(&self) -> &str {
        self.base.get_current_directory()
    }
    fn trace(&self, message: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]) {
        self.base.trace(message, args);
    }
    fn get_resolved_project_reference(
        &self,
        name: &str,
        path: Path,
    ) -> Option<P<tsrs_tsoptions::ParsedCommandLine>> {
        self.base.get_resolved_project_reference(name, path)
    }
    fn get_source_file(&self, options: tsrs_ast::SourceFileParseOptions) -> Option<P<SourceFile>> {
        let file = self.base.get_source_file(options)?;
        if !(self.get_rules)(file).is_empty() && file.lint_nodes.get().is_none() {
            assert!(
                !file.is_bound(),
                "linting requires freshly parsed source files"
            );
            file.lint_nodes.set(Some(P::new(LintNodes {
                expression_statements: OwnedCell::new(&[]),
            })));
        }
        Some(file)
    }
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
    get_rules: Arc<RulesForFile>,
    setup: &dyn Fn(&'static Program) -> Result<(), String>,
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
    let mut opts = ProgramOptions::new(config, linting_host(host, get_rules));
    opts.use_source_of_project_reference = true;
    opts.single_threaded = Tristate::False;
    let program = new_program(opts);
    setup(program)?;
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
    Ok((Some(program), Vec::new()))
}

fn create_empty_program(
    fs: Arc<dyn FS>,
    cwd: &str,
    files: &[String],
    get_rules: Arc<RulesForFile>,
    setup: &dyn Fn(&'static Program) -> Result<(), String>,
) -> Result<&'static Program, String> {
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
    let mut opts = ProgramOptions::new(config, linting_host(host, get_rules));
    opts.single_threaded = Tristate::False;
    let program = new_program(opts);
    setup(program)?;
    Ok(program)
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

struct FileRules {
    rules: Vec<ConfiguredRule>,
    completed: Once,
}

/// A per-program lint pass. Executes on the file's checker after
/// semantic checking, including deferred checks, finishes. It never acquires another checker.
pub struct LintSession {
    files: FxHashMap<P<SourceFile>, FileRules>,
    fixes: Fixes,
    on_diagnostic: Arc<dyn Fn(RuleDiagnostic) + Send + Sync>,
    timings_enabled: bool,
    timings: Mutex<FxHashMap<&'static str, (Duration, u64)>>,
    error: Mutex<Option<String>>,
}

impl LintSession {
    pub fn attach(
        program: &'static Program,
        files: &[P<SourceFile>],
        get_rules: &dyn Fn(P<SourceFile>) -> Vec<ConfiguredRule>,
        fixes: Fixes,
        on_diagnostic: Arc<dyn Fn(RuleDiagnostic) + Send + Sync>,
        timings: bool,
    ) -> Result<Arc<Self>, String> {
        let mut configured = FxHashMap::default();
        for &file in files {
            let rules = get_rules(file);
            if !rules.is_empty() && file.lint_nodes.get().is_none() {
                return Err(format!("linting host did not prepare {}", file.file_name()));
            }
            configured.insert(
                file,
                FileRules {
                    rules,
                    completed: Once::new(),
                },
            );
        }
        let session = Arc::new(Self {
            files: configured,
            fixes,
            on_diagnostic,
            timings_enabled: timings,
            timings: Mutex::new(FxHashMap::default()),
            error: Mutex::new(None),
        });
        program.set_check_file_hook(Arc::clone(&session) as Arc<dyn CheckFileHook>);
        Ok(session)
    }

    pub fn result(&self) -> Result<Vec<RuleTimingRecord>, String> {
        if let Some(error) = self.error.lock().unwrap().as_ref() {
            return Err(error.clone());
        }
        let mut records: Vec<_> = self
            .timings
            .lock()
            .unwrap()
            .iter()
            .map(|(&name, &(duration, calls))| RuleTimingRecord {
                rule_name: name.to_string(),
                duration,
                calls,
            })
            .collect();
        records.sort_by(|a, b| a.rule_name.cmp(&b.rule_name));
        Ok(records)
    }
}

impl CheckFileHook for LintSession {
    fn includes(&self, file: P<SourceFile>) -> bool {
        self.files.contains_key(&file)
    }

    fn after_check(&self, program: &'static Program, checker: &mut Checker, file: P<SourceFile>) {
        let configured = &self.files[&file];
        configured.completed.call_once(|| {
            for configured in &configured.rules {
                let mut context = RuleContext {
                    source_file: file,
                    program,
                    checker,
                    rule_name: configured.definition.name,
                    fixes: self.fixes,
                    on_diagnostic: &self.on_diagnostic,
                };
                let start = self.timings_enabled.then(Instant::now);
                let result = (configured.definition.run)(&mut context, &configured.options);
                if let Some(start) = start {
                    let mut all = self.timings.lock().unwrap();
                    let stat = all.entry(configured.definition.name).or_default();
                    stat.0 += start.elapsed();
                    if let Ok(calls) = &result {
                        stat.1 += *calls;
                    }
                }
                if let Err(error) = result {
                    self.error.lock().unwrap().get_or_insert(error);
                    break;
                }
            }
        });
    }
}

fn run_on_program(
    options: &RunLinterOptions,
    program: &'static Program,
    files: &[P<SourceFile>],
    timings: &mut BTreeMap<String, (Duration, u64)>,
    session: &LintSession,
) -> Result<(), String> {
    program.bind_source_files();
    let ctx = Context::default();
    if !program.for_each_checker_group(files, |checker, _, file| {
        // Reporting switches never decide whether a requested file is checked.
        let semantic = program.get_semantic_diagnostics_with_checker(&ctx, checker, file);
        let mut diagnostics = if options.type_errors.report_syntactic {
            program.get_syntactic_diagnostics(&ctx, Some(file))
        } else {
            Vec::new()
        };
        if options.type_errors.report_semantic {
            diagnostics.extend(semantic);
        }
        for diagnostic in diagnostics {
            if diagnostic
                .file()
                .is_some_and(|f| f.file_name() == file.file_name())
            {
                (options.on_internal_diagnostic)(diagnostic_to_internal(
                    diagnostic,
                    Some(file.file_name()),
                    false,
                ));
            }
        }
    }) {
        return Err("program does not expose a checker pool".to_string());
    }
    for record in session.result()? {
        let stat = timings.entry(record.rule_name).or_default();
        stat.0 += record.duration;
        stat.1 += record.calls;
    }
    Ok(())
}

pub fn run_linter(options: &RunLinterOptions) -> Result<Vec<RuleTimingRecord>, String> {
    let mut timings = BTreeMap::new();
    for (config_name, names) in &options.workload.programs {
        let cwd = tspath::get_directory_path(config_name);
        let session = OnceLock::new();
        let setup = |program| {
            let files = requested_source_files(program, names, &cwd)?;
            session
                .set(LintSession::attach(
                    program,
                    &files,
                    &*options.get_rules_for_file,
                    options.fixes,
                    Arc::clone(&options.on_rule_diagnostic),
                    options.timings,
                )?)
                .ok();
            Ok(())
        };
        let (program, diagnostics) = create_configured_program(
            Arc::clone(&options.fs),
            config_name,
            options.suppress_program_diagnostics,
            Arc::clone(&options.get_rules_for_file),
            &setup,
        )?;
        for diagnostic in diagnostics {
            (options.on_internal_diagnostic)(diagnostic);
        }
        let Some(program) = program else { continue };
        let files = requested_source_files(program, names, &cwd)?;
        run_on_program(
            options,
            program,
            &files,
            &mut timings,
            session.get().unwrap(),
        )?;
    }
    if !options.workload.unmatched_files.is_empty() {
        let session = OnceLock::new();
        let setup = |program| {
            let files = requested_source_files(
                program,
                &options.workload.unmatched_files,
                &options.current_directory,
            )?;
            session
                .set(LintSession::attach(
                    program,
                    &files,
                    &*options.get_rules_for_file,
                    options.fixes,
                    Arc::clone(&options.on_rule_diagnostic),
                    options.timings,
                )?)
                .ok();
            Ok(())
        };
        let program = create_empty_program(
            Arc::clone(&options.fs),
            &options.current_directory,
            &options.workload.unmatched_files,
            Arc::clone(&options.get_rules_for_file),
            &setup,
        )?;
        let files = requested_source_files(
            program,
            &options.workload.unmatched_files,
            &options.current_directory,
        )?;
        run_on_program(
            options,
            program,
            &files,
            &mut timings,
            session.get().unwrap(),
        )?;
    }
    let mut result: Vec<_> = timings
        .into_iter()
        .map(|(name, (duration, calls))| RuleTimingRecord {
            rule_name: name,
            duration,
            calls,
        })
        .collect();
    result.sort_by(|a, b| a.rule_name.cmp(&b.rule_name));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuleDefinition;
    use serde_json::{Value, json};
    use tsrs_ast::{Kind, Node};
    use tsrs_vfs::vfstest;

    static CHECKED_RULE: RuleDefinition = RuleDefinition {
        name: "assert-checked",
        run: |context, _| {
            let links = context.checker.source_file_links.get(context.source_file);
            assert!(
                links.type_checked.get(),
                "lint ran before semantic checking completed"
            );
            assert!(
                links.unused_checked.get(),
                "lint ran before deferred diagnostics completed"
            );
            Ok(1)
        },
    };

    #[test]
    fn checking_is_mandatory_and_rules_run_once() {
        for (mut settings, directive) in [
            (json!({}), ""),
            (json!({"noCheck": true}), ""),
            (json!({}), "// @ts-nocheck\n"),
        ] {
            settings["noUnusedLocals"] = json!(true);
            for enabled in [false, true] {
                let source = format!(
                    "{directive}const x: number = 'bad';\nfunction f() {{ const unused = 1; }}\n"
                );
                let config = json!({"compilerOptions": settings, "files": ["file.ts"]}).to_string();
                let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(
                    [
                        ("/file.ts", source.as_str()),
                        ("/tsconfig.json", config.as_str()),
                    ],
                    true,
                )));
                let get_rules: Arc<RulesForFile> = Arc::new(move |_| {
                    if enabled {
                        vec![ConfiguredRule {
                            definition: &CHECKED_RULE,
                            options: Value::Null,
                        }]
                    } else {
                        Vec::new()
                    }
                });
                let session = OnceLock::new();
                let (program, errors) = create_configured_program(
                    Arc::clone(&fs),
                    "/tsconfig.json",
                    false,
                    Arc::clone(&get_rules),
                    &|program| {
                        let file = program
                            .source_files()
                            .iter()
                            .copied()
                            .find(|f| f.file_name() == "/file.ts")
                            .unwrap();
                        session
                            .set(LintSession::attach(
                                program,
                                &[file],
                                &*get_rules,
                                Fixes::default(),
                                Arc::new(|_| {}),
                                true,
                            )?)
                            .ok();
                        Ok(())
                    },
                )
                .unwrap();
                assert!(errors.is_empty());
                let program = program.unwrap();
                let file = program
                    .source_files()
                    .iter()
                    .copied()
                    .find(|f| f.file_name() == "/file.ts")
                    .unwrap();
                let options = RunLinterOptions {
                    current_directory: "/".into(),
                    workload: Workload::default(),
                    fs,
                    get_rules_for_file: get_rules,
                    on_rule_diagnostic: Arc::new(|_| panic!("unexpected rule report")),
                    on_internal_diagnostic: Arc::new(|_| panic!("reporting is disabled")),
                    fixes: Fixes::default(),
                    type_errors: TypeErrors::default(),
                    suppress_program_diagnostics: false,
                    timings: true,
                };
                run_on_program(
                    &options,
                    program,
                    &[file],
                    &mut BTreeMap::new(),
                    session.get().unwrap(),
                )
                .unwrap();
                program.for_each_checker_group(&[file], |checker, _, file| {
                    assert!(checker.source_file_links.get(file).type_checked.get());
                });
                program.get_semantic_diagnostics(&Context::default(), Some(file));
                let records = session.get().unwrap().result().unwrap();
                assert_eq!(
                    records.iter().map(|record| record.calls).sum::<u64>(),
                    u64::from(enabled)
                );
            }
        }
    }

    #[test]
    fn binder_candidates_match_source_order_including_deferred_and_unreachable_nodes() {
        let code = r#"
declare function p(): Promise<void>;
p();
function f() { p(); return; p(); }
class C { static { p(); } method() { p(); } field = () => { p(); }; }
(() => { p(); })();
with ({}) { p(); }
if (false) { p(); }
label: { p(); }
try { p(); } catch { p(); } finally { p(); }
"#;
        let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(
            [("/file.ts", code)],
            true,
        )));
        let rules: Arc<RulesForFile> = Arc::new(|_| {
            vec![ConfiguredRule {
                definition: &CHECKED_RULE,
                options: Value::Null,
            }]
        });
        let program =
            create_empty_program(fs, "/", &["/file.ts".into()], rules, &|_| Ok(())).unwrap();
        program.bind_source_files();
        let file = program
            .source_files()
            .iter()
            .copied()
            .find(|f| f.file_name() == "/file.ts")
            .unwrap();
        fn walk(node: P<Node>, nodes: &mut Vec<P<Node>>) {
            if node.kind() == Kind::ExpressionStatement {
                nodes.push(node);
            }
            node.for_each_child(&mut |child| {
                walk(child, nodes);
                false
            });
        }
        let mut expected = Vec::new();
        walk(file.as_node(), &mut expected);
        assert_eq!(expected.len(), 14);
        assert_eq!(
            file.lint_nodes.get().unwrap().expression_statements.get(),
            expected
        );
    }
}
