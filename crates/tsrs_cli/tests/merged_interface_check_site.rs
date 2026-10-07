// testdata/regressions/merged-interface-check-site: the global interface `Merged` is declared in a.ts, b.ts, and
// twice each in c.ts and d.d.ts, and extends `Left` (a.ts) and `Right` (b.ts), whose `kind` properties differ, so the
// merged interface fails checkInheritedPropertiesAreIdentical (TS2320). Go runs that check once per checker, at the
// first declaration the checker visits, so the files that report depend on which files share a checker (tsgo-ref:
// a.ts only at 1-2 checkers, a.ts and b.ts at 4, all four files at 8; expected.txt is its output at 8). By default
// tsrs reports at the first declaration in each file whatever the assignment and visit order; `--checkerAssignment
// go` keeps Go's behaviour (notes/fix-history-dependent-diagnostics.md).

use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> String {
    let case = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions/merged-interface-check-site");
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .current_dir(case)
        .args(["-p", ".", "--pretty", "false"])
        .args(args)
        .env_remove("TSRS_CHECKER_ASSIGNMENT")
        .env_remove("TSRS_HISTORY")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn merged_interface_reports_in_every_declaring_file() {
    let expected = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions/merged-interface-check-site/expected.txt"),
    )
    .unwrap();
    assert_eq!(run(&["--singleThreaded"]), expected, "--singleThreaded");
    for checkers in ["1", "2", "3", "4"] {
        assert_eq!(run(&["--checkers", checkers]), expected, "--checkers {checkers}");
        for seed in 1..=8 {
            let assignment = format!("random:{seed}");
            assert_eq!(run(&["--checkers", checkers, "--checkerAssignment", &assignment]), expected, "--checkers {checkers} {assignment}");
        }
    }
    // Go's history: one checker reports at the first declaration it visits only, as tsgo-ref does.
    let go_one = run(&["--checkers", "1", "--checkerAssignment", "go"]);
    assert_eq!(go_one, expected.lines().take(2).map(|l| format!("{l}\n")).collect::<String>());
    assert_eq!(run(&["--checkers", "8", "--checkerAssignment", "go"]), expected);
}
