// Skipping flow walks that nothing can narrow (crates/tsrs_checker/src/flowskip.rs, the binder's flow name index in
// crates/tsrs_binder/src/flownames.rs) must not change any output. Each directory under testdata/flow-skip is a project
// whose expected.txt is tsgo's output for it (the reference binary built from the pinned commit), and each narrows a
// reference through a node the index has to see without the reference being named there: condition aliases (same
// scope, enclosing scope, module level after use, destructured, transitive, in another file of a global script),
// `in` and `hasOwnProperty` on the parent path, assertion calls, an immediately invoked function's arguments, loops,
// `for...in`, assignments, and walks that continue into enclosing functions. tsrs must print exactly that with the
// skip on, off, and in shadow mode (which walks every skipped reference anyway and panics on a different answer).
// Without the alias closure, the `in` and `hasOwnProperty` keys, the call mentions, the effects-signature peek, the
// loop and immediately-invoked spans, the `for...in` expression, the assignment targets or the continuation into
// enclosing functions, at least one case fails (checked by disabling each). testdata/flow-memo/depth-limit covers the
// node limit that keeps TS2563 where tsgo reports it.

use std::path::Path;
use std::process::Command;

fn run(case: &Path, mode: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .args(["-p", ".", "--pretty", "false"])
        .current_dir(case)
        .env("TSRS_FLOW_SKIP", mode)
        .output()
        .unwrap();
    assert!(out.status.code().is_some_and(|c| c <= 2), "{} ({mode}): {}", case.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn flow_skip_cases_match_tsgo() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/flow-skip");
    let mut cases: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    cases.sort();
    assert!(cases.len() >= 8, "testdata/flow-skip has {} cases", cases.len());
    for case in &cases {
        let expected = std::fs::read_to_string(case.join("expected.txt")).unwrap();
        for mode in ["1", "0", "shadow"] {
            assert_eq!(run(case, mode), expected, "{} with TSRS_FLOW_SKIP={mode}", case.display());
        }
    }
}
