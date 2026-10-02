// Not a Go test: an end-to-end smoke test of references, rename, prepareRename, document highlights,
// implementations and call hierarchy over an in-memory program with the bundled libs. The requests
// (testdata/refs_smoke/reqs.json) and the expected responses (expected.txt) were recorded from
// `tsgo-ref --lsp -stdio` with testdata/refs_smoke/drive.py (`python3 drive.py <dir with a.ts b.ts tsconfig.json>
// reqs.json`), with the project directory replaced by `/`.

use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use tsrs_compiler::{new_compiler_host, new_program, CheckerHandle, CheckerPool, CompilerHost, PooledChecker, Program, ProgramOptions};
use tsrs_core::context::Context;
use tsrs_core::json::{self, Value};
use tsrs_core::P;
use tsrs_lsproto::{self as lsproto, Json};
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{bundled, vfstest, FS};

use crate::autoimport::{ProjectID, Registry};
use crate::lsconv::{self, compute_lsp_line_starts, Converters};
use crate::lsutil::{new_default_user_preferences, UserPreferences};
use crate::sourcemap::ECMALineInfo;
use crate::{new_language_service, Host, LanguageService};

struct dataset {
    a: &'static str,
    b: &'static str,
    tsconfig: &'static str,
    reqs: &'static str,
    expected: &'static str,
    // Client capabilities: Visual Studio extensions and implementation links (recorded with CAPS set).
    vs: bool,
    // The first file's name (a JS file for JS-only behavior).
    a_name: &'static str,
}

macro_rules! dataset {
    ($dir:literal) => {
        dataset {
            a: include_str!(concat!("../testdata/", $dir, "/a.ts")),
            b: include_str!(concat!("../testdata/", $dir, "/b.ts")),
            tsconfig: include_str!(concat!("../testdata/", $dir, "/tsconfig.json")),
            reqs: include_str!(concat!("../testdata/", $dir, "/reqs.json")),
            expected: include_str!(concat!("../testdata/", $dir, "/expected.txt")),
            vs: false,
            a_name: "a.ts",
        }
    };
}

struct testHost {
    fs: Arc<dyn FS>,
    converters: Arc<Converters>,
}

impl Host for testHost {
    fn use_case_sensitive_file_names(&self) -> bool {
        true
    }
    fn read_file(&self, path: &str) -> Option<String> {
        self.fs.read_file(path)
    }
    fn converters(&self) -> Arc<Converters> {
        self.converters.clone()
    }
    fn get_preferences(&self, _active_file: &str) -> UserPreferences {
        new_default_user_preferences()
    }
    fn get_ecma_line_info(&self, _file_name: &str) -> Option<Arc<ECMALineInfo>> {
        None
    }
    fn auto_import_registry(&self) -> Option<Arc<Registry>> {
        None
    }
    fn read_directory(&self, _: &str, _: &str, _: &[String], _: &[String], _: &[String], _: usize) -> Vec<String> {
        Vec::new()
    }
    fn get_directories(&self, _path: &str) -> Vec<String> {
        Vec::new()
    }
    fn directory_exists(&self, path: &str) -> bool {
        self.fs.directory_exists(path)
    }
    fn file_exists(&self, path: &str) -> bool {
        self.fs.file_exists(path)
    }
}

struct parseConfigHost {
    fs: Arc<dyn FS>,
}

impl ParseConfigHost for parseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        "/"
    }
}

// A minimal stand-in for the project system's query-checker pool: like Go's project pool (and unlike the
// compiler pool, whose checkers are locked per acquisition here), a nested acquisition while a checker is held
// gets another checker. Call hierarchy acquires checkers while holding one, as Go does.
struct testPoolState {
    program: &'static Program,
    slots: Mutex<Vec<(PooledChecker, bool)>>,
}

