use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_compiler::{Context, Program, ProgramOptions, new_cached_fs_compiler_host, new_program};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{
    CompilerOptions, JsxEmit, ModuleKind, ModuleResolutionKind, P, ScriptTarget, TextRange,
    Tristate,
};
use tsrs_tsoptions::{
    ParseConfigHost, get_parsed_command_line_of_config_file, new_parsed_command_line,
};
use tsrs_vfs::{FS, bundled};

use crate::{LintConfig, LintOutput};

#[derive(Clone, Debug)]
pub struct InternalDiagnostic {
    pub range: Option<TextRange>,
    pub id: String,
    pub description: String,
    pub help: Option<String>,
    pub file_path: Option<String>,
}

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

pub struct RunLinterOptions {
    pub current_directory: String,
    pub workload: Workload,
    pub fs: Arc<dyn FS>,
    pub lint: Arc<LintConfig>,
    pub type_errors: TypeErrors,
    pub suppress_program_diagnostics: bool,
}

pub struct LinterResult {
    pub lint: LintOutput,
    pub diagnostics: Vec<InternalDiagnostic>,
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
    lint: Arc<LintConfig>,
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
    let diagnostics = parsed.get_config_file_parsing_diagnostics();
    if !diagnostics.is_empty() && !suppress_program_diagnostics {
        return Ok((
            None,
            diagnostics
                .into_iter()
                .map(|d| diagnostic_to_internal(d, Some(config_name), true))
                .collect(),
        ));
    }
    let config = P::new(parsed);
    let host = new_cached_fs_compiler_host(&cwd, fs, &bundled::lib_path(), None, None, None);
    let mut opts = ProgramOptions::new(config, host);
    opts.use_source_of_project_reference = true;
    opts.single_threaded = Tristate::False;
    opts.lint = Some(lint);
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
    Ok((Some(program), Vec::new()))
}

fn create_empty_program(
    fs: Arc<dyn FS>,
    cwd: &str,
    files: &[String],
    lint: Arc<LintConfig>,
) -> &'static Program {
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
    let host = new_cached_fs_compiler_host(cwd, fs, &bundled::lib_path(), None, None, None);
    let mut opts = ProgramOptions::new(config, host);
    opts.single_threaded = Tristate::False;
    opts.lint = Some(lint);
    new_program(opts)
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

fn run_on_program(
    options: &RunLinterOptions,
    program: &'static Program,
    files: &[P<SourceFile>],
) -> Result<Vec<InternalDiagnostic>, String> {
    program.bind_source_files();
    let ctx = Context::default();
    let output = Mutex::new(Vec::new());
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
        output.lock().unwrap().extend(
            diagnostics
                .into_iter()
                .filter(|diagnostic| {
                    diagnostic
                        .file()
                        .is_some_and(|f| f.file_name() == file.file_name())
                })
                .map(|diagnostic| {
                    diagnostic_to_internal(diagnostic, Some(file.file_name()), false)
                }),
        );
    }) {
        return Err("program does not expose a checker pool".to_string());
    }
    Ok(output.into_inner().unwrap())
}

