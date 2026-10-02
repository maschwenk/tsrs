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

// fourslash.go:1140 (items are *lsproto.CompletionItem | string). `exact` and `unsorted` are `Option` because a nil
// and an empty Go slice mean different things there (an empty one requires an empty list).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionsExpectedItems {
    pub includes: Vec<Any>,
    pub excludes: Vec<String>,
    pub exact: Option<Vec<Any>>,
    pub unsorted: Option<Vec<Any>>,
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
        t.helper();
        let mut list: Option<lsproto::CompletionList> = None;
        match &marker_input {
            Any::String(marker) => {
                self.go_to_marker(t, marker);
                list = self.verify_completions_worker(t, expected.as_ref());
            }
            Any::Marker(marker) => {
                self.go_to_marker_impl(t, &MarkerOrRange::Marker(marker.clone()));
                list = self.verify_completions_worker(t, expected.as_ref());
            }
            Any::StringSlice(markers) => {
                for marker_name in markers {
                    self.go_to_marker(t, marker_name);
                    self.verify_completions_worker(t, expected.as_ref());
                }
            }
            Any::MarkerSlice(markers) => {
                for marker in markers {
                    self.go_to_marker_impl(t, &MarkerOrRange::Marker(marker.clone()));
                    self.verify_completions_worker(t, expected.as_ref());
                }
            }
            Any::Nil => {
                list = self.verify_completions_worker(t, expected.as_ref());
            }
            _ => t.fatal(&format!("Invalid marker input type: {}. Expected string, *Marker, []string, or []*Marker.", marker_input.type_name())),
        }
        self.verify_completions_actions(list)
    }

    // fourslash.go:1191
    fn verify_completions_actions(&self, list: Option<lsproto::CompletionList>) -> VerifyCompletionsResult {
        // Go's closures share the list; each Rust closure gets its own copy.
        let list_for_no_action = list.clone();
        // (Go dereferences a nil list and a nil expected action, which panics; so do these.)
        fn find(list: &Option<lsproto::CompletionList>, action: &CompletionsExpectedCodeAction) -> Option<lsproto::CompletionItem> {
            let list = list.as_ref().expect("runtime error: invalid memory address or nil pointer dereference");
            list.items
                .iter()
                .find(|item| {
                    if item.label != action.name {
                        return false;
                    }
                    let Some(data) = &item.data else {
                        return false;
                    };
                    let Some(auto_import) = &data.auto_import else {
                        return false;
                    };
                    auto_import.module_specifier == action.source
                })
                .cloned()
        }
        VerifyCompletionsResult {
            and_apply_code_action: Box::new(move |f: &mut FourslashTest, t: &T, expected_action: Option<CompletionsExpectedCodeAction>| {
                let expected_action = expected_action.expect("runtime error: invalid memory address or nil pointer dereference");
                let Some(item) = find(&list, &expected_action) else {
                    t.fatal(&format!("Code action '{}' from source '{}' not found in completions.", expected_action.name, expected_action.source));
                };
                // Detail and AdditionalTextEdits for auto-import items are populated by
                // completionItem/resolve, not in the initial completion list.
                let item = f.resolve_completion_item_impl(t, item);
                crate::go::assert::check(
                    t,
                    item.detail.as_ref().is_some_and(|d| d.contains(&expected_action.description)),
                    "Completion item detail does not contain expected description.",
                );
                let Some(additional_text_edits) = item.additional_text_edits else {
                    panic!("runtime error: invalid memory address or nil pointer dereference");
                };
                f.apply_text_edits(t, additional_text_edits);
                crate::go::assert::equal(
                    t,
                    f.get_script_info(&f.active_filename.clone()).content.as_str(),
                    expected_action.new_file_content.as_str(),
                    &format!("File content after applying code action '{}' did not match expected content.", expected_action.name),
                );
            }),
            and_has_no_code_action: Box::new(move |_f: &mut FourslashTest, t: &T, unexpected_action: Option<CompletionsExpectedCodeAction>| {
                let unexpected_action = unexpected_action.expect("runtime error: invalid memory address or nil pointer dereference");
                if find(&list_for_no_action, &unexpected_action).is_some() {
                    t.fatal(&format!("Unexpected code action '{}' from source '{}' found in completions.", unexpected_action.name, unexpected_action.source));
                }
            }),
        }
    }

    // fourslash.go:1232
    fn verify_completions_worker(&mut self, t: &T, expected: Option<&CompletionsExpectedList>) -> Option<lsproto::CompletionList> {
        t.helper();
        let user_preferences = expected.and_then(|e| e.user_preferences.clone());
        let prefix = self.get_current_position_prefix();
        let list = self.get_completions_impl(t, user_preferences);
        self.verify_completions_result(t, list.as_ref(), expected, &prefix);
        list
    }

    // fourslash.go:1245
    pub fn get_completions(&mut self, t: &T, user_preferences: Option<lsutil::UserPreferences>) -> Option<lsproto::CompletionList> {
        t.helper();
        self.get_completions_impl(t, user_preferences)
    }

    // fourslash.go:1250
    pub fn verify_jsdoc_completion(&mut self, t: &T, marker_input: Any, expected_offset: i32, expected_text: &str, generate_return_in_doc_template: Option<bool>) {
        t.helper();
        self.go_to_marker_input(t, &marker_input);

        let mut user_preferences: Option<lsutil::UserPreferences> = None;
        if let Some(generate_return_in_doc_template) = generate_return_in_doc_template {
            let mut prefs = lsutil::new_default_user_preferences();
            prefs.generate_return_in_doc_template = tsrs_core::bool_to_tristate(generate_return_in_doc_template);
            user_preferences = Some(prefs);
        }

        let mut list = self.get_completions_impl(t, user_preferences.clone());
        let mut item = find_jsdoc_completion_item(list.as_ref());
        if item.is_none() {
            let script = self.get_script_info(&self.active_filename.clone());
            let insert_start = self.converters.line_and_character_to_position(script.clone(), self.current_caret_position);
            self.insert(t, "/**");
            list = self.get_completions_impl(t, user_preferences);
            item = find_jsdoc_completion_item(list.as_ref());
            let active = self.active_filename.clone();
            self.edit_script_and_update_markers(t, &active, insert_start, insert_start + 3, "");
            // (Go converts with the script info captured before the edits.)
            self.current_caret_position = self.converters.position_to_line_and_character(&script, insert_start);
        }
        let Some(list) = list else {
            t.fatal(&format!("{}Expected JSDoc completion, got nil completion list.", self.get_current_position_prefix()));
        };
        let Some(item) = item else {
            t.fatal(&format!("{}Expected JSDoc completion item, got {:?}.", self.get_current_position_prefix(), list.items));
        };
        let Some(insert_replace_edit) = item.text_edit.as_ref().and_then(|e| e.insert_replace_edit.as_ref()) else {
            t.fatal(&format!("{}Expected JSDoc completion to have insert/replace edit, got {:?}.", self.get_current_position_prefix(), item.text_edit));
        };
        crate::go::assert::equal(t, insert_replace_edit.new_text.as_str(), expected_text, &self.get_current_position_prefix());
        let _ = expected_offset; // The completion path uses snippet placeholders for caret placement.
    }

    // fourslash.go:1285
    pub fn verify_no_jsdoc_completion(&mut self, t: &T, marker_input: Any) {
        t.helper();
        self.go_to_marker_input(t, &marker_input);

        let list = self.get_completions_impl(t, None /*userPreferences*/);
        if find_jsdoc_completion_item(list.as_ref()).is_some() {
            t.fatal(&format!("{}Did not expect JSDoc completion item.", self.get_current_position_prefix()));
        }

        let script = self.get_script_info(&self.active_filename.clone());
        let insert_start = self.converters.line_and_character_to_position(script.clone(), self.current_caret_position);
        self.insert(t, "/**");
        let list = self.get_completions_impl(t, None /*userPreferences*/);
        let item = find_jsdoc_completion_item(list.as_ref());
        let active = self.active_filename.clone();
        self.edit_script_and_update_markers(t, &active, insert_start, insert_start + 3, "");
        self.current_caret_position = self.converters.position_to_line_and_character(&script, insert_start);
        if item.is_some() {
            t.fatal(&format!("{}Did not expect JSDoc completion item.", self.get_current_position_prefix()));
        }
    }

    // fourslash.go:1327
    fn get_completions_impl(&mut self, t: &T, user_preferences: Option<lsutil::UserPreferences>) -> Option<lsproto::CompletionList> {
        t.helper();
        let params = lsproto::CompletionParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            context: Some(lsproto::CompletionContext::default()),
            ..Default::default()
        };
        let mut reset: Option<DoneFn> = None;
        if let Some(user_preferences) = user_preferences {
            let mut preferences = user_preferences;
            preferences.format_code_settings = self.user_preferences.format_code_settings.clone();
            reset = Some(self.configure_with_reset(t, preferences));
        }
        let mut result = lsproto::CompletionResponse::default();
        let r = crate::go::run(|| {
            result = self.send_request(t, lsproto::TEXT_DOCUMENT_COMPLETION_INFO, params);
        });
        if let Some(reset) = reset {
            reset(self, t);
        }
        crate::go::resume(r);
        // For performance, the server may return unsorted completion lists.
        // The client is expected to sort them by SortText and then by Label.
        // We are the client here.
        if let Some(list) = &mut result.list {
            list.items.sort_by(|a, b| tsrs_ls::compare_completion_entries(a, b).cmp(&0));
        }
        result.list
    }

    // fourslash.go:1354
    fn verify_completions_result(&mut self, t: &T, actual: Option<&lsproto::CompletionList>, expected: Option<&CompletionsExpectedList>, prefix: &str) {
        let Some(actual) = actual else {
            if !is_empty_expected_list(expected) {
                t.fatal(&format!("{prefix}Expected completion list but got nil."));
            }
            return;
        };
        let Some(expected) = expected else {
            if actual.items.is_empty() {
                return;
            }
            t.fatal(&format!("{prefix}Expected nil completion list but got non-nil: {actual:?}"));
        };
        crate::go::assert::equal(t, actual.is_incomplete, expected.is_incomplete, &format!("{prefix}IsIncomplete mismatch"));
        verify_completions_item_defaults(t, actual.item_defaults.as_ref(), expected.item_defaults.as_ref(), &format!("{prefix}ItemDefaults mismatch: "));
        // (Go dereferences expected.Items, which panics when it is nil.)
        let Some(items) = &expected.items else {
            panic!("runtime error: invalid memory address or nil pointer dereference");
        };
        self.verify_completions_items(t, prefix, &actual.items, items);
    }

    // fourslash.go:1419
    fn verify_completions_items(&mut self, t: &T, prefix: &str, actual: &[lsproto::CompletionItem], expected: &CompletionsExpectedItems) {
        if let Some(exact) = &expected.exact {
            if !expected.includes.is_empty() {
                t.fatal(&format!("{prefix}Expected exact completion list but also specified 'includes'."));
            }
            if !expected.excludes.is_empty() {
                t.fatal(&format!("{prefix}Expected exact completion list but also specified 'excludes'."));
            }
            if expected.unsorted.is_some() {
                t.fatal(&format!("{prefix}Expected exact completion list but also specified 'unsorted'."));
            }
            if actual.len() != exact.len() {
                t.fatal(&format!("{prefix}Expected {} exact completion items but got {}.", exact.len(), actual.len()));
            }
            if !actual.is_empty() {
                self.verify_completions_are_exactly(t, prefix, actual, exact);
            }
            return;
        }
        let mut name_to_actual_items: OrderedMap<String, Vec<lsproto::CompletionItem>> = OrderedMap::default();
        for item in actual {
            match name_to_actual_items.get_mut(&item.label) {
                Some(items) => items.push(item.clone()),
                None => {
                    name_to_actual_items.insert(item.label.clone(), vec![item.clone()]);
                }
            }
        }
        if let Some(unsorted) = &expected.unsorted {
            if !expected.includes.is_empty() {
                t.fatal(&format!("{prefix}Expected unsorted completion list but also specified 'includes'."));
            }
            if !expected.excludes.is_empty() {
                t.fatal(&format!("{prefix}Expected unsorted completion list but also specified 'excludes'."));
            }
            for item in unsorted {
                match item {
                    Any::String(item) => {
                        if name_to_actual_items.get(item).is_none() {
                            t.fatal(&format!("{prefix}Label '{item}' not found in actual items."));
                        }
                        name_to_actual_items.shift_remove(item);
                    }
                    Any::CompletionItem(item) => {
                        let Some(actual_items) = name_to_actual_items.get(&item.label).cloned() else {
                            t.fatal(&format!("{prefix}Label '{}' not found in actual items.", item.label));
                        };
                        let mut mismatch_prefix = if actual_items.len() > 1 {
                            format!("{prefix}No completion item match for label {} (multiple candidates found): ", item.label)
                        } else {
                            format!("{prefix}Includes completion item mismatch for label {}: ", item.label)
                        };
                        let item_index = actual_items.iter().position(|actual_item| {
                            let err = self.verify_completion_item(t, prefix, actual_item, item);
                            if !err.is_empty() {
                                mismatch_prefix += &format!("\n    {err}");
                                return false;
                            }
                            true
                        });

                        // fail test if no match found
                        let Some(item_index) = item_index else {
                            t.fatal(&mismatch_prefix);
                        };

                        if actual_items.len() == 1 {
                            name_to_actual_items.shift_remove(&item.label);
                        } else {
                            let mut rest = actual_items;
                            rest.remove(item_index);
                            name_to_actual_items.insert(item.label.clone(), rest);
                        }
                    }
                    _ => t.fatal(&format!("{prefix}Expected completion item to be a string or *lsproto.CompletionItem, got {}", item.type_name())),
                }
            }
            if unsorted.len() != actual.len() {
                let unmatched: Vec<String> = name_to_actual_items.keys().cloned().collect();
                t.fatal(&format!("{prefix}Additional completions found but not included in 'unsorted': {}", unmatched.join("\n")));
            }
            return;
        }
        for item in &expected.includes {
            match item {
                Any::String(item) => {
                    if name_to_actual_items.get(item).is_none() {
                        t.fatal(&format!("{prefix}Label '{item}' not found in actual items."));
                    }
                }
                Any::CompletionItem(item) => {
                    let Some(actual_items) = name_to_actual_items.get(&item.label).cloned() else {
                        t.fatal(&format!("{prefix}Label '{}' not found in actual items.", item.label));
                    };

                    let mut mismatch_prefix = if actual_items.len() > 1 {
                        format!("{prefix}No completion item match for label {} (multiple candidates found): ", item.label)
                    } else {
                        format!("{prefix}Includes completion item mismatch for label {}: ", item.label)
                    };
                    let item_index = actual_items.iter().position(|actual_item| {
                        let err = self.verify_completion_item(t, prefix, actual_item, item);
                        if !err.is_empty() {
                            mismatch_prefix += &format!("\n    {err}");
                            return false;
                        }
                        true
                    });

                    // fail test if no match found
                    let Some(item_index) = item_index else {
                        t.fatal(&mismatch_prefix);
                    };

                    // delete previous entries since we verify entries in order
                    if actual_items.len() == 1 || item_index == actual_items.len() - 1 {
                        name_to_actual_items.shift_remove(&item.label);
                    } else {
                        name_to_actual_items.insert(item.label.clone(), actual_items[item_index..].to_vec());
                    }
                }
                _ => t.fatal(&format!("{prefix}Expected completion item to be a string or *lsproto.CompletionItem, got {}", item.type_name())),
            }
        }
        for exclude in &expected.excludes {
            if name_to_actual_items.get(exclude).is_some() {
                t.fatal(&format!("{prefix}Label '{exclude}' should not be in actual items but was found."));
            }
        }
    }

    // fourslash.go:1541
    fn verify_completions_are_exactly(&mut self, t: &T, prefix: &str, actual: &[lsproto::CompletionItem], expected: &[Any]) {
        let label_mismatch_prefix = format!("{prefix}Label mismatch");
        for (i, actual_item) in actual.iter().enumerate() {
            match &expected[i] {
                Any::String(expected_item) => {
                    assert_deep_equal(t, &actual_item.label, expected_item, &label_mismatch_prefix);
                }
                Any::CompletionItem(expected_item) => {
                    assert_deep_equal(t, &actual_item.label, &expected_item.label, &label_mismatch_prefix);
                    let item_prefix = format!("{prefix}Completion item mismatch for label {}", actual_item.label);
                    let err = self.verify_completion_item(t, &item_prefix, actual_item, expected_item);
                    if !err.is_empty() {
                        t.fatal(&format!("{item_prefix}:\n{err}"));
                    }
                }
                other => t.fatal(&format!("Expected completion item to be a string or *lsproto.CompletionItem, got {}", other.type_name())),
            }
        }
    }

    // fourslash.go:1575
    // Returns an error message if the items do not match. cmp.Diff's report is replaced by both values' Debug forms;
    // the ignorePaths options clear the ignored fields (at any depth, like cmp's path filter: `.Kind` also matches
    // MarkupContent.Kind in the documentation) of both sides before comparing.
    fn verify_completion_item(&mut self, t: &T, prefix: &str, actual: &lsproto::CompletionItem, expected: &lsproto::CompletionItem) -> String {
        t.helper();
        let _ = prefix;
        let actual_auto_import_fix = actual.data.as_ref().and_then(|d| d.auto_import.clone());
        let expected_auto_import_fix = expected.data.as_ref().and_then(|d| d.auto_import.clone());
        if actual_auto_import_fix.is_none() != expected_auto_import_fix.is_none() {
            return "Mismatch in auto-import data presence".to_string();
        }

        let mut actual = actual.clone();
        if expected.detail.is_some() || expected.documentation.is_some() || actual_auto_import_fix.is_some() {
            actual = self.resolve_completion_item_impl(t, actual);
        }

        if let Some(actual_auto_import_fix) = &actual_auto_import_fix {
            let err = diff(&auto_import_ignore(&actual), &auto_import_ignore(expected));
            if !err.is_empty() {
                return err;
            }
            if is_any_text_edits(&expected.additional_text_edits) {
                if !actual.additional_text_edits.as_ref().is_some_and(|e| !e.is_empty()) {
                    return "Expected non-nil AdditionalTextEdits for auto-import completion item".to_string();
                }
            } else if is_no_text_edits(&expected.additional_text_edits) && actual.additional_text_edits.as_ref().is_some_and(|e| !e.is_empty()) {
                return "Expected no AdditionalTextEdits for auto-import completion item".to_string();
            }
            if expected.label_details.is_some() {
                let err = diff(&actual.label_details, &expected.label_details);
                if !err.is_empty() {
                    return format!("LabelDetailsMismatch:\n{err}");
                }
            }
            if actual_auto_import_fix.module_specifier != expected_auto_import_fix.as_ref().unwrap().module_specifier {
                return "ModuleSpecifier mismatch".to_string();
            }
        } else {
            let err = diff(&completion_ignore(&actual), &completion_ignore(expected));
            if !err.is_empty() {
                return err;
            }
            if is_any_text_edits(&expected.additional_text_edits) {
                if actual.additional_text_edits.as_ref().is_none_or(|e| e.is_empty()) {
                    return "Expected non-empty AdditionalTextEdits for completion item".to_string();
                }
            } else {
                // NoTextEdits is a pointer to a nil slice in Go.
                let expected_edits = if is_no_text_edits(&expected.additional_text_edits) { Some(Vec::new()) } else { expected.additional_text_edits.clone() };
                let err = diff(&actual.additional_text_edits, &expected_edits);
                if !err.is_empty() {
                    return format!("AdditionalTextEdits mismatch:\n{err}");
                }
            }
        }

        if expected.filter_text.is_some() {
            let err = diff(&actual.filter_text, &expected.filter_text);
            if !err.is_empty() {
                return format!("FilterText mismatch:\n{err}");
            }
        }
        if expected.kind.is_some() {
            let err = diff(&actual.kind, &expected.kind);
            if !err.is_empty() {
                return format!("Kind mismatch:\n{err}");
            }
        }
        let expected_sort_text = Some(expected.sort_text.clone().unwrap_or_else(|| tsrs_ls::SORT_TEXT_LOCATION_PRIORITY.to_string()));
        let err = diff(&actual.sort_text, &expected_sort_text);
        if !err.is_empty() {
            return format!("SortText mismatch:\n{err}");
        }

        String::new()
    }

    // fourslash.go:1655
    pub fn resolve_completion_item(&mut self, t: &T, item: Option<lsproto::CompletionItem>) -> Option<lsproto::CompletionItem> {
        t.helper();
        let item = item.expect("runtime error: invalid memory address or nil pointer dereference");
        Some(self.resolve_completion_item_impl(t, item))
    }

    // fourslash.go:1660
    fn resolve_completion_item_impl(&mut self, t: &T, item: lsproto::CompletionItem) -> lsproto::CompletionItem {
        self.send_request(t, lsproto::COMPLETION_ITEM_RESOLVE_INFO, item)
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
        Self::server_unavailable(t, "feature not ported: code actions (VerifyOrganizeImportsWithRequestKind)")
    }

    // fourslash.go:2222
    fn find_completion_for_code_action(&mut self, t: &T, items: &[lsproto::CompletionItem], description: &str) -> Option<lsproto::CompletionItem> {
        t.helper();
        if let Some(item) = items.iter().find(|item| item.additional_text_edits.as_ref().is_some_and(|e| !e.is_empty())) {
            return Some(item.clone());
        }
        for item in items {
            let resolved_item = self.resolve_completion_item_impl(t, item.clone());
            if resolved_item.additional_text_edits.as_ref().is_none_or(|e| e.is_empty()) {
                continue;
            }
            if !description.is_empty() && !resolved_item.detail.as_ref().is_some_and(|d| d.contains(description)) {
                continue;
            }
            return Some(resolved_item);
        }
        None
    }

    // fourslash.go:2241
    pub fn verify_apply_code_action_from_completion(&mut self, t: &T, marker_name: Option<String>, options: Option<ApplyCodeActionFromCompletionOptions>) {
        t.helper();
        let nil = || -> ! { panic!("runtime error: invalid memory address or nil pointer dereference") };
        let marker_name = marker_name.unwrap_or_else(|| nil());
        self.go_to_marker(t, &marker_name);
        let user_preferences = match options.as_ref().and_then(|o| o.user_preferences.clone()) {
            Some(user_preferences) => user_preferences,
            // Default preferences: enables auto-imports
            None => lsutil::new_default_user_preferences(),
        };

        let reset = self.configure_with_reset(t, user_preferences);
        let r = crate::go::run(|| {
            let completions_list = self.get_completions_impl(t, None /*userPreferences*/); // Already configured, so we do not need to pass it in again
            let completions_list = completions_list.unwrap_or_else(|| nil());
            let options = options.as_ref().unwrap_or_else(|| nil());
            let items: Vec<lsproto::CompletionItem> = completions_list
                .items
                .iter()
                .filter(|item| {
                    if item.label != options.name {
                        return false;
                    }
                    let Some(data) = &item.data else {
                        return false;
                    };
                    if let Some(auto_import_fix) = &options.auto_import_fix {
                        let mut module_specifier = auto_import_fix.module_specifier.as_str();
                        if module_specifier.is_empty() {
                            module_specifier = &options.source;
                        }
                        return data.auto_import.as_ref().is_some_and(|a| a.module_specifier == module_specifier);
                    }
                    if data.auto_import.is_none() && !data.source.is_empty() && data.source == options.source {
                        return true;
                    }
                    if data.auto_import.as_ref().is_some_and(|a| a.module_specifier == options.source) {
                        return true;
                    }
                    false
                })
                .cloned()
                .collect();

            let Some(item) = self.find_completion_for_code_action(t, &items, &options.description) else {
                t.fatal(&format!("Code action '{}' from source '{}' not found.", options.name, options.source));
            };

            // apply the item to the test files
            self.apply_text_edits(t, item.additional_text_edits.unwrap_or_else(|| nil()));
            if let Some(new_file_content) = &options.new_file_content {
                crate::go::assert::equal(
                    t,
                    self.get_script_info(&self.active_filename.clone()).content.as_str(),
                    new_file_content.as_str(),
                    "File content after applying code action did not match expected content.",
                );
            } else if options.new_range_content.is_some() {
                t.fatal("!!! TODO");
            }
        });
        reset(self, t);
        crate::go::resume(r);
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
        let reset = preferences.map(|preferences| self.configure_with_reset(t, preferences));

        let mut found_at_least_one_code_lens = false;
        let mut open_files: Vec<String> = self.open_files.keys().cloned().collect();
        open_files.sort();
        for open_file in &open_files {
            let params = lsproto::CodeLensParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(open_file) },
                ..Default::default()
            };

            let unresolved_code_lens_list = self.send_request(t, lsproto::TEXT_DOCUMENT_CODE_LENS_INFO, params);
            let Some(code_lenses) = unresolved_code_lens_list.code_lenses.filter(|c| !c.is_empty()) else {
                continue;
            };
            found_at_least_one_code_lens = true;

            for unresolved_code_lens in code_lenses {
                let resolved_code_lens = self.send_request(t, lsproto::CODE_LENS_RESOLVE_INFO, unresolved_code_lens);
                let Some(command) = &resolved_code_lens.command else {
                    t.fatal("Expected resolved code lens to have a command.");
                };
                if !command.command.is_empty() {
                    assert_deep_equal(t, &command.command.as_str(), &SHOW_CODE_LENS_LOCATIONS_COMMAND_NAME, "");
                }

                let mut locations: Vec<lsproto::Location> = Vec::new();
                // commandArgs: (DocumentUri, Position, Location[])
                if let Some(command_args) = &command.arguments {
                    match <Vec<lsproto::Location> as Json>::from_json(&command_args[2]) {
                        Ok(locs) => locations = locs,
                        Err(err) => t.fatal(&format!("failed to re-encode code lens locations: {err}")),
                    }
                }

                let ranges = self.converters.converters.from_lsp_range(self.get_script_info(open_file), resolved_code_lens.range, Feature::All);
                if ranges.len() != 1 {
                    continue;
                }
                let code_lens_range = ranges[0].span;
                let baseline = self.get_baseline_for_locations_with_file_contents(
                    &locations,
                    BaselineFourslashLocationsOptions {
                        marker: Some(MarkerOrRange::RangeMarker(Arc::new(RangeMarker {
                            file_name: open_file.clone(),
                            ls_range: resolved_code_lens.range,
                            range: code_lens_range,
                            marker: None,
                        }))),
                        marker_name: format!("/*CODELENS: {}*/", command.title),
                        ..Default::default()
                    },
                );
                self.add_result_to_baseline(t, CODE_LENSES_CMD, &baseline);
            }
        }

        if !found_at_least_one_code_lens {
            t.fatal("Expected at least one code lens in any open file, but got none.");
        }
        if let Some(reset) = reset {
            reset(self, t);
        }
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
        let result = self.send_request(t, lsproto::WORKSPACE_SYMBOL_INFO, lsproto::WorkspaceSymbolParams { query: query.to_string(), ..Default::default() });

        let mut location_to_text: FxHashMap<DocumentSpan, lsproto::SymbolInformation> = FxHashMap::default();
        let mut grouped_ranges: MultiMap<lsproto::DocumentUri, DocumentSpan> = MultiMap::default();
        let symbol_informations = result.symbol_informations.unwrap_or_default();
        for symbol in symbol_informations {
            let uri = symbol.location.uri.clone();
            let span = location_to_span(&symbol.location);
            grouped_ranges.add(uri, span.clone());
            location_to_text.insert(span, symbol);
        }

        let baseline = self.get_baseline_for_grouped_spans_with_file_contents(
            &grouped_ranges,
            &BaselineFourslashLocationsOptions {
                get_location_data: Some(Box::new(move |span: &DocumentSpan| symbol_information_to_data(&location_to_text[span]))),
                ..Default::default()
            },
        );
        self.add_result_to_baseline(t, BaselineCommand("workspaceSymbol"), &baseline);
    }

    // fourslash.go:2850
    pub fn verify_outlining_spans(&mut self, t: &T, folding_range_kind: &[lsproto::FoldingRangeKind]) {
        let params = lsproto::FoldingRangeParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_FOLDING_RANGE_INFO, params);
        let Some(folding_ranges) = result.folding_ranges else {
            t.fatal("Nil response received for folding range request");
        };

        // Extract actual folding ranges from the result and filter by kind if specified
        let mut actual_ranges = folding_ranges;
        if let Some(&target_kind) = folding_range_kind.first() {
            actual_ranges.retain(|r| r.kind == Some(target_kind));
        }

        if actual_ranges.len() != self.test_data.ranges.len() {
            t.fatal(&format!(
                "verifyOutliningSpans failed - expected total spans to be {}, but was {}",
                self.test_data.ranges.len(),
                actual_ranges.len()
            ));
        }

        // Go sorts the test data's range slice in place (f.Ranges() returns it).
        tsrs_core::goslices::sort_func(&mut self.test_data.ranges, |a, b| lsproto::compare_positions(a.ls_pos(), b.ls_pos()));

        for (i, expected_range) in self.test_data.ranges.iter().enumerate() {
            let actual_range = &actual_ranges[i];
            let start_pos = lsproto::Position { line: actual_range.start_line, character: actual_range.start_character.unwrap() };
            let end_pos = lsproto::Position { line: actual_range.end_line, character: actual_range.end_character.unwrap() };

            if lsproto::compare_positions(start_pos, expected_range.ls_range.start) != 0 || lsproto::compare_positions(end_pos, expected_range.ls_range.end) != 0 {
                t.fatal(&format!(
                    "verifyOutliningSpans failed - span {} has invalid positions:\n  actual: start ({},{}), end ({},{})\n  expected: start ({},{}), end ({},{})",
                    i + 1,
                    actual_range.start_line,
                    actual_range.start_character.unwrap(),
                    actual_range.end_line,
                    actual_range.end_character.unwrap(),
                    expected_range.ls_range.start.line,
                    expected_range.ls_range.start.character,
                    expected_range.ls_range.end.line,
                    expected_range.ls_range.end.character
                ));
            }
        }
    }

    // VerifyFoldingRangeLines verifies folding ranges by comparing only start and end lines.
    // This is useful for testing with lineFoldingOnly where character positions are ignored.
    // fourslash.go:2907
    pub fn verify_folding_range_lines(&mut self, t: &T, expected: &[FoldingRangeLineExpected]) {
        let params = lsproto::FoldingRangeParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_FOLDING_RANGE_INFO, params);
        let Some(actual_ranges) = result.folding_ranges else {
            t.fatal("Nil response received for folding range request");
        };

        if actual_ranges.len() != expected.len() {
            t.fatal(&format!("verifyFoldingRangeLines failed - expected {} ranges, got {}", expected.len(), actual_ranges.len()));
        }

        for (i, exp) in expected.iter().enumerate() {
            let got = &actual_ranges[i];
            if got.start_line != exp.start_line || got.end_line != exp.end_line {
                t.error(&format!(
                    "verifyFoldingRangeLines failed - range {}: expected (startLine={}, endLine={}), got (startLine={}, endLine={})",
                    i, exp.start_line, exp.end_line, got.start_line, got.end_line
                ));
            }
        }
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
        let mut markers_and_items: Vec<MarkerAndItem<Option<lsproto::SignatureHelp>>> = Vec::new();
        for marker in self.markers() {
            if marker.name.is_none() {
                continue;
            }

            let params = lsproto::SignatureHelpParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&marker.file_name) },
                position: marker.ls_position,
                ..Default::default()
            };

            let result = self.send_request(t, lsproto::TEXT_DOCUMENT_SIGNATURE_HELP_INFO, params);
            markers_and_items.push(MarkerAndItem { marker, item: result.signature_help });
        }

        // SignatureHelp doesn't have a range like hover does
        let get_range = |_item: &Option<lsproto::SignatureHelp>| -> Option<lsproto::Range> { None };

        let get_tooltip_lines = |item: &Option<lsproto::SignatureHelp>, _prev: &Option<lsproto::SignatureHelp>| -> Vec<String> {
            let Some(item) = item.as_ref().filter(|item| !item.signatures.is_empty()) else {
                return vec!["No signature help available".to_string()];
            };

            // Show active signature if specified, otherwise first signature
            let mut active_signature = 0;
            if let Some(a) = item.active_signature {
                if (a as usize) < item.signatures.len() {
                    active_signature = a as usize;
                }
            }

            let sig = &item.signatures[active_signature];

            // Build signature display
            let mut signature_line = sig.label.clone();
            let mut active_param_line = String::new();

            // Determine active parameter: per-signature takes precedence over top-level per LSP spec
            // "If provided (or `null`), this is used in place of `SignatureHelp.activeParameter`."
            let active_param_ptr = if sig.active_parameter.is_some() { &sig.active_parameter } else { &item.active_parameter };

            // Show active parameter if specified, and the signature text.
            if let (Some(active_param_index), Some(parameters)) = (active_param_ptr.as_ref().and_then(|p| p.uinteger), &sig.parameters) {
                let active_param_index = active_param_index as usize;
                if active_param_index < parameters.len() {
                    let active_param = &parameters[active_param_index];

                    // Get the parameter label and bold the
                    // parameter text within the original string.
                    let active_param_label = if let Some(s) = &active_param.label.string {
                        s.clone()
                    } else if let Some(tuple) = &active_param.label.tuple {
                        signature_line[tuple[0] as usize..tuple[1] as usize].to_string()
                    } else {
                        t.fatal("Unsupported param label kind.");
                    };
                    signature_line = signature_line.replacen(&active_param_label, &format!("**{active_param_label}**"), 1);

                    if let Some(documentation) = &active_param.documentation {
                        if let Some(markup_content) = &documentation.markup_content {
                            active_param_line = markup_content.value.clone();
                        } else if let Some(s) = &documentation.string {
                            active_param_line = s.clone();
                        }

                        active_param_line = format!("- `{active_param_label}`: {active_param_line}");
                    }
                }
            }

            let mut result: Vec<String> = Vec::with_capacity(16);
            result.push(signature_line);
            if !active_param_line.is_empty() {
                result.push(active_param_line);
            }

            // ORIGINALLY we would "only display signature documentation on the last argument when multiple arguments are marked".
            // !!!
            // Note that this is harder than in Strada, because LSP signature help has no concept of
            // applicable spans.
            if let Some(documentation) = &sig.documentation {
                if let Some(markup_content) = &documentation.markup_content {
                    result.extend(markup_content.value.split('\n').map(str::to_string));
                } else if let Some(s) = &documentation.string {
                    result.extend(s.split('\n').map(str::to_string));
                } else {
                    t.fatal("Unsupported documentation format.");
                }
            }

            result
        };

        let annotated = self.annotate_content_with_tooltips(t, &markers_and_items, "signaturehelp", get_range, get_tooltip_lines);
        self.add_result_to_baseline(t, SIGNATURE_HELP_CMD, &annotated);
        let json = Value::Array(markers_and_items.iter().map(|m| marker_and_item_to_json(m, |h| h.as_ref().map_or(Value::Null, |h| h.to_json()))).collect());
        match tsrs_core::json::marshal_indent(&json, "", "  ") {
            Ok(json_str) => self.write_to_baseline(SIGNATURE_HELP_CMD, &json_str),
            Err(err) => t.fatal(&format!("Failed to stringify markers and items for baseline: {err}")),
        }
    }

    // fourslash.go:3283
    pub fn verify_baseline_selection_ranges(&mut self, t: &T) {
        let markers = self.markers();
        let mut result = String::new();
        let new_line = "\n";

        for (i, marker) in markers.iter().enumerate() {
            if i > 0 {
                result.push_str(new_line);
                result.push_str(&"=".repeat(80));
                result.push_str(new_line);
                result.push_str(new_line);
            }

            let script = self.get_script_info(&marker.file_name());
            let file_content = script.content.clone();

            // Add the marker position indicator
            let marker_pos = marker.position as usize;
            let baseline_content = format!("{}/**/{}{}", &file_content[..marker_pos], &file_content[marker_pos..], new_line);
            result.push_str(&baseline_content);

            // Get selection ranges at this marker
            let params = lsproto::SelectionRangeParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&marker.file_name()) },
                positions: vec![marker.ls_position],
                ..Default::default()
            };

            let selection_range_result = self.send_request(t, lsproto::TEXT_DOCUMENT_SELECTION_RANGE_INFO, params);

            let selection_ranges = selection_range_result.selection_ranges.unwrap_or_default();
            if selection_ranges.is_empty() {
                result.push_str("No selection ranges available\n");
                continue;
            }

            let mut selection_range = Some(&selection_ranges[0]);

            // Add blank line after source code section
            result.push_str(new_line);

            // Walk through the selection range chain
            while let Some(sr) = selection_range {
                let start = self.converters.line_and_character_to_position(script.clone(), sr.range.start) as usize;
                let end = self.converters.line_and_character_to_position(script.clone(), sr.range.end) as usize;

                // Create a masked version of the file showing only this range
                // (Go compares rune indices with the byte offsets above.)
                let masked: String = file_content
                    .chars()
                    .enumerate()
                    .map(|(i, ch)| {
                        if i >= start && i < end {
                            // Keep characters in the selection range
                            if ch == ' ' {
                                '•'
                            } else {
                                ch
                            }
                        } else if ch == '\n' || ch == '\r' {
                            // Replace characters outside the range
                            ch
                        } else {
                            ' '
                        }
                    })
                    .collect();

                let mut masked_str = masked;

                // Add line break arrows
                masked_str = masked_str.replace('\n', "↲\n");
                masked_str = masked_str.replace('\r', "↲\r");

                // Remove blank lines
                let mut non_blank_lines: Vec<&str> = Vec::new();
                for line in masked_str.split('\n') {
                    let trimmed = line.trim_matches(go_is_space);
                    if !trimmed.is_empty() && trimmed != "↲" {
                        non_blank_lines.push(line);
                    }
                }
                masked_str = non_blank_lines.join("\n");

                // Find leading and trailing width of non-whitespace characters
                let masked_runes: Vec<char> = masked_str.chars().collect();
                let is_real_character = |ch: char| ch != '•' && ch != '↲' && !stringutil::is_white_space_like(ch);

                let leading_width = masked_runes.iter().position(|&ch| is_real_character(ch));
                let trailing_width = masked_runes.iter().rposition(|&ch| is_real_character(ch));

                if let (Some(leading_width), Some(trailing_width)) = (leading_width, trailing_width) {
                    if leading_width <= trailing_width {
                        // Clean up middle section
                        let prefix: String = masked_runes[..leading_width].iter().collect();
                        let middle: String = masked_runes[leading_width..trailing_width + 1].iter().collect();
                        let suffix: String = masked_runes[trailing_width + 1..].iter().collect();

                        let middle = middle.replace('•', " ").replace('↲', "");

                        masked_str = prefix + &middle + &suffix;
                    }
                }

                // Add blank line before multi-line ranges
                if masked_str.contains('\n') {
                    result.push_str(new_line);
                }

                result.push_str(&masked_str);
                if !masked_str.ends_with('\n') {
                    result.push_str(new_line);
                }

                selection_range = sr.parent.as_deref();
            }
        }
        let result = result.strip_suffix('\n').unwrap_or(&result).to_string();
        self.add_result_to_baseline(t, SMART_SELECTION_CMD, &result);
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
        for (marker, expected_text) in markers_to_new_text.iter() {
            self.go_to_marker(t, marker);
            let params = lsproto::VSOnAutoInsertParams {
                vs_text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
                vs_position: self.current_caret_position,
                vs_ch: ">".to_string(),
                ..Default::default()
            };

            let request_result = self.send_request(t, lsproto::TEXT_DOCUMENT_VS_ON_AUTO_INSERT_INFO, params);

            let mut actual_text: Option<String> = None;
            if let Some(item) = &request_result.vs_on_auto_insert_response_item {
                let mut new_text = item.vs_text_edit.new_text.clone();
                if item.vs_text_edit_format == lsproto::InsertTextFormat::Snippet {
                    match new_text.strip_prefix("$0") {
                        Some(rest) => new_text = rest.to_string(),
                        None => t.fatal(&format!(
                            "{}expected JSX closing tag snippet to begin with $0, got {}",
                            self.get_current_position_prefix(),
                            crate::go::quote(&item.vs_text_edit.new_text)
                        )),
                    }
                }
                actual_text = Some(new_text);
            }
            assert_deep_equal(t, &actual_text, expected_text, &format!("{}JSX closing tag text mismatch", self.get_current_position_prefix()));
        }
    }

    // VerifyBaselineClosingTags generates a baseline for JSX closing tag completions at all markers.
    // fourslash.go:4285
    pub fn verify_baseline_closing_tags(&mut self, t: &T) {
        t.helper();

        let mut markers_and_items: Vec<MarkerAndItem<Option<lsproto::VSOnAutoInsertResponseItem>>> = Vec::new();
        for marker in self.markers() {
            if marker.name.is_none() {
                continue;
            }

            let params = lsproto::VSOnAutoInsertParams {
                vs_text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&marker.file_name) },
                vs_position: marker.ls_position,
                vs_ch: ">".to_string(),
                ..Default::default()
            };

            let result = self.send_request(t, lsproto::TEXT_DOCUMENT_VS_ON_AUTO_INSERT_INFO, params);
            markers_and_items.push(MarkerAndItem { marker, item: result.vs_on_auto_insert_response_item });
        }

        // Returning nil lets annotateContentWithTooltips render the caret marker at
        // the marker position. The text edit's range is zero-width at the cursor,
        // which would render as an empty underline.
        let get_range = |_item: &Option<lsproto::VSOnAutoInsertResponseItem>| -> Option<lsproto::Range> { None };

        let get_tooltip_lines = |item: &Option<lsproto::VSOnAutoInsertResponseItem>, _prev: &Option<lsproto::VSOnAutoInsertResponseItem>| -> Vec<String> {
            let Some(item) = item else {
                return vec!["No closing tag".to_string()];
            };
            let mut format = "plaintext";
            if item.vs_text_edit_format == lsproto::InsertTextFormat::Snippet {
                format = "snippet";
            }
            vec![format!("{format}: {}", crate::go::quote(&item.vs_text_edit.new_text))]
        };

        let result = self.annotate_content_with_tooltips(t, &markers_and_items, "closing tag", get_range, get_tooltip_lines);
        self.add_result_to_baseline(t, CLOSING_TAG_CMD, &result);
    }

    // VerifySignatureHelp verifies signature help at the current position matches the expected options.
    // fourslash.go:4353
    pub fn verify_signature_help(&mut self, t: &T, expected: VerifySignatureHelpOptions) {
        t.helper();
        let prefix = self.get_current_position_prefix();
        let params = lsproto::SignatureHelpParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_SIGNATURE_HELP_INFO, params);
        let Some(help) = result.signature_help else {
            t.fatal(&format!("{prefix}Could not get signature help"));
        };

        // Determine which signature to check
        let mut selected_index: usize = 0;
        if expected.override_selected_item_index > 0 {
            selected_index = expected.override_selected_item_index as usize;
        } else if let Some(active_signature) = help.active_signature {
            selected_index = active_signature as usize;
        }

        if selected_index >= help.signatures.len() {
            t.fatal(&format!("{prefix}Selected signature index {selected_index} out of range (have {} signatures)", help.signatures.len()));
        }

        let selected_sig = &help.signatures[selected_index];

        // Verify overloads count
        if expected.overloads_count > 0 && help.signatures.len() != expected.overloads_count as usize {
            t.error(&format!("{prefix}Expected {} overloads, got {}", expected.overloads_count, help.signatures.len()));
        }

        // Verify signature text
        if !expected.text.is_empty() && selected_sig.label != expected.text {
            t.error(&format!("{prefix}Expected signature text {}, got {}", crate::go::quote(&expected.text), crate::go::quote(&selected_sig.label)));
        }

        // Verify doc comment
        if !expected.doc_comment.is_empty() {
            let actual_doc = documentation_text(&selected_sig.documentation);
            if actual_doc != expected.doc_comment {
                t.error(&format!("{prefix}Expected doc comment {}, got {}", crate::go::quote(&expected.doc_comment), crate::go::quote(&actual_doc)));
            }
        }

        // Verify parameter count
        if expected.parameter_count > 0 {
            let param_count = selected_sig.parameters.as_ref().map_or(0, Vec::len);
            if param_count != expected.parameter_count as usize {
                t.error(&format!("{prefix}Expected {} parameters, got {param_count}", expected.parameter_count));
            }
        }

        // Get active parameter
        let mut active_param_index: usize = 0;
        if let Some(i) = selected_sig.active_parameter.as_ref().and_then(|p| p.uinteger) {
            active_param_index = i as usize;
        } else if let Some(i) = help.active_parameter.as_ref().and_then(|p| p.uinteger) {
            active_param_index = i as usize;
        }

        let active_param: Option<&lsproto::ParameterInformation> = selected_sig.parameters.as_ref().and_then(|p| p.get(active_param_index));

        // Verify parameter name
        if !expected.parameter_name.is_empty() {
            match active_param {
                None => t.error(&format!("{prefix}Expected parameter name {}, but no active parameter", crate::go::quote(&expected.parameter_name))),
                Some(active_param) => {
                    // Parameter name is extracted from the label
                    let mut actual_name = String::new();
                    if let Some(label) = &active_param.label.string {
                        // Extract name from label like "x: string" -> "x" or "T extends Foo" -> "T" or "...x: any[]" -> "x"
                        // Strip leading "..." for rest parameters
                        let label = label.strip_prefix("...").unwrap_or(label);
                        if let Some((name, _)) = label.split_once(':') {
                            actual_name = name.trim().to_string();
                        } else if let Some((name, _)) = label.split_once(" extends ") {
                            actual_name = name.trim().to_string();
                        } else {
                            actual_name = label.to_string();
                        }
                    }
                    if actual_name != expected.parameter_name {
                        t.error(&format!("{prefix}Expected parameter name {}, got {}", crate::go::quote(&expected.parameter_name), crate::go::quote(&actual_name)));
                    }
                }
            }
        }

        // Verify parameter span (label)
        if !expected.parameter_span.is_empty() {
            match active_param {
                None => t.error(&format!("{prefix}Expected parameter span {}, but no active parameter", crate::go::quote(&expected.parameter_span))),
                Some(active_param) => {
                    let actual_span = active_param.label.string.clone().unwrap_or_default();
                    if actual_span != expected.parameter_span {
                        t.error(&format!("{prefix}Expected parameter span {}, got {}", crate::go::quote(&expected.parameter_span), crate::go::quote(&actual_span)));
                    }
                }
            }
        }

        // Verify parameter doc comment
        if !expected.parameter_doc_comment.is_empty() {
            match active_param {
                None => t.error(&format!("{prefix}Expected parameter doc comment {}, but no active parameter", crate::go::quote(&expected.parameter_doc_comment))),
                Some(active_param) => {
                    let actual_doc = documentation_text(&active_param.documentation);
                    if actual_doc != expected.parameter_doc_comment {
                        t.error(&format!(
                            "{prefix}Expected parameter doc comment {}, got {}",
                            crate::go::quote(&expected.parameter_doc_comment),
                            crate::go::quote(&actual_doc)
                        ));
                    }
                }
            }
        }

        // Verify isVariadic (check if any parameter starts with "...")
        if expected.is_variadic_set {
            let actual_is_variadic =
                selected_sig.parameters.as_ref().is_some_and(|params| params.iter().any(|param| param.label.string.as_ref().is_some_and(|s| s.starts_with("..."))));
            if actual_is_variadic != expected.is_variadic {
                t.error(&format!("{prefix}Expected isVariadic={}, got {actual_is_variadic}", expected.is_variadic));
            }
        }
    }

    // VerifyNoSignatureHelp verifies that no signature help is available at the current position.
    // fourslash.go:4513
    pub fn verify_no_signature_help(&mut self, t: &T) {
        t.helper();
        let prefix = self.get_current_position_prefix();
        let params = lsproto::SignatureHelpParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_SIGNATURE_HELP_INFO, params);
        if let Some(help) = result.signature_help.as_ref().filter(|h| !h.signatures.is_empty()) {
            t.error(&format!("{prefix}Expected no signature help, but got {} signatures", help.signatures.len()));
        }
    }

    // VerifyNoSignatureHelpWithContext verifies that no signature help is available at the current position with a given context.
    // fourslash.go:4529
    pub fn verify_no_signature_help_with_context(&mut self, t: &T, context: Option<lsproto::SignatureHelpContext>) {
        t.helper();
        let prefix = self.get_current_position_prefix();
        let params = lsproto::SignatureHelpParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            context,
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_SIGNATURE_HELP_INFO, params);
        if let Some(help) = result.signature_help.as_ref().filter(|h| !h.signatures.is_empty()) {
            t.error(&format!("{prefix}Expected no signature help, but got {} signatures", help.signatures.len()));
        }
    }

    // fourslash.go:4546
    pub fn verify_no_signature_help_for_markers_with_context(&mut self, t: &T, context: Option<lsproto::SignatureHelpContext>, markers: &[&str]) {
        for marker in markers {
            self.go_to_marker(t, marker);
            self.verify_no_signature_help_with_context(t, context.clone());
        }
    }

    // VerifySignatureHelpPresent verifies that signature help is available at the current position with a given context.
    // fourslash.go:4555
    pub fn verify_signature_help_present(&mut self, t: &T, context: Option<lsproto::SignatureHelpContext>) {
        t.helper();
        let prefix = self.get_current_position_prefix();
        let params = lsproto::SignatureHelpParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            context,
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_SIGNATURE_HELP_INFO, params);
        if result.signature_help.as_ref().is_none_or(|h| h.signatures.is_empty()) {
            t.error(&format!("{prefix}Expected signature help to be present, but got none"));
        }
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

    // VerifySignatureHelpWithCases verifies signature help using detailed SignatureHelpCase structs.
    // This is useful for more complex tests that need to verify the full signature help response.
    // fourslash.go:4597
    pub fn verify_signature_help_with_cases(&mut self, t: &T, signature_help_cases: &[SignatureHelpCase]) {
        for option in signature_help_cases {
            match &option.marker_input {
                Any::String(marker) => {
                    self.go_to_marker(t, marker);
                    self.verify_signature_help_impl(t, option.context.clone(), option.expected.as_ref());
                }
                Any::Marker(marker) => {
                    self.go_to_marker_impl(t, &MarkerOrRange::Marker(marker.clone()));
                    self.verify_signature_help_impl(t, option.context.clone(), option.expected.as_ref());
                }
                Any::StringSlice(markers) => {
                    for marker_name in markers {
                        self.go_to_marker(t, marker_name);
                        self.verify_signature_help_impl(t, option.context.clone(), option.expected.as_ref());
                    }
                }
                Any::MarkerSlice(markers) => {
                    for marker in markers {
                        self.go_to_marker_impl(t, &MarkerOrRange::Marker(marker.clone()));
                        self.verify_signature_help_impl(t, option.context.clone(), option.expected.as_ref());
                    }
                }
                Any::Nil => self.verify_signature_help_impl(t, option.context.clone(), option.expected.as_ref()),
                other => t.fatal(&format!("Invalid marker input type: {}. Expected string, *Marker, []string, or []*Marker.", other.type_name())),
            }
        }
    }

    // fourslash.go:4626 (verifySignatureHelp)
    fn verify_signature_help_impl(&mut self, t: &T, context: Option<lsproto::SignatureHelpContext>, expected: Option<&lsproto::SignatureHelp>) {
        let prefix = self.get_current_position_prefix();
        let params = lsproto::SignatureHelpParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            position: self.current_caret_position,
            context,
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_SIGNATURE_HELP_INFO, params);
        self.verify_signature_help_result(t, result.signature_help.as_ref(), expected, &prefix);
    }

    // fourslash.go:4642
    fn verify_signature_help_result(&self, t: &T, actual: Option<&lsproto::SignatureHelp>, expected: Option<&lsproto::SignatureHelp>, prefix: &str) {
        assert_deep_equal(t, &actual, &expected, &format!("{prefix} SignatureHelp mismatch"));
    }

    // fourslash.go:4657
    pub fn baseline_auto_imports_completions(&mut self, t: &T, marker_names: &[&str]) {
        t.helper();
        let reset = self.configure_with_reset(
            t,
            lsutil::UserPreferences {
                include_completions_for_module_exports: Tristate::True,
                include_completions_for_import_statements: Tristate::True,
                import_module_specifier_preference: self.user_preferences.import_module_specifier_preference.clone(),
                import_module_specifier_ending: self.user_preferences.import_module_specifier_ending.clone(),
                auto_import_specifier_exclude_regexes: self.user_preferences.auto_import_specifier_exclude_regexes.clone(),
                auto_import_file_exclude_patterns: self.user_preferences.auto_import_file_exclude_patterns.clone(),
                prefer_type_only_auto_imports: self.user_preferences.prefer_type_only_auto_imports,
                auto_import_entrypoint_directory_search: self.user_preferences.auto_import_entrypoint_directory_search,
                ..Default::default()
            },
        );
        let r = crate::go::run(|| {
            for &marker_name in marker_names {
                self.go_to_marker(t, marker_name);
                let params = lsproto::CompletionParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
                    position: self.current_caret_position,
                    context: Some(lsproto::CompletionContext::default()),
                    ..Default::default()
                };
                let result = self.send_request(t, lsproto::TEXT_DOCUMENT_COMPLETION_INFO, params);

                let prefix = format!("At marker '{marker_name}': ");

                self.write_to_baseline(AUTO_IMPORTS_CMD, "// === Auto Imports === \n");

                let active = self.active_filename.clone();
                let Some(file_content) = self.text_of_file(&active) else {
                    t.fatal(&format!("{prefix}Failed to read file {active} for auto-import baseline"));
                };

                let marker = self.test_data.marker_positions[marker_name].clone();
                let ext = tspath::get_any_extension_from_path(&active, &[], true);
                let ext = ext.strip_prefix('.').unwrap_or(&ext);
                let lang = if ext == "mts" || ext == "cts" { "ts" } else { ext };
                let position = marker.position as usize;
                self.write_to_baseline(
                    AUTO_IMPORTS_CMD,
                    &code_fence(lang, &format!("// @FileName: {active}\n{}/*{marker_name}*/{}", &file_content[..position], &file_content[position..])),
                );

                let current_file = new_script_info(&active, &file_content);
                let line_map = current_file.line_map.clone();
                let converters = new_test_converters(lsconv::new_converters(lsproto::PositionEncodingKind::UTF8, move |_| Some(line_map.clone())));
                let list: Vec<lsproto::CompletionItem> = match result.items.filter(|items| !items.is_empty()) {
                    Some(items) => items,
                    None => match result.list.filter(|list| !list.items.is_empty()) {
                        Some(list) => list.items,
                        None => {
                            self.write_to_baseline(AUTO_IMPORTS_CMD, "no autoimport completions found\n\n");
                            continue;
                        }
                    },
                };

                for item in list {
                    // (Go dereferences SortText, which panics when it is nil.)
                    if item.data.is_none() || item.sort_text.as_deref().expect("runtime error: invalid memory address or nil pointer dereference") != tsrs_ls::SORT_TEXT_AUTO_IMPORT_SUGGESTIONS {
                        continue;
                    }
                    let label = item.label.clone();
                    let detail = item.detail.clone();
                    let details = self.send_request(t, lsproto::COMPLETION_ITEM_RESOLVE_INFO, item);
                    let Some(mut all_changes) = details.additional_text_edits.filter(|edits| !edits.is_empty()) else {
                        // (Go formats the *string detail with %s, which prints the pointer.)
                        t.fatal(&format!("{prefix}Entry {label} from {detail:?} returned no code changes from completion details request"));
                    };

                    // !!! calculate the change provided by the completiontext
                    // sorted from back-of-file-most to front-of-file-most
                    all_changes.sort_by(|a, b| lsproto::compare_positions(b.range.start, a.range.start).cmp(&0));
                    let mut new_file_content = file_content.clone();
                    for change in &all_changes {
                        let start = converters.line_and_character_to_position(current_file.clone(), change.range.start) as usize;
                        let end = converters.line_and_character_to_position(current_file.clone(), change.range.end) as usize;
                        new_file_content = format!("{}{}{}", &new_file_content[..start], change.new_text, &new_file_content[end..]);
                    }
                    self.write_to_baseline(AUTO_IMPORTS_CMD, &format!("{}\n\n", code_fence(lang, &new_file_content)));
                }
            }
        });
        reset(self, t);
        crate::go::resume(r);
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
        let file_name = self.active_filename.clone();
        let lsp_range = match span {
            None => {
                let script = self.get_script_info(&file_name);
                let (r, _) = self.converters.converters.to_lsp_range(&script, TextRange::new(0, script.content.len() as i32));
                r
            }
            Some(span) => span,
        };

        let params = lsproto::InlayHintParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&file_name) },
            range: lsp_range,
            ..Default::default()
        };

        let preferences = test_preferences.unwrap_or_else(lsutil::new_default_user_preferences);
        let reset = self.configure_with_reset(t, preferences);

        let prefix = format!("At position (Ln {}, Col {}): ", lsp_range.start.line, lsp_range.start.character);
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_INLAY_HINT_INFO, params);
        let content = self.get_script_info(&file_name).content;
        let file_lines: Vec<&str> = content.split('\n').collect();
        let mut annotations: Vec<String> = Vec::new();
        if let Some(mut inlay_hints) = result.inlay_hints {
            tsrs_core::goslices::sort_func(&mut inlay_hints, |a, b| lsproto::compare_positions(a.position, b.position));
            for mut hint in inlay_hints {
                if let Some(parts) = &mut hint.label.inlay_hint_label_parts {
                    for part in parts {
                        // Avoid diffs caused by lib file updates.
                        if let Some(location) = &mut part.location {
                            if is_lib_file(&location.uri.file_name()) {
                                location.range.start = lsproto::Position { line: 0, character: 0 };
                                location.range.end = lsproto::Position { line: 0, character: 0 };
                            }
                        }
                    }
                }
                let underline = format!("{}^", " ".repeat(hint.position.character as usize));
                let hint_json = match tsrs_core::json::marshal_indent(&hint.to_json(), "", "  ") {
                    Ok(j) => j,
                    Err(err) => t.fatal(&format!("{prefix}Failed to stringify inlay hint for baseline: {err}")),
                };
                let mut annotation = file_lines[hint.position.line as usize].to_string();
                annotation += &format!("\n{underline}\n{hint_json}");
                annotations.push(annotation);
            }
        }

        if annotations.is_empty() {
            annotations.push("=== No inlay hints ===".to_string());
        }

        self.add_result_to_baseline(t, INLAY_HINTS_CMD, &annotations.join("\n\n"));
        reset(self, t);
    }

    // fourslash.go:5328
    pub fn verify_baseline_linked_editing(&mut self, t: &T) {
        let mut baseline_builder = String::new();
        let mut offset = 0;

        // write to baseline in order of file appearance in test data
        for file in self.test_data.files.clone() {
            baseline_builder.push_str("// === Linked Editing ===\n");
            baseline_builder.push_str(&format!("=== {} ===\n", file.file_name()));
            let mut results: Vec<lsproto::LinkedEditingRanges> = Vec::new();
            let mut found: FxHashMap<lsproto::Range, bool> = FxHashMap::default();

            // request linkedEditing at every position in the file
            for i in 0..file.content.len() {
                let script = self.get_script_info(&file.file_name());
                let params = lsproto::LinkedEditingRangeParams {
                    text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&file.file_name()) },
                    position: self.converters.position_to_line_and_character(&script, i as TextPos),
                    ..Default::default()
                };
                let result = self.send_request(t, lsproto::TEXT_DOCUMENT_LINKED_EDITING_RANGE_INFO, params);
                if let Some(ranges) = result.linked_editing_ranges {
                    if !ranges.ranges.is_empty() && !found.get(&ranges.ranges[0]).copied().unwrap_or(false) {
                        found.insert(ranges.ranges[0], true);
                        results.push(ranges);
                    }
                }
            }

            if results.is_empty() {
                baseline_builder.push_str(&format!("{}\n\n--No linked edits found--\n\n\n", file.content));
                continue;
            }

            // sort entries in each file
            results.sort_by(|a, b| lsproto::compare_positions(a.ranges[0].start, b.ranges[0].start).cmp(&0));
            let mut baseline_details: Vec<(lsproto::Position, String)> = Vec::new();
            let mut found_edit_info_builder = String::new();
            for edit in &results {
                baseline_details.push((edit.ranges[0].start, format!("[|/*{offset}*/")));
                baseline_details.push((edit.ranges[0].end, "|]".to_string()));
                baseline_details.push((edit.ranges[1].start, format!("[|/*{offset}*/")));
                baseline_details.push((edit.ranges[1].end, "|]".to_string()));

                let json = tsrs_core::json::marshal_indent(&edit.to_json(), "", "  ").expect("stringify linked editing ranges");
                found_edit_info_builder.push_str(&format!("\n\n=== {offset} ===\n{json}"));
                offset += 1;
            }

            // sort baselineDetails by position
            baseline_details.sort_by(|a, b| lsproto::compare_positions(a.0, b.0).cmp(&0));

            // write file content with inline annotations for linked edits
            let mut last_position = 0usize;
            for (pos, position_marker) in &baseline_details {
                let current_position = self.converters.line_and_character_to_position(self.get_script_info(&file.file_name()), *pos) as usize;
                baseline_builder.push_str(&file.content[last_position..current_position]);
                baseline_builder.push_str(position_marker);
                last_position = current_position;
            }
            baseline_builder.push_str(&file.content[last_position..]);
            baseline_builder.push_str(&format!("{found_edit_info_builder}\n\n\n"));
        }

        self.write_to_baseline(LINKED_EDITING_CMD, &baseline_builder);
    }

    // fourslash.go:5407
    pub fn verify_linked_editing(&mut self, t: &T, marker_names_to_expected: OrderedMap<String, Vec<lsproto::Range>>) {
        for (marker_name, expected_ranges) in marker_names_to_expected.iter() {
            self.go_to_marker(t, marker_name);
            let params = lsproto::LinkedEditingRangeParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
                position: self.current_caret_position,
                ..Default::default()
            };
            let result = self.send_request(t, lsproto::TEXT_DOCUMENT_LINKED_EDITING_RANGE_INFO, params);
            let actual_ranges = result.linked_editing_ranges;
            if expected_ranges.is_empty() {
                if let Some(actual_ranges) = actual_ranges.as_ref().filter(|r| !r.ranges.is_empty()) {
                    t.fatal(&format!("Expected no linked editing ranges for marker '{marker_name}', but found {actual_ranges:?}"));
                }
                continue;
            } else {
                let Some(actual_ranges) = actual_ranges.filter(|r| !r.ranges.is_empty()) else {
                    t.fatal(&format!("Expected linked editing ranges for marker '{marker_name}', but found none"));
                };

                assert_deep_equal(
                    t,
                    &actual_ranges.ranges[0],
                    &expected_ranges[0],
                    &format!("Linked editing ranges for opening element do not match expected for marker '{marker_name}'"),
                );
                assert_deep_equal(
                    t,
                    &actual_ranges.ranges[1],
                    &expected_ranges[1],
                    &format!("Linked editing ranges for closing element do not match expected for marker '{marker_name}'"),
                );
            }
        }
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
        let original_preferences = self.user_preferences.clone();
        for test_case in cases {
            let preferences = test_case.preferences.clone().unwrap_or_else(lsutil::new_default_user_preferences);
            self.configure(t, preferences);
            let result = self.send_request(
                t,
                lsproto::WORKSPACE_SYMBOL_INFO,
                lsproto::WorkspaceSymbolParams {
                    query: test_case.pattern.clone(),
                    text_document: Some(lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) }),
                    ..Default::default()
                },
            );
            let Some(symbol_informations) = result.symbol_informations else {
                t.fatal("Expected non-nil symbol information array from workspace symbol request");
            };
            if let Some(includes) = &test_case.includes {
                if test_case.exact.is_some() {
                    t.fatal("Test case cannot have both 'Includes' and 'Exact' fields set");
                }
                verify_includes_symbols(t, &symbol_informations, includes, &format!("Workspace symbols mismatch with pattern '{}'", test_case.pattern));
            } else {
                let Some(exact) = &test_case.exact else {
                    t.fatal("Test case must have either 'Includes' or 'Exact' field set");
                };
                verify_exact_symbols(t, &symbol_informations, exact, &format!("Workspace symbols mismatch with pattern '{}'", test_case.pattern));
            }
        }
        self.configure(t, original_preferences);
    }

    // fourslash.go:5796
    pub fn verify_baseline_document_symbol(&mut self, t: &T) {
        let params = lsproto::DocumentSymbolParams {
            text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(&self.active_filename) },
            ..Default::default()
        };
        let result = self.send_request(t, lsproto::TEXT_DOCUMENT_DOCUMENT_SYMBOL_INFO, params);
        let uri = lsconv::file_name_to_document_uri(&self.active_filename);
        // Go collects into a map (random iteration order); the baseline orders spans by position.
        let mut symbol_by_span: OrderedMap<DocumentSpanKey, lsproto::DocumentSymbol> = OrderedMap::default();
        if let Some(document_symbols) = &result.document_symbols {
            for symbol in document_symbols {
                collect_document_symbol_spans(&uri, symbol, &mut symbol_by_span);
            }
        }
        let mut spans = Vec::with_capacity(symbol_by_span.len());
        for (key, symbol) in symbol_by_span.iter() {
            spans.push(DocumentSpan { uri: key.uri.clone(), text_span: key.text_span, context_span: Some(symbol.range) });
        }
        let baseline = self.get_baseline_for_spans_with_file_contents(
            &spans,
            BaselineFourslashLocationsOptions {
                get_location_data: Some(Box::new(move |span: &DocumentSpan| {
                    let symbol = &symbol_by_span[&DocumentSpanKey { uri: span.uri.clone(), text_span: span.text_span, context_span: span.context_span.unwrap() }];
                    format!("{{| name: {}, kind: {} |}}", symbol.name, symbol.kind.string())
                })),
                ..Default::default()
            },
        );
        self.add_result_to_baseline(t, DOCUMENT_SYMBOLS_CMD, &baseline);

        let mut details_builder = String::new();
        if let Some(document_symbols) = &result.document_symbols {
            write_document_symbol_details(document_symbols, 0, &mut details_builder);
        }
        self.write_to_baseline(DOCUMENT_SYMBOLS_CMD, &format!("\n\n// === Details ===\n{details_builder}"));
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

