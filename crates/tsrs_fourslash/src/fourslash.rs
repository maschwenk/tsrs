// Port of Go's fourslash/fourslash.go: the FourslashTest harness the generated tests drive. The tests talk to
// the real language server (tsrs_lsp) in-process over framed byte pipes (tsrs_lsp::lsptestutil, Go's
// testutil/lsptestutil). Methods for language-service features that are not ported yet fail the test through
// `FourslashTest::server_unavailable` ("feature not ported: <name>").

use std::sync::{Arc, LazyLock, Mutex, OnceLock, RwLock};

use rustc_hash::FxHashMap;
use tsrs_core::collections::{MultiMap, OrderedMap};
use tsrs_core::json::Value;
use tsrs_core::stringutil;
use tsrs_core::tspath;
use tsrs_core::{CompilerOptions, JsxEmit, ScriptTarget, TextChange, TextPos, TextRange, Tristate, P};
use tsrs_ls::lsconv::{self, Converters, LSPLineMap, Script};
use tsrs_ls::lsutil;
use tsrs_ls::spanmap::{Feature, SpanMap};
use tsrs_lsp::lsptestutil::{self, closeClient, LSPClient};
use tsrs_lsproto as lsproto;
use tsrs_lsproto::{Json, RequestMessage, ResponseMessage};
use tsrs_vfs::FS;

pub use crate::baselineutil::*;
pub use crate::contentmapper::Spawner;
pub use crate::go::Any;
pub use crate::semantictokens::*;
pub use crate::statebaseline::*;
pub use crate::test_parser::*;
use crate::harnessutil;
use crate::testing::T;

// fourslash.go:46
pub struct FourslashTest {
    pub(crate) client: Option<Arc<LSPClient>>,
    // Go's closeClient closure (called by `done`); dropping the test closes it too, so a test that fails before
    // `done` does not leave the server threads running.
    pub(crate) close_client: Option<closeClient>,
    pub(crate) vfs: Arc<dyn FS>,
    pub(crate) test_data: TestData, // !!! consolidate test files from test data and script info
    pub(crate) baselines: OrderedMap<BaselineCommand, String>,
    // (a OnceLock so that GetRangesByText can take &self: tests call it in arguments of other methods)
    pub(crate) ranges_by_text: OnceLock<Arc<MultiMap<String, Arc<RangeMarker>>>>,
    pub(crate) open_files: OrderedMap<String, ()>,
    pub(crate) state_baseline: Option<StateBaseline>,

    // Go's map[string]*scriptInfo is shared with the converters' line-map callback.
    pub(crate) script_infos: Arc<RwLock<FxHashMap<String, ScriptInfo>>>,
    pub(crate) converters: TestConverters,

    pub(crate) state_enable_formatting: bool,
    pub(crate) report_format_on_type_crash: bool,
    pub(crate) user_preferences: lsutil::UserPreferences,
    // handleServerRequest reads f.userPreferences on the client's message-router goroutine; the Rust handler
    // reads this copy, kept equal to `user_preferences` by Configure.
    pub(crate) shared_user_preferences: Arc<Mutex<lsutil::UserPreferences>>,
    pub(crate) current_caret_position: lsproto::Position,
    pub(crate) last_known_marker_name: Option<String>,
    pub(crate) active_filename: String,
    pub(crate) selection_end: Option<lsproto::Position>,

    pub(crate) capabilities: Option<lsproto::ClientCapabilities>,
    pub(crate) is_strada_server: bool, // Whether this is a fourslash server test in Strada. !!! Remove once we don't need to diff baselines.

    // Semantic token configuration
    pub(crate) semantic_token_types: Vec<String>,
    pub(crate) semantic_token_modifiers: Vec<String>,
}

impl Drop for FourslashTest {
    fn drop(&mut self) {
        if let Some(close_client) = self.close_client.take() {
            let _ = close_client.close();
        }
    }
}

// The `done` / `reset` closures the harness returns. Go's closures capture the *FourslashTest (and t); the
// Rust ones receive them, so the test can keep using `f` while it holds the closure.
pub type DoneFn = Box<dyn FnOnce(&mut FourslashTest, &T)>;

// fourslash.go:75
#[derive(Clone, Debug)]
pub(crate) struct ScriptInfo {
    pub(crate) file_name: String,
    pub(crate) content: String,
    pub(crate) line_map: Arc<LSPLineMap>,
    pub(crate) version: i32,
}

// fourslash.go:82
#[derive(Clone)]
pub struct TestConverters {
    pub converters: Arc<Converters>,
}

// fourslash.go:86
pub fn new_test_converters(converters: Arc<Converters>) -> TestConverters {
    TestConverters { converters }
}

impl TestConverters {
    // fourslash.go:90
    pub fn position_to_line_and_character(&self, script: &dyn Script, position: TextPos) -> lsproto::Position {
        let (lsp_position, _) = self.converters.to_lsp_position(script, position);
        lsp_position
    }

    // fourslash.go:95
    pub fn line_and_character_to_position<S: Script + Clone>(&self, script: S, position: lsproto::Position) -> TextPos {
        let positions = self.converters.from_lsp_position(script, position, Feature::All);
        assert!(positions.len() == 1, "fourslash script must have exactly one position projection");
        positions[0].position
    }
}

// fourslash.go:101
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TextEditSpan {
    pub(crate) start: i32,
    pub(crate) end: i32,
    pub(crate) length: i32,
}

// fourslash.go:107
pub(crate) fn new_script_info(file_name: &str, content: &str) -> ScriptInfo {
    ScriptInfo { file_name: file_name.to_string(), content: content.to_string(), line_map: lsconv::compute_lsp_line_starts(content), version: 1 }
}

impl ScriptInfo {
    // fourslash.go:116
    pub(crate) fn edit_content(&mut self, change: TextChange) {
        self.content = change.apply_to(&self.content);
        self.line_map = lsconv::compute_lsp_line_starts(&self.content);
        self.version += 1;
    }

    // fourslash.go:136
    pub(crate) fn get_line_content(&self, line: i32) -> String {
        let num_lines = self.line_map.line_starts.len() as i32;
        if line < 0 || line >= num_lines {
            return String::new();
        }
        let start = self.line_map.line_starts[line as usize];
        let end = if line + 1 < num_lines { self.line_map.line_starts[line as usize + 1] } else { self.content.len() as TextPos };

        self.content[start as usize..end as usize].trim_end_matches(['\r', '\n']).to_string()
    }
}

// fourslash.go:122-134: scriptInfo implements lsconv.Script.
impl Script for ScriptInfo {
    fn file_name(&self) -> &str {
        &self.file_name
    }
    fn original_file_name(&self) -> &str {
        &self.file_name
    }
    fn text(&self) -> &str {
        &self.content
    }
    fn span_map(&self) -> Option<&SpanMap> {
        None
    }
    fn original_text(&self) -> &str {
        &self.content
    }
}

// fourslash.go:152
const ROOT_DIR: &str = "/";

// fourslash.go:154
static PARSE_CACHE: LazyLock<Arc<tsrs_project::ParseCache>> =
    LazyLock::new(|| Arc::new(tsrs_project::new_parse_cache(tsrs_project::RefCountCacheOptions { disable_deletion: true })));

// fourslash.go:160
pub fn new_fourslash(t: &T, capabilities: Option<lsproto::ClientCapabilities>, content: &str) -> (FourslashTest, DoneFn) {
    let test_path = t.file().to_string();
    new_fourslash_impl(t, content, Some(FourslashOptions { capabilities, ..Default::default() }), &test_path)
}

// fourslash.go:165
#[derive(Clone, Debug, Default)]
pub struct FourslashOptions {
    pub capabilities: Option<lsproto::ClientCapabilities>,
    pub content_mapper_spawner: Option<Spawner>,
    pub run_external_code: bool,
    // Makes every textDocument/diagnostic request also emit the program and compare
    // the diagnostics before and after emit, e.g. to catch ones added by the emit resolver.
    pub track_flaky_diagnostics: Option<lsproto::DiagnosticFlakeLogLevel>,
}

// fourslash.go:174
pub fn new_fourslash_with_options(t: &T, content: &str, options: Option<FourslashOptions>) -> (FourslashTest, DoneFn) {
    let test_path = t.file().to_string();
    new_fourslash_impl(t, content, options, &test_path)
}

// A file of the test's virtual file system (Go: `map[string]any` holding contents and vfstest.Symlink values).
#[derive(Clone, Debug)]
pub(crate) enum TestFsEntry {
    File(String),
    Symlink(String),
}

// fourslash.go:179
fn new_fourslash_impl(t: &T, content: &str, options: Option<FourslashOptions>, test_path: &str) -> (FourslashTest, DoneFn) {
    let options = options.unwrap_or_default();
    // bundled.Embedded: the libs are always embedded in tsrs.

    let file_name = get_base_file_name_from_test(t) + tspath::EXTENSION_TS;
    let mut testfs: OrderedMap<String, TestFsEntry> = OrderedMap::default();
    let mut script_infos: FxHashMap<String, ScriptInfo> = FxHashMap::default();
    let test_data = parse_test_data(t, content, &file_name);
    for file in &test_data.files {
        let file_path = tspath::get_normalized_absolute_path(&file.file_name, ROOT_DIR);
        // Dynamic files (e.g., untitled:) shouldn't be added to the VFS
        if !tspath::is_dynamic_file_name(&file_path) {
            testfs.insert(file_path.clone(), TestFsEntry::File(file.content.clone()));
        }
        script_infos.insert(file_path.clone(), new_script_info(&file_path, &file.content));
    }

    for (link, target) in &test_data.symlinks {
        let file_path = tspath::get_normalized_absolute_path(link, ROOT_DIR);
        testfs.insert(file_path, TestFsEntry::Symlink(tspath::get_normalized_absolute_path(target, ROOT_DIR)));
    }

    // !!! use default compiler options for inferred project as base
    let mut compiler_options =
        CompilerOptions { skip_default_lib_check: Tristate::True, target: ScriptTarget::LatestStandard, jsx: JsxEmit::Preserve, ..Default::default() };
    let mut harness_options = harnessutil::HarnessOptions { use_case_sensitive_file_names: true, current_directory: ROOT_DIR.to_string(), ..Default::default() };
    harnessutil::set_options_from_test_config(t, &test_data.global_options, &mut compiler_options, &mut harness_options, ROOT_DIR, true /*allowUnknownOptions*/);
    if let Some(command_lines) = test_data.global_options.get("tsc") {
        if !command_lines.is_empty() {
            // tsctests.GetFileMapWithBuild runs `tsc --build` into the test file system; emit is not ported.
            t.fatal("feature not ported: @tsc command lines (tsctests.GetFileMapWithBuild needs emit)");
        }
    }

    harnessutil::skip_unsupported_compiler_options(t, &compiler_options);

    let entries: Vec<(String, tsrs_vfs::vfstest::MapFile)> = testfs
        .iter()
        .map(|(path, entry)| {
            let file = match entry {
                TestFsEntry::File(content) => tsrs_vfs::vfstest::MapFile::from(content.as_str()),
                TestFsEntry::Symlink(target) => tsrs_vfs::vfstest::symlink(target),
            };
            (path.clone(), file)
        })
        .collect();
    let fs_from_map = tsrs_vfs::vfstest::from_map(entries, harness_options.use_case_sensitive_file_names);
    let fs: Arc<dyn FS> = Arc::new(tsrs_vfs::bundled::wrap_fs(fs_from_map));

    if options.content_mapper_spawner.is_some() {
        // serverOpts.Spawn = options.ContentMapperSpawner.Spawn: content mappers are out of scope (docs/LSP.md).
        t.fatal("feature not ported: content mappers (out of scope)");
    }
    let server_opts = tsrs_lsp::ServerOptions {
        in_: tsrs_lsp::to_reader(std::io::empty()),
        out: tsrs_lsp::to_writer(std::io::sink()),
        err: Box::new(std::io::sink()),

        cwd: "/".to_string(),
        fs: Some(fs.clone()),
        default_library_path: tsrs_vfs::bundled::lib_path(),
        typings_location: String::new(),

        parse_cache: Some(PARSE_CACHE.clone()),
        npm_install: None,
        progress_delay: std::time::Duration::ZERO,
        set_parent_process_id: None,
    };

    let script_infos = Arc::new(RwLock::new(script_infos));
    let shared = script_infos.clone();
    let converters = new_test_converters(lsconv::new_converters(lsproto::PositionEncodingKind::UTF8, move |file_name| {
        shared.read().unwrap().get(file_name).map(|s| s.line_map.clone())
    }));

    let user_preferences = lsutil::new_default_user_preferences();
    let shared_user_preferences = Arc::new(Mutex::new(user_preferences.clone()));
    let mut f = FourslashTest {
        client: None,
        close_client: None,
        vfs: fs,
        test_data,
        state_enable_formatting: true,
        report_format_on_type_crash: true,
        user_preferences,
        shared_user_preferences: shared_user_preferences.clone(),
        script_infos,
        converters,
        baselines: OrderedMap::default(),
        open_files: OrderedMap::default(),
        semantic_token_types: default_semantic_token_types(),
        semantic_token_modifiers: default_semantic_token_modifiers(),
        ranges_by_text: OnceLock::new(),
        state_baseline: None,
        current_caret_position: lsproto::Position::default(),
        last_known_marker_name: None,
        active_filename: String::new(),
        selection_end: None,
        capabilities: None,
        is_strada_server: false,
    };
    let (client, close_client) =
        lsptestutil::new_lsp_client(server_opts, Some(Box::new(move |req: &RequestMessage| Some(handle_server_request(&shared_user_preferences, req)))));
    f.client = Some(client);
    f.close_client = Some(close_client);

    // !!! temporary; remove when we have `handleDidChangeConfiguration`/implicit project config support
    // !!! replace with a proper request *after initialize*
    f.client().set_compiler_options_for_inferred_projects(Some(P::new(compiler_options)));
    f.initialize(t, &options);

    if f.test_data.is_state_baselining_enabled() {
        // Single baseline, so initialize project state baseline too
        // (newStateBaseline needs fsbaselineutil.FSDiffer and the project-state printers of statebaseline.go.)
        t.fatal("feature not ported: state baselines (statebaseline.go, fsbaselineutil)");
    } else {
        let files: Vec<Arc<TestFileInfo>> = f.test_data.files.clone();
        for file in &files {
            if file.open {
                f.open_file(t, &file.file_name);
            }
        }
        f.active_filename = f.test_data.files[0].file_name.clone();
    }

    (f, new_done_fn(test_path))
}

// fourslash.go:280: the `done` closure NewFourslash returns.
fn new_done_fn(test_path: &str) -> DoneFn {
    let test_path = test_path.to_string();
    Box::new(move |f: &mut FourslashTest, t: &T| {
        t.helper();
        if let Some(close_client) = f.close_client.take() {
            if let Err(err) = close_client.close() {
                t.error(&format!("goroutine error: {err}"));
            }
        }
        f.verify_baselines(t, &test_path);
    })
}

// handleServerRequest handles requests initiated by the server (e.g., workspace/configuration).
// fourslash.go:291
fn handle_server_request(user_preferences: &Mutex<lsutil::UserPreferences>, req: &RequestMessage) -> ResponseMessage {
    match req.method {
        lsproto::Method::WorkspaceConfiguration => {
            // Return current user preferences for each requested section.
            // The server requests multiple sections (js/ts, typescript, javascript, editor);
            // we return user preferences for "js/ts" and nil for others.
            let prefs = || user_preferences.lock().unwrap().marshal_json_to().unwrap_or_default();
            let params = req.unmarshal_params::<lsproto::ConfigurationParams>();
            let Ok(params) = params else {
                return ResponseMessage { id: req.id.clone(), result: Some(Value::Array(vec![prefs()])), ..Default::default() };
            };
            let mut results = Vec::with_capacity(params.items.len());
            for item in &params.items {
                if item.section.as_deref() == Some("js/ts") {
                    results.push(prefs());
                } else {
                    results.push(Value::Null);
                }
            }
            ResponseMessage { id: req.id.clone(), result: Some(Value::Array(results)), ..Default::default() }
        }

        lsproto::Method::ClientRegisterCapability => {
            // Accept all capability registrations
            ResponseMessage { id: req.id.clone(), result: Some(lsproto::Null.to_json()), ..Default::default() }
        }

        lsproto::Method::ClientUnregisterCapability => {
            // Accept all capability unregistrations
            ResponseMessage { id: req.id.clone(), result: Some(lsproto::Null.to_json()), ..Default::default() }
        }

        _ => {
            // Unknown server request
            ResponseMessage {
                id: req.id.clone(),
                error: Some(tsrs_lsproto::jsonrpc::ResponseError {
                    code: lsproto::ErrorCode::MethodNotFound.0,
                    message: format!("Unknown method: {}", req.method),
                    data: None,
                }),
                ..Default::default()
            }
        }
    }
}

// fourslash.go:346
pub(crate) fn get_base_file_name_from_test(t: &T) -> String {
    let name = t.name();
    let mut name = name.split('/').last().unwrap_or("").to_string();
    name = name.strip_prefix("Test").unwrap_or(&name).to_string();
    name = stringutil::lower_first_char(&name);

    // Special case: TypeScript has "callHierarchyFunctionAmbiguity.N" with periods
    match name.as_str() {
        "callHierarchyFunctionAmbiguity1" => name = "callHierarchyFunctionAmbiguity.1".to_string(),
        "callHierarchyFunctionAmbiguity2" => name = "callHierarchyFunctionAmbiguity.2".to_string(),
        "callHierarchyFunctionAmbiguity3" => name = "callHierarchyFunctionAmbiguity.3".to_string(),
        "callHierarchyFunctionAmbiguity4" => name = "callHierarchyFunctionAmbiguity.4".to_string(),
        "callHierarchyFunctionAmbiguity5" => name = "callHierarchyFunctionAmbiguity.5".to_string(),
        _ => {}
    }

    name
}

// fourslash.go:369
const SHOW_CODE_LENS_LOCATIONS_COMMAND_NAME: &str = "typescript.showCodeLensLocations";

impl FourslashTest {
    pub(crate) fn client(&self) -> &Arc<LSPClient> {
        self.client.as_ref().expect("fourslash client is closed")
    }

    // fourslash.go:371
    fn initialize(&mut self, t: &T, options: &FourslashOptions) {
        let mut initialization_options = lsproto::InitializationOptions {
            code_lens_show_locations_command_name: Some(SHOW_CODE_LENS_LOCATIONS_COMMAND_NAME.to_string()),
            track_flaky_diagnostics: options.track_flaky_diagnostics,
            ..Default::default()
        };
        if options.run_external_code {
            initialization_options.run_external_code = Some(true);
        }
        let mut params = lsproto::InitializeParams {
            locale: Some("en-US".to_string()),
            initialization_options: Some(lsproto::InitializationOptionsOrNull { initialization_options: Some(initialization_options) }),
            ..Default::default()
        };
        params.capabilities = get_capabilities_with_defaults(options.capabilities.as_ref());
        self.capabilities = Some(params.capabilities.clone());
        let (resp, _) = self.client().send_request(lsproto::INITIALIZE_INFO, params);
        if let Some(err) = &resp.error {
            t.fatal(&format!("Initialize request returned error: {}", err.string()));
        }
        self.client().send_notification(lsproto::INITIALIZED_INFO, lsproto::InitializedParams::default());

        // Wait for the initial configuration exchange to complete
        // The server will send workspace/configuration as part of handleInitialized
        self.client().server.init_complete().wait();
    }
}

// fourslash.go:401
fn default_semantic_token_types() -> Vec<String> {
    [
        lsproto::SemanticTokenType::Namespace,
        lsproto::SemanticTokenType::Class,
        lsproto::SemanticTokenType::Enum,
        lsproto::SemanticTokenType::Interface,
        lsproto::SemanticTokenType::Struct,
        lsproto::SemanticTokenType::TypeParameter,
        lsproto::SemanticTokenType::Type,
        lsproto::SemanticTokenType::Parameter,
        lsproto::SemanticTokenType::Variable,
        lsproto::SemanticTokenType::Property,
        lsproto::SemanticTokenType::EnumMember,
        lsproto::SemanticTokenType::Decorator,
        lsproto::SemanticTokenType::Event,
        lsproto::SemanticTokenType::Function,
        lsproto::SemanticTokenType::Method,
        lsproto::SemanticTokenType::Macro,
        lsproto::SemanticTokenType::Label,
        lsproto::SemanticTokenType::Comment,
        lsproto::SemanticTokenType::String,
        lsproto::SemanticTokenType::Keyword,
        lsproto::SemanticTokenType::Number,
        lsproto::SemanticTokenType::Regexp,
        lsproto::SemanticTokenType::Operator,
    ]
    .iter()
    .map(|k| k.0.to_string())
    .collect()
}

