// `--generateTrace <dir>`: tsgo writes a trace and type-checks as usual. tsrs has no trace writer, so it takes the
// branch Go takes when the trace cannot be started (tsc.go startTracingIfNeeded): one warning line, then the same
// compilation, diagnostics and exit status as without the flag. Without the warning a run that asked for a trace
// silently got none.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const WARNING: &str = "Warning: Failed to start tracing: --generateTrace is not supported by tsrs\n";

fn scratch(name: &str, incremental: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrs-generate-trace-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let tsconfig = format!(r#"{{ "compilerOptions": {{ "strict": true, "noEmit": true, "incremental": {incremental} }}, "files": ["a.ts"] }}"#);
    std::fs::write(dir.join("tsconfig.json"), tsconfig).unwrap();
    std::fs::write(dir.join("a.ts"), "const x: number = \"not a number\";\n").unwrap();
    dir
}

fn tsrs(dir: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tsrs")).args(["-p", "tsconfig.json", "--pretty", "false"]).args(extra).current_dir(dir).output().unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn generate_trace_warns_and_still_type_checks() {
    for incremental in [false, true] {
        let dir = scratch(&format!("incremental-{incremental}"), incremental);
        let trace_dir = dir.join("trace");
        let trace_dir = trace_dir.to_str().unwrap();
        let without = tsrs(&dir, &[]);
        assert!(stdout(&without).contains("error TS2322"), "{}", stdout(&without));
        assert_ne!(without.status.code(), Some(0));
        // The incremental project's second run is served from the build info (as in tsgo); its errors still count.
        for _ in 0..2 {
            let with = tsrs(&dir, &["--generateTrace", trace_dir]);
            assert_eq!(stdout(&with), format!("{WARNING}{}", stdout(&without)), "incremental: {incremental}");
            assert_eq!(with.status.code(), without.status.code(), "incremental: {incremental}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
