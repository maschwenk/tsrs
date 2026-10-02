// Port of Go's fourslash/fourslash.go: the FourslashTest harness the generated tests drive.
//
// The in-process language server (Go: testutil/lsptestutil.LSPClient over internal/lsp) is not available yet.
// Everything that does not need it is ported; every method that talks to the server calls
// `FourslashTest::server_unavailable` (the one place to replace when the client exists).

use std::sync::{Arc, LazyLock, OnceLock, RwLock};

use rustc_hash::FxHashMap;
use tsrs_core::collections::{MultiMap, OrderedMap};
use tsrs_core::stringutil;
use tsrs_core::tspath;
use tsrs_core::{TextChange, TextPos};
use tsrs_ls::lsconv::{self, Converters, LSPLineMap, Script};
use tsrs_ls::lsutil;
use tsrs_ls::spanmap::{Feature, SpanMap};
use tsrs_lsproto as lsproto;

pub use crate::baselineutil::*;
pub use crate::contentmapper::Spawner;
pub use crate::go::Any;
pub use crate::semantictokens::*;
pub use crate::statebaseline::*;
pub use crate::test_parser::*;
use crate::testing::T;

// fourslash.go:46
pub struct FourslashTest {
    // !!! client *lsptestutil.LSPClient and vfs vfs.FS: the in-process server is not ported yet.
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

    // !!! compiler options for the inferred project, harnessutil.SetOptionsFromTestConfig, `@tsc` command lines,
    // harnessutil.SkipUnsupportedCompilerOptions, vfstest.FromMap + bundled.WrapFS and lsp.ServerOptions: all of
    // this configures the in-process server.
    let _ = (&testfs, &options, test_path);

    let script_infos = Arc::new(RwLock::new(script_infos));
    let shared = script_infos.clone();
    let converters = new_test_converters(lsconv::new_converters(lsproto::PositionEncodingKind::UTF8, move |file_name| {
        shared.read().unwrap().get(file_name).map(|s| s.line_map.clone())
    }));

