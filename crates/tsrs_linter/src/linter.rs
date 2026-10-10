use std::collections::BTreeMap;
use std::sync::Arc;

use rustc_hash::FxHashSet;
use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_compiler::{Context, Program};
use tsrs_core::context::{CheckerLifetime, with_checker_lifetime};
use tsrs_core::tspath;
use tsrs_core::{
    CompilerOptions, JsxEmit, ModuleKind, ModuleResolutionKind, P, ScriptTarget, TextRange,
    Tristate,
};
use tsrs_project::{SessionInit, SessionOptions, Snapshot, new_snapshot_host};
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

#[derive(Clone, Copy, Debug, Default)]
pub struct TypeErrors {
    pub report_syntactic: bool,
    pub report_semantic: bool,
}

pub struct RunLinterOptions {
    pub current_directory: String,
    pub file_names: Vec<String>,
    pub fs: Arc<dyn FS>,
    pub lint: Arc<LintConfig>,
    pub type_errors: TypeErrors,
    pub suppress_program_diagnostics: bool,
}

/// Keeps the discovered programs and their ASTs alive while diagnostics are consumed.
pub struct LinterResult {
    lint: LintOutput,
    diagnostics: Vec<InternalDiagnostic>,
    _projects: LoadedProjects,
}

impl LinterResult {
    pub fn lint(&self) -> &LintOutput {
        &self.lint
    }

    pub fn diagnostics(&self) -> &[InternalDiagnostic] {
        &self.diagnostics
    }
}

struct LoadedProjects(Arc<Snapshot>);

impl Drop for LoadedProjects {
    fn drop(&mut self) {
        self.0.deref();
    }
}

