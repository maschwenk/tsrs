// Release guard for the TSRS_EMIT gate (docs/EMIT.md section 6): without TSRS_EMIT=1 the binary behaves like
// `tsc --noEmit` and never writes a file, even for a project that asks for JS, declarations and source maps.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrs-emit-gate-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "esnext", "module": "nodenext", "declaration": true, "declarationMap": true, "sourceMap": true, "composite": true, "incremental": true, "rootDir": "src", "outDir": "out" }, "include": ["src"] }"#,
    )
    .unwrap();
    std::fs::write(dir.join("src/a.ts"), "export const a: number = 1;\nexport function f(x: string) { return x; }\n").unwrap();
    dir
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn run(dir: &Path, emit_var: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    cmd.arg("-p").arg(dir).env_remove("TSRS_EMIT");
    if let Some(v) = emit_var {
        cmd.env("TSRS_EMIT", v);
    }
    cmd.output().unwrap()
}

#[test]
fn no_files_written_without_tsrs_emit() {
    for (i, value) in [None, Some("0"), Some(""), Some("true"), Some("yes")].into_iter().enumerate() {
        let dir = scratch(&format!("off{i}"));
        let before = files_under(&dir);
        let out = run(&dir, value);
        assert!(out.status.success(), "TSRS_EMIT={value:?}: {}", String::from_utf8_lossy(&out.stdout));
        assert_eq!(files_under(&dir), before, "TSRS_EMIT={value:?} wrote files");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn help_and_version_unchanged_by_gate() {
    let dir = scratch("help");
    for flag in ["--version", "--help"] {
        let off = Command::new(env!("CARGO_BIN_EXE_tsrs")).arg(flag).env_remove("TSRS_EMIT").current_dir(&dir).output().unwrap();
        let on = Command::new(env!("CARGO_BIN_EXE_tsrs")).arg(flag).env("TSRS_EMIT", "1").current_dir(&dir).output().unwrap();
        assert_eq!(off.stdout, on.stdout, "{flag}");
        assert!(String::from_utf8_lossy(&off.stdout).contains("Version"), "{flag}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn declarations_written_with_tsrs_emit() {
    let dir = scratch("on");
    let out = Command::new(env!("CARGO_BIN_EXE_tsrs"))
        .args(["-p".as_ref(), dir.as_os_str(), "--emitDeclarationOnly".as_ref(), "--declarationMap".as_ref(), "false".as_ref()])
        .env("TSRS_EMIT", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let dts = std::fs::read_to_string(dir.join("out/a.d.ts")).unwrap();
    assert_eq!(dts, "export declare const a: number;\nexport declare function f(x: string): string;\n");
    let _ = std::fs::remove_dir_all(&dir);
}