    let f = FourslashTest {
        test_data,
        state_enable_formatting: true,
        report_format_on_type_crash: true,
        user_preferences: lsutil::new_default_user_preferences(),
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
    // client, closeClient := lsptestutil.NewLSPClient(t, serverOpts, f.handleServerRequest)
    // client.SetCompilerOptionsForInferredProjects(compilerOptions); f.initialize(t, options); state baseline or
    // open the files; then `return (f, new_done_fn(test_path))`.
    let _ = f;
    FourslashTest::server_unavailable(t, "NewFourslash")
}

// fourslash.go:280: the `done` closure NewFourslash returns.
fn new_done_fn(test_path: &str) -> DoneFn {
    let test_path = test_path.to_string();
    Box::new(move |f: &mut FourslashTest, t: &T| {
        t.helper();
        // !!! err := closeClient(); if err != nil { t.Errorf("goroutine error: %v", err) }
        f.verify_baselines(t, &test_path);
    })
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
    // The single place where the missing in-process language server surfaces: every harness operation that
    // would send a request or notification ends here.
    pub fn server_unavailable(t: &T, method: &str) -> ! {
        t.fatal(&format!("server unavailable: {method} needs the in-process language server (not ported yet)"))
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
        self.user_preferences = config;
        Self::server_unavailable(t, "Configure")
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
        Self::server_unavailable(t, "CloseFileOfMarker")
    }

    // fourslash.go:997
    fn open_file(&mut self, t: &T, filename: &str) {
        Self::server_unavailable(t, "openFile (textDocument/didOpen)")
    }

    // fourslash.go:1018
    pub fn format_document(&mut self, t: &T, filename: &str) {
        Self::server_unavailable(t, "FormatDocument")
    }

    // fourslash.go:1034
    pub fn format_selection(&mut self, t: &T, start_marker_name: &str, end_marker_name: &str) {
        Self::server_unavailable(t, "FormatSelection")
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
        Self::server_unavailable(t, "VerifyCompletions")
    }

    // fourslash.go:1245
    pub fn get_completions(&mut self, t: &T, user_preferences: Option<lsutil::UserPreferences>) -> Option<lsproto::CompletionList> {
        Self::server_unavailable(t, "GetCompletions")
    }

    // fourslash.go:1250
    pub fn verify_jsdoc_completion(&mut self, t: &T, marker_input: Any, expected_offset: i32, expected_text: &str, generate_return_in_doc_template: Option<bool>) {
        Self::server_unavailable(t, "VerifyJSDocCompletion")
    }

    // fourslash.go:1285
    pub fn verify_no_jsdoc_completion(&mut self, t: &T, marker_input: Any) {
        Self::server_unavailable(t, "VerifyNoJSDocCompletion")
    }

    // fourslash.go:1655
    pub fn resolve_completion_item(&mut self, t: &T, item: Option<lsproto::CompletionItem>) -> Option<lsproto::CompletionItem> {
        Self::server_unavailable(t, "ResolveCompletionItem")
    }

    // fourslash.go:1691
    pub fn verify_code_fix(&mut self, t: &T, options: VerifyCodeFixOptions) {
        Self::server_unavailable(t, "VerifyCodeFix")
    }

    // fourslash.go:1768
    pub fn verify_range_after_code_fix(&mut self, t: &T, expected_text: &str, include_whitespace: bool, error_code: i32, index: i32) {
        Self::server_unavailable(t, "VerifyRangeAfterCodeFix")
    }

    // fourslash.go:1822
    pub fn verify_code_fix_available(&mut self, t: &T, expected_descriptions: &[&str]) {
        Self::server_unavailable(t, "VerifyCodeFixAvailable")
    }

    // fourslash.go:1857
    pub fn verify_code_fix_not_available(&mut self, t: &T, expected: &[&str]) {
        Self::server_unavailable(t, "VerifyCodeFixNotAvailable")
    }

    // fourslash.go:1884
    pub fn verify_code_fix_available_exact(&mut self, t: &T, expected_descriptions: &[&str]) {
        Self::server_unavailable(t, "VerifyCodeFixAvailableExact")
    }

    // fourslash.go:1918
    pub fn verify_code_fix_all(&mut self, t: &T, options: VerifyCodeFixAllOptions) {
        Self::server_unavailable(t, "VerifyCodeFixAll")
    }

    // fourslash.go:1973
    pub fn verify_source_fix_all(&mut self, t: &T, expected_content: &str) {
        Self::server_unavailable(t, "VerifySourceFixAll")
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
        Self::server_unavailable(t, "VerifyOrganizeImportsWithRequestKind")
    }

    // fourslash.go:2241
    pub fn verify_apply_code_action_from_completion(&mut self, t: &T, marker_name: Option<String>, options: Option<ApplyCodeActionFromCompletionOptions>) {
        Self::server_unavailable(t, "VerifyApplyCodeActionFromCompletion")
    }

    // fourslash.go:2291
    pub fn verify_import_fix_at_position(&mut self, t: &T, expected_texts: &[&str], preferences: Option<lsutil::UserPreferences>) {
        Self::server_unavailable(t, "VerifyImportFixAtPosition")
    }

    // fourslash.go:2414
    pub fn verify_import_fix_module_specifiers(
        &mut self,
        t: &T,
        marker_name: &str,
        expected_module_specifiers: &[&str],
        preferences: Option<lsutil::UserPreferences>,
    ) {
        Self::server_unavailable(t, "VerifyImportFixModuleSpecifiers")
    }

    // fourslash.go:2523
    pub fn verify_baseline_find_all_references(&mut self, t: &T, markers: &[&str]) {
        Self::server_unavailable(t, "VerifyBaselineFindAllReferences")
    }

    // fourslash.go:2551
    pub fn verify_baseline_vs_find_all_references(&mut self, t: &T, markers: &[&str]) {
        Self::server_unavailable(t, "VerifyBaselineVSFindAllReferences")
    }

    // fourslash.go:2631
    pub fn verify_baseline_code_lens(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>) {
        Self::server_unavailable(t, "VerifyBaselineCodeLens")
    }

    // fourslash.go:2691
    pub fn mark_test_as_strada_server(&mut self) {
        self.is_strada_server = true;
    }

    // fourslash.go:2695
    pub fn verify_baseline_go_to_definition(&mut self, t: &T, include_original_selection_range: bool, markers: &[&str]) {
        Self::server_unavailable(t, "VerifyBaselineGoToDefinition")
    }

    // fourslash.go:2775
    pub fn verify_baseline_go_to_type_definition(&mut self, t: &T, markers: &[&str]) {
        Self::server_unavailable(t, "VerifyBaselineGoToTypeDefinition")
    }

    // fourslash.go:2798
    pub fn verify_baseline_go_to_source_definition(&mut self, t: &T, markers: &[&str]) {
        Self::server_unavailable(t, "VerifyBaselineGoToSourceDefinition")
    }

    // fourslash.go:2825
    pub fn verify_baseline_workspace_symbol(&mut self, t: &T, query: &str) {
        Self::server_unavailable(t, "VerifyBaselineWorkspaceSymbol")
    }

    // fourslash.go:2850
    pub fn verify_outlining_spans(&mut self, t: &T, folding_range_kind: &[lsproto::FoldingRangeKind]) {
        Self::server_unavailable(t, "VerifyOutliningSpans")
    }

    // fourslash.go:2907
    pub fn verify_folding_range_lines(&mut self, t: &T, expected: &[FoldingRangeLineExpected]) {
        Self::server_unavailable(t, "VerifyFoldingRangeLines")
    }

    // fourslash.go:2932
    pub fn verify_baseline_hover(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyBaselineHover")
    }

    // fourslash.go:2992
    pub fn verify_baseline_vs_hover(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyBaselineVSHover")
    }

    // fourslash.go:3084
    pub fn verify_baseline_hover_with_verbosity(&mut self, t: &T, verbosity_levels: OrderedMap<String, Vec<i32>>) {
        Self::server_unavailable(t, "VerifyBaselineHoverWithVerbosity")
    }

    // fourslash.go:3173
    pub fn verify_baseline_signature_help(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyBaselineSignatureHelp")
    }

    // fourslash.go:3283
    pub fn verify_baseline_selection_ranges(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyBaselineSelectionRanges")
    }

    // fourslash.go:3421
    pub fn verify_baseline_call_hierarchy(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyBaselineCallHierarchy")
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
        Self::server_unavailable(t, "VerifyBaselineDocumentHighlightsWithOptions")
    }

    // fourslash.go:3890
    pub fn insert(&mut self, t: &T, text: &str) {
        Self::server_unavailable(t, "Insert")
    }

    // fourslash.go:3897
    pub fn insert_line(&mut self, t: &T, text: &str) {
        Self::server_unavailable(t, "InsertLine")
    }

    // fourslash.go:3904
    pub fn backspace(&mut self, t: &T, count: i32) {
        Self::server_unavailable(t, "Backspace")
    }

    // fourslash.go:3920
    pub fn delete_at_caret(&mut self, t: &T, count: i32) {
        Self::server_unavailable(t, "DeleteAtCaret")
    }

    // fourslash.go:3932
    pub fn paste(&mut self, t: &T, text: &str) {
        Self::server_unavailable(t, "Paste")
    }

    // fourslash.go:3958
    pub fn replace_line(&mut self, t: &T, line_index: i32, text: &str) {
        Self::server_unavailable(t, "ReplaceLine")
    }

    // fourslash.go:4032
    pub fn replace(&mut self, t: &T, start: i32, length: i32, text: &str) {
        Self::server_unavailable(t, "Replace")
    }

    // fourslash.go:4183
    pub fn verify_quick_info_at(&mut self, t: &T, marker: &str, expected_text: &str, expected_documentation: &str) {
        Self::server_unavailable(t, "VerifyQuickInfoAt")
    }

    // fourslash.go:4229
    pub fn verify_quick_info_exists(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyQuickInfoExists")
    }

    // fourslash.go:4235
    pub fn verify_not_quick_info_exists(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyNotQuickInfoExists")
    }

    // fourslash.go:4250
    pub fn verify_quick_info_is(&mut self, t: &T, expected_text: &str, expected_documentation: &str) {
        Self::server_unavailable(t, "VerifyQuickInfoIs")
    }

    // fourslash.go:4255
    pub fn verify_jsx_closing_tag(&mut self, t: &T, markers_to_new_text: OrderedMap<String, Option<String>>) {
        Self::server_unavailable(t, "VerifyJsxClosingTag")
    }

    // fourslash.go:4285
    pub fn verify_baseline_closing_tags(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyBaselineClosingTags")
    }

    // fourslash.go:4353
    pub fn verify_signature_help(&mut self, t: &T, expected: VerifySignatureHelpOptions) {
        Self::server_unavailable(t, "VerifySignatureHelp")
    }

    // fourslash.go:4513
    pub fn verify_no_signature_help(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyNoSignatureHelp")
    }

    // fourslash.go:4529
    pub fn verify_no_signature_help_with_context(&mut self, t: &T, context: Option<lsproto::SignatureHelpContext>) {
        Self::server_unavailable(t, "VerifyNoSignatureHelpWithContext")
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
        Self::server_unavailable(t, "VerifySignatureHelpPresent")
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
        Self::server_unavailable(t, "VerifySignatureHelpWithCases")
    }

    // fourslash.go:4657
    pub fn baseline_auto_imports_completions(&mut self, t: &T, marker_names: &[&str]) {
        Self::server_unavailable(t, "BaselineAutoImportsCompletions")
    }

    // fourslash.go:4754
    pub fn verify_baseline_rename(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>, marker_or_name_or_ranges: &[Any]) {
        Self::server_unavailable(t, "VerifyBaselineRename")
    }

    // fourslash.go:4862
    pub fn verify_rename_succeeded(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>) {
        Self::server_unavailable(t, "VerifyRenameSucceeded")
    }

    // fourslash.go:4892
    pub fn verify_rename_range(&mut self, t: &T, expected_range: lsproto::Range, expected_placeholder: &str, preferences: Option<lsutil::UserPreferences>) {
        Self::server_unavailable(t, "VerifyRenameRange")
    }

    // fourslash.go:4912
    pub fn rename_at_caret(&mut self, t: &T, new_name: &str) -> lsproto::RenameResponse {
        Self::server_unavailable(t, "RenameAtCaret")
    }

    // fourslash.go:4984
    pub fn will_rename_files(&mut self, t: &T, files: &[lsproto::FileRename]) -> lsproto::WillRenameFilesResponse {
        Self::server_unavailable(t, "WillRenameFiles")
    }

    // fourslash.go:5055
    pub fn verify_rename(&mut self, t: &T, marker_name: &str, new_name: &str, expected_file_contents: OrderedMap<String, String>) {
        Self::server_unavailable(t, "VerifyRename")
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
        Self::server_unavailable(t, "VerifyWillRenameFilesEdits")
    }

    // fourslash.go:5186
    pub fn verify_rename_failed(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>) {
        Self::server_unavailable(t, "VerifyRenameFailed")
    }

    // fourslash.go:5226
    pub fn verify_baseline_rename_at_ranges_with_text(&mut self, t: &T, preferences: Option<lsutil::UserPreferences>, texts: &[&str]) {
        Self::server_unavailable(t, "VerifyBaselineRenameAtRangesWithText")
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
        Self::server_unavailable(t, "VerifyBaselineInlayHints")
    }

    // fourslash.go:5328
    pub fn verify_baseline_linked_editing(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyBaselineLinkedEditing")
    }

    // fourslash.go:5407
    pub fn verify_linked_editing(&mut self, t: &T, marker_names_to_expected: OrderedMap<String, Vec<lsproto::Range>>) {
        Self::server_unavailable(t, "VerifyLinkedEditing")
    }

    // fourslash.go:5434
    pub fn verify_diagnostics(&mut self, t: &T, expected: &[lsproto::Diagnostic]) {
        Self::server_unavailable(t, "VerifyDiagnostics")
    }

    // fourslash.go:5439
    pub fn verify_non_suggestion_diagnostics(&mut self, t: &T, expected: &[lsproto::Diagnostic]) {
        Self::server_unavailable(t, "VerifyNonSuggestionDiagnostics")
    }

    // fourslash.go:5444
    pub fn verify_suggestion_diagnostics(&mut self, t: &T, expected: &[lsproto::Diagnostic]) {
        Self::server_unavailable(t, "VerifySuggestionDiagnostics")
    }

    // fourslash.go:5489
    pub fn verify_baseline_non_suggestion_diagnostics(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyBaselineNonSuggestionDiagnostics")
    }

    // fourslash.go:5698
    pub fn verify_baseline_go_to_implementation(&mut self, t: &T, marker_names: &[&str]) {
        Self::server_unavailable(t, "VerifyBaselineGoToImplementation")
    }

    // fourslash.go:5726
    pub fn verify_workspace_symbol(&mut self, t: &T, cases: &[VerifyWorkspaceSymbolCase]) {
        Self::server_unavailable(t, "VerifyWorkspaceSymbol")
    }

    // fourslash.go:5796
    pub fn verify_baseline_document_symbol(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyBaselineDocumentSymbol")
    }

    // fourslash.go:5871
    pub fn verify_number_of_errors_in_current_file(&mut self, t: &T, expected_count: i32) {
        Self::server_unavailable(t, "VerifyNumberOfErrorsInCurrentFile")
    }

    // fourslash.go:5883
    pub fn verify_no_errors(&mut self, t: &T) {
        Self::server_unavailable(t, "VerifyNoErrors")
    }

    // fourslash.go:5901
    pub fn verify_error_exists_at_range(&mut self, t: &T, range_marker: Arc<RangeMarker>, code: i32, message: &str) {
        Self::server_unavailable(t, "VerifyErrorExistsAtRange")
    }

    // fourslash.go:5922
    pub fn verify_error_exists_between_markers(&mut self, t: &T, start_marker_name: &str, end_marker_name: &str) {
        Self::server_unavailable(t, "VerifyErrorExistsBetweenMarkers")
    }

    // fourslash.go:5952
    pub fn verify_error_exists_after_marker(&mut self, t: &T, marker_name: &str) {
        Self::server_unavailable(t, "VerifyErrorExistsAfterMarker")
    }

    // fourslash.go:5983
    pub fn verify_error_exists_before_marker(&mut self, t: &T, marker_name: &str) {
        Self::server_unavailable(t, "VerifyErrorExistsBeforeMarker")
    }

    // fourslash.go:4166 (getScriptInfo): Go returns the shared *scriptInfo; the Rust caller gets a snapshot.
    pub(crate) fn get_script_info(&self, file_name: &str) -> ScriptInfo {
        match self.script_infos.read().unwrap().get(file_name) {
            Some(s) => s.clone(),
            None => panic!("script info for {file_name} not found"),
        }
    }
}
