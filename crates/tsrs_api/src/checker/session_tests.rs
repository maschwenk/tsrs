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

pub(super) struct S {
    pub(super) session: Arc<Session>,
    pub(super) snapshot: f64,
    pub(super) project: String,
    lines: Vec<(String, Value)>,
}

fn utf16_pos(text: &str, needle: &str) -> usize {
    text[..text.find(needle).unwrap_or_else(|| panic!("{needle:?}"))].encode_utf16().count()
}

pub(super) fn get(v: &Value, k: &str) -> Value {
    match v {
        Value::Object(o) => o.get(k).cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

impl S {
    pub(super) fn call(&self, method: &str, params: &Value) -> Result<Value, String> {
        let text = json::marshal(params).unwrap();
        match self.session.handle_request(method, text.as_bytes()) {
            Ok(Response::Json(t)) => Ok(json::unmarshal(&t).unwrap()),
            Ok(Response::Binary(b)) => panic!("{method}: unexpected binary ({} bytes)", b.len()),
            Err(e) => Err(e.to_string().lines().next().unwrap_or_default().to_string()),
        }
    }

    fn call_raw(&self, method: &str, raw: &str) -> Result<Value, String> {
        match self.session.handle_request(method, raw.as_bytes()) {
            Ok(Response::Json(t)) => Ok(json::unmarshal(&t).unwrap()),
            Ok(Response::Binary(b)) => panic!("{method}: unexpected binary ({} bytes)", b.len()),
            Err(e) => Err(e.to_string().lines().next().unwrap_or_default().to_string()),
        }
    }

    pub(super) fn sp(&self, extra: &[(&str, Value)]) -> Value {
        let mut o = tsrs_core::collections::OrderedMap::default();
        o.insert("snapshot".to_string(), Value::Number(self.snapshot));
        o.insert("project".to_string(), Value::String(self.project.clone()));
        for (k, v) in extra {
            o.insert(k.to_string(), v.clone());
        }
        Value::Object(o)
    }

    pub(super) fn at(&self, needle: &str) -> Value {
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

pub(super) fn obj(pairs: &[(&str, Value)]) -> Value {
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
    session_with(&[], &["/p/tsconfig.json"])
}

/// A session over the fixture (plus `extra` files) with one snapshot opening `configs`; `S.project`
/// is the `/p` project.
pub(super) fn session_with(extra: &[(&str, &str)], configs: &[&str]) -> S {
    let mut files = vec![("/p/tsconfig.json", TSCONFIG), ("/p/main.ts", MAIN), ("/p/types.d.ts", TYPES), ("/p/other.ts", OTHER)];
    files.extend_from_slice(extra);
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|(k, v)| (k.to_string(), v.to_string())), false)));
    let session = Session::new(SessionOptions::new("/".to_string(), bundled::lib_path(), fs, false));
    let mut s = S { session, snapshot: 0.0, project: String::new(), lines: Vec::new() };
    let open = Value::Array(configs.iter().map(|c| Value::String(c.to_string())).collect());
    let snap = s.call("createSnapshot", &obj(&[("openProjects", open)])).unwrap();
    s.snapshot = match get(&snap, "snapshot") {
        Value::Number(x) => x,
        other => panic!("{other:?}"),
    };
    let Value::Array(projects) = get(&snap, "projects") else { panic!("no projects") };
    s.project = projects
        .iter()
        .find_map(|p| match get(p, "id") {
            Value::String(id) if id.starts_with("/p/") => Some(id),
            _ => None,
        })
        .expect("/p project");
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

    // Missing / null / empty parameter values (same raw payloads as the Go probe).
    let sp = format!(r#""snapshot":{},"project":{}"#, json::marshal(&n(s.snapshot)).unwrap(), json::marshal_string(&s.project));
    for (method, raw) in [
        ("getSymbolAtPosition", r#"{<sp>,"position":0}"#),
        ("getSymbolAtPosition", r#"{<sp>,"file":null,"position":0}"#),
        ("getSymbolAtPosition", r#"{<sp>,"file":{},"position":0}"#),
        ("getSymbolAtPosition", r#"{<sp>,"file":{"uri":null},"position":0}"#),
        ("getSymbolAtPosition", r#"{<sp>,"file":5,"position":0}"#),
        ("getSymbolAtPosition", "null"),
        ("getSymbolsOfSourceFiles", r#"{<sp>,"files":[null]}"#),
        ("getSymbolsOfSourceFiles", r#"{<sp>,"files":[{}]}"#),
        ("getSymbolsOfSourceFiles", r#"{<sp>,"files":null}"#),
        ("resolveName", r#"{<sp>,"name":"Array","meaning":788968,"file":null,"position":0}"#),
        ("resolveName", r#"{<sp>,"name":"box","meaning":111551,"file":"/p/main.ts","position":null}"#),
        ("getSymbolsInScope", r#"{<sp>,"file":null,"position":0,"meaning":1}"#),
        ("getSymbolsAtPositions", r#"{<sp>,"file":"/p/main.ts","positions":null}"#),
        ("getTypesOfSymbols", r#"{<sp>,"symbols":[null]}"#),
        ("getTypeOfSymbol", r#"{<sp>,"symbol":null}"#),
        ("getSymbolAtLocation", r#"{<sp>,"location":null}"#),
        ("getSymbolAtPosition", r#"{<sp>,"file":true,"position":0}"#),
        ("getSymbolAtPosition", r#"{<sp>,"file":[],"position":0}"#),
    ] {
        let r = match s.call_raw(method, &raw.replace("<sp>", &sp)) {
            Ok(r) => obj(&[("result", r)]),
            Err(e) => obj(&[("error", Value::String(e))]),
        };
        s.lines.push((format!("params:{method} {raw}"), r));
    }
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
        let (mut go_n, mut rs_n) = (normalize(&go, &mut Ids::default(), false), normalize(rs, &mut Ids::default(), false));
        // Go's json/v2 decoder picks "cannot unmarshal" or "unable to unmarshal" per process (observed
        // both on regeneration); the rest of the text is compared exactly.
        for v in [&mut go_n, &mut rs_n] {
            if let Value::Object(o) = v {
                if let Some(Value::String(e)) = o.get_mut("error") {
                    *e = e.replace("json: unable to unmarshal", "json: cannot unmarshal");
                }
            }
        }
        if go_n != rs_n {
            diffs.push(format!("{q}\n  go: {}\n  rs: {}", json::marshal(&go_n).unwrap(), json::marshal(&rs_n).unwrap()));
        }
    }
    assert_eq!(compared, GO_SHAPES.lines().filter(|l| !l.is_empty()).count());
    assert!(diffs.is_empty(), "{} of {compared} differ from pinned Go:\n{}", diffs.len(), diffs.join("\n"));
}

/// Exact uint64 handles through core's Session: values above 2^53 must reach the lookup unrounded (pinned Go
/// reports the exact number), and small handles keep working. Recorded against pinned Go with
/// testdata/node/numeric_ids_raw.mjs over raw sync/async payloads.
#[test]
fn uint64_handles_above_2_pow_53_are_looked_up_exactly() {
    let s = session();
    let raw = |method: &str, params: String| s.call_raw(method, &params);
    let sp = format!(r#""snapshot":{},"project":{}"#, json::marshal(&n(s.snapshot)).unwrap(), json::marshal_string(&s.project));
    // Small handles are unaffected (and register the project's type/signature registries first).
    let t = s.call("getTypeAtPosition", &s.at("box:")).unwrap();
    assert_eq!(raw("typeToString", format!(r#"{{{sp},"type":{}}}"#, json::marshal(&get(&t, "id")).unwrap())), Ok(Value::String("Box<number>".into())));
    let over = s.call("getTypeAtPosition", &s.at("over(x: string)")).unwrap();
    let Value::Array(sigs) = s.call("getSignaturesOfType", &s.sp(&[("type", get(&over, "id")), ("kind", n(0))])).unwrap() else { panic!() };
    let ret = raw("getReturnTypeOfSignature", format!(r#"{{{sp},"objectId":{}}}"#, json::marshal(&get(&sigs[0], "id")).unwrap())).unwrap();
    assert_eq!(get(&ret, "flags"), Value::Number(32.0), "string return type");
    for id in ["9007199254740993", "9007199254740992", "18446744073709551615"] {
        assert_eq!(
            raw("getTypeAtPosition", format!(r#"{{"snapshot":{id},"project":{},"file":"/p/main.ts","position":0}}"#, json::marshal_string(&s.project))),
            Err(format!("api: client error: snapshot {id} not found"))
        );
        assert_eq!(raw("getReturnTypeOfSignature", format!(r#"{{{sp},"objectId":{id}}}"#)), Err(format!("api: client error: signature handle {id} not found in project registry")));
        assert_eq!(raw("getRestTypeOfSignature", format!(r#"{{{sp},"signature":{id}}}"#)), Err(format!("api: client error: signature handle {id} not found in project registry")));
        // Escaped member names reach the same exact literal.
        assert_eq!(
            raw("getTypeAtPosition", format!(r#"{{"snap\u0073hot":{id},"project":{},"file":"/p/main.ts","position":0}}"#, json::marshal_string(&s.project))),
            Err(format!("api: client error: snapshot {id} not found"))
        );
    }
    // Type ids are uint32: above it is an invalid request (decode error), at the bound a lookup.
    assert!(raw("typeToString", format!(r#"{{{sp},"type":4294967296}}"#)).unwrap_err().starts_with("api: invalid request: "));
    assert_eq!(raw("typeToString", format!(r#"{{{sp},"type":4294967295}}"#)), Err("api: client error: type handle 4294967295 not found in project registry".to_string()));

}
