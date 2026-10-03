// Integrated regression tests through core's real `Session` (codec node handles, source-file
// descriptors and leases, transport-independent request dispatch), compared with responses recorded
// from the pinned Go session (testdata/go_probe/go_shapes_b85298b6.jsonl, see regen.sh).
//
// Compared as JSON values after one normalization: ids that are per-process or per-checker counters
// (type / signature / symbol ids, source-file node ids) are renamed per line in order of first
// appearance, so identity relations (e.g. `target == id`, `freshType == regularType`) are still
// checked, but absolute counter values are not; the same applies to the symbol id embedded in the
// escaped name of a well-known-symbol member (`__@iterator@<id>`). Node handles, content hashes, flags, names, values,
// field presence (encoding/json v2 omitempty / nil-slice rules) and error texts are compared as is.

use std::collections::HashMap;
use std::sync::Arc;

use tsrs_core::json::{self, Value};
use tsrs_vfs::{bundled, vfstest, FS};

use crate::handler::{Handler, Response};
use crate::session::{Session, SessionOptions};

const MAIN: &str = include_str!("testdata/go_probe/fixture/main.ts");
const TYPES: &str = include_str!("testdata/go_probe/fixture/types.d.ts");
const TSCONFIG: &str = include_str!("testdata/go_probe/fixture/tsconfig.json");
const OTHER: &str = include_str!("testdata/go_probe/fixture/other.ts");
const GO_SHAPES: &str = include_str!("testdata/go_probe/go_shapes_b85298b6.jsonl");

const ID_KEYS: &[&str] = &[
    "id", "target", "freshType", "regularType", "thisType", "typeParameter", "constraintType", "nameType", "templateType", "objectType",
    "indexType", "checkType", "extendsType", "baseType", "substConstraint", "unknown", "undefined", "arguments",
];
const ID_ARRAY_KEYS: &[&str] = &["typeParameters", "outerTypeParameters", "localTypeParameters", "aliasTypeArguments"];

#[derive(Default)]
struct Ids {
    numbers: HashMap<u64, usize>,
    symbols: HashMap<u64, usize>,
    strings: HashMap<String, usize>,
}

/// Keys whose object values are symbol references (their `id` is a symbol id, a separate counter).
const SYMBOL_KEYS: &[&str] = &["symbol", "aliasSymbol", "parent", "exportSymbol", "reference", "thisParameter", "parameters"];

impl Ids {
    fn num(&mut self, n: f64) -> Value {
        let next = self.numbers.len() + 1;
        Value::String(format!("#{}", self.numbers.entry(n as u64).or_insert(next)))
    }
    fn sym(&mut self, n: f64) -> Value {
        let next = self.symbols.len() + 1;
        Value::String(format!("$${}", self.symbols.entry(n as u64).or_insert(next)))
    }
    fn str(&mut self, s: &str) -> Value {
        let next = self.strings.len() + 1;
        Value::String(format!("@{}", self.strings.entry(s.to_string()).or_insert(next)))
    }
}

fn normalize(v: &Value, ids: &mut Ids, in_symbol: bool) -> Value {
    match v {
        Value::Object(o) => {
            let mut out = tsrs_core::collections::OrderedMap::default();
            // Visit keys sorted so first-appearance numbering does not depend on field order.
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            for k in keys {
                let child = &o[k];
                let nv = match (k.as_str(), child) {
                    ("id", Value::Number(n)) if in_symbol => ids.sym(*n),
                    (key, Value::Number(n)) if ID_KEYS.contains(&key) => ids.num(*n),
                    (key, Value::Array(a)) if ID_ARRAY_KEYS.contains(&key) => {
                        Value::Array(a.iter().map(|x| if let Value::Number(n) = x { ids.num(*n) } else { x.clone() }).collect())
                    }
                    ("file", Value::String(s)) | ("nodeId", Value::String(s)) => ids.str(s),
                    // Escaped names of well-known-symbol members embed the unique ES symbol's
                    // process-wide symbol id: `__@iterator@1360`.
                    ("name", Value::String(s)) if s.starts_with("__@") && s.rfind('@').is_some_and(|i| i > 2 && s[i + 1..].bytes().all(|b| b.is_ascii_digit())) => {
                        let i = s.rfind('@').unwrap();
                        Value::String(format!("{}@<symbol id>", &s[..i]))
                    }
                    (key, _) => normalize(child, ids, SYMBOL_KEYS.contains(&key)),
                };
                out.insert(k.clone(), nv);
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| normalize(x, ids, in_symbol)).collect()),
        other => other.clone(),
    }
}

