// A Go program whose write to stdout fails with EPIPE dies of SIGPIPE (os/signal), so `tsgo -p . | head -1` exits with
// status 141 as soon as `head` is gone. tsrs ends the same way instead of finishing the check and exiting with 2.

#![cfg(unix)]

use std::fmt::Write as _;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

#[test]
fn closed_stdout_ends_the_process_with_sigpipe() {
    let dir = std::env::temp_dir().join(format!("tsrs-broken-pipe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tsconfig.json"), r#"{ "compilerOptions": { "noEmit": true }, "files": ["a.ts"] }"#).unwrap();
    // Far more diagnostics text than a pipe buffer holds.
    let source = (0..20_000).fold(String::new(), |mut out, i| {
        writeln!(out, "const v{i}: number = \"s\";").unwrap();
        out
    });
    std::fs::write(dir.join("a.ts"), source).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .args(["-p", ".", "--pretty", "false"])
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGPIPE), "{status:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
