use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn scratch(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("tsrs-headless-{name}-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str], payload: &Value) -> Output {
    run_bytes(dir, args, &serde_json::to_vec(payload).unwrap())
}

fn run_bytes(dir: &Path, args: &[&str], payload: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .arg("headless")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(payload).unwrap();
    child.wait_with_output().unwrap()
}

fn frames(output: &[u8]) -> Vec<(u8, Value)> {
    let mut offset = 0;
    let mut frames = Vec::new();
    while offset < output.len() {
        assert!(output.len() - offset >= 5, "incomplete frame header");
        let len = u32::from_le_bytes(output[offset..offset + 4].try_into().unwrap()) as usize;
        let kind = output[offset + 4];
        offset += 5;
        assert!(output.len() - offset >= len, "incomplete frame payload");
        frames.push((
            kind,
            serde_json::from_slice(&output[offset..offset + len]).unwrap(),
        ));
        offset += len;
    }
    frames
}

#[test]
fn oxc_v2_protocol_emits_rule_diagnostics_suggestions_and_timings() {
    let dir = scratch("rule");
    let file = dir.join("index.ts");
    std::fs::write(&file, "async function work(): Promise<void> {}\nwork();\n").unwrap();
    std::fs::write(
        dir.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true, "target": "ES2022" }, "files": ["index.ts"] }"#,
    )
    .unwrap();
    let payload = json!({
        "version": 2,
        "configs": [{
            "file_paths": [file],
            "rules": [{ "name": "no-floating-promises" }]
        }],
        "source_overrides": null,
        "report_syntactic": false,
        "report_semantic": false
    });
    let output = run(&dir, &["-fix-suggestions", "-debug", "timings"], &payload);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frames = frames(&output.stdout);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].0, 1);
    assert_eq!(frames[0].1["kind"], 0);
    assert_eq!(frames[0].1["rule"], "no-floating-promises");
    assert_eq!(frames[0].1["message"]["id"], "floatingVoid");
    assert_eq!(frames[0].1["suggestions"].as_array().unwrap().len(), 2);
    assert_eq!(frames[1].0, 2);
    assert_eq!(frames[1].1["rules"][0]["rule_name"], "no-floating-promises");
    assert_eq!(frames[1].1["rules"][0]["calls"], 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn source_overrides_are_used_for_type_aware_linting() {
    let dir = scratch("overlay");
    let file = dir.join("index.ts");
    std::fs::write(
        &file,
        "async function work(): Promise<void> {}\nvoid work();\n",
    )
    .unwrap();
    std::fs::write(dir.join("tsconfig.json"), r#"{ "files": ["index.ts"] }"#).unwrap();
    let payload = json!({
        "version": 2,
        "configs": [{
            "file_paths": [file],
            "rules": [{ "name": "no-floating-promises" }]
        }],
        "source_overrides": {
            file.to_string_lossy(): "async function work(): Promise<void> {}\nwork();\n"
        }
    });
    let output = run(&dir, &[], &payload);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frames = frames(&output.stdout);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].1["message"]["id"], "floatingVoid");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn type_check_only_and_bad_tsconfig_emit_internal_diagnostics() {
    let dir = scratch("type-errors");
    let file = dir.join("index.ts");
    std::fs::write(&file, "const value: number = 'text';\n").unwrap();
    std::fs::write(dir.join("tsconfig.json"), r#"{ "files": ["index.ts"] }"#).unwrap();
    let payload = json!({
        "version": 2,
        "configs": [{ "file_paths": [file], "rules": [] }],
        "report_syntactic": true,
        "report_semantic": true
    });
    let output = run(&dir, &[], &payload);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed_frames = frames(&output.stdout);
    assert!(parsed_frames.iter().any(|(kind, diagnostic)| *kind == 1
        && diagnostic["kind"] == 1
        && diagnostic["message"]["id"] == "TS2322"));

    std::fs::write(
        dir.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "not-a-target" }, "files": ["index.ts"] }"#,
    )
    .unwrap();
    let output = run(&dir, &[], &payload);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed_frames = frames(&output.stdout);
    assert!(parsed_frames.iter().any(|(kind, diagnostic)| {
        *kind == 1
            && diagnostic["kind"] == 1
            && diagnostic["message"]["id"] == "tsconfig-error"
            && diagnostic["message"]["description"] == "Invalid tsconfig"
    }));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unsupported_rules_are_skipped_and_bad_payloads_use_error_frames() {
    let dir = scratch("errors");
    let file = dir.join("index.ts");
    std::fs::write(&file, "Promise.resolve();\n").unwrap();
    let payload = json!({
        "version": 2,
        "configs": [{ "file_paths": [file], "rules": [{ "name": "await-thenable" }] }]
    });
    let output = run(&dir, &[], &payload);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(frames(&output.stdout).is_empty());

    let output = run(&dir, &[], &json!({ "version": 3, "configs": [] }));
    assert!(!output.status.success());
    let parsed_frames = frames(&output.stdout);
    assert_eq!(parsed_frames.len(), 1);
    assert_eq!(parsed_frames[0].0, 0);
    assert!(
        parsed_frames[0].1["error"]
            .as_str()
            .unwrap()
            .contains("unsupported version")
    );

    let output = run_bytes(&dir, &[], b"not JSON");
    assert!(!output.status.success());
    let parsed_frames = frames(&output.stdout);
    assert_eq!(parsed_frames.len(), 1);
    assert_eq!(parsed_frames[0].0, 0);
    assert!(
        parsed_frames[0].1["error"]
            .as_str()
            .unwrap()
            .contains("error parsing config")
    );
    let _ = std::fs::remove_dir_all(dir);
}
