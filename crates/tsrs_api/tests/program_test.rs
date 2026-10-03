mod common;
use common::*;
use tsrs_core::json::{self, Value};

fn arr(v: &Value) -> &Vec<Value> {
    match v {
        Value::Array(a) => a,
        _ => panic!("not an array: {}", json::marshal(v).unwrap()),
    }
}

fn create_program(s: &tsrs_api::Session, roots: &[String], options: &str) -> (u64, String) {
    let roots: Vec<String> = roots.iter().map(|r| quote(r)).collect();
    let r = call(s, "createSnapshot", &format!("{{\"createPrograms\":[{{\"rootFiles\":[{}],\"compilerOptions\":{options}}}]}}", roots.join(",")));
    let snapshot = match get(&r, "snapshot") {
        Value::Number(n) => *n as u64,
        v => panic!("{v:?}"),
    };
    let id = str_of(get(&r, "operation.createdPrograms.0")).to_string();
    assert!(arr(get(&r, "projects")).iter().any(|p| str_of(get(p, "id")) == id));
    (snapshot, id)
}

#[test]
fn create_program_diagnostics_emit_release() {
    let dir = TempDir::new("prog");
    let a = dir.write("src/a.ts", "export const a: number = 'x';\nexport function f(x: string) { return x.length; }\n");
    let s = session(&dir.dir(), false);
    let (snap, project) = create_program(&s, &[a.clone()], &format!("{{\"strict\":true,\"target\":9,\"outDir\":{},\"declaration\":true}}", quote(&dir.path("out"))));
    let sp = format!("\"snapshot\":{snap},\"project\":{}", quote(&project));

    let names = call(&s, "getSourceFileNames", &format!("{{{sp}}}"));
    assert!(arr(&names).iter().any(|n| str_of(n) == a), "{}", json::marshal(&names).unwrap());
    assert!(arr(&names).iter().any(|n| str_of(n).ends_with("lib.es2022.d.ts")));

    let syn = call(&s, "getSyntacticDiagnostics", &format!("{{{sp},\"files\":[{}]}}", quote(&a)));
    assert_eq!(syn, Value::Array(vec![]));
    let sem = call(&s, "getSemanticDiagnostics", &format!("{{{sp},\"files\":[{}]}}", quote(&a)));
    let d = &arr(&sem)[0];
    assert_eq!(get(d, "code"), &Value::Number(2322.0));
    assert_eq!(str_of(get(d, "fileName")), a);
    assert_eq!(get(d, "startPosition.line"), &Value::Number(0.0));
    assert_eq!(get(d, "pos"), &Value::Number(13.0));
    let all = call(&s, "getSemanticDiagnostics", &format!("{{{sp}}}"));
    assert_eq!(arr(&all).len(), 1);
    assert_eq!(call(&s, "getProgramDiagnostics", &format!("{{{sp}}}")), Value::Array(vec![]));
    assert_eq!(call(&s, "getGlobalDiagnostics", &format!("{{{sp}}}")), Value::Array(vec![]));

    let out = call(&s, "emitToString", &format!("{{{sp}}}"));
    assert_eq!(get(&out, "emitSkipped"), &Value::Bool(false));
    let files = arr(get(&out, "outputFiles"));
    let names: Vec<&str> = files.iter().map(|f| str_of(get(f, "fileName"))).collect();
    assert_eq!(names, vec![dir.path("out/a.d.ts"), dir.path("out/a.js")]);
    assert!(str_of(get(&files[1], "text")).contains("export function f(x) {"), "{}", str_of(get(&files[1], "text")));
    assert!(str_of(get(&files[0], "text")).contains("export declare function f(x: string): number;"));
    assert_eq!(str_of(get(&files[1], "sourceFileName")), a);

    let js = call(&s, "getJavaScriptEmit", &format!("{{{sp},\"files\":[{}]}}", quote(&a)));
    assert_eq!(arr(get(&js, "outputFiles")).len(), 1);
    let e = call_err(&s, "getJavaScriptEmit", &format!("{{{sp}}}"));
    assert!(e.contains("files is required"), "{e}");

    // Write-through emit follows compiler options (no TSRS_EMIT gate).
    let w = call(&s, "emit", &format!("{{{sp}}}"));
    assert_eq!(arr(get(&w, "emittedFiles")).len(), 2);
    assert_eq!(arr(get(&w, "emittedFilesContents")).len(), 0, "write-through emit has no in-memory contents");
    assert!(std::path::Path::new(&dir.path("out/a.js")).exists());
    assert!(std::fs::read_to_string(dir.path("out/a.d.ts")).unwrap().contains("declare const a: number"));

    let e = call_err(&s, "getSemanticDiagnostics", &format!("{{\"snapshot\":{snap},\"project\":\"nope\"}}"));
    assert!(e.starts_with("api: client error: project nope not found"), "{e}");
    assert_eq!(call(&s, "release", &format!("{{\"snapshot\":{snap}}}")), Value::Bool(true));
    let e = call_err(&s, "getSourceFileNames", &format!("{{{sp}}}"));
    assert_eq!(e, format!("api: client error: snapshot {snap} not found"));
    let e = call_err(&s, "release", &format!("{{\"snapshot\":{snap}}}"));
    assert!(e.contains("not found"), "{e}");
}

