// Runs tests/node/roundtrip.test.mjs (the pinned Node sync and async clients from ts-ref) against
// examples/transport_test_server. Needs `node` (>= 22.6, type stripping) and the pinned
// microsoft/TypeScript checkout in `<workspace>/ts-ref` or `$TSRS_TS_REF`. When either is missing the
// test reports the skip on stderr and passes; set TSRS_REQUIRE_NODE_TESTS=1 to make that a failure.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn pinned_node_clients_round_trip() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ts_ref = std::env::var_os("TSRS_TS_REF").map(PathBuf::from).unwrap_or_else(|| manifest.join("../../ts-ref"));
    let require = std::env::var_os("TSRS_REQUIRE_NODE_TESTS").is_some();
    let client = ts_ref.join("packages/typescript/src/api/sync/client.ts");
    let node_ok = Command::new("node").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    if !client.exists() || !node_ok {
        let why = if node_ok { format!("pinned client not found at {}", client.display()) } else { "node not found".to_string() };
        assert!(!require, "node round-trip required but skipped: {why}");
        eprintln!("SKIPPED pinned_node_clients_round_trip: {why}");
        return;
    }
    // cargo test builds examples next to the test's deps directory.
    let exe = std::env::current_exe().unwrap();
    let server = exe.parent().unwrap().parent().unwrap().join("examples").join(format!("transport_test_server{}", std::env::consts::EXE_SUFFIX));
    assert!(server.exists(), "example server not built at {}", server.display());
    let output = Command::new("node")
        .args(["--conditions=@typescript/source", "--test", "--test-timeout=60000", "--test-reporter=tap"])
        .arg(manifest.join("tests/node/roundtrip.test.mjs"))
        .env("TS_REF", &ts_ref)
        .env("SERVER", &server)
        .output()
        .expect("run node");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let count = |key: &str| stdout.lines().find_map(|l| l.strip_prefix(&format!("# {key} "))).and_then(|v| v.trim().parse::<u32>().ok());
    let (pass, fail, tests) = (count("pass"), count("fail"), count("tests"));
    assert!(
        output.status.success() && fail == Some(0) && pass.is_some_and(|p| p >= 14) && pass == tests,
        "node round trip failed: tests={tests:?} pass={pass:?} fail={fail:?}\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("pinned node clients: {} tests passed", pass.unwrap());
}
