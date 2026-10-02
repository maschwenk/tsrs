// Not a Go test: an end-to-end smoke test of the phase-1 language service (hover, definition, type definition,
// diagnostics) over an in-memory program with the bundled libs. The exact output is gated by the LSP oracle
// (tools/oracle/lsp); the expected strings here were written by hand from tsgo's behavior.

use std::sync::Arc;

use tsrs_compiler::{new_compiler_host, new_program, CompilerHost, Program, ProgramOptions};
use tsrs_core::context::Context;
use tsrs_core::P;
use tsrs_lsproto as lsproto;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{bundled, vfstest, FS};

use crate::autoimport::{ProjectID, Registry};
use crate::lsconv::{self, compute_lsp_line_starts, Converters};
use crate::lsutil::{new_default_user_preferences, UserPreferences};
use crate::sourcemap::ECMALineInfo;
use crate::{new_language_service, Host, LanguageService};

struct TestHost {
    fs: Arc<dyn FS>,
    converters: Arc<Converters>,
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

fn setup(files: &[(&str, &str)]) -> (LanguageService, Context) {
    setup_with(files, false)
}

fn setup_with(files: &[(&str, &str)], vs: bool) -> (LanguageService, Context) {
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|&(k, v)| (k, v)), true)));
    let config_host: &'static parseConfigHost = Box::leak(Box::new(parseConfigHost { fs: fs.clone() }));
    let (config, diagnostics) = tsoptions::get_parsed_command_line_of_config_file("/tsconfig.json", None, None, config_host, None);
    assert!(diagnostics.is_empty());
    let host: Arc<dyn CompilerHost> = new_compiler_host("/", fs.clone(), &bundled::lib_path(), None, None);
    let program: &'static Program = new_program(ProgramOptions::new(P::new(config.unwrap()), host));
    let fs_for_lines = fs.clone();
    let converters = lsconv::new_converters(lsproto::PositionEncodingKind::UTF16, move |file_name| {
        fs_for_lines.read_file(file_name).map(|text| compute_lsp_line_starts(&text))
    });
    let host = Arc::new(TestHost { fs, converters });
    let ls = new_language_service(ProjectID("/tsconfig.json".to_string()), program, host, "/index.ts");
    let mut caps = lsproto::ResolvedClientCapabilities::default();
    caps.text_document.hover.content_format = vec![lsproto::MarkupKind::Markdown];
    caps.text_document.definition.link_support = true;
    caps.text_document.type_definition.link_support = true;
    caps.vs_supports_visual_studio_extensions = vs;
    let ctx = lsproto::with_client_capabilities(&Context::background(), Arc::new(caps));
    (ls, ctx)
}

fn position_of(text: &str, marker: &str) -> lsproto::Position {
    let offset = text.find(marker).unwrap();
    let before = &text[..offset];
    let line = before.matches('\n').count() as u32;
    let character = (offset - before.rfind('\n').map(|i| i + 1).unwrap_or(0)) as u32;
    lsproto::Position { line, character }
}

const INDEX: &str = r#"/**
 * Adds two numbers.
 * @param a the first
 * @returns the sum
 */
function add(a: number, b: number): number { return a + b; }
interface Point { x: number; y: number }
const p: Point = { x: 1, y: 2 };
const total = add(p.x, p.y);
let s: string = 1;
"#;

fn json<T: lsproto::Json>(v: &T) -> String {
    tsrs_core::json::marshal(&v.to_json()).unwrap()
}