struct S {
    session: Arc<Session>,
    snapshot: f64,
    project: String,
    lines: Vec<(String, Value)>,
}

fn utf16_pos(text: &str, needle: &str) -> usize {
    text[..text.find(needle).unwrap_or_else(|| panic!("{needle:?}"))].encode_utf16().count()
}

fn get(v: &Value, k: &str) -> Value {
    match v {
        Value::Object(o) => o.get(k).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

impl S {
    fn call(&self, method: &str, params: &Value) -> Result<Value, String> {
        let text = json::marshal(params).unwrap();
        match self.session.handle_request(method, text.as_bytes()) {
            Ok(Response::Json(t)) => Ok(json::unmarshal(&t).unwrap()),
            Ok(Response::Binary(b)) => panic!("{method}: unexpected binary ({} bytes)", b.len()),
            Err(e) => Err(e.to_string().lines().next().unwrap_or_default().to_string()),
        }
    }

    fn sp(&self, extra: &[(&str, Value)]) -> Value {
        let mut o = tsrs_core::collections::OrderedMap::default();
        o.insert("snapshot".to_string(), Value::Number(self.snapshot));
        o.insert("project".to_string(), Value::String(self.project.clone()));
        for (k, v) in extra {
            o.insert(k.to_string(), v.clone());
        }
        Value::Object(o)
    }

    fn at(&self, needle: &str) -> Value {
        self.sp(&[("file", Value::String("/p/main.ts".into())), ("position", Value::Number(utf16_pos(MAIN, needle) as f64))])
    }

    fn sym_ref(&self, needle: &str) -> Value {
        get(&self.call("getSymbolAtPosition", &self.at(needle)).unwrap(), "reference")
    }

    fn shape(&mut self, label: &str, method: &str, params: Value) -> Value {
        match self.call(method, &params) {
            Ok(r) => {
                self.lines.push((format!("shape:{label}"), r.clone()));
                r
            }
            Err(e) => {
                self.lines.push((format!("shape:{label}"), obj(&[("error", Value::String(e))])));
                Value::Null
            }
        }
    }

    fn wrong(&mut self, label: &str, method: &str, params: Value) {
        let r = match self.call(method, &params) {
            Ok(r) => obj(&[("result", r)]),
            Err(e) => obj(&[("error", Value::String(e))]),
        };
        self.lines.push((format!("wrong:{label}"), r));
    }
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    let mut o = tsrs_core::collections::OrderedMap::default();
    for (k, v) in pairs {
        o.insert(k.to_string(), v.clone());
    }
    Value::Object(o)
}

fn n(x: impl Into<f64>) -> Value {
    Value::Number(x.into())
}

fn session() -> S {
    let files = [("/p/tsconfig.json", TSCONFIG), ("/p/main.ts", MAIN), ("/p/types.d.ts", TYPES), ("/p/other.ts", OTHER)];
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|(k, v)| (k.to_string(), v.to_string())), false)));
    let session = Session::new(SessionOptions::new("/".to_string(), bundled::lib_path(), fs, false));
    let mut s = S { session, snapshot: 0.0, project: String::new(), lines: Vec::new() };
    let snap = s.call("createSnapshot", &obj(&[("openProjects", Value::Array(vec![Value::String("/p/tsconfig.json".into())]))])).unwrap();
    s.snapshot = match get(&snap, "snapshot") {
        Value::Number(x) => x,
        other => panic!("{other:?}"),
    };
    s.project = match get(&match get(&snap, "projects") {
        Value::Array(a) => a[0].clone(),
        other => panic!("{other:?}"),
    }, "id")
    {
        Value::String(p) => p,
        other => panic!("{other:?}"),
    };
    s
}

