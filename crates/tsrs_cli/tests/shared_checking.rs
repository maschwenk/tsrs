//! The batch executor and payload interner must preserve the sequential checker's observable behavior.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions")
}

fn run(case: &Path, engine: &str, args: &[&str], poison: bool, lazy: bool) -> Output {
    run_with_env(case, engine, args, poison, lazy, &[])
}

fn run_with_env(case: &Path, engine: &str, args: &[&str], poison: bool, lazy: bool, environment: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    command.current_dir(case).args(["-p", ".", "--noEmit", "--incremental", "false", "--pretty", "false"]);
    for variable in [
        "TSRS_CHECKER_ASSIGNMENT", "TSRS_HISTORY", "TSRS_MAX_MEMORY", "TSRS_SHARED_GRAPH", "TSRS_ASSIGNMENT_STATS",
        "TSRS_FILE_TIMES", "TSRS_CHECKER_COST_CACHE", "TSRS_MEM_SPLIT", "TSRS_HEAP_CENSUS", "TSRS_CENSUS",
        "TSRS_WORK_CENSUS", "TSRS_FLOW_MEMO_STATS",
    ] {
        command.env_remove(variable);
    }
    command.env("TSRS_CHECK_ENGINE", engine).env("TSRS_LAZY_MEMBERS", if lazy { "1" } else { "0" });
    if poison {
        command.env("TSRS_ARENA_POISON", "1").env("TSRS_CHECK_SHARED", "1");
    } else {
        command.env_remove("TSRS_ARENA_POISON").env_remove("TSRS_CHECK_SHARED");
    }
    let output = command.envs(environment.iter().copied()).args(args).output().expect("run tsrs");
    assert!(matches!(output.status.code(), Some(0 | 2)), "{} engine={engine} {args:?}: {}", case.display(), String::from_utf8_lossy(&output.stderr));
    output
}

#[test]
fn shared_batch_preserves_diagnostics_and_exit_status() {
    let mut cases: Vec<_> = std::fs::read_dir(root()).unwrap().map(|entry| entry.unwrap().path())
        .filter(|path| path.join("expected.txt").is_file()).collect();
    cases.sort();
    assert!(!cases.is_empty());
    for case in cases {
        for (args, poison, lazy) in [
            (&["--singleThreaded"][..], false, true),
            (&["--checkers", "2"][..], true, true),
            (&["--checkers", "4"][..], false, false),
        ] {
            let legacy = run(&case, "legacy", args, poison, lazy);
            let shared = run(&case, "shared", args, poison, lazy);
            assert_eq!(shared.status.code(), legacy.status.code(), "{} {args:?}", case.display());
            assert_eq!(shared.stdout, legacy.stdout, "{} {args:?}: shared output\n{}", case.display(), String::from_utf8_lossy(&shared.stdout));
            assert_eq!(shared.stderr, legacy.stderr, "{} {args:?}", case.display());
        }
    }
}

#[test]
fn backend_selector_preserves_compatibility_routes() {
    let case = root().join("base-type-cycle-entry");
    let args = ["--checkers", "2", "--extendedDiagnostics"];
    let shared = run(&case, "shared", &args, false, true);
    let legacy = run(&case, "legacy", &args, false, true);
    assert!(String::from_utf8_lossy(&shared.stdout).contains("Check contexts:"));
    assert!(!String::from_utf8_lossy(&legacy.stdout).contains("Check contexts:"));

    for compatibility in [
        &["--checkerAssignment", "go"][..],
        &["--checkerAssignment", "locality"][..],
        &["--maxMemory", "1G"][..],
    ] {
        let args: Vec<_> = args.iter().chain(compatibility).copied().collect();
        let output = run(&case, "shared", &args, false, true);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Check contexts:"), "{compatibility:?}");
    }
}

#[test]
fn checker_reports_remain_available_after_checking() {
    let case = root().join("base-type-cycle-entry");
    for (variable, report) in [
        ("TSRS_HEAP_CENSUS", "heap census checker"),
        ("TSRS_FLOW_MEMO_STATS", "flow memo ("),
    ] {
        let output = run_with_env(&case, "shared", &["--checkers", "2", "--extendedDiagnostics"], false, true, &[(variable, "1")]);
        assert!(String::from_utf8_lossy(&output.stdout).contains("Check contexts:"), "{variable}");
        assert!(String::from_utf8_lossy(&output.stderr).contains(report), "{variable}: {}", String::from_utf8_lossy(&output.stderr));
    }
}