// fourslash.go:1306
fn find_jsdoc_completion_item(list: Option<&lsproto::CompletionList>) -> Option<lsproto::CompletionItem> {
    let list = list?;
    list.items.iter().find(|item| item.label == "/** */").cloned()
}

// fourslash.go:1377
fn is_empty_expected_list(expected: Option<&CompletionsExpectedList>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    // (Go dereferences expected.Items, which panics when it is nil.)
    let Some(items) = &expected.items else {
        panic!("runtime error: invalid memory address or nil pointer dereference");
    };
    items.exact.as_ref().is_none_or(Vec::is_empty) && items.includes.is_empty() && items.excludes.is_empty() && items.unsorted.as_ref().is_none_or(Vec::is_empty)
}

// fourslash.go:1381
fn verify_completions_item_defaults(t: &T, actual: Option<&lsproto::CompletionItemDefaults>, expected: Option<&CompletionsExpectedItemDefaults>, prefix: &str) {
    let Some(actual) = actual else {
        if expected.is_none() {
            return;
        }
        t.fatal(&format!("{prefix}Expected non-nil completion item defaults but got nil"));
    };
    let Some(expected) = expected else {
        t.fatal(&format!("{prefix}Expected nil completion item defaults but got non-nil: {actual:?}"));
    };
    assert_deep_equal(t, &actual.commit_characters, &expected.commit_characters, &format!("{prefix}CommitCharacters mismatch:"));
    match &expected.edit_range {
        Any::EditRange(edit_range) => {
            if actual.edit_range.is_none() {
                t.fatal(&format!("{prefix}Expected non-nil EditRange but got nil"));
            }
            // (Go dereferences the range markers, which panics when one is nil.)
            let nil = || -> ! { panic!("runtime error: invalid memory address or nil pointer dereference") };
            let expected_insert = edit_range.insert.as_ref().unwrap_or_else(|| nil()).ls_range;
            let expected_replace = edit_range.replace.as_ref().unwrap_or_else(|| nil()).ls_range;
            assert_deep_equal(
                t,
                &actual.edit_range,
                &Some(lsproto::RangeOrEditRangeWithInsertReplace {
                    edit_range_with_insert_replace: Some(lsproto::EditRangeWithInsertReplace { insert: expected_insert, replace: expected_replace }),
                    ..Default::default()
                }),
                &format!("{prefix}EditRange mismatch:"),
            );
        }
        Any::Nil => {
            if let Some(edit_range) = &actual.edit_range {
                t.fatal(&format!("{prefix}Expected nil EditRange but got non-nil: {edit_range:?}"));
            }
        }
        Any::Ignored => {
            // The edit range is intentionally ignored.
        }
        other => t.fatal(&format!("{prefix}Expected EditRange to be *EditRange or Ignored, got {}", other.type_name())),
    }
}

