// tsrs-only: `-b --watch` keeps each task's incremental program between cycles, and reparses only the changed file
// when one file of a program changed (buildtask.rs `old_watch_state`, `single_changed_file`). After every edit, the
// outputs and diagnostics must be those of a fresh build. Each scenario has an edit that one of the conditions of
// `single_changed_file` must turn into a full build.

use rustc_hash::FxHashMap;

use super::{run_scenario, scenario, scenarioEdit, scenarioFile, scenarioOp};

const LIB_FILE: &str = "/home/src/tslibs/TS/Lib/lib.esnext.full.d.ts";
const LIB: &str = "/// <reference no-default-lib=\"true\"/>\ninterface Boolean {}\ninterface Function {}\ninterface CallableFunction {}\ninterface NewableFunction {}\ninterface IArguments {}\ninterface Number {}\ninterface Object {}\ninterface RegExp {}\ninterface String {}\ninterface Array<T> { length: number; [n: number]: T; }\ninterface ReadonlyArray<T> {}\n";
const ROOT: &str = "/home/src/workspaces/solution";
const CORE_CONFIG: &str = r#"{ "compilerOptions": { "composite": true, "target": "esnext" } }"#;
const APP_CONFIG: &str = r#"{ "compilerOptions": { "composite": true, "target": "esnext" }, "references": [{ "path": "../core" }] }"#;

#[test]
fn one_file_edits_build_what_a_fresh_build_does() {
    let main = |rest: &str| format!("import {{ f }} from \"../core/a\";\n{rest}");
    let other = |rest: &str| format!("import {{ x }} from \"./main\";\n{rest}");
    let baseline = run(
        "one-file-edits",
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

// Runs `tsgo -b app -w` on `files` (paths under ROOT), asserts that no edit differs from a fresh build, and returns
// the baseline.
fn run(name: &str, files: &[(&str, &str)], edits: Vec<(&str, Vec<scenarioOp>)>) -> String {
    let mut files: Vec<scenarioFile> = files.iter().map(|(path, content)| file(&format!("{ROOT}/{path}"), content)).collect();
    files.push(file(LIB_FILE, LIB));
    let mut all_ops: Vec<scenarioOp> = Vec::new();
    let edits = edits
        .into_iter()
        .map(|(caption, ops)| {
            all_ops.extend(ops.iter().map(clone_op));
            scenarioEdit { caption: caption.to_string(), args: None, expected_diff: String::new(), ops, fresh_ops: all_ops.iter().map(clone_op).collect() }
        })
        .collect();
    let test = scenario {
        baseline: format!("buildwatch_test/{name}"),
        args: ["--b", "app", "-w"].map(String::from).to_vec(),
        cwd: ROOT.to_string(),
        env: FxHashMap::default(),
        output_is_tty: None,
        ignore_case: false,
        windows_style_root: String::new(),
        files,
        default_libs: vec![LIB_FILE.to_string()],
        edits,
    };
    let baseline = run_scenario(&test);
    assert!(!baseline.contains("\nDiff:: "), "{name}: an edit differs from a fresh build\n{baseline}");
    baseline
}

fn file(path: &str, content: &str) -> scenarioFile {
    scenarioFile { path: path.to_string(), content: content.to_string(), symlink: String::new() }
}

fn write(path: &str, content: &str) -> scenarioOp {
    scenarioOp { op: "write".to_string(), path: format!("{ROOT}/{path}"), content: content.to_string(), target: String::new() }
}

fn delete(path: &str) -> scenarioOp {
    scenarioOp { op: "delete".to_string(), path: format!("{ROOT}/{path}"), content: String::new(), target: String::new() }
}

fn clone_op(op: &scenarioOp) -> scenarioOp {
    scenarioOp { op: op.op.clone(), path: op.path.clone(), content: op.content.clone(), target: op.target.clone() }
}
