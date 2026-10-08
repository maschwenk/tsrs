// testdata/regressions/deferred-type-argument-check: checking a.ts computes the type of b.ts's `make`, which checks the
// arrow function's signature, including the constraint of `Box<number>` (TS2344, located in b.ts). A checker that has
// not started b.ts leaves that check to its own check of b.ts (`defer_type_argument_constraints`), and a checker that
// never checks b.ts never runs it. The error must be reported once whichever checker checks which file in which order:
// if the deferred check were not run when b.ts is checked, the error would be lost, because the signature is checked
// once per checker.

use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> String {
    let case = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions/deferred-type-argument-check");
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
fn deferred_constraint_check_reports_in_its_file() {
    let expected = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions/deferred-type-argument-check/expected.txt"),
    )
    .unwrap();
    assert_eq!(run(&["--singleThreaded"]), expected, "--singleThreaded");
    for checkers in ["1", "2", "3"] {
        assert_eq!(run(&["--checkers", checkers]), expected, "--checkers {checkers}");
        for seed in 1..=8 {
            let assignment = format!("random:{seed}");
            assert_eq!(run(&["--checkers", checkers, "--checkerAssignment", &assignment]), expected, "--checkers {checkers} {assignment}");
        }
    }
    assert_eq!(run(&["--checkers", "2", "--checkerAssignment", "go"]), expected, "go");
}
