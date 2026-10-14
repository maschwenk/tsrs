// tsrs-only watch behavior, in scenarios whose every edit must give the outputs and diagnostics of a fresh build.
//
// `-b --watch` keeps each task's incremental program between cycles, and reparses only the changed file when one file
// of a program changed (buildtask.rs `old_watch_state`, `single_changed_file`). Each of its scenarios has an edit that
// one of the conditions of `single_changed_file` must turn into a full build. Content mappers keep transforming in
// later cycles. `--watch` ignores the events of its own outputs. The signature job, which the harness otherwise skips,
// spares the first edit of a file the recheck of its importers in both modes.

use rustc_hash::FxHashMap;

use super::{run_scenario, scenario, scenarioEdit, scenarioFile, scenarioOp, LATE_EVENT_WRITE};

const LIB_FILE: &str = "/home/src/tslibs/TS/Lib/lib.esnext.full.d.ts";
const LIB: &str = "/// <reference no-default-lib=\"true\"/>\ninterface Boolean {}\ninterface Function {}\ninterface CallableFunction {}\ninterface NewableFunction {}\ninterface IArguments {}\ninterface Number {}\ninterface Object {}\ninterface RegExp {}\ninterface String {}\ninterface Array<T> { length: number; [n: number]: T; }\ninterface ReadonlyArray<T> {}\n";
const ROOT: &str = "/home/src/workspaces/solution";
// --runExternalCode runs the content mappers (the in-process test mappers, under test).
const BUILD_APP: &[&str] = &["--b", "app", "-w", "--runExternalCode"];
const CORE_CONFIG: &str = r#"{ "compilerOptions": { "composite": true, "target": "esnext" } }"#;
const APP_CONFIG: &str = r#"{ "compilerOptions": { "composite": true, "target": "esnext" }, "references": [{ "path": "../core" }] }"#;

#[test]
fn one_file_edits_build_what_a_fresh_build_does() {
    let main = |rest: &str| format!("import {{ f }} from \"../core/a\";\n{rest}");
    let other = |rest: &str| format!("import {{ x }} from \"./main\";\n{rest}");
    let baseline = run(
        "one-file-edits",
        BUILD_APP,
        &[
            ("core/tsconfig.json", CORE_CONFIG),
            ("core/a.ts", "export function f() { return 1; }\n"),
            ("app/tsconfig.json", APP_CONFIG),
            ("app/main.ts", &main("export const x = f();\n")),
            ("app/other.ts", &other("export const y: number = x;\n")),
        ],
        vec![
            ("comment", vec![write("app/main.ts", &main("export const x = f();\n// comment\n"))]),
            ("shape change", vec![write("app/main.ts", &main("export const x = String(f());\n"))]),
            ("fix", vec![write("app/other.ts", &other("export const y: string = x;\n"))]),
            ("syntax error", vec![write("app/main.ts", &main("export const x = ;\n"))]),
            ("syntax fixed", vec![write("app/main.ts", &main("export const x = f();\n"))]),
            ("upstream shape change", vec![write("core/a.ts", "export function f() { return 's'; }\n")]),
            ("edit after the upstream build", vec![write("app/main.ts", &main("export const x: number = f();\n"))]),
            // The upstream file is not a file of the app program, which has its declaration output instead.
            (
                "upstream and app edit",
                vec![write("core/a.ts", "export function f() { return 1; }\n"), write("app/main.ts", &main("export const x: number = f();\n// again\n"))],
            ),
            (
                "two files",
                vec![write("app/main.ts", &main("export const x = 's';\n")), write("app/other.ts", &other("export const y: number = x;\n// other\n"))],
            ),
            ("new file", vec![write("app/added.ts", "import { y } from \"./other\";\nexport const z: boolean = y;\n")]),
            ("delete", vec![delete("app/added.ts")]),
        ],
    );
    assert!(baseline.contains("TS2322"), "the scenario reports type errors\n{baseline}");
}

