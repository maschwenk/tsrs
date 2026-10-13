// A global interface declared in several files whose merged checks fail: Go runs them once per checker, at the first
// declaration the checker visits, so the files that report TS2320 / TS2430 depend on which files share a checker
// (tsgo-ref on merged-interface-check-site: a.ts only at 1-2 checkers, a.ts and b.ts at 3-4, all four files at 8). By
// default tsrs reports them where tsgo does at its default four checkers, whatever its own checker count and assignment,
// and still reports each file's index-constraint errors (merged-interface-extends-site: TS2411 in c.ts and d.ts, which
// do not report the TS2430). expected.txt is tsgo-ref's default output; `--checkerAssignment go` keeps Go's behaviour
// (notes/fix-history-dependent-diagnostics.md).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn case(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions").join(name)
}

fn run(name: &str, args: &[&str]) -> String {
    run_in(&case(name), args)
}

fn run_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .current_dir(dir)
        .args(["-p", ".", "--pretty", "false"])
        .args(args)
        .env_remove("TSRS_CHECKER_ASSIGNMENT")
        .env_remove("TSRS_HISTORY")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{} {args:?}: {}", dir.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

// Adding an unrelated module changes the virtual check slots. Reusing the unchanged interface files' diagnostics
// used to retain a TS2320 that a clean build suppressed, and could also retain an empty cache where an error appears.
#[test]
fn incremental_merged_interface_sites_match_clean_builds() {
    for mode in [&["--singleThreaded"][..], &["--checkers", "4"][..]] {
        let dir = std::env::temp_dir().join(format!("tsrs-merged-incremental-{}-{}", std::process::id(), mode[0]));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["bases.ts", "a.ts", "b.ts", "c.ts", "d.d.ts"] {
            std::fs::copy(case("merged-interface-check-site").join(name), dir.join(name)).unwrap();
        }
        let roots = r#""bases.ts","a.ts","b.ts","c.ts","d.d.ts""#;
        let extra_first = format!(r#""extra.ts",{roots}"#);
        let config = |files: &str| {
            std::fs::write(dir.join("tsconfig.json"), format!(r#"{{"compilerOptions":{{"strict":true,"noEmit":true,"incremental":true,"target":"es2020","types":[],"skipDefaultLibCheck":true}},"files":[{files}]}}"#)).unwrap();
        };
        let clean = || {
            std::fs::remove_file(dir.join("tsconfig.tsbuildinfo")).unwrap();
            run_in(&dir, mode)
        };
        config(roots);
        let initial = run_in(&dir, mode);
        std::fs::write(dir.join("extra.ts"), (0..100).fold(String::new(), |mut out, i| {
            writeln!(out, "export const e0_{i}: number = {i};").unwrap();
            out
        })).unwrap();
        config(&extra_first);
        let added = run_in(&dir, mode);
        let added_clean = clean();
        assert_ne!(initial, added_clean, "fixture must change reporting sites");
        assert_eq!(added, added_clean, "adding unrelated module: {mode:?}");
        // The same files and contents can move the reporting site when their root order changes.
        config(&format!(r#"{roots},"extra.ts""#));
        let reordered = run_in(&dir, mode);
        let reordered_clean = clean();
        assert_ne!(added_clean, reordered_clean, "root order must change reporting sites");
        assert_eq!(reordered, reordered_clean, "reordering roots: {mode:?}");
        config(&extra_first);
        assert_eq!(run_in(&dir, mode), clean(), "restoring root order: {mode:?}");
        config(roots);
        std::fs::remove_file(dir.join("extra.ts")).unwrap();
        assert_eq!(run_in(&dir, mode), clean(), "removing unrelated module: {mode:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn incremental_merged_interface_sites_follow_history_changes() {
    let dir = std::env::temp_dir().join(format!("tsrs-merged-history-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["bases.ts", "a.ts", "b.ts", "c.ts", "d.d.ts"] {
        std::fs::copy(case("merged-interface-check-site").join(name), dir.join(name)).unwrap();
    }
    let config = std::fs::read_to_string(case("merged-interface-check-site").join("tsconfig.json")).unwrap();
    std::fs::write(dir.join("tsconfig.json"), config.replace("\"strict\":true", "\"strict\":true,\"incremental\":true")).unwrap();
    let info = dir.join("tsconfig.tsbuildinfo");
    let default = ["--checkers", "8"];
    let go = ["--checkers", "8", "--checkerAssignment", "go"];
    let unchanged = |args: &[&str], expected: &str| {
        let bytes = std::fs::read(&info).unwrap();
        let modified = std::fs::metadata(&info).unwrap().modified().unwrap();
        assert_eq!(run_in(&dir, args), expected);
        assert_eq!(std::fs::read(&info).unwrap(), bytes, "unchanged history must preserve build info");
        assert_eq!(std::fs::metadata(&info).unwrap().modified().unwrap(), modified, "unchanged history must not rewrite build info");
    };
    let initial = run_in(&dir, &default);
    assert_eq!(initial.lines().filter(|line| line.contains("TS2320")).count(), 2);
    let default_info = std::fs::read_to_string(&info).unwrap();
    unchanged(&default, &initial);
    let switched = run_in(&dir, &go);
    let go_info = std::fs::read_to_string(&info).unwrap();
    assert!(!go_info.contains("tsrsCanonicalCheckSignature"), "Go history must rewrite tagged default caches");
    std::fs::remove_file(&info).unwrap();
    let fresh_go = run_in(&dir, &go);
    assert_eq!(fresh_go.lines().filter(|line| line.contains("TS2320")).count(), 4);
    assert_eq!(switched, fresh_go, "default-to-Go must preserve the full ordered diagnostics");
    assert_eq!(std::fs::read_to_string(&info).unwrap(), go_info);
    unchanged(&go, &fresh_go);

    let key = "\"tsrsCanonicalCheckSignature\":";
    let start = default_info.find(key).unwrap() + key.len();
    let end = start + default_info[start..].find(',').unwrap();
    for value in ["\"\"", "\"go4-v1:bad\"", "\"go4-v2:0123456789abcdef0123456789abcdef\"", "null", "123", "false", "{}", "[]"] {
        std::fs::write(&info, format!("{}{value}{}", &default_info[..start], &default_info[end..])).unwrap();
        assert_eq!(run_in(&dir, &go), fresh_go, "Go history must reject every present default tag: {value}");
        assert_eq!(std::fs::read_to_string(&info).unwrap(), go_info, "Go rewrite must omit the default tag: {value}");
        unchanged(&go, &fresh_go);
    }
    assert_eq!(run_in(&dir, &default), initial, "Go-to-default must refresh the reporting sites");
    assert_eq!(std::fs::read_to_string(&info).unwrap(), default_info);
    unchanged(&default, &initial);
    std::fs::remove_dir_all(dir).unwrap();
}

// Package exports can change the resolved import multigraph without changing source text, root order or the
// deduplicated reference sets in build info. Cached TS2320 presence and absence must follow the new report slots.
#[test]
fn incremental_merged_interface_sites_follow_package_export_aliases() {
    for mode in [&["--singleThreaded"][..], &["--checkers", "4"][..], &["--checkers", "8", "--checkerAssignment", "random:3"][..]] {
        let dir = std::env::temp_dir().join(format!("tsrs-merged-exports-{}-{}", std::process::id(), mode.join("-")));
        std::fs::create_dir_all(dir.join("node_modules/p")).unwrap();
        let files = ["node_modules/p/a.ts", "node_modules/p/b.ts", "c.ts", "d.ts", "e.ts", "f.ts", "g.ts", "h.ts"];
        for (i, name) in files.iter().enumerate() {
            let imports = if i == 4 { "import 'p/a';\nimport 'p/b';\nimport 'p/alias';\n" } else { "import 'missing/a';\nimport 'missing/b';\nimport 'missing/c';\n" };
            let (body, nodes) = match i {
                0 => ("declare global { interface Array<T> { length: number; [n: number]: T } interface Boolean {} interface CallableFunction {} interface Function {} interface IArguments {} interface NewableFunction {} interface Number {} interface Object {} interface RegExp {} interface String {} interface Left { kind: 'l' } interface Right { kind: 'r' } interface M extends Left, Right {} }\n", 64),
                4 => ("declare global { interface M { extra?: number } }\n", 18),
                _ => ("", 8),
            };
            let mut text = format!("{imports}{body}");
            // Equal node-plus-text weights and import counts leave only the resolved edge multiplicities different.
            let padding = (100 - nodes) * 100 - text.len() - 4;
            write!(text, "/*{}*/", "x".repeat(padding)).unwrap();
            std::fs::write(dir.join(name), text).unwrap();
        }
        std::fs::write(dir.join("tsconfig.json"), r#"{"compilerOptions":{"noLib":true,"noEmit":true,"incremental":true,"module":"nodenext"},"files":["node_modules/p/a.ts","node_modules/p/b.ts","c.ts","d.ts","e.ts","f.ts","g.ts","h.ts"]}"#).unwrap();
        let package = |target: &str| {
            std::fs::write(dir.join("node_modules/p/package.json"), format!(r#"{{"name":"p","version":"1.0.0","type":"module","exports":{{"./a":"./a.ts","./b":"./b.ts","./alias":"./{target}.ts"}}}}"#)).unwrap();
        };
        let clean = || {
            std::fs::remove_file(dir.join("tsconfig.tsbuildinfo")).unwrap();
            run_in(&dir, mode)
        };
        package("a");
        let initial = run_in(&dir, mode);
        assert_eq!(initial.lines().filter(|line| line.contains("TS2320")).count(), 1);
        let build_info = std::fs::read(dir.join("tsconfig.tsbuildinfo")).unwrap();
        let modified = std::fs::metadata(dir.join("tsconfig.tsbuildinfo")).unwrap().modified().unwrap();
        assert_eq!(run_in(&dir, mode), initial, "unchanged build: {mode:?}");
        assert_eq!(std::fs::read(dir.join("tsconfig.tsbuildinfo")).unwrap(), build_info, "unchanged build info: {mode:?}");
        assert_eq!(std::fs::metadata(dir.join("tsconfig.tsbuildinfo")).unwrap().modified().unwrap(), modified, "unchanged build info timestamp: {mode:?}");
        package("b");
        let changed = run_in(&dir, mode);
        let fresh = clean();
        assert_eq!(fresh.lines().filter(|line| line.contains("TS2320")).count(), 2);
        assert_eq!(changed, fresh, "alias a -> b must discard cached absence: {mode:?}");
        package("a");
        assert_eq!(run_in(&dir, mode), clean(), "alias b -> a must discard cached error: {mode:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn merged_interface_reports_where_tsgo_does() {
    for name in ["merged-interface-check-site", "merged-interface-extends-site"] {
        let expected = std::fs::read_to_string(case(name).join("expected.txt")).unwrap();
        assert_eq!(run(name, &["--singleThreaded"]), expected, "{name} --singleThreaded");
        for checkers in ["1", "2", "3", "4", "8"] {
            assert_eq!(run(name, &["--checkers", checkers]), expected, "{name} --checkers {checkers}");
            for seed in 1..=8 {
                let assignment = format!("random:{seed}");
                assert_eq!(run(name, &["--checkers", checkers, "--checkerAssignment", &assignment]), expected, "{name} --checkers {checkers} {assignment}");
            }
        }
        assert_eq!(run(name, &["--checkers", "4", "--checkerAssignment", "go"]), expected, "{name} go at 4 checkers");
    }
    // Go's history at one checker: only the first declaration reports, as tsgo-ref does.
    let go_one = run("merged-interface-check-site", &["--checkers", "1", "--checkerAssignment", "go"]);
    assert_eq!(go_one.lines().filter(|l| l.contains("TS2320")).collect::<Vec<_>>(), ["a.ts(1,11): error TS2320: Interface 'Merged' cannot simultaneously extend types 'Left' and 'Right'."]);
    let go_eight = run("merged-interface-check-site", &["--checkers", "8", "--checkerAssignment", "go"]);
    assert_eq!(go_eight.lines().filter(|l| l.contains("TS2320")).count(), 4);
}