// Expected responses recorded from `tsgo-ref --lsp -stdio` with the same file and client capabilities
// (target/scratch/lscore/drive.py), with the project directory replaced by `/`.
#[test]
fn hover_definition_diagnostics() {
    let files = [("/tsconfig.json", r#"{"compilerOptions":{"strict":true},"files":["index.ts"]}"#), ("/index.ts", INDEX)];
    let (ls, ctx) = setup(&files);
    let uri = lsconv::file_name_to_document_uri("/index.ts");
    let hover = |marker: &str| {
        let params = lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX, marker), ..Default::default() };
        json(&ls.provide_hover(&ctx, &params).unwrap())
    };

    assert_eq!(
        hover("add(p.x"),
        r#"{"contents":{"kind":"markdown","value":"```typescript\nfunction add(a: number, b: number): number\n```\nAdds two numbers.\n\n*@param* `a` — the first\n\n*@returns* — the sum"},"range":{"start":{"line":8,"character":14},"end":{"line":8,"character":17}}}"#
    );
    assert_eq!(hover("total"), r#"{"contents":{"kind":"markdown","value":"```typescript\nconst total: number\n```\n"},"range":{"start":{"line":8,"character":6},"end":{"line":8,"character":11}}}"#);
    assert_eq!(hover("Point {"), r#"{"contents":{"kind":"markdown","value":"```typescript\ninterface Point\n```\n"},"range":{"start":{"line":6,"character":10},"end":{"line":6,"character":15}}}"#);
    assert_eq!(hover("x: 1"), r#"{"contents":{"kind":"markdown","value":"```typescript\n(property) Point.x: number\n```\n"},"range":{"start":{"line":7,"character":19},"end":{"line":7,"character":20}}}"#);

    assert_eq!(
        json(&ls.provide_definition(&ctx, &uri, position_of(INDEX, "add(p.x")).unwrap()),
        r#"[{"originSelectionRange":{"start":{"line":8,"character":14},"end":{"line":8,"character":17}},"targetUri":"file:///index.ts","targetRange":{"start":{"line":5,"character":0},"end":{"line":5,"character":60}},"targetSelectionRange":{"start":{"line":5,"character":9},"end":{"line":5,"character":12}}}]"#
    );
    assert_eq!(
        json(&ls.provide_type_definition(&ctx, &uri, position_of(INDEX, "p.x, p.y")).unwrap()),
        r#"[{"originSelectionRange":{"start":{"line":8,"character":18},"end":{"line":8,"character":19}},"targetUri":"file:///index.ts","targetRange":{"start":{"line":6,"character":0},"end":{"line":6,"character":40}},"targetSelectionRange":{"start":{"line":6,"character":10},"end":{"line":6,"character":15}}}]"#
    );
    assert_eq!(
        json(&ls.provide_diagnostics(&ctx, &uri).unwrap()),
        r#"{"kind":"full","items":[{"range":{"start":{"line":9,"character":4},"end":{"line":9,"character":5}},"severity":1,"code":2322,"source":"ts","message":"Type 'number' is not assignable to type 'string'."}]}"#
    );
}

const INDEX2: &str = r#"/** A widget. See {@link Gadget} and {@link https://example.com docs}.
 * @example
 * const w = new Widget<number>();
 * @see Gadget for more
 * @deprecated use Gadget
 */
class Widget<T extends object = {}> { value?: T; static count = 0; method(n: number): string { return ""; } }
class Gadget {}
enum Color { Red = 1, Green = "g" }
namespace NS { export const inner = 1; }
function over(a: string): string;
function over(a: number): number;
function over(a: any) { return a; }
type Alias<U> = { u: U } | null;
const w = new Widget<{}>();
w.method(1);
const c = Color.Green;
over(1);
let a: Alias<string> = null;
import G = NS.inner;
for (let i = 0; i < 1; i++) { this; }
"#;

// Expected responses recorded from `tsgo-ref --lsp -stdio` (target/scratch/lscore/drive.py, proj2).
#[test]
fn hover_jsdoc_aliases_overloads() {
    let files = [("/tsconfig.json", r#"{"compilerOptions":{"strict":true},"files":["index.ts"]}"#), ("/index.ts", INDEX2)];
    let (ls, ctx) = setup(&files);
    let uri = lsconv::file_name_to_document_uri("/index.ts");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "Widget<number>"), ..Default::default() }).unwrap()), r#"null"#, "{}", "Widget<number>");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "Widget<{}>"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\nconstructor Widget<{}>(): Widget<{}>\n```\nA widget. See [Gadget](file:///index.ts#8,7-8,13) and [docs](https://example.com).\n\n*@example*\n```tsx\nconst w = new Widget<number>();\n```\n\n\n*@see* — [Gadget](file:///index.ts#8,7-8,13) for more\n\n*@deprecated* — use Gadget"},"range":{"start":{"line":14,"character":14},"end":{"line":14,"character":20}}}"#, "{}", "Widget<{}>");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "method(1"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\n(method) Widget<{}>.method(n: number): string\n```\n"},"range":{"start":{"line":15,"character":2},"end":{"line":15,"character":8}}}"#, "{}", "method(1");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "Green;"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\n(enum member) Color.Green = \"g\"\n```\n"},"range":{"start":{"line":16,"character":16},"end":{"line":16,"character":21}}}"#, "{}", "Green;");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "NS {"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\nnamespace NS\n```\n"},"range":{"start":{"line":9,"character":10},"end":{"line":9,"character":12}}}"#, "{}", "NS {");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "over(1"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\nfunction over(a: number): number\n```\n"},"range":{"start":{"line":17,"character":0},"end":{"line":17,"character":4}}}"#, "{}", "over(1");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "Alias<string>"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\ntype Alias<U> = {\n    u: U;\n} | null\n```\n"},"range":{"start":{"line":18,"character":7},"end":{"line":18,"character":12}}}"#, "{}", "Alias<string>");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "U> ="), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\n(type parameter) U in type Alias<U>\n```\n"},"range":{"start":{"line":13,"character":11},"end":{"line":13,"character":12}}}"#, "{}", "U> =");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "G ="), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\n(alias) const NS.inner: 1\n```\n"},"range":{"start":{"line":19,"character":7},"end":{"line":19,"character":8}}}"#, "{}", "G =");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "count"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\n(property) Widget<T extends object = {}>.count: number\n```\n"},"range":{"start":{"line":6,"character":56},"end":{"line":6,"character":61}}}"#, "{}", "count");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "i <"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\nlet i: number\n```\n"},"range":{"start":{"line":20,"character":16},"end":{"line":20,"character":17}}}"#, "{}", "i <");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "this;"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\nthis: typeof globalThis\n```\n"},"range":{"start":{"line":20,"character":30},"end":{"line":20,"character":34}}}"#, "{}", "this;");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX2, "value?"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\n(property) Widget<T extends object = {}>.value?: T | undefined\n```\n"},"range":{"start":{"line":6,"character":38},"end":{"line":6,"character":43}}}"#, "{}", "value?");
    assert_eq!(json(&ls.provide_definition(&ctx, &uri, position_of(INDEX2, "Gadget}")).unwrap()), r#"[{"originSelectionRange":{"start":{"line":0,"character":25},"end":{"line":0,"character":31}},"targetUri":"file:///index.ts","targetRange":{"start":{"line":7,"character":0},"end":{"line":7,"character":15}},"targetSelectionRange":{"start":{"line":7,"character":6},"end":{"line":7,"character":12}}}]"#, "{}", "Gadget}");
    assert_eq!(json(&ls.provide_definition(&ctx, &uri, position_of(INDEX2, "G =")).unwrap()), r#"[{"originSelectionRange":{"start":{"line":19,"character":7},"end":{"line":19,"character":8}},"targetUri":"file:///index.ts","targetRange":{"start":{"line":9,"character":15},"end":{"line":9,"character":38}},"targetSelectionRange":{"start":{"line":9,"character":28},"end":{"line":9,"character":33}}}]"#, "{}", "G =");
    assert_eq!(json(&ls.provide_diagnostics(&ctx, &uri).unwrap()), r#"{"kind":"full","items":[{"range":{"start":{"line":6,"character":74},"end":{"line":6,"character":75}},"severity":4,"code":6133,"source":"ts","message":"'n' is declared but its value is never read."},{"range":{"start":{"line":14,"character":14},"end":{"line":14,"character":20}},"severity":4,"code":6385,"source":"ts","message":"'Widget' is deprecated."}]}"#, "{}", "");
}

// Visual Studio client: classified runs and `_vs_rawContent`, recorded from tsgo-ref (drive_vs.py).
#[test]
fn hover_visual_studio_raw_content() {
    let files = [("/tsconfig.json", r#"{"compilerOptions":{"strict":true},"files":["index.ts"]}"#), ("/index.ts", INDEX)];
    let (ls, ctx) = setup_with(&files, true);
    let uri = lsconv::file_name_to_document_uri("/index.ts");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX, "add(p.x"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\nfunction add(a: number, b: number): number;\n```\nAdds two numbers.\n\n*@param* `a` — the first\n\n*@returns* — the sum"},"range":{"start":{"line":8,"character":14},"end":{"line":8,"character":17}},"_vs_rawContent":{"Style":1,"Elements":[{"Style":0,"Elements":[{"ImageId":{"Guid":"ae27a6b0-e345-4288-96df-5eaf394ee369","Id":1880,"_vs_type":"ImageId"},"_vs_type":"ImageElement"},{"Runs":[{"ClassificationTypeName":"keyword","Text":"function ","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"method name","Text":"add","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"punctuation","Text":"(","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"parameter name","Text":"a","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"punctuation","Text":":","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"whitespace","Text":" ","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"keyword","Text":"number","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"punctuation","Text":",","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"whitespace","Text":" ","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"parameter name","Text":"b","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"punctuation","Text":":","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"whitespace","Text":" ","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"keyword","Text":"number","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"punctuation","Text":")","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"punctuation","Text":":","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"whitespace","Text":" ","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"keyword","Text":"number","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"punctuation","Text":";","_vs_type":"ClassifiedTextRun"}],"_vs_type":"ClassifiedTextElement"}],"_vs_type":"ContainerElement"},{"Runs":[{"ClassificationTypeName":"text","Text":"Adds two numbers.","_vs_type":"ClassifiedTextRun"}],"_vs_type":"ClassifiedTextElement"}],"_vs_type":"ContainerElement"}}"#, "{}", "add(p.x");
    assert_eq!(json(&ls.provide_hover(&ctx, &lsproto::HoverParams { text_document: lsproto::TextDocumentIdentifier { uri: uri.clone() }, position: position_of(INDEX, "x: 1"), ..Default::default() }).unwrap()), r#"{"contents":{"kind":"markdown","value":"```typescript\n(property) Point.x: number\n```\n"},"range":{"start":{"line":7,"character":19},"end":{"line":7,"character":20}},"_vs_rawContent":{"Style":0,"Elements":[{"ImageId":{"Guid":"ae27a6b0-e345-4288-96df-5eaf394ee369","Id":2436,"_vs_type":"ImageId"},"_vs_type":"ImageElement"},{"Runs":[{"ClassificationTypeName":"punctuation","Text":"(","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"text","Text":"property","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"punctuation","Text":") ","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"property name","Text":"Point.x","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"punctuation","Text":": ","_vs_type":"ClassifiedTextRun"},{"ClassificationTypeName":"keyword","Text":"number","_vs_type":"ClassifiedTextRun"}],"_vs_type":"ClassifiedTextElement"}],"_vs_type":"ContainerElement"}}"#, "{}", "x: 1");
}
