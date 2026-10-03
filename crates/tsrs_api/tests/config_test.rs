mod common;
use common::*;
use tsrs_core::json::{self, Value};

#[test]
fn initialize_and_unknown_methods() {
    let dir = TempDir::new("init");
    let s = session(&dir.dir(), false);
    let r = call(&s, "initialize", "");
    assert_eq!(str_of(get(&r, "currentDirectory")), dir.dir());
    assert!(matches!(get(&r, "useCaseSensitiveFileNames"), Value::Bool(_)));
    assert_eq!(call(&s, "ping", ""), Value::String("pong".into()));
    let e = call_err(&s, "noSuchMethod", "{}");
    assert!(e.starts_with("api: invalid request: unknown API method"), "{e}");
    // Go `noParams`: initialize ignores its payload.
    call(&s, "initialize", "{not json");
    let e = call_err(&s, "parseCommandLine", "{not json");
    assert!(e.starts_with("api: invalid request: failed to unmarshal *api.ParseCommandLineParams:"), "{e}");
}

#[test]
fn parse_config_file_reads_options_and_files() {
    let dir = TempDir::new("cfg");
    let cfg = dir.write("tsconfig.json", r#"{ "compilerOptions": { "strict": true, "target": "es2020", "outDir": "out" }, "include": ["src"] }"#);
    let a = dir.write("src/a.ts", "export const a = 1;");
    let s = session(&dir.dir(), false);
    let r = call(&s, "parseConfigFile", &format!("{{\"file\":{}}}", quote(&cfg)));
    assert_eq!(get(&r, "fileNames"), &Value::Array(vec![Value::String(a.clone())]));
    assert_eq!(get(&r, "options.strict"), &Value::Bool(true));
    assert_eq!(get(&r, "options.target"), &Value::Number(7.0));
    assert_eq!(str_of(get(&r, "options.outDir")), dir.path("out"));
    assert_eq!(get(&r, "errors"), &Value::Array(vec![]));
    assert!(matches!(get(&r, "raw"), Value::Object(_)));

    // Missing file is a client error, not a fake success.
    let e = call_err(&s, "parseConfigFile", &format!("{{\"file\":{}}}", quote(&dir.path("nope.json"))));
    assert!(e.contains("api: client error: could not read file"), "{e}");
}

#[test]
fn read_config_file_and_errors() {
    let dir = TempDir::new("read");
    let cfg = dir.write("tsconfig.json", r#"{ "compilerOptions": { "strict": true, } "#);
    let s = session(&dir.dir(), false);
    let r = call(&s, "readConfigFile", &format!("{{\"file\":{}}}", quote(&cfg)));
    assert_eq!(get(&r, "config.compilerOptions.strict"), &Value::Bool(true));
    assert_eq!(get(&r, "error.category"), &Value::Number(1.0));
    let r = call(&s, "readConfigFile", &format!("{{\"file\":{}}}", quote(&dir.path("missing.json"))));
    assert_eq!(get(&r, "error.code"), &Value::Number(5083.0));
}

#[test]
fn parse_json_config_file_content_and_command_line() {
    let dir = TempDir::new("json");
    let a = dir.write("a.ts", "let x = 1;");
    let s = session(&dir.dir(), false);
    let r = call(&s, "parseJsonConfigFileContent", &format!("{{\"json\":{{\"compilerOptions\":{{\"noEmit\":true}},\"files\":[\"a.ts\"]}},\"configDirectory\":{}}}", quote(&dir.dir())));
    assert_eq!(get(&r, "fileNames"), &Value::Array(vec![Value::String(a)]));
    assert_eq!(get(&r, "options.noEmit"), &Value::Bool(true));
    let e = call_err(&s, "parseJsonConfigFileContent", "{\"json\":{}}");
    assert!(e.contains("exactly one of configDirectory or configFileName"), "{e}");

    let r = call(&s, "parseCommandLine", r#"{"commandLine":["--strict","--target","es2022","x.ts"]}"#);
    assert_eq!(get(&r, "options.strict"), &Value::Bool(true));
    assert_eq!(get(&r, "options.target"), &Value::Number(9.0));
    assert_eq!(json::marshal(get(&r, "fileNames")).unwrap(), "[\"x.ts\"]");
}

#[test]
fn strict_json_params_like_go() {
    let dir = TempDir::new("strict");
    let s = session(&dir.dir(), true);
    let e = call_err(&s, "parseConfigFile", r#"{"file":"/p/\udc00.json"}"#);
    assert!(e.starts_with("api: invalid request: failed to unmarshal *api.ParseConfigFileParams: jsontext:"), "{e}");
    let e = call_err(&s, "createSnapshot", r#"{"fileSystem":{"kind":"full","files":{"/p/a.ts":"x","/p/a.ts":"y"}}}"#);
    assert!(e.contains("failed to unmarshal *api.CreateSnapshotParams") && e.contains("duplicate object member name"), "{e}");
    // noParams methods ignore their payload; echo returns it verbatim.
    call(&s, "initialize", "not json");
    match tsrs_api::Handler::handle_request(&*s, "echo", br#""x\ud800""#).unwrap() {
        tsrs_api::Response::Binary(b) => assert_eq!(b, br#""x\ud800""#.to_vec()),
        r => panic!("{r:?}"),
    }
}

/// Go decodes params into the method's struct: a non-object payload is an invalid request, never a success
/// (runtime review of d6: `[1]` used to create a build orchestrator).
#[test]
fn non_object_params_are_typed_invalid_requests() {
    let dir = TempDir::new("shape");
    let s = session(&dir.dir(), false);
    for (method, ty) in [
        ("createBuildOrchestrator", "CreateBuildOrchestratorParams"),
        ("parseCommandLine", "ParseCommandLineParams"),
        ("createSourceFile", "CreateSourceFileParams"),
        ("transpileModule", "TranspileParams"),
        ("transpileDeclaration", "TranspileParams"),
        ("createSnapshot", "CreateSnapshotParams"),
        ("getTypeAtPosition", "GetTypeAtPositionParams"),
    ] {
        let e = call_err(&s, method, "[1]");
        assert_eq!(e, format!("api: invalid request: failed to unmarshal *api.{ty}: json: cannot unmarshal JSON array into Go api.{ty}"));
        let e = call_err(&s, method, "\"x\"");
        assert!(e.contains("cannot unmarshal JSON string"), "{e}");
    }
    // Wrong field type: an invalid request of the params type.
    let e = call_err(&s, "parseCommandLine", r#"{"commandLine":"x"}"#);
    assert!(e.starts_with("api: invalid request: failed to unmarshal *api.ParseCommandLineParams: json:"), "{e}");
    // `null` decodes to the zero value (Go): parseCommandLine with no args succeeds.
    let r = call(&s, "parseCommandLine", "null");
    assert!(matches!(get(&r, "fileNames"), tsrs_core::json::Value::Array(_)));
}