#[test]
fn no_emit_option_skips_emit() {
    let dir = TempDir::new("noemit");
    let a = dir.write("a.ts", "export const a = 1;\n");
    let s = session(&dir.dir(), false);
    let (snap, project) = create_program(&s, &[a], "{\"noEmit\":true}");
    let out = call(&s, "emitToString", &format!("{{\"snapshot\":{snap},\"project\":{}}}", quote(&project)));
    // Go HandleNoEmitOptions: whole-program noEmit returns an empty, non-skipped result.
    assert_eq!(get(&out, "emitSkipped"), &Value::Bool(false));
    assert_eq!(arr(get(&out, "outputFiles")).len(), 0);
    // Selected-file emit forces emit regardless of noEmit (Go ForceEmit: true).
    let a = dir.path("a.ts");
    let js = call(&s, "getJavaScriptEmit", &format!("{{\"snapshot\":{snap},\"project\":{},\"files\":[{}]}}", quote(&project), quote(&a)));
    assert_eq!(arr(get(&js, "outputFiles")).len(), 1);
}

#[test]
fn configured_project_update_and_retained_old_snapshot() {
    let dir = TempDir::new("cfgproj");
    let cfg = dir.write("tsconfig.json", r#"{ "compilerOptions": { "strict": true, "noEmit": true }, "include": ["*.ts"] }"#);
    let a = dir.write("a.ts", "export const a: number = 1;\n");
    let s = session(&dir.dir(), true);
    let r = call(&s, "createSnapshot", &format!("{{\"openProjects\":[{}]}}", quote(&cfg)));
    let snap1 = match get(&r, "snapshot") { Value::Number(n) => *n as u64, _ => unreachable!() };
    let project = str_of(get(&r, "projects.0.id")).to_string();
    assert_eq!(str_of(get(&r, "projects.0.configFileName")), cfg);
    assert_eq!(get(&r, "projects.0.parsedCommandLine.options.strict"), &Value::Bool(true));

    let dp = call(&s, "getDefaultProjectForFile", &format!("{{\"snapshot\":{snap1},\"file\":{}}}", quote(&a)));
    assert_eq!(str_of(get(&dp, "id")), project);
    let sp1 = format!("\"snapshot\":{snap1},\"project\":{}", quote(&project));
    assert_eq!(call(&s, "getSemanticDiagnostics", &format!("{{{sp1}}}")), Value::Array(vec![]));

    // Change the file on disk and notify; the old snapshot stays valid and unchanged.
    std::fs::write(&a, "export const a: number = 'no';\n").unwrap();
    let r2 = call(&s, "updateSnapshot", &format!("{{\"snapshot\":{snap1},\"changes\":{{\"ensurePrograms\":true,\"fileNotifications\":{{\"changed\":[{}]}}}}}}", quote(&a)));
    let snap2 = match get(&r2, "snapshot") { Value::Number(n) => *n as u64, _ => unreachable!() };
    assert_ne!(snap1, snap2);
    let changed = match get(&r2, "changes.changedProjects") {
        Value::Object(o) => get(o.get(&project).expect("project changes"), "changedFiles"),
        v => panic!("{v:?}"),
    };
    assert_eq!(arr(changed).len(), 1);
    let sp2 = format!("\"snapshot\":{snap2},\"project\":{}", quote(&project));
    assert_eq!(arr(&call(&s, "getSemanticDiagnostics", &format!("{{{sp2}}}"))).len(), 1);
    assert_eq!(call(&s, "getSemanticDiagnostics", &format!("{{{sp1}}}")), Value::Array(vec![]));

    // An update with no project changes omits `changes` (Go `omitempty`, json/v2: `{}` is empty); parity f703.
    let r3 = call(&s, "updateSnapshot", &format!("{{\"snapshot\":{snap2}}}"));
    let Value::Object(o3) = &r3 else { unreachable!() };
    assert!(o3.get("changes").is_none(), "{}", tsrs_core::json::marshal(&r3).unwrap());
    call(&s, "release", &format!("{{\"snapshot\":{}}}", match get(&r3, "snapshot") { Value::Number(n) => *n as u64, _ => unreachable!() }));

    // Releasing the newer snapshot leaves the old one usable; a second session cannot use these handles.
    call(&s, "release", &format!("{{\"snapshot\":{snap2}}}"));
    assert_eq!(call(&s, "getSemanticDiagnostics", &format!("{{{sp1}}}")), Value::Array(vec![]));
    let other = session(&dir.dir(), true);
    let e = call_err(&other, "getSemanticDiagnostics", &format!("{{{sp1}}}"));
    assert!(e.contains("not found"), "{e}");
    s.close();
    let e = call_err(&s, "getSemanticDiagnostics", &format!("{{{sp1}}}"));
    assert!(e.contains("session is closed"), "{e}");
}

#[test]
fn malformed_snapshot_params_are_bounded_errors() {
    let dir = TempDir::new("bad");
    let s = session(&dir.dir(), false);
    let e = call_err(&s, "createSnapshot", r#"{"createPrograms":[null]}"#);
    assert!(e.contains("createPrograms[0] must not be null"), "{e}");
    let e = call_err(&s, "createSnapshot", r#"{"createPrograms":[{"rootFiles":[],"compilerOptions":{"target":"es5"}}]}"#);
    assert!(e.starts_with("api: invalid request"), "{e}");
    let e = call_err(&s, "createSnapshot", r#"{"reconfigurePrograms":[{"id":"bogus","rootFiles":[],"compilerOptions":{}}]}"#);
    assert!(e.contains("invalid synthetic project handle"), "{e}");
    let e = call_err(&s, "updateSnapshot", r#"{"snapshot":12345}"#);
    assert!(e.contains("snapshot 12345 not found"), "{e}");
    let e = call_err(&s, "release", r#"{"snapshot":0}"#);
    assert!(e.contains("empty handle"), "{e}");
}

#[test]
fn remaining_diagnostic_kinds_and_declaration_emit() {
    let dir = TempDir::new("diagkinds");
    // Duplicate block-scoped declaration is a bind diagnostic; a private name in an exported signature is a
    // declaration emit diagnostic.
    let a = dir.write("a.ts", "let x = 1;\nlet x = 2;\nclass Hidden {}\nexport function g() { return new Hidden(); }\n");
    let s = session(&dir.dir(), false);
    let (snap, project) = create_program(&s, &[a.clone()], "{\"declaration\":true,\"isolatedDeclarations\":true}");
    let sp = format!("\"snapshot\":{snap},\"project\":{}", quote(&project));
    let bind = call(&s, "getBindDiagnostics", &format!("{{{sp},\"files\":[{}]}}", quote(&a)));
    assert!(arr(&bind).iter().any(|d| get(d, "code") == &Value::Number(2451.0)), "{}", json::marshal(&bind).unwrap());
    let decl = call(&s, "getDeclarationDiagnostics", &format!("{{{sp},\"files\":[{}]}}", quote(&a)));
    assert!(!arr(&decl).is_empty(), "expected declaration diagnostics");
    let sugg = call(&s, "getSuggestionDiagnostics", &format!("{{{sp},\"files\":[{}]}}", quote(&a)));
    assert!(matches!(sugg, Value::Null | Value::Array(_)));
    assert_eq!(call(&s, "getConfigFileParsingDiagnostics", &format!("{{{sp}}}")), Value::Array(vec![]));
    let dts = call(&s, "getDeclarationEmit", &format!("{{{sp},\"files\":[{}]}}", quote(&a)));
    let files = arr(get(&dts, "outputFiles"));
    assert_eq!(files.len(), 1);
    assert!(str_of(get(&files[0], "fileName")).ends_with("a.d.ts"));
    let e = call_err(&s, "getDeclarationEmit", &format!("{{{sp},\"files\":[{}]}}", quote(&dir.path("missing.ts"))));
    assert!(e.contains("source file not found"), "{e}");
}

#[test]
fn config_file_parsing_diagnostics_round_trip() {
    let dir = TempDir::new("cfgdiag");
    let a = dir.write("a.ts", "export {};\n");
    let s = session(&dir.dir(), false);
    let r = call(&s, "createSnapshot", &format!(
        "{{\"createPrograms\":[{{\"rootFiles\":[{}],\"compilerOptions\":{{}},\"options\":{{\"configFileParsingDiagnostics\":[{{\"pos\":0,\"end\":0,\"code\":5023,\"category\":1,\"text\":\"Unknown compiler option 'x'.\"}}]}}}}]}}",
        quote(&a)
    ));
    let snap = match get(&r, "snapshot") { Value::Number(n) => *n as u64, _ => unreachable!() };
    let project = str_of(get(&r, "operation.createdPrograms.0")).to_string();
    let d = call(&s, "getConfigFileParsingDiagnostics", &format!("{{\"snapshot\":{snap},\"project\":{}}}", quote(&project)));
    assert_eq!(get(&d, "0.code"), &Value::Number(5023.0));
    assert_eq!(str_of(get(&d, "0.text")), "Unknown compiler option 'x'.");
}

#[test]
fn config_file_names_and_source_file_metadata() {
    let dir = TempDir::new("meta");
    let base = dir.write("base.json", r#"{ "compilerOptions": { "strict": true } }"#);
    let cfg = dir.write("tsconfig.json", r#"{ "extends": "./base.json", "compilerOptions": { "noEmit": true, "module": "nodenext" }, "include": ["*.mts"] }"#);
    let a = dir.write("a.mts", "export {};\n");
    let s = session(&dir.dir(), false);
    let r = call(&s, "createSnapshot", &format!("{{\"openProjects\":[{}]}}", quote(&cfg)));
    let snap = match get(&r, "snapshot") { Value::Number(n) => *n as u64, _ => unreachable!() };
    let project = str_of(get(&r, "projects.0.id")).to_string();
    let sp = format!("\"snapshot\":{snap},\"project\":{}", quote(&project));
    let names = call(&s, "getConfigFileNames", &format!("{{{sp}}}"));
    assert_eq!(names, Value::Array(vec![Value::String(cfg.clone()), Value::String(base)]));
    let m = call(&s, "getSourceFileMetadata", &format!("{{{sp},\"file\":{}}}", quote(&a)));
    assert_eq!(get(&m, "isDefaultLibrary"), &Value::Bool(false));
    assert_eq!(get(&m, "impliedNodeFormat"), &Value::Number(99.0));
    let names = call(&s, "getSourceFileNames", &format!("{{{sp}}}"));
    let lib = arr(&names).iter().map(|n| str_of(n).to_string()).find(|n| n.contains("lib.")).expect("a lib file");
    let m = call(&s, "getSourceFileMetadata", &format!("{{{sp},\"file\":{}}}", quote(&lib)));
    assert_eq!(get(&m, "isDefaultLibrary"), &Value::Bool(true));
    assert_eq!(call(&s, "getSourceFileMetadata", &format!("{{{sp},\"file\":{}}}", quote(&dir.path("zz.ts")))), Value::Null);

    // Synthetic programs have no config file.
    let (snap2, p2) = create_program(&s, &[a], "{}");
    assert_eq!(call(&s, "getConfigFileNames", &format!("{{\"snapshot\":{snap2},\"project\":{}}}", quote(&p2))), Value::Array(vec![]));
}

#[test]
fn no_emit_on_error_reports_diagnostics_without_output() {
    let dir = TempDir::new("noemitonerror");
    let a = dir.write("a.ts", "export const a: number = 'x';\n");
    let s = session(&dir.dir(), false);
    let (snap, project) = create_program(&s, &[a], &format!("{{\"noEmitOnError\":true,\"outDir\":{}}}", quote(&dir.path("out"))));
    let sp = format!("\"snapshot\":{snap},\"project\":{}", quote(&project));
    let out = call(&s, "emit", &format!("{{{sp}}}"));
    assert_eq!(get(&out, "emitSkipped"), &Value::Bool(true));
    assert_eq!(get(&out, "diagnostics.0.code"), &Value::Number(2322.0));
    assert!(!std::path::Path::new(&dir.path("out/a.js")).exists());
    let out = call(&s, "emitToString", &format!("{{{sp}}}"));
    assert_eq!(get(&out, "emitSkipped"), &Value::Bool(true));
}

/// Inferred project rebuilt while opening several node_modules files (a program is replaced, and its
/// shared data freed, inside one snapshot build). Freeing the project-reference mapper there was a
/// use-after-free; repeat to make the race likely to show.
#[test]
fn inferred_project_rebuild_frees_safely() {
    let dir = TempDir::new("inferred");
    dir.write("tsconfig.json", r#"{ "compilerOptions": { "strict": true } }"#);
    dir.write("src/index.ts", "export const x = 1;");
    dir.write("node_modules/my-lib/package.json", r#"{"name":"my-lib","types":"./index.d.ts"}"#);
    let a = dir.write("node_modules/my-lib/index.d.ts", "export declare const foo: string;");
    dir.write("node_modules/other-lib/package.json", r#"{"name":"other-lib","types":"./index.d.ts"}"#);
    let b = dir.write("node_modules/other-lib/index.d.ts", "export declare const bar: number;");
    let s = session(&dir.dir(), true);
    for _ in 0..25 {
        let r = call(&s, "createSnapshot", &format!("{{\"openFiles\":[{},{}]}}", quote(&a), quote(&b)));
        let snap = match get(&r, "snapshot") { Value::Number(n) => *n as u64, _ => unreachable!() };
        let dp = call(&s, "getDefaultProjectForFile", &format!("{{\"snapshot\":{snap},\"file\":{}}}", quote(&b)));
        assert!(matches!(dp, Value::Object(_)));
        call(&s, "release", &format!("{{\"snapshot\":{snap}}}"));
    }
}