// fourslash.go:429
fn default_semantic_token_modifiers() -> Vec<String> {
    let mut v: Vec<String> = [
        lsproto::SemanticTokenModifier::Declaration,
        lsproto::SemanticTokenModifier::Definition,
        lsproto::SemanticTokenModifier::Readonly,
        lsproto::SemanticTokenModifier::Static,
        lsproto::SemanticTokenModifier::Deprecated,
        lsproto::SemanticTokenModifier::Abstract,
        lsproto::SemanticTokenModifier::Async,
        lsproto::SemanticTokenModifier::Modification,
        lsproto::SemanticTokenModifier::Documentation,
        lsproto::SemanticTokenModifier::DefaultLibrary,
    ]
    .iter()
    .map(|k| k.0.to_string())
    .collect();
    v.push("local".to_string());
    v
}

// If modifying the defaults, update GetDefaultCapabilities too.
// fourslash.go:446
fn ptr_true() -> Option<bool> {
    Some(true)
}

fn ptr_false() -> Option<bool> {
    Some(false)
}

static DEFAULT_COMPLETION_CAPABILITIES: LazyLock<lsproto::CompletionClientCapabilities> = LazyLock::new(|| lsproto::CompletionClientCapabilities {
    completion_item: Some(lsproto::ClientCompletionItemOptions {
        snippet_support: ptr_false(),
        commit_characters_support: ptr_true(),
        preselect_support: ptr_true(),
        label_details_support: ptr_false(),
        insert_replace_support: ptr_true(),
        documentation_format: Some(vec![lsproto::MarkupKind::Markdown, lsproto::MarkupKind::PlainText]),
        ..Default::default()
    }),
    completion_list: Some(lsproto::CompletionListCapabilities {
        item_defaults: Some(vec!["commitCharacters".to_string(), "editRange".to_string()]),
        ..Default::default()
    }),
    ..Default::default()
});

static DEFAULT_DEFINITION_CAPABILITIES: LazyLock<lsproto::DefinitionClientCapabilities> =
    LazyLock::new(|| lsproto::DefinitionClientCapabilities { link_support: ptr_true(), ..Default::default() });

static DEFAULT_TYPE_DEFINITION_CAPABILITIES: LazyLock<lsproto::TypeDefinitionClientCapabilities> =
    LazyLock::new(|| lsproto::TypeDefinitionClientCapabilities { link_support: ptr_true(), ..Default::default() });

static DEFAULT_IMPLEMENTATION_CAPABILITIES: LazyLock<lsproto::ImplementationClientCapabilities> =
    LazyLock::new(|| lsproto::ImplementationClientCapabilities { link_support: ptr_true(), ..Default::default() });

static DEFAULT_HOVER_CAPABILITIES: LazyLock<lsproto::HoverClientCapabilities> = LazyLock::new(|| lsproto::HoverClientCapabilities {
    content_format: Some(vec![lsproto::MarkupKind::Markdown, lsproto::MarkupKind::PlainText]),
    ..Default::default()
});

static DEFAULT_EXPERIMENTAL_CAPABILITIES: LazyLock<lsproto::ExperimentalClientCapabilities> =
    LazyLock::new(|| lsproto::ExperimentalClientCapabilities { hover_verbosity_level: ptr_true(), ..Default::default() });

static DEFAULT_SIGNATURE_HELP_CAPABILITIES: LazyLock<lsproto::SignatureHelpClientCapabilities> =
    LazyLock::new(|| lsproto::SignatureHelpClientCapabilities {
        signature_information: Some(lsproto::ClientSignatureInformationOptions {
            documentation_format: Some(vec![lsproto::MarkupKind::Markdown, lsproto::MarkupKind::PlainText]),
            parameter_information: Some(lsproto::ClientSignatureParameterInformationOptions { label_offset_support: ptr_true() }),
            active_parameter_support: ptr_true(),
            ..Default::default()
        }),
        context_support: ptr_true(),
        ..Default::default()
    });

static DEFAULT_DOCUMENT_SYMBOL_CAPABILITIES: LazyLock<lsproto::DocumentSymbolClientCapabilities> =
    LazyLock::new(|| lsproto::DocumentSymbolClientCapabilities { hierarchical_document_symbol_support: ptr_true(), ..Default::default() });

static DEFAULT_FOLDING_RANGE_CAPABILITIES: LazyLock<lsproto::FoldingRangeClientCapabilities> =
    LazyLock::new(|| lsproto::FoldingRangeClientCapabilities {
        range_limit: Some(5000),
        // LineFoldingOnly: ptrTrue,
        folding_range_kind: Some(lsproto::ClientFoldingRangeKindOptions {
            value_set: Some(vec![lsproto::FoldingRangeKind::Comment, lsproto::FoldingRangeKind::Imports, lsproto::FoldingRangeKind::Region]),
        }),
        folding_range: Some(lsproto::ClientFoldingRangeOptions {
            collapsed_text: ptr_true(), // Unused by our testing, but set to exercise the code.
        }),
        ..Default::default()
    });

static DEFAULT_DIAGNOSTIC_CAPABILITIES: LazyLock<lsproto::DiagnosticClientCapabilities> =
    LazyLock::new(|| lsproto::DiagnosticClientCapabilities {
        related_information: ptr_true(),
        tag_support: Some(lsproto::ClientDiagnosticsTagOptions {
            value_set: vec![lsproto::DiagnosticTag::Unnecessary, lsproto::DiagnosticTag::Deprecated],
        }),
        ..Default::default()
    });

static DEFAULT_PUBLISH_DIAGNOSTIC_CAPABILITIES: LazyLock<lsproto::PublishDiagnosticsClientCapabilities> =
    LazyLock::new(|| lsproto::PublishDiagnosticsClientCapabilities {
        related_information: ptr_true(),
        tag_support: Some(lsproto::ClientDiagnosticsTagOptions {
            value_set: vec![lsproto::DiagnosticTag::Unnecessary, lsproto::DiagnosticTag::Deprecated],
        }),
        ..Default::default()
    });

static DEFAULT_WORKSPACE_EDIT_CAPABILITIES: LazyLock<lsproto::WorkspaceEditClientCapabilities> =
    LazyLock::new(|| lsproto::WorkspaceEditClientCapabilities {
        document_changes: ptr_true(),
        resource_operations: Some(vec![lsproto::ResourceOperationKind::Rename]),
        ..Default::default()
    });