// cmp.Diff's report is replaced by both values' Debug forms ("" when equal).
fn diff<V: PartialEq + std::fmt::Debug>(actual: &V, expected: &V) -> String {
    if actual == expected {
        return String::new();
    }
    format!("-actual:   {actual:?}\n+expected: {expected:?}")
}

// The `.Kind` path of ignorePaths also matches the documentation's MarkupContent.Kind.
fn ignore_markup_kind(item: &mut lsproto::CompletionItem) {
    if let Some(markup_content) = item.documentation.as_mut().and_then(|d| d.markup_content.as_mut()) {
        markup_content.kind = lsproto::MarkupKind::PlainText;
    }
}

// fourslash.go:1569 (completionIgnoreOpts)
fn completion_ignore(item: &lsproto::CompletionItem) -> lsproto::CompletionItem {
    let mut item = lsproto::CompletionItem { kind: None, sort_text: None, filter_text: None, data: None, additional_text_edits: None, ..item.clone() };
    ignore_markup_kind(&mut item);
    item
}

// fourslash.go:1570 (autoImportIgnoreOpts)
fn auto_import_ignore(item: &lsproto::CompletionItem) -> lsproto::CompletionItem {
    let mut item =
        lsproto::CompletionItem { kind: None, sort_text: None, filter_text: None, data: None, label_details: None, detail: None, additional_text_edits: None, ..item.clone() };
    ignore_markup_kind(&mut item);
    item
}

