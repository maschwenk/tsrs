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
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .current_dir(case(name))
        .args(["-p", ".", "--pretty", "false"])
        .args(args)
        .env_remove("TSRS_CHECKER_ASSIGNMENT")
        .env_remove("TSRS_HISTORY")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{name} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
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