fn load_projects(options: &RunLinterOptions) -> Result<LoadedProjects, String> {
    let inferred_options = P::new(CompilerOptions {
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
    let host = new_snapshot_host(&SessionInit {
        background_ctx: Context::default(),
        options: Arc::new(SessionOptions {
            current_directory: options.current_directory.clone(),
            default_library_path: bundled::lib_path(),
            compiler_options_for_inferred_projects: Some(inferred_options),
            lint: Some(Arc::clone(&options.lint)),
            ..Default::default()
        }),
        fs: Arc::clone(&options.fs),
        client: None,
        logger: None,
        npm_executor: None,
        parse_cache: None,
        content_mapped_parse_cache: None,
    });
    let root = LoadedProjects(host.new_root_snapshot());
    host.open_files(&Context::default(), &root.0, &options.file_names)
        .map(LoadedProjects)
        .map_err(|error| error.to_string())
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

fn run_on_program(
    options: &RunLinterOptions,
    program: &'static Program,
    files: &[P<SourceFile>],
) -> Vec<InternalDiagnostic> {
    let ctx = with_checker_lifetime(&Context::default(), CheckerLifetime::Diagnostics);
    let mut checker = program.get_type_checker(&ctx);
    let mut output = Vec::new();
    for &file in files {
        // Reporting switches never decide whether a requested file is checked.
        let semantic = program.get_semantic_diagnostics_with_checker(&ctx, &mut checker, file);
        let mut diagnostics = if options.type_errors.report_syntactic {
            program.get_syntactic_diagnostics(&ctx, Some(file))
        } else {
            Vec::new()
        };
        if options.type_errors.report_semantic {
            diagnostics.extend(semantic);
        }
        output.extend(
            diagnostics
                .into_iter()
                .filter(|diagnostic| diagnostic.file().is_some_and(|f| f == file))
                .map(|diagnostic| {
                    diagnostic_to_internal(diagnostic, Some(file.file_name()), false)
                }),
        );
    }
    output
}

/// Discover projects for the requested paths and check the selected files in their existing programs.
pub fn run_linter(options: &RunLinterOptions) -> Result<LinterResult, String> {
    let projects = load_projects(options)?;
    let collection = &projects.0.project_collection;
    let mut selected = BTreeMap::<_, FxHashSet<P<SourceFile>>>::new();
    for name in &options.file_names {
        let name = tspath::get_normalized_absolute_path(name, &options.current_directory);
        let path = tspath::to_path(
            &name,
            &options.current_directory,
            options.fs.use_case_sensitive_file_names(),
        );
        let project = collection
            .get_default_project(&path)
            .ok_or_else(|| format!("no project found for requested file: {name}"))?;
        let file = project
            .get_program()
            .and_then(|program| program.get_source_file(&name))
            .ok_or_else(|| format!("requested file is not in its TypeScript program: {name}"))?;
        selected.entry(project.id()).or_default().insert(file);
    }
    let mut diagnostics = Vec::new();
    for (id, selected_files) in selected {
        let project = collection.get_project(&id).unwrap();
        let program = project.get_program().unwrap();
        if id.configured().is_some() && !options.suppress_program_diagnostics {
            let mut errors = project
                .command_line
                .unwrap()
                .get_config_file_parsing_diagnostics();
            if errors.is_empty() {
                errors = program.get_program_diagnostics();
            }
            if !errors.is_empty() {
                diagnostics.extend(
                    errors
                        .into_iter()
                        .map(|d| diagnostic_to_internal(d, Some(project.config_file_name()), true)),
                );
                continue;
            }
        }
        let files: Vec<_> = program
            .source_files()
            .iter()
            .copied()
            .filter(|file| selected_files.contains(file))
            .collect();
        diagnostics.extend(run_on_program(options, program, &files));
    }
    Ok(LinterResult {
        lint: options.lint.take_output(),
        diagnostics,
        _projects: projects,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileConfig, Fixes, NO_FLOATING_PROMISES, RequestedRule};
    use serde_json::{Value, json};
    use tsrs_ast::{Kind, Node};
    use tsrs_vfs::vfstest;

    fn program_for_file(result: &LinterResult, name: &str) -> &'static Program {
        result
            ._projects
            .0
            .project_collection
            .get_default_project(&tspath::Path::new(name))
            .unwrap()
            .get_program()
            .unwrap()
    }

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
                let result = run_linter(&RunLinterOptions {
                    current_directory: "/".into(),
                    file_names: vec![file_name.into()],
                    fs,
                    lint: Arc::clone(&lint),
                    type_errors: TypeErrors::default(),
                    suppress_program_diagnostics: false,
                })
                .unwrap();
                assert!(result.diagnostics.is_empty());
                let program = program_for_file(&result, file_name);
                let file = program.get_source_file(file_name).unwrap();
                let ctx = with_checker_lifetime(&Context::default(), CheckerLifetime::Diagnostics);
                {
                    let mut checker = program.get_type_checker(&ctx);
                    assert!(checker.source_file_links.get(file).type_checked.get());
                    assert!(checker.source_file_links.get(file).unused_checked.get());
                }
                let output = &result.lint;
                assert_eq!(output.diagnostics.len(), usize::from(enabled));
                assert_eq!(
                    output
                        .timings
                        .iter()
                        .map(|record| record.calls)
                        .sum::<u64>(),
                    u64::from(enabled)
                );
                program.get_semantic_diagnostics(&ctx, Some(file));
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
        let result = run_linter(&RunLinterOptions {
            current_directory: "/".into(),
            file_names: vec!["/file.ts".into()],
            fs,
            lint: Arc::clone(&lint),
            type_errors: TypeErrors::default(),
            suppress_program_diagnostics: false,
        })
        .unwrap();
        let program = program_for_file(&result, "/file.ts");
        let file = program.get_source_file("/file.ts").unwrap();
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
        {
            let ctx = with_checker_lifetime(&Context::default(), CheckerLifetime::Diagnostics);
            let mut checker = program.get_type_checker(&ctx);
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
        }
        let output = &result.lint;
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
            file_names: vec!["/a.ts".into(), "/b.ts".into()],
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
        let projects = load_projects(&RunLinterOptions {
            current_directory: "/".into(),
            file_names: vec!["/a.ts".into(), "/b.ts".into()],
            fs: Arc::new(bundled::wrap_fs(fs)),
            lint: Arc::clone(&lint),
            type_errors: TypeErrors::default(),
            suppress_program_diagnostics: false,
        })
        .unwrap();
        let program = projects.0.project_collection
            .get_default_project(&tspath::Path::new("/a.ts"))
            .unwrap()
            .get_program()
            .unwrap();
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
            file_names: names.iter().map(|name| (*name).into()).collect(),
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

    fn discover(files: &[(&str, &str)], names: &[&str]) -> LinterResult {
        run_linter(&RunLinterOptions {
            current_directory: "/repo".into(),
            file_names: names.iter().map(|name| (*name).into()).collect(),
            fs: Arc::new(bundled::wrap_fs(vfstest::from_map(
                files.iter().copied(),
                true,
            ))),
            lint: config(names, true),
            type_errors: TypeErrors {
                report_syntactic: true,
                report_semantic: true,
            },
            suppress_program_diagnostics: false,
        })
        .unwrap()
    }

    #[test]
    fn discovery_selects_nearest_ancestor_and_inferred_projects() {
        let files = [
            ("/repo/tsconfig.json", r#"{"include":["packages/**/*.ts"]}"#),
            (
                "/repo/packages/cli/tsconfig.json",
                r#"{"files":["kept.ts"]}"#,
            ),
            ("/repo/packages/cli/kept.ts", "export {};"),
            ("/repo/packages/cli/excluded.ts", "export {};"),
            ("/repo/outside.ts", "Promise.resolve();"),
        ];
        for (name, expected) in [
            (
                "/repo/packages/cli/kept.ts",
                "/repo/packages/cli/tsconfig.json",
            ),
            ("/repo/packages/cli/excluded.ts", "/repo/tsconfig.json"),
            ("/repo/outside.ts", ""),
        ] {
            let result = discover(&files, &[name]);
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            let program = program_for_file(&result, name);
            assert_eq!(program.command_line().config_name(), expected);
            assert_eq!(
                result.lint.diagnostics.len(),
                usize::from(expected.is_empty())
            );
        }
        let result = discover(
            &files,
            &[
                "/repo/packages/cli/kept.ts",
                "/repo/packages/cli/excluded.ts",
                "/repo/outside.ts",
            ],
        );
        assert!(result.diagnostics.is_empty());
        for (name, expected) in [
            (
                "/repo/packages/cli/kept.ts",
                "/repo/packages/cli/tsconfig.json",
            ),
            ("/repo/packages/cli/excluded.ts", "/repo/tsconfig.json"),
            ("/repo/outside.ts", ""),
        ] {
            assert_eq!(
                program_for_file(&result, name).command_line().config_name(),
                expected
            );
        }
        assert_eq!(result.lint.diagnostics.len(), 1);
    }

    #[test]
    fn discovery_follows_solution_references_and_handles_cycles() {
        let files = [
            (
                "/repo/tsconfig.json",
                r#"{"files":[],"references":[{"path":"./project"}]}"#,
            ),
            (
                "/repo/project/tsconfig.json",
                r#"{"compilerOptions":{"target":"es2022"},"files":["../shared.ts"]}"#,
            ),
            ("/repo/shared.ts", "export {}; Promise.resolve();"),
        ];
        let result = discover(&files, &["/repo/shared.ts"]);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            program_for_file(&result, "/repo/shared.ts")
                .command_line()
                .config_name(),
            "/repo/project/tsconfig.json"
        );
        assert_eq!(result.lint.diagnostics.len(), 1);

        let result = discover(
            &[
                (
                    "/repo/tsconfig.json",
                    r#"{"files":[],"references":[{"path":"./project"}]}"#,
                ),
                (
                    "/repo/project/tsconfig.json",
                    r#"{"files":[],"references":[{"path":".."}]}"#,
                ),
                ("/repo/outside.ts", "Promise.resolve();"),
            ],
            &["/repo/outside.ts"],
        );
        assert_eq!(
            program_for_file(&result, "/repo/outside.ts")
                .command_line()
                .config_name(),
            ""
        );
        assert_eq!(result.lint.diagnostics.len(), 1);
    }

    #[test]
    fn imported_files_use_the_discovered_config_and_only_requested_files_are_linted() {
        let result = discover(
            &[
                (
                    "/repo/tsconfig.json",
                    r#"{"compilerOptions":{"target":"es2022","strictNullChecks":false},"files":["index.ts"],"exclude":["imported.ts"]}"#,
                ),
                ("/repo/index.ts", "import './imported'; Promise.resolve();"),
                (
                    "/repo/imported.ts",
                    "export const value: string = null; Promise.resolve();",
                ),
            ],
            &["/repo/imported.ts", "/repo/imported.ts"],
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let program = program_for_file(&result, "/repo/imported.ts");
        assert_eq!(program.command_line().config_name(), "/repo/tsconfig.json");
        assert_eq!(program.command_line().file_names(), ["/repo/index.ts"]);
        assert!(program.get_source_file("/repo/index.ts").is_some());
        assert_eq!(result.lint.diagnostics.len(), 1);
        assert_eq!(
            result.lint.diagnostics[0].source_file.file_name(),
            "/repo/imported.ts"
        );
        assert_eq!(result.lint.timings[0].calls, 1);
    }

    #[test]
    fn discovery_normalizes_relative_paths_and_case() {
        let names = vec!["src/../src/INDEX.ts".into()];
        let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(
            [
                (
                    "/repo/tsconfig.json",
                    r#"{"compilerOptions":{"target":"es2022"},"files":["src/index.ts"]}"#,
                ),
                ("/repo/src/index.ts", "Promise.resolve();"),
            ],
            false,
        )));
        let lint = Arc::new(
            LintConfig::new(
                &[FileConfig {
                    file_paths: names.clone(),
                    rules: vec![RequestedRule {
                        name: NO_FLOATING_PROMISES.into(),
                        options: Value::Null,
                    }],
                }],
                "/repo",
                false,
                Fixes::default(),
                true,
            )
            .unwrap(),
        );
        let result = run_linter(&RunLinterOptions {
            current_directory: "/repo".into(),
            file_names: names,
            fs,
            lint,
            type_errors: TypeErrors::default(),
            suppress_program_diagnostics: false,
        })
        .unwrap();
        assert!(result.diagnostics.is_empty());
        assert_eq!(result.lint.diagnostics.len(), 1);
        assert_eq!(result.lint.timings[0].calls, 1);
    }
}