// The app lists its files: a new file that resolves an import is not an input of the task, but its event comes in
// the same cycle as the edit of a program file.
#[test]
fn a_path_module_resolution_could_find_builds_the_program_again() {
    // An outDir keeps the declaration output of a deleted file from resolving its imports.
    let config = r#"{ "compilerOptions": { "composite": true, "target": "esnext", "outDir": "out" }, "files": ["main.ts", "other.ts"] }"#;
    let main = |rest: &str| format!("import {{ g }} from \"./gen\";\n{rest}");
    let baseline = run(
        "resolution",
        BUILD_APP,
        &[
            ("app/tsconfig.json", config),
            ("app/main.ts", &main("export const x: number = g;\n")),
            ("app/other.ts", "export const y = 1;\n"),
            ("app/node_modules/pkg/package.json", r#"{ "name": "pkg", "types": "index.d.ts" }"#),
            ("app/node_modules/pkg/index.d.ts", "export declare const p: number;\n"),
            ("app/node_modules/pkg/other.d.ts", "export declare const p: string;\n"),
        ],
        vec![
            ("comment", vec![write("app/other.ts", "export const y = 1;\n// comment\n")]),
            ("new module and edit", vec![write("app/gen.ts", "export const g = 's';\n"), write("app/other.ts", "export const y = 2;\n")]),
            ("import a package", vec![write("app/other.ts", "import { p } from \"pkg\";\nexport const y: number = p;\n")]),
            (
                "package.json and edit",
                vec![
                    write("app/node_modules/pkg/package.json", r#"{ "name": "pkg", "types": "other.d.ts" }"#),
                    write("app/main.ts", &main("export const x: number = g;\n// comment\n")),
                ],
            ),
            ("delete an imported file", vec![delete("app/gen.ts")]),
            ("delete a root file", vec![delete("app/other.ts")]),
        ],
    );
    assert!(baseline.contains("TS2322"), "the scenario reports type errors\n{baseline}");
}

// The upstream's changed file has an extension module resolution does not look for, and is not a file of the app
// program, whose declaration file of the upstream changes in the same cycle. Its declaration output, next to it, is not
// an input of the next cycles.
#[test]
fn an_upstream_build_builds_the_program_again() {
    let core_config = r#"{ "compilerOptions": { "composite": true, "target": "esnext" }, "contentMappers": [{ "package": "mapper", "extensions": [".vue"] }] }"#;
    let baseline = run(
        "upstream",
        BUILD_APP,
        &[
            ("core/tsconfig.json", core_config),
            ("core/node_modules/mapper/package.json", r#"{ "name": "mapper", "version": "1.0.0", "typescript": { "contentMapper": { "exec": ["verbatim-mapper"] } } }"#),
            ("core/v.vue", "export const v = 1;\n"),
            ("core/a.ts", "import { v } from \"./v.vue\";\nexport function f() { return v; }\n"),
            ("app/tsconfig.json", APP_CONFIG),
            ("app/main.ts", "import { f } from \"../core/a\";\nexport const x: number = f();\n"),
        ],
        vec![
            ("comment", vec![write("app/main.ts", "import { f } from \"../core/a\";\nexport const x: number = f();\n// comment\n")]),
            (
                "upstream shape change and edit",
                vec![
                    write("core/v.vue", "export const v = 's';\n"),
                    write("app/main.ts", "import { f } from \"../core/a\";\nexport const x: number = f();\n// again\n"),
                ],
            ),
            // The task's config is parsed again with the new file; the mapper keeps transforming.
            ("new upstream file", vec![write("core/b.ts", "export const b = 1;\n")]),
            ("mapped file edit", vec![write("core/v.vue", "export const v = 2;\n")]),
        ],
    );
    assert!(baseline.contains("TS2322"), "the scenario reports type errors\n{baseline}");
}

// Every task reads a file through one cache, so the time it was read at says only that some task has that version.
// Here a file of both projects is saved while a cycle runs: the full build of `a` reads the new version, `b` does not
// build, and the file's event comes with the next edit. That event still changes `b`.
#[test]
fn a_file_another_project_read_again_still_changes_this_one() {
    let config = r#"{ "compilerOptions": { "incremental": true, "target": "esnext", "noEmit": true } }"#;
    let a_main = |rest: &str| format!("import {{ v }} from \"../shared/f\";\nexport const y = v;\n{rest}");
    let baseline = run(
        "shared-file",
        &["--b", "a", "b", "-w"],
        &[
            ("shared/f.ts", "export const v: number = 1;\n"),
            ("a/tsconfig.json", config),
            ("a/main.ts", &a_main("")),
            ("b/tsconfig.json", config),
            ("b/main.ts", "import { v } from \"../shared/f\";\nexport const x: number = v;\n"),
        ],
        vec![
            ("new file in a, f saved late", vec![write("a/added.ts", "export const z = 1;\n"), write_late_event("shared/f.ts", "export const v: string = 's';\n")]),
            ("edit in a", vec![write("a/main.ts", &a_main("// comment\n"))]),
        ],
    );
    assert!(baseline.contains("TS2322"), "b reports the change of f\n{baseline}");
}

// `--watch` reparses a content-mapped file in every full build. The snapshot of a full build keeps the diagnostics of
// an unchanged file from the one before, which refer to the previous build's version of the file; that version stays
// as long as they do (watcher.rs `live_files`), and a later cycle reports them.
#[test]
fn kept_diagnostics_of_a_content_mapped_file_outlive_a_full_build() {
    let config = r#"{ "compilerOptions": { "composite": true, "target": "esnext" }, "contentMappers": [{ "package": "mapper", "extensions": [".vue"] }] }"#;
    let main = |rest: &str| format!("import {{ v }} from \"./v.vue\";\n{rest}");
    let baseline = run(
        "content-mapped-diagnostics",
        &["-w", "-p", "app", "--runExternalCode"],
        &[
            ("app/tsconfig.json", config),
            ("app/node_modules/mapper/package.json", r#"{ "name": "mapper", "version": "1.0.0", "typescript": { "contentMapper": { "exec": ["verbatim-mapper"] } } }"#),
            ("app/v.vue", "export const v: number = 's';\n"),
            ("app/main.ts", &main("export const x = v;\n")),
        ],
        vec![
            ("new file", vec![write("app/b.ts", "export const b = 1;\n")]),
            ("comment", vec![write("app/main.ts", &main("export const x = v;\n// comment\n"))]),
            ("another new file", vec![write("app/c.ts", "export const c = 1;\n")]),
            ("comment again", vec![write("app/main.ts", &main("export const x = v;\n// again\n"))]),
        ],
    );
    assert!(baseline.contains("TS2322"), "the scenario reports the error of v.vue\n{baseline}");
}

// The events of the files a build wrote (outputs next to their sources, the build info) come after its report; they
// start no cycle, and the next edit is built as usual.
#[test]
fn the_watch_ignores_its_own_outputs() {
    let config = r#"{ "compilerOptions": { "composite": true, "target": "esnext" } }"#;
    let main = |rest: &str| format!("import {{ v }} from \"./v\";\n{rest}");
    let baseline = run(
        "own-outputs",
        &["-w", "-p", "app"],
        &[("app/tsconfig.json", config), ("app/v.ts", "export const v = 1;\n"), ("app/main.ts", &main("export const x: number = v;\n"))],
        vec![
            ("edit", vec![write("app/v.ts", "export const v = 's';\n")]),
            ("the edit's outputs", vec![touch("app/v.js"), touch("app/v.d.ts"), touch("app/main.js"), touch("app/tsconfig.tsbuildinfo")]),
            ("fix", vec![write("app/v.ts", "export const v = 2;\n")]),
        ],
    );
    let cycles: Vec<&str> = baseline.split("\nEdit [").skip(1).collect();
    assert!(cycles[0].contains("File change detected") && cycles[0].contains("TS2322"), "{baseline}");
    assert!(!cycles[1].contains("File change detected"), "the outputs start no cycle\n{baseline}");
    assert!(cycles[2].contains("File change detected") && cycles[2].contains("Found 0 errors"), "{baseline}");
}

// A file with JSX (`react-jsx`) or under `importHelpers` has synthetic imports, of the JSX runtime and the helpers
// module; an edit that keeps them reuses the program (`can_keep_synthetic_imports`), one that changes the runtime
// (a pragma) builds it again.
#[test]
fn edits_of_files_with_synthetic_imports_build_what_a_fresh_build_does() {
    let config = r#"{ "compilerOptions": { "composite": true, "target": "esnext", "module": "commonjs", "jsx": "react-jsx", "importHelpers": true, "experimentalDecorators": true } }"#;
    let runtime = "export declare function jsx(type: any, props: { id?: number }): void;\nexport declare function jsxs(type: any, props: any): void;\ndeclare global { namespace JSX { interface IntrinsicElements { div: { id?: number }; } } }\n";
    let view = |rest: &str| format!("export const e = <div id={{1}} />;\n{rest}");
    let decorated = |rest: &str| format!("declare function d(target: any): void;\n{rest}");
    for args in [BUILD_APP, &["-w", "-p", "app", "--runExternalCode"]] {
        let baseline = run(
            "synthetic-imports",
            args,
            &[
                ("app/tsconfig.json", config),
                ("app/view.tsx", &view("")),
                ("app/dec.ts", &decorated("export class A {}\n")),
                ("app/node_modules/react/package.json", r#"{ "name": "react", "types": "index.d.ts" }"#),
                ("app/node_modules/react/index.d.ts", "export {};\n"),
                ("app/node_modules/react/jsx-runtime.d.ts", runtime),
            ],
            vec![
                ("comment", vec![write("app/view.tsx", &view("// comment\n"))]),
                ("type error", vec![write("app/view.tsx", "export const e = <div id=\"s\" />;\n")]),
                ("fix", vec![write("app/view.tsx", &view("// fixed\n"))]),
                ("missing runtime", vec![write("app/view.tsx", &format!("/** @jsxImportSource preact */\n{}", view("")))]),
                ("runtime back", vec![write("app/view.tsx", &view("// back\n"))]),
                ("helper without tslib", vec![write("app/dec.ts", &decorated("@d\nexport class A {}\n"))]),
                ("helper again", vec![write("app/dec.ts", &decorated("@d\nexport class A {}\n// again\n"))]),
                // `-b --watch` does not see a new module of no task's inputs alone (nor without this change).
                (
                    "tslib and edit",
                    vec![
                        write("app/node_modules/tslib/package.json", r#"{ "name": "tslib", "types": "tslib.d.ts" }"#),
                        write("app/node_modules/tslib/tslib.d.ts", "export declare function __decorate(decorators: any, target: any, key?: any, desc?: any): any;\n"),
                        write("app/dec.ts", &decorated("@d\nexport class A {}\n// tslib\n")),
                    ],
                ),
                ("helper with tslib", vec![write("app/dec.ts", &decorated("@d\nexport class A {}\n// again\n"))]),
            ],
        );
        for code in ["TS2322", "TS2875", "TS2354"] {
            assert!(baseline.contains(code), "{args:?}: the scenario reports {code}\n{baseline}");
        }
    }
}

// The initial build of a project without declaration emit stores each file's version as a stand-in for its declaration
// signature, so the first edit of a file counts as a change of its declarations and rechecks the files importing it.
// The signature job (watcher.rs and orchestrator.rs `start_signature_job`) computes the real signatures after each
// cycle, in both modes; after it, a comment edit rechecks the edited file only.
#[test]
fn a_first_edit_after_the_signature_job_rechecks_only_the_edited_file() {
    let config = r#"{ "compilerOptions": { "incremental": true, "target": "esnext" } }"#;
    let files = [
        ("app/tsconfig.json", config),
        ("app/a.ts", "export function f() { return 1; }\n"),
        ("app/b.ts", "import { f } from \"./a\";\nexport const g = f();\n"),
        ("app/c.ts", "import { g } from \"./b\";\nexport const h: number = g;\n"),
        ("app/d.ts", "import { h } from \"./c\";\nexport const k = h;\n"),
        ("app/e.ts", "import { k } from \"./d\";\nexport const m: number = k;\n"),
    ];
    for (mode, args) in [("watch", &["-w", "-p", "app"][..]), ("build", &["--b", "app", "-w"][..])] {
        for signature_job in [true, false] {
            let name = format!("signature-job-{mode}-{signature_job}");
            let edits = vec![
                ("comment", vec![write("app/a.ts", "export function f() { return 1; }\n// comment\n")]),
                // The cycle stores stand-in signatures for the files two imports or more away (d.ts and e.ts).
                ("shape change", vec![write("app/a.ts", "export function f() { return 's'; }\n")]),
                ("comment after a cycle", vec![write("app/d.ts", "import { h } from \"./c\";\nexport const k = h;\n// comment\n")]),
            ];
            let baseline = run_with(&name, args, &files, edits, signature_job);
            for (index, caption, edited, importer) in [(0, "comment", "a.ts", "b.ts"), (2, "comment after a cycle", "d.ts", "e.ts")] {
                let start = baseline.find(&format!("\nEdit [{index}]:: {caption}\n")).unwrap();
                let edit = &baseline[start..baseline[start + 1..].find("\nEdit [").map_or(baseline.len(), |end| start + 1 + end)];
                let checked = |file: &str| edit.contains(&format!("*refresh*    {ROOT}/app/{file}\n"));
                assert!(checked(edited), "{name}: the edited file is checked\n{edit}");
                assert_eq!(checked(importer), !signature_job, "{name}: the importer is checked only without the job\n{edit}");
            }
        }
    }
}

// Runs `tsgo <args>` on `files` (paths under ROOT), asserts that no edit differs from a fresh build, and returns the
// baseline. An edit that only touches files starts no cycle: its output is empty, unlike a fresh build's.
fn run(name: &str, args: &[&str], files: &[(&str, &str)], edits: Vec<(&str, Vec<scenarioOp>)>) -> String {
    run_with(name, args, files, edits, false)
}

// `run`, with the watch's signature job after each cycle when `signature_job` is set.
fn run_with(name: &str, args: &[&str], files: &[(&str, &str)], edits: Vec<(&str, Vec<scenarioOp>)>, signature_job: bool) -> String {
    let mut files: Vec<scenarioFile> = files.iter().map(|(path, content)| file(&format!("{ROOT}/{path}"), content)).collect();
    files.push(file(LIB_FILE, LIB));
    let mut all_ops: Vec<scenarioOp> = Vec::new();
    let edits = edits
        .into_iter()
        .map(|(caption, ops)| {
            // A fresh build has the files the edits wrote; a touch changes no content (and its file may be an output).
            all_ops.extend(ops.iter().filter(|op| op.op != "touch").map(clone_op));
            let expected_diff = if ops.iter().all(|op| op.op == "touch") {
                NO_CYCLE
            } else if ops.iter().any(|op| op.op == LATE_EVENT_WRITE) {
                LATE_EVENT
            } else {
                ""
            };
            scenarioEdit { caption: caption.to_string(), args: None, expected_diff: expected_diff.to_string(), ops, fresh_ops: all_ops.iter().map(clone_op).collect() }
        })
        .collect();
    let test = scenario {
        baseline: format!("watch_test/{name}"),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        cwd: ROOT.to_string(),
        env: FxHashMap::default(),
        output_is_tty: None,
        ignore_case: false,
        windows_style_root: String::new(),
        files,
        default_libs: vec![LIB_FILE.to_string()],
        edits,
        signature_job,
    };
    let baseline = run_scenario(&test);
    let diffs = baseline.matches("\nDiff:: ").count();
    let expected = baseline.matches(&format!("\nDiff:: {NO_CYCLE}\n")).count() + baseline.matches(&format!("\nDiff:: {LATE_EVENT}\n")).count();
    assert_eq!(diffs, expected, "{name}: an edit differs from a fresh build\n{baseline}");
    baseline
}

const NO_CYCLE: &str = "No cycle runs";
const LATE_EVENT: &str = "A file's event comes with the next edit";

fn file(path: &str, content: &str) -> scenarioFile {
    scenarioFile { path: path.to_string(), content: content.to_string(), symlink: String::new() }
}

fn write_late_event(path: &str, content: &str) -> scenarioOp {
    scenarioOp { op: LATE_EVENT_WRITE.to_string(), path: format!("{ROOT}/{path}"), content: content.to_string(), target: String::new() }
}

fn write(path: &str, content: &str) -> scenarioOp {
    scenarioOp { op: "write".to_string(), path: format!("{ROOT}/{path}"), content: content.to_string(), target: String::new() }
}

fn delete(path: &str) -> scenarioOp {
    scenarioOp { op: "delete".to_string(), path: format!("{ROOT}/{path}"), content: String::new(), target: String::new() }
}

fn touch(path: &str) -> scenarioOp {
    scenarioOp { op: "touch".to_string(), path: format!("{ROOT}/{path}"), content: String::new(), target: String::new() }
}

fn clone_op(op: &scenarioOp) -> scenarioOp {
    scenarioOp { op: op.op.clone(), path: op.path.clone(), content: op.content.clone(), target: op.target.clone() }
}
