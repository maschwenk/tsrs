// TSRS_DERIVED_VARIANCE (crates/tsrs_checker/src/relater_derived.rs) relates a derived generic to a reference to its
// generic base by variances. Each case below is a disagreement between that shortcut and the structural comparison
// that a guard closes; with the switch on, tsrs must still print what tsgo-ref prints (expected.txt), and shadow mode
// must not report a disagreement (it exits 7 when it does). Removing a guard fails its case.

use std::path::Path;
use std::process::Command;

fn run(case: &str, mode: &str) -> (String, Option<i32>) {
    run_without(case, mode, "")
}

fn run_without(case: &str, mode: &str, no_guard: &str) -> (String, Option<i32>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions").join(case);
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .current_dir(&dir)
        .args(["-p", ".", "--pretty", "false", "--singleThreaded"])
        .env("TSRS_DERIVED_VARIANCE", mode)
        // The cases are small interfaces; the default only tries targets with 16 or more properties.
        .env("TSRS_DERIVED_VARIANCE_MIN_MEMBERS", "0")
        .env_remove("TSRS_CHECKER_ASSIGNMENT")
        .env_remove("TSRS_DERIVED_VARIANCE_RELIABLE")
        .env_remove("TSRS_DERIVED_VARIANCE_LOG")
        .env("TSRS_DERIVED_VARIANCE_NO_GUARD", no_guard)
        .output()
        .expect("run tsrs");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code())
}

// (case, guard that closes it). Guards 1-3: notes/perf-derived-variance.md; 4-6: notes/fuzz-derived-variance.md.
// Guards 1-3 are not switchable; for 4-6 the test also checks that the case fails without its guard.
const CASES: &[(&str, u8)] = &[
    ("derived-variance-this-type", 1),
    ("derived-variance-any-conditional", 3),
    ("derived-variance-any-keyof", 4),
    ("derived-variance-keyof-optional", 4),
    ("derived-variance-mutual-conditional", 4),
    ("derived-variance-this-conditional", 4),
    ("derived-variance-any-template", 4),
    ("derived-variance-intersection", 4),
    ("derived-variance-generic-rest", 4),
    ("derived-variance-annotation", 5),
    ("derived-variance-void-arity", 6),
    ("derived-variance-void-rest", 6),
];

#[test]
fn derived_variance_guards() {
    for &(case, guard) in CASES {
        let expected = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions").join(case).join("expected.txt")).unwrap();
        for mode in ["off", "on", "shadow"] {
            let (stdout, code) = run(case, mode);
            assert_eq!(stdout, expected, "{case} with TSRS_DERIVED_VARIANCE={mode}");
            assert_ne!(code, Some(7), "{case}: shadow mode found a disagreement");
        }
        if guard >= 4 {
            let g = guard.to_string();
            assert_eq!(run_without(case, "shadow", &g).1, Some(7), "{case}: no disagreement without guard {guard}");
            assert_ne!(run_without(case, "on", &g).0, expected, "{case}: the error is kept without guard {guard}");
        }
    }
}

// Open, cosmetic (notes/fuzz-derived-variance.md): with the switch on, an error involving an expanding recursive
// generic is elaborated one level deeper than tsgo does; the reported errors are the same. Shadow mode reports no
// disagreement and prints tsgo-ref's output.
#[test]
fn derived_variance_open_elaboration() {
    let case = "derived-variance-expanding-elaboration";
    let expected = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions").join(case).join("expected.txt")).unwrap();
    for mode in ["off", "shadow"] {
        let (stdout, code) = run(case, mode);
        assert_eq!(stdout, expected, "{case} with TSRS_DERIVED_VARIANCE={mode}");
        assert_ne!(code, Some(7), "{case}: shadow mode found a disagreement");
    }
}
