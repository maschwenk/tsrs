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

/// Go decodes `null` params into the zero-value struct (runtime review of 7c34965).
#[test]
fn null_params_are_the_zero_value_struct() {
    let dir = TempDir::new("nullparams");
    let s = session(&dir.dir(), false);
    // readConfigFile({}): file "" resolves to the cwd, which cannot be read: config {} plus TS5083.
    let r = call(&s, "readConfigFile", "null");
    assert_eq!(get(&r, "error.code"), &tsrs_core::json::Value::Number(5083.0));
    let r2 = call(&s, "readConfigFile", "{}");
    assert_eq!(r, r2);
    // An explicit null DocumentIdentifier is a decode error (Go's custom decoder), unlike an absent field.
    let e = call_err(&s, "readConfigFile", r#"{"file":null}"#);
    assert_eq!(e, "api: invalid request: failed to unmarshal *api.ReadConfigFileParams: json: cannot unmarshal into Go api.DocumentIdentifier within \"/file\": DocumentIdentifier: expected string or object, got null");
    let e = call_err(&s, "getDefaultProjectForFile", r#"{"snapshot":0,"file":null}"#);
    assert!(e.starts_with("api: invalid request: failed to unmarshal *api.GetDefaultProjectForFileParams:"), "{e}");
    // parseConfigFile({}): a client error (cannot read the file), not an invalid request.
    let e = call_err(&s, "parseConfigFile", "null");
    assert!(e.starts_with("api: client error: could not read file"), "{e}");
    // Source file descriptor field types are decoding errors.
    for method in ["retainSourceFile", "getCachedSourceFile"] {
        let e = call_err(&s, method, r#"{"file":{"fileName":1}}"#);
        assert!(e.starts_with("api: invalid request:"), "{method}: {e}");
        let e = call_err(&s, method, r#"{"file":{"scriptKind":"x"}}"#);
        assert!(e.starts_with("api: invalid request:"), "{method}: {e}");
    }
}

/// Every field of the method's pinned params struct is decoded before lookups; unknown keys are ignored
/// (runtime review of c2abd68: payload matrix in crates/tsrs_api_transport/INTEGRATION.md).
#[test]
fn params_decode_before_lookups_and_ignore_unknown_keys() {
    let dir = TempDir::new("predecode");
    let s = session(&dir.dir(), false);
    for (payload, field) in [
        (r#"{"snapshot":"x"}"#, "/snapshot"),
        (r#"{"snapshot":"x","moduleName":"m"}"#, "/snapshot"),
        (r#"{"resolver":"x"}"#, "/resolver"),
        (r#"{"resolutionMode":"x"}"#, "/resolutionMode"),
        (r#"{"inProgressSnapshot":-1}"#, "/inProgressSnapshot"),
    ] {
        let e = call_err(&s, "resolveModuleName", payload);
        assert!(e.starts_with("api: invalid request: failed to unmarshal *api.ResolveModuleNameParams: json:") && e.contains(field), "{payload}: {e}");
    }
    // A malformed file wins over an unknown snapshot, for checker-lane methods too.
    for method in ["getSymbolAtPosition", "getTypeAtPosition", "getSymbolOfSourceFile", "getCompletionsAtPosition", "getSourceFile"] {
        let e = call_err(&s, method, r#"{"snapshot":999,"project":"p","file":null}"#);
        assert!(e.starts_with("api: invalid request:") && e.contains("DocumentIdentifier: expected string or object, got null"), "{method}: {e}");
        // An absent file is the zero value; the lookup error comes back.
        let e = call_err(&s, method, r#"{"snapshot":999,"project":"p"}"#);
        assert!(e.starts_with("api: client error:"), "{method}: {e}");
    }
    // Unknown keys are ignored (Go): `file` is not a field of these methods.
    call(&s, "transpileModule", r#"{"input":"let x = 1;","options":{},"file":7}"#);
    call(&s, "parseCommandLine", r#"{"commandLine":[],"file":null}"#);
    let r = call(&s, "createSnapshot", r#"{"file":null,"extra":[1]}"#);
    assert!(matches!(get(&r, "snapshot"), tsrs_core::json::Value::Number(_)));
    call(&s, "batchRequests", r#"{"requests":[],"file":null}"#);
    let e = call_err(&s, "release", r#"{"snapshot":0,"file":null}"#);
    assert!(e.contains("empty handle"), "{e}");
}

/// Runtime field/value matrix on f70371e (crates/tsrs_api_transport/INTEGRATION.md, "419 class mismatches").
#[test]
fn field_values_match_go_classes() {
    let dir = TempDir::new("fieldvalues");
    let s = session(&dir.dir(), false);
    // Accepted by Go: signed int field, null elements of []string / []project.ID.
    call(&s, "batchRequests", r#"{"requests":[],"maxResponseBytesPerPage":-1}"#);
    let r = call(&s, "parseCommandLine", r#"{"commandLine":[null]}"#);
    assert!(matches!(get(&r, "fileNames"), tsrs_core::json::Value::Array(_)));
    call(&s, "createSnapshot", r#"{"ensurePrograms":[null]}"#);
    // Exponent / fraction syntax for integers is invalid syntax in Go, before any lookup.
    for (method, payload) in [
        ("release", r#"{"snapshot":1e3}"#),
        ("release", r#"{"snapshot":1.0}"#),
        ("getSourceFileNames", r#"{"snapshot":1E2,"project":"p"}"#),
        ("getTypesAtPositions", r#"{"snapshot":1,"project":"p","file":"/a.ts","positions":[1e1]}"#),
    ] {
        let e = call_err(&s, method, payload);
        assert!(e.starts_with("api: invalid request: failed to unmarshal"), "{method} {payload}: {e}");
    }
    // Out of the Go type's range.
    for (method, payload) in [
        ("resolveModuleName", r#"{"resolutionMode":4294967296}"#),
        ("resolveModuleName", r#"{"resolutionMode":2147483648}"#),
        ("release", r#"{"snapshot":18446744073709551616}"#),
        ("getTypesAtPositions", r#"{"snapshot":1,"project":"p","file":"/a.ts","positions":[4294967296]}"#),
    ] {
        let e = call_err(&s, method, payload);
        assert!(e.starts_with("api: invalid request: failed to unmarshal"), "{method} {payload}: {e}");
    }
    // Array element kinds are part of decoding: no lookup, no per-item batch error.
    for (method, payload) in [
        ("getSymbolsAtLocations", r#"{"snapshot":0,"project":"p","locations":[1]}"#),
        ("getTypesAtPositions", r#"{"snapshot":0,"project":"p","file":"/a.ts","positions":["x"]}"#),
        ("batchRequests", r#"{"requests":[1]}"#),
    ] {
        let e = call_err(&s, method, payload);
        assert!(e.starts_with("api: invalid request: failed to unmarshal"), "{method} {payload}: {e}");
    }
    // A large uint64 in range is a lookup, as in Go.
    let e = call_err(&s, "release", r#"{"snapshot":4294967296000}"#);
    assert!(e.starts_with("api: client error: snapshot"), "{e}");
}

/// Integer bounds use the exact literal, IDs keep their exact value, and escaped member names are matched like
/// the decoded object keys (review of b2769b8).
#[test]
fn exact_integer_literals_and_escaped_keys() {
    let dir = TempDir::new("exactints");
    let s = session(&dir.dir(), false);
    // u64::MAX and i64-range values are in range (f64 would round them out of range).
    let e = call_err(&s, "release", r#"{"snapshot":18446744073709551615}"#);
    assert_eq!(e, "api: client error: snapshot 18446744073709551615 not found");
    let e = call_err(&s, "release", r#"{"snapshot":18446744073709551616}"#);
    assert!(e.starts_with("api: invalid request:"), "{e}");
    // An exact ID above 2^53 is looked up as written.
    let e = call_err(&s, "release", r#"{"snapshot":9007199254740993}"#);
    assert_eq!(e, "api: client error: snapshot 9007199254740993 not found");
    // batchRequests maxResponseBytesPerPage is a Go int: i64::MAX is in range, 2^63 is not.
    call(&s, "batchRequests", r#"{"requests":[],"maxResponseBytesPerPage":9223372036854775807}"#);
    let e = call_err(&s, "batchRequests", r#"{"requests":[],"maxResponseBytesPerPage":9223372036854775808}"#);
    assert!(e.starts_with("api: invalid request:"), "{e}");
    // An escaped member name is the same field.
    let e = call_err(&s, "release", r#"{"snap\u0073hot":1e3}"#);
    assert!(e.starts_with("api: invalid request: failed to unmarshal *api.ReleaseParams"), "{e}");
}

/// `removePrograms` elements are project.SyntheticProjectID (custom decoder): `null` is rejected before any
/// snapshot is allocated, so the next snapshot ID is unchanged (runtime review of b2769b8).
#[test]
fn remove_programs_null_is_rejected_before_allocation() {
    let dir = TempDir::new("removenull");
    let s = session(&dir.dir(), false);
    let first = call(&s, "createSnapshot", "{}");
    let first_id = match get(&first, "snapshot") { tsrs_core::json::Value::Number(n) => *n as u64, _ => unreachable!() };
    let e = call_err(&s, "createSnapshot", r#"{"removePrograms":[null]}"#);
    assert!(e.starts_with("api: invalid request: failed to unmarshal *api.CreateSnapshotParams: json: cannot unmarshal into Go project.SyntheticProjectID within \"/removePrograms/0\""), "{e}");
    let next = call(&s, "createSnapshot", "{}");
    let next_id = match get(&next, "snapshot") { tsrs_core::json::Value::Number(n) => *n as u64, _ => unreachable!() };
    assert_eq!(next_id, first_id + 1, "the rejected request must not consume a snapshot ID");
    // ensurePrograms is []project.ID: a null element is the zero ID (accepted).
    call(&s, "createSnapshot", r#"{"ensurePrograms":[null]}"#);
}

/// Nested api structs are decoded like Go (whole params struct): integer syntax and exact values apply to the
/// value's own literal, never to a same-named top-level field (runtime af8 review).
#[test]
fn nested_integer_literals_are_exact_and_checked() {
    let dir = TempDir::new("nestedints");
    let s = session(&dir.dir(), false);
    let program = |resolver: &str| format!(r#"{{"moduleResolver":9007199254740993,"createPrograms":[{{"rootFiles":[],"compilerOptions":{{}},"options":{{"moduleResolver":{resolver}}}}}]}}"#);
    let e = call_err(&s, "createSnapshot", &program("9007199254740992"));
    assert_eq!(e, "api: client error: module resolver 9007199254740992 not found");
    let e = call_err(&s, "createSnapshot", &program("9007199254740993"));
    assert_eq!(e, "api: client error: module resolver 9007199254740993 not found");
    for bad in ["1e3", "1.0", "18446744073709551616", "-1", "\"x\""] {
        let e = call_err(&s, "createSnapshot", &program(bad));
        assert!(e.starts_with("api: invalid request: failed to unmarshal *api.CreateSnapshotParams"), "{bad}: {e}");
    }
    // Synthetic project IDs are decoded by project.SyntheticProjectID (invalid text is a decode error).
    for payload in [r#"{"removePrograms":["x"]}"#, r#"{"reconfigurePrograms":[{"id":"x","rootFiles":[],"compilerOptions":{}}]}"#] {
        let e = call_err(&s, "createSnapshot", payload);
        assert!(e.starts_with("api: invalid request: failed to unmarshal *api.CreateSnapshotParams"), "{payload}: {e}");
    }
    // Nested wrong kinds inside arrays of structs.
    let e = call_err(&s, "getTypesOfSymbols", r#"{"snapshot":1,"project":"p","symbols":[{"id":1e3}]}"#);
    assert!(e.starts_with("api: invalid request: failed to unmarshal"), "{e}");
}

/// Request params from runtime's c246 session review (REPORT.md findings 2-5), sent verbatim.
#[test]
fn external_struct_decoding_matches_go_classes() {
    let dir = TempDir::new("external");
    let a = dir.write("a.ts", "export const a = 1;\n");
    let s = session(&dir.dir(), false);
    let f = quote(&a);
    let snapshot = |r: &tsrs_core::json::Value| match get(r, "snapshot") { tsrs_core::json::Value::Number(n) => *n as u64, _ => unreachable!() };
    let invalid = |method: &str, payload: &str| {
        let e = call_err(&s, method, payload);
        assert!(e.starts_with("api: invalid request:"), "{method} {payload}: {e}");
    };
    // CompilerOptions: Go keeps an unknown int32 enum value (echoed back); exponent syntax is invalid; malformed
    // tristates and unknown options are ignored.
    call(&s, "transpileModule", r#"{"input":"const x: number = 1;","options":{"compilerOptions":{"target":12345}}}"#);
    let r = call(&s, "createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{f}],"compilerOptions":{{"target":12345,"module":1}}}}]}}"#));
    assert_eq!(get(&r, "projects.0.compilerOptions.target"), &tsrs_core::json::Value::Number(12345.0), "{}", tsrs_core::json::marshal(&r).unwrap());
    assert_eq!(get(&r, "projects.0.parsedCommandLine.options.target"), &tsrs_core::json::Value::Number(12345.0));
    assert_eq!(get(&r, "projects.0.compilerOptions.module"), &tsrs_core::json::Value::Number(1.0));
    invalid("transpileModule", r#"{"input":"const x: number = 1;","options":{"compilerOptions":{"target":1e1}}}"#);
    invalid("createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{f}],"compilerOptions":{{"target":1e1}}}}]}}"#));
    invalid("transpileModule", r#"{"input":"const x: number = 1;","options":{"compilerOptions":{"target":9007199254740993}}}"#);
    for co in [r#"{"strict":"yes"}"#, r#"{"strict":null}"#, r#"{"strict":1}"#, r#"{"bogusOption":true}"#, r#"{"noEmit":{}}"#] {
        call(&s, "transpileModule", &format!(r#"{{"input":"const x: number = 1;","options":{{"compilerOptions":{co}}}}}"#));
    }
    // BuildOptions.builders is a Go *int.
    invalid("createBuildOrchestrator", r#"{"rootNames":["/x/tsconfig.json"],"buildOptions":{"builders":1e1}}"#);
    // Request filesystem: wrong JSON kinds are decode errors; null values are zero values.
    let base = snapshot(&call(&s, "createSnapshot", "{}"));
    for fs in [
        r#"{"kind":1}"#,
        r#"{"kind":"layer","files":{"/v/x.ts":5}}"#,
        r#"{"kind":"layer","files":["x"]}"#,
        r#"{"kind":"layer","directories":{"/v":{"files":"x"}}}"#,
        r#"{"kind":"layer","symlinks":{"/v/l":{"target":5}}}"#,
        r#"{"kind":"layer","symlinks":{"/v/l":{"target":"/v","host":"yes"}}}"#,
        r#"{"kind":"layer","removedPaths":[1]}"#,
    ] {
        invalid("createSnapshot", &format!(r#"{{"fileSystem":{fs}}}"#));
        invalid("updateSnapshot", &format!(r#"{{"snapshot":{base},"changes":{{"fileSystem":{fs}}}}}"#));
    }
    for fs in [r#"{"kind":"layer","removedPaths":[null]}"#, r#"{"kind":"layer","files":{"/v/x.ts":null}}"#] {
        call(&s, "createSnapshot", &format!(r#"{{"fileSystem":{fs}}}"#));
        call(&s, "updateSnapshot", &format!(r#"{{"snapshot":{base},"changes":{{"fileSystem":{fs}}}}}"#));
    }
    // Project references: pinned Go crashes on these; tsrs answers a stable error (relative/null) or reports
    // the missing project as a program diagnostic, and stays up.
    for refs in [r#"[{"path":"../q"}]"#, r#"[null]"#, r#"[{"path":"x"}]"#] {
        let e = call_err(&s, "createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{f}],"compilerOptions":{{}},"options":{{"projectReferences":{refs}}}}}]}}"#));
        assert!(e.starts_with("api: client error: projectReferences[0].path must be an absolute path"), "{refs}: {e}");
    }
    let r = call(&s, "createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{f}],"compilerOptions":{{}},"options":{{"projectReferences":[{{"path":"/nonexistent/q"}}]}}}}]}}"#));
    let project = str_of(get(&r, "projects.0.id")).to_string();
    let d = call(&s, "getProgramDiagnostics", &format!(r#"{{"snapshot":{},"project":{}}}"#, snapshot(&r), quote(&project)));
    assert!(tsrs_core::json::marshal(&d).unwrap().contains("not found"), "{}", tsrs_core::json::marshal(&d).unwrap());
}

/// runtime-f2-review (f2_enum_cases.json): unknown int32 enum values compile like pinned Go, and Go `*int`
/// options keep their exact 64-bit value.
#[test]
fn unknown_enum_values_and_go_int_options_match_go() {
    let dir = TempDir::new("f2enums");
    let a = dir.write("a.ts", "export const a = 1;\n");
    let j = dir.write("j.tsx", "const e = <div a=\"1\"></div>;\nexport { e };\n");
    let s = session(&dir.dir(), false);
    let raw = |method: &str, payload: &str| match tsrs_api::Handler::handle_request(&*s, method, payload.as_bytes()) {
        Ok(tsrs_api::Response::Json(t)) => Ok(t),
        Ok(_) => unreachable!(),
        Err(e) => Err(e.to_string()),
    };
    let input = r#"class C { x = 1; #p = 2; m() { return this.#p ** 2; } }\nasync function f() { await 1; for await (const z of []) {} }\nexport const o: any = {}; export const v = o?.b ?? 1;\n"#;
    let transpile = |options: &str| raw("transpileModule", &format!(r#"{{"input":"{input}","options":{{"compilerOptions":{options}}}}}"#)).unwrap();
    // target: Go's switch defaults transform maximally; comparisons see the raw number.
    for target in ["12345", "-1", "2147483647"] {
        let out = transpile(&format!(r#"{{"target":{target}}}"#));
        assert!(out.contains("__awaiter"), "target {target}: {out}");
    }
    assert!(!transpile(r#"{"target":99}"#).contains("__awaiter"));
    // module: Go's default module transformer is CommonJS.
    let module_input = r#"import { y } from './y';\nexport const x = y;\n"#;
    for module in ["12345", "-1"] {
        let out = raw("transpileModule", &format!(r#"{{"input":"{module_input}","options":{{"compilerOptions":{{"module":{module},"target":99}}}}}}"#)).unwrap();
        assert!(out.contains("require(") && out.contains("exports"), "module {module}: {out}");
    }
    // jsx: an unknown value is not "unset" (no 17004), and preserves JSX on emit.
    let r = call(&s, "createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{}],"compilerOptions":{{"jsx":12345,"target":99,"strict":true}}}}]}}"#, quote(&j)));
    let snap = match get(&r, "snapshot") { tsrs_core::json::Value::Number(n) => *n as u64, _ => unreachable!() };
    let diags = raw("getSemanticDiagnostics", &format!(r#"{{"snapshot":{snap},"project":"/dev/null/synthetic/1"}}"#)).unwrap();
    assert!(diags.contains("\"code\":7026") && !diags.contains("\"code\":17004"), "{diags}");
    assert!(json_text(&r).contains("\"jsx\":12345"));
    // moduleResolution with no named kind: an import-free program works as in Go; resolving a module is where
    // pinned Go panics, and tsrs answers a stable client error and stays usable.
    call(&s, "createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{}],"compilerOptions":{{"moduleResolution":12345}}}}]}}"#, quote(&a)));
    let imp = dir.write("imp.ts", "import { a } from './a';\nexport const b = a;\n");
    let e = call_err(&s, "createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{}],"compilerOptions":{{"moduleResolution":-1}}}}]}}"#, quote(&imp)));
    assert_eq!(e, "api: client error: unsupported moduleResolution value -1 (not a ModuleResolutionKind)");
    call(&s, "createSnapshot", "{}");
    // Go `*int`: any int64, echoed exactly; beyond int64, fractions and exponents are invalid (accepted builders
    // values are covered with the CLI build backend in tsrs_cli `api::tests`).
    for n in ["2147483648", "-2147483649", "9223372036854775807"] {
        let out = raw("createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{}],"compilerOptions":{{"maxNodeModuleJsDepth":{n}}}}}]}}"#, quote(&a))).unwrap();
        assert!(out.contains(&format!("\"maxNodeModuleJsDepth\":{n}")), "{n}: {out}");
    }
    // `checkers` is a Go *int too; the checker pool clamps it like Go (max(min(n, files, 256), 1)).
    for n in ["2147483648", "9223372036854775807", "-1", "0", "4"] {
        let out = raw("createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{}],"compilerOptions":{{"checkers":{n}}}}}]}}"#, quote(&a))).unwrap();
        assert!(out.contains(&format!("\"checkers\":{n}")) || n == "0", "{n}: {out}");
        let snap = match get(&tsrs_core::json::unmarshal(&out).unwrap(), "snapshot") { tsrs_core::json::Value::Number(n) => *n as u64, _ => unreachable!() };
        assert_eq!(raw("getSemanticDiagnostics", &format!(r#"{{"snapshot":{snap},"project":"/dev/null/synthetic/1"}}"#)).unwrap(), "[]");
        raw("transpileModule", &format!(r#"{{"input":"const x = 1;","options":{{"compilerOptions":{{"checkers":{n}}}}}}}"#)).unwrap();
    }
    for n in ["9223372036854775808", "1.5", "1e0", "1.0"] {
        let e = raw("createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{}],"compilerOptions":{{"checkers":{n}}}}}]}}"#, quote(&a))).unwrap_err();
        assert!(e.starts_with("api: invalid request:"), "checkers {n}: {e}");
        let e = raw("transpileModule", &format!(r#"{{"input":"const x = 1;","options":{{"compilerOptions":{{"checkers":{n}}}}}}}"#)).unwrap_err();
        assert!(e.starts_with("api: invalid request:"), "checkers {n}: {e}");
    }
    for n in ["9223372036854775808", "1.5", "1e0", "1.0"] {
        let e = raw("createSnapshot", &format!(r#"{{"createPrograms":[{{"rootFiles":[{}],"compilerOptions":{{"maxNodeModuleJsDepth":{n}}}}}]}}"#, quote(&a))).unwrap_err();
        assert!(e.starts_with("api: invalid request:"), "{n}: {e}");
        let e = raw("createBuildOrchestrator", &format!(r#"{{"rootNames":["/x/tsconfig.json"],"buildOptions":{{"builders":{n}}}}}"#)).unwrap_err();
        assert!(e.starts_with("api: invalid request:"), "{n}: {e}");
    }
}

fn json_text(v: &tsrs_core::json::Value) -> String {
    tsrs_core::json::marshal(v).unwrap()
}