// fourslash.go:530
pub fn get_default_capabilities() -> Option<lsproto::ClientCapabilities> {
    Some(lsproto::ClientCapabilities {
        general: Some(lsproto::GeneralClientCapabilities {
            position_encodings: Some(vec![lsproto::PositionEncodingKind::UTF8]),
            ..Default::default()
        }),
        experimental: Some(lsproto::ExperimentalClientCapabilities { hover_verbosity_level: ptr_true(), ..Default::default() }),
        text_document: Some(lsproto::TextDocumentClientCapabilities {
            completion: Some(lsproto::CompletionClientCapabilities {
                completion_item: Some(lsproto::ClientCompletionItemOptions {
                    snippet_support: ptr_false(),
                    commit_characters_support: ptr_true(),
                    preselect_support: ptr_true(),
                    label_details_support: ptr_false(),
                    insert_replace_support: ptr_true(),
                    documentation_format: Some(vec![lsproto::MarkupKind::Markdown, lsproto::MarkupKind::PlainText]),
                    ..Default::default()
                }),
                completion_list: Some(lsproto::CompletionListCapabilities {
                    item_defaults: Some(vec!["commitCharacters".to_string(), "editRange".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            diagnostic: Some(lsproto::DiagnosticClientCapabilities {
                related_information: ptr_true(),
                tag_support: Some(lsproto::ClientDiagnosticsTagOptions {
                    value_set: vec![lsproto::DiagnosticTag::Unnecessary, lsproto::DiagnosticTag::Deprecated],
                }),
                ..Default::default()
            }),
            publish_diagnostics: Some(lsproto::PublishDiagnosticsClientCapabilities {
                related_information: ptr_true(),
                tag_support: Some(lsproto::ClientDiagnosticsTagOptions {
                    value_set: vec![lsproto::DiagnosticTag::Unnecessary, lsproto::DiagnosticTag::Deprecated],
                }),
                ..Default::default()
            }),
            definition: Some(lsproto::DefinitionClientCapabilities { link_support: ptr_true(), ..Default::default() }),
            type_definition: Some(lsproto::TypeDefinitionClientCapabilities { link_support: ptr_true(), ..Default::default() }),
            implementation: Some(lsproto::ImplementationClientCapabilities { link_support: ptr_true(), ..Default::default() }),
            hover: Some(lsproto::HoverClientCapabilities {
                content_format: Some(vec![lsproto::MarkupKind::Markdown, lsproto::MarkupKind::PlainText]),
                ..Default::default()
            }),
            signature_help: Some(lsproto::SignatureHelpClientCapabilities {
                signature_information: Some(lsproto::ClientSignatureInformationOptions {
                    documentation_format: Some(vec![lsproto::MarkupKind::Markdown, lsproto::MarkupKind::PlainText]),
                    parameter_information: Some(lsproto::ClientSignatureParameterInformationOptions { label_offset_support: ptr_true() }),
                    active_parameter_support: ptr_true(),
                    ..Default::default()
                }),
                context_support: ptr_true(),
                ..Default::default()
            }),
            document_symbol: Some(lsproto::DocumentSymbolClientCapabilities { hierarchical_document_symbol_support: ptr_true(), ..Default::default() }),
            folding_range: Some(lsproto::FoldingRangeClientCapabilities {
                range_limit: Some(5000),
                folding_range_kind: Some(lsproto::ClientFoldingRangeKindOptions {
                    value_set: Some(vec![lsproto::FoldingRangeKind::Comment, lsproto::FoldingRangeKind::Imports, lsproto::FoldingRangeKind::Region]),
                }),
                folding_range: Some(lsproto::ClientFoldingRangeOptions { collapsed_text: ptr_true() }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        workspace: Some(lsproto::WorkspaceClientCapabilities {
            configuration: ptr_true(),
            file_operations: Some(lsproto::FileOperationClientCapabilities { will_rename: ptr_true(), ..Default::default() }),
            workspace_edit: Some(lsproto::WorkspaceEditClientCapabilities {
                document_changes: ptr_true(),
                resource_operations: Some(vec![lsproto::ResourceOperationKind::Rename]),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    })
}

// fourslash.go:624
#[derive(Clone, Debug, Default)]
pub struct ClientCapabilitiesOptions {
    pub completion_item: Option<lsproto::ClientCompletionItemOptions>,
}

// fourslash.go:628
pub fn get_default_capabilities_with_options(options: Option<ClientCapabilitiesOptions>) -> Option<lsproto::ClientCapabilities> {
    let mut capabilities = get_default_capabilities();
    let Some(options) = options else {
        return capabilities;
    };
    if let Some(completion_item_options) = &options.completion_item {
        let target = capabilities.as_mut().unwrap().text_document.as_mut().unwrap().completion.as_mut().unwrap().completion_item.as_mut().unwrap();
        if completion_item_options.snippet_support.is_some() {
            target.snippet_support = completion_item_options.snippet_support;
        }
        if completion_item_options.commit_characters_support.is_some() {
            target.commit_characters_support = completion_item_options.commit_characters_support;
        }
        if completion_item_options.documentation_format.is_some() {
            target.documentation_format = completion_item_options.documentation_format.clone();
        }
        if completion_item_options.deprecated_support.is_some() {
            target.deprecated_support = completion_item_options.deprecated_support;
        }
        if completion_item_options.preselect_support.is_some() {
            target.preselect_support = completion_item_options.preselect_support;
        }
        if completion_item_options.tag_support.is_some() {
            target.tag_support = completion_item_options.tag_support.clone();
        }
        if completion_item_options.insert_replace_support.is_some() {
            target.insert_replace_support = completion_item_options.insert_replace_support;
        }
        if completion_item_options.resolve_support.is_some() {
            target.resolve_support = completion_item_options.resolve_support.clone();
        }
        if completion_item_options.insert_text_mode_support.is_some() {
            target.insert_text_mode_support = completion_item_options.insert_text_mode_support.clone();
        }
        if completion_item_options.label_details_support.is_some() {
            target.label_details_support = completion_item_options.label_details_support;
        }
    }
    capabilities
}

// fourslash.go:670
pub(crate) fn get_capabilities_with_defaults(capabilities: Option<&lsproto::ClientCapabilities>) -> lsproto::ClientCapabilities {
    let mut capabilities_with_defaults = lsproto::ClientCapabilities::default();
    if let Some(capabilities) = capabilities {
        capabilities_with_defaults = capabilities.clone();
    }
    capabilities_with_defaults.general =
        Some(lsproto::GeneralClientCapabilities { position_encodings: Some(vec![lsproto::PositionEncodingKind::UTF8]), ..Default::default() });
    if capabilities_with_defaults.experimental.is_none() {
        capabilities_with_defaults.experimental = Some(DEFAULT_EXPERIMENTAL_CAPABILITIES.clone());
    }
    let text_document = capabilities_with_defaults.text_document.get_or_insert_with(Default::default);
    if text_document.completion.is_none() {
        text_document.completion = Some(DEFAULT_COMPLETION_CAPABILITIES.clone());
    }
    if text_document.diagnostic.is_none() {
        text_document.diagnostic = Some(DEFAULT_DIAGNOSTIC_CAPABILITIES.clone());
    }
    if text_document.publish_diagnostics.is_none() {
        text_document.publish_diagnostics = Some(DEFAULT_PUBLISH_DIAGNOSTIC_CAPABILITIES.clone());
    }
    if text_document.semantic_tokens.is_none() {
        text_document.semantic_tokens = Some(lsproto::SemanticTokensClientCapabilities {
            requests: lsproto::ClientSemanticTokensRequestOptions {
                full: Some(lsproto::BooleanOrClientSemanticTokensRequestFullDelta { boolean: ptr_true(), ..Default::default() }),
                ..Default::default()
            },
            token_types: default_semantic_token_types(),
            token_modifiers: default_semantic_token_modifiers(),
            formats: vec![lsproto::TokenFormat::Relative],
            ..Default::default()
        });
    }
    let workspace = capabilities_with_defaults.workspace.get_or_insert_with(Default::default);
    if workspace.file_operations.is_none() {
        workspace.file_operations = Some(lsproto::FileOperationClientCapabilities { will_rename: ptr_true(), ..Default::default() });
    }
    if workspace.workspace_edit.is_none() {
        workspace.workspace_edit = Some(DEFAULT_WORKSPACE_EDIT_CAPABILITIES.clone());
    }
    if workspace.configuration.is_none() {
        workspace.configuration = ptr_true();
    }
    let text_document = capabilities_with_defaults.text_document.as_mut().unwrap();
    if text_document.definition.is_none() {
        text_document.definition = Some(DEFAULT_DEFINITION_CAPABILITIES.clone());
    }
    if text_document.type_definition.is_none() {
        text_document.type_definition = Some(DEFAULT_TYPE_DEFINITION_CAPABILITIES.clone());
    }
    if text_document.implementation.is_none() {
        text_document.implementation = Some(DEFAULT_IMPLEMENTATION_CAPABILITIES.clone());
    }
    if text_document.hover.is_none() {
        text_document.hover = Some(DEFAULT_HOVER_CAPABILITIES.clone());
    }
    if text_document.signature_help.is_none() {
        text_document.signature_help = Some(DEFAULT_SIGNATURE_HELP_CAPABILITIES.clone());
    }
    if text_document.document_symbol.is_none() {
        text_document.document_symbol = Some(DEFAULT_DOCUMENT_SYMBOL_CAPABILITIES.clone());
    }
    if text_document.folding_range.is_none() {
        text_document.folding_range = Some(DEFAULT_FOLDING_RANGE_CAPABILITIES.clone());
    }
    capabilities_with_defaults
}

// fourslash.go:1090
pub(crate) fn get_language_kind(filename: &str) -> lsproto::LanguageKind {
    if tspath::file_extension_is_one_of(
        filename,
        &[tspath::EXTENSION_TS, tspath::EXTENSION_MTS, tspath::EXTENSION_CTS, tspath::EXTENSION_DMTS, tspath::EXTENSION_DCTS, tspath::EXTENSION_DTS],
    ) {
        return lsproto::LanguageKind::TypeScript;
    }
    if tspath::file_extension_is_one_of(filename, &[tspath::EXTENSION_JS, tspath::EXTENSION_MJS, tspath::EXTENSION_CJS]) {
        return lsproto::LanguageKind::JavaScript;
    }
    if tspath::file_extension_is(filename, tspath::EXTENSION_JSX) {
        return lsproto::LanguageKind::JavaScriptReact;
    }
    if tspath::file_extension_is(filename, tspath::EXTENSION_TSX) {
        return lsproto::LanguageKind::TypeScriptReact;
    }
    if tspath::file_extension_is(filename, tspath::EXTENSION_JSON) {
        return lsproto::LanguageKind::JSON;
    }
    lsproto::LanguageKind::TypeScript // !!! should we error in this case?
}

// fourslash.go:1115
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionsExpectedList {
    pub is_incomplete: bool,
    pub item_defaults: Option<CompletionsExpectedItemDefaults>,
    pub items: Option<CompletionsExpectedItems>,
    pub user_preferences: Option<lsutil::UserPreferences>,
}

// fourslash.go:1122 (`type Ignored = struct{}`): `Any::Ignored` in Rust.

// fourslash.go:1127
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditRange {
    pub insert: Option<Arc<RangeMarker>>,
    pub replace: Option<Arc<RangeMarker>>,
}

// fourslash.go:1132
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionsExpectedItemDefaults {
    pub commit_characters: Option<Vec<String>>,
    pub edit_range: Any, // *EditRange | Ignored
}

// fourslash.go:1140 (items are *lsproto.CompletionItem | string)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionsExpectedItems {
    pub includes: Vec<Any>,
    pub excludes: Vec<String>,
    pub exact: Vec<Any>,
    pub unsorted: Vec<Any>,
}

// fourslash.go:1147
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionsExpectedCodeAction {
    pub name: String,
    pub source: String,
    pub description: String,
    pub new_file_content: String,
}

// fourslash.go:1154
pub struct VerifyCompletionsResult {
    pub and_apply_code_action: Box<dyn FnOnce(&mut FourslashTest, &T, Option<CompletionsExpectedCodeAction>)>,
    pub and_has_no_code_action: Box<dyn FnOnce(&mut FourslashTest, &T, Option<CompletionsExpectedCodeAction>)>,
}

// fourslash.go:1675
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VerifyCodeFixOptions {
    pub description: String,
    pub new_file_content: String,
    pub new_range_content: String,
    pub index: i32,
    pub apply_changes: bool,
    pub user_preferences: Option<lsutil::UserPreferences>,
}

// fourslash.go:1685
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VerifyCodeFixAllOptions {
    pub fix_id: String,
    pub new_file_content: String,
}

// fourslash.go:2212
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplyCodeActionFromCompletionOptions {
    pub name: String,
    pub source: String,
    pub auto_import_fix: Option<lsproto::AutoImportFix>,
    pub description: String,
    pub new_file_content: Option<String>,
    pub new_range_content: Option<String>,
    pub user_preferences: Option<lsutil::UserPreferences>,
}

// fourslash.go:2900
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FoldingRangeLineExpected {
    pub start_line: u32,
    pub end_line: u32,
}

// fourslash.go:4329
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VerifySignatureHelpOptions {
    // Text is the full signature text (e.g., "fn(x: string, y: number): void")
    pub text: String,
    // DocComment is the documentation comment for the signature
    pub doc_comment: String,
    // ParameterCount is the expected number of parameters
    pub parameter_count: i32,
    // ParameterName is the expected name of the active parameter
    pub parameter_name: String,
    // ParameterSpan is the expected label of the active parameter (e.g., "x: string")
    pub parameter_span: String,
    // ParameterDocComment is the documentation for the active parameter
    pub parameter_doc_comment: String,
    // OverloadsCount is the expected number of overloads (signatures)
    pub overloads_count: i32,
    // OverrideSelectedItemIndex overrides which signature to check (default: ActiveSignature)
    pub override_selected_item_index: i32,
    // IsVariadic indicates if the signature has a rest parameter
    pub is_variadic: bool,
    // IsVariadicSet is true when IsVariadic was explicitly set (to distinguish from default false)
    pub is_variadic_set: bool,
}

// fourslash.go:4589
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SignatureHelpCase {
    pub context: Option<lsproto::SignatureHelpContext>,
    pub marker_input: Any,
    pub expected: Option<lsproto::SignatureHelp>,
}

// Go compares expected.AdditionalTextEdits against these two pointers by identity. Rust values have no identity,
// so they are sentinel lists of one edit whose text cannot occur in a real expectation (see is_any_text_edits).
// fourslash.go:5694
pub static ANY_TEXT_EDITS: LazyLock<Option<Vec<lsproto::TextEdit>>> =
    LazyLock::new(|| Some(vec![lsproto::TextEdit { new_text: "\u{0}fourslash.AnyTextEdits\u{0}".to_string(), ..Default::default() }]));
pub static NO_TEXT_EDITS: LazyLock<Option<Vec<lsproto::TextEdit>>> =
    LazyLock::new(|| Some(vec![lsproto::TextEdit { new_text: "\u{0}fourslash.NoTextEdits\u{0}".to_string(), ..Default::default() }]));

pub(crate) fn is_any_text_edits(edits: &Option<Vec<lsproto::TextEdit>>) -> bool {
    *edits == *ANY_TEXT_EDITS
}

pub(crate) fn is_no_text_edits(edits: &Option<Vec<lsproto::TextEdit>>) -> bool {
    *edits == *NO_TEXT_EDITS
}

// fourslash.go:5718
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VerifyWorkspaceSymbolCase {
    pub pattern: String,
    pub includes: Option<Vec<lsproto::SymbolInformation>>,
    pub exact: Option<Vec<lsproto::SymbolInformation>>,
    pub preferences: Option<lsutil::UserPreferences>,
}

impl FourslashTest {
    // The single place where a language-service feature that is not ported yet surfaces: the methods that need it
    // fail the test with "feature not ported: <feature> (<Method>)".
    pub fn server_unavailable(t: &T, reason: &str) -> ! {
        t.fatal(reason)
    }

    // fourslash.go:803
    pub fn get_options(&self) -> lsutil::UserPreferences {
        self.user_preferences.clone()
    }

    // fourslash.go:807
    pub fn configure(&mut self, t: &T, config: lsutil::UserPreferences) {
        // We send 'js/ts' by default because that is what we expect the primary config to be in vscode and VS (one
        // set of preferences for both languages). This should be fine in fourslash since tests that need
        // multiple options usually send reconfiguration commands for each `verify` anyways
        self.user_preferences = config.clone();
        *self.shared_user_preferences.lock().unwrap() = config.clone();
        let mut settings = OrderedMap::default();
        settings.insert("js/ts".to_string(), config.marshal_json_to().unwrap_or_default());
        self.send_notification(t, lsproto::WORKSPACE_DID_CHANGE_CONFIGURATION_INFO, lsproto::DidChangeConfigurationParams { settings: Value::Object(settings) });
    }

    // fourslash.go:819
    pub fn configure_with_reset(&mut self, t: &T, config: lsutil::UserPreferences) -> DoneFn {
        let original_config = self.user_preferences.clone();
        self.configure(t, config);
        Box::new(move |f: &mut FourslashTest, t: &T| f.configure(t, original_config))
    }

    // fourslash.go:827
    pub fn go_to_marker_or_range(&mut self, t: &T, marker_or_range: MarkerOrRange) {
        self.go_to_marker_impl(t, &marker_or_range);
    }

    // fourslash.go:831
    pub fn go_to_marker(&mut self, t: &T, marker_name: &str) {
        let Some(marker) = self.test_data.marker_positions.get(marker_name).cloned() else {
            t.fatal(&format!("Marker '{marker_name}' not found"));
        };
        self.go_to_marker_impl(t, &MarkerOrRange::Marker(marker));
    }

    // fourslash.go:839 (goToMarker)
    fn go_to_marker_impl(&mut self, t: &T, marker_or_range: &MarkerOrRange) {
        self.ensure_active_file(t, &marker_or_range.file_name());
        self.go_to_position_impl(t, marker_or_range.ls_pos());
        self.last_known_marker_name = marker_or_range.get_name();
    }

    // fourslash.go:845
    pub fn go_to_eof(&mut self, t: &T) {
        let script = self.get_script_info(&self.active_filename.clone());
        let pos = script.content.len() as TextPos;
        let lsp_pos = self.converters.position_to_line_and_character(&script, pos);
        self.go_to_position_impl(t, lsp_pos);
    }

    // fourslash.go:852
    pub fn go_to_bof(&mut self, t: &T) {
        self.go_to_position_impl(t, lsproto::Position { line: 0, character: 0 });
    }

    // fourslash.go:856
    pub fn go_to_position(&mut self, t: &T, position: i32) {
        let script = self.get_script_info(&self.active_filename.clone());
        let lsp_pos = self.converters.position_to_line_and_character(&script, position);
        self.go_to_position_impl(t, lsp_pos);
    }

    // fourslash.go:862 (goToPosition)
    fn go_to_position_impl(&mut self, t: &T, position: lsproto::Position) {
        self.current_caret_position = position;
        self.selection_end = None;
    }

    // fourslash.go:867
    pub fn go_to_each_marker(&mut self, t: &T, marker_names: &[&str], mut action: impl FnMut(&mut FourslashTest, Arc<Marker>, i32)) {
        let mut markers: Vec<Arc<Marker>> = Vec::new();
        // (Go tests the length of the still-empty `markers`, so the names are never used.)
        if markers.is_empty() {
            markers = self.markers();
        } else {
            markers = Vec::with_capacity(marker_names.len());
            for name in marker_names {
                let Some(marker) = self.test_data.marker_positions.get(*name).cloned() else {
                    t.fatal(&format!("Marker '{name}' not found"));
                };
                markers.push(marker);
            }
        }
        for (i, marker) in markers.into_iter().enumerate() {
            self.go_to_marker_impl(t, &MarkerOrRange::Marker(marker.clone()));
            action(self, marker, i as i32);
        }
    }

    // fourslash.go:887
    pub fn go_to_each_range(&mut self, t: &T, mut action: impl FnMut(&mut FourslashTest, &T, Arc<RangeMarker>)) {
        let ranges = self.ranges();
        for range_marker in ranges {
            self.go_to_position_impl(t, range_marker.ls_range.start);
            action(self, t, range_marker);
        }
    }

    // fourslash.go:895
    pub fn go_to_range_start(&mut self, t: &T, range_marker: Arc<RangeMarker>) {
        self.open_file(t, &range_marker.file_name());
        self.go_to_position_impl(t, range_marker.ls_range.start);
    }

    // fourslash.go:900
    pub fn go_to_select(&mut self, t: &T, start_marker_name: &str, end_marker_name: &str) {
        let Some(start_marker) = self.test_data.marker_positions.get(start_marker_name).cloned() else {
            t.fatal(&format!("Start marker '{start_marker_name}' not found"));
        };
        let Some(end_marker) = self.test_data.marker_positions.get(end_marker_name).cloned() else {
            t.fatal(&format!("End marker '{end_marker_name}' not found"));
        };
        if start_marker.file_name() != end_marker.file_name() {
            t.fatal(&format!("Markers '{start_marker_name}' and '{end_marker_name}' are in different files"));
        }
        self.ensure_active_file(t, &start_marker.file_name());
        self.go_to_position_impl(t, start_marker.ls_position);
        self.selection_end = Some(end_marker.ls_position);
    }

    // fourslash.go:917
    pub fn go_to_select_range(&mut self, t: &T, range_marker: Arc<RangeMarker>) {
        self.go_to_range_start(t, range_marker.clone());
        self.selection_end = Some(range_marker.ls_range.end);
    }

    // fourslash.go:922
    pub fn go_to_file(&mut self, t: &T, filename: &str) {
        let filename = tspath::get_normalized_absolute_path(filename, ROOT_DIR);
        self.open_file(t, &filename);
    }

    // fourslash.go:927
    pub fn go_to_file_number(&mut self, t: &T, index: i32) {
        if index < 0 || index as usize >= self.test_data.files.len() {
            t.fatal(&format!("File index {} out of range (0-{})", index, self.test_data.files.len() as i32 - 1));
        }
        let filename = self.test_data.files[index as usize].file_name.clone();
        self.open_file(t, &filename);
    }

    // fourslash.go:935
    pub fn markers(&self) -> Vec<Arc<Marker>> {
        self.test_data.markers.clone()
    }

    // fourslash.go:939
    pub fn marker_names(&self) -> Vec<String> {
        self.test_data.markers.iter().filter_map(|marker| marker.name.clone()).collect()
    }

    // fourslash.go:948 (Go returns nil for an unknown name; the tests only ask for existing markers)
    pub fn marker_by_name(&self, t: &T, name: &str) -> Arc<Marker> {
        match self.test_data.marker_positions.get(name) {
            Some(m) => m.clone(),
            None => t.fatal(&format!("Marker '{name}' not found")),
        }
    }

    // fourslash.go:952
    pub fn ranges(&self) -> Vec<Arc<RangeMarker>> {
        self.test_data.ranges.clone()
    }

    // fourslash.go:956
    pub(crate) fn get_ranges_in_file(&self, file_name: &str) -> Vec<Arc<RangeMarker>> {
        let mut ranges_in_file = Vec::new();
        for range_marker in &self.test_data.ranges {
            if range_marker.file_name == file_name {
                ranges_in_file.push(range_marker.clone());
            }
        }
        ranges_in_file
    }

    // fourslash.go:966
    fn ensure_active_file(&mut self, t: &T, filename: &str) {
        if self.active_filename != filename {
            if !self.open_files.contains_key(filename) {
                self.open_file(t, filename);
            } else {
                self.active_filename = filename.to_string();
            }
        }
    }

    // fourslash.go:976
    pub fn close_file_of_marker(&mut self, t: &T, marker_name: &str) {
        let Some(marker) = self.test_data.marker_positions.get(marker_name).cloned() else {
            t.fatal(&format!("Marker '{marker_name}' not found"));
        };
        if self.active_filename == marker.file_name() {
            self.active_filename = String::new();
        }
        if let Some(index) = self.test_data.files.iter().position(|f| f.file_name == marker.file_name()) {
            let test_file = self.test_data.files[index].clone();
            self.script_infos.write().unwrap().insert(test_file.file_name.clone(), new_script_info(&test_file.file_name, &test_file.content));
        } else {
            self.script_infos.write().unwrap().remove(&marker.file_name());
        }
        self.send_notification(
            t,
            lsproto::TEXT_DOCUMENT_DID_CLOSE_INFO,
            lsproto::DidCloseTextDocumentParams { text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&marker.file_name()) } },
        );
    }

    // fourslash.go:997
    fn open_file(&mut self, t: &T, filename: &str) {
        let script = match self.try_get_script_info(filename) {
            Some(script) => script,
            None => match self.vfs.read_file(filename) {
                Some(content) => {
                    let script = new_script_info(filename, &content);
                    self.script_infos.write().unwrap().insert(filename.to_string(), script.clone());
                    script
                }
                None => t.fatal(&format!("File {filename} not found in test data")),
            },
        };
        self.active_filename = filename.to_string();
        self.send_notification(
            t,
            lsproto::TEXT_DOCUMENT_DID_OPEN_INFO,
            lsproto::DidOpenTextDocumentParams {
                text_document: lsproto::TextDocumentItem {
                    uri: lsconv::file_name_to_document_uri(filename),
                    language_id: get_language_kind(filename),
                    text: script.content.clone(),
                    ..Default::default()
                },
            },
        );
        self.baseline_projects_after_notification(t, filename);
    }

    // fourslash.go:1018
    pub fn format_document(&mut self, t: &T, filename: &str) {
        let filename = if filename.is_empty() { self.active_filename.clone() } else { filename.to_string() };
        let result = self.send_request(
            t,
            lsproto::TEXT_DOCUMENT_FORMATTING_INFO,
            lsproto::DocumentFormattingParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&filename) },
                options: self.user_preferences.format_code_settings.to_ls_format_options(),
                ..Default::default()
            },
        );
        let Some(text_edits) = result.text_edits else {
            return;
        };
        self.apply_text_edits(t, text_edits);
    }

    // fourslash.go:1034
    pub fn format_selection(&mut self, t: &T, start_marker_name: &str, end_marker_name: &str) {
        t.helper();
        let Some(start_marker) = self.test_data.marker_positions.get(start_marker_name).cloned() else {
            t.fatal(&format!("Marker '{start_marker_name}' not found"));
        };
        let Some(end_marker) = self.test_data.marker_positions.get(end_marker_name).cloned() else {
            t.fatal(&format!("Marker '{end_marker_name}' not found"));
        };
        if start_marker.file_name() != end_marker.file_name() {
            t.fatal(&format!("Markers '{start_marker_name}' and '{end_marker_name}' are in different files"));
        }
        let filename = start_marker.file_name();
        let result = self.send_request(
            t,
            lsproto::TEXT_DOCUMENT_RANGE_FORMATTING_INFO,
            lsproto::DocumentRangeFormattingParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&filename) },
                range: lsproto::Range { start: start_marker.ls_position, end: end_marker.ls_position },
                options: self.user_preferences.format_code_settings.to_ls_format_options(),
                ..Default::default()
            },
        );
        let Some(text_edits) = result.text_edits else {
            return;
        };
        self.apply_text_edits(t, text_edits);
    }

    // fourslash.go:1064
    pub fn verify_current_file_content(&mut self, t: &T, expected_content: &str) {
        let actual_content = self.get_script_info(&self.active_filename.clone()).content;
        crate::go::assert::equal(t, actual_content.as_str(), expected_content, "");
    }

    // fourslash.go:1070
    pub fn verify_current_line_content(&mut self, t: &T, expected_content: &str) {
        let actual_content = self.get_script_info(&self.active_filename.clone()).get_line_content(self.current_caret_position.line as i32);
        crate::go::assert::equal(
            t,
            actual_content.as_str(),
            expected_content,
            &format!("\n  actual line: \"{actual_content}\"\nexpected line: \"{expected_content}\"\n"),
        );
    }

    // fourslash.go:1083
    pub fn verify_indentation(&mut self, t: &T, num_spaces: i32) {
        // not implemented
    }

    // fourslash.go:1164
    pub fn verify_completions(&mut self, t: &T, marker_input: Any, expected: Option<CompletionsExpectedList>) -> VerifyCompletionsResult {
        Self::server_unavailable(t, "feature not ported: completions (VerifyCompletions)")
    }

    // fourslash.go:1245
    pub fn get_completions(&mut self, t: &T, user_preferences: Option<lsutil::UserPreferences>) -> Option<lsproto::CompletionList> {
        Self::server_unavailable(t, "feature not ported: completions (GetCompletions)")
    }

    // fourslash.go:1250
    pub fn verify_jsdoc_completion(&mut self, t: &T, marker_input: Any, expected_offset: i32, expected_text: &str, generate_return_in_doc_template: Option<bool>) {
        Self::server_unavailable(t, "feature not ported: completions (VerifyJSDocCompletion)")
    }

    // fourslash.go:1285
    pub fn verify_no_jsdoc_completion(&mut self, t: &T, marker_input: Any) {
        Self::server_unavailable(t, "feature not ported: completions (VerifyNoJSDocCompletion)")
    }

    // fourslash.go:1655
    pub fn resolve_completion_item(&mut self, t: &T, item: Option<lsproto::CompletionItem>) -> Option<lsproto::CompletionItem> {
        Self::server_unavailable(t, "feature not ported: completions (ResolveCompletionItem)")
    }

    // fourslash.go:1691
    pub fn verify_code_fix(&mut self, t: &T, options: VerifyCodeFixOptions) {
        Self::server_unavailable(t, "feature not ported: code actions (VerifyCodeFix)")
    }

    // fourslash.go:1768
    pub fn verify_range_after_code_fix(&mut self, t: &T, expected_text: &str, include_whitespace: bool, error_code: i32, index: i32) {
        Self::server_unavailable(t, "feature not ported: code actions (VerifyRangeAfterCodeFix)")
    }

    // fourslash.go:1822
    pub fn verify_code_fix_available(&mut self, t: &T, expected_descriptions: &[&str]) {
        Self::server_unavailable(t, "feature not ported: code actions (VerifyCodeFixAvailable)")
    }

    // fourslash.go:1857
    pub fn verify_code_fix_not_available(&mut self, t: &T, expected: &[&str]) {
        Self::server_unavailable(t, "feature not ported: code actions (VerifyCodeFixNotAvailable)")
    }

    // fourslash.go:1884
    pub fn verify_code_fix_available_exact(&mut self, t: &T, expected_descriptions: &[&str]) {
        Self::server_unavailable(t, "feature not ported: code actions (VerifyCodeFixAvailableExact)")
    }

    // fourslash.go:1918
    pub fn verify_code_fix_all(&mut self, t: &T, options: VerifyCodeFixAllOptions) {
        Self::server_unavailable(t, "feature not ported: code actions (VerifyCodeFixAll)")
    }

    // fourslash.go:1973
    pub fn verify_source_fix_all(&mut self, t: &T, expected_content: &str) {
        Self::server_unavailable(t, "feature not ported: code actions (VerifySourceFixAll)")
    }

    // fourslash.go:2135
    pub fn verify_organize_imports(&mut self, t: &T, expected_content: &str, code_action_kind: lsproto::CodeActionKind, preferences: Option<lsutil::UserPreferences>) {
        self.verify_organize_imports_with_request_kind(t, expected_content, code_action_kind, code_action_kind, preferences);
    }

    // fourslash.go:2140
    pub fn verify_organize_imports_with_request_kind(
        &mut self,
        t: &T,
        expected_content: &str,
        requested_kind: lsproto::CodeActionKind,
        expected_kind: lsproto::CodeActionKind,
        preferences: Option<lsutil::UserPreferences>,
    ) {
        self.verify_organize_imports_worker(t, expected_content, requested_kind, expected_kind, preferences);
    }

    // fourslash.go:2151
    fn verify_organize_imports_worker(
        &mut self,
        t: &T,
        expected_content: &str,
        requested_kind: lsproto::CodeActionKind,
        expected_kind: lsproto::CodeActionKind,
        preferences: Option<lsutil::UserPreferences>,
    ) {
        t.helper();

        let reset = preferences.map(|preferences| self.configure_with_reset(t, preferences));

        let active = self.active_filename.clone();
        let script = self.get_script_info(&active);
        let params = lsproto::CodeActionParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&active) },
            range: lsproto::Range {
                start: lsproto::Position { line: 0, character: 0 },
                end: self.converters.position_to_line_and_character(&script, script.content.len() as TextPos),
            },
            context: lsproto::CodeActionContext { only: Some(vec![requested_kind]), ..Default::default() },
            ..Default::default()
        };

        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_CODE_ACTION_INFO, params);

        let Some(items) = result.command_or_code_action_array.filter(|a| !a.is_empty()) else {
            t.fatal("No organize imports code action found");
        };

        let mut organize_action: Option<lsproto::CodeAction> = None;
        for item in items {
            if let Some(code_action) = item.code_action {
                if code_action.kind == Some(expected_kind) {
                    organize_action = Some(code_action);
                    break;
                }
            }
        }

        let Some(organize_action) = organize_action else {
            t.fatal("No organize imports code action found");
        };

        let expected_uri = lsconv::file_name_to_document_uri(&active);
        if let Some(changes) = organize_action.edit.and_then(|e| e.changes) {
            for (uri, edits) in changes {
                if uri != expected_uri {
                    t.fatal(&format!("Organize imports changed unexpected file: {} (expected {})", uri, expected_uri));
                }
                self.apply_text_edits(t, edits);
            }
        }

        let actual_content = self.get_script_info(&active).content;
        if actual_content != expected_content {
            t.fatal(&format!("Organize imports result doesn't match.\nExpected:\n{}\n\nActual:\n{}", expected_content, actual_content));
        }
        if let Some(reset) = reset {
            reset(self, t);
        }
    }

    // fourslash.go:2241
    pub fn verify_apply_code_action_from_completion(&mut self, t: &T, marker_name: Option<String>, options: Option<ApplyCodeActionFromCompletionOptions>) {
        Self::server_unavailable(t, "feature not ported: completions (VerifyApplyCodeActionFromCompletion)")
    }

    // fourslash.go:2291
    pub fn verify_import_fix_at_position(&mut self, t: &T, expected_texts: &[&str], preferences: Option<lsutil::UserPreferences>) {
        Self::server_unavailable(t, "feature not ported: code actions (VerifyImportFixAtPosition)")
    }

    // fourslash.go:2414
    pub fn verify_import_fix_module_specifiers(
        &mut self,
        t: &T,
        marker_name: &str,
        expected_module_specifiers: &[&str],
        preferences: Option<lsutil::UserPreferences>,
    ) {
        Self::server_unavailable(t, "feature not ported: code actions (VerifyImportFixModuleSpecifiers)")
    }

    // fourslash.go:2523
    pub fn verify_baseline_find_all_references(&mut self, t: &T, markers: &[&str]) {
        let reference_locations = self.lookup_markers_or_get_ranges(t, markers);

        for marker_or_range in reference_locations {
            // worker in `baselineEachMarkerOrRange`
            self.go_to_marker_or_range(t, marker_or_range.clone());

            let params = lsproto::ReferenceParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
                position: self.current_caret_position,
                context: lsproto::ReferenceContext { include_declaration: true },
                ..Default::default()
            };
            let result = self.send_request(t, lsproto::TEXT_DOCUMENT_REFERENCES_INFO, params);
            let Some(locations) = result.locations else {
                t.fatal("nil pointer dereference: result.Locations");
            };
            let baseline = self.get_baseline_for_locations_with_file_contents(
                &locations,
                BaselineFourslashLocationsOptions { marker: Some(marker_or_range), marker_name: "/*FIND ALL REFS*/".to_string(), ..Default::default() },
            );
            self.add_result_to_baseline(t, FIND_ALL_REFERENCES_CMD, &baseline);
        }
    }

    // fourslash.go:2551
    pub fn verify_baseline_vs_find_all_references(&mut self, t: &T, markers: &[&str]) {
        let reference_locations = self.lookup_markers_or_get_ranges(t, markers);

        for marker_or_range in reference_locations {
            self.go_to_marker_or_range(t, marker_or_range.clone());

            let params = lsproto::ReferenceParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
                position: self.current_caret_position,
                context: lsproto::ReferenceContext { include_declaration: true },
                ..Default::default()
            };
            let mut result = self.send_request(t, lsproto::TEXT_DOCUMENT_VS_REFERENCES_INFO, params);
            // Sort cross-project results for deterministic baselines
            if let Some(items) = result.vs_reference_items.as_mut().filter(|items| !items.is_empty()) {
                items.sort_by(|a, b| {
                    let ap = a.vs_project_name.clone().unwrap_or_default();
                    let bp = b.vs_project_name.clone().unwrap_or_default();
                    if ap != bp {
                        return ap.cmp(&bp);
                    }
                    if a.vs_location.uri != b.vs_location.uri {
                        return a.vs_location.uri.0.cmp(&b.vs_location.uri.0);
                    }
                    if a.vs_location.range.start.line != b.vs_location.range.start.line {
                        return a.vs_location.range.start.line.cmp(&b.vs_location.range.start.line);
                    }
                    a.vs_location.range.start.character.cmp(&b.vs_location.range.start.character)
                });
                // Re-number IDs sequentially after sort
                let mut id_remap: FxHashMap<i32, i32> = FxHashMap::default();
                for (i, item) in items.iter_mut().enumerate() {
                    id_remap.insert(item.vs_id, i as i32);
                    item.vs_id = i as i32;
                }
                for item in items.iter_mut() {
                    if let Some(definition_id) = item.vs_definition_id {
                        item.vs_definition_id = Some(id_remap.get(&definition_id).copied().unwrap_or(0));
                    }
                }
            }
            // Include file contents with markers
            let mut locations: Vec<lsproto::Location> = Vec::new();
            if let Some(items) = &result.vs_reference_items {
                for item in items {
                    locations.push(item.vs_location.clone());
                }
            }
            let file_contents = self.get_baseline_for_locations_with_file_contents(
                &locations,
                BaselineFourslashLocationsOptions { marker: Some(marker_or_range), marker_name: "/*FIND ALL REFS*/".to_string(), ..Default::default() },
            );

            match tsrs_core::json::marshal_indent(&result.to_json(), "", "  ") {
                Ok(json_str) => self.add_result_to_baseline(t, VS_FIND_ALL_REFERENCES_CMD, &(file_contents + "\n\n" + &json_str)),
                Err(err) => t.fatal(&format!("Failed to stringify VS references result for baseline: {err}")),
            }
        }
    }

    // fourslash.go:2631
    pub fn verify_baseline_code_lens(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>) {
        Self::server_unavailable(t, "feature not ported: code lens (VerifyBaselineCodeLens)")
    }

    // fourslash.go:2691
    pub fn mark_test_as_strada_server(&mut self) {
        self.is_strada_server = true;
    }

    // fourslash.go:2695
    pub fn verify_baseline_go_to_definition(&mut self, t: &T, include_original_selection_range: bool, markers: &[&str]) {
        self.verify_baseline_definitions(
            t,
            GO_TO_DEFINITION_CMD,
            "/*GOTO DEF*/", /*definitionMarker*/
            |t: &T, f: &mut FourslashTest, _file_name: &str, _position: lsproto::Position| {
                let params = lsproto::DefinitionParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&f.active_filename) },
                    position: f.current_caret_position,
                    ..Default::default()
                };

                f.send_request(t, lsproto::TEXT_DOCUMENT_DEFINITION_INFO, params)
            },
            include_original_selection_range,
            markers,
        );
    }

    // fourslash.go:2775
    pub fn verify_baseline_go_to_type_definition(&mut self, t: &T, markers: &[&str]) {
        self.verify_baseline_definitions(
            t,
            GO_TO_TYPE_DEFINITION_CMD,
            "/*GOTO TYPE*/", /*definitionMarker*/
            |t: &T, f: &mut FourslashTest, _file_name: &str, _position: lsproto::Position| {
                let params = lsproto::TypeDefinitionParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&f.active_filename) },
                    position: f.current_caret_position,
                    ..Default::default()
                };

                f.send_request(t, lsproto::TEXT_DOCUMENT_TYPE_DEFINITION_INFO, params)
            },
            false, /*includeOriginalSelectionRange*/
            markers,
        );
    }

    // fourslash.go:2798
    pub fn verify_baseline_go_to_source_definition(&mut self, t: &T, markers: &[&str]) {
        self.verify_baseline_definitions(
            t,
            GO_TO_SOURCE_DEFINITION_CMD,
            "/*GOTO SOURCE DEF*/", /*definitionMarker*/
            |t: &T, f: &mut FourslashTest, _file_name: &str, _position: lsproto::Position| {
                let params = lsproto::TextDocumentPositionParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&f.active_filename) },
                    position: f.current_caret_position,
                };

                // (Go's result is a pointer to the union; nil and the empty union print the same.)
                f.send_request(t, lsproto::CUSTOM_TEXT_DOCUMENT_SOURCE_DEFINITION_INFO, params)
            },
            false, /*includeOriginalSelectionRange*/
            markers,
        );
    }

    // fourslash.go:2825
    pub fn verify_baseline_workspace_symbol(&mut self, t: &T, query: &str) {
        Self::server_unavailable(t, "feature not ported: workspace symbols (VerifyBaselineWorkspaceSymbol)")
    }

    // fourslash.go:2850
    pub fn verify_outlining_spans(&mut self, t: &T, folding_range_kind: &[lsproto::FoldingRangeKind]) {
        Self::server_unavailable(t, "feature not ported: folding ranges (VerifyOutliningSpans)")
    }

    // fourslash.go:2907
    pub fn verify_folding_range_lines(&mut self, t: &T, expected: &[FoldingRangeLineExpected]) {
        Self::server_unavailable(t, "feature not ported: folding ranges (VerifyFoldingRangeLines)")
    }

    // fourslash.go:2932
    pub fn verify_baseline_hover(&mut self, t: &T) {
        let mut markers_and_items: Vec<MarkerAndItem<Option<lsproto::Hover>>> = Vec::new();
        for marker in self.markers() {
            if marker.name.is_none() {
                continue;
            }

            let params = lsproto::HoverParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&marker.file_name) },
                position: marker.ls_position,
                ..Default::default()
            };

            let result = self.send_request(t, lsproto::TEXT_DOCUMENT_HOVER_INFO, params);
            markers_and_items.push(MarkerAndItem { marker, item: result.hover });
        }

        let get_range = |item: &Option<lsproto::Hover>| -> Option<lsproto::Range> { item.as_ref().and_then(|item| item.range) };

        let get_tooltip_lines = |item: &Option<lsproto::Hover>, _prev: &Option<lsproto::Hover>| -> Vec<String> {
            let item = item.as_ref().unwrap();
            hover_tooltip_lines(item)
        };

        let annotated = self.annotate_content_with_tooltips(t, &markers_and_items, "quickinfo", get_range, get_tooltip_lines);
        self.add_result_to_baseline(t, QUICK_INFO_CMD, &annotated);
        let json = Value::Array(markers_and_items.iter().map(|m| marker_and_item_to_json(m, |h| h.as_ref().map_or(Value::Null, |h| h.to_json()))).collect());
        match tsrs_core::json::marshal_indent(&json, "", "  ") {
            Ok(json_str) => self.write_to_baseline(QUICK_INFO_CMD, &json_str),
            Err(err) => t.fatal(&format!("Failed to stringify markers and items for baseline: {err}")),
        }
    }
    // VerifyBaselineVSHover is like VerifyBaselineHover, but asserts on the VS-specific rich hover
    // content (Hover.VSRawContent / the "_vs_rawContent" wire field) that the LSP server emits for
    // clients advertising the VSSupportsVisualStudioExtensions capability.

    // fourslash.go:2992
    pub fn verify_baseline_vs_hover(&mut self, t: &T) {
        let mut markers_and_items: Vec<MarkerAndItem<Option<lsproto::Hover>>> = Vec::new();
        for marker in self.markers() {
            if marker.name.is_none() {
                continue;
            }

            let params = lsproto::HoverParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&marker.file_name) },
                position: marker.ls_position,
                ..Default::default()
            };

            let result = self.send_request(t, lsproto::TEXT_DOCUMENT_HOVER_INFO, params);
            markers_and_items.push(MarkerAndItem { marker, item: result.hover });
        }

        let get_range = |item: &Option<lsproto::Hover>| -> Option<lsproto::Range> { item.as_ref().and_then(|item| item.range) };

        let get_tooltip_lines = |item: &Option<lsproto::Hover>, _prev: &Option<lsproto::Hover>| -> Vec<String> {
            let Some(item) = item else {
                return Vec::new();
            };
            let Some(vs_raw_content) = &item.vs_raw_content else {
                return vec!["(no _vs_rawContent; is VSSupportsVisualStudioExtensions set on the test's ClientCapabilities?)".to_string()];
            };
            render_vs_container_element(vs_raw_content, "")
        };

        let annotated = self.annotate_content_with_tooltips(t, &markers_and_items, "vsquickinfo", get_range, get_tooltip_lines);
        self.add_result_to_baseline(t, VS_QUICK_INFO_CMD, &annotated);
        let json = Value::Array(markers_and_items.iter().map(|m| marker_and_item_to_json(m, |h| h.as_ref().map_or(Value::Null, |h| h.to_json()))).collect());
        match tsrs_core::json::marshal_indent(&json, "", "  ") {
            Ok(json_str) => self.write_to_baseline(VS_QUICK_INFO_CMD, &json_str),
            Err(err) => t.fatal(&format!("Failed to stringify markers and items for baseline: {err}")),
        }
    }

    // fourslash.go:3084
    pub fn verify_baseline_hover_with_verbosity(&mut self, t: &T, verbosity_levels: OrderedMap<String, Vec<i32>>) {
        let mut markers_and_items: Vec<MarkerAndItem<Option<HoverWithVerbosity>>> = Vec::new();
        for marker in self.markers() {
            let Some(name) = &marker.name else {
                continue;
            };
            let levels = match verbosity_levels.get(name) {
                Some(levels) => levels.clone(),
                None => vec![0],
            };
            for (i, &level) in levels.iter().enumerate() {
                let verb_level = if level > 0 { Some(level) } else { None };
                let params = lsproto::HoverParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&marker.file_name) },
                    position: marker.ls_position,
                    verbosity_level: verb_level,
                    ..Default::default()
                };
                let result = self.send_request(t, lsproto::TEXT_DOCUMENT_HOVER_INFO, params);
                let item = HoverWithVerbosity { hover: result.hover, verbosity_level: level };
                // If the previous level said it can't expand further, verify the hover
                // content is identical, meaning the flag was accurate.
                if i > 0 && level > levels[i - 1] {
                    if let Some(prev_item) = &markers_and_items.last().unwrap().item {
                        if let Some(prev_hover) = &prev_item.hover {
                            if !prev_hover.can_increase_verbosity {
                                let prev_content = hover_content_string(prev_item.hover.as_ref());
                                let cur_content = hover_content_string(item.hover.as_ref());
                                if prev_content != cur_content {
                                    t.error(&format!(
                                        "At marker {}: verbosity level {} response differs from level {}, but level {} had canIncreaseVerbosity=false.\n  level {}: {}\n  level {}: {}",
                                        crate::go::quote(name),
                                        level,
                                        levels[i - 1],
                                        levels[i - 1],
                                        levels[i - 1],
                                        prev_content,
                                        level,
                                        cur_content
                                    ));
                                }
                            }
                        }
                    }
                }
                markers_and_items.push(MarkerAndItem { marker: marker.clone(), item: Some(item) });
            }
        }

        let get_range = |item: &Option<HoverWithVerbosity>| -> Option<lsproto::Range> { item.as_ref().and_then(|item| item.hover.as_ref()).and_then(|h| h.range) };

        let get_tooltip_lines = |item: &Option<HoverWithVerbosity>, _prev: &Option<HoverWithVerbosity>| -> Vec<String> {
            let Some(item) = item else {
                return Vec::new();
            };
            let Some(hover) = &item.hover else {
                return Vec::new();
            };
            let mut result = hover_tooltip_lines(hover);

            result.push(format!("(verbosity level: {})", item.verbosity_level));

            result
        };

        let annotated = self.annotate_content_with_tooltips(t, &markers_and_items, "quickinfo", get_range, get_tooltip_lines);
        self.add_result_to_baseline(t, QUICK_INFO_CMD, &annotated);
        let json = Value::Array(
            markers_and_items
                .iter()
                .map(|m| {
                    marker_and_item_to_json(m, |h| match h {
                        None => Value::Null,
                        Some(h) => {
                            let mut o = OrderedMap::default();
                            o.insert("hover".to_string(), h.hover.as_ref().map_or(Value::Null, |h| h.to_json()));
                            o.insert("verbosityLevel".to_string(), Value::Number(h.verbosity_level as f64));
                            Value::Object(o)
                        }
                    })
                })
                .collect(),
        );
        match tsrs_core::json::marshal_indent(&json, "", "  ") {
            Ok(json_str) => self.write_to_baseline(QUICK_INFO_CMD, &json_str),
            Err(err) => t.fatal(&format!("Failed to stringify markers and items for baseline: {err}")),
        }
    }

    // fourslash.go:3173
    pub fn verify_baseline_signature_help(&mut self, t: &T) {
        Self::server_unavailable(t, "feature not ported: signature help (VerifyBaselineSignatureHelp)")
    }

    // fourslash.go:3283
    pub fn verify_baseline_selection_ranges(&mut self, t: &T) {
        Self::server_unavailable(t, "feature not ported: selection ranges (VerifyBaselineSelectionRanges)")
    }

    // fourslash.go:3421
    pub fn verify_baseline_call_hierarchy(&mut self, t: &T) {
        let file_name = self.active_filename.clone();
        let position = self.current_caret_position;

        let params = lsproto::CallHierarchyPrepareParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&file_name) },
            position,
            ..Default::default()
        };

        let prepare_result = self.send_request(t, lsproto::TEXT_DOCUMENT_PREPARE_CALL_HIERARCHY_INFO, params);
        let items = match prepare_result.call_hierarchy_items {
            Some(items) if !items.is_empty() => items,
            _ => {
                self.add_result_to_baseline(t, CALL_HIERARCHY_CMD, "No call hierarchy items available");
                return;
            }
        };

        let mut result = String::new();

        for call_hierarchy_item in items {
            let mut seen: rustc_hash::FxHashSet<callHierarchyItemKey> = rustc_hash::FxHashSet::default();
            let item_file_name = call_hierarchy_item.uri.file_name();
            let script = self.get_or_load_script_info(&item_file_name);
            format_call_hierarchy_item(t, self, script.as_ref(), &mut result, &call_hierarchy_item, callHierarchyItemDirection::Root, &mut seen, "");
        }

        self.add_result_to_baseline(t, CALL_HIERARCHY_CMD, result.strip_suffix('\n').unwrap_or(&result));
    }

    // fourslash.go:3740
    pub fn verify_baseline_document_highlights(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>, marker_or_range_or_names: &[Any]) {
        self.verify_baseline_document_highlights_with_options(t, preferences, &[], marker_or_range_or_names);
    }

    // fourslash.go:3748
    pub fn verify_baseline_document_highlights_with_options(
        &mut self,
        t: &T,
        preferences: Option<lsutil::UserPreferences>,
        files_to_search: &[&str],
        marker_or_range_or_names: &[Any],
    ) {
        let mut marker_or_ranges: Vec<MarkerOrRange> = Vec::new();
        for marker_or_range_or_name in marker_or_range_or_names {
            match marker_or_range_or_name {
                Any::String(name) => match self.test_data.marker_positions.get(name.as_str()) {
                    Some(marker) => marker_or_ranges.push(MarkerOrRange::Marker(marker.clone())),
                    None => t.fatal(&format!("Marker '{name}' not found")),
                },
                Any::Marker(marker) => marker_or_ranges.push(MarkerOrRange::Marker(marker.clone())),
                Any::RangeMarker(range) => marker_or_ranges.push(MarkerOrRange::RangeMarker(range.clone())),
                other => t.fatal(&format!("Invalid marker or range type: {}. Expected string, *Marker, or *RangeMarker.", other.type_name())),
            }
        }

        self.verify_baseline_document_highlights_impl(t, preferences, files_to_search, marker_or_ranges);
    }

    // fourslash.go:3775
    fn verify_baseline_document_highlights_impl(
        &mut self,
        t: &T,
        _preferences: Option<lsutil::UserPreferences>,
        files_to_search: &[&str],
        marker_or_ranges: Vec<MarkerOrRange>,
    ) {
        for marker_or_range in marker_or_ranges {
            self.go_to_marker_impl(t, &marker_or_range);

            let mut spans: Vec<lsproto::Location> = Vec::new();
            let mut header = String::new();

            if !files_to_search.is_empty() {
                // Multi-file: use the custom method.
                let search_uris: Vec<lsproto::DocumentUri> = files_to_search.iter().map(|file| lsconv::file_name_to_document_uri(file)).collect();

                let params = lsproto::MultiDocumentHighlightParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
                    position: self.current_caret_position,
                    files_to_search: search_uris,
                };
                let result = self.send_request(t, lsproto::CUSTOM_TEXT_DOCUMENT_MULTI_DOCUMENT_HIGHLIGHT_INFO, params);
                let multi_highlights = result.multi_document_highlights.unwrap_or_default();

                for mh in &multi_highlights {
                    for h in &mh.highlights {
                        spans.push(lsproto::Location { uri: mh.uri.clone(), range: h.range });
                    }
                }

                header.push_str("// filesToSearch:\n");
                for file in files_to_search {
                    header.push_str(&format!("//   {}\n", file));
                }
                header.push('\n');
            } else {
                // Single-file: use the standard LSP method.
                let params = lsproto::DocumentHighlightParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
                    position: self.current_caret_position,
                    ..Default::default()
                };
                let result = self.send_request(t, lsproto::TEXT_DOCUMENT_DOCUMENT_HIGHLIGHT_INFO, params);
                let highlights = result.document_highlights.unwrap_or_default();

                for h in &highlights {
                    spans.push(lsproto::Location { uri: lsconv::file_name_to_document_uri(&self.active_filename), range: h.range });
                }
            }

            // Add result to baseline
            let baseline = self.get_baseline_for_locations_with_file_contents(
                &spans,
                BaselineFourslashLocationsOptions { marker: Some(marker_or_range), marker_name: "/*HIGHLIGHTS*/".to_string(), ..Default::default() },
            );
            self.add_result_to_baseline(t, DOCUMENT_HIGHLIGHTS_CMD, &(header + &baseline));
        }
    }

    // fourslash.go:3890
    // Insert text at the current caret position.
    pub fn insert(&mut self, t: &T, text: &str) {
        t.helper();
        self.baseline_state(t);
        self.type_text(t, text);
    }

    // fourslash.go:3897
    // Insert text and a new line at the current caret position.
    pub fn insert_line(&mut self, t: &T, text: &str) {
        t.helper();
        self.baseline_state(t);
        self.type_text(t, &format!("{text}\n"));
    }

    // fourslash.go:3904
    // Removes the text at the current caret position as if the user pressed backspace `count` times.
    pub fn backspace(&mut self, t: &T, count: i32) {
        let script = self.get_script_info(&self.active_filename.clone());
        let mut offset = self.converters.line_and_character_to_position(script, self.current_caret_position);
        self.baseline_state(t);

        for _ in 0..count {
            offset -= 1;
            let active = self.active_filename.clone();
            self.edit_script_and_update_markers(t, &active, offset, offset + 1, "");
            // (Go's `script` is the shared *scriptInfo, so it sees the edit.)
            let script = self.get_script_info(&active);
            self.current_caret_position = self.converters.position_to_line_and_character(&script, offset);
            // Don't need to examine formatting because there are no formatting changes on backspace.
        }

        // f.checkPostEditInvariants() // !!! do we need this?
    }

    // fourslash.go:3920
    // DeleteAtCaret removes the text at the current caret position as if the user pressed delete `count` times.
    pub fn delete_at_caret(&mut self, t: &T, count: i32) {
        let script = self.get_script_info(&self.active_filename.clone());
        let offset = self.converters.line_and_character_to_position(script, self.current_caret_position);
        self.baseline_state(t);

        for _ in 0..count {
            let active = self.active_filename.clone();
            self.edit_script_and_update_markers(t, &active, offset, offset + 1, "");
            // Position stays the same after delete (unlike backspace)
        }
    }

    // fourslash.go:3932
    // Enters text as if the user had pasted it.
    pub fn paste(&mut self, t: &T, text: &str) {
        let script = self.get_script_info(&self.active_filename.clone());
        let start = self.converters.line_and_character_to_position(script, self.current_caret_position);
        self.baseline_state(t);
        let active = self.active_filename.clone();
        self.edit_script_and_update_markers(t, &active, start, start, text);

        // post-paste fomatting
        if self.state_enable_formatting {
            let script = self.get_script_info(&active);
            let params = lsproto::DocumentRangeFormattingParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&active) },
                range: lsproto::Range {
                    start: self.current_caret_position,
                    end: self.converters.position_to_line_and_character(&script, start + text.len() as i32),
                },
                options: self.user_preferences.format_code_settings.to_ls_format_options(),
                ..Default::default()
            };
            let result = self.send_request_and_baseline_worker(t, lsproto::TEXT_DOCUMENT_RANGE_FORMATTING_INFO, params, false);
            if let Some(text_edits) = result.text_edits {
                self.apply_text_edits(t, text_edits);
            }
        }
        // this.checkPostEditInvariants(); // !!! do we need this?
    }

    // fourslash.go:3958
    // Selects a line and replaces it with a new text.
    pub fn replace_line(&mut self, t: &T, line_index: i32, text: &str) {
        self.baseline_state(t);
        self.select_line(t, line_index);
        self.type_text(t, text);
    }

    // fourslash.go:4032
    pub fn replace(&mut self, t: &T, start: i32, length: i32, text: &str) {
        self.baseline_state(t);
        self.replace_worker(t, start, length, text);
    }

    // fourslash.go:4183
    // !!! expected tags
    pub fn verify_quick_info_at(&mut self, t: &T, marker: &str, expected_text: &str, expected_documentation: &str) {
        self.go_to_marker(t, marker);
        let Some(hover) = self.get_quick_info_at_current_position(t) else {
            t.fatal(&format!("Expected hover result at marker '{}' but got nil", self.last_known_marker_name.clone().unwrap_or_default()));
        };
        let prefix = self.get_current_position_prefix();
        self.verify_hover_content(t, &hover.contents, expected_text, expected_documentation, &prefix);
    }

    // fourslash.go:4229
    pub fn verify_quick_info_exists(&mut self, t: &T) {
        if self.quick_info_is_empty(t).0 {
            t.fatal(&format!("Expected non-nil hover content at marker '{}'", self.last_known_marker_name.clone().unwrap_or_default()));
        }
    }

    // fourslash.go:4235
    pub fn verify_not_quick_info_exists(&mut self, t: &T) {
        let (is_empty, hover) = self.quick_info_is_empty(t);
        if !is_empty {
            t.fatal(&format!("Expected empty hover content at marker '{}', got '{:?}'", self.last_known_marker_name.clone().unwrap_or_default(), hover));
        }
    }

    // fourslash.go:4250
    pub fn verify_quick_info_is(&mut self, t: &T, expected_text: &str, expected_documentation: &str) {
        let hover = self.get_quick_info_at_current_position(t);
        // (Go dereferences the nil hover here.)
        let Some(hover) = hover else {
            panic!("runtime error: invalid memory address or nil pointer dereference");
        };
        let prefix = self.get_current_position_prefix();
        self.verify_hover_content(t, &hover.contents, expected_text, expected_documentation, &prefix);
    }

    // fourslash.go:4255
    pub fn verify_jsx_closing_tag(&mut self, t: &T, markers_to_new_text: OrderedMap<String, Option<String>>) {
        Self::server_unavailable(t, "feature not ported: _vs_onAutoInsert (VerifyJsxClosingTag)")
    }

    // fourslash.go:4285
    pub fn verify_baseline_closing_tags(&mut self, t: &T) {
        Self::server_unavailable(t, "feature not ported: _vs_onAutoInsert (VerifyBaselineClosingTags)")
    }

    // fourslash.go:4353
    pub fn verify_signature_help(&mut self, t: &T, expected: VerifySignatureHelpOptions) {
        Self::server_unavailable(t, "feature not ported: signature help (VerifySignatureHelp)")
    }

    // fourslash.go:4513
    pub fn verify_no_signature_help(&mut self, t: &T) {
        Self::server_unavailable(t, "feature not ported: signature help (VerifyNoSignatureHelp)")
    }

    // fourslash.go:4529
    pub fn verify_no_signature_help_with_context(&mut self, t: &T, context: Option<lsproto::SignatureHelpContext>) {
        Self::server_unavailable(t, "feature not ported: signature help (VerifyNoSignatureHelpWithContext)")
    }

    // fourslash.go:4546
    pub fn verify_no_signature_help_for_markers_with_context(&mut self, t: &T, context: Option<lsproto::SignatureHelpContext>, markers: &[&str]) {
        for marker in markers {
            self.go_to_marker(t, marker);
            self.verify_no_signature_help_with_context(t, context.clone());
        }
    }

    // fourslash.go:4555
    pub fn verify_signature_help_present(&mut self, t: &T, context: Option<lsproto::SignatureHelpContext>) {
        Self::server_unavailable(t, "feature not ported: signature help (VerifySignatureHelpPresent)")
    }

    // fourslash.go:4572
    pub fn verify_signature_help_present_for_markers(&mut self, t: &T, context: Option<lsproto::SignatureHelpContext>, markers: &[&str]) {
        for marker in markers {
            self.go_to_marker(t, marker);
            self.verify_signature_help_present(t, context.clone());
        }
    }

    // fourslash.go:4581
    pub fn verify_no_signature_help_for_markers(&mut self, t: &T, markers: &[&str]) {
        for marker in markers {
            self.go_to_marker(t, marker);
            self.verify_no_signature_help(t);
        }
    }

    // fourslash.go:4597
    pub fn verify_signature_help_with_cases(&mut self, t: &T, signature_help_cases: &[SignatureHelpCase]) {
        Self::server_unavailable(t, "feature not ported: signature help (VerifySignatureHelpWithCases)")
    }

    // fourslash.go:4657
    pub fn baseline_auto_imports_completions(&mut self, t: &T, marker_names: &[&str]) {
        Self::server_unavailable(t, "feature not ported: completions (BaselineAutoImportsCompletions)")
    }

    // fourslash.go:4754
    pub fn verify_baseline_rename(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>, marker_or_name_or_ranges: &[Any]) {
        let mut marker_or_ranges: Vec<MarkerOrRange> = Vec::new();
        for marker_or_name_or_range in marker_or_name_or_ranges {
            match marker_or_name_or_range {
                Any::String(name) => match self.test_data.marker_positions.get(name.as_str()) {
                    Some(marker) => marker_or_ranges.push(MarkerOrRange::Marker(marker.clone())),
                    None => t.fatal(&format!("Marker '{name}' not found")),
                },
                Any::Marker(marker) => marker_or_ranges.push(MarkerOrRange::Marker(marker.clone())),
                Any::RangeMarker(range) => marker_or_ranges.push(MarkerOrRange::RangeMarker(range.clone())),
                other => t.fatal(&format!("Invalid marker or range type: {}. Expected string, *Marker, or *RangeMarker.", other.type_name())),
            }
        }

        self.verify_baseline_rename_impl(t, preferences, marker_or_ranges);
    }

    // fourslash.go:4780
    fn verify_baseline_rename_impl(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>, marker_or_ranges: Vec<MarkerOrRange>) {
        let reset = preferences.clone().map(|preferences| self.configure_with_reset(t, preferences));

        for marker_or_range in marker_or_ranges {
            self.go_to_marker_or_range(t, marker_or_range.clone());

            let params = lsproto::RenameParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
                position: self.current_caret_position,
                new_name: "?".to_string(),
                ..Default::default()
            };

            let result = self.send_request(t, lsproto::TEXT_DOCUMENT_RENAME_INFO, params);

            let changes = result.workspace_edit.and_then(|edit| edit.changes).unwrap_or_default();
            let mut span_to_text: FxHashMap<DocumentSpan, String> = FxHashMap::default();
            let mut file_to_span: MultiMap<lsproto::DocumentUri, DocumentSpan> = MultiMap::default();
            for (uri, edits) in &changes {
                for edit in edits {
                    let span = DocumentSpan { uri: uri.clone(), text_span: edit.range, context_span: None };
                    file_to_span.add(uri.clone(), span.clone());
                    span_to_text.insert(span, edit.new_text.clone());
                }
            }

            let mut rename_options = String::new();
            if let Some(preferences) = &preferences {
                if preferences.use_aliases_for_rename != Tristate::Unknown {
                    rename_options.push_str(&format!("// @useAliasesForRename: {}\n", preferences.use_aliases_for_rename.is_true()));
                }
                if preferences.quote_preference != lsutil::QuotePreference::Unknown {
                    rename_options.push_str(&format!("// @quotePreference: {}\n", preferences.quote_preference.as_str()));
                }
            }

            let span_to_text = Arc::new(span_to_text);
            let prefix_texts = span_to_text.clone();
            let suffix_texts = span_to_text.clone();
            let baseline_file_content = self.get_baseline_for_grouped_spans_with_file_contents(
                &file_to_span,
                &BaselineFourslashLocationsOptions {
                    marker: Some(marker_or_range),
                    marker_name: "/*RENAME*/".to_string(),
                    end_marker: "RENAME|]".to_string(),
                    start_marker_prefix: Some(Box::new(move |span: &DocumentSpan| {
                        let text = prefix_texts.get(span).cloned().unwrap_or_default();
                        let prefix_and_suffix: Vec<&str> = text.split('?').collect();
                        if !prefix_and_suffix[0].is_empty() {
                            return Some(format!("/*START PREFIX*/{}", prefix_and_suffix[0]));
                        }
                        None
                    })),
                    end_marker_suffix: Some(Box::new(move |span: &DocumentSpan| {
                        let text = suffix_texts.get(span).cloned().unwrap_or_default();
                        let prefix_and_suffix: Vec<&str> = text.split('?').collect();
                        // (Go indexes [1] and panics when the text has no "?".)
                        if !prefix_and_suffix[1].is_empty() {
                            return Some(format!("{}/*END SUFFIX*/", prefix_and_suffix[1]));
                        }
                        None
                    })),
                    ..Default::default()
                },
            );

            let baseline_result = if !rename_options.is_empty() { rename_options + "\n" + &baseline_file_content } else { baseline_file_content };

            self.add_result_to_baseline(t, RENAME_CMD, &baseline_result);
        }
        if let Some(reset) = reset {
            reset(self, t);
        }
    }

    // fourslash.go:4862
    pub fn verify_rename_succeeded(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>) {
        let reset = preferences.map(|preferences| self.configure_with_reset(t, preferences));
        let params = lsproto::PrepareRenameParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            ..Default::default()
        };

        let prefix = self.get_current_position_prefix();
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_PREPARE_RENAME_INFO, params);
        if result.range.is_none() && result.prepare_rename_placeholder.is_none() && result.prepare_rename_default_behavior.is_none() {
            t.fatal(&(prefix + "Expected rename to succeed, but prepareRename returned null"));
        }

        // Also verify that textDocument/rename produces edits, since prepareRename is optional.
        let rename_params = lsproto::RenameParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            new_name: "RENAME_SUCCEEDED_TEST".to_string(),
            ..Default::default()
        };
        let rename_result = self.send_request(t, lsproto::TEXT_DOCUMENT_RENAME_INFO, rename_params);
        if rename_result.workspace_edit.and_then(|edit| edit.changes).is_none_or(|changes| changes.is_empty()) {
            t.fatal(&(prefix + "prepareRename succeeded but textDocument/rename returned no changes"));
        }
        if let Some(reset) = reset {
            reset(self, t);
        }
    }

    // fourslash.go:4892
    pub fn verify_rename_range(&mut self, t: &T, expected_range: lsproto::Range, expected_placeholder: &str, preferences: Option<lsutil::UserPreferences>) {
        t.helper();
        let reset = preferences.map(|preferences| self.configure_with_reset(t, preferences));
        let params = lsproto::PrepareRenameParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            ..Default::default()
        };

        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_PREPARE_RENAME_INFO, params);
        let Some(placeholder) = result.prepare_rename_placeholder else {
            t.fatal(&(self.get_current_position_prefix() + "Expected prepareRename to return a range and placeholder"));
        };
        assert_deep_equal(t, &placeholder.range, &expected_range, "");
        if placeholder.placeholder != expected_placeholder {
            t.fatal(&format!("assertion failed: {:?} (result.PrepareRenamePlaceholder.Placeholder) != {:?} (expectedPlaceholder)", placeholder.placeholder, expected_placeholder));
        }
        if let Some(reset) = reset {
            reset(self, t);
        }
    }

    // fourslash.go:4912
    pub fn rename_at_caret(&mut self, t: &T, new_name: &str) -> lsproto::RenameResponse {
        t.helper();
        let params = lsproto::RenameParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            new_name: new_name.to_string(),
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_RENAME_INFO, params);

        let Some(workspace_edit) = &result.workspace_edit else {
            return result;
        };

        if let Some(changes) = &workspace_edit.changes {
            for (uri, edits) in changes {
                let file_name = uri.file_name();
                let script = self.get_or_load_script_info(&file_name).expect("nil pointer dereference: scriptInfo");
                let changes: Vec<TextChange> =
                    edits.iter().map(|edit| TextChange { text_range: self.from_lsp_range(&script, edit.range), new_text: edit.new_text.clone() }).collect();
                self.edit_script_and_update_markers_worker(t, &file_name, &changes);
            }
        }

        let mut rename_files: Vec<lsproto::RenameFile> = Vec::new();
        if let Some(document_changes) = &workspace_edit.document_changes {
            for doc_change in document_changes {
                if let Some(text_document_edit) = &doc_change.text_document_edit {
                    let file_name = text_document_edit.text_document.uri.file_name();
                    let script = self.get_or_load_script_info(&file_name).expect("nil pointer dereference: scriptInfo");
                    let changes: Vec<TextChange> = text_document_edit
                        .edits
                        .iter()
                        .map(|edit| {
                            let text_edit = edit.text_edit.as_ref().expect("nil pointer dereference: TextEdit");
                            TextChange { text_range: self.from_lsp_range(&script, text_edit.range), new_text: text_edit.new_text.clone() }
                        })
                        .collect();
                    self.edit_script_and_update_markers_worker(t, &file_name, &changes);
                } else if let Some(rename_file) = &doc_change.rename_file {
                    rename_files.push(rename_file.clone());
                }
            }
        }

        if !rename_files.is_empty() {
            let file_renames: Vec<lsproto::FileRename> =
                rename_files.iter().map(|rename_file| lsproto::FileRename { old_uri: rename_file.old_uri.clone(), new_uri: rename_file.new_uri.clone() }).collect();
            let will_rename = self
                .capabilities
                .as_ref()
                .and_then(|c| c.workspace.as_ref())
                .and_then(|w| w.file_operations.as_ref())
                .and_then(|o| o.will_rename)
                .unwrap_or(false);
            if will_rename {
                self.will_rename_files_worker(t, &file_renames);
            } else {
                for rename_file in &rename_files {
                    self.rename_file_or_directory(t, &rename_file.old_uri.file_name(), &rename_file.new_uri.file_name());
                }
            }
        }

        result
    }

    // fourslash.go:4992 (needs workspace/willRenameFiles, i.e. ls/file_rename.go, which is not ported)
    fn will_rename_files_worker(&mut self, t: &T, _files: &[lsproto::FileRename]) {
        Self::server_unavailable(t, "feature not ported: willRenameFiles (willRenameFilesWorker)")
    }

    // fourslash.go:4984
    pub fn will_rename_files(&mut self, t: &T, files: &[lsproto::FileRename]) -> lsproto::WillRenameFilesResponse {
        Self::server_unavailable(t, "feature not ported: willRenameFiles (WillRenameFiles)")
    }

    // fourslash.go:5055
    pub fn verify_rename(&mut self, t: &T, marker_name: &str, new_name: &str, expected_file_contents: OrderedMap<String, String>) {
        t.helper();
        self.go_to_marker(t, marker_name);
        self.rename_at_caret(t, new_name);
        for (file_name, expected_content) in &expected_file_contents {
            let Some(script) = self.try_get_script_info(file_name) else {
                t.fatal(&format!("Expected script info for {file_name}, but got nil"));
            };
            if script.content != *expected_content {
                t.fatal(&format!(
                    "assertion failed: {:?} (script.content) != {:?} (expectedContent): File content after rename did not match expected content for {}.",
                    script.content, expected_content, file_name
                ));
            }
        }
    }

    // fourslash.go:5068
    pub fn verify_will_rename_files_edits(
        &mut self,
        t: &T,
        old_path: &str,
        new_path: &str,
        expected_file_contents: OrderedMap<String, String>,
        preferences: Option<lsutil::UserPreferences>,
    ) {
        Self::server_unavailable(t, "feature not ported: willRenameFiles (VerifyWillRenameFilesEdits)")
    }

    // fourslash.go:5186
    pub fn verify_rename_failed(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>) {
        let reset = preferences.map(|preferences| self.configure_with_reset(t, preferences));
        let params = lsproto::PrepareRenameParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            ..Default::default()
        };

        let prefix = self.get_current_position_prefix();
        self.baseline_state(t);
        self.baseline_request_or_notification(t, lsproto::TEXT_DOCUMENT_PREPARE_RENAME_INFO.method, || params.to_json());
        let (res_msg, result) = self.client().send_request(lsproto::TEXT_DOCUMENT_PREPARE_RENAME_INFO, params);
        self.baseline_state(t);

        // prepareRename can reject via an error response (with a localized message) or a null result.
        if res_msg.error.is_some() {
            // Error response — rename was rejected with a message. This is expected.
        } else if let Some(result) = &result {
            if result.range.is_some() || result.prepare_rename_placeholder.is_some() || result.prepare_rename_default_behavior.is_some() {
                t.fatal(&format!("{prefix}Expected rename to fail, but prepareRename returned a result"));
            }
        }

        // Also verify that textDocument/rename does not produce usable edits, since prepareRename is optional.
        let rename_params = lsproto::RenameParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            new_name: "RENAME_FAILED_TEST".to_string(),
            ..Default::default()
        };
        let (rename_msg, rename_result) = self.client().send_request(lsproto::TEXT_DOCUMENT_RENAME_INFO, rename_params);
        if rename_msg.error.is_none() {
            if rename_result.and_then(|r| r.workspace_edit).and_then(|edit| edit.changes).is_some_and(|changes| !changes.is_empty()) {
                t.fatal(&format!("{prefix}prepareRename returned null but textDocument/rename returned changes"));
            }
        }
        if let Some(reset) = reset {
            reset(self, t);
        }
    }

    // fourslash.go:5226
    pub fn verify_baseline_rename_at_ranges_with_text(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>, texts: &[&str]) {
        let mut marker_or_ranges: Vec<MarkerOrRange> = Vec::new();
        for text in texts {
            let ranges_by_text = self.get_ranges_by_text();
            marker_or_ranges.extend(ranges_by_text.get(&text.to_string()).iter().map(|r| MarkerOrRange::RangeMarker(r.clone())));
        }
        self.verify_baseline_rename_impl(t, preferences, marker_or_ranges);
    }

    // fourslash.go:5239
    pub fn get_ranges_by_text(&self) -> Arc<MultiMap<String, Arc<RangeMarker>>> {
        if let Some(r) = self.ranges_by_text.get() {
            return r.clone();
        }
        let mut ranges_by_text: MultiMap<String, Arc<RangeMarker>> = MultiMap::default();
        for r in &self.test_data.ranges {
            let range_text = self.get_range_text(r);
            ranges_by_text.add(range_text, r.clone());
        }
        let r = Arc::new(ranges_by_text);
        let _ = self.ranges_by_text.set(r.clone());
        r
    }

    // fourslash.go:5252
    pub(crate) fn get_range_text(&self, r: &RangeMarker) -> String {
        let script = self.get_script_info(&r.file_name);
        script.content[r.range.pos() as usize..r.range.end() as usize].to_string()
    }

    // fourslash.go:5257
    pub(crate) fn verify_baselines(&mut self, t: &T, test_path: &str) {
        if !self.test_data.is_state_baselining_enabled() {
            for (command, content) in &self.baselines {
                crate::testutil::baseline::run(t, &get_baseline_file_name(t, *command), content, self.get_baseline_options(*command, test_path));
            }
        } else {
            let baseline = self.state_baseline.as_ref().map(|s| s.baseline.clone()).unwrap_or_default();
            crate::testutil::baseline::run(
                t,
                &(get_base_file_name_from_test(t) + ".baseline"),
                &baseline,
                crate::testutil::baseline::Options { subfolder: "fourslash/state".to_string(), ..Default::default() },
            );
        }
    }

    // fourslash.go:5267
    pub fn verify_baseline_inlay_hints(&mut self, t: &T, span: Option<lsproto::Range>, test_preferences: Option<lsutil::UserPreferences>) {
        Self::server_unavailable(t, "feature not ported: inlay hints (VerifyBaselineInlayHints)")
    }

    // fourslash.go:5328
    pub fn verify_baseline_linked_editing(&mut self, t: &T) {
        Self::server_unavailable(t, "feature not ported: linked editing (VerifyBaselineLinkedEditing)")
    }

    // fourslash.go:5407
    pub fn verify_linked_editing(&mut self, t: &T, marker_names_to_expected: OrderedMap<String, Vec<lsproto::Range>>) {
        Self::server_unavailable(t, "feature not ported: linked editing (VerifyLinkedEditing)")
    }

    // fourslash.go:5434
    pub fn verify_diagnostics(&mut self, t: &T, expected: &[lsproto::Diagnostic]) {
        self.verify_diagnostics_impl(t, expected, |_d| true);
    }
    // Similar to `VerifyDiagnostics`, but excludes suggestion diagnostics returned from server.

    // fourslash.go:5439
    pub fn verify_non_suggestion_diagnostics(&mut self, t: &T, expected: &[lsproto::Diagnostic]) {
        self.verify_diagnostics_impl(t, expected, |d| !is_suggestion_diagnostic(d));
    }
    // Similar to `VerifyDiagnostics`, but includes only suggestion diagnostics returned from server.

    // fourslash.go:5444
    pub fn verify_suggestion_diagnostics(&mut self, t: &T, expected: &[lsproto::Diagnostic]) {
        self.verify_diagnostics_impl(t, expected, is_suggestion_diagnostic);
    }

    // fourslash.go:5489
    pub fn verify_baseline_non_suggestion_diagnostics(&mut self, t: &T) {
        let mut diagnostics: Vec<FourslashDiagnostic> = Vec::new();
        let mut files: Vec<crate::tsbaseline::TestFile> = Vec::new();
        // (Go ranges over the map in random order; files and diagnostics are sorted below.)
        let mut script_infos: Vec<ScriptInfo> = self.script_infos.read().unwrap().values().cloned().collect();
        script_infos.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        for script_info in &script_infos {
            let file_name = &script_info.file_name;
            if tspath::has_json_file_extension(file_name) {
                continue;
            }
            files.push(crate::tsbaseline::TestFile { unit_name: file_name.clone(), content: script_info.content.clone() });
            let lsp_diagnostics: Vec<lsproto::Diagnostic> = self.get_diagnostics(t, file_name).into_iter().filter(|d| !is_suggestion_diagnostic(d)).collect();
            for d in &lsp_diagnostics {
                diagnostics.push(self.to_diagnostic(script_info, d));
            }
        }
        files.sort_by(|a, b| a.unit_name.cmp(&b.unit_name));
        let result = crate::tsbaseline::get_error_baseline(t, &files, &diagnostics, compare_diagnostics);
        self.add_result_to_baseline(t, NON_SUGGESTION_DIAGNOSTICS_CMD, &result);
    }

    // fourslash.go:5698
    pub fn verify_baseline_go_to_implementation(&mut self, t: &T, marker_names: &[&str]) {
        self.verify_baseline_definitions(
            t,
            GO_TO_IMPLEMENTATION_CMD,
            "/*GOTO IMPL*/", /*definitionMarker*/
            |t: &T, f: &mut FourslashTest, _file_name: &str, _position: lsproto::Position| {
                let params = lsproto::ImplementationParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&f.active_filename) },
                    position: f.current_caret_position,
                    ..Default::default()
                };

                f.send_request(t, lsproto::TEXT_DOCUMENT_IMPLEMENTATION_INFO, params)
            },
            false, /*includeOriginalSelectionRange*/
            marker_names,
        );
    }

    // fourslash.go:5726
    pub fn verify_workspace_symbol(&mut self, t: &T, cases: &[VerifyWorkspaceSymbolCase]) {
        Self::server_unavailable(t, "feature not ported: workspace symbols (VerifyWorkspaceSymbol)")
    }

    // fourslash.go:5796
    pub fn verify_baseline_document_symbol(&mut self, t: &T) {
        Self::server_unavailable(t, "feature not ported: document symbols (VerifyBaselineDocumentSymbol)")
    }

    // fourslash.go:5871
    pub fn verify_number_of_errors_in_current_file(&mut self, t: &T, expected_count: i32) {
        let diagnostics = self.get_diagnostics(t, &self.active_filename.clone());
        // Filter to only include errors (not suggestions/hints)
        let errors: Vec<&lsproto::Diagnostic> = diagnostics.iter().filter(|d| !is_suggestion_diagnostic(d)).collect();
        if errors.len() as i32 != expected_count {
            t.fatal(&format!("Expected {} errors in current file, but got {}", expected_count, errors.len()));
        }
    }
    // VerifyNoErrors verifies that no errors exist in any open files.

    // fourslash.go:5883
    pub fn verify_no_errors(&mut self, t: &T) {
        let open_files: Vec<String> = self.open_files.keys().cloned().collect();
        for file_name in &open_files {
            let diagnostics = self.get_diagnostics(t, file_name);
            // Filter to only include errors (not suggestions/hints)
            let errors: Vec<&lsproto::Diagnostic> = diagnostics.iter().filter(|d| !is_suggestion_diagnostic(d)).collect();
            if !errors.is_empty() {
                let messages: Vec<String> = errors.iter().map(|err| err.message.as_string()).collect();
                t.fatal(&format!("Expected no errors but found {} in {}: [{}]", errors.len(), file_name, messages.join(" ")));
            }
        }
    }
    // VerifyErrorExistsAtRange verifies that an error with the given code exists at the given range.

    // fourslash.go:5901
    pub fn verify_error_exists_at_range(&mut self, t: &T, range_marker: Arc<RangeMarker>, code: i32, message: &str) {
        let diagnostics = self.get_diagnostics(t, &range_marker.file_name());
        for diag in &diagnostics {
            if let Some(diag_code) = diag.code.as_ref().and_then(|c| c.integer) {
                if diag_code == code {
                    // Check if the range matches
                    if diag.range.start.line == range_marker.ls_range.start.line
                        && diag.range.start.character == range_marker.ls_range.start.character
                        && diag.range.end.line == range_marker.ls_range.end.line
                        && diag.range.end.character == range_marker.ls_range.end.character
                    {
                        // If message is provided, verify it matches
                        if !message.is_empty() && diag.message.as_string() != message {
                            t.fatal(&format!(
                                "Error at range has code {} but message mismatch. Expected: {}, Got: {}",
                                code,
                                crate::go::quote(message),
                                crate::go::quote(&diag.message.as_string())
                            ));
                        }
                        return;
                    }
                }
            }
        }
        t.fatal(&format!("Expected error with code {} at range {:?} but it was not found", code, range_marker.ls_range));
    }
    // VerifyErrorExistsBetweenMarkers verifies that an error exists between the two markers.

    // fourslash.go:5922
    pub fn verify_error_exists_between_markers(&mut self, t: &T, start_marker_name: &str, end_marker_name: &str) {
        let Some(start_marker) = self.test_data.marker_positions.get(start_marker_name).cloned() else {
            t.fatal(&format!("Start marker '{start_marker_name}' not found"));
        };
        let Some(end_marker) = self.test_data.marker_positions.get(end_marker_name).cloned() else {
            t.fatal(&format!("End marker '{end_marker_name}' not found"));
        };
        if start_marker.file_name() != end_marker.file_name() {
            t.fatal(&format!("Markers '{start_marker_name}' and '{end_marker_name}' are in different files"));
        }

        let diagnostics = self.get_diagnostics(t, &start_marker.file_name());
        let start_pos = start_marker.position;
        let end_pos = end_marker.position;

        for diag in &diagnostics {
            if !is_suggestion_diagnostic(diag) {
                let diag_start = self.converters.line_and_character_to_position(self.get_script_info(&start_marker.file_name()), diag.range.start);
                let diag_end = self.converters.line_and_character_to_position(self.get_script_info(&start_marker.file_name()), diag.range.end);
                if diag_start >= start_pos && diag_end <= end_pos {
                    return; // Found an error in the range
                }
            }
        }
        t.fatal(&format!("Expected error between markers '{start_marker_name}' and '{end_marker_name}' but none was found"));
    }
    // VerifyErrorExistsAfterMarker verifies that an error exists after the given marker.

    // fourslash.go:5952
    pub fn verify_error_exists_after_marker(&mut self, t: &T, marker_name: &str) {
        let file_name: String;
        let marker_pos: i32;

        if marker_name.is_empty() {
            // Use current position
            file_name = self.active_filename.clone();
            marker_pos = self.converters.line_and_character_to_position(self.get_script_info(&self.active_filename.clone()), self.current_caret_position);
        } else {
            let Some(marker) = self.test_data.marker_positions.get(marker_name).cloned() else {
                t.fatal(&format!("Marker '{marker_name}' not found"));
            };
            file_name = marker.file_name();
            marker_pos = marker.position;
        }

        let diagnostics = self.get_diagnostics(t, &file_name);

        for diag in &diagnostics {
            if !is_suggestion_diagnostic(diag) {
                let diag_start = self.converters.line_and_character_to_position(self.get_script_info(&file_name), diag.range.start);
                if diag_start >= marker_pos {
                    return; // Found an error after the marker
                }
            }
        }
        t.fatal(&format!("Expected error after marker '{marker_name}' but none was found"));
    }
    // VerifyErrorExistsBeforeMarker verifies that an error exists before the given marker.

    // fourslash.go:5983
    pub fn verify_error_exists_before_marker(&mut self, t: &T, marker_name: &str) {
        let file_name: String;
        let marker_pos: i32;

        if marker_name.is_empty() {
            // Use current position
            file_name = self.active_filename.clone();
            marker_pos = self.converters.line_and_character_to_position(self.get_script_info(&self.active_filename.clone()), self.current_caret_position);
        } else {
            let Some(marker) = self.test_data.marker_positions.get(marker_name).cloned() else {
                t.fatal(&format!("Marker '{marker_name}' not found"));
            };
            file_name = marker.file_name();
            marker_pos = marker.position;
        }

        let diagnostics = self.get_diagnostics(t, &file_name);

        for diag in &diagnostics {
            if !is_suggestion_diagnostic(diag) {
                let diag_end = self.converters.line_and_character_to_position(self.get_script_info(&file_name), diag.range.end);
                if diag_end <= marker_pos {
                    return; // Found an error before the marker
                }
            }
        }
        t.fatal(&format!("Expected error before marker '{marker_name}' but none was found"));
    }

    // fourslash.go:4166 (getScriptInfo): Go returns the shared *scriptInfo; the Rust caller gets a snapshot.
    pub(crate) fn get_script_info(&self, file_name: &str) -> ScriptInfo {
        match self.script_infos.read().unwrap().get(file_name) {
            Some(s) => s.clone(),
            None => panic!("script info for {file_name} not found"),
        }
    }
}

