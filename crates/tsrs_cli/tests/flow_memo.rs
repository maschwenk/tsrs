// The flow memo (crates/tsrs_checker/src/flowmemo.rs) must not change any output. Each directory under
// testdata/flow-memo is a project whose expected.txt is tsgo's output for it (the reference binary built from the
// pinned commit); tsrs must print exactly that with the memo on, off, and in shadow mode (which walks every memo hit
// again and panics on a different answer). The cases cover the memo's hazards: the depth limit (TS2563) reached past
// memo entries and after a walk used one, loops and their in-process types, try/finally, inlined condition aliases,
// answers that depend on the reference's position or its property symbol, fresh types and circularities.

use std::path::Path;
use std::process::Command;

fn run(case: &Path, mode: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .args(["-p", ".", "--pretty", "false"])
        .current_dir(case)
        .env("TSRS_FLOW_MEMO", mode)
        .output()
        .unwrap();
    assert!(out.status.code().is_some_and(|c| c <= 2), "{} ({mode}): {}", case.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn flow_memo_cases_match_tsgo() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/flow-memo");
    let mut cases: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    cases.sort();
    assert!(cases.len() >= 9, "testdata/flow-memo has {} cases", cases.len());
    for case in &cases {
        let expected = std::fs::read_to_string(case.join("expected.txt")).unwrap();
        for mode in ["1", "0", "shadow"] {
            assert_eq!(run(case, mode), expected, "{} with TSRS_FLOW_MEMO={mode}", case.display());
        }
    }
}
