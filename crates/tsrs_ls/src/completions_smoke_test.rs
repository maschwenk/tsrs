// Not a Go test: an end-to-end smoke test of completions (textDocument/completion, completionItem/resolve,
// linked editing, auto-insert) over an in-memory program with the bundled libs. The expected responses in
// testdata/completions_smoke/*.out were recorded from `tsgo-ref --lsp -stdio` with the same project, client
// capabilities and user preferences (tools/oracle/completions/record.py); project paths are rewritten to `/`.
//
// Go returns some completion lists in map order (globals, path completions), and clients sort them by sort text
// and label (fourslash does the same with ls.CompareCompletionEntries), so both lists are stably sorted that way
// before comparing.

use std::sync::Arc;

use tsrs_compiler::{new_compiler_host, new_program, CompilerHost, Program, ProgramOptions};
use tsrs_core::collections::OrderedMap;
use tsrs_core::context::Context;
use tsrs_core::json::{self, Value};
use tsrs_core::P;
use tsrs_lsproto::{self as lsproto, Json};
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{bundled, vfsmatch, vfstest, FS};

use crate::autoimport::{ProjectID, Registry};
use crate::lsconv::{self, compute_lsp_line_starts, Converters};
use crate::lsutil::{parse_user_preferences, UserPreferences};
use crate::sourcemap::ECMALineInfo;
use crate::{compare_completion_entries, new_language_service, Host, LanguageService};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/completions_smoke");

struct TestHost {
    fs: Arc<dyn FS>,
    converters: Arc<Converters>,
    preferences: UserPreferences,
}

impl Host for TestHost {
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
        self.preferences.clone()
    }
    fn get_ecma_line_info(&self, _file_name: &str) -> Option<Arc<ECMALineInfo>> {
        None
    }
    fn auto_import_registry(&self) -> Option<Arc<Registry>> {
        None
    }
    fn read_directory(&self, current_dir: &str, path: &str, extensions: &[String], excludes: &[String], includes: &[String], depth: usize) -> Vec<String> {
        vfsmatch::read_directory(&*self.fs, current_dir, path, extensions, excludes, includes, depth)
    }
    fn get_directories(&self, path: &str) -> Vec<String> {
        self.fs.get_accessible_entries(path).directories
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

fn project_files() -> Vec<(String, String)> {
    let mut files = Vec::new();
    fn walk(dir: &std::path::Path, rel: &str, files: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            let path = format!("{}/{}", rel, name);
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), &path, files);
            } else {
                files.push((path, std::fs::read_to_string(entry.path()).unwrap()));
            }
        }
    }
    walk(&std::path::Path::new(DIR).join("proj"), "", &mut files);
    files
}

fn setup(spec: &Value) -> (LanguageService, Context, Vec<(String, String)>) {
    let files = project_files();
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|(k, v)| (k.as_str(), v.as_str())), true)));
    let config_host: &'static parseConfigHost = Box::leak(Box::new(parseConfigHost { fs: fs.clone() }));
    let (config, diagnostics) = tsoptions::get_parsed_command_line_of_config_file("/tsconfig.json", None, None, config_host, None);
    assert!(diagnostics.is_empty());
    let host: Arc<dyn CompilerHost> = new_compiler_host("/", fs.clone(), &bundled::lib_path(), None, None, None);
    let program: &'static Program = new_program(ProgramOptions::new(P::new(config.unwrap()), host));
    let fs_for_lines = fs.clone();
    let converters = lsconv::new_converters(lsproto::PositionEncodingKind::UTF16, move |file_name| fs_for_lines.read_file(file_name).map(|text| compute_lsp_line_starts(&text)));
    // Go: lsutil.ParseUserPreferences(map[string]any{"js/ts": initializationOptions.userPreferences})
    let mut items: OrderedMap<String, Value> = OrderedMap::default();
    items.insert("js/ts".to_string(), field(spec, "preferences").clone());
    let preferences = parse_user_preferences(&items);
    let host = Arc::new(TestHost { fs, converters, preferences });
    let ls = new_language_service(ProjectID("/tsconfig.json".to_string()), program, host, "/index.ts");
    let caps = lsproto::ClientCapabilities::from_json(field(spec, "capabilities")).unwrap();
    let ctx = lsproto::with_client_capabilities(&Context::background(), Arc::new(caps.resolve()));
    (ls, ctx, files)
}

fn field<'a>(v: &'a Value, name: &str) -> &'a Value {
    match v {
        Value::Object(m) => m.get(name).unwrap_or(&Value::Null),
        _ => &Value::Null,
    }
}

fn str_field<'a>(v: &'a Value, name: &str) -> &'a str {
    match field(v, name) {
        Value::String(s) => s,
        _ => "",
    }
}

fn position_of(text: &str, caret: &str) -> lsproto::Position {
    let needle = caret.replace('|', "");
    let offset = text.find(&needle).unwrap_or_else(|| panic!("caret not found: {caret}")) + caret.find('|').unwrap();
    let before = &text[..offset];
    let line = before.matches('\n').count() as u32;
    let character = before[before.rfind('\n').map(|i| i + 1).unwrap_or(0)..].encode_utf16().count() as u32;
    lsproto::Position { line, character }
}