// The request / notification plumbing of fourslash.go (generic methods there, generic methods here).
impl FourslashTest {
    // fourslash.go:743
    pub(crate) fn send_request<Params: Json, Resp: Json + Default>(&mut self, t: &T, info: lsproto::RequestInfo<Params, Resp>, params: Params) -> Resp {
        t.helper();
        self.send_request_and_baseline_worker(t, info, params, true)
    }

    // fourslash.go:748
    pub(crate) fn send_request_and_baseline_worker<Params: Json, Resp: Json + Default>(
        &mut self,
        t: &T,
        info: lsproto::RequestInfo<Params, Resp>,
        params: Params,
        baseline_projects: bool,
    ) -> Resp {
        t.helper();
        let prefix = self.get_current_position_prefix();
        if baseline_projects {
            self.baseline_state(t);
        }
        self.baseline_request_or_notification(t, info.method, || params.to_json());
        let (res_msg, result) = self.client().send_request(info, params);
        if baseline_projects {
            self.baseline_state(t);
        }
        let check = info.method != lsproto::Method::TextDocumentOnTypeFormatting || self.report_format_on_type_crash;
        if check {
            if let Some(err) = &res_msg.error {
                t.fatal(&format!("{prefix}{} request returned error: {}", info.method, err.string()));
            }
            if result.is_none() {
                t.fatal(&format!("{prefix}Unexpected {} response type: {:?}, error: {:?}", info.method, res_msg.result, res_msg.error));
            }
        }
        result.unwrap_or_default()
    }

