// lsp.go and api.go run their servers under signal.NotifyContext(ctx, os.Interrupt, syscall.SIGTERM), so a
// signal cancels the server's context instead of killing the process. The language server then exits with
// status 1 and "context canceled". The API server checks its context only before reading the next message:
// it answers that message (or sees EOF) and exits with status 0.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

#[test]
fn sigterm_cancels_the_language_server() {
    let mut child = spawn(&["--lsp", "-stdio"]);
    signal(&child, libc::SIGTERM);
    let status = wait(&mut child, Duration::from_secs(10)).expect("the language server did not exit");
    let mut stderr = String::new();
    child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    assert_eq!(status.code(), Some(1), "{status:?}");
    assert_eq!(stderr, "context canceled\n");
}

#[test]
fn sigterm_stops_the_api_server_after_the_next_message() {
    let mut child = spawn(&["--api"]);
    signal(&child, libc::SIGTERM);
    assert!(wait(&mut child, Duration::from_millis(500)).is_none(), "the API server exited before the next message");

    // A sync (MessagePack) request: [Request, method, payload].
    let (method, payload) = (b"bogusMethod", b"{}");
    let mut request = vec![0x93, 1, 0xC4, method.len() as u8];
    request.extend_from_slice(method);
    request.extend_from_slice(&[0xC4, payload.len() as u8]);
    request.extend_from_slice(payload);
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&request).unwrap();
    stdin.flush().unwrap();

    let status = wait(&mut child, Duration::from_secs(10)).expect("the API server kept running after the next message");
    drop(stdin);
    let mut stdout = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut stdout).unwrap();
    assert_eq!(status.code(), Some(0), "{status:?}");
    // [Error, method, message]
    assert_eq!(stdout[..4], [0x93, 5, 0xC4, method.len() as u8]);
}

#[test]
fn sigint_stops_the_async_api_server_at_eof() {
    let mut child = spawn(&["--api", "--async"]);
    signal(&child, libc::SIGINT);
    assert!(wait(&mut child, Duration::from_millis(500)).is_none(), "the API server exited before EOF");
    drop(child.stdin.take());
    let status = wait(&mut child, Duration::from_secs(10)).expect("the API server did not exit at EOF");
    assert_eq!(status.code(), Some(0), "{status:?}");
}

fn spawn(args: &[&str]) -> Child {
    let child = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .args(args)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Give the server time to start catching signals.
    std::thread::sleep(Duration::from_millis(500));
    child
}

fn signal(child: &Child, signal: i32) {
    // SAFETY: kill only sends a signal to the child process.
    assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
}

fn wait(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        if start.elapsed() > timeout {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
