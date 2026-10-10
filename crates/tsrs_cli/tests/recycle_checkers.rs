// `--checkerMemoryBudget` (tsrs_compiler checkerpool.rs `retire_checker`, notes/mem-recycle-checkers.md): the
// `--noEmit` type-check pass retires a checker whose region holds more than the budget and checks the rest of its
// files with a fresh one. With a 1 MiB budget nearly every checker is retired after its first file. Each case of
// testdata/regressions must still print tsgo-ref's output (expected.txt), also in poison mode (TSRS_ARENA_POISON=1:
// a retired checker's region is filled with 0xA5 and kept, so any later read of it crashes).

use std::path::Path;
use std::process::Command;

fn run(dir: &Path, checkers: &str, poison: bool, extended: bool) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    cmd.current_dir(dir).args(["-p", ".", "--pretty", "false", "--checkers", checkers, "--checkerMemoryBudget", "1"]);
    if extended {
        cmd.arg("--extendedDiagnostics");
    }
    for var in ["TSRS_CHECKER_ASSIGNMENT", "TSRS_FILE_TIMES", "TSRS_ASSIGNMENT_STATS", "TSRS_HEAP_CENSUS", "TSRS_CENSUS", "TSRS_CHECKER_MEMORY_BUDGET"] {
        cmd.env_remove(var);
    }
    if poison {
        cmd.env("TSRS_ARENA_POISON", "1");
    } else {
        cmd.env_remove("TSRS_ARENA_POISON");
    }
    let out = cmd.output().expect("run tsrs");
    assert!(matches!(out.status.code(), Some(0 | 2)), "{}: {}", dir.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn retired_checkers_report_what_one_checker_reports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions");
    let mut cases: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.join("expected.txt").is_file()).collect();
    cases.sort();
    let mut retired = 0;
    for dir in &cases {
        let expected = std::fs::read_to_string(dir.join("expected.txt")).unwrap();
        for (checkers, poison) in [("2", true), ("3", false)] {
            assert_eq!(run(dir, checkers, poison, false), expected, "{} --checkers {checkers} poison={poison}", dir.display());
        }
        let stats = run(dir, "2", false, true);
        retired += stats.lines().find_map(|l| l.strip_prefix("Checkers: retired:")).map_or(0, |n| n.trim().parse::<usize>().unwrap());
    }
    // Most cases are `--noEmit` checks that retire both checkers at least once.
    assert!(retired >= cases.len(), "only {retired} checkers retired over {} cases", cases.len());
}
