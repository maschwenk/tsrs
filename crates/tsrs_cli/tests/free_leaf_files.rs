// The CLI's `--noEmit` check frees the tree of each leaf file once it is checked (TSRS_FREE_LEAVES, tsrs_compiler
// fileregions.rs, notes/mem-free-leaf-files.md). In testdata/regressions/leaf-alternative-containers, leaf.ts is a
// leaf that is checked and freed before a.ts (one checker, program order: q.ts, m.ts, leaf.ts, a.ts). a.ts's error
// prints `{ a: Q; }` with the object literal as the enclosing declaration; `Q` is not imported there, so the node
// builder searches every module of the program for one that re-exports it (getAlternativeContainingModules),
// which must pass over the freed leaf.ts (`Checker::is_unreadable_check_leaf`). Without that guard the poison run
// (TSRS_ARENA_POISON=1: freed memory is filled with 0xA5 and kept) crashes, failing this test.

use std::path::Path;
use std::process::Command;

fn case_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions/leaf-alternative-containers")
}

fn run(free_leaves: &str, poison: bool) -> (String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    cmd.current_dir(case_dir()).args(["-p", ".", "--pretty", "false", "--singleThreaded"]).env("TSRS_FREE_LEAVES", free_leaves);
    for var in ["TSRS_CHECKER_ASSIGNMENT", "TSRS_FILE_TIMES", "TSRS_ASSIGNMENT_STATS", "TSRS_HEAP_CENSUS", "TSRS_CENSUS"] {
        cmd.env_remove(var);
    }
    if poison {
        cmd.env("TSRS_ARENA_POISON", "1");
    } else {
        cmd.env_remove("TSRS_ARENA_POISON");
    }
    let out = cmd.output().expect("run tsrs");
    assert_eq!(out.status.code(), Some(2), "TSRS_FREE_LEAVES={free_leaves} poison={poison}: {}", String::from_utf8_lossy(&out.stderr));
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn freed_leaf_is_skipped_by_the_alternative_container_search() {
    let expected = std::fs::read_to_string(case_dir().join("expected.txt")).unwrap();
    for (free_leaves, poison) in [("1", false), ("0", false), ("keep", false), ("stats", false), ("stats", true)] {
        let (stdout, stderr) = run(free_leaves, poison);
        assert_eq!(stdout, expected, "TSRS_FREE_LEAVES={free_leaves} poison={poison}");
        if free_leaves == "stats" {
            // leaf.ts and a.ts are leaves (nothing imports them); q.ts and m.ts are imported.
            assert!(stderr.contains("leaf files: 2 of 4 checked files") && stderr.contains("freed 2 "), "{stderr}");
        }
    }
}