    // fourslash.go:780
    pub(crate) fn send_notification<Params: Json + 'static>(&mut self, t: &T, info: lsproto::NotificationInfo<Params>, params: Params) {
        t.helper();
        if info.method != lsproto::Method::TextDocumentDidChange {
            // This is called eg when doing typeText = which is series of edits and formatting - which becomes non deterministic "after state"
            // The notification can only guarantee before state and thats what it baselines, but in case of type it creates
            // multiple edits which results in getting different state -based on if the snapshot was updated or not at the time of formatting requests
            // So this is used for all the incremental edits - to baseline only request data but not project state between those edits
            self.baseline_state(t);
            self.update_state(info.method, &params);
        }
        self.baseline_request_or_notification(t, info.method, || params.to_json());
        self.client().send_notification(info, params);
    }

    // fourslash.go:794
    fn update_state(&mut self, method: lsproto::Method, params: &dyn std::any::Any) {
        match method {
            lsproto::Method::TextDocumentDidOpen => {
                let params = params.downcast_ref::<lsproto::DidOpenTextDocumentParams>().unwrap();
                self.open_files.insert(params.text_document.uri.file_name(), ());
            }
            lsproto::Method::TextDocumentDidClose => {
                let params = params.downcast_ref::<lsproto::DidCloseTextDocumentParams>().unwrap();
                self.open_files.shift_remove(&params.text_document.uri.file_name());
            }
            _ => {}
        }
    }

    // fourslash.go:1315
    pub(crate) fn go_to_marker_input(&mut self, t: &T, marker_input: &Any) {
        t.helper();
        match marker_input {
            Any::String(marker) => self.go_to_marker(t, marker),
            Any::Marker(marker) => self.go_to_marker_impl(t, &MarkerOrRange::Marker(marker.clone())),
            _ => t.fatal(&format!("Invalid marker input type: {}. Expected string or *Marker.", marker_input.type_name())),
        }
    }

    // fourslash.go:2719
    fn verify_baseline_definitions(
        &mut self,
        t: &T,
        definition_command: BaselineCommand,
        definition_marker: &str,
        get_definitions: impl Fn(&T, &mut FourslashTest, &str, lsproto::Position) -> lsproto::LocationOrLocationsOrDefinitionLinksOrNull,
        include_original_selection_range: bool,
        markers: &[&str],
    ) {
        let reference_locations = self.lookup_markers_or_get_ranges(t, markers);

        for marker_or_range in reference_locations {
            // worker in `baselineEachMarkerOrRange`
            self.go_to_marker_or_range(t, marker_or_range.clone());

            let active = self.active_filename.clone();
            let caret = self.current_caret_position;
            let result = get_definitions(t, self, &active, caret);

            let mut result_as_spans: Vec<DocumentSpan> = Vec::new();
            let mut additional_span: Option<DocumentSpan> = None;
            if let Some(locations) = &result.locations {
                result_as_spans = locations.iter().map(location_to_span).collect();
            } else if let Some(location) = &result.location {
                result_as_spans = vec![location_to_span(location)];
            } else if let Some(definition_links) = &result.definition_links {
                let mut origin_range: Option<lsproto::Range> = None;
                result_as_spans = definition_links
                    .iter()
                    .map(|link| {
                        if origin_range.is_some() && (link.origin_selection_range.is_none() || origin_range != link.origin_selection_range) {
                            panic!("multiple different origin ranges in definition links");
                        }
                        origin_range = link.origin_selection_range;
                        let mut context_span: Option<lsproto::Range> = None;
                        if link.target_range != link.target_selection_range && !self.is_strada_server {
                            context_span = Some(link.target_range);
                        }
                        DocumentSpan { uri: link.target_uri.clone(), text_span: link.target_selection_range, context_span }
                    })
                    .collect();
                if let Some(origin_range) = origin_range {
                    if include_original_selection_range {
                        additional_span =
                            Some(DocumentSpan { uri: lsconv::file_name_to_document_uri(&self.active_filename), text_span: origin_range, context_span: None });
                    }
                }
            }

            let baseline = self.get_baseline_for_spans_with_file_contents(
                &result_as_spans,
                BaselineFourslashLocationsOptions {
                    marker: Some(marker_or_range),
                    marker_name: definition_marker.to_string(),
                    additional_span,
                    preserve_result_order: definition_command == GO_TO_SOURCE_DEFINITION_CMD,
                    ..Default::default()
                },
            );
            self.add_result_to_baseline(t, definition_command, &baseline);
        }
    }

    // fourslash.go:3854
    pub(crate) fn lookup_markers_or_get_ranges(&self, t: &T, markers: &[&str]) -> Vec<MarkerOrRange> {
        if markers.is_empty() {
            self.test_data.ranges.iter().map(|r| MarkerOrRange::RangeMarker(r.clone())).collect()
        } else {
            markers
                .iter()
                .map(|marker_name| match self.test_data.marker_positions.get(*marker_name) {
                    Some(marker) => MarkerOrRange::Marker(marker.clone()),
                    None => t.fatal(&format!("Marker '{marker_name}' not found")),
                })
                .collect()
        }
    }

    // fourslash.go:3964
    fn select_line(&mut self, t: &T, line_index: i32) {
        let script = self.get_script_info(&self.active_filename.clone());
        let start = script.line_map.line_starts[line_index as usize];
        let end = if line_index as usize + 1 >= script.line_map.line_starts.len() {
            script.content.len() as TextPos
        } else {
            script.line_map.line_starts[line_index as usize + 1] - 1
        };
        self.select_range(t, TextRange::new(start, end));
    }

    // fourslash.go:3976
    fn select_range(&mut self, t: &T, text_range: TextRange) {
        let script = self.get_script_info(&self.active_filename.clone());
        let start = self.converters.position_to_line_and_character(&script, text_range.pos());
        let end = self.converters.position_to_line_and_character(&script, text_range.end());
        self.go_to_position_impl(t, start);
        self.selection_end = Some(end);
    }

    // fourslash.go:3984
    fn get_selection(&self) -> TextRange {
        let script = self.get_script_info(&self.active_filename);
        match self.selection_end {
            None => TextRange::new(
                self.converters.line_and_character_to_position(script.clone(), self.current_caret_position),
                self.converters.line_and_character_to_position(script, self.current_caret_position),
            ),
            Some(selection_end) => TextRange::new(
                self.converters.line_and_character_to_position(script.clone(), self.current_caret_position),
                self.converters.line_and_character_to_position(script, selection_end),
            ),
        }
    }

    // Updates f.currentCaretPosition
    // fourslash.go:3999
    pub(crate) fn apply_text_edits(&mut self, t: &T, mut edits: Vec<lsproto::TextEdit>) -> i32 {
        let script = self.get_script_info(&self.active_filename.clone());
        edits.sort_by(|a, b| {
            let a_start = self.converters.line_and_character_to_position(script.clone(), a.range.start);
            let b_start = self.converters.line_and_character_to_position(script.clone(), b.range.start);
            a_start.cmp(&b_start)
        });

        let mut total_offset = 0;
        let mut current_caret_position = self.converters.line_and_character_to_position(script.clone(), self.current_caret_position);
        // Apply edits in reverse order to avoid affecting the positions of earlier edits.
        for edit in edits.iter().rev() {
            // (Go's `script` is the shared *scriptInfo: positions are converted against the edited text.)
            let script = self.get_script_info(&self.active_filename.clone());
            let start = self.converters.line_and_character_to_position(script.clone(), edit.range.start);
            let end = self.converters.line_and_character_to_position(script, edit.range.end);
            let active = self.active_filename.clone();
            self.edit_script_and_update_markers(t, &active, start, end, &edit.new_text);

            let delta = edit.new_text.len() as i32 - (end - start);
            if start <= current_caret_position {
                if end <= current_caret_position {
                    // The entirety of the edit span falls before the caret position, shift the caret accordingly
                    current_caret_position += delta;
                } else {
                    // The span being replaced includes the caret position, place the caret at the beginning of the span
                    current_caret_position = start;
                }
            }
            total_offset += delta;
        }
        let script = self.get_script_info(&self.active_filename.clone());
        self.current_caret_position = self.converters.position_to_line_and_character(&script, current_caret_position);
        total_offset
    }

    // fourslash.go:4037
    fn replace_worker(&mut self, t: &T, start: i32, length: i32, text: &str) {
        t.helper();
        let active = self.active_filename.clone();
        self.edit_script_and_update_markers(t, &active, start, start + length, text);
        // f.checkPostEditInvariants() // !!! do we need this?
    }

    // Inserts the text currently at the caret position character by character, as if the user typed it.
    // fourslash.go:4044
    fn type_text(&mut self, t: &T, text: &str) {
        // temprorary -- this disables tests failing if format crashes; this unblocks unrelated tests such as codefixes
        self.report_format_on_type_crash = false;
        let r = crate::go::run(|| {
            let selection = self.get_selection();
            self.replace_worker(t, selection.pos(), selection.end() - selection.pos(), "");

            let mut total_size = 0;

            let script = self.get_script_info(&self.active_filename.clone());
            let mut offset = self.converters.line_and_character_to_position(script, self.current_caret_position);
            while total_size < text.len() {
                let r = text[total_size..].chars().next().unwrap();
                let size = r.len_utf8();
                let active = self.active_filename.clone();
                self.edit_script_and_update_markers(t, &active, offset, offset, &text[total_size..total_size + size]);

                total_size += size;
                offset += size as i32;
                let script = self.get_script_info(&active);
                self.current_caret_position = self.converters.position_to_line_and_character(&script, offset);

                // Handle post-keystroke formatting
                if self.state_enable_formatting {
                    let params = lsproto::DocumentOnTypeFormattingParams {
                        text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&active) },
                        position: self.current_caret_position,
                        ch: r.to_string(),
                        options: self.user_preferences.format_code_settings.to_ls_format_options(),
                    };
                    let result = self.send_request_and_baseline_worker(t, lsproto::TEXT_DOCUMENT_ON_TYPE_FORMATTING_INFO, params, false);
                    if let Some(text_edits) = result.text_edits {
                        offset += self.apply_text_edits(t, text_edits);
                    }
                }
            }

            // f.checkPostEditInvariants() // !!! do we need this?
        });
        self.report_format_on_type_crash = true;
        crate::go::resume(r);
    }

    // Edits the script and updates marker and range positions accordingly.
    // This does not update the current caret position.
    // fourslash.go:4087
    pub(crate) fn edit_script_and_update_markers(&mut self, t: &T, file_name: &str, edit_start: i32, edit_end: i32, new_text: &str) {
        self.edit_script_and_update_markers_worker(t, file_name, &[TextChange { text_range: TextRange::new(edit_start, edit_end), new_text: new_text.to_string() }]);
    }

    // fourslash.go:4091
    // Go updates the shared *Marker / *RangeMarker objects in place. The Rust markers are shared `Arc`s, so the
    // edited ones are replaced by updated copies everywhere the test data refers to them (markers, marker
    // positions, ranges and the ranges' markers); copies a test took out earlier keep the old positions.
    pub(crate) fn edit_script_and_update_markers_worker(&mut self, t: &T, file_name: &str, changes: &[TextChange]) {
        // Sort changes by position (ascending) so we can apply in reverse
        let mut sorted_changes = changes.to_vec();
        sorted_changes.sort_by_key(|c| c.pos());

        // Apply changes in reverse order to preserve positions of earlier changes
        for change in sorted_changes.iter().rev() {
            let edit_start = change.pos();
            let edit_end = change.end();
            let script = self.edit_script(t, file_name, change);
            let mut replaced: FxHashMap<*const Marker, Arc<Marker>> = FxHashMap::default();
            for marker in self.test_data.markers.iter_mut() {
                if marker.file_name() == file_name {
                    let mut updated = (**marker).clone();
                    updated.position = update_position(updated.position, edit_start, edit_end, &change.new_text);
                    updated.ls_position = self.converters.position_to_line_and_character(&script, updated.position);
                    let updated = Arc::new(updated);
                    replaced.insert(Arc::as_ptr(marker), updated.clone());
                    *marker = updated;
                }
            }
            for (_, marker) in self.test_data.marker_positions.iter_mut() {
                if let Some(updated) = replaced.get(&Arc::as_ptr(marker)) {
                    *marker = updated.clone();
                }
            }
            for range_marker in self.test_data.ranges.iter_mut() {
                let relink = range_marker.marker.as_ref().and_then(|m| replaced.get(&Arc::as_ptr(m)).cloned());
                if range_marker.file_name() == file_name || relink.is_some() {
                    let mut updated = (**range_marker).clone();
                    if let Some(m) = relink {
                        updated.marker = Some(m);
                    }
                    if updated.file_name() == file_name {
                        let start = update_position(updated.range.pos(), edit_start, edit_end, &change.new_text);
                        let end = update_position(updated.range.end(), edit_start, edit_end, &change.new_text);
                        updated.range = TextRange::new(start, end);
                        updated.ls_range = self.converters.converters.to_lsp_range(&script, updated.range).0;
                    }
                    *range_marker = Arc::new(updated);
                }
            }
        }
        self.ranges_by_text = OnceLock::new();
    }

    // fourslash.go:4133
    pub(crate) fn from_lsp_range(&self, script: &ScriptInfo, r: lsproto::Range) -> TextRange {
        let ranges = self.converters.converters.from_lsp_range(script.clone(), r, Feature::All);
        if ranges.len() != 1 {
            return TextRange::default();
        }
        ranges[0].span
    }

    // fourslash.go:4141 (returns a copy of the edited script info)
    fn edit_script(&mut self, t: &T, file_name: &str, change: &TextChange) -> ScriptInfo {
        let Some(mut script) = self.get_or_load_script_info(file_name) else {
            panic!("Script info for file {file_name} not found");
        };
        let (change_range, _) = self.converters.converters.to_lsp_range(&script, TextRange::new(change.pos(), change.end()));
        script.edit_content(change.clone());
        self.script_infos.write().unwrap().insert(file_name.to_string(), script.clone());
        if let Err(err) = self.vfs.write_file(file_name, &script.content) {
            t.fatal(&format!("failed to write to VFS for {file_name}: {err}"));
        }
        self.send_notification(
            t,
            lsproto::TEXT_DOCUMENT_DID_CHANGE_INFO,
            lsproto::DidChangeTextDocumentParams {
                text_document: lsproto::VersionedTextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(file_name), version: script.version },
                content_changes: vec![lsproto::TextDocumentContentChangePartialOrWholeDocument {
                    partial: Some(lsproto::TextDocumentContentChangePartial { range: change_range, text: change.new_text.clone(), ..Default::default() }),
                    ..Default::default()
                }],
            },
        );
        script
    }

    // fourslash.go:4166 (getScriptInfo returning nil for an unknown file)
    pub(crate) fn try_get_script_info(&self, file_name: &str) -> Option<ScriptInfo> {
        self.script_infos.read().unwrap().get(file_name).cloned()
    }

    // fourslash.go:4170
    fn get_or_load_script_info(&mut self, file_name: &str) -> Option<ScriptInfo> {
        if let Some(script) = self.try_get_script_info(file_name) {
            return Some(script);
        }
        if let Some(content) = self.vfs.read_file(file_name) {
            let script = new_script_info(file_name, &content);
            self.script_infos.write().unwrap().insert(file_name.to_string(), script.clone());
            return Some(script);
        }
        None
    }

    // fourslash.go:4192
    fn get_quick_info_at_current_position(&mut self, t: &T) -> Option<lsproto::Hover> {
        let params = lsproto::HoverParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_HOVER_INFO, params);
        result.hover
    }

    // fourslash.go:4203
    fn verify_hover_content(
        &self,
        t: &T,
        actual: &lsproto::MarkupContentOrStringOrMarkedStringWithLanguageOrMarkedStrings,
        expected_text: &str,
        expected_documentation: &str,
        prefix: &str,
    ) {
        match &actual.markup_content {
            Some(markup_content) => self.verify_hover_markdown(t, &markup_content.value, expected_text, expected_documentation, prefix),
            None => t.fatal(&format!("{prefix}Expected markup content, got: {actual:?}")),
        }
    }

    // fourslash.go:4218
    fn verify_hover_markdown(&self, t: &T, actual: &str, expected_text: &str, expected_documentation: &str, prefix: &str) {
        let expected = format!("```typescript\n{expected_text}\n```\n{expected_documentation}");
        assert_deep_equal(t, &actual.to_string(), &expected, &format!("{prefix}Hover markdown content mismatch"));
    }

    // fourslash.go:4241
    fn quick_info_is_empty(&mut self, t: &T) -> (bool, Option<lsproto::Hover>) {
        let hover = self.get_quick_info_at_current_position(t);
        match hover {
            None => (true, None),
            Some(h) if h.contents.markup_content.is_none() && h.contents.marked_strings.is_none() && h.contents.string.is_none() => (true, None),
            Some(h) => (false, Some(h)),
        }
    }

    // fourslash.go:4650
    pub(crate) fn get_current_position_prefix(&self) -> String {
        if let Some(name) = &self.last_known_marker_name {
            return format!("At marker '{name}': ");
        }
        format!("At position {}(Ln {}, Col {}): ", self.active_filename, self.current_caret_position.line, self.current_caret_position.character)
    }

    // fourslash.go:5448 (verifyDiagnostics)
    fn verify_diagnostics_impl(&mut self, t: &T, expected: &[lsproto::Diagnostic], filter_diagnostics: impl Fn(&lsproto::Diagnostic) -> bool) {
        let actual_diagnostics = self.get_diagnostics(t, &self.active_filename.clone());
        let actual_diagnostics: Vec<lsproto::Diagnostic> = actual_diagnostics.into_iter().filter(|d| filter_diagnostics(d)).collect();
        let empty_range = lsproto::Range::default();
        let mut expected_with_ranges = Vec::with_capacity(expected.len());
        for diag in expected {
            if diag.range == empty_range {
                let ranges_in_file = self.get_ranges_in_file(&self.active_filename);
                if ranges_in_file.is_empty() {
                    t.fatal(&format!("No ranges found in file {} to assign to diagnostic with empty range", self.active_filename));
                }
                let mut diag_with_range = diag.clone();
                diag_with_range.range = ranges_in_file[0].ls_range;
                expected_with_ranges.push(diag_with_range);
            } else {
                expected_with_ranges.push(diag.clone());
            }
        }
        if actual_diagnostics.is_empty() && expected_with_ranges.is_empty() {
            return;
        }
        // diagnosticsIgnoreOpts: ignorePaths(".Severity", ".Source", ".RelatedInformation")
        let ignore = |d: &lsproto::Diagnostic| lsproto::Diagnostic { severity: None, source: None, related_information: None, ..d.clone() };
        let actual: Vec<lsproto::Diagnostic> = actual_diagnostics.iter().map(ignore).collect();
        let expected: Vec<lsproto::Diagnostic> = expected_with_ranges.iter().map(ignore).collect();
        assert_deep_equal(t, &actual, &expected, "Diagnostics do not match expected");
    }

    // fourslash.go:5472
    pub(crate) fn get_diagnostics(&mut self, t: &T, file_name: &str) -> Vec<lsproto::Diagnostic> {
        let params = lsproto::DocumentDiagnosticParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(file_name) },
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_DIAGNOSTIC_INFO, params);
        if let Some(report) = result.full_document_diagnostic_report {
            return report.items;
        }
        Vec::new()
    }
}