struct testPool(&'static testPoolState);

impl CheckerPool for testPool {
    fn get_checker(&self, _ctx: &Context, _file: Option<P<tsrs_ast::SourceFile>>) -> CheckerHandle {
        let state = self.0;
        let mut slots = state.slots.lock().unwrap();
        let index = match slots.iter().position(|(_, held)| !held) {
            Some(index) => index,
            None => {
                slots.push((PooledChecker::new(tsrs_checker::new_checker(state.program)), false));
                slots.len() - 1
            }
        };
        slots[index].1 = true;
        let checker: NonNull<tsrs_compiler::Checker> = slots[index].0.as_non_null();
        drop(slots);
        // SAFETY: the checker is boxed (stable address), never removed from `slots`, and marked held until the
        // release function runs, so nothing else hands it out meanwhile (single-threaded test).
        unsafe {
            CheckerHandle::from_raw(checker, move || {
                state.slots.lock().unwrap()[index].1 = false;
            })
        }
    }
}

fn setup(d: &dataset) -> (LanguageService, Context) {
    let a_path = format!("/{}", d.a_name);
    let files = [("/tsconfig.json", d.tsconfig), (a_path.as_str(), d.a), ("/b.ts", d.b)];
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|&(k, v)| (k, v)), true)));
    let config_host: &'static parseConfigHost = Box::leak(Box::new(parseConfigHost { fs: fs.clone() }));
    let (config, diagnostics) = tsoptions::get_parsed_command_line_of_config_file("/tsconfig.json", None, None, config_host, None);
    assert!(diagnostics.is_empty());
    let host: Arc<dyn CompilerHost> = new_compiler_host("/", fs.clone(), &bundled::lib_path(), None, None);
    let mut options = ProgramOptions::new(P::new(config.unwrap()), host);
    options.create_checker_pool = Some(Arc::new(|program: &'static Program| {
        let state: &'static testPoolState = Box::leak(Box::new(testPoolState { program, slots: Mutex::new(Vec::new()) }));
        Box::new(testPool(state)) as Box<dyn CheckerPool>
    }));
    let program: &'static Program = new_program(options);
    program.bind_source_files();
    let fs_for_lines = fs.clone();
    // Cached like the project system's snapshot line maps (keyword references land thousands of times in the libs).
    let line_maps: Mutex<rustc_hash::FxHashMap<String, Arc<lsconv::LSPLineMap>>> = Mutex::new(Default::default());
    let converters = lsconv::new_converters(lsproto::PositionEncodingKind::UTF16, move |file_name| {
        if let Some(line_map) = line_maps.lock().unwrap().get(file_name) {
            return Some(line_map.clone());
        }
        let line_map = fs_for_lines.read_file(file_name).map(|text| compute_lsp_line_starts(&text))?;
        line_maps.lock().unwrap().insert(file_name.to_string(), line_map.clone());
        Some(line_map)
    });
    let host = Arc::new(testHost { fs, converters });
    let ls = new_language_service(ProjectID("/tsconfig.json".to_string()), program, host, "/b.ts");
    let mut caps = lsproto::ResolvedClientCapabilities::default();
    caps.vs_supports_visual_studio_extensions = d.vs;
    caps.text_document.implementation.link_support = d.vs;
    let ctx = lsproto::with_client_capabilities(&Context::background(), Arc::new(caps));
    (ls, ctx)
}

fn position_of(text: &str, marker: &str, off: usize) -> Value {
    let offset = text.find(marker).unwrap() + off;
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let character = offset - before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let mut pos = tsrs_core::collections::OrderedMap::default();
    pos.insert("line".to_string(), Value::Number(line as f64));
    pos.insert("character".to_string(), Value::Number(character as f64));
    Value::Object(pos)
}

fn to_line<T: Json>(v: &T) -> String {
    json::marshal(&v.to_json()).unwrap()
}

fn decode<T: Json>(v: &Value) -> T {
    T::from_json(v).unwrap()
}

fn get<'v>(v: &'v Value, key: &str) -> Option<&'v Value> {
    match v {
        Value::Object(m) => m.get(key),
        _ => None,
    }
}

fn str_of(v: &Value) -> &str {
    match v {
        Value::String(s) => s,
        _ => panic!("not a string"),
    }
}

// a.ts/b.ts: cross-file references through named/namespace/default imports and aliases, shorthand properties,
// labels, `this`; rename of import aliases, shorthand properties, parameter properties; highlights (semantic,
// if/else, return/throw, break/label, multi-document); implementations; call hierarchy.
#[test]
fn references_rename_highlights_call_hierarchy() {
    run(&dataset!("refs_smoke"));
}

// Classes (constructor/super/static/private/parameter properties), export specifiers with aliases, anonymous
// default export, string literal types, keyword references into the lib, destructuring, namespaces, structural
// property references; renames through export aliases, binding elements and string literals.
#[test]
fn references_rename_classes_exports_destructuring() {
    run(&dataset!("refs_smoke2"));
}