/// Same request sequence as `TestTsrsCheckerShapes` in testdata/go_probe/zz_tsrs_probe_test.go.
fn shapes() -> Vec<(String, Value)> {
    let mut s = session();
    let lit = s.shape("type.literal", "getTypeAtPosition", s.at("lit ="));
    let boxt = s.shape("type.reference", "getTypeAtPosition", s.at("box:"));
    s.shape("type.union", "getTypeAtPosition", s.at("maybe:"));
    s.shape("type.bigint", "getTypeAtPosition", s.at("big ="));
    let pair = s.call("getDeclaredTypeOfSymbol", &s.sp(&[("symbol", s.sym_ref("Pair<A, B>"))])).unwrap();
    s.shape("type.tupleTarget", "getTargetOfType", s.sp(&[("objectId", get(&pair, "id"))]));
    let box_target = s.shape("type.interfaceTarget", "getTargetOfType", s.sp(&[("objectId", get(&boxt, "id"))]));
    s.shape("type.typeParameter", "getLocalTypeParametersOfType", s.sp(&[("objectId", get(&box_target, "id"))]));
    s.shape("type.thisType", "getThisTypeOfType", s.sp(&[("objectId", get(&box_target, "id"))]));
    for (label, needle) in [("type.mapped", "M = "), ("type.conditional", "C<T> ="), ("type.templateLiteral", "TL = "), ("type.indexedAccess", "IA<T, K")] {
        let params = s.sp(&[("symbol", s.sym_ref(needle))]);
        s.shape(label, "getDeclaredTypeOfSymbol", params);
    }
    let str_t = s.call("getStringType", &s.sp(&[])).unwrap();
    s.shape("type.intrinsic", "getStringType", s.sp(&[]));
    s.shape("indexInfos", "getIndexInfosOfType", s.sp(&[("type", get(&boxt, "id"))]));
    s.shape("indexInfo.number.none", "getIndexInfoOfType", s.sp(&[("type", get(&str_t, "id")), ("kind", n(1))]));
    s.shape("symbol.variable", "getSymbolAtPosition", s.at("box:"));
    s.shape("symbol.class", "getSymbolAtPosition", s.at("Dog extends"));
    s.shape("symbol.global", "resolveName", s.sp(&[("name", Value::String("Array".into())), ("meaning", n(tsrs_ast::SymbolFlags::Type.bits()))]));
    s.shape("symbol.none", "resolveName", s.sp(&[("name", Value::String("nope".into())), ("meaning", n(tsrs_ast::SymbolFlags::Value.bits()))]));
    s.shape("members.empty", "getMembersOfSymbol", obj(&[("symbol", s.sym_ref("box:"))]));
    s.shape("exports.empty", "getExportsOfSymbol", obj(&[("symbol", s.sym_ref("box:"))]));
    s.shape("properties.empty", "getPropertiesOfType", s.sp(&[("type", get(&str_t, "id"))]));
    s.shape("typeArguments", "getTypeArguments", s.sp(&[("type", get(&boxt, "id"))]));
    s.shape("aliasTypeArguments.empty", "getAliasTypeArgumentsOfType", s.sp(&[("objectId", get(&boxt, "id"))]));
    let over = s.call("getTypeAtPosition", &s.at("over(x: string)")).unwrap();
    let sigs = s.shape("signatures", "getSignaturesOfType", s.sp(&[("type", get(&over, "id")), ("kind", n(0))]));
    s.shape("signatures.construct.empty", "getSignaturesOfType", s.sp(&[("type", get(&over, "id")), ("kind", n(1))]));
    let sig0 = match &sigs {
        Value::Array(a) => a[0].clone(),
        other => panic!("{other:?}"),
    };
    s.shape("signature.typeParameters.empty", "getTypeParametersOfSignature", s.sp(&[("objectId", get(&sig0, "id"))]));
    s.shape("signature.thisParameter.none", "getThisParameterOfSignature", s.sp(&[("objectId", get(&sig0, "id"))]));
    let is_str = s.call("getTypeAtPosition", &s.at("isStr(")).unwrap();
    let is_str_sigs = s.call("getSignaturesOfType", &s.sp(&[("type", get(&is_str, "id")), ("kind", n(0))])).unwrap();
    let is_str_sig = match &is_str_sigs {
        Value::Array(a) => a[0].clone(),
        other => panic!("{other:?}"),
    };
    s.shape("typePredicate", "getTypePredicateOfSignature", s.sp(&[("signature", get(&is_str_sig, "id"))]));
    s.shape("typePredicate.none", "getTypePredicateOfSignature", s.sp(&[("signature", get(&sig0, "id"))]));
    s.shape("jsDocTags", "getJsDocTags", s.sp(&[("symbol", s.sym_ref("Animal {"))]));
    s.shape("jsDocTags.none", "getJsDocTags", s.sp(&[("symbol", s.sym_ref("box:"))]));
    s.shape("exportsOfModule.none", "getExportsOfModule", s.sp(&[("symbol", s.sym_ref("box:"))]));
    s.shape("baseTypes.none", "getBaseTypes", s.sp(&[("type", get(&box_target, "id"))]));
    let first_decl = |s: &S, needle: &str| match get(&s.call("getSymbolAtPosition", &s.at(needle)).unwrap(), "declarations") {
        Value::Array(a) => a[0].clone(),
        other => panic!("{other:?}"),
    };
    let box_decl = first_decl(&s, "box:");
    let red_decl = first_decl(&s, "Red =");
    s.shape("constantValue.none", "getConstantValue", s.sp(&[("location", box_decl)]));
    s.shape("constantValue.number", "getConstantValue", s.sp(&[("location", red_decl)]));
    s.shape(
        "completions",
        "getCompletionsAtPosition",
        s.sp(&[("file", Value::String("/p/main.ts".into())), ("position", n((utf16_pos(MAIN, "box.value") + 4) as f64))]),
    );
    s.shape("wellKnownSignatures", "getWellKnownSignatures", s.sp(&[]));

    let (lit_id, obj_id) = (get(&lit, "id"), get(&boxt, "id"));
    let o = |k: &str| k.to_string();
    let _ = o;
    for (label, method, key, id) in [
        ("getTypeArguments(literal)", "getTypeArguments", "type", &lit_id),
        ("getBaseTypes(literal)", "getBaseTypes", "type", &lit_id),
        ("getTargetOfType(literal)", "getTargetOfType", "objectId", &lit_id),
        ("getFreshTypeOfType(object)", "getFreshTypeOfType", "objectId", &obj_id),
        ("getRegularTypeOfType(object)", "getRegularTypeOfType", "objectId", &obj_id),
        ("getTypesOfType(literal)", "getTypesOfType", "objectId", &lit_id),
        ("getTypeParametersOfType(literal)", "getTypeParametersOfType", "objectId", &lit_id),
        ("getOuterTypeParametersOfType(literal)", "getOuterTypeParametersOfType", "objectId", &lit_id),
        ("getLocalTypeParametersOfType(literal)", "getLocalTypeParametersOfType", "objectId", &lit_id),
        ("getThisTypeOfType(literal)", "getThisTypeOfType", "objectId", &lit_id),
        ("getObjectTypeOfType(literal)", "getObjectTypeOfType", "objectId", &lit_id),
        ("getIndexTypeOfType(literal)", "getIndexTypeOfType", "objectId", &lit_id),
        ("getCheckTypeOfType(literal)", "getCheckTypeOfType", "objectId", &lit_id),
        ("getExtendsTypeOfType(literal)", "getExtendsTypeOfType", "objectId", &lit_id),
        ("getBaseTypeOfType(literal)", "getBaseTypeOfType", "objectId", &lit_id),
        ("getConstraintOfType(literal)", "getConstraintOfType", "objectId", &lit_id),
        ("getTypeParameterOfMappedType(literal)", "getTypeParameterOfMappedType", "objectId", &lit_id),
        ("getConstraintTypeOfMappedType(literal)", "getConstraintTypeOfMappedType", "objectId", &lit_id),
        ("getNameTypeOfMappedType(literal)", "getNameTypeOfMappedType", "objectId", &lit_id),
        ("getTemplateTypeOfMappedType(literal)", "getTemplateTypeOfMappedType", "objectId", &lit_id),
        ("getTrueTypeOfConditionalType(literal)", "getTrueTypeOfConditionalType", "objectId", &lit_id),
        ("getFalseTypeOfConditionalType(literal)", "getFalseTypeOfConditionalType", "objectId", &lit_id),
        ("getConstraintOfTypeParameter(literal)", "getConstraintOfTypeParameter", "objectId", &lit_id),
        ("getDefaultFromTypeParameter(literal)", "getDefaultFromTypeParameter", "objectId", &lit_id),
    ] {
        let params = s.sp(&[(key, id.clone())]);
        s.wrong(label, method, params);
    }
    let p = s.sp(&[("type", obj_id.clone()), ("kind", n(7))]);
    s.wrong("getSignaturesOfType(kind 7)", "getSignaturesOfType", p);
    let p = s.sp(&[("type", obj_id.clone()), ("kind", n(7))]);
    s.wrong("getIndexInfoOfType(kind 7)", "getIndexInfoOfType", p);
    let p = s.sp(&[("signature", get(&sig0, "id")), ("kind", n(100000))]);
    s.wrong("signatureToSignatureDeclaration(kind 100000)", "signatureToSignatureDeclaration", p);
    let p = s.sp(&[("signature", get(&sig0, "id")), ("index", n(-1))]);
    s.wrong("getParameterType(index -1)", "getParameterType", p);
    s.lines
}

