// `--maxMemory` (tsrs_compiler checkerpool.rs `retire_checker`, notes/mem-recycle-checkers.md): above the target,
// the `--noEmit` type-check pass retires its largest checker and checks the rest of that checker's files with a fresh
// one. With a 1 MiB target and no minimum checker size (TSRS_RETIRE_MIN=0) checkers are retired after nearly every
// file. Each case of
// testdata/regressions must still print tsgo-ref's output (expected.txt), also in poison mode (TSRS_ARENA_POISON=1:
// a retired checker's region is filled with 0xA5 and kept, so any later read of it crashes), and with the relation
// caches emptied after every file.

use std::path::Path;
use std::process::Command;

fn run(dir: &Path, checkers: &str, poison: bool, extended: bool) -> String {
    run_with(dir, checkers, poison, extended, None)
}

// `relation_limit`: TSRS_RELATION_CACHE_LIMIT (Some(0): the relation caches are emptied after every file).
fn run_with(dir: &Path, checkers: &str, poison: bool, extended: bool, relation_limit: Option<&str>) -> String {
    run_target(dir, checkers, poison, extended, relation_limit, Some("1M"), None)
}

// `max_memory`: `--maxMemory <size>`, if any. `available`: TSRS_AVAILABLE_MEMORY, which the default target (no
// `--maxMemory`) takes as the machine's available memory.
fn run_target(
    dir: &Path,
    checkers: &str,
    poison: bool,
    extended: bool,
    relation_limit: Option<&str>,
    max_memory: Option<&str>,
    available: Option<&str>,
) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    cmd.current_dir(dir).args(["-p", ".", "--pretty", "false", "--checkers", checkers]);
    cmd.env("TSRS_RETIRE_MIN", "0");
    if extended {
        cmd.arg("--extendedDiagnostics");
    }
    let vars = ["TSRS_CHECKER_ASSIGNMENT", "TSRS_FILE_TIMES", "TSRS_ASSIGNMENT_STATS", "TSRS_HEAP_CENSUS", "TSRS_CENSUS"];
    for var in vars.into_iter().chain(["TSRS_MAX_MEMORY", "TSRS_AVAILABLE_MEMORY"]) {
        cmd.env_remove(var);
    }
    if let Some(size) = max_memory {
        cmd.args(["--maxMemory", size]);
    }
    if let Some(size) = available {
        cmd.env("TSRS_AVAILABLE_MEMORY", size);
    }
    match relation_limit {
        Some(limit) => cmd.env("TSRS_RELATION_CACHE_LIMIT", limit),
        None => cmd.env_remove("TSRS_RELATION_CACHE_LIMIT"),
    };
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
        assert_eq!(run_with(dir, "2", false, false, Some("0")), expected, "{} relation caches emptied after every file", dir.display());
        let stats = run(dir, "2", false, true);
        retired += stats.lines().find_map(|l| l.strip_prefix("Checkers: retired:")).map_or(0, |n| n.trim().parse::<usize>().unwrap());
    }
    // Most cases are `--noEmit` checks that retire a checker at least once.
    assert!(retired >= cases.len(), "only {retired} checkers retired over {} cases", cases.len());
}

// The default target (no `--maxMemory`): on a machine with almost no memory available it applies and retires checkers,
// with the same diagnostics; `--maxMemory 0` turns it off. A regression that stops deriving it, or that ignores `0`,
// shows here as no retirement, or as one.
#[test]
fn default_target_applies_on_a_machine_short_of_memory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions");
    let mut cases: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.join("expected.txt").is_file()).collect();
    cases.sort();
    let retired = |out: &str| out.lines().find_map(|l| l.strip_prefix("Checkers: retired:")).map(|n| n.trim().parse::<usize>().unwrap());
    let (mut with_default, mut with_zero) = (0, 0);
    for dir in &cases {
        let expected = std::fs::read_to_string(dir.join("expected.txt")).unwrap();
        assert_eq!(run_target(dir, "2", false, false, None, None, Some("1M")), expected, "{}", dir.display());
        with_default += retired(&run_target(dir, "2", false, true, None, None, Some("1M"))).unwrap_or(0);
        with_zero += retired(&run_target(dir, "2", false, true, None, Some("0"), Some("1M"))).map_or(0, |_| 1);
    }
    assert!(with_default >= cases.len(), "only {with_default} checkers retired over {} cases", cases.len());
    assert_eq!(with_zero, 0, "--maxMemory 0 still applied a target");
}