// Visual Studio client (`_vs_references` with classified definition text) and implementation location links;
// recorded with CAPS='{"_vs_supportsVisualStudioExtensions":true,"textDocument":{"implementation":{"linkSupport":true}}}'.
#[test]
fn vs_references_and_implementation_links() {
    run(&dataset { vs: true, ..dataset!("refs_smoke_vs") });
}

fn run(d: &dataset) {
    let (ls, ctx) = setup(d);
    let reqs = json::unmarshal(d.reqs).unwrap();
    let Value::Array(reqs) = reqs else { panic!() };
    let expected: Vec<&str> = d.expected.lines().collect();
    assert_eq!(reqs.len(), expected.len());
    let mut failures = Vec::new();
    for (r, want) in reqs.iter().zip(expected) {
        let method = str_of(get(r, "m").unwrap());
        let file = str_of(get(r, "f").unwrap());
        let at = str_of(get(r, "at").unwrap());
        let off = get(r, "off").map_or(0, |v| if let Value::Number(n) = v { *n as usize } else { 0 });
        let text = if file == d.a_name { d.a } else { d.b };
        let mut params = tsrs_core::collections::OrderedMap::default();
        let mut text_document = tsrs_core::collections::OrderedMap::default();
        text_document.insert("uri".to_string(), Value::String(format!("file:///{}", file)));
        params.insert("textDocument".to_string(), Value::Object(text_document));
        params.insert("position".to_string(), position_of(text, at, off));
        if let Some(Value::Object(extra)) = get(r, "params") {
            for (k, v) in extra {
                params.insert(k.clone(), v.clone());
            }
        }
        let params = Value::Object(params);
        let started = std::time::Instant::now();
        let got = match method {
            "textDocument/references" => to_line(&ls.provide_references(&ctx, &decode(&params), None).unwrap()),
            "textDocument/_vs_references" => to_line(&ls.provide_vs_references(&ctx, &decode(&params), None).unwrap()),
            "textDocument/rename" => to_line(&ls.provide_rename(&ctx, &decode(&params), None).unwrap()),
            "textDocument/prepareRename" => {
                let p: lsproto::PrepareRenameParams = decode(&params);
                let info = ls.get_rename_info(&ctx, "", &p.text_document.uri, p.position);
                if !info.can_rename {
                    format!(r#"{{"error":{{"code":-32803,"message":"{}"}}}}"#, info.localized_error_message)
                } else {
                    to_line(&lsproto::PrepareRenamePlaceholder { range: info.trigger_span, placeholder: info.display_name })
                }
            }
            "textDocument/documentHighlight" => {
                let p: lsproto::DocumentHighlightParams = decode(&params);
                to_line(&ls.provide_document_highlights(&ctx, &p.text_document.uri, p.position).unwrap())
            }
            "custom/textDocument/multiDocumentHighlight" => {
                let p: lsproto::MultiDocumentHighlightParams = decode(&params);
                to_line(&ls.provide_multi_document_highlights(&ctx, &p.text_document.uri, p.position, &p.files_to_search).unwrap())
            }
            "textDocument/implementation" => to_line(&ls.provide_implementations(&ctx, &decode(&params), None).unwrap()),
            "textDocument/prepareCallHierarchy" => {
                let p: lsproto::CallHierarchyPrepareParams = decode(&params);
                to_line(&ls.provide_prepare_call_hierarchy(&ctx, &p.text_document.uri, p.position).unwrap())
            }
            "callHierarchy/incomingCalls" | "callHierarchy/outgoingCalls" => {
                let p: lsproto::CallHierarchyPrepareParams = decode(&params);
                let items = ls.provide_prepare_call_hierarchy(&ctx, &p.text_document.uri, p.position).unwrap().call_hierarchy_items.unwrap();
                if method == "callHierarchy/incomingCalls" {
                    to_line(&ls.provide_call_hierarchy_incoming_calls(&ctx, &items[0], None).unwrap())
                } else {
                    to_line(&ls.provide_call_hierarchy_outgoing_calls(&ctx, &items[0]).unwrap())
                }
            }
            _ => panic!("unknown method {}", method),
        };
        if std::env::var("REFS_SMOKE_TIMES").is_ok() { eprintln!("{:?} {} {}", started.elapsed(), method, at); }
        if got != want {
            failures.push(format!("{} {} {:?}\n  want: {}\n  got:  {}", method, file, at, want, got));
        }
    }
    assert!(failures.is_empty(), "{} of {} responses differ:\n{}", failures.len(), d.expected.lines().count(), failures.join("\n"));
}