// fourslash.go:1665 (cmp.Diff's report is replaced by both values' Debug forms)
pub(crate) fn assert_deep_equal<V: PartialEq + std::fmt::Debug>(t: &T, actual: &V, expected: &V, prefix: &str) {
    t.helper();
    if actual != expected {
        t.fatal(&format!("{prefix}:\n-actual:   {actual:?}\n+expected: {expected:?}"));
    }
}

// fourslash.go:3036
// renderVSContainerElement renders a VS rich-content container element (icon + colorized runs,
// possibly nested for the stacked display-line/documentation shape) into readable baseline lines.
fn render_vs_container_element(el: &lsproto::VSContainerElement, indent: &str) -> Vec<String> {
    let mut lines = vec![format!("{indent}ContainerElement (Style={})", el.style)];
    let child_indent = format!("{indent}  ");
    for child in &el.elements {
        if let Some(image_element) = &child.image_element {
            let image_id = &image_element.image_id;
            lines.push(format!("{child_indent}ImageElement {{ Guid: {}, Id: 0x{:X} }}", image_id.guid, image_id.id));
        } else if let Some(classified_text_element) = &child.classified_text_element {
            lines.push(format!("{child_indent}ClassifiedTextElement"));
            for run in &classified_text_element.runs {
                lines.push(format!("{child_indent}  [{}] {}", run.classification_type_name, crate::go::quote(&run.text)));
            }
        } else if let Some(container_element) = &child.container_element {
            lines.extend(render_vs_container_element(container_element, &child_indent));
        } else {
            lines.push(format!("{child_indent}<empty union element>"));
        }
    }
    lines
}

