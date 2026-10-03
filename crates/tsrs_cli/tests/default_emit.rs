// tsrs emits by default, like tsc: an unchanged command writes what the project's options ask for (JS, declarations,
// maps, tsbuildinfo), and only the options (noEmit, emitDeclarationOnly, noEmitOnError, incremental/composite) turn
// that off. The old TSRS_EMIT environment variable no longer changes anything. Expected file sets were taken from tsgo
// built from the pinned ts-ref commit on the same project.

use std::path::{Path, PathBuf};
use std::process::Command;

const TSCONFIG: &str = r#"{ "compilerOptions": { "target": "esnext", "module": "nodenext", "declaration": true, "declarationMap": true, "sourceMap": true, "composite": true, "incremental": true, "rootDir": "src", "outDir": "out" }, "include": ["src"] }"#;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrs-default-emit-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("tsconfig.json"), TSCONFIG).unwrap();
    std::fs::write(dir.join("src/a.ts"), "export const a: number = 1;\nexport function f(x: string) { return x; }\n").unwrap();
    dir
}

// Files the compiler wrote: everything but the inputs, relative to `dir`.
fn outputs(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
                if rel != "tsconfig.json" && !rel.starts_with("src/") {
                    out.push(rel);
                }
            }
        }
    }
    out.sort();
    out
}

fn tsrs(args: &[&str], dir: &Path, emit_var: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsrs"));
    cmd.args(args).current_dir(dir).env_remove("TSRS_EMIT");
    if let Some(v) = emit_var {
        cmd.env("TSRS_EMIT", v);
    }
    cmd.output().unwrap()
}

fn ok(out: &std::process::Output) {
    assert!(out.status.success(), "exit {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stdout));
}

const ALL: [&str; 5] = ["out/a.d.ts", "out/a.d.ts.map", "out/a.js", "out/a.js.map", "tsconfig.tsbuildinfo"];

#[test]
fn unchanged_command_emits_and_old_env_var_is_ignored() {
    let mut contents = Vec::new();
    for (i, value) in [None, Some("0"), Some("1"), Some("")].into_iter().enumerate() {
        let dir = scratch(&format!("default{i}"));
        ok(&tsrs(&["-p", "."], &dir, value));
        assert_eq!(outputs(&dir), ALL, "TSRS_EMIT={value:?}");
        let js = std::fs::read_to_string(dir.join("out/a.js")).unwrap();
        // tsgo's output for this project (module nodenext without a package.json `type`: CommonJS).
        let expected = "\"use strict\";\nObject.defineProperty(exports, \"__esModule\", { value: true });\nexports.a = void 0;\nexports.f = f;\nexports.a = 1;\nfunction f(x) { return x; }\n//# sourceMappingURL=a.js.map";
        assert_eq!(js, expected);
        contents.push(ALL.iter().filter(|f| **f != "tsconfig.tsbuildinfo").map(|f| std::fs::read(dir.join(f)).unwrap()).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(contents.windows(2).all(|w| w[0] == w[1]), "TSRS_EMIT changed the output");
}

#[test]
fn options_disable_emit_like_tsc() {
    // (args, expected outputs) from tsgo on the same project.
    let cases: [(&[&str], &[&str]); 5] = [
        (&["-p", ".", "--noEmit"], &["tsconfig.tsbuildinfo"]),
        (&["-p", ".", "--noEmit", "--incremental", "false", "--composite", "false"], &[]),
        (&["-p", ".", "--emitDeclarationOnly"], &["out/a.d.ts", "out/a.d.ts.map", "tsconfig.tsbuildinfo"]),
        (&["-p", ".", "--incremental", "false", "--composite", "false", "--declaration", "false", "--declarationMap", "false"], &["out/a.js", "out/a.js.map"]),
        (&["-p", ".", "--sourceMap", "false", "--declarationMap", "false"], &["out/a.d.ts", "out/a.js", "tsconfig.tsbuildinfo"]),
    ];
    for (i, (args, expected)) in cases.iter().enumerate() {
        let dir = scratch(&format!("opts{i}"));
        ok(&tsrs(args, &dir, Some("1")));
        assert_eq!(outputs(&dir), *expected, "{args:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn no_emit_on_error_writes_nothing() {
    let dir = scratch("noemitonerror");
    std::fs::write(dir.join("src/b.ts"), "export const b: number = \"not a number\";\n").unwrap();
    let out = tsrs(&["-p", ".", "--noEmitOnError", "--incremental", "false", "--composite", "false"], &dir, None);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    assert_eq!(outputs(&dir), Vec::<String>::new());
    // Without noEmitOnError the outputs are written and the exit status says so (DiagnosticsPresent_OutputsGenerated).
    let out = tsrs(&["-p", ".", "--incremental", "false", "--composite", "false"], &dir, None);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stdout));
    assert!(outputs(&dir).contains(&"out/b.js".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn incremental_rebuild_reuses_build_info() {
    let dir = scratch("incr");
    ok(&tsrs(&["-p", "."], &dir, None));
    let build_info = std::fs::read_to_string(dir.join("tsconfig.tsbuildinfo")).unwrap();
    assert!(build_info.starts_with("{\"version\":") && build_info.contains("\"root\":["), "{build_info}");
    // A second run reads it back and has nothing to do: neither the build info nor the outputs are rewritten.
    let m_time = |f: &str| std::fs::metadata(dir.join(f)).unwrap().modified().unwrap();
    let (bi, js) = (m_time("tsconfig.tsbuildinfo"), m_time("out/a.js"));
    ok(&tsrs(&["-p", "."], &dir, None));
    assert_eq!((m_time("tsconfig.tsbuildinfo"), m_time("out/a.js")), (bi, js));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_mode_builds_without_env() {
    let dir = scratch("build");
    let out = tsrs(&["-b", ".", "--verbose"], &dir, None);
    ok(&out);
    assert_eq!(outputs(&dir), ALL);
    let out = tsrs(&["-b", ".", "--verbose"], &dir, None);
    ok(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("is up to date"), "{}", String::from_utf8_lossy(&out.stdout));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_and_version_do_not_depend_on_env() {
    let dir = scratch("help");
    for flag in ["--version", "--help"] {
        let off = tsrs(&[flag], &dir, None);
        let on = tsrs(&[flag], &dir, Some("1"));
        assert_eq!(off.stdout, on.stdout, "{flag}");
        let text = String::from_utf8_lossy(&off.stdout);
        assert!(text.contains("Version") && !text.contains("noEmit"), "{flag}: {text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