/// Create the requested TypeScript programs and check their files. Rules run inside those checks.
pub fn run_linter(options: &RunLinterOptions) -> Result<LinterResult, String> {
    let mut diagnostics = Vec::new();
    for (config_name, names) in &options.workload.programs {
        let cwd = tspath::get_directory_path(config_name);
        let (program, errors) = create_configured_program(
            Arc::clone(&options.fs),
            config_name,
            options.suppress_program_diagnostics,
            Arc::clone(&options.lint),
        )?;
        diagnostics.extend(errors);
        let Some(program) = program else { continue };
        let files = requested_source_files(program, names, &cwd)?;
        diagnostics.extend(run_on_program(options, program, &files)?);
    }
    if !options.workload.unmatched_files.is_empty() {
        let program = create_empty_program(
            Arc::clone(&options.fs),
            &options.current_directory,
            &options.workload.unmatched_files,
            Arc::clone(&options.lint),
        );
        let mut files = Vec::with_capacity(options.workload.unmatched_files.len());
        let mut seen = FxHashSet::default();
        for name in &options.workload.unmatched_files {
            // Like tsgolint's inferred program, look up each path so package redirects and
            // symlink aliases resolve to their source file. Check each resolved file only once.
            let file = program.get_source_file(name).ok_or_else(|| {
                format!("requested file is not in its inferred TypeScript program: {name}")
            })?;
            if seen.insert(file) {
                files.push(file);
            }
        }
        diagnostics.extend(run_on_program(options, program, &files)?);
    }
    Ok(LinterResult {
        lint: options.lint.take_output(),
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileConfig, Fixes, NO_FLOATING_PROMISES, RequestedRule};
    use serde_json::{Value, json};
    use tsrs_ast::{Kind, Node};
    use tsrs_vfs::vfstest;

    fn config(files: &[&str], enabled: bool) -> Arc<LintConfig> {
        Arc::new(
            LintConfig::new(
                &[FileConfig {
                    file_paths: files.iter().map(|file| file.to_string()).collect(),
                    rules: if enabled {
                        vec![RequestedRule {
                            name: NO_FLOATING_PROMISES.into(),
                            options: Value::Null,
                        }]
                    } else {
                        vec![]
                    },
                }],
                "/",
                true,
                Fixes::default(),
                true,
            )
            .unwrap(),
        )
    }

    #[test]
    fn checking_is_mandatory_and_each_node_runs_once() {
        for (mut settings, directive, file_name) in [
            (json!({}), "", "/file.ts"),
            (json!({"noCheck": true}), "", "/file.ts"),
            (json!({}), "// @ts-nocheck\n", "/file.ts"),
            (json!({"skipLibCheck": true}), "", "/file.d.ts"),
        ] {
            settings["noUnusedLocals"] = json!(true);
            for enabled in [false, true] {
                let source = format!(
                    "{directive}declare function p(): Promise<void>;\np();\nconst x: number = 'bad';\n"
                );
                let tsconfig =
                    json!({"compilerOptions": settings, "files": [file_name]}).to_string();
                let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(
                    [
                        (file_name, source.as_str()),
                        ("/tsconfig.json", tsconfig.as_str()),
                    ],
                    true,
                )));
                let lint = config(&[file_name], enabled);
                let (program, errors) = create_configured_program(
                    Arc::clone(&fs),
                    "/tsconfig.json",
                    false,
                    Arc::clone(&lint),
                )
                .unwrap();
                assert!(errors.is_empty());
                let program = program.unwrap();
                let file = program.get_source_file(file_name).unwrap();
                let options = RunLinterOptions {
                    current_directory: "/".into(),
                    workload: Workload::default(),
                    fs,
                    lint: Arc::clone(&lint),
                    type_errors: TypeErrors::default(),
                    suppress_program_diagnostics: false,
                };
                assert!(
                    run_on_program(&options, program, &[file])
                        .unwrap()
                        .is_empty()
                );
                program.for_each_checker_group(&[file], |checker, _, file| {
                    assert!(checker.source_file_links.get(file).type_checked.get());
                    assert!(checker.source_file_links.get(file).unused_checked.get());
                });
                let output = lint.take_output();
                assert_eq!(output.diagnostics.len(), usize::from(enabled));
                assert_eq!(
                    output
                        .timings
                        .iter()
                        .map(|record| record.calls)
                        .sum::<u64>(),
                    u64::from(enabled)
                );
                program.get_semantic_diagnostics(&Context::default(), Some(file));
                let repeated = lint.take_output();
                assert!(repeated.diagnostics.is_empty());
                assert!(repeated.timings.is_empty());
            }
        }
    }

    #[test]
    fn checker_dispatch_covers_deferred_unreachable_and_skipped_bodies() {
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
        let lint = config(&["/file.ts"], true);
        let program = create_empty_program(fs, "/", &["/file.ts".into()], Arc::clone(&lint));
        let file = program.get_source_file("/file.ts").unwrap();
        program.get_semantic_diagnostics(&Context::default(), Some(file));
        fn walk(node: P<Node>, nodes: &mut Vec<P<Node>>) {
            if node.kind() == Kind::ExpressionStatement {
                nodes.push(node);
            }
            node.for_each_child(&mut |child| {
                walk(child, nodes);
                false
            });
        }
        let mut nodes = Vec::new();
        walk(file.as_node(), &mut nodes);
        assert_eq!(nodes.len(), 14);
        program.for_each_checker_group(&[file], |checker, _, _| {
            for &node in &nodes {
                assert!(
                    checker
                        .node_links
                        .get(node)
                        .flags
                        .get()
                        .intersects(tsrs_checker::NodeCheckFlags::LintChecked),
                    "statement at {} was skipped",
                    node.pos()
                );
            }
        });
        let output = lint.take_output();
        assert_eq!(output.timings[0].calls, 14);
        assert_eq!(output.diagnostics.len(), 12); // with's `any` and the void-returning IIFE are not promises.
        assert!(
            output
                .diagnostics
                .windows(2)
                .all(|pair| pair[0].range.pos() < pair[1].range.pos())
        );
    }

    #[test]
    fn per_file_options_and_cross_file_inference_keep_results_on_the_owning_checker() {
        let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(
            [
                (
                    "/a.ts",
                    "import { b } from './b'; export const a = b(); Promise.resolve(a); void Promise.resolve(a);",
                ),
                (
                    "/b.ts",
                    "export const b = () => { Promise.resolve(1); return 1; }; void Promise.resolve(1);",
                ),
                (
                    "/tsconfig.json",
                    r#"{"compilerOptions":{"target":"esnext","module":"esnext","checkers":2},"files":["a.ts","b.ts"]}"#,
                ),
            ],
            true,
        )));
        let configs = [
            FileConfig {
                file_paths: vec!["/a.ts".into()],
                rules: vec![RequestedRule {
                    name: NO_FLOATING_PROMISES.into(),
                    options: Value::Null,
                }],
            },
            FileConfig {
                file_paths: vec!["/b.ts".into()],
                rules: vec![RequestedRule {
                    name: NO_FLOATING_PROMISES.into(),
                    options: json!({"ignoreVoid":false}),
                }],
            },
        ];
        let lint = Arc::new(LintConfig::new(&configs, "/", true, Fixes::default(), true).unwrap());
        let result = run_linter(&RunLinterOptions {
            current_directory: "/".into(),
            workload: Workload {
                programs: BTreeMap::from([(
                    "/tsconfig.json".into(),
                    vec!["/a.ts".into(), "/b.ts".into()],
                )]),
                unmatched_files: vec![],
            },
            fs,
            lint,
            type_errors: TypeErrors::default(),
            suppress_program_diagnostics: false,
        })
        .unwrap();
        assert!(result.diagnostics.is_empty());
        let files: Vec<_> = result
            .lint
            .diagnostics
            .iter()
            .map(|d| d.source_file.file_name())
            .collect();
        assert_eq!(files, ["/a.ts", "/b.ts", "/b.ts"]);
        assert_eq!(result.lint.timings[0].calls, 4);
    }

    #[test]
    fn forked_checker_preserves_pending_lint_output() {
        let lint = config(&["/a.ts", "/b.ts"], true);
        let fs = vfstest::from_map(
            [
                ("/a.ts", vfstest::MapFile::from("Promise.resolve();")),
                ("/b.ts", vfstest::MapFile::from("Promise.reject();")),
            ],
            true,
        );
        let program = create_empty_program(
            Arc::new(bundled::wrap_fs(fs)),
            "/",
            &["/a.ts".into(), "/b.ts".into()],
            Arc::clone(&lint),
        );
        program.bind_source_files();
        let file = program.get_source_file("/a.ts").unwrap();
        let ctx = Context::default();
        let mut seed = tsrs_checker::new_checker(program);
        seed.get_diagnostics_exported(&ctx, file);
        let mut fork = tsrs_checker::Checker::fork(Box::leak(seed));
        lint.collect(&mut fork, file);
        let file = program.get_source_file("/b.ts").unwrap();
        assert!(
            program
                .get_semantic_diagnostics_with_checker(&ctx, &mut fork, file)
                .is_empty()
        );
        let output = lint.take_output();
        assert_eq!(output.diagnostics.len(), 2);
        assert_eq!(output.diagnostics[0].message.id, "floatingVoid");
        assert_eq!(output.diagnostics[1].message.id, "floatingVoid");
        assert_eq!(output.timings[0].calls, 2);
    }

    #[test]
    fn inferred_project_resolves_symlink_aliases_and_checks_each_file_once() {
        let fs = vfstest::from_map(
            [
                (
                    "/repo/packages/pkg/package.json",
                    vfstest::MapFile::from(
                        r##"{"name":"pkg","version":"1.0.0","type":"module","imports":{"#value":"./value.ts"}}"##,
                    ),
                ),
                (
                    "/repo/packages/pkg/index.ts",
                    vfstest::MapFile::from("import { value } from '#value'; void value;"),
                ),
                (
                    "/repo/packages/pkg/value.ts",
                    vfstest::MapFile::from(
                        "export const value: number = 'bad'; Promise.resolve();",
                    ),
                ),
                ("/repo/linked/pkg", vfstest::symlink("/repo/packages/pkg")),
            ],
            true,
        );
        let names = [
            "/repo/linked/pkg/index.ts",
            "/repo/linked/pkg/value.ts",
            "/repo/packages/pkg/index.ts",
            "/repo/packages/pkg/value.ts",
        ];
        let result = run_linter(&RunLinterOptions {
            current_directory: "/repo".into(),
            workload: Workload {
                programs: BTreeMap::new(),
                unmatched_files: names.iter().map(|name| (*name).into()).collect(),
            },
            fs: Arc::new(bundled::wrap_fs(fs)),
            lint: config(&names, true),
            type_errors: TypeErrors {
                report_syntactic: true,
                report_semantic: true,
            },
            suppress_program_diagnostics: false,
        })
        .unwrap();
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].id, "TS2322");
        assert_eq!(result.lint.diagnostics.len(), 1);
        assert_eq!(result.lint.diagnostics[0].message.id, "floatingVoid");
        assert_eq!(result.lint.timings[0].calls, 3);
    }
}