// fourslash.go:3058
fn append_lines_for_marked_string_with_language(mut result: Vec<String>, ms: &lsproto::MarkedStringWithLanguage) -> Vec<String> {
    result.push(format!("```{}", ms.language));
    result.push(ms.value.clone());
    result.push("```".to_string());
    result
}

// The getTooltipLines body shared by VerifyBaselineHover and VerifyBaselineHoverWithVerbosity (fourslash.go:2955).
fn hover_tooltip_lines(item: &lsproto::Hover) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();

    if let Some(markup_content) = &item.contents.markup_content {
        result = markup_content.value.split('\n').map(str::to_string).collect();
    }
    if let Some(s) = &item.contents.string {
        result = s.split('\n').map(str::to_string).collect();
    }
    if let Some(ms) = &item.contents.marked_string_with_language {
        result = append_lines_for_marked_string_with_language(result, ms);
    }
    if let Some(marked_strings) = &item.contents.marked_strings {
        for ms in marked_strings {
            if let Some(msl) = &ms.marked_string_with_language {
                result = append_lines_for_marked_string_with_language(result, msl);
            } else {
                result.push(ms.string.clone().unwrap());
            }
        }
    }

    result
}

// fourslash.go:3066
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HoverWithVerbosity {
    pub(crate) hover: Option<lsproto::Hover>,
    pub(crate) verbosity_level: i32,
}

// hoverContentString extracts the text content from a hover response for comparison.
// fourslash.go:3071
fn hover_content_string(hover: Option<&lsproto::Hover>) -> String {
    let Some(hover) = hover else {
        return String::new();
    };
    if let Some(markup_content) = &hover.contents.markup_content {
        return markup_content.value.clone();
    }
    if let Some(s) = &hover.contents.string {
        return s.clone();
    }
    String::new()
}

// fourslash.go:4122
fn update_position(pos: i32, edit_start: i32, edit_end: i32, new_text: &str) -> i32 {
    if pos <= edit_start {
        return pos;
    }
    // If inside the edit, return -1 to mark as invalid
    if pos < edit_end {
        return -1;
    }
    pos + new_text.len() as i32 - (edit_end - edit_start)
}

// fourslash.go:5485
fn is_suggestion_diagnostic(diag: &lsproto::Diagnostic) -> bool {
    diag.severity == Some(lsproto::DiagnosticSeverity::Hint)
}

// The JSON Go's encoding of `markerAndItem[T]` produces (`{"marker": {...}, "item": ...}`; Marker's exported
// fields: Position, LSPosition, Name, Data).
pub(crate) fn marker_and_item_to_json<I>(m: &MarkerAndItem<I>, item_to_json: impl Fn(&I) -> Value) -> Value {
    let mut o = OrderedMap::default();
    o.insert("marker".to_string(), marker_to_json(&m.marker));
    o.insert("item".to_string(), item_to_json(&m.item));
    Value::Object(o)
}

pub(crate) fn marker_to_json(marker: &Marker) -> Value {
    let mut o = OrderedMap::default();
    o.insert("Position".to_string(), Value::Number(marker.position as f64));
    o.insert("LSPosition".to_string(), marker.ls_position.to_json());
    o.insert("Name".to_string(), marker.name.as_ref().map_or(Value::Null, |n| Value::String(n.clone())));
    // json/v2 marshals a nil map as {}.
    o.insert("Data".to_string(), Value::Object(crate::go::json_object(&marker.data)));
    Value::Object(o)
}

