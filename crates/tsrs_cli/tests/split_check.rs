// Checking a declaration file in pieces on several checkers (crates/tsrs_compiler/src/splitcheck.rs) must not change
// any output. Each directory under testdata/split-check is a project whose expected.txt is tsgo's output for it (the
// reference binary built from the pinned commit); tsrs must print exactly that at four checkers with the split off, at
// its default, and with every checked declaration file forced into 2, 3, 5 or 9 pieces, in shadow mode too (the owner
// checks the other checkers' pieces again and panics if the split would report something else).
// `declaration-errors` has errors in most statements of its declaration file, including ones that span statements
// (merged interfaces with different type parameters, redeclared variables, overloads); `ambient-statements` has two
// executable statements at the top level of a declaration file, of which only the first may report TS1036, so the
// file must not be split at all.

use std::path::Path;
use std::process::Command;

fn run(case: &Path, mode: &str, stats: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .args(["-p", ".", "--pretty", "false", "--checkers", "4"])
        .current_dir(case)
        .env("TSRS_SPLIT_FILES", format!("{mode},stats:{}", stats.display()))
        .env_remove("TSRS_CHECKER_ASSIGNMENT")
        .output()
        .unwrap();
    assert!(out.status.code().is_some_and(|c| c <= 2), "{} ({mode}): {}", case.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn split_check_cases_match_tsgo() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/split-check");
    let mut cases: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    cases.sort();
    assert!(cases.len() >= 2, "testdata/split-check has {} cases", cases.len());
    let stats = std::env::temp_dir().join(format!("tsrs-split-check-{}.txt", std::process::id()));
    for case in &cases {
        let expected = std::fs::read_to_string(case.join("expected.txt")).unwrap();
        let _ = std::fs::remove_file(&stats);
        for mode in ["0", "on", "force:2,shadow", "force:3,shadow", "force:5", "force:9,shadow"] {
            assert_eq!(run(case, mode, &stats), expected, "{} with TSRS_SPLIT_FILES={mode}", case.display());
        }
        let split = std::fs::read_to_string(&stats).unwrap_or_default();
        if case.ends_with("ambient-statements") {
            assert!(split.is_empty(), "{}: a declaration file with executable statements was split: {split}", case.display());
        } else {
            assert!(split.lines().count() >= 4, "{}: the forced modes did not split: {split:?}", case.display());
        }
    }
    let _ = std::fs::remove_file(&stats);
}
