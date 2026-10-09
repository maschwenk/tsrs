use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tsrs_core::Locale;
use tsrs_spanmap::{Feature, Kind, Segment};
use tsrs_tsoptions::{Manifest, Mapper};

use crate::{new_host, new_host_with_options, spawn_process, HostOptions, ProcessSpawner, Request};

const LAUNCHER_ARG: &str = "child-process-launcher";
const DESCENDANT_ARG: &str = "child-process-descendant";
const CLOSE_TEST: &str = "process_test::test_child_process_close_does_not_wait_for_launcher_descendants";

// The test binary re-run as a launcher or descendant process: libtest takes the trailing role argument as one more
// name filter, which `--exact` keeps from matching anything else.
fn self_command(role: &str) -> Vec<String> {
    let exe = std::env::current_exe().unwrap().to_string_lossy().into_owned();
    vec![exe, CLOSE_TEST.to_string(), "--exact".to_string(), "--nocapture".to_string(), role.to_string()]
}

// Go `isProcessAlive` (cmd/tsc/isprocessalive_unix.go): signal 0.
fn is_process_alive(pid: u32) -> bool {
    Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
}

fn kill(pid: u32) {
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).stderr(Stdio::null()).status();
}

// cmd/tsc/sys_unix_test.go:20 (The launcher prints the descendant's pid as `pid:<n>`: libtest writes its own lines to
// the same stdout first, where Go's test binary is silent.)
#[test]
fn test_child_process_close_does_not_wait_for_launcher_descendants() {
    match std::env::args().last().as_deref() {
        Some(LAUNCHER_ARG) => {
            let command = self_command(DESCENDANT_ARG);
            let mut cmd = Command::new(&command[0]).args(&command[1..]).spawn().unwrap();
            println!("pid:{}", cmd.id());
            cmd.wait().unwrap();
            return;
        }
        Some(DESCENDANT_ARG) => {
            thread::sleep(Duration::from_secs(60));
            return;
        }
        _ => {}
    }

    let process = spawn_process(&self_command(LAUNCHER_ARG), "", Some(Box::new(Vec::<u8>::new()))).unwrap();
    // Go's bufio.Reader keeps the pipe open, and so does this one until the end of the test: a descendant whose stdout
    // is closed dies at its next write.
    let mut stdout = BufReader::new(process.reader);
    let mut descendant_pid = None;
    let mut line = String::new();
    while stdout.read_line(&mut line).unwrap() > 0 {
        if let Some(pid) = line.strip_prefix("pid:") {
            descendant_pid = Some(pid.trim().parse::<u32>().unwrap());
            break;
        }
        line.clear();
    }
    let descendant_pid = descendant_pid.expect("the launcher printed its descendant's pid");
    let (done_tx, done) = mpsc::channel();
    let closer = process.closer;
    thread::spawn(move || {
        let _ = done_tx.send(closer.close());
    });

    let completed = match done.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => {
            result.unwrap();
            true
        }
        Err(_) => {
            kill(descendant_pid);
            let _ = done.recv();
            false
        }
    };
    assert!(completed, "child process shutdown waited for a launcher descendant");
    assert!(is_process_alive(descendant_pid), "the descendant had already exited, so the close did not wait on it");
    kill(descendant_pid);
    drop(stdout);
}

// testdata/contentmapper/header-mapper: a Node mapper that prefixes HEADER and maps the original text verbatim
// after it, and reports `//!mapper-error` as diagnostic 7 of source `header`.
const HEADER: &str = "// synthesized by header-mapper\nexport {};\n";

fn header_mapper_directory() -> String {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/contentmapper/header-mapper");
    directory.canonicalize().unwrap().to_string_lossy().into_owned()
}

fn header_mapper(exec: &[&str]) -> &'static Mapper {
    Box::leak(Box::new(Mapper {
        manifest: Manifest {
            name: "header-mapper".to_string(),
            version: "1.0.0".to_string(),
            exec: exec.iter().map(|arg| arg.to_string()).collect(),
            ..Default::default()
        },
        package_directory: header_mapper_directory(),
        ..Default::default()
    }))
}

