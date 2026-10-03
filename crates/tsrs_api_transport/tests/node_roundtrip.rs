// Runs tests/node/roundtrip.test.mjs (the pinned Node sync and async clients from ts-ref) against
// examples/transport_test_server. This is a mandatory parity gate: it FAILS (never skips) when `node`
// (>= 22.6, type stripping) or the pinned microsoft/TypeScript checkout (`<workspace>/ts-ref` or
// `$TSRS_TS_REF`, with packages/typescript/{src,lib,vendor}) is missing. For the Rust-only tests run
// `cargo test -p tsrs_api_transport --lib --test conn --test go_frames --test strictjson_oracle`.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn pinned_node_clients_round_trip() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ts_ref = std::env::var_os("TSRS_TS_REF").map(PathBuf::from).unwrap_or_else(|| manifest.join("../../ts-ref"));
    let client = ts_ref.join("packages/typescript/src/api/sync/client.ts");
    let node_ok = Command::new("node").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    let commit = "b85298b6a81f772d080b0455de0ca9d744cd6fd6";
    assert!(
        node_ok,
        "missing dependency: `node` (>= 22.6) is not on PATH. Install Node 24, or run the Rust-only tests with \
         `cargo test -p tsrs_api_transport --lib --test conn --test go_frames --test strictjson_oracle`."
    );
    assert!(
        client.exists() && ts_ref.join("packages/typescript/vendor/vscode-jsonrpc").exists(),
        "missing dependency: pinned TypeScript client not found at {}. Check out microsoft/TypeScript {commit} into \
         <workspace>/ts-ref (or set TSRS_TS_REF), e.g.\n  git -C ts-ref init && git -C ts-ref remote add origin \
         https://github.com/microsoft/TypeScript.git && git -C ts-ref fetch --depth 1 --filter=blob:none origin {commit} && \
         git -C ts-ref sparse-checkout set packages/typescript/src packages/typescript/lib packages/typescript/vendor && \
         git -C ts-ref checkout FETCH_HEAD",
        client.display()
    );
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
        output.status.success() && fail == Some(0) && pass.is_some_and(|p| p >= 15) && pass == tests,
        "node round trip failed: tests={tests:?} pass={pass:?} fail={fail:?}\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("pinned node clients: {} tests passed", pass.unwrap());
}