// The text of a signature or parameter documentation (markup value or plain string; "" when absent).
fn documentation_text(documentation: &Option<lsproto::StringOrMarkupContent>) -> String {
    let Some(documentation) = documentation else {
        return String::new();
    };
    if let Some(markup_content) = &documentation.markup_content {
        return markup_content.value.clone();
    }
    documentation.string.clone().unwrap_or_default()
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

// fourslash.go:5757
fn verify_exact_symbols(t: &T, actual: &[lsproto::SymbolInformation], expected: &[lsproto::SymbolInformation], prefix: &str) {
    if actual.len() != expected.len() {
        t.fatal(&format!("{}: Expected {} symbols, but got {}:\n{:?}\n{:?}", prefix, expected.len(), actual.len(), actual, expected));
    }
    for i in 0..actual.len() {
        assert_deep_equal(t, &actual[i], &expected[i], prefix);
    }
}

// fourslash.go:5771
fn verify_includes_symbols(t: &T, actual: &[lsproto::SymbolInformation], includes: &[lsproto::SymbolInformation], prefix: &str) {
    let mut name_and_loc_to_actual_symbol: FxHashMap<(String, lsproto::Location), &lsproto::SymbolInformation> = FxHashMap::default();
    for sym in actual {
        name_and_loc_to_actual_symbol.insert((sym.name.clone(), sym.location.clone()), sym);
    }

    for sym in includes {
        let Some(actual_sym) = name_and_loc_to_actual_symbol.get(&(sym.name.clone(), sym.location.clone())) else {
            t.fatal(&format!("{}: Expected symbol '{}' at location '{:?}' not found", prefix, sym.name, sym.location));
        };
        assert_deep_equal(t, *actual_sym, sym, &format!("{}: Symbol '{}' at location '{:?}' mismatch", prefix, sym.name, sym.location));
    }
}

// fourslash.go:5830
fn write_document_symbol_details(symbols: &[lsproto::DocumentSymbol], indent: usize, builder: &mut String) {
    for symbol in symbols {
        builder.push_str(&format!("{}({}) {}\n", "  ".repeat(indent), symbol.kind.string(), symbol.name));
        if let Some(children) = &symbol.children {
            write_document_symbol_details(children, indent + 1, builder);
        }
    }
}

// fourslash.go:5839
fn collect_document_symbol_spans(uri: &lsproto::DocumentUri, symbol: &lsproto::DocumentSymbol, symbol_by_span: &mut OrderedMap<DocumentSpanKey, lsproto::DocumentSymbol>) {
    // Deduplicate by value rather than by the documentSpan key, which holds a pointer to
    // the symbol's Range. The same logical symbol can be reached more than once
    // (e.g. a merged declaration), and depending on transport those occurrences may
    // be the same object (shared *Range) or independent copies (distinct *Range
    // after a JSON round-trip). A value-based key collapses them consistently.
    let key = DocumentSpanKey { uri: uri.clone(), text_span: symbol.selection_range, context_span: symbol.range };
    if !symbol_by_span.contains_key(&key) {
        symbol_by_span.insert(key, symbol.clone());
    }
    if let Some(children) = &symbol.children {
        for child in children {
            collect_document_symbol_spans(uri, child, symbol_by_span);
        }
    }
}

// documentSpanKey is a value-comparable variant of documentSpan used to deduplicate
// document symbols regardless of pointer identity.
// fourslash.go:5863
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DocumentSpanKey {
    uri: lsproto::DocumentUri,
    text_span: lsproto::Range,
    context_span: lsproto::Range,
}

// Go unicode.IsSpace (strings.TrimSpace).
fn go_is_space(r: char) -> bool {
    matches!(r, '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{85}' | '\u{A0}') || (!r.is_ascii() && r.is_whitespace())
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
