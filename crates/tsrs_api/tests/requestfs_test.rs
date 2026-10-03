// Ports of tsc/internal/api/session_requestfilesystem_test.go scenarios through the wire.
mod common;
use common::*;
use tsrs_core::json::{self, Value};

fn num(v: &Value) -> u64 {
    match v {
        Value::Number(n) => *n as u64,
        _ => panic!("{v:?}"),
    }
}

fn files_json(entries: &[(&str, &str)]) -> String {
    let parts: Vec<String> = entries.iter().map(|(k, v)| format!("{}:{}", quote(k), quote(v))).collect();
    format!("{{{}}}", parts.join(","))
}

#[test]
fn create_snapshot_uses_full_file_system_and_layers() {
    let dir = TempDir::new("rfs-full");
    let host = dir.write("host.ts", "export const host = 1;");
    let cfg = dir.path("tsconfig.json");
    let index = dir.path("src/index.ts");
    let other = dir.path("src/other.ts");
    let s = session(&dir.dir(), false);
    let r = call(&s, "createSnapshot", &format!(
        "{{\"openProjects\":[{}],\"fileSystem\":{{\"kind\":\"full\",\"files\":{}}}}}",
        quote(&cfg),
        files_json(&[(&cfg, r#"{ "compilerOptions": { "noLib": true, "noEmit": true }, "files": ["src/index.ts"] }"#), (&index, "export const value: number = \"memory\";"), (&other, "export const other = true;")])
    ));
    let snap = num(get(&r, "snapshot"));
    assert_eq!(str_of(get(&r, "projects.0.configFileName")), cfg);
    let project = str_of(get(&r, "projects.0.id")).to_string();
    let sp = format!("\"snapshot\":{snap},\"project\":{}", quote(&project));
    assert_eq!(call(&s, "getSourceFileNames", &format!("{{{sp}}}")), Value::Array(vec![Value::String(index.clone())]));
    let d = call(&s, "getSemanticDiagnostics", &format!("{{{sp}}}"));
    assert_eq!(get(&d, "0.code"), &Value::Number(2322.0));
    // The full filesystem is total: host files are invisible.
    assert_eq!(call(&s, "getDefaultProjectForFile", &format!("{{\"snapshot\":{snap},\"file\":{}}}", quote(&host))), Value::Null);
    // Carrying the same filesystem forward without changes keeps the program (incremental state).
    let u = call(&s, "updateSnapshot", &format!("{{\"snapshot\":{snap}}}"));
    assert_eq!(get(&u, "projects"), &Value::Array(vec![]));
}

#[test]
fn full_file_system_is_total_and_layer_retains_base() {
    let dir = TempDir::new("rfs-total");
    dir.write("host.ts", "export const host = 1;");
    let cfg = dir.path("tsconfig.json");
    let index = dir.path("src/index.ts");
    let other = dir.path("src/other.ts");
    let s = session(&dir.dir(), false);
    let base = call(&s, "createSnapshot", "{}");
    let r = call(&s, "updateSnapshot", &format!(
        "{{\"snapshot\":{},\"changes\":{{\"openProjects\":[{}],\"fileSystem\":{{\"kind\":\"full\",\"files\":{}}}}}}}",
        num(get(&base, "snapshot")),
        quote(&cfg),
        files_json(&[(&cfg, r#"{ "compilerOptions": { "noLib": true, "noEmit": true }, "include": ["**/*.ts"] }"#), (&index, "export const value = 1;"), (&other, "export const other = 2;")])
    ));
    let snap = num(get(&r, "snapshot"));
    let project = str_of(get(&r, "projects.0.id")).to_string();
    let names = call(&s, "getSourceFileNames", &format!("{{\"snapshot\":{snap},\"project\":{}}}", quote(&project)));
    let names = json::marshal(&names).unwrap();
    assert!(names.contains("src/index.ts") && names.contains("src/other.ts"), "{names}");
    assert!(!names.contains("host.ts"), "full filesystem must hide host files: {names}");

    // A layer overrides one file and keeps the rest of the full filesystem.
    let r2 = call(&s, "updateSnapshot", &format!(
        "{{\"snapshot\":{snap},\"changes\":{{\"ensurePrograms\":true,\"fileSystem\":{{\"kind\":\"layer\",\"files\":{}}}}}}}",
        files_json(&[(&index, "export const value: string = 3;")])
    ));
    let snap2 = num(get(&r2, "snapshot"));
    let sp2 = format!("\"snapshot\":{snap2},\"project\":{}", quote(&project));
    let d = call(&s, "getSemanticDiagnostics", &format!("{{{sp2}}}"));
    assert_eq!(get(&d, "0.code"), &Value::Number(2322.0), "{}", json::marshal(&d).unwrap());
    let names2 = json::marshal(&call(&s, "getSourceFileNames", &format!("{{{sp2}}}"))).unwrap();
    assert_eq!(names, names2);
    // Old snapshot still sees the original content.
    assert_eq!(call(&s, "getSemanticDiagnostics", &format!("{{\"snapshot\":{snap},\"project\":{}}}", quote(&project))), Value::Null);
}

#[test]
fn layer_over_host_overrides_and_removes() {
    let dir = TempDir::new("rfs-layer");
    let cfg = dir.write("tsconfig.json", r#"{ "compilerOptions": { "strict": true, "noEmit": true }, "include": ["*.ts"] }"#);
    let a = dir.write("a.ts", "export const a: number = 1;");
    let b = dir.write("b.ts", "export const b: number = 'bad';");
    let s = session(&dir.dir(), false);
    let r = call(&s, "createSnapshot", &format!(
        "{{\"openProjects\":[{}],\"fileSystem\":{{\"kind\":\"layer\",\"files\":{},\"removedPaths\":[{}]}}}}",
        quote(&cfg),
        files_json(&[(&a, "export const a: number = 'layered';")]),
        quote(&b)
    ));
    let snap = num(get(&r, "snapshot"));
    let project = str_of(get(&r, "projects.0.id")).to_string();
    let sp = format!("\"snapshot\":{snap},\"project\":{}", quote(&project));
    let names = json::marshal(&call(&s, "getSourceFileNames", &format!("{{{sp}}}"))).unwrap();
    assert!(names.contains("a.ts") && !names.contains("b.ts"), "{names}");
    let d = call(&s, "getSemanticDiagnostics", &format!("{{{sp}}}"));
    assert_eq!(str_of(get(&d, "0.fileName")), a);
    // Disk content unchanged.
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "export const a: number = 1;");
}

#[test]
fn emit_from_layer_over_full_file_system_returns_contents() {
    let dir = TempDir::new("rfs-emit");
    let cfg = dir.path("tsconfig.json");
    let main = dir.path("src/main.ts");
    let out = dir.path("out");
    let s = session(&dir.dir(), false);
    let base = call(&s, "createSnapshot", &format!(
        "{{\"openProjects\":[{}],\"fileSystem\":{{\"kind\":\"full\",\"files\":{}}}}}",
        quote(&cfg),
        files_json(&[(&cfg, &format!("{{ \"compilerOptions\": {{ \"noLib\": true, \"outDir\": {} }}, \"files\": [\"src/main.ts\"] }}", quote(&out))), (&main, "export const value: number = 1;")])
    ));
    let base_snap = num(get(&base, "snapshot"));
    let project = str_of(get(&base, "projects.0.id")).to_string();
    let layered = call(&s, "updateSnapshot", &format!("{{\"snapshot\":{base_snap},\"changes\":{{\"fileSystem\":{{\"kind\":\"layer\",\"files\":{{}}}}}}}}"));
    assert_eq!(get(&layered, "projects"), &Value::Array(vec![]));
    let lsnap = num(get(&layered, "snapshot"));
    let emit = |snap: u64| call(&s, "emit", &format!("{{\"snapshot\":{snap},\"project\":{}}}", quote(&project)));
    let e = emit(lsnap);
    assert_eq!(get(&e, "emittedFiles"), &Value::Array(vec![Value::String(dir.path("out/src/main.js"))]));
    assert_eq!(get(&e, "emittedFilesContents"), &Value::Array(vec![Value::String("export const value = 1;\n".into())]));
    assert!(!std::path::Path::new(&dir.path("out/src/main.js")).exists(), "full filesystem emit must not write to disk");
    call(&s, "release", &format!("{{\"snapshot\":{base_snap}}}"));
    assert_eq!(emit(lsnap), e);
}

#[test]
fn request_file_system_errors_are_client_errors() {
    let dir = TempDir::new("rfs-err");
    let s = session(&dir.dir(), false);
    let e = call_err(&s, "createSnapshot", r#"{"fileSystem":{"kind":"partial","files":{}}}"#);
    assert!(e.starts_with("api: client error: unknown request filesystem kind"), "{e}");
    let e = call_err(&s, "createSnapshot", r#"{"fileSystem":{"kind":"full","files":{"/a.ts":"x","/A.ts/../a.ts":"y"}}}"#);
    assert!(e.contains("duplicate request filesystem file path"), "{e}");
}
