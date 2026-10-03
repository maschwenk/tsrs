// Real-program tests for the checker lane: a real tsrs_project snapshot (configured tsconfig project
// over an in-memory FS with the bundled libs) and the real API checker; no mocks of checker behavior.
//
// `TestHost` stands in for the parts core/codec own (node index tables, source-file descriptors, AST
// encoding). Its node index follows Go `encoder.BuildNodeIndexTable` ordering for nodes (pre-order,
// JSDoc after children) but does not reserve NodeList slots, so its handles are only meaningful inside
// these tests; the real handles come from tsrs_api_codec.

use std::sync::Arc;

use tsrs_ast::{Node, SourceFile};
use tsrs_compiler::Program;
use tsrs_core::collections::Set;
use tsrs_core::context::Context;
use tsrs_core::json::{self, Value};
use tsrs_core::tspath::Path;
use tsrs_core::P;
use tsrs_project::{APISnapshotRequest, FileChangeSummary, SessionInit, SessionOptions, Snapshot, SnapshotHost};
use tsrs_vfs::{bundled, vfstest, FS};

use super::coverage_table::PINNED_METHODS;
use super::coverage_types::{Lane, Status};
use super::dispatch::{handle, is_checker_method};
use super::host::*;
use super::registry::CheckerRegistry;

const MAIN: &str = r#"import { type Box, make } from "./types";
export type Pair<A, B> = [first: A, second: B];
export type U = string | number;
export function over(x: string): string;
export function over(x: number): number;
export function over(x: any) { return x; }
const ünïcödé = "é😀";
export const box: Box<number> = make(1);
export const p: Pair<string, U> = ["a", 1];
export function isStr(x: unknown): x is string { return typeof x === "string"; }
/** Docs for Animal.
 * @deprecated use Dog */
export class Animal { name = "a"; }
export class Dog extends Animal { readonly legs = 4; }
export enum Color { Red = 1, Green = 2 }
export const r = over(42);
export type M = { [K in "a" | "b"]: K };
export type C<T> = T extends string ? 1 : 2;
export const lit = "hi" as const;
export const fn = <T,>(v: T) => v;
export const o = { box };
export { ünïcödé as uni };
export type IA<T, K extends keyof T> = T[K];
export async function aw(): Promise<number> { return 1; }
export let maybe: string | undefined;
export const doubled = [1, 2].map(x => x * 2);
export const cb: (n: number) => void = (n) => {};
export function withThis(this: Dog, a: number) { return a; }
export function rest(a: string, ...xs: number[]) { return xs; }
export const arr: number[] = [];
export type S<T> = T extends string ? Box<T> : never;
export interface WithDefault<T = string> { v: T }
export function useAll() { return box.value + p[1].toString(); }
export type Up = Uppercase<"a">;
export type TL = `x${string}`;
export const big = 10n;
"#;

const TYPES: &str = r#"export interface Box<T> { value: T; [key: string]: unknown }
export declare function make<T>(v: T): Box<T>;
"#;

const TSCONFIG: &str = r#"{ "compilerOptions": { "strict": true, "target": "es2022", "module": "esnext", "moduleResolution": "bundler" }, "files": ["main.ts", "types.d.ts", "other.ts"] }"#;

const OTHER: &str = "export const helper = 1;\nexport function useArgs() { return arguments.length; }\ndeclare const notFn: number;\n// @ts-ignore\nnotFn();\n";

struct TestHost {
    snapshot_host: Arc<SnapshotHost>,
    snapshot: Arc<Snapshot>,
    registry: Arc<CheckerRegistry>,
    handle: u64,
    project: String,
}

impl Drop for TestHost {
    fn drop(&mut self) {
        self.registry.release();
        self.snapshot.deref();
    }
}

fn node_table(file: P<SourceFile>) -> Vec<P<Node>> {
    fn visit(node: P<Node>, file: P<SourceFile>, out: &mut Vec<P<Node>>) {
        out.push(node);
        node.for_each_child(&mut |child: P<Node>| {
            visit(child, file, out);
            false
        });
        for jsdoc in node.jsdoc(Some(file.get())) {
            visit(*jsdoc, file, out);
        }
    }
    let mut out = Vec::new();
    visit(file.as_node(), file, &mut out);
    out
}

impl TestHost {
    fn new(files: &[(&str, &str)]) -> TestHost {
        TestHost::with_configs(files, &["/p/tsconfig.json"])
    }