#[test]
fn session_responses_match_pinned_go() {
    let ours: HashMap<String, Value> = shapes().into_iter().collect();
    if let Ok(out) = std::env::var("TSRS_CHECKER_SHAPES_OUT") {
        let mut text = String::new();
        for (q, r) in &ours {
            text.push_str(&json::marshal(&obj(&[("q", Value::String(q.clone())), ("r", r.clone())])).unwrap());
            text.push('\n');
        }
        std::fs::write(out, text).unwrap();
    }
    let mut diffs = Vec::new();
    let mut compared = 0;
    for line in GO_SHAPES.lines().filter(|l| !l.is_empty()) {
        let v = json::unmarshal(line).unwrap();
        let Value::String(q) = get(&v, "q") else { panic!() };
        let go = get(&v, "r");
        let Some(rs) = ours.get(&q) else {
            diffs.push(format!("{q}: missing"));
            continue;
        };
        compared += 1;
        let (go_n, rs_n) = (normalize(&go, &mut Ids::default(), false), normalize(rs, &mut Ids::default(), false));
        if go_n != rs_n {
            diffs.push(format!("{q}\n  go: {}\n  rs: {}", json::marshal(&go_n).unwrap(), json::marshal(&rs_n).unwrap()));
        }
    }
    assert_eq!(compared, GO_SHAPES.lines().filter(|l| !l.is_empty()).count());
    assert!(diffs.is_empty(), "{} of {compared} differ from pinned Go:\n{}", diffs.len(), diffs.join("\n"));
}

