// TSRS_DERIVED_VARIANCE (crates/tsrs_checker/src/relater_derived.rs) relates a derived generic to a reference to its
// generic base by variances. Each case below is a disagreement between that shortcut and the structural comparison
// that a guard closes; with the switch on, tsrs must still print what tsgo-ref prints (expected.txt), and shadow mode
// must not report a disagreement (it exits 7 when it does). Removing a guard fails its case.

use std::path::Path;
use std::process::Command;

fn run(case: &str, mode: &str) -> (String, Option<i32>) {
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
        .output()
        .expect("run tsrs");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code())
}

#[test]
fn derived_variance_guards() {
    for case in ["derived-variance-this-type", "derived-variance-any-conditional"] {
        let expected = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions").join(case).join("expected.txt")).unwrap();
        for mode in ["off", "on", "shadow"] {
            let (stdout, code) = run(case, mode);
            assert_eq!(stdout, expected, "{case} with TSRS_DERIVED_VARIANCE={mode}");
            assert_ne!(code, Some(7), "{case}: shadow mode found a disagreement");
        }
    }
}