// fourslash.go:5514
pub(crate) struct FourslashDiagnostic {
    pub(crate) file: Arc<FourslashDiagnosticFile>,
    pub(crate) loc: TextRange,
    pub(crate) code: i32,
    pub(crate) category: tsrs_diagnostics::Category,
    pub(crate) message: String,
    pub(crate) related_diagnostics: Vec<FourslashDiagnostic>,
    pub(crate) reports_unnecessary: bool,
    pub(crate) reports_deprecated: bool,
}

// fourslash.go:5525 (file = harnessutil.TestFile{UnitName, Content}; the ECMA line map is computed eagerly)
pub(crate) struct FourslashDiagnosticFile {
    pub(crate) file_name: String,
    pub(crate) content: String,
    pub(crate) ecma_line_map: Vec<TextPos>,
}

fn new_fourslash_diagnostic_file(unit_name: &str, content: &str) -> Arc<FourslashDiagnosticFile> {
    Arc::new(FourslashDiagnosticFile {
        file_name: unit_name.to_string(),
        content: content.to_string(),
        ecma_line_map: tsrs_core::compute_ecma_line_starts(content).to_vec(),
    })
}

impl FourslashTest {
    // fourslash.go:5597
    fn to_diagnostic(&self, script_info: &ScriptInfo, lsp_diagnostic: &lsproto::Diagnostic) -> FourslashDiagnostic {
        use tsrs_diagnostics::Category;
        let category = match lsp_diagnostic.severity.unwrap() {
            lsproto::DiagnosticSeverity::Error => Category::Error,
            lsproto::DiagnosticSeverity::Warning => Category::Warning,
            lsproto::DiagnosticSeverity::Information => Category::Message,
            lsproto::DiagnosticSeverity::Hint => Category::Suggestion,
            _ => Category::Error,
        };
        let code = lsp_diagnostic.code.as_ref().unwrap().integer.unwrap();

        let mut related_diagnostics = Vec::new();
        if let Some(related_information) = &lsp_diagnostic.related_information {
            for info in related_information {
                let Some(related_script_info) = self.try_get_script_info(&info.location.uri.file_name()) else {
                    continue;
                };
                let related_diagnostic = FourslashDiagnostic {
                    file: new_fourslash_diagnostic_file(&related_script_info.file_name, &related_script_info.content),
                    loc: self.from_lsp_range(&related_script_info, info.location.range),
                    code,
                    category,
                    message: info.message.clone(),
                    related_diagnostics: Vec::new(),
                    reports_unnecessary: false,
                    reports_deprecated: false,
                };
                related_diagnostics.push(related_diagnostic);
            }
        }

        FourslashDiagnostic {
            file: new_fourslash_diagnostic_file(&script_info.file_name, &script_info.content),
            loc: self.from_lsp_range(script_info, lsp_diagnostic.range),
            code,
            category,
            message: lsp_diagnostic.message.as_string(),
            related_diagnostics,
            reports_unnecessary: false,
            reports_deprecated: false,
        }
    }
}

// fourslash.go:5647
fn compare_diagnostics(d1: &FourslashDiagnostic, d2: &FourslashDiagnostic) -> i32 {
    let mut c = d1.file.file_name.cmp(&d2.file.file_name) as i32;
    if c != 0 {
        return c;
    }
    c = d1.loc.pos() - d2.loc.pos();
    if c != 0 {
        return c;
    }
    c = d1.loc.end() - d2.loc.end();
    if c != 0 {
        return c;
    }
    c = d1.code - d2.code;
    if c != 0 {
        return c;
    }
    c = d1.message.cmp(&d2.message) as i32;
    if c != 0 {
        return c;
    }
    compare_related_diagnostics(&d1.related_diagnostics, &d2.related_diagnostics)
}

// fourslash.go:5671
fn compare_related_diagnostics(d1: &[FourslashDiagnostic], d2: &[FourslashDiagnostic]) -> i32 {
    let c = d2.len() as i32 - d1.len() as i32;
    if c != 0 {
        return c;
    }
    for i in 0..d1.len() {
        let c = compare_diagnostics(&d1[i], &d2[i]);
        if c != 0 {
            return c;
        }
    }
    0
}

// fourslash.go:3450
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum callHierarchyItemDirection {
    Root,
    Incoming,
    Outgoing,
}

// fourslash.go:3458
#[derive(Clone, PartialEq, Eq, Hash)]
struct callHierarchyItemKey {
    uri: lsproto::DocumentUri,
    range_: lsproto::Range,
    direction: callHierarchyItemDirection,
}

// fourslash.go:3464
fn symbol_kind_to_lowercase(kind: lsproto::SymbolKind) -> String {
    kind.string().to_lowercase()
}

// Go passes a possibly nil `*scriptInfo` and dereferences it when formatting a span.
// fourslash.go:3468
#[allow(clippy::too_many_arguments)]
fn format_call_hierarchy_item(
    t: &T,
    f: &mut FourslashTest,
    file: Option<&ScriptInfo>,
    result: &mut String,
    call_hierarchy_item: &lsproto::CallHierarchyItem,
    direction: callHierarchyItemDirection,
    seen: &mut rustc_hash::FxHashSet<callHierarchyItemKey>,
    prefix: &str,
) {
    let key = callHierarchyItemKey { uri: call_hierarchy_item.uri.clone(), range_: call_hierarchy_item.range, direction };
    let already_seen = !seen.insert(key);

    struct callResult<V> {
        skip: bool,
        seen: bool,
        values: Vec<V>,
    }

    let mut incoming_calls: callResult<lsproto::CallHierarchyIncomingCall> = callResult { skip: false, seen: false, values: Vec::new() };
    let mut outgoing_calls: callResult<lsproto::CallHierarchyOutgoingCall> = callResult { skip: false, seen: false, values: Vec::new() };

    if direction == callHierarchyItemDirection::Outgoing {
        incoming_calls.skip = true;
    } else if already_seen {
        incoming_calls.seen = true;
    } else {
        let incoming_params = lsproto::CallHierarchyIncomingCallsParams { item: call_hierarchy_item.clone(), ..Default::default() };
        let incoming_result = f.send_request(t, lsproto::CALL_HIERARCHY_INCOMING_CALLS_INFO, incoming_params);
        if let Some(values) = incoming_result.call_hierarchy_incoming_calls {
            incoming_calls.values = values;
        }
    }

    if direction == callHierarchyItemDirection::Incoming {
        outgoing_calls.skip = true;
    } else if already_seen {
        outgoing_calls.seen = true;
    } else {
        let outgoing_params = lsproto::CallHierarchyOutgoingCallsParams { item: call_hierarchy_item.clone(), ..Default::default() };
        let outgoing_result = f.send_request(t, lsproto::CALL_HIERARCHY_OUTGOING_CALLS_INFO, outgoing_params);
        if let Some(values) = outgoing_result.call_hierarchy_outgoing_calls {
            outgoing_calls.values = values;
        }
    }

    let trailing_prefix = prefix;
    result.push_str(&format!("{}╭ name: {}\n", prefix, call_hierarchy_item.name));
    result.push_str(&format!("{}├ kind: {}\n", prefix, symbol_kind_to_lowercase(call_hierarchy_item.kind)));
    if let Some(detail) = call_hierarchy_item.detail.as_ref().filter(|d| !d.is_empty()) {
        result.push_str(&format!("{}├ containerName: {}\n", prefix, detail));
    }
    result.push_str(&format!("{}├ file: {}\n", prefix, call_hierarchy_item.uri.file_name()));
    result.push_str(prefix);
    result.push_str("├ span:\n");
    format_call_hierarchy_item_span(f, file, result, call_hierarchy_item.range, &format!("{prefix}│ "), &format!("{prefix}│ "));
    result.push_str(prefix);
    result.push_str("├ selectionSpan:\n");
    format_call_hierarchy_item_span(f, file, result, call_hierarchy_item.selection_range, &format!("{prefix}│ "), &format!("{prefix}│ "));

    // Handle incoming calls
    if incoming_calls.seen {
        if outgoing_calls.skip {
            result.push_str(trailing_prefix);
            result.push_str("╰ incoming: ...\n");
        } else {
            result.push_str(prefix);
            result.push_str("├ incoming: ...\n");
        }
    } else if !incoming_calls.skip {
        if incoming_calls.values.is_empty() {
            if outgoing_calls.skip {
                result.push_str(trailing_prefix);
                result.push_str("╰ incoming: none\n");
            } else {
                result.push_str(prefix);
                result.push_str("├ incoming: none\n");
            }
        } else {
            result.push_str(prefix);
            result.push_str("├ incoming:\n");
            let count = incoming_calls.values.len();
            for (i, incoming_call) in incoming_calls.values.iter().enumerate() {
                let from_file_name = incoming_call.from.uri.file_name();
                let from_file = f.get_or_load_script_info(&from_file_name);
                result.push_str(prefix);
                result.push_str("│ ╭ from:\n");
                format_call_hierarchy_item(t, f, from_file.as_ref(), result, &incoming_call.from, callHierarchyItemDirection::Incoming, seen, &format!("{prefix}│ │ "));
                result.push_str(prefix);
                result.push_str("│ ├ fromSpans:\n");

                let mut from_spans_trailing_prefix = format!("{trailing_prefix}╰ ╰ ");
                if i < count - 1 {
                    from_spans_trailing_prefix = format!("{prefix}│ ╰ ");
                } else if !outgoing_calls.skip && (!outgoing_calls.seen || !outgoing_calls.values.is_empty()) {
                    from_spans_trailing_prefix = format!("{prefix}│ ╰ ");
                }
                format_call_hierarchy_item_spans(f, from_file.as_ref(), result, &incoming_call.from_ranges, &format!("{prefix}│ │ "), &from_spans_trailing_prefix);
            }
        }
    }

    // Handle outgoing calls
    if outgoing_calls.seen {
        result.push_str(trailing_prefix);
        result.push_str("╰ outgoing: ...\n");
    } else if !outgoing_calls.skip {
        if outgoing_calls.values.is_empty() {
            result.push_str(trailing_prefix);
            result.push_str("╰ outgoing: none\n");
        } else {
            result.push_str(prefix);
            result.push_str("├ outgoing:\n");
            let count = outgoing_calls.values.len();
            for (i, outgoing_call) in outgoing_calls.values.iter().enumerate() {
                let to_file_name = outgoing_call.to.uri.file_name();
                let to_file = f.get_or_load_script_info(&to_file_name);
                result.push_str(prefix);
                result.push_str("│ ╭ to:\n");
                format_call_hierarchy_item(t, f, to_file.as_ref(), result, &outgoing_call.to, callHierarchyItemDirection::Outgoing, seen, &format!("{prefix}│ │ "));
                result.push_str(prefix);
                result.push_str("│ ├ fromSpans:\n");

                let mut from_spans_trailing_prefix = format!("{trailing_prefix}╰ ╰ ");
                if i < count - 1 {
                    from_spans_trailing_prefix = format!("{prefix}│ ╰ ");
                }
                format_call_hierarchy_item_spans(f, file, result, &outgoing_call.from_ranges, &format!("{prefix}│ │ "), &from_spans_trailing_prefix);
            }
        }
    }
}

// fourslash.go:3613
fn format_call_hierarchy_item_span(f: &FourslashTest, file: Option<&ScriptInfo>, result: &mut String, span: lsproto::Range, prefix: &str, closing_prefix: &str) {
    let file = file.expect("nil pointer dereference: scriptInfo");
    let start_lc = span.start;
    let end_lc = span.end;
    let start_pos = f.converters.line_and_character_to_position(file.clone(), span.start);
    let end_pos = f.converters.line_and_character_to_position(file.clone(), span.end);

    // Compute line starts for the file
    let line_starts = compute_line_starts(&file.content);
    let content = file.content.as_bytes();

    // Find the line boundaries - expand to full lines
    let mut context_start = start_pos as usize;
    let mut context_end = end_pos as usize;

    // Expand to start of first line
    while context_start > 0 && content[context_start - 1] != b'\n' && content[context_start - 1] != b'\r' {
        context_start -= 1;
    }

    // Expand to end of last line
    while context_end < content.len() && content[context_end] != b'\n' && content[context_end] != b'\r' {
        context_end += 1;
    }

    // Get actual line and character positions for the context
    let context_start_line = start_lc.line as usize;
    let context_end_line = end_lc.line as usize;

    // Calculate line number padding
    let line_num_width = (context_end_line + 1).to_string().len() + 2;

    result.push_str(&format!("{}╭ {}:{}:{}-{}:{}\n", prefix, file.file_name, start_lc.line + 1, start_lc.character + 1, end_lc.line + 1, end_lc.character + 1));

    for line_num in context_start_line..=context_end_line {
        let line_start = line_starts[line_num];
        let mut line_end = file.content.len();
        if line_num + 1 < line_starts.len() {
            line_end = line_starts[line_num + 1];
        }

        // Get the line content, trimming trailing newlines
        let line_content = file.content[line_start..line_end].trim_end_matches(['\r', '\n']);

        // Format with line number
        let line_num_str = format!("{}:", line_num + 1);
        let padded_line_num = " ".repeat(line_num_width - line_num_str.len() - 1) + &line_num_str;
        if line_content.is_empty() {
            result.push_str(&format!("{}│ {}\n", prefix, padded_line_num));
        } else {
            result.push_str(&format!("{}│ {} {}\n", prefix, padded_line_num, line_content));
        }

        // Add selection carets if this line contains part of the span
        if line_num >= start_lc.line as usize && line_num <= end_lc.line as usize {
            let mut sel_start: usize = 0;
            let mut sel_end: usize = line_content.len();

            if line_num == start_lc.line as usize {
                sel_start = start_lc.character as usize;
            }
            if line_num == end_lc.line as usize {
                sel_end = end_lc.character as usize;
            }

            // Don't show carets for empty selections
            let is_empty = start_lc.line == end_lc.line && start_lc.character == end_lc.character;
            if is_empty {
                // For empty selections, show a single "<" character
                let padding = " ".repeat(line_num_width + sel_start);
                result.push_str(&format!("{}│ {}<\n", prefix, padding));
            } else {
                // Calculate selection length (at least 1)
                let mut sel_length = sel_end as isize - sel_start as isize;
                sel_length = sel_length.max(1); // Trim to actual content on the line
                if line_num < end_lc.line as usize {
                    // For lines before the last, trim to line content length
                    if sel_end > line_content.len() {
                        sel_end = line_content.len();
                        sel_length = sel_end as isize - sel_start as isize;
                    }
                }

                let padding = " ".repeat(line_num_width + sel_start);
                let carets = "^".repeat(sel_length.max(0) as usize);
                result.push_str(&format!("{}│ {}{}\n", prefix, padding, carets));
            }
        }
    }

    result.push_str(closing_prefix);
    result.push_str("╰\n");
}

// fourslash.go:3712
fn compute_line_starts(content: &str) -> Vec<usize> {
    let mut line_starts = vec![0];
    for (i, ch) in content.char_indices() {
        if ch == '\n' {
            line_starts.push(i + 1);
        }
    }
    line_starts
}

// fourslash.go:3723
fn format_call_hierarchy_item_spans(f: &FourslashTest, file: Option<&ScriptInfo>, result: &mut String, spans: &[lsproto::Range], prefix: &str, trailing_prefix: &str) {
    for (i, span) in spans.iter().enumerate() {
        let mut closing_prefix = prefix;
        if i == spans.len() - 1 {
            closing_prefix = trailing_prefix;
        }
        format_call_hierarchy_item_span(f, file, result, *span, prefix, closing_prefix);
    }
}

impl FourslashTest {
    // fourslash.go:5086
    fn get_path_updater(&self, old_path: &str, new_path: &str) -> impl Fn(&str) -> Option<String> {
        let use_case_sensitive_file_names = self.vfs.use_case_sensitive_file_names();
        let (old_path, new_path) = (old_path.to_string(), new_path.to_string());
        move |path: &str| {
            let compare_options = tspath::ComparePathsOptions { use_case_sensitive_file_names, ..Default::default() };
            if tspath::compare_paths(path, &old_path, &compare_options) == 0 {
                return Some(new_path.clone());
            }
            if tspath::starts_with_directory(path, &old_path, use_case_sensitive_file_names) {
                return Some(format!("{}{}", new_path, &path[old_path.len()..]));
            }
            None
        }
    }

    // fourslash.go:5101
    fn rename_file_or_directory(&mut self, t: &T, old_path: &str, new_path: &str) {
        t.helper();

        let path_updater = self.get_path_updater(old_path, new_path);

        // Collect all file paths that need to be renamed. (Go: a map, iterated in random order.)
        let mut old_file_names: OrderedMap<String, ()> = OrderedMap::default();
        if self.vfs.read_file(old_path).is_some() {
            old_file_names.insert(old_path.to_string(), ());
        } else {
            for path in get_accessible_file_paths(&*self.vfs, old_path) {
                old_file_names.insert(path, ());
            }
        }
        if old_file_names.is_empty() {
            t.fatal(&format!("rename source {old_path} did not exist in test environment"));
        }

        // !!! TODO: handle overwrites if we need to.
        // For each file: close if open, update script infos, write to VFS at new path, and collect file-watch events.
        let mut file_events: Vec<lsproto::FileEvent> = Vec::with_capacity(old_file_names.len() * 2);
        let mut reopen_at_new_path: OrderedMap<String, String> = OrderedMap::default(); // newFileName -> content, for files that were open
        for old_file_name in old_file_names.keys() {
            let Some(new_file_name) = path_updater(old_file_name) else {
                t.fatal(&format!("failed to compute renamed path for {old_file_name}"));
            };

            // Send didClose for open files; get content from the old script info.
            if self.open_files.contains_key(old_file_name) {
                let script = self.get_script_info(old_file_name);
                reopen_at_new_path.insert(new_file_name.clone(), script.content.clone());
                self.send_notification(
                    t,
                    lsproto::TEXT_DOCUMENT_DID_CLOSE_INFO,
                    lsproto::DidCloseTextDocumentParams { text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(old_file_name) } },
                );
                self.open_files.shift_remove(old_file_name);
            }

            {
                let mut script_infos = self.script_infos.write().unwrap();
                let old_content = script_infos.get(old_file_name).expect("nil pointer dereference: scriptInfo").content.clone();
                script_infos.insert(new_file_name.clone(), new_script_info(&new_file_name, &old_content));
                script_infos.remove(old_file_name);
            }

            // Write renamed file to VFS.
            let Some(content) = self.vfs.read_file(old_file_name) else {
                t.fatal(&format!("failed to read content for {old_file_name} during rename to {new_file_name}"));
            };
            if let Err(err) = self.vfs.write_file(&new_file_name, &content) {
                t.fatal(&format!("failed to write renamed file {new_file_name}: {err}"));
            }

            file_events.push(lsproto::FileEvent { uri: lsconv::file_name_to_document_uri(old_file_name), type_: lsproto::FileChangeType::Deleted });
            file_events.push(lsproto::FileEvent { uri: lsconv::file_name_to_document_uri(&new_file_name), type_: lsproto::FileChangeType::Created });
        }

        // Remove the old path from VFS and notify the server of all file-system changes.
        if let Err(err) = self.vfs.remove(old_path) {
            t.fatal(&format!("failed to remove old path {old_path}: {err}"));
        }
        self.send_notification(t, lsproto::WORKSPACE_DID_CHANGE_WATCHED_FILES_INFO, lsproto::DidChangeWatchedFilesParams { changes: file_events });

        // Reopen files that were previously open at their new paths.
        for (new_file_name, content) in &reopen_at_new_path {
            self.send_notification(
                t,
                lsproto::TEXT_DOCUMENT_DID_OPEN_INFO,
                lsproto::DidOpenTextDocumentParams {
                    text_document: lsproto::TextDocumentItem {
                        uri: lsconv::file_name_to_document_uri(new_file_name),
                        language_id: get_language_kind(new_file_name),
                        text: content.clone(),
                        ..Default::default()
                    },
                },
            );
            self.open_files.insert(new_file_name.clone(), ());
        }

        // Update active filename if it was under the renamed path.
        if let Some(updated_active) = path_updater(&self.active_filename) {
            self.active_filename = updated_active;
        }
    }
}
