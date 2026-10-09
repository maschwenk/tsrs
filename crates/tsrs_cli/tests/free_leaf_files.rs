// The CLI's `--noEmit` check frees the tree of each leaf file once it is checked (TSRS_FREE_LEAVES, tsrs_compiler
// fileregions.rs, notes/mem-free-leaf-files.md). In testdata/regressions/leaf-alternative-containers, leaf.test.ts is
// a leaf (predicted by its name) that is checked and freed before a.ts (one checker, program order: q.ts, m.ts,
// leaf.test.ts, a.ts). a.ts's error prints `{ a: Q; }` with the object literal as the enclosing declaration; `Q` is
// not imported there, so the node builder searches every module of the program for one that re-exports it
// (getAlternativeContainingModules), which must pass over the freed leaf.test.ts (`Checker::is_unreadable_check_leaf`).
// Without that guard the poison run (TSRS_ARENA_POISON=1: freed memory is filled with 0xA5 and kept) crashes, failing
// this test.

use std::path::Path;
use std::process::Command;

fn case_dir(case: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/regressions").join(case)
}

fn run(case: &str, free_leaves: &str, poison: bool) -> (String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    cmd.current_dir(case_dir(case)).args(["-p", ".", "--pretty", "false", "--singleThreaded"]).env("TSRS_FREE_LEAVES", free_leaves);
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
    let case = "leaf-alternative-containers";
    let expected = std::fs::read_to_string(case_dir(case).join("expected.txt")).unwrap();
    let cases = [("1", false), ("0", false), ("keep", false), ("stats", false), ("stats", true)];
    for (free_leaves, poison) in cases {
        let (stdout, stderr) = run(case, free_leaves, poison);
        assert_eq!(stdout, expected, "TSRS_FREE_LEAVES={free_leaves} poison={poison}");
        // leaf.test.ts and a.ts are leaves (nothing imports them); q.ts and m.ts are imported. Only leaf.test.ts is
        // predicted, so only it gets a region and is freed.
        if free_leaves == "stats" {
            assert!(stderr.contains("leaf files: 1 of 4 checked files (1 more not predicted") && stderr.contains("freed 1 "), "{stderr}");
        }
    }
}

// testdata/regressions/leaf-structural-instantiation: the freed leaf.test.ts instantiates G<{ a: number }> with an
// object literal type of its own; other.ts, checked afterwards by the same checker, instantiates G with a structurally
// identical type and prints it in an error. The checker's instantiation and relation caches are keyed by type ids,
// not structure, so other.ts never reaches the leaf's types or nodes: the poison run must print tsgo-ref's output.
#[test]
fn structurally_identical_instantiation_after_a_freed_leaf() {
    let case = "leaf-structural-instantiation";
    let expected = std::fs::read_to_string(case_dir(case).join("expected.txt")).unwrap();
    for (free_leaves, poison) in [("stats", false), ("stats", true), ("0", false)] {
        let (stdout, stderr) = run(case, free_leaves, poison);
        assert_eq!(stdout, expected, "TSRS_FREE_LEAVES={free_leaves} poison={poison}");
        if free_leaves == "stats" {
            // leaf.test.ts is freed; index.ts is a leaf too, but not predicted.
            assert!(stderr.contains("leaf files: 1 of 4 checked files (1 more not predicted") && stderr.contains("freed 1 "), "{stderr}");
        }
    }
}