    /// Opens every config; `project` is the first one.
    fn with_configs(files: &[(&str, &str)], configs: &[&str]) -> TestHost {
        let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|(k, v)| (k.to_string(), v.to_string())), true)));
        let init = SessionInit {
            background_ctx: Context::background(),
            options: Arc::new(SessionOptions { current_directory: "/".to_string(), default_library_path: bundled::lib_path(), ..Default::default() }),
            fs,
            client: None,
            logger: None,
            npm_executor: None,
            parse_cache: None,
            content_mapped_parse_cache: None,
        };
        let snapshot_host = tsrs_project::new_snapshot_host(&init);
        let root = snapshot_host.new_root_snapshot();
        let mut open = Set::default();
        let mut ensure = Set::default();
        let ids: Vec<tsrs_project::ID> = configs.iter().map(|c| tsrs_project::parse_configured_project_id(&Path::new(c.to_string())).unwrap().as_id()).collect();
        for (config, id) in configs.iter().zip(&ids) {
            open.add(config.to_string());
            ensure.add(id.clone());
        }
        let id = ids[0].clone();
        let request = APISnapshotRequest { open_projects: Some(open), ensure_programs: Some(ensure), ..Default::default() };
        let snapshot = match snapshot_host.clone_snapshot(&Context::background(), &root, FileChangeSummary::default(), Some(Arc::new(request))) {
            Ok(s) => s,
            Err((_, e)) => panic!("snapshot failed: {e:?}"),
        };
        root.deref();
        let handle = snapshot.id();
        TestHost { snapshot_host, snapshot, registry: Arc::new(CheckerRegistry::new()), handle, project: id.0 }
    }

    fn program(&self) -> &'static Program {
        self.snapshot.project_collection.get_project(&tsrs_project::ID(self.project.clone())).unwrap().get_program().unwrap()
    }

    fn call(&self, method: &str, params: &str) -> CheckerResult<Value> {
        let params = json::unmarshal(params).unwrap_or_else(|e| panic!("bad test params {e}: {params}"));
        match handle(self, method, &params).expect("checker method")? {
            CheckerResponse::Json(v) => Ok(v),
            CheckerResponse::EncodedNode(bytes) => Ok(Value::String(String::from_utf8(bytes).unwrap())),
        }
    }

    fn ok(&self, method: &str, params: &str) -> Value {
        self.call(method, params).unwrap_or_else(|e| panic!("{method}({params}) failed: {e}"))
    }

    /// `{"snapshot":H,"project":P,<extra>}`
    fn sp(&self, extra: &str) -> String {
        let sep = if extra.is_empty() { "" } else { "," };
        format!(r#"{{"snapshot":{},"project":{}{sep}{extra}}}"#, self.handle, json::marshal_string(&self.project))
    }

    fn type_at(&self, file: &str, text: &str, needle: &str) -> Value {
        self.ok("getTypeAtPosition", &self.sp(&format!(r#""file":"{file}","position":{}"#, utf16_pos(text, needle))))
    }

    fn symbol_at(&self, file: &str, text: &str, needle: &str) -> Value {
        self.ok("getSymbolAtPosition", &self.sp(&format!(r#""file":"{file}","position":{}"#, utf16_pos(text, needle))))
    }

    fn type_string(&self, t: &Value) -> String {
        str_of(&self.ok("typeToString", &self.sp(&format!(r#""type":{}"#, num(t, "id")))))
    }

    fn symbol_ref(&self, symbol: &Value) -> String {
        json::marshal(field(symbol, "reference")).unwrap()
    }

    fn handle_at(&self, file: &str, text: &str, needle: &str) -> String {
        let sf = self.program().get_source_file(file).unwrap();
        let utf8 = text.find(needle).unwrap() as i32;
        let node = tsrs_astnav::get_touching_property_name(sf, utf8);
        self.node_handle(node).unwrap()
    }
}

impl CheckerHost for TestHost {
    fn context(&self) -> Context {
        Context::background()
    }

    fn snapshot(&self, handle: u64) -> CheckerResult<SnapshotScope> {
        if handle != self.handle || self.registry.is_released() {
            return Err(CheckerError::client(format!("snapshot {handle} not found")));
        }
        self.snapshot_host.retain_snapshot(&self.snapshot);
        let snapshot = self.snapshot.clone();
        let release_snapshot = snapshot.clone();
        Ok(SnapshotScope { handle, snapshot, registry: self.registry.clone(), release: Some(Box::new(move || release_snapshot.deref())) })
    }

    fn acquire_cached_source_file(&self, descriptor: &Value) -> CheckerResult<CachedFileScope> {
        // Mirrors core's lease cache: once the last snapshot holding the file is released it is gone
        // (Go: "source file is not available").
        if self.registry.is_released() {
            return Err(CheckerError::client("source file is not available"));
        }
        let path = str_of(field(descriptor, "path"));
        let file = self.program().get_source_file_by_path(&Path::new(path)).ok_or_else(|| CheckerError::client("source file not cached"))?;
        if self.source_file_descriptor(file)? != *descriptor {
            return Err(CheckerError::client("source file descriptor mismatch"));
        }
        Ok(CachedFileScope { file, release: None })
    }

    fn source_file_descriptor(&self, file: P<SourceFile>) -> CheckerResult<Value> {
        Ok(json::unmarshal(&format!(
            r#"{{"fileName":{},"path":{},"contentHash":"","parseOptionsKey":"","scriptKind":{},"nodeId":"{}"}}"#,
            json::marshal_string(file.file_name()),
            json::marshal_string(file.path()),
            file.script_kind.get() as i32,
            self.source_file_node_id(file)?
        ))
        .unwrap())
    }

    fn source_file_node_id(&self, file: P<SourceFile>) -> CheckerResult<u64> {
        Ok(tsrs_ast::get_node_id(file.as_node()).0)
    }

    fn node_handle(&self, node: P<Node>) -> CheckerResult<String> {
        let file = tsrs_ast::get_source_file_of_node(node).ok_or_else(|| CheckerError::client("node has no source file"))?;
        let idx = node_table(file).iter().position(|n| *n == node).ok_or_else(|| CheckerError::client("node not in table"))?;
        Ok(format!("{}.{}.{}", idx + 1, node.kind() as i16, file.path().as_str()))
    }

    fn resolve_node_handle(&self, program: &'static Program, handle: &str) -> CheckerResult<P<Node>> {
        let mut parts = handle.splitn(3, '.');
        let (Some(idx), Some(_kind), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(CheckerError::client(format!("invalid node handle {handle:?}")));
        };
        let idx: usize = idx.parse().map_err(|_| CheckerError::client(format!("invalid node handle {handle:?}")))?;
        let file = program.get_source_file_by_path(&Path::new(path)).ok_or_else(|| CheckerError::client("stale node handle"))?;
        node_table(file).get(idx.wrapping_sub(1)).copied().ok_or_else(|| CheckerError::client("stale node handle"))
    }

    fn clone_snapshot_with_auto_imports(&self, base: &Snapshot, file_name: &str) -> CheckerResult<Arc<Snapshot>> {
        let uri = tsrs_ls::lsconv::file_name_to_document_uri(file_name);
        Ok(self.snapshot_host.clone_snapshot_with_auto_imports(&Context::background(), base, &uri, None))
    }

    fn encode_node(&self, node: P<Node>) -> CheckerResult<Vec<u8>> {
        // Test stand-in for the codec: the node kind is enough to check the handler built a node.
        Ok(format!("{:?}", node.kind()).into_bytes())
    }
}

// --- JSON helpers ---

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    match v {
        Value::Object(o) => o.get(key).unwrap_or(&Value::Null),
        _ => panic!("not an object ({key}): {v:?}"),
    }
}

fn num(v: &Value, key: &str) -> u64 {
    match field(v, key) {
        Value::Number(n) => *n as u64,
        other => panic!("{key} not a number: {other:?} in {}", json::marshal(v).unwrap()),
    }
}

fn str_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        _ => panic!("not a string: {v:?}"),
    }
}

fn arr(v: &Value) -> &Vec<Value> {
    match v {
        Value::Array(a) => a,
        _ => panic!("not an array: {}", json::marshal(v).unwrap()),
    }
}

fn names(v: &Value) -> Vec<String> {
    arr(v).iter().map(|s| str_of(field(s, "name"))).collect()
}

/// UTF-16 offset of the first occurrence of `needle` (what the JS client sends).
fn utf16_pos(text: &str, needle: &str) -> usize {
    let byte = text.find(needle).unwrap_or_else(|| panic!("{needle:?} not in fixture"));
    text[..byte].encode_utf16().count()
}

fn fixture() -> TestHost {
    TestHost::new(&[("/p/tsconfig.json", TSCONFIG), ("/p/main.ts", MAIN), ("/p/types.d.ts", TYPES), ("/p/other.ts", OTHER)])
}

const MAIN_FILE: &str = "/p/main.ts";

// --- Tests ---

#[test]
fn coverage_inventory_matches_pinned_proto_and_dispatch() {
    assert_eq!(PINNED_METHODS.len(), 172);
    let mut seen = std::collections::HashSet::new();
    for row in PINNED_METHODS {
        assert!(seen.insert(row.method), "duplicate {}", row.method);
        let dispatched = is_checker_method(row.method);
        match row.status {
            Status::Implemented | Status::Tested => assert!(dispatched, "{} marked {:?} but not dispatched", row.method, row.status),
            Status::Planned | Status::NotOwned => assert!(!dispatched, "{} dispatched but marked {:?}", row.method, row.status),
        }
        assert_eq!(row.lane == Lane::Checker, dispatched || row.status == Status::Planned, "{} lane/dispatch mismatch", row.method);
    }
    // Same set and order as core's methods.rs inventory.
    let core: Vec<&str> = crate::methods::METHODS.iter().map(|m| m.name).collect();
    let ours: Vec<&str> = PINNED_METHODS.iter().map(|r| r.method).collect();
    assert_eq!(core, ours);
    for m in crate::methods::METHODS.iter() {
        if is_checker_method(m.name) {
            assert_eq!(m.owner, crate::methods::Owner::Checker, "{} dispatched by checker but owned by core", m.name);
        }
    }
    // Against the pinned Go source when ts-ref is checked out.
    let proto = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ts-ref/tsc/internal/api/proto.go");
    if let Ok(src) = std::fs::read_to_string(proto) {
        let pinned: Vec<String> = src
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                let rest = l.strip_prefix("Method")?;
                let (_, value) = rest.split_once("Method = \"")?;
                Some(value.trim_end_matches('"').to_string())
            })
            .collect();
        assert_eq!(pinned, ours);
    }
}

#[test]
fn types_and_symbols_at_utf16_positions() {
    let h = fixture();
    // `box` comes after a line with non-BMP / non-ASCII text: positions are UTF-16 offsets.
    let s = h.symbol_at(MAIN_FILE, MAIN, "box:");
    assert_eq!(str_of(field(&s, "name")), "box");
    let t = h.type_at(MAIN_FILE, MAIN, "box:");
    assert_eq!(h.type_string(&t), "Box<number>");
    // Unicode identifier.
    let s = h.symbol_at(MAIN_FILE, MAIN, "ünïcödé");
    assert_eq!(str_of(field(&s, "name")), "ünïcödé");
    let t = h.type_at(MAIN_FILE, MAIN, "ünïcödé");
    assert_eq!(h.type_string(&t), "\"é😀\"");
    assert_eq!(field(&t, "value"), &Value::String("é😀".to_string()));
    // Batch variants keep positions aligned, with null where nothing is found.
    let positions = format!("[{},{},{}]", utf16_pos(MAIN, "box:"), utf16_pos(MAIN, "p:"), MAIN.encode_utf16().count() + 50);
    let syms = h.ok("getSymbolsAtPositions", &h.sp(&format!(r#""file":"{MAIN_FILE}","positions":{positions}"#)));
    let syms = arr(&syms);
    assert_eq!(syms.len(), 3);
    assert_eq!(str_of(field(&syms[0], "name")), "box");
    assert_eq!(str_of(field(&syms[1], "name")), "p");
    // Past EOF GetTouchingPropertyName yields the source file itself: its module symbol (as in Go).
    assert_eq!(str_of(field(&syms[2], "name")), "\"/p/main\"");
    let types = h.ok("getTypesAtPositions", &h.sp(&format!(r#""file":"{MAIN_FILE}","positions":{positions}"#)));
    assert_eq!(h.type_string(&arr(&types)[1]), "Pair<string, U>");
    // Snapshot-owned vs file-owned references.
    let s = h.symbol_at(MAIN_FILE, MAIN, "box:");
    assert_eq!(num(field(&s, "reference"), "kind"), 0, "binder symbol of a source file is file-owned");
    assert!(!arr(field(&s, "declarations")).is_empty());
}

#[test]
fn symbol_types_aliases_and_imports() {
    let h = fixture();
    // Imported alias from a .d.ts.
    let make_alias = h.symbol_at(MAIN_FILE, MAIN, "make }");
    let aliased = h.ok("getAliasedSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&make_alias))));
    assert_eq!(str_of(field(&aliased, "name")), "make");
    let decl = str_of(&arr(field(&aliased, "declarations"))[0]);
    assert!(decl.ends_with("/p/types.d.ts"), "{decl}");
    let immediate = h.ok("getImmediateAliasedSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&make_alias))));
    assert_eq!(str_of(field(&immediate, "name")), "make");
    let fq = h.ok("getFullyQualifiedName", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&aliased))));
    assert_eq!(str_of(&fq), "\"/p/types\".make");
    // Type alias: declared type is the union; types and alias symbol come back.
    let u = h.symbol_at(MAIN_FILE, MAIN, "U = ");
    let declared = h.ok("getDeclaredTypeOfSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&u))));
    assert_eq!(h.type_string(&declared), "U");
    let members = h.ok("getTypesOfType", &h.sp(&format!(r#""objectId":{}"#, num(&declared, "id"))));
    let mut member_strings: Vec<String> = arr(&members).iter().map(|t| h.type_string(t)).collect();
    member_strings.sort();
    assert_eq!(member_strings, ["number", "string"]);
    let alias_symbol = h.ok("getAliasSymbolOfType", &h.sp(&format!(r#""objectId":{}"#, num(&declared, "id"))));
    assert_eq!(str_of(field(&alias_symbol, "name")), "U");
    // Assignability between handles.
    let string_t = h.ok("getStringType", &h.sp(""));
    let a = h.ok("isTypeAssignableTo", &h.sp(&format!(r#""source":{},"target":{}"#, num(&string_t, "id"), num(&declared, "id"))));
    let b = h.ok("isTypeAssignableTo", &h.sp(&format!(r#""source":{},"target":{}"#, num(&declared, "id"), num(&string_t, "id"))));
    assert_eq!((a, b), (Value::Bool(true), Value::Bool(false)));
    // getTypesOfSymbols batch.
    let box_sym = h.symbol_at(MAIN_FILE, MAIN, "box:");
    let types = h.ok("getTypesOfSymbols", &h.sp(&format!(r#""symbols":[{},{}]"#, h.symbol_ref(&box_sym), h.symbol_ref(&u))));
    assert_eq!(h.type_string(&arr(&types)[0]), "Box<number>");
}

#[test]
fn generics_properties_index_infos_and_type_arguments() {
    let h = fixture();
    let t = h.type_at(MAIN_FILE, MAIN, "box:");
    let id = num(&t, "id");
    let args = h.ok("getTypeArguments", &h.sp(&format!(r#""type":{id}"#)));
    assert_eq!(arr(&args).iter().map(|a| h.type_string(a)).collect::<Vec<_>>(), ["number"]);
    let target = h.ok("getTargetOfType", &h.sp(&format!(r#""objectId":{id}"#)));
    assert_eq!(num(&target, "id"), num(&t, "target"));
    let tps = h.ok("getTypeParametersOfType", &h.sp(&format!(r#""objectId":{}"#, num(&target, "id"))));
    assert_eq!(arr(&tps).iter().map(|a| h.type_string(a)).collect::<Vec<_>>(), ["T"]);
    let props = h.ok("getPropertiesOfType", &h.sp(&format!(r#""type":{id}"#)));
    assert_eq!(names(&props), ["value"]);
    let prop = h.ok("getPropertyOfType", &h.sp(&format!(r#""type":{id},"name":"value""#)));
    let prop_type = h.ok("getTypeOfSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&prop))));
    assert_eq!(h.type_string(&prop_type), "number");
    let missing = h.ok("getPropertyOfType", &h.sp(&format!(r#""type":{id},"name":"nope""#)));
    assert_eq!(missing, Value::Null);
    let infos = h.ok("getIndexInfosOfType", &h.sp(&format!(r#""type":{id}"#)));
    assert_eq!(arr(&infos).len(), 1);
    assert_eq!(h.type_string(field(&arr(&infos)[0], "keyType")), "string");
    let info = h.ok("getIndexInfoOfType", &h.sp(&format!(r#""type":{id},"kind":0"#)));
    assert_eq!(h.type_string(field(&info, "valueType")), "unknown");
    let bad = h.call("getIndexInfoOfType", &h.sp(&format!(r#""type":{id},"kind":7"#))).unwrap_err();
    assert_eq!(bad.kind, CheckerErrorKind::Client);
    // Generic arrow: call signature type parameters.
    let f = h.type_at(MAIN_FILE, MAIN, "fn =");
    let sigs = h.ok("getSignaturesOfType", &h.sp(&format!(r#""type":{},"kind":0"#, num(&f, "id"))));
    let sig = &arr(&sigs)[0];
    assert_eq!(arr(field(sig, "typeParameters")).len(), 1);
    let tps = h.ok("getTypeParametersOfSignature", &h.sp(&format!(r#""objectId":{}"#, num(sig, "id"))));
    assert_eq!(h.type_string(&arr(&tps)[0]), "T");
}

#[test]
fn overloads_signatures_and_resolution() {
    let h = fixture();
    let over = h.symbol_at(MAIN_FILE, MAIN, "over(x: string)");
    let t = h.ok("getTypeOfSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&over))));
    let sigs = h.ok("getSignaturesOfType", &h.sp(&format!(r#""type":{},"kind":0"#, num(&t, "id"))));
    let sigs = arr(&sigs);
    assert_eq!(sigs.len(), 2);
    let returns: Vec<String> = sigs
        .iter()
        .map(|s| h.type_string(&h.ok("getReturnTypeOfSignature", &h.sp(&format!(r#""objectId":{}"#, num(s, "id"))))))
        .collect();
    assert_eq!(returns, ["string", "number"]);
    let params = h.ok("getParametersOfSignature", &h.sp(&format!(r#""objectId":{}"#, num(&sigs[1], "id"))));
    assert_eq!(names(&params), ["x"]);
    let p0 = h.ok("getParameterType", &h.sp(&format!(r#""signature":{},"index":0"#, num(&sigs[1], "id"))));
    assert_eq!(h.type_string(&p0), "number");
    let neg = h.call("getParameterType", &h.sp(&format!(r#""signature":{},"index":-1"#, num(&sigs[1], "id")))).unwrap_err();
    assert_eq!(neg.kind, CheckerErrorKind::Client);
    let construct = h.ok("getSignaturesOfType", &h.sp(&format!(r#""type":{},"kind":1"#, num(&t, "id"))));
    assert!(arr(&construct).is_empty());
    // Overload resolution at the call site `over(42)`.
    let call = {
        let sf = h.program().get_source_file(MAIN_FILE).unwrap();
        let ident = tsrs_astnav::get_touching_property_name(sf, MAIN.find("over(42)").unwrap() as i32);
        h.node_handle(ident.parent().unwrap()).unwrap()
    };
    let resolved = h.ok("getResolvedSignature", &h.sp(&format!(r#""location":"{call}""#)));
    let ret = h.ok("getReturnTypeOfSignature", &h.sp(&format!(r#""objectId":{}"#, num(&resolved, "id"))));
    assert_eq!(h.type_string(&ret), "number");
    assert_eq!(h.type_string(&h.type_at(MAIN_FILE, MAIN, "r =")), "number");
    // Signature display.
    let decl = h.ok("signatureToSignatureDeclaration", &h.sp(&format!(r#""signature":{},"kind":{}"#, num(&sigs[0], "id"), tsrs_ast::Kind::FunctionDeclaration as i32)));
    assert_eq!(str_of(&decl), "FunctionDeclaration");
    let node = h.ok("typeToTypeNode", &h.sp(&format!(r#""type":{}"#, num(&t, "id"))));
    // Two overloads print as a type literal with two call signatures.
    assert_eq!(str_of(&node), "TypeLiteral");
}

#[test]
fn classes_predicates_enums_and_jsdoc() {
    let h = fixture();
    let dog = h.symbol_at(MAIN_FILE, MAIN, "Dog extends");
    let dog_t = h.ok("getDeclaredTypeOfSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&dog))));
    let bases = h.ok("getBaseTypes", &h.sp(&format!(r#""type":{}"#, num(&dog_t, "id"))));
    assert_eq!(arr(&bases).iter().map(|b| h.type_string(b)).collect::<Vec<_>>(), ["Animal"]);
    let props = h.ok("getPropertiesOfType", &h.sp(&format!(r#""type":{}"#, num(&dog_t, "id"))));
    assert_eq!(names(&props), ["legs", "name"]);
    let legs = &arr(&props)[0];
    assert_eq!(h.ok("isReadonlySymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(legs)))), Value::Bool(true));
    // Members of a file-owned class symbol resolve without a checker, ordered by declaration.
    let members = h.ok("getMembersOfSymbol", &format!(r#"{{"symbol":{}}}"#, h.symbol_ref(&dog)));
    assert_eq!(names(&members), ["legs"]);
    // Type predicates.
    let is_str = h.type_at(MAIN_FILE, MAIN, "isStr(");
    let sig = &arr(&h.ok("getSignaturesOfType", &h.sp(&format!(r#""type":{},"kind":0"#, num(&is_str, "id")))))[0].clone();
    let pred = h.ok("getTypePredicateOfSignature", &h.sp(&format!(r#""signature":{}"#, num(sig, "id"))));
    assert_eq!(num(&pred, "kind"), 1);
    assert_eq!(str_of(field(&pred, "parameterName")), "x");
    assert_eq!(h.type_string(field(&pred, "type")), "string");
    // Enum constant values via a declaration node handle.
    let red = h.symbol_at(MAIN_FILE, MAIN, "Red =");
    let red_decl = str_of(&arr(field(&red, "declarations"))[0]);
    let value = h.ok("getConstantValue", &h.sp(&format!(r#""location":"{red_decl}""#)));
    assert_eq!(field(&value, "isNumber"), &Value::Bool(true));
    assert_eq!(field(&value, "value"), &Value::Number(1.0));
    // JSDoc.
    let animal = h.symbol_at(MAIN_FILE, MAIN, "Animal {");
    let doc = h.ok("getDocumentationComment", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&animal))));
    assert_eq!(str_of(&doc), "Docs for Animal.");
    let tags = h.ok("getJsDocTags", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&animal))));
    assert_eq!(str_of(field(&arr(&tags)[0], "name")), "deprecated");
    assert_eq!(str_of(field(&arr(&tags)[0], "text")), "use Dog");
}

#[test]
fn structured_type_shapes_tuple_mapped_conditional_literal() {
    let h = fixture();
    // Labeled tuple target.
    let pair = h.symbol_at(MAIN_FILE, MAIN, "Pair<A, B> =");
    let pair_t = h.ok("getDeclaredTypeOfSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&pair))));
    assert_eq!(field(&pair_t, "isTupleType"), &Value::Bool(true));
    let tuple_target = h.ok("getTargetOfType", &h.sp(&format!(r#""objectId":{}"#, num(&pair_t, "id"))));
    assert_eq!(arr(field(&tuple_target, "labeledElementDeclarations")).len(), 2);
    assert_eq!(num(&tuple_target, "fixedLength"), 2);
    // Mapped type components are registered and resolvable.
    let m = h.symbol_at(MAIN_FILE, MAIN, "M = ");
    let m_t = h.ok("getDeclaredTypeOfSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&m))));
    let tp = h.ok("getTypeParameterOfMappedType", &h.sp(&format!(r#""objectId":{}"#, num(&m_t, "id"))));
    assert_eq!(num(&tp, "id"), num(&m_t, "typeParameter"));
    let ct = h.ok("getConstraintTypeOfMappedType", &h.sp(&format!(r#""objectId":{}"#, num(&m_t, "id"))));
    assert_eq!(h.type_string(&ct), "\"a\" | \"b\"");
    assert_eq!(h.ok("getNameTypeOfMappedType", &h.sp(&format!(r#""objectId":{}"#, num(&m_t, "id")))), Value::Null);
    // Conditional type.
    let c = h.symbol_at(MAIN_FILE, MAIN, "C<T> =");
    let c_t = h.ok("getDeclaredTypeOfSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&c))));
    let check = h.ok("getCheckTypeOfType", &h.sp(&format!(r#""objectId":{}"#, num(&c_t, "id"))));
    assert_eq!(h.type_string(&check), "T");
    let tru = h.ok("getTrueTypeOfConditionalType", &h.sp(&format!(r#""objectId":{}"#, num(&c_t, "id"))));
    assert_eq!(h.type_string(&tru), "1");
    let constraint = h.ok("getConstraintOfTypeParameter", &h.sp(&format!(r#""objectId":{}"#, num(&check, "id"))));
    // Pinned Go (b85298b6) answers with a *different* type parameter `T` of the same symbol (the
    // declared, unconstrained T), whose own constraint is null; tsrs matches (see pinned_go_differential).
    assert_eq!(h.type_string(&constraint), "T");
    assert_ne!(num(&constraint, "id"), num(&check, "id"));
    assert_eq!(field(field(&constraint, "symbol"), "id"), field(field(&check, "symbol"), "id"));
    assert_eq!(h.tid("getConstraintOfTypeParameter", "objectId", &constraint), Value::Null);
    assert_eq!(h.call("getConstraintOfTypeParameter", &h.sp(&format!(r#""objectId":{}"#, num(&c_t, "id")))).unwrap_err().kind, CheckerErrorKind::Client);
    // Literal freshness and widening.
    let lit = h.type_at(MAIN_FILE, MAIN, "lit =");
    assert_eq!(field(&lit, "value"), &Value::String("hi".to_string()));
    let regular = h.ok("getRegularTypeOfType", &h.sp(&format!(r#""objectId":{}"#, num(&lit, "id"))));
    assert_eq!(h.type_string(&regular), "\"hi\"");
    let base = h.ok("getBaseTypeOfLiteralType", &h.sp(&format!(r#""type":{}"#, num(&lit, "id"))));
    assert_eq!(h.type_string(&base), "string");
    // Wrong-kind property requests are explicit client errors, not panics.
    let err = h.call("getCheckTypeOfType", &h.sp(&format!(r#""objectId":{}"#, num(&lit, "id")))).unwrap_err();
    assert_eq!(err.kind, CheckerErrorKind::Client);
    let err = h.call("getTypesOfType", &h.sp(&format!(r#""objectId":{}"#, num(&lit, "id")))).unwrap_err();
    assert_eq!(err.kind, CheckerErrorKind::Client);
}

#[test]
fn scope_names_exports_intrinsics_and_well_known() {
    let h = fixture();
    let pos = utf16_pos(MAIN, "r = over");
    let animal = h.ok("resolveName", &h.sp(&format!(r#""name":"Animal","file":"{MAIN_FILE}","position":{pos},"meaning":{}"#, tsrs_ast::SymbolFlags::Value.bits())));
    assert_eq!(str_of(field(&animal, "name")), "Animal");
    let none = h.ok("resolveName", &h.sp(&format!(r#""name":"nope","file":"{MAIN_FILE}","position":{pos},"meaning":{}"#, tsrs_ast::SymbolFlags::Value.bits())));
    assert_eq!(none, Value::Null);
    let global = h.ok("resolveName", &h.sp(&format!(r#""name":"Array","meaning":{}"#, tsrs_ast::SymbolFlags::Type.bits())));
    assert_eq!(str_of(field(&global, "name")), "Array");
    assert_eq!(num(field(&global, "reference"), "kind"), 1, "merged global lib symbol is snapshot-owned");
    let in_scope = h.ok("getSymbolsInScope", &h.sp(&format!(r#""file":"{MAIN_FILE}","position":{pos},"meaning":{}"#, tsrs_ast::SymbolFlags::Value.bits())));
    let scope_names = names(&in_scope);
    assert!(scope_names.iter().any(|n| n == "box") && scope_names.iter().any(|n| n == "ünïcödé"));
    let err = h.call("getSymbolsInScope", &h.sp(r#""meaning":1"#)).unwrap_err();
    assert_eq!(err.kind, CheckerErrorKind::Client);
    // Module symbol and its exports (checker ordering).
    let module = h.ok("getSymbolOfSourceFile", &h.sp(&format!(r#""file":"{MAIN_FILE}""#)));
    let exports = h.ok("getExportsOfModule", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&module))));
    let export_names = names(&exports);
    for n in ["Animal", "Color", "Dog", "box", "over", "p", "r"] {
        assert!(export_names.iter().any(|e| e == n), "{n} missing from {export_names:?}");
    }
    let member = h.ok("getMemberInModuleExports", &h.sp(&format!(r#""symbol":{},"name":"Dog""#, h.symbol_ref(&module))));
    assert_eq!(str_of(field(&member, "name")), "Dog");
    // Intrinsics.
    for (method, expected) in [("getAnyType", "any"), ("getNumberType", "number"), ("getNeverType", "never"), ("getUnknownType", "unknown"), ("getESSymbolType", "symbol"), ("getNonPrimitiveType", "object")] {
        let t = h.ok(method, &h.sp(""));
        assert_eq!(h.type_string(&t), expected, "{method}");
        assert_eq!(str_of(field(&t, "intrinsicName")), expected);
    }
    let wk = h.ok("getWellKnownSymbols", &h.sp(""));
    assert!(num(&wk, "unknown") != 0 && num(&wk, "undefined") != 0 && num(&wk, "arguments") != 0);
    let sigs = h.ok("getWellKnownSignatures", &h.sp(""));
    assert!(num(&sigs, "unknown") != 0);
}

#[test]
fn handle_validation_stale_cross_project_and_release() {
    let h = fixture();
    let t = h.type_at(MAIN_FILE, MAIN, "box:");
    let id = num(&t, "id");
    // Unknown type id, wrong project, wrong snapshot, malformed params.
    assert_eq!(h.call("getTypeArguments", &h.sp(r#""type":999999"#)).unwrap_err().kind, CheckerErrorKind::Client);
    let other_project = format!(r#"{{"snapshot":{},"project":"/other/tsconfig.json","type":{id}}}"#, h.handle);
    assert_eq!(h.call("getTypeArguments", &other_project).unwrap_err().kind, CheckerErrorKind::Client);
    let other_snapshot = format!(r#"{{"snapshot":{},"project":{},"type":{id}}}"#, h.handle + 1000, json::marshal_string(&h.project));
    assert_eq!(h.call("getTypeArguments", &other_snapshot).unwrap_err().kind, CheckerErrorKind::Client);
    assert_eq!(h.call("getTypeArguments", &h.sp(r#""type":"x""#)).unwrap_err().kind, CheckerErrorKind::InvalidRequest);
    assert_eq!(h.call("getTypeArguments", &h.sp(r#""type":-1"#)).unwrap_err().kind, CheckerErrorKind::InvalidRequest);
    assert_eq!(h.call("getTypeArguments", "[]").unwrap_err().kind, CheckerErrorKind::InvalidRequest);
    assert_eq!(h.call("getSymbolAtLocation", &h.sp(r#""location":"garbage""#)).unwrap_err().kind, CheckerErrorKind::Client);
    // Snapshot symbol reference that names a different snapshot.
    let global = h.ok("resolveName", &h.sp(&format!(r#""name":"Array","meaning":{}"#, tsrs_ast::SymbolFlags::Type.bits())));
    let mut forged = h.symbol_ref(&global);
    forged = forged.replace(&format!(r#""snapshot":{}"#, h.handle), &format!(r#""snapshot":{}"#, h.handle + 1));
    assert_eq!(h.call("getTypeOfSymbol", &h.sp(&format!(r#""symbol":{forged}"#))).unwrap_err().kind, CheckerErrorKind::Client);
    // After release every handle is rejected.
    h.registry.release();
    assert_eq!(h.call("getTypeArguments", &h.sp(&format!(r#""type":{id}"#))).unwrap_err().kind, CheckerErrorKind::Client);
}

#[test]
fn replaced_api_checker_makes_old_handles_stale() {
    let h = fixture();
    let t = h.type_at(MAIN_FILE, MAIN, "box:");
    let id = num(&t, "id") as u32;
    let checker_id = h.registry.project_checker_id(&h.project);
    assert_ne!(checker_id, 0);
    assert!(h.registry.resolve_type(&h.project, id, Some(checker_id)).is_ok());
    let err = h.registry.resolve_type(&h.project, id, Some(checker_id + 1)).unwrap_err();
    assert!(err.message.contains("stale"), "{err}");
}

#[test]
fn unowned_methods_fall_through() {
    let h = fixture();
    let params = json::unmarshal(&h.sp("")).unwrap();
    for method in ["getSourceFile", "createSnapshot", "emit", "printNode"] {
        assert!(handle(&h, method, &params).is_none(), "{method}");
    }
}

impl TestHost {
    /// Handle of the `levels`-th ancestor of the token touching `needle`.
    fn ancestor_handle_at(&self, needle: &str, levels: usize) -> String {
        let sf = self.program().get_source_file(MAIN_FILE).unwrap();
        let mut node = tsrs_astnav::get_touching_property_name(sf, MAIN.find(needle).unwrap() as i32);
        for _ in 0..levels {
            node = node.parent().unwrap();
        }
        self.node_handle(node).unwrap()
    }

    fn declared(&self, needle: &str) -> Value {
        let s = self.symbol_at(MAIN_FILE, MAIN, needle);
        self.ok("getDeclaredTypeOfSymbol", &self.sp(&format!(r#""symbol":{}"#, self.symbol_ref(&s))))
    }

    fn sig0(&self, t: &Value) -> Value {
        arr(&self.ok("getSignaturesOfType", &self.sp(&format!(r#""type":{},"kind":0"#, num(t, "id")))))[0].clone()
    }

    fn tid(&self, method: &str, field_name: &str, t: &Value) -> Value {
        self.ok(method, &self.sp(&format!(r#""{field_name}":{}"#, num(t, "id"))))
    }
}

#[test]
fn locations_batches_and_symbol_relations() {
    let h = fixture();
    let b = h.handle_at(MAIN_FILE, MAIN, "box.value");
    let pp = h.handle_at(MAIN_FILE, MAIN, "p[1]");
    let syms = h.ok("getSymbolsAtLocations", &h.sp(&format!(r#""locations":["{b}","{pp}"]"#)));
    assert_eq!(names(&syms), ["box", "p"]);
    let one = h.ok("getSymbolAtLocation", &h.sp(&format!(r#""location":"{b}""#)));
    assert_eq!(str_of(field(&one, "name")), "box");
    let t = h.ok("getTypeAtLocation", &h.sp(&format!(r#""location":"{b}""#)));
    assert_eq!(h.type_string(&t), "Box<number>");
    let ts = h.ok("getTypeAtLocations", &h.sp(&format!(r#""locations":["{b}","{pp}"]"#)));
    assert_eq!(arr(&ts).iter().map(|t| h.type_string(t)).collect::<Vec<_>>(), ["Box<number>", "Pair<string, U>"]);
    let modules = h.ok("getSymbolsOfSourceFiles", &h.sp(r#""files":["/p/main.ts",{"uri":"file:///p/types.d.ts"}]"#));
    assert_eq!(names(&modules), ["\"/p/main\"", "\"/p/types\""]);
    // File-owned relations resolve without a snapshot.
    let dog_t = h.declared("Dog extends");
    let legs = arr(&h.ok("getPropertiesOfType", &h.sp(&format!(r#""type":{}"#, num(&dog_t, "id")))))[0].clone();
    let parent = h.ok("getParentOfSymbol", &format!(r#"{{"symbol":{}}}"#, h.symbol_ref(&legs)));
    assert_eq!(str_of(field(&parent, "name")), "Dog");
    let module = h.ok("getSymbolOfSourceFile", &h.sp(&format!(r#""file":"{MAIN_FILE}""#)));
    let exports = h.ok("getExportsOfSymbol", &format!(r#"{{"symbol":{}}}"#, h.symbol_ref(&module)));
    let export_names = names(&exports);
    assert_eq!(&export_names[..3], ["Pair", "U", "over"], "file-owned tables are ordered by declaration position");
    assert!(export_names.iter().any(|n| n == "uni"));
    // Local symbol -> export symbol.
    let scope = h.ok("getSymbolsInScope", &h.sp(&format!(r#""file":"{MAIN_FILE}","position":{},"meaning":{}"#, utf16_pos(MAIN, "box.value"), tsrs_ast::SymbolFlags::Value.bits())));
    let local_box = arr(&scope).iter().find(|s| str_of(field(s, "name")) == "box").unwrap().clone();
    let es = h.ok("getExportSymbolOfSymbol", &format!(r#"{{"symbol":{}}}"#, h.symbol_ref(&local_box)));
    let es2 = h.ok("getExportSymbolOfSymbolForChecker", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&local_box))));
    // Pinned Go: the local symbol (ExportValue, file-owned) maps to the exported variable `box` (flags 2).
    assert_eq!(num(&local_box, "flags"), tsrs_ast::SymbolFlags::ExportValue.bits() as u64);
    for exported in [&es, &es2] {
        assert_eq!(str_of(field(exported, "name")), "box");
        assert_eq!(num(exported, "flags"), tsrs_ast::SymbolFlags::BlockScopedVariable.bits() as u64);
    }
    // Shorthand, export specifier, references, narrowed type at location.
    let shorthand = h.ancestor_handle_at("box };", 1);
    let v = h.ok("getShorthandAssignmentValueSymbol", &h.sp(&format!(r#""location":"{shorthand}""#)));
    assert_eq!(str_of(field(&v, "name")), "box");
    let spec = h.ancestor_handle_at("ünïcödé as uni", 1);
    let local = h.ok("getExportSpecifierLocalTargetSymbol", &h.sp(&format!(r#""location":"{spec}""#)));
    assert_eq!(str_of(field(&local, "name")), "ünïcödé");
    let box_sym = h.symbol_at(MAIN_FILE, MAIN, "box:");
    let refs = h.ok("getReferencesToSymbolInFile", &h.sp(&format!(r#""file":"{MAIN_FILE}","symbol":{}"#, h.symbol_ref(&box_sym))));
    assert!(arr(&refs).len() >= 3, "{}", json::marshal(&refs).unwrap());
    let at = h.ok("getTypeOfSymbolAtLocation", &h.sp(&format!(r#""symbol":{},"location":"{b}""#, h.symbol_ref(&box_sym))));
    assert_eq!(h.type_string(&at), "Box<number>");
    let maybe = h.symbol_at(MAIN_FILE, MAIN, "maybe:");
    let nm = h.ok("getNonMissingTypeOfSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&maybe))));
    assert_eq!(h.type_string(&nm), "string | undefined");
    // Instantiated property -> target symbol; type of property.
    let box_t = h.type_at(MAIN_FILE, MAIN, "box:");
    let value = h.ok("getPropertyOfType", &h.sp(&format!(r#""type":{},"name":"value""#, num(&box_t, "id"))));
    let target = h.ok("getTargetSymbol", &h.sp(&format!(r#""symbol":{}"#, h.symbol_ref(&value))));
    assert_eq!(str_of(field(&target, "name")), "value");
    let vt = h.ok("getTypeOfPropertyOfType", &h.sp(&format!(r#""type":{},"name":"value""#, num(&box_t, "id"))));
    assert_eq!(h.type_string(&vt), "number");
    let sym_of_type = h.tid("getSymbolOfType", "objectId", &box_t);
    assert_eq!(str_of(field(&sym_of_type, "name")), "Box");
}

#[test]
fn type_structure_accessors() {
    let h = fixture();
    let box_target = h.tid("getTargetOfType", "objectId", &h.type_at(MAIN_FILE, MAIN, "box:"));
    assert_eq!(h.tid("getOuterTypeParametersOfType", "objectId", &box_target), Value::Null);
    let local = h.tid("getLocalTypeParametersOfType", "objectId", &box_target);
    assert_eq!(h.type_string(&arr(&local)[0]), "T");
    let this_t = h.tid("getThisTypeOfType", "objectId", &box_target);
    assert_eq!(field(&this_t, "isThisType"), &Value::Bool(true));
    let p = h.type_at(MAIN_FILE, MAIN, "p:");
    let alias_args = h.tid("getAliasTypeArgumentsOfType", "objectId", &p);
    assert_eq!(arr(&alias_args).iter().map(|t| h.type_string(t)).collect::<Vec<_>>(), ["string", "U"]);
    let ia = h.declared("IA<T, K");
    assert_eq!(h.type_string(&h.tid("getObjectTypeOfType", "objectId", &ia)), "T");
    let k = h.tid("getIndexTypeOfType", "objectId", &ia);
    assert_eq!(h.type_string(&k), "K");
    let k_base = h.tid("getBaseConstraintOfType", "type", &k);
    assert_eq!(h.type_string(&k_base), "string | number | symbol");
    let c = h.declared("C<T> =");
    assert_eq!(h.type_string(&h.tid("getExtendsTypeOfType", "objectId", &c)), "string");
    assert_eq!(h.type_string(&h.tid("getFalseTypeOfConditionalType", "objectId", &c)), "2");
    let m = h.declared("M = ");
    assert_eq!(h.type_string(&h.tid("getTemplateTypeOfMappedType", "objectId", &m)), "K");
    let lit = h.type_at(MAIN_FILE, MAIN, "lit =");
    let fresh = h.tid("getFreshTypeOfType", "objectId", &lit);
    assert_eq!(h.type_string(&fresh), "\"hi\"");
    assert_eq!(num(&fresh, "id"), num(&lit, "freshType"));
    assert_eq!(num(&regular_of(&h, &fresh), "id"), num(&lit, "regularType"));
    let wd = h.declared("WithDefault<T");
    let tp = arr(&h.tid("getTypeParametersOfType", "objectId", &wd))[0].clone();
    assert_eq!(h.type_string(&h.tid("getDefaultFromTypeParameter", "objectId", &tp)), "string");
    let up = h.declared("Up = ");
    assert_eq!(h.type_string(&up), "\"A\"");
    let tl = h.declared("TL = ");
    assert_eq!(field(&tl, "texts"), &json::unmarshal(r#"["x",""]"#).unwrap());
    assert_eq!(arr(&h.tid("getTypesOfType", "objectId", &tl)).iter().map(|t| h.type_string(t)).collect::<Vec<_>>(), ["string"]);
    // Substitution type: `T` in the true branch of `T extends string ? Box<T> : never`.
    let t_ref = h.ancestor_handle_at("T> : never", 1);
    let sub = h.ok("getTypeFromTypeNode", &h.sp(&format!(r#""location":"{t_ref}""#)));
    assert_ne!(num(&sub, "flags") as u32 & tsrs_checker::TypeFlags::Substitution.bits(), 0);
    assert_eq!(num(&sub, "baseType"), num(&h.tid("getBaseTypeOfType", "objectId", &sub), "id"));
    assert_eq!(h.type_string(&h.tid("getBaseTypeOfType", "objectId", &sub)), "T");
    assert_eq!(h.type_string(&h.tid("getConstraintOfType", "objectId", &sub)), "string");
    let arr_node = h.ancestor_handle_at("number[] = []", 1);
    let arr_t = h.ok("getTypeFromTypeNode", &h.sp(&format!(r#""location":"{arr_node}""#)));
    assert_eq!(h.type_string(&arr_t), "number[]");
    assert_eq!(h.tid("isArrayType", "type", &arr_t), Value::Bool(true));
    assert_eq!(h.tid("isArrayLikeType", "type", &arr_t), Value::Bool(true));
    assert_eq!(h.tid("isArrayType", "type", &h.type_at(MAIN_FILE, MAIN, "box:")), Value::Bool(false));
}

#[test]
fn checker_operations_on_types_and_signatures() {
    let h = fixture();
    let aw = h.type_at(MAIN_FILE, MAIN, "aw()");
    let ret = h.tid("getReturnTypeOfSignature", "objectId", &h.sig0(&aw));
    assert_eq!(h.type_string(&ret), "Promise<number>");
    assert_eq!(h.type_string(&h.tid("getAwaitedType", "type", &ret)), "number");
    let maybe = h.type_at(MAIN_FILE, MAIN, "maybe:");
    assert_eq!(h.type_string(&h.tid("getNonNullableType", "objectId", &maybe)), "string");
    let lit = h.type_at(MAIN_FILE, MAIN, "lit =");
    assert_eq!(h.type_string(&h.tid("getWidenedType", "type", &lit)), "\"hi\"");
    let num_t = h.ok("getNumberType", &h.sp(""));
    assert_eq!(h.type_string(&h.tid("getApparentType", "objectId", &num_t)), "Number");
    let u = h.declared("U = ");
    assert_eq!(h.type_string(&h.tid("getReducedType", "objectId", &u)), "U");
    let str_t = h.ok("getStringType", &h.sp(""));
    let apparent = h.tid("getApparentPropertiesOfType", "objectId", &str_t);
    assert!(names(&apparent).iter().any(|n| n == "length"));
    // Signatures: this parameter, rest type, type parameter at position, target of instantiation.
    let with_this = h.sig0(&h.type_at(MAIN_FILE, MAIN, "withThis("));
    let this_p = h.ok("getThisParameterOfSignature", &h.sp(&format!(r#""objectId":{}"#, num(&with_this, "id"))));
    assert_eq!(str_of(field(&this_p, "name")), "this");
    let rest = h.sig0(&h.type_at(MAIN_FILE, MAIN, "rest("));
    assert_eq!(h.type_string(&h.ok("getRestTypeOfSignature", &h.sp(&format!(r#""signature":{}"#, num(&rest, "id"))))), "number", "rest type is the rest element type (tryGetRestTypeOfSignature)");
    let tpa = h.ok("getTypeParameterAtPosition", &h.sp(&format!(r#""signature":{},"index":1"#, num(&rest, "id"))));
    assert_eq!(h.type_string(&tpa), "number", "getTypeAtPosition of a rest position is the element type");
    let make_call = h.ancestor_handle_at("make(1)", 1);
    let resolved = h.ok("getResolvedSignature", &h.sp(&format!(r#""location":"{make_call}""#)));
    assert_ne!(num(&resolved, "target"), 0);
    let target = h.ok("getTargetOfSignature", &h.sp(&format!(r#""objectId":{}"#, num(&resolved, "id"))));
    assert_eq!(num(&target, "id"), num(&resolved, "target"));
    let decl = str_of(&arr(field(&h.symbol_at(MAIN_FILE, MAIN, "over(x: string)"), "declarations"))[0]);
    let from_decl = h.ok("getSignatureFromDeclaration", &h.sp(&format!(r#""location":"{decl}""#)));
    assert_eq!(h.type_string(&h.tid("getReturnTypeOfSignature", "objectId", &from_decl)), "string");
    // Contextual typing.
    let arrow = h.ancestor_handle_at("n) => {}", 2);
    let ctx = h.ok("getContextualType", &h.sp(&format!(r#""location":"{arrow}""#)));
    assert_eq!(h.type_string(&ctx), "(n: number) => void");
    assert_eq!(h.ok("isContextSensitive", &h.sp(&format!(r#""location":"{arrow}""#))), Value::Bool(true));
    let one = h.ancestor_handle_at("1, 2]", 0);
    assert_eq!(h.ok("isContextSensitive", &h.sp(&format!(r#""location":"{one}""#))), Value::Bool(false));
    let map_call = h.ancestor_handle_at("map(x", 2);
    let arg_ctx = h.ok("getContextualTypeForArgument", &h.sp(&format!(r#""location":"{map_call}","index":0"#)));
    assert_eq!(h.type_string(&arg_ctx), "(value: number, index: number, array: number[]) => number");
    // Remaining intrinsics and flags-driven display.
    for (method, expected) in [("getBooleanType", "boolean"), ("getVoidType", "void"), ("getUndefinedType", "undefined"), ("getNullType", "null"), ("getBigIntType", "bigint")] {
        assert_eq!(h.type_string(&h.ok(method, &h.sp(""))), expected, "{method}");
    }
    let big = h.type_at(MAIN_FILE, MAIN, "big =");
    assert_eq!(field(&big, "value"), &Value::String("10".to_string()));
    let no_trunc = h.ok("typeToString", &h.sp(&format!(r#""type":{},"flags":{}"#, num(&u, "id"), tsrs_checker::TypeFormatFlags::InTypeAlias.bits())));
    assert_eq!(str_of(&no_trunc), "string | number");
}

#[test]
fn language_service_backed_methods() {
    let h = fixture();
    // getSignatureUsages: usages of the overloaded `over` (declarations excluded, call attached).
    let over = h.symbol_at(MAIN_FILE, MAIN, "over(x: string)");
    let decl = str_of(&arr(field(&over, "declarations"))[0]);
    let usages = h.ok("getSignatureUsages", &h.sp(&format!(r#""signatureDecl":"{decl}""#)));
    let usages = arr(&usages);
    assert_eq!(usages.len(), 1, "{}", json::marshal(&Value::Array(usages.clone())).unwrap());
    assert_eq!(str_of(field(&usages[0], "call")), h.ancestor_handle_at("over(42)", 1));
    // getReferencedSymbolsForNode on the `box` declaration name.
    let name = h.handle_at(MAIN_FILE, MAIN, "box:");
    let pos = MAIN.find("box:").unwrap();
    let refs = h.ok("getReferencedSymbolsForNode", &h.sp(&format!(r#""node":"{name}","position":{pos}"#)));
    let entry = &arr(&refs)[0];
    assert_eq!(str_of(field(field(entry, "symbol"), "name")), "box");
    assert!(arr(field(entry, "references")).len() >= 3);
    // Completions after `box.` with symbols.
    let at = utf16_pos(MAIN, "box.value") + 4;
    let list = h.ok("getCompletionsAtPosition", &h.sp(&format!(r#""file":"{MAIN_FILE}","position":{at},"includeSymbol":true"#)));
    let value = arr(field(&list, "entries")).iter().find(|e| str_of(field(e, "name")) == "value").expect("value completion").clone();
    assert_eq!(str_of(field(field(&value, "symbol"), "name")), "value");
    assert_eq!(field(&list, "isIncomplete"), &Value::Bool(false));
    let missing = h.ok("getCompletionsAtPosition", &h.sp(&format!(r#""file":"/p/nope.ts","position":0"#)));
    assert_eq!(missing, Value::Null);
    // Import adder: import `helper` from ./other into main.ts.
    let helper = h.symbol_at("/p/other.ts", OTHER, "helper");
    let edits = h.ok("getImportAdderEdits", &h.sp(&format!(r#""file":"{MAIN_FILE}","actions":[{{"kind":"importSymbol","symbol":{}}}]"#, h.symbol_ref(&helper))));
    let edits = arr(&edits);
    assert_eq!(edits.len(), 1, "{}", json::marshal(&Value::Array(edits.clone())).unwrap());
    assert!(str_of(field(&edits[0], "newText")).contains("helper"), "{}", json::marshal(&edits[0]).unwrap());
    let bad = h.call("getImportAdderEdits", &h.sp(&format!(r#""file":"{MAIN_FILE}","actions":[{{"kind":"nope"}}]"#))).unwrap_err();
    assert_eq!(bad.kind, CheckerErrorKind::Client);
}

#[test]
fn core_session_hook_maps_errors_and_encodings() {
    use crate::handler::{ErrorKind, Handler};
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(Vec::<(String, String)>::new(), true)));
    let session = crate::session::Session::new(crate::session::SessionOptions {
        cwd: "/".to_string(),
        default_library_path: bundled::lib_path(),
        fs,
        binary_responses: false,
        run_external_code: false,
    });
    let err = session.handle_request("getTypeAtPosition", br#"{"snapshot":1,"project":"/p/tsconfig.json","file":"/p/a.ts","position":0}"#).unwrap_err();
    assert_eq!(err.kind, ErrorKind::ClientError, "{err}");
    let err = session.handle_request("getTypeAtPosition", br#"{"snapshot":"x"}"#).unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest, "{err}");
    let err = session.handle_request("getTypeAtPosition", b"[1]").unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest, "{err}");
    assert_eq!(super::base64_encode(b""), "");
    assert_eq!(super::base64_encode(b"f"), "Zg==");
    assert_eq!(super::base64_encode(b"fo"), "Zm8=");
    assert_eq!(super::base64_encode(b"foobar"), "Zm9vYmFy");
}

/// Needles of the differential probe (same list the Go probe used to produce the golden file).
const PROBE_NEEDLES: &[&str] = &[
    "box:", "p:", "ünïcödé", "over(x: string)", "U = ", "Pair<A, B>", "isStr(", "Animal {", "Dog extends", "legs", "Red =", "r = over", "M = ",
    "C<T> =", "lit =", "fn =", "o = {", "box };", "uni", "IA<T", "aw()", "maybe:", "doubled", "x * 2", "cb:", "n) => {}", "withThis(", "rest(",
    "arr:", "S<T>", "WithDefault", "useAll", "box.value", "value +", "Up =", "TL =", "big =", "make }", "make(1)", "toString",
];

/// Differential check against the pinned Go API session (microsoft/TypeScript b85298b6, tsc/internal/api):
/// testdata/go_probe_b85298b6.jsonl was produced by driving Go's `Session.HandleRequest` with the same
/// fixture and the same request sequence (a local probe test in ts-ref, not part of this repo). Compares
/// type strings, type/symbol flags, symbol names and ownership, file-owned lookups without a snapshot,
/// resolveName with and without a location, constraint results and error texts. `objectFlags` are not
/// compared: they carry lazily computed cache bits (e.g. MembersResolved) that differ even between two
/// Go queries of the same type. Process-wide symbol ids are normalized to "same symbol as" booleans
/// (the golden file was normalized the same way). Set `TSRS_CHECKER_PROBE_OUT=<file>` to dump the tsrs lines.
#[test]
fn pinned_go_differential() {
    let lines = probe_lines();
    if let Ok(out) = std::env::var("TSRS_CHECKER_PROBE_OUT") {
        std::fs::write(out, lines.join("\n") + "\n").unwrap();
    }
    let golden = include_str!("testdata/go_probe_b85298b6.jsonl");
    let parse = |l: &str| -> (String, Value) {
        let v = json::unmarshal(l).unwrap();
        let q = str_of(field(&v, "q"));
        let mut r = field(&v, "r").clone();
        if let Value::Object(o) = &mut r {
            o.shift_remove("objectFlags");
        }
        (q, r)
    };
    let ours: std::collections::HashMap<String, Value> = lines.iter().map(|l| parse(l)).collect();
    let mut compared = 0;
    let mut diffs = Vec::new();
    for line in golden.lines().filter(|l| !l.is_empty()) {
        let (q, go) = parse(line);
        let rs = ours.get(&q).cloned().unwrap_or_else(|| Value::String("<missing>".to_string()));
        compared += 1;
        let same = match q.as_str() {
            // Same error class; the decoder wording differs (Go reports its json/v2 unmarshal error).
            "err.badType" => str_of(&rs).starts_with("api: invalid request: ") && str_of(&go).starts_with("api: invalid request: "),
            _ => rs == go,
        };
        if !same {
            diffs.push(format!("{q}\n  go: {}\n  rs: {}", json::marshal(&go).unwrap(), json::marshal(&rs).unwrap()));
        }
    }
    assert_eq!(compared, 71);
    assert!(diffs.is_empty(), "{} differences from pinned Go:\n{}", diffs.len(), diffs.join("\n"));
}

fn probe_lines() -> Vec<String> {
    let h = TestHost::with_configs(
        &[
            ("/p/tsconfig.json", TSCONFIG),
            ("/p/main.ts", MAIN),
            ("/p/types.d.ts", TYPES),
            ("/p/other.ts", OTHER),
            ("/q/tsconfig.json", r#"{"files":["q.ts"]}"#),
            ("/q/q.ts", "export const q: number = 1;\n"),
        ],
        &["/p/tsconfig.json", "/q/tsconfig.json"],
    );
    let mut lines = Vec::new();
    let mut emit = |q: &str, r: Value| {
        let mut o = super::json::obj();
        o.set("q", Value::String(q.to_string()));
        o.set("r", r);
        lines.push(json::marshal(&o.build()).unwrap());
    };
    let get = |v: &Value, k: &str| -> Value {
        match v {
            Value::Object(o) => o.get(k).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    };
    let res = |r: CheckerResult<Value>| -> (Value, String) {
        match r {
            Ok(v) => (v, String::new()),
            Err(e) => (Value::Null, e.to_string()),
        }
    };
    let ts = |t: &Value| -> Value {
        if *t == Value::Null {
            return Value::Null;
        }
        match h.call("typeToString", &h.sp(&format!(r#""type":{}"#, num(t, "id")))) {
            Ok(v) => v,
            Err(e) => Value::String(e.to_string()),
        }
    };
    let obj = |pairs: Vec<(&str, Value)>| {
        let mut o = super::json::obj();
        for (k, v) in pairs {
            o.set(k, v);
        }
        o.build()
    };
    let s = |x: &str| Value::String(x.to_string());
    let names = |v: &Value| match v {
        Value::Array(a) => Value::Array(a.iter().map(|x| get(x, "name")).collect()),
        other => other.clone(),
    };
    emit("flags", obj(vec![("Value", Value::Number(tsrs_ast::SymbolFlags::Value.bits() as f64)), ("Type", Value::Number(tsrs_ast::SymbolFlags::Type.bits() as f64))]));
    for n in PROBE_NEEDLES.iter().copied() {
        let pos = utf16_pos(MAIN, n);
        let (ty, e1) = res(h.call("getTypeAtPosition", &h.sp(&format!(r#""file":"/p/main.ts","position":{pos}"#))));
        let (sy, e2) = res(h.call("getSymbolAtPosition", &h.sp(&format!(r#""file":"/p/main.ts","position":{pos}"#))));
        emit(
            &format!("at:{n}"),
            obj(vec![
                ("type", ts(&ty)),
                ("typeFlags", get(&ty, "flags")),
                ("objectFlags", match get(&ty, "objectFlags") { Value::Null if ty != Value::Null => Value::Number(0.0), v => v }),
                ("sym", get(&sy, "name")),
                ("symFlags", get(&sy, "flags")),
                ("kind", get(&get(&sy, "reference"), "kind")),
                ("e", s(&(e1 + &e2))),
            ]),
        );
    }
    let sym_ref = |needle: &str| json::marshal(&get(&h.symbol_at(MAIN_FILE, MAIN, needle), "reference")).unwrap();
    let declared = |needle: &str| h.ok("getDeclaredTypeOfSymbol", &h.sp(&format!(r#""symbol":{}"#, sym_ref(needle))));
    let c = declared("C<T> =");
    let check = h.tid("getCheckTypeOfType", "objectId", &c);
    let (cons, e) = res(h.call("getConstraintOfTypeParameter", &h.sp(&format!(r#""objectId":{}"#, num(&check, "id")))));
    emit("C.check", obj(vec![("str", ts(&check)), ("flags", get(&check, "flags"))]));
    emit("C.constraint", obj(vec![("str", ts(&cons)), ("flags", get(&cons, "flags")), ("sameAsCheck", Value::Bool(get(&cons, "id") == get(&check, "id"))), ("sameSymbolAsCheck", Value::Bool(get(&get(&cons, "symbol"), "id") == get(&get(&check, "symbol"), "id"))), ("e", s(&e))]));
    if cons != Value::Null {
        let cons2 = h.tid("getConstraintOfTypeParameter", "objectId", &cons);
        emit("C.constraint.constraint", ts(&cons2));
    }
    let ia = declared("IA<T, K");
    let k = h.tid("getIndexTypeOfType", "objectId", &ia);
    emit("IA.K.constraint", ts(&h.tid("getConstraintOfTypeParameter", "objectId", &k)));
    let dog = declared("Dog extends");
    let props = h.tid("getPropertiesOfType", "type", &dog);
    emit("Dog.props", names(&props));
    let legs = arr(&props)[0].clone();
    emit("legs.ref.kind", get(&get(&legs, "reference"), "kind"));
    let (par, e) = res(h.call("getParentOfSymbol", &format!(r#"{{"symbol":{}}}"#, json::marshal(&get(&legs, "reference")).unwrap())));
    emit("legs.parent", obj(vec![("name", get(&par, "name")), ("kind", get(&get(&par, "reference"), "kind")), ("e", s(&e))]));
    let (mem, e) = res(h.call("getMembersOfSymbol", &format!(r#"{{"symbol":{}}}"#, sym_ref("Dog extends"))));
    emit("Dog.members", obj(vec![("names", names(&mem)), ("e", s(&e))]));
    let module = h.ok("getSymbolOfSourceFile", &h.sp(r#""file":"/p/main.ts""#));
    let module_ref = json::marshal(&get(&module, "reference")).unwrap();
    let (exp, e) = res(h.call("getExportsOfSymbol", &format!(r#"{{"symbol":{module_ref}}}"#)));
    emit("main.exportsOfSymbol", obj(vec![("names", names(&exp)), ("e", s(&e))]));
    let (expm, e) = res(h.call("getExportsOfModule", &h.sp(&format!(r#""symbol":{module_ref}"#))));
    emit("main.exportsOfModule", obj(vec![("names", names(&expm)), ("e", s(&e))]));
    let scope = h.ok("getSymbolsInScope", &h.sp(&format!(r#""file":"/p/main.ts","position":{},"meaning":{}"#, utf16_pos(MAIN, "box.value"), tsrs_ast::SymbolFlags::Value.bits())));
    for sym in arr(&scope) {
        if get(sym, "name") == s("box") {
            let r = json::marshal(&get(sym, "reference")).unwrap();
            let (es, e) = res(h.call("getExportSymbolOfSymbol", &format!(r#"{{"symbol":{r}}}"#)));
            emit("box.local.exportSymbol", obj(vec![("name", get(&es, "name")), ("localFlags", get(sym, "flags")), ("localKind", get(&get(sym, "reference"), "kind")), ("exportFlags", get(&es, "flags")), ("e", s(&e))]));
            let (es2, e) = res(h.call("getExportSymbolOfSymbolForChecker", &h.sp(&format!(r#""symbol":{r}"#))));
            emit("box.local.exportSymbolForChecker", obj(vec![("name", get(&es2, "name")), ("flags", get(&es2, "flags")), ("e", s(&e))]));
        }
    }
    let value = tsrs_ast::SymbolFlags::Value.bits();
    let ty = tsrs_ast::SymbolFlags::Type.bits();
    for (name, meaning) in [("Array", ty), ("box", value), ("Promise", value), ("Animal", value)] {
        for (label, extra) in [("noloc", String::new()), ("loc", format!(r#","file":"/p/main.ts","position":{}"#, utf16_pos(MAIN, "r = over")))] {
            let (r, e) = res(h.call("resolveName", &h.sp(&format!(r#""name":"{name}","meaning":{meaning}{extra}"#))));
            emit(&format!("resolveName.{label}:{name}"), obj(vec![("name", get(&r, "name")), ("flags", get(&r, "flags")), ("kind", get(&get(&r, "reference"), "kind")), ("e", s(&e))]));
        }
    }
    let boxt = h.type_at(MAIN_FILE, MAIN, "box:");
    let id = num(&boxt, "id");
    let (_, e) = res(h.call("getTypeArguments", &format!(r#"{{"snapshot":{},"project":"/other/tsconfig.json","type":{id}}}"#, h.handle)));
    emit("err.unknownProject", s(&e));
    let (r, e) = res(h.call("getTypeArguments", &format!(r#"{{"snapshot":{},"project":"/q/tsconfig.json","type":{id}}}"#, h.handle)));
    emit("err.foreignProjectType", obj(vec![("r", r), ("e", s(&e))]));
    let (_, e) = res(h.call("getTypeArguments", &h.sp(r#""type":999999"#)));
    emit("err.unknownType", s(&e));
    let (_, e) = res(h.call("getTypeArguments", &format!(r#"{{"snapshot":{},"project":{},"type":{id}}}"#, h.handle + 1000, json::marshal_string(&h.project))));
    emit("err.unknownSnapshot", s(&e));
    let (_, e) = res(h.call("getTypeArguments", &h.sp(r#""type":"x""#)));
    emit("err.badType", s(&e));
    let array = h.ok("resolveName", &h.sp(&format!(r#""name":"Array","meaning":{ty}"#)));
    let aref = json::marshal(&get(&array, "reference")).unwrap();
    let forged = aref.replace(&format!(r#""snapshot":{}"#, h.handle), &format!(r#""snapshot":{}"#, h.handle + 1));
    let (_, e) = res(h.call("getTypeOfSymbol", &h.sp(&format!(r#""symbol":{forged}"#))));
    emit("err.forgedSnapshotRef", s(&e));
    let (qt, e) = res(h.call("getTypeOfSymbol", &format!(r#"{{"snapshot":{},"project":"/q/tsconfig.json","symbol":{aref}}}"#, h.handle)));
    let qts = if qt == Value::Null { Value::Null } else { h.ok("typeToString", &format!(r#"{{"snapshot":{},"project":"/q/tsconfig.json","type":{}}}"#, h.handle, num(&qt, "id"))) };
    emit("crossProject.snapshotSymbol", obj(vec![("type", qts), ("e", s(&e))]));
    let (_, e) = res(h.call("getParentOfSymbol", r#"{"symbol":{"kind":0,"id":1}}"#));
    emit("err.fileRefNoDescriptor", s(&e));
    h.registry.release();
    let (_, e2) = res(h.call("getTypeArguments", &h.sp(&format!(r#""type":{id}"#))));
    emit("err.afterRelease", obj(vec![("release", s("")), ("e", s(&e2))]));
    let (_, e) = res(h.call("getParentOfSymbol", &format!(r#"{{"symbol":{}}}"#, json::marshal(&get(&legs, "reference")).unwrap())));
    emit("afterRelease.fileOwnedParent", s(&e));
    drop(emit);
    lines
}

fn regular_of(h: &TestHost, t: &Value) -> Value {
    h.tid("getRegularTypeOfType", "objectId", t)
}

#[test]
fn well_known_singletons_identify_checker_results() {
    let h = fixture();
    let wk = h.ok("getWellKnownSymbols", &h.sp(""));
    let args = h.symbol_at("/p/other.ts", OTHER, "arguments");
    assert_eq!(num(field(&args, "reference"), "kind"), 1, "arguments is a checker (transient) symbol");
    assert_eq!(num(field(&args, "reference"), "id"), num(&wk, "arguments"));
    let sigs = h.ok("getWellKnownSignatures", &h.sp(""));
    let call = {
        let sf = h.program().get_source_file("/p/other.ts").unwrap();
        let ident = tsrs_astnav::get_touching_property_name(sf, OTHER.find("notFn()").unwrap() as i32);
        h.node_handle(ident.parent().unwrap()).unwrap()
    };
    let resolved = h.ok("getResolvedSignature", &h.sp(&format!(r#""location":"{call}""#)));
    assert_eq!(num(&resolved, "id"), num(&sigs, "unknown"), "an uncallable call resolves to the unknown signature");
}
