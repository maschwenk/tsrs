// testdata/regressions/union-too-complex-cross-product (issue #218; the reporter's reproduction, PR #228): relating an
// arrow function to `GetModel` needs the write constraint of `ModelMap[T]`, the intersection of three unions of 50
// object types, a cross product of 125,000 constituents, over the TS2590 limit. x/a.ts evaluates it first under
// `// @ts-ignore`; x/zz.ts evaluates it again without one. Go reports TS2590 only the first time a checker evaluates
// the type, because the error type it returns is cached one level up (the intersection split, the relation), so
// tsgo-ref reports x/zz.ts at 3 checkers and nothing at 1 or 2, and tsrs with work stealing reported it in about half
// of the runs. By default tsrs no longer caches a result computed while a TS2590 was reported, so every file that
// evaluates the type reports it, at the innermost of its nested sites, whatever the assignment; `--checkerAssignment
// go` keeps Go's behaviour (notes/open-history-dependence.md section 4).

use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> (bool, String) {
    let case = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions/union-too-complex-cross-product");
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .current_dir(case)
        .args(["-p", ".", "--pretty", "false"])
        .args(args)
        .env_remove("TSRS_CHECKER_ASSIGNMENT")
        .env_remove("TSRS_HISTORY")
        .output()
        .unwrap();
    assert!(out.status.code().is_some_and(|c| c <= 2), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    (out.status.success(), String::from_utf8(out.stdout).unwrap())
}

#[test]
fn too_complex_union_reports_at_every_site_whatever_the_assignment() {
    let expected = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions/union-too-complex-cross-product/expected.txt"),
    )
    .unwrap();
    assert_eq!(run(&["--singleThreaded"]), (false, expected.clone()), "--singleThreaded");
    for checkers in ["1", "2", "3", "4"] {
        assert_eq!(run(&["--checkers", checkers]), (false, expected.clone()), "--checkers {checkers}");
        for seed in 1..=8 {
            let assignment = format!("random:{seed}");
            assert_eq!(run(&["--checkers", checkers, "--checkerAssignment", &assignment]), (false, expected.clone()), "--checkers {checkers} {assignment}");
        }
    }
    // Go's history: the checker that evaluated the type under x/a.ts's `@ts-ignore` reports nothing at x/zz.ts, as
    // tsgo-ref does with one checker.
    assert_eq!(run(&["--checkers", "1", "--checkerAssignment", "go"]), (true, String::new()), "--checkerAssignment go");
}