// These tests run a real mapper process, which needs `node` on PATH.
fn node_available() -> bool {
    let available = Command::new("node").arg("--version").output().is_ok_and(|output| output.status.success());
    if !available {
        eprintln!("skipping: the header-mapper test needs `node` on PATH");
    }
    available
}

// The production spawner runs a real mapper: the transform comes back over the child's stdio with the header, one
// verbatim mapping offset by the header's length, and the mapper's own diagnostic.
#[test]
fn test_process_spawner_runs_header_mapper() {
    if !node_available() {
        return;
    }
    let host = new_host(Arc::new(ProcessSpawner), Locale::DEFAULT);
    let mapper = header_mapper(&["node", "mapper.mjs"]);

    let content = "export const x: number = 1;\n";
    let result = host.transform(mapper, Request { file_name: "/project/a.hdr", content }).unwrap();
    assert_eq!(result.text, format!("{HEADER}{content}"));
    assert_eq!(result.virtual_extension, ".ts");
    assert_eq!(
        result.mappings.unwrap().segments(),
        vec![Segment {
            virtual_start: HEADER.len() as i32,
            virtual_end: (HEADER.len() + content.len()) as i32,
            original_start: 0,
            original_end: content.len() as i32,
            kind: Kind::Verbatim,
            features: Feature::All,
        }]
    );
    assert!(result.diagnostics.is_empty());

    let content = "const y = 2;\n//!mapper-error\n";
    let result = host.transform(mapper, Request { file_name: "/project/b.hdr", content }).unwrap();
    assert_eq!(result.text, format!("{HEADER}{content}"));
    assert_eq!(result.diagnostics.len(), 1);
    let diagnostic = result.diagnostics[0];
    assert_eq!(diagnostic.source(), "header");
    assert_eq!(diagnostic.code(), 7);
    assert_eq!(diagnostic.pos(), content.find("//!mapper-error").unwrap() as i32);
    assert_eq!(diagnostic.end(), diagnostic.pos() + "//!mapper-error".len() as i32);

    host.close().unwrap();
}

// A mapper that leaves a background process holding its stdout (and stderr): killing the mapper does not end the
// host's read of that pipe, which the host never joins, so closing the host returns after the stderr grace of one
// second (process.rs) instead of waiting for the background process. The mapper reports the background process's pid
// on stderr, which the host's logger receives.
#[test]
fn test_host_close_does_not_wait_for_mapper_descendants() {
    if !node_available() {
        return;
    }
    let logs = Arc::new(Mutex::new(Vec::<String>::new()));
    let logger = {
        let logs = Arc::clone(&logs);
        Arc::new(move |message: &str| logs.lock().unwrap().push(message.to_string()))
    };
    let host = new_host_with_options(Arc::new(ProcessSpawner), Locale::DEFAULT, HostOptions { logger: Some(logger) });
    let mapper = header_mapper(&["sh", "-c", "sleep 30 & echo background:$! >&2; exec node mapper.mjs"]);
    let content = "export {};\n";
    let result = host.transform(mapper, Request { file_name: "/project/a.hdr", content }).unwrap();
    assert_eq!(result.text, format!("{HEADER}{content}"));
    let background_pid = logs
        .lock()
        .unwrap()
        .iter()
        .find_map(|line| line.strip_prefix("[content mapper: header-mapper] stderr: background:")?.trim().parse::<u32>().ok())
        .expect("the mapper reported its background process");

    let start = Instant::now();
    host.close().unwrap();
    let elapsed = start.elapsed();
    let alive = is_process_alive(background_pid);
    kill(background_pid);
    assert!(alive, "the background process had already exited, so the close did not wait on it");
    assert!(elapsed < Duration::from_secs(2), "closing the host waited {elapsed:?} for a mapper descendant");
}