// Sorts completion list items the way clients (and fourslash) do, then renders the result as JSON.
fn normalize(result: &Value) -> String {
    let mut result = result.clone();
    if let Value::Object(m) = &mut result {
        if let Some(Value::Array(items)) = m.get("items") {
            let mut parsed: Vec<lsproto::CompletionItem> = items.iter().map(|i| lsproto::CompletionItem::from_json(i).unwrap()).collect();
            parsed.sort_by(|a, b| compare_completion_entries(a, b).cmp(&0));
            let sorted: Vec<Value> = parsed.iter().map(|i| i.to_json()).collect();
            m.insert("items".to_string(), Value::Array(sorted));
        }
    }
    json::marshal(&result).unwrap()
}

fn uri_of(case: &Value) -> lsproto::DocumentUri {
    lsconv::file_name_to_document_uri(&format!("/{}", str_field(case, "file")))
}

fn position_in(case: &Value, files: &[(String, String)]) -> lsproto::Position {
    let file = str_field(case, "file");
    let text = &files.iter().find(|(k, _)| k.trim_start_matches('/') == file).unwrap().1;
    position_of(text, str_field(case, "caret"))
}

fn read_expected(name: &str) -> OrderedMap<String, Value> {
    let mut expected: OrderedMap<String, Value> = OrderedMap::default();
    let Ok(text) = std::fs::read_to_string(format!("{DIR}/{name}.out")) else {
        return expected;
    };
    for line in text.lines() {
        let v = json::unmarshal(line).unwrap();
        expected.insert(str_field(&v, "id").to_string(), v);
    }
    expected
}

fn rich_expected() -> OrderedMap<String, Value> {
    read_expected("rich")
}

fn run_profile(name: &str) {
    let spec = json::unmarshal(&std::fs::read_to_string(format!("{DIR}/{name}.json")).unwrap()).unwrap();
    let expected = read_expected(name);
    let (ls, ctx, files) = setup(&spec);
    let Value::Array(cases) = field(&spec, "cases") else { panic!("cases") };
    let mut failures = Vec::new();
    for case in cases {
        let id = str_field(case, "id");
        let method = str_field(case, "method");
        if let Value::String(message) = field(case, "expect_error") {
            let err = ls.provide_completion(&ctx, &uri_of(case), position_in(case, &files), None).unwrap_err();
            if err.message != *message {
                failures.push(format!("{id}: want error {message:?}, got {:?}", err.message));
            }
            continue;
        }
        let want = match field(case, "expect_same_as") {
            Value::String(other) => &rich_expected()[other.as_str()],
            _ => &expected[id],
        };
        let actual: Value = if method == "completionItem/resolve" {
            let from = &expected[str_field(case, "from")];
            let Value::Array(items) = field(field(from, "result"), "items") else { panic!("items") };
            let item = items.iter().find(|i| str_field(i, "label") == str_field(case, "label")).unwrap();
            let item = lsproto::CompletionItem::from_json(item).unwrap();
            let data = item.data.clone();
            ls.resolve_completion_item(&ctx, item, data.as_ref()).unwrap().to_json()
        } else {
            let file = str_field(case, "file");
            let text = &files.iter().find(|(k, _)| k.trim_start_matches('/') == file).unwrap().1;
            let uri = lsconv::file_name_to_document_uri(&format!("/{file}"));
            let position = position_of(text, str_field(case, "caret"));
            match method {
                "textDocument/completion" => {
                    let context = match field(case, "context") {
                        Value::Null => lsproto::CompletionContext { trigger_kind: lsproto::CompletionTriggerKind::Invoked, ..Default::default() },
                        c => lsproto::CompletionContext::from_json(c).unwrap(),
                    };
                    ls.provide_completion(&ctx, &uri, position, Some(&context)).unwrap().to_json()
                }
                "textDocument/linkedEditingRange" => {
                    let params = lsproto::LinkedEditingRangeParams { text_document: lsproto::TextDocumentIdentifier { uri }, position, ..Default::default() };
                    ls.provide_linked_editing_range(&ctx, &params).unwrap().to_json()
                }
                "textDocument/_vs_onAutoInsert" => {
                    let params = lsproto::VSOnAutoInsertParams {
                        vs_text_document: lsproto::TextDocumentIdentifier { uri },
                        vs_position: position,
                        vs_ch: str_field(case, "ch").to_string(),
                        ..Default::default()
                    };
                    ls.provide_on_auto_insert(&ctx, &params).unwrap().to_json()
                }
                _ => panic!("unknown method {method}"),
            }
        };
        let want = normalize(field(want, "result"));
        let got = normalize(&actual);
        if want != got {
            failures.push(format!("{id}:\n  want {want}\n  got  {got}"));
        }
    }
    assert!(failures.is_empty(), "{} of {} responses differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
}

#[test]
fn completions_rich_client() {
    run_profile("rich");
}

#[test]
fn completions_minimal_client() {
    run_profile("min");
}

#[test]
fn completions_preferences() {
    run_profile("pref");
}

// The auto-import registry is a placeholder that is never prepared (docs/LSP.md "Known gaps"), so completions
// that would collect auto-imports or build an import adder take Go's ErrNeedsAutoImports path (the server then
// asks the session for a language service with auto-imports and retries).
#[test]
fn completions_registry_not_prepared() {
    run_profile("autoimports");
}
