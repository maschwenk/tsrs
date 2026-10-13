// A global interface declared in several files whose merged checks fail: Go runs them once per checker, at the first
// declaration the checker visits, so the files that report TS2320 / TS2430 depend on which files share a checker
// (tsgo-ref on merged-interface-check-site: a.ts only at 1-2 checkers, a.ts and b.ts at 3-4, all four files at 8). By
// default tsrs reports them where tsgo does at its default four checkers, whatever its own checker count and assignment,
// and still reports each file's index-constraint errors (merged-interface-extends-site: TS2411 in c.ts and d.ts, which
// do not report the TS2430). expected.txt is tsgo-ref's default output; `--checkerAssignment go` keeps Go's behaviour
// (notes/fix-history-dependent-diagnostics.md).

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
    assert_eq!(out.status.code(), Some(2), "{dir:?} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
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
        std::fs::write(dir.join("extra.ts"), (0..100).map(|i| format!("export const e0_{i}: number = {i};\n")).collect::<String>()).unwrap();
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
