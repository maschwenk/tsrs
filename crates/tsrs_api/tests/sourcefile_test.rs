mod common;
use common::*;
use tsrs_api::{Handler, Response};
use tsrs_core::json::{self, Value};

fn program(s: &tsrs_api::Session, root: &str) -> (u64, String) {
    let r = call(s, "createSnapshot", &format!("{{\"createPrograms\":[{{\"rootFiles\":[{}],\"compilerOptions\":{{\"strict\":true}}}}]}}", quote(root)));
    let snap = match get(&r, "snapshot") { Value::Number(n) => *n as u64, _ => unreachable!() };
    (snap, str_of(get(&r, "operation.createdPrograms.0")).to_string())
}

#[test]
fn get_source_file_binary_and_json_and_checker_handles() {
    let dir = TempDir::new("sf");
    let a = dir.write("a.ts", "export const greeting: string = 'hi';\nexport function f(x: number) { return x; }\n");
    // Binary (msgpack mode) response.
    let s = session(&dir.dir(), true);
    let (snap, project) = program(&s, &a);
    let sp = format!("\"snapshot\":{snap},\"project\":{}", quote(&project));
    let bin = match s.handle_request("getSourceFile", format!("{{{sp},\"file\":{}}}", quote(&a)).as_bytes()).unwrap() {
        Response::Binary(b) => b,
        r => panic!("{r:?}"),
    };
    assert!(bin.len() > 100);
    let decoded = tsrs_api_codec::decode_source_file(&bin).expect("decodes");
    let _ = decoded;
    // Missing file: empty RawBinary.
    assert_eq!(s.handle_request("getSourceFile", format!("{{{sp},\"file\":\"/nope.ts\"}}").as_bytes()).unwrap(), Response::Binary(Vec::new()));

    // Checker queries now produce node handles that resolve.
    let pos = "export const ".len();
    let sym = call(&s, "getSymbolAtPosition", &format!("{{{sp},\"file\":{},\"position\":{pos}}}", quote(&a)));
    assert_eq!(str_of(get(&sym, "name")), "greeting", "{}", json::marshal(&sym).unwrap());
    let ty = call(&s, "getTypeAtPosition", &format!("{{{sp},\"file\":{},\"position\":{pos}}}", quote(&a)));
    assert!(matches!(get(&ty, "id"), Value::Number(_)), "{}", json::marshal(&ty).unwrap());

    // JSON mode: { data: base64 }.
    let s2 = session(&dir.dir(), false);
    let (snap2, project2) = program(&s2, &a);
    let r = call(&s2, "getSourceFile", &format!("{{\"snapshot\":{snap2},\"project\":{},\"file\":{}}}", quote(&project2), quote(&a)));
    assert!(str_of(get(&r, "data")).len() > 100);
    assert_eq!(call(&s2, "getSourceFile", &format!("{{\"snapshot\":{snap2},\"project\":{},\"file\":\"/nope.ts\"}}", quote(&project2))), Value::Null);
}

#[test]
fn create_source_file_leases() {
    let dir = TempDir::new("csf");
    let s = session(&dir.dir(), false);
    let r = call(&s, "createSourceFile", r#"{"fileName":"/component.tsx","sourceText":"export const e = <div />;","options":{}}"#);
    assert!(str_of(get(&r, "data")).len() > 50);
    let e = call_err(&s, "createSourceFile", r#"{"fileName":"/x.ts","sourceText":"","options":{"scriptKind":5}}"#);
    assert!(e.contains("invalid scriptKind 5"), "{e}");
    let e = call_err(&s, "releaseSourceFile", r#"{"lease":999}"#);
    assert!(e.contains("source file lease 999 not found"), "{e}");
    assert_eq!(call(&s, "releaseSourceFile", r#"{"lease":1}"#), Value::Bool(true));
    let e = call_err(&s, "retainSourceFile", r#"{"file":{"fileName":"/a.ts","path":"/a.ts","contentHash":"00","parseOptionsKey":"0","scriptKind":3,"nodeId":"1"}}"#);
    assert!(e.contains("invalid source file descriptor"), "{e}");
}

#[test]
fn get_config_source_file_root_and_extended_repeatedly() {
    let dir = TempDir::new("cfgsf");
    let base = dir.write("base.json", r#"{ "compilerOptions": { "strict": true } }"#);
    let cfg = dir.write("tsconfig.json", r#"{ "extends": "./base.json", "include": ["*.ts"] }"#);
    dir.write("a.ts", "export {};\n");
    let s = session(&dir.dir(), true);
    let r = call(&s, "createSnapshot", &format!("{{\"openProjects\":[{}]}}", quote(&cfg)));
    let snap = match get(&r, "snapshot") { Value::Number(n) => *n as u64, _ => unreachable!() };
    let project = str_of(get(&r, "projects.0.id")).to_string();
    let req = |f: &str| format!("{{\"snapshot\":{snap},\"project\":{},\"file\":{}}}", quote(&project), quote(f));
    let get_bin = |f: &str| match s.handle_request("getConfigSourceFile", req(f).as_bytes()).unwrap() {
        Response::Binary(b) => b,
        r => panic!("{r:?}"),
    };
    let root = get_bin(&cfg);
    assert!(!root.is_empty());
    let first = get_bin(&base);
    assert!(!first.is_empty());
    for _ in 0..50 {
        let again = get_bin(&base);
        // Same content each time (the source file id header word differs per fresh parse, like Go).
        assert_eq!(again.len(), first.len());
        tsrs_api_codec::decode_source_file(&again).expect("decodes");
    }
    assert!(get_bin(&dir.path("other.json")).is_empty());
}