// --- API checker lease re-entrancy through the real transport (runtime's reentrancy hooks) ---

mod lease_reentrancy {
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use tsrs_api_transport as transport;
    use tsrs_api_transport::jsonrpc::{FrameReader, FrameWriter};
    use tsrs_api_transport::msgpack::{MessagePackReader, MessagePackWriter, MessageType};
    use tsrs_core::json::{self, Value};

    use super::{get, obj, session, utf16_pos, MAIN, S};
    use crate::checker::lease;

    /// Forwards to core's Session, plus test-only methods that hold the fixture program's API checker
    /// gate while calling the client ("outerHold") or while sleeping ("slowHold").
    struct H {
        s: S,
        caller: Arc<transport::LateCaller>,
    }

    impl H {
        fn program(&self) -> &'static tsrs_compiler::Program {
            let sd = self.s.session.snapshot_data(self.s.snapshot as u64).unwrap();
            sd.get_program(&tsrs_project::ID(self.s.project.clone())).unwrap()
        }
    }

    impl transport::Handler for H {
        fn handle_request(&self, _cx: &transport::RequestContext, method: &str, params: &[u8]) -> Result<transport::Response, transport::ApiError> {
            match method {
                "outerHold" => {
                    let _lease = lease::acquire(self.program()).map_err(|e| transport::ApiError::internal(e.message))?;
                    let r = transport::Caller::call(&*self.caller, "cb", None).map_err(|e| transport::ApiError::internal(e.to_string()))?;
                    Ok(transport::Response::Json(r))
                }
                "slowHold" => {
                    let _lease = lease::acquire(self.program()).map_err(|e| transport::ApiError::internal(e.message))?;
                    thread::sleep(Duration::from_millis(150));
                    Ok(transport::Response::Json(b"\"held\"".to_vec()))
                }
                _ => match crate::handler::Handler::handle_request(&*self.s.session, method, params) {
                    Ok(crate::handler::Response::Json(t)) => Ok(transport::Response::Json(t.into_bytes())),
                    Ok(crate::handler::Response::Binary(b)) => Ok(transport::Response::Binary(b)),
                    Err(e) => Err(transport::ApiError::internal(e.to_string())),
                },
            }
        }

        fn handle_notification(&self, _cx: &transport::RequestContext, _method: &str, _params: &[u8]) {}
    }

    fn bounded<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| panic!("{what}: hung"))
    }

    /// (params for a lease-needing checker request, params for a lease-free checker request)
    fn requests(s: &S) -> (String, String) {
        let needs = json::marshal(&s.at("box:")).unwrap();
        let reference = super::get(&s.call("getSymbolAtPosition", &s.at("legs")).unwrap(), "reference");
        let free = json::marshal(&obj(&[("symbol", reference)])).unwrap();
        (needs, free)
    }

    const REENTRANT_PREFIX: &str = "api: client error: the program's API checker is in use by a request that is waiting on a client callback";

    #[test]
    fn async_reentry_on_held_api_checker_fails_boundedly_and_unrelated_requests_proceed() {
        bounded("async api-checker re-entry", || {
            let s = session();
            let (needs, free) = requests(&s);
            let keep_alive = s.session.snapshot_data(s.snapshot as u64).unwrap();
            let program = keep_alive.get_program(&tsrs_project::ID(s.project.clone())).unwrap();
            let late = transport::LateCaller::new();
            let (server_r, client_w) = std::io::pipe().unwrap();
            let (client_r, server_w) = std::io::pipe().unwrap();
            let conn = transport::AsyncConn::new(
                Box::new(FrameReader::new(server_r)),
                Box::new(FrameWriter::new(server_w)),
                Arc::new(H { s, caller: late.clone() }),
                transport::ConnOptions::default(),
                None,
            );
            late.set(conn.caller());
            let run = thread::spawn(move || conn.run());
            let mut w = FrameWriter::new(client_w);
            let mut r = FrameReader::new(client_r);
            let mut read = || json::unmarshal(std::str::from_utf8(&r.read_frame().unwrap()).unwrap()).unwrap();
            w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"outerHold"}"#).unwrap();
            let call: Value = read();
            assert_eq!(get(&call, "method"), Value::String("cb".into()));
            // Needs the held API checker while its holder waits on the client: bounded error.
            w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":2,"method":"getTypeAtPosition","params":{needs}}}"#).as_bytes()).unwrap();
            let resp = read();
            let message = match get(&get(&resp, "error"), "message") {
                Value::String(m) => m,
                other => panic!("expected error, got {other:?} in {}", json::marshal(&resp).unwrap()),
            };
            assert!(message.starts_with(REENTRANT_PREFIX), "{message}");
            // A checker request that does not need the API checker is not rejected.
            w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":3,"method":"getParentOfSymbol","params":{free}}}"#).as_bytes()).unwrap();
            let resp = read();
            assert_eq!(get(&get(&resp, "result"), "name"), Value::String("Dog".into()), "{}", json::marshal(&resp).unwrap());
            let id = json::marshal(&get(&call, "id")).unwrap();
            w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{id},"result":"done"}}"#).as_bytes()).unwrap();
            let resp = read();
            assert_eq!(get(&resp, "result"), Value::String("done".into()));
            // Released: the same request now succeeds.
            w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":4,"method":"getTypeAtPosition","params":{needs}}}"#).as_bytes()).unwrap();
            let resp = read();
            assert_eq!(get(&get(&resp, "result"), "flags"), Value::Number(1048576.0), "{}", json::marshal(&resp).unwrap());
            drop(w);
            assert!(run.join().unwrap().is_ok());
            assert!(!lease::is_tracked(program), "the gate is dropped once unused");
            drop(keep_alive);
        });
    }

    #[test]
    fn async_plain_contention_on_api_checker_waits() {
        bounded("async api-checker contention", || {
            let s = session();
            let (needs, _) = requests(&s);
            let late = transport::LateCaller::new();
            let (server_r, client_w) = std::io::pipe().unwrap();
            let (client_r, server_w) = std::io::pipe().unwrap();
            let conn = transport::AsyncConn::new(
                Box::new(FrameReader::new(server_r)),
                Box::new(FrameWriter::new(server_w)),
                Arc::new(H { s, caller: late.clone() }),
                transport::ConnOptions::default(),
                None,
            );
            late.set(conn.caller());
            let run = thread::spawn(move || conn.run());
            let mut w = FrameWriter::new(client_w);
            let mut r = FrameReader::new(client_r);
            w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"slowHold"}"#).unwrap();
            thread::sleep(Duration::from_millis(30));
            w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":2,"method":"getTypeAtPosition","params":{needs}}}"#).as_bytes()).unwrap();
            let mut results = Vec::new();
            for _ in 0..2 {
                let v: Value = json::unmarshal(std::str::from_utf8(&r.read_frame().unwrap()).unwrap()).unwrap();
                assert_eq!(get(&v, "error"), Value::Null, "{}", json::marshal(&v).unwrap());
                results.push(get(&v, "id"));
            }
            assert_eq!(results, [Value::Number(1.0), Value::Number(2.0)], "the waiter finishes after the holder");
            drop(w);
            assert!(run.join().unwrap().is_ok());
        });
    }

    #[test]
    fn sync_nested_request_on_held_api_checker_fails_boundedly() {
        bounded("sync api-checker re-entry", || {
            let s = session();
            let (needs, free) = requests(&s);
            let late = transport::LateCaller::new();
            let (server_r, client_w) = std::io::pipe().unwrap();
            let (client_r, server_w) = std::io::pipe().unwrap();
            let conn = transport::SyncConn::new(
                Box::new(MessagePackReader::new(server_r)),
                Box::new(MessagePackWriter::new(server_w)),
                Arc::new(H { s, caller: late.clone() }),
                transport::ConnOptions::default(),
            );
            late.set(conn.caller());
            let run = thread::spawn(move || conn.run());
            let mut w = MessagePackWriter::new(client_w);
            let mut r = MessagePackReader::new(client_r);
            let mut recv = || {
                let t = r.read_tuple().unwrap();
                (t.msg_type, String::from_utf8(t.method).unwrap(), String::from_utf8(t.payload).unwrap())
            };
            w.write_tuple(MessageType::Request, b"outerHold", b"").unwrap();
            assert_eq!(recv().0, MessageType::Call);
            w.write_tuple(MessageType::Request, b"getTypeAtPosition", needs.as_bytes()).unwrap();
            let (ty, method, payload) = recv();
            assert_eq!((ty, method.as_str()), (MessageType::Error, "getTypeAtPosition"), "{payload}");
            assert!(payload.starts_with(REENTRANT_PREFIX), "{payload}");
            w.write_tuple(MessageType::Request, b"getParentOfSymbol", free.as_bytes()).unwrap();
            let (ty, _, payload) = recv();
            assert_eq!(ty, MessageType::Response, "{payload}");
            assert!(payload.contains("\"name\":\"Dog\""), "{payload}");
            w.write_tuple(MessageType::CallResponse, b"cb", b"\"done\"").unwrap();
            assert_eq!(recv(), (MessageType::Response, "outerHold".into(), "\"done\"".into()));
            w.write_tuple(MessageType::Request, b"getTypeAtPosition", needs.as_bytes()).unwrap();
            let (ty, _, payload) = recv();
            assert_eq!(ty, MessageType::Response, "{payload}");
            drop(w);
            assert!(run.join().unwrap().is_ok());
        });
    }

    #[allow(unused)]
    fn _uses(_: usize) {
        let _ = utf16_pos(MAIN, "box");
    }
}
