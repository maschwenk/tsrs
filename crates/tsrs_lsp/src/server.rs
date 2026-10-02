use std::any::Any;
use std::io::{Read, Write};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, OnceLock, RwLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_core::context::{self, CancelCauseFunc, CancelFunc, Context, ContextError, Locale};
use tsrs_lsproto as lsproto;
use tsrs_lsproto::jsonrpc::{self, MessageKind, ID};
use tsrs_lsproto::{Error, ErrorCode, ErrorTag, Json, Message, Method, RequestMessage, ResponseMessage};
use tsrs_core::collections::OrderedMap;
use tsrs_core::CompilerOptions;
use tsrs_core::collections::Set;
use tsrs_core::tspath::{self, Path};
use tsrs_core::P;
use tsrs_ls::{lsutil, LanguageService};
use tsrs_project as project;
use tsrs_project::logging;
use tsrs_vfs::FS;

use crate::dynamic_queue::{dynamicQueue, new_dynamic_queue, wait_until};
use crate::logger::{is_valid_log_verbosity, logger, new_logger};
use crate::lsconsts;
use crate::progress::{new_project_loading_progress, projectLoadingProgress};
use crate::stack_sanitizer::sanitize_stack_trace;
use crate::workerpool;

// server.go:42
pub struct ServerOptions {
    pub in_: Box<dyn Reader>,
    pub out: Box<dyn Writer>,
    pub err: Box<dyn Write + Send>,

    pub cwd: String,
    pub fs: Option<Arc<dyn FS>>,
    pub default_library_path: String,
    pub typings_location: String,
    pub parse_cache: Option<Arc<project::ParseCache>>,
    pub npm_install: Option<npmInstallFunc>,
    // Spawn (content mappers) is not ported: content mappers are out of scope (docs/LSP.md).
    pub progress_delay: Duration, // delay before showing progress UI; 0 means no delay
    pub set_parent_process_id: Option<Box<dyn Fn(i32) + Send + Sync>>,
}

pub type npmInstallFunc = Arc<dyn Fn(&str, &[String]) -> Result<Vec<u8>, String> + Send + Sync>;

// server.go:60
pub fn new_server(opts: ServerOptions) -> Arc<Server> {
    if opts.cwd.is_empty() {
        panic!("Cwd is required");
    }

    Arc::new_cyclic(|weak: &Weak<Server>| Server {
        r: Mutex::new(Some(opts.in_)),
        w: Mutex::new(opts.out),
        background_ctx: OnceLock::new(),
        stderr: Mutex::new(opts.err),
        logger: Arc::new(new_logger(weak.clone())),
        init_started: AtomicBool::new(false),
        client_seq: AtomicI32::new(0),
        request_queue: new_dynamic_queue(),
        outgoing_queue: new_dynamic_queue(),
        pending_client_requests: Mutex::new(FxHashMap::default()),
        pending_server_requests: Mutex::new(FxHashMap::default()),
        cwd: opts.cwd,
        fs: opts.fs,
        default_library_path: opts.default_library_path,
        typings_location: opts.typings_location,
        initialize_params: OnceLock::new(),
        initialization_options: OnceLock::new(),
        client_capabilities: OnceLock::new(),
        position_encoding: OnceLock::new(),
        locale: RwLock::new(Locale::DEFAULT),
        init_locale: Mutex::new(Locale::DEFAULT),
        watch_enabled: AtomicBool::new(false),
        telemetry_enabled: AtomicBool::new(false),
        watcher_id: AtomicU32::new(0),
        watchers: Mutex::new(FxHashSet::default()),
        content_mapper_registration_mu: Mutex::new(false),
        builtin_watcher: None,
        last_request_time_ms: AtomicI64::new(0),
        init_complete: Context::background().with_cancel(),
        session: OnceLock::new(),
        compiler_options_for_inferred_projects: Mutex::new(None),
        parse_cache: opts.parse_cache,
        npm_install: opts.npm_install,
        progress_delay: opts.progress_delay,
        project_progress: OnceLock::new(),
        start_watchdog: opts.set_parent_process_id,
        flake_logging: OnceLock::new(),
    })
}

// server.go:90
fn file_rename_filters() -> Vec<lsproto::FileOperationFilter> {
    vec![lsproto::FileOperationFilter {
        scheme: Some("file".to_string()),
        pattern: lsproto::FileOperationPattern { glob: "**/*.{ts,tsx,js,jsx,cts,cjs,mts,mjs,json}".to_string(), ..Default::default() },
    }]
}

// server.go:102
struct pendingClientRequest {
    req: Arc<RequestMessage>,
    cancel: CancelFunc,
}

// server.go:107. Go returns `(msg, err)` where both may be set (an invalid-params error keeps the message).
pub trait Reader: Send {
    fn read(&mut self) -> (Option<Message>, Option<Error>);
}

// server.go:111
pub trait Writer: Send {
    fn write(&mut self, msg: &Message) -> Result<(), Error>;
}

// server.go:115
struct lspReader<R: Read> {
    r: lsproto::BaseReader<R>,
}

// server.go:119
struct lspWriter<W: Write> {
    w: lsproto::BaseWriter<W>,
}

// server.go:123: `*messageMarshalError` (`errors.As`), unwrapping to ErrorCodeInternalError and the cause.
const MESSAGE_MARSHAL_ERROR: ErrorTag = ErrorTag::Sentinel("messageMarshalError");

fn message_marshal_error(err: &str) -> Error {
    Error { message: format!("failed to marshal message: {}", err), tags: vec![MESSAGE_MARSHAL_ERROR, ErrorTag::Code(ErrorCode::InternalError)] }
}

impl<R: Read + Send> Reader for lspReader<R> {
    // server.go:133
    fn read(&mut self) -> (Option<Message>, Option<Error>) {
        let data = match self.r.read() {
            Ok(data) => data,
            Err(err) => return (None, Some(err)),
        };

        match lsproto::unmarshal::<Message>(&data) {
            Ok(req) => (Some(req), None),
            Err(err) => {
                let err = Error::from(err);
                if err.is_code(ErrorCode::InvalidParams) {
                    return (None, Some(Error::wrap_code(ErrorCode::InvalidParams, err)));
                }
                (None, Some(Error::wrap_code(ErrorCode::InvalidRequest, err)))
            }
        }
    }
}

// server.go:150
pub fn to_reader(r: impl Read + Send + 'static) -> Box<dyn Reader> {
    Box::new(lspReader { r: lsproto::new_base_reader(r) })
}

// encoding/json/v2's maximum nesting depth (jsontext: "exceeded max depth").
const MAX_NESTING_DEPTH: usize = 10000;

fn exceeds_max_depth(v: &lsproto::Value) -> bool {
    let mut stack: Vec<(&lsproto::Value, usize)> = vec![(v, 0)];
    while let Some((v, depth)) = stack.pop() {
        match v {
            lsproto::Value::Array(items) => {
                if depth + 1 > MAX_NESTING_DEPTH {
                    return true;
                }
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            lsproto::Value::Object(members) => {
                if depth + 1 > MAX_NESTING_DEPTH {
                    return true;
                }
                stack.extend(members.values().map(|item| (item, depth + 1)));
            }
            _ => {}
        }
    }
    false
}

impl<W: Write + Send> Writer for lspWriter<W> {
    // server.go:154
    fn write(&mut self, msg: &Message) -> Result<(), Error> {
        let value = msg.to_json();
        if exceeds_max_depth(&value) {
            return Err(message_marshal_error("jsontext: exceeded max depth"));
        }
        let data = tsrs_core::json::marshal(&value).map_err(|err| message_marshal_error(&err))?;
        self.w.write(data.as_bytes())
    }
}

// server.go:162
pub fn to_writer(w: impl Write + Send + 'static) -> Box<dyn Writer> {
    Box::new(lspWriter { w: lsproto::new_base_writer(w) })
}

// lspwatcher (the builtin in-process file watcher) is ported in phase 4. Until then no value of this type
// exists: `builtin_watcher` is always None and every "builtin watcher" branch keeps Go's structure.
pub(crate) enum builtinWatcher {}

// server.go:171
pub struct Server {
    r: Mutex<Option<Box<dyn Reader>>>,
    w: Mutex<Box<dyn Writer>>,
    pub(crate) background_ctx: OnceLock<Context>,

    stderr: Mutex<Box<dyn Write + Send>>,

    pub(crate) logger: Arc<logger>,
    pub(crate) init_started: AtomicBool,
    client_seq: AtomicI32,
    request_queue: dynamicQueue<Arc<RequestMessage>>,
    pub(crate) outgoing_queue: dynamicQueue<Message>,
    pending_client_requests: Mutex<FxHashMap<ID, pendingClientRequest>>,
    pending_server_requests: Mutex<FxHashMap<ID, responseChan>>,

    cwd: String,
    fs: Option<Arc<dyn FS>>,
    default_library_path: String,
    typings_location: String,

    initialize_params: OnceLock<Arc<lsproto::InitializeParams>>,
    initialization_options: OnceLock<lsproto::InitializationOptions>,
    client_capabilities: OnceLock<Arc<lsproto::ResolvedClientCapabilities>>,
    position_encoding: OnceLock<lsproto::PositionEncodingKind>,
    locale: RwLock<Locale>,
    // initLocale is the locale resolved from the initialize request; it is
    // used as the fallback when the user's locale preference is "auto".
    init_locale: Mutex<Locale>,

    watch_enabled: AtomicBool,
    telemetry_enabled: AtomicBool,
    watcher_id: AtomicU32,
    watchers: Mutex<FxHashSet<String>>,

    // contentMapperRegistrationMu serializes RegisterContentMapperExtensions so the method is correctly
    // synchronized on its own rather than relying on callers to serialize it. It guards
    // contentMapperExtensionsRegistered and the ordered unregister/register requests the method sends.
    // (The bool it guards is contentMapperExtensionsRegistered: whether a content mapper text document sync
    // registration is currently active with the client, so it can be replaced or removed.)
    content_mapper_registration_mu: Mutex<bool>,
    // builtinWatcher is non-nil when the server is running its own
    // in-process file watcher instead of using LSP-based watching. It
    // is enabled when the client lacks DynamicRegistration for
    // workspace/didChangeWatchedFiles and the builtin watcher backend
    // supports efficient recursive watching (Windows or FSEvents).
    builtin_watcher: Option<builtinWatcher>,

    last_request_time_ms: AtomicI64,

    // initComplete is closed when handleInitialized completes.
    // Used by tests to wait for full initialization. (A context canceled when it is "closed".)
    init_complete: (Context, CancelFunc),

    session: OnceLock<Arc<project::Session>>,

    // !!! temporary; remove when we have `handleDidChangeConfiguration`/implicit project config support
    compiler_options_for_inferred_projects: Mutex<Option<P<CompilerOptions>>>,
    // parseCache can be passed in so separate tests can share ASTs
    parse_cache: Option<Arc<project::ParseCache>>,

    npm_install: Option<npmInstallFunc>,

    progress_delay: Duration,
    project_progress: OnceLock<Arc<projectLoadingProgress>>,

    start_watchdog: Option<Box<dyn Fn(i32) + Send + Sync>>,

    flake_logging: OnceLock<lsproto::DiagnosticFlakeLogLevel>,
}

// Go's `chan *lsproto.ResponseMessage` with buffer 1 in pendingServerRequests.
#[derive(Clone)]
struct responseChan {
    inner: Arc<(Mutex<responseChanState>, Condvar)>,
}

#[derive(Default)]
struct responseChanState {
    value: Option<ResponseMessage>,
    closed: bool,
}

impl responseChan {
    fn new() -> responseChan {
        responseChan { inner: Arc::new((Mutex::new(responseChanState::default()), Condvar::new())) }
    }

    fn send(&self, resp: ResponseMessage) {
        self.inner.0.lock().unwrap().value = Some(resp);
        self.inner.1.notify_all();
    }

    fn close(&self) {
        self.inner.0.lock().unwrap().closed = true;
        self.inner.1.notify_all();
    }

    // `select { case <-ctx.Done(): ...; case resp := <-responseChan: ... }`; Ok(None) is a receive from the
    // closed, empty channel (Go: a nil response).
    fn recv(&self, ctx: &Context) -> Result<Option<ResponseMessage>, ContextError> {
        let inner = self.inner.clone();
        let guard = self.inner.0.lock().unwrap();
        let mut state = wait_until(ctx, &self.inner.1, guard, |s| s.value.is_some() || s.closed, move || {
            drop(inner.0.lock().unwrap());
            inner.1.notify_all();
        })?;
        Ok(state.value.take())
    }
}

static DEFAULT_CLIENT_CAPABILITIES: LazyLock<Arc<lsproto::ResolvedClientCapabilities>> = LazyLock::new(|| Arc::new(lsproto::ResolvedClientCapabilities::default()));

pub(crate) type asyncWork = Box<dyn FnOnce() -> Result<(), Error> + Send>;

impl Server {
    // The server's lifecycle context (Go `s.backgroundCtx`, set by Run).
    pub(crate) fn background_ctx(&self) -> Context {
        self.background_ctx.get().cloned().unwrap_or_default()
    }

    pub(crate) fn write_stderr_line(&self, message: &str) {
        let mut stderr = self.stderr.lock().unwrap();
        let _ = writeln!(stderr, "{}", message);
        let _ = stderr.flush();
    }

    // Go `s.clientCapabilities` (zero until initialize).
    pub(crate) fn client_capabilities(&self) -> &lsproto::ResolvedClientCapabilities {
        self.client_capabilities.get().map(|c| &**c).unwrap_or(&DEFAULT_CLIENT_CAPABILITIES)
    }

    fn client_capabilities_arc(&self) -> Arc<lsproto::ResolvedClientCapabilities> {
        self.client_capabilities.get().cloned().unwrap_or_else(|| DEFAULT_CLIENT_CAPABILITIES.clone())
    }

    fn initialization_options(&self) -> &lsproto::InitializationOptions {
        static EMPTY: LazyLock<lsproto::InitializationOptions> = LazyLock::new(Default::default);
        self.initialization_options.get().unwrap_or(&EMPTY)
    }

    fn position_encoding(&self) -> lsproto::PositionEncodingKind {
        self.position_encoding.get().copied().unwrap_or_default()
    }

    fn flake_logging(&self) -> lsproto::DiagnosticFlakeLogLevel {
        self.flake_logging.get().copied().unwrap_or_default()
    }

    // server.go:255
    pub fn session(&self) -> &Arc<project::Session> {
        self.session.get().expect("the session is created by the initialized notification")
    }

    #[cfg(test)]
    pub(crate) fn set_session_for_test(&self, session: Arc<project::Session>) {
        let _ = self.session.set(session);
    }

    pub(crate) fn logger_arc(&self) -> Arc<logger> {
        self.logger.clone()
    }

    // server.go:257
    // InitComplete returns a channel that is closed when the server has finished
    // processing the initialized notification, including the initial configuration
    // exchange with the client. (A context that is canceled then.)
    pub fn init_complete(&self) -> Context {
        self.init_complete.0.clone()
    }

    // server.go:263
    // WatchFiles implements project.Client.
    pub fn watch_files(&self, ctx: &Context, id: &str, watchers: Vec<lsproto::FileSystemWatcher>) -> Result<(), Error> {
        if let Some(builtin_watcher) = &self.builtin_watcher {
            match *builtin_watcher {}
        }
        let result = self.send_client_request(
            ctx,
            lsproto::CLIENT_REGISTER_CAPABILITY_INFO,
            lsproto::RegistrationParams {
                registrations: vec![lsproto::Registration {
                    id: id.to_string(),
                    register_options: Some(lsproto::RegisterOptions {
                        workspace_did_change_watched_files: Some(lsproto::DidChangeWatchedFilesRegistrationOptions { watchers }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
            },
        );
        if let Err(err) = result {
            return Err(Error::wrap("failed to register file watcher: ", err));
        }

        self.watchers.lock().unwrap().insert(id.to_string());
        Ok(())
    }

    // server.go:292
    // UnwatchFiles implements project.Client.
    pub fn unwatch_files(&self, ctx: &Context, id: &str) -> Result<(), Error> {
        if let Some(builtin_watcher) = &self.builtin_watcher {
            match *builtin_watcher {}
        }
        let has = self.watchers.lock().unwrap().contains(id);
        if has {
            let result = self.send_client_request(
                ctx,
                lsproto::CLIENT_UNREGISTER_CAPABILITY_INFO,
                lsproto::UnregistrationParams {
                    unregisterations: vec![lsproto::Unregistration { id: id.to_string(), method: Method::WorkspaceDidChangeWatchedFiles.0.to_string() }],
                },
            );
            if let Err(err) = result {
                return Err(Error::wrap("failed to unregister file watcher: ", err));
            }

            self.watchers.lock().unwrap().remove(id);
            return Ok(());
        }

        Err(Error::new(format!("no file watcher exists with ID {}", id)))
    }

    // server.go:362
    fn supports_content_mapper_registration(&self, id: &str) -> bool {
        let caps = self.client_capabilities();
        match id {
            CONTENT_MAPPER_DID_OPEN_REGISTRATION_ID | CONTENT_MAPPER_DID_CHANGE_REGISTRATION_ID | CONTENT_MAPPER_DID_CLOSE_REGISTRATION_ID => {
                caps.text_document.synchronization.dynamic_registration
            }
            CONTENT_MAPPER_DIAGNOSTIC_REGISTRATION_ID => caps.text_document.diagnostic.dynamic_registration,
            CONTENT_MAPPER_HOVER_REGISTRATION_ID => caps.text_document.hover.dynamic_registration,
            CONTENT_MAPPER_SIGNATURE_HELP_REGISTRATION_ID => caps.text_document.signature_help.dynamic_registration,
            CONTENT_MAPPER_DEFINITION_REGISTRATION_ID => caps.text_document.definition.dynamic_registration,
            CONTENT_MAPPER_TYPE_DEFINITION_REGISTRATION_ID => caps.text_document.type_definition.dynamic_registration,
            CONTENT_MAPPER_IMPLEMENTATION_REGISTRATION_ID => caps.text_document.implementation.dynamic_registration,
            CONTENT_MAPPER_REFERENCES_REGISTRATION_ID => caps.text_document.references.dynamic_registration,
            CONTENT_MAPPER_DOCUMENT_HIGHLIGHT_REGISTRATION_ID => caps.text_document.document_highlight.dynamic_registration,
            CONTENT_MAPPER_COMPLETION_REGISTRATION_ID => caps.text_document.completion.dynamic_registration,
            CONTENT_MAPPER_RENAME_REGISTRATION_ID => caps.text_document.rename.dynamic_registration,
            CONTENT_MAPPER_SEMANTIC_TOKENS_REGISTRATION_ID => caps.text_document.semantic_tokens.dynamic_registration,
            CONTENT_MAPPER_DOCUMENT_SYMBOL_REGISTRATION_ID => caps.text_document.document_symbol.dynamic_registration,
            CONTENT_MAPPER_FOLDING_RANGE_REGISTRATION_ID => caps.text_document.folding_range.dynamic_registration,
            CONTENT_MAPPER_SELECTION_RANGE_REGISTRATION_ID => caps.text_document.selection_range.dynamic_registration,
            CONTENT_MAPPER_INLAY_HINT_REGISTRATION_ID => caps.text_document.inlay_hint.dynamic_registration,
            CONTENT_MAPPER_CODE_LENS_REGISTRATION_ID => caps.text_document.code_lens.dynamic_registration,
            CONTENT_MAPPER_CODE_ACTION_REGISTRATION_ID => caps.text_document.code_action.dynamic_registration,
            CONTENT_MAPPER_FORMATTING_REGISTRATION_ID => caps.text_document.formatting.dynamic_registration,
            CONTENT_MAPPER_RANGE_FORMATTING_REGISTRATION_ID => caps.text_document.range_formatting.dynamic_registration,
            CONTENT_MAPPER_ON_TYPE_FORMATTING_REGISTRATION_ID => caps.text_document.on_type_formatting.dynamic_registration,
            CONTENT_MAPPER_LINKED_EDITING_REGISTRATION_ID => caps.text_document.linked_editing_range.dynamic_registration,
            CONTENT_MAPPER_CALL_HIERARCHY_REGISTRATION_ID => caps.text_document.call_hierarchy.dynamic_registration,
            CONTENT_MAPPER_WILL_RENAME_FILES_REGISTRATION_ID => caps.workspace.file_operations.dynamic_registration && caps.workspace.file_operations.will_rename,
            _ => false,
        }
    }

    // server.go:422
    // RegisterContentMapperExtensions implements project.Client. It dynamically registers text document
    // synchronization and pull diagnostics for the given otherwise unsupported file extensions so the editor forwards their
    // open/change/close notifications to the server and requests diagnostics for them. It is called with the
    // full desired set each time it changes; an empty slice removes any prior registration.
    pub fn register_content_mapper_extensions(&self, ctx: &Context, extensions: Vec<String>) -> Result<(), Error> {
        if !self.client_capabilities().text_document.synchronization.dynamic_registration {
            return Ok(());
        }

        let mut content_mapper_extensions_registered = self.content_mapper_registration_mu.lock().unwrap();

        if *content_mapper_extensions_registered {
            let unregistration = |id: &str, method: Method| lsproto::Unregistration { id: id.to_string(), method: method.0.to_string() };
            let mut unregistrations = vec![
                unregistration(CONTENT_MAPPER_DID_OPEN_REGISTRATION_ID, Method::TextDocumentDidOpen),
                unregistration(CONTENT_MAPPER_DID_CHANGE_REGISTRATION_ID, Method::TextDocumentDidChange),
                unregistration(CONTENT_MAPPER_DID_CLOSE_REGISTRATION_ID, Method::TextDocumentDidClose),
                unregistration(CONTENT_MAPPER_DIAGNOSTIC_REGISTRATION_ID, Method::TextDocumentDiagnostic),
                unregistration(CONTENT_MAPPER_HOVER_REGISTRATION_ID, Method::TextDocumentHover),
                unregistration(CONTENT_MAPPER_SIGNATURE_HELP_REGISTRATION_ID, Method::TextDocumentSignatureHelp),
                unregistration(CONTENT_MAPPER_DEFINITION_REGISTRATION_ID, Method::TextDocumentDefinition),
                unregistration(CONTENT_MAPPER_TYPE_DEFINITION_REGISTRATION_ID, Method::TextDocumentTypeDefinition),
                unregistration(CONTENT_MAPPER_IMPLEMENTATION_REGISTRATION_ID, Method::TextDocumentImplementation),
                unregistration(CONTENT_MAPPER_REFERENCES_REGISTRATION_ID, Method::TextDocumentReferences),
                unregistration(CONTENT_MAPPER_DOCUMENT_HIGHLIGHT_REGISTRATION_ID, Method::TextDocumentDocumentHighlight),
                unregistration(CONTENT_MAPPER_COMPLETION_REGISTRATION_ID, Method::TextDocumentCompletion),
                unregistration(CONTENT_MAPPER_RENAME_REGISTRATION_ID, Method::TextDocumentRename),
                unregistration(CONTENT_MAPPER_SEMANTIC_TOKENS_REGISTRATION_ID, Method::TextDocumentSemanticTokens),
                unregistration(CONTENT_MAPPER_DOCUMENT_SYMBOL_REGISTRATION_ID, Method::TextDocumentDocumentSymbol),
                unregistration(CONTENT_MAPPER_FOLDING_RANGE_REGISTRATION_ID, Method::TextDocumentFoldingRange),
                unregistration(CONTENT_MAPPER_SELECTION_RANGE_REGISTRATION_ID, Method::TextDocumentSelectionRange),
                unregistration(CONTENT_MAPPER_INLAY_HINT_REGISTRATION_ID, Method::TextDocumentInlayHint),
                unregistration(CONTENT_MAPPER_CODE_LENS_REGISTRATION_ID, Method::TextDocumentCodeLens),
                unregistration(CONTENT_MAPPER_CODE_ACTION_REGISTRATION_ID, Method::TextDocumentCodeAction),
                unregistration(CONTENT_MAPPER_FORMATTING_REGISTRATION_ID, Method::TextDocumentFormatting),
                unregistration(CONTENT_MAPPER_RANGE_FORMATTING_REGISTRATION_ID, Method::TextDocumentRangeFormatting),
                unregistration(CONTENT_MAPPER_ON_TYPE_FORMATTING_REGISTRATION_ID, Method::TextDocumentOnTypeFormatting),
                unregistration(CONTENT_MAPPER_LINKED_EDITING_REGISTRATION_ID, Method::TextDocumentLinkedEditingRange),
                unregistration(CONTENT_MAPPER_CALL_HIERARCHY_REGISTRATION_ID, Method::TextDocumentPrepareCallHierarchy),
                unregistration(CONTENT_MAPPER_WILL_RENAME_FILES_REGISTRATION_ID, Method::WorkspaceWillRenameFiles),
            ];
            unregistrations.retain(|registration| self.supports_content_mapper_registration(&registration.id));
            if let Err(err) =
                self.send_client_request(ctx, lsproto::CLIENT_UNREGISTER_CAPABILITY_INFO, lsproto::UnregistrationParams { unregisterations: unregistrations })
            {
                return Err(Error::wrap("failed to unregister content mapper text document sync: ", err));
            }
            *content_mapper_extensions_registered = false;
        }

        if extensions.is_empty() {
            return Ok(());
        }

        let mut filters = Vec::with_capacity(extensions.len());
        for ext in &extensions {
            filters.push(lsproto::TextDocumentFilterLanguageOrSchemeOrPattern {
                pattern: Some(lsproto::TextDocumentFilterPattern {
                    pattern: lsproto::PatternOrRelativePattern { pattern: Some(format!("**/*{}", ext)), ..Default::default() },
                    ..Default::default()
                }),
                ..Default::default()
            });
        }
        let selector = lsproto::DocumentSelectorOrNull { document_selector: Some(filters) };
        let mut content_mapper_file_rename_filters = Vec::with_capacity(extensions.len());
        for extension in &extensions {
            content_mapper_file_rename_filters.push(lsproto::FileOperationFilter {
                scheme: Some("file".to_string()),
                pattern: lsproto::FileOperationPattern { glob: format!("**/*{}", extension), ..Default::default() },
            });
        }

        let registration = |id: &str, register_options: lsproto::RegisterOptions| lsproto::Registration {
            id: id.to_string(),
            register_options: Some(register_options),
            ..Default::default()
        };
        let mut registrations = vec![
            registration(
                CONTENT_MAPPER_DID_OPEN_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_did_open: Some(lsproto::TextDocumentRegistrationOptions { document_selector: selector.clone() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_DID_CHANGE_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_did_change: Some(lsproto::TextDocumentChangeRegistrationOptions {
                        document_selector: selector.clone(),
                        sync_kind: lsproto::TextDocumentSyncKind::Incremental,
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_DID_CLOSE_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_did_close: Some(lsproto::TextDocumentRegistrationOptions { document_selector: selector.clone() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_DIAGNOSTIC_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_diagnostic: Some(lsproto::DiagnosticRegistrationOptions {
                        document_selector: selector.clone(),
                        identifier: Some("typescript".to_string()),
                        inter_file_dependencies: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_HOVER_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_hover: Some(lsproto::HoverRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_SIGNATURE_HELP_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_signature_help: Some(lsproto::SignatureHelpRegistrationOptions {
                        document_selector: selector.clone(),
                        trigger_characters: Some(lsconsts::strings(&lsconsts::SIGNATURE_HELP_TRIGGER_CHARACTERS)),
                        retrigger_characters: Some(lsconsts::strings(&lsconsts::SIGNATURE_HELP_RETRIGGER_CHARACTERS)),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_DEFINITION_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_definition: Some(lsproto::DefinitionRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_TYPE_DEFINITION_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_type_definition: Some(lsproto::TypeDefinitionRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_IMPLEMENTATION_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_implementation: Some(lsproto::ImplementationRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_REFERENCES_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_references: Some(lsproto::ReferenceRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_DOCUMENT_HIGHLIGHT_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_document_highlight: Some(lsproto::DocumentHighlightRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_COMPLETION_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_completion: Some(lsproto::CompletionRegistrationOptions {
                        document_selector: selector.clone(),
                        trigger_characters: Some(lsconsts::strings(&lsconsts::COMPLETION_TRIGGER_CHARACTERS)),
                        resolve_provider: Some(true),
                        completion_item: Some(lsproto::ServerCompletionItemOptions { label_details_support: Some(true) }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_RENAME_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_rename: Some(lsproto::RenameRegistrationOptions { document_selector: selector.clone(), prepare_provider: Some(true), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_SEMANTIC_TOKENS_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_semantic_tokens: Some(lsproto::SemanticTokensRegistrationOptions {
                        document_selector: selector.clone(),
                        legend: lsconsts::semantic_tokens_legend(&self.client_capabilities().text_document.semantic_tokens),
                        full: Some(lsproto::BooleanOrSemanticTokensFullDelta { boolean: Some(true), ..Default::default() }),
                        range: Some(lsproto::BooleanOrEmptyObject { boolean: Some(true), ..Default::default() }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_DOCUMENT_SYMBOL_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_document_symbol: Some(lsproto::DocumentSymbolRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_FOLDING_RANGE_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_folding_range: Some(lsproto::FoldingRangeRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_SELECTION_RANGE_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_selection_range: Some(lsproto::SelectionRangeRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_INLAY_HINT_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_inlay_hint: Some(lsproto::InlayHintRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_CODE_LENS_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_code_lens: Some(lsproto::CodeLensRegistrationOptions {
                        document_selector: selector.clone(),
                        resolve_provider: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_CODE_ACTION_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_code_action: Some(lsproto::CodeActionRegistrationOptions {
                        document_selector: selector.clone(),
                        code_action_kinds: Some(supported_code_action_kinds()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_FORMATTING_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_formatting: Some(lsproto::DocumentFormattingRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_RANGE_FORMATTING_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_range_formatting: Some(lsproto::DocumentRangeFormattingRegistrationOptions {
                        document_selector: selector.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_ON_TYPE_FORMATTING_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_on_type_formatting: Some(lsproto::DocumentOnTypeFormattingRegistrationOptions {
                        document_selector: selector.clone(),
                        first_trigger_character: "{".to_string(),
                        more_trigger_character: Some(vec!["}".to_string(), ";".to_string(), "\n".to_string()]),
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_LINKED_EDITING_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_linked_editing_range: Some(lsproto::LinkedEditingRangeRegistrationOptions {
                        document_selector: selector.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_CALL_HIERARCHY_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    text_document_prepare_call_hierarchy: Some(lsproto::CallHierarchyRegistrationOptions { document_selector: selector.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ),
            registration(
                CONTENT_MAPPER_WILL_RENAME_FILES_REGISTRATION_ID,
                lsproto::RegisterOptions {
                    workspace_will_rename_files: Some(lsproto::FileOperationRegistrationOptions { filters: content_mapper_file_rename_filters }),
                    ..Default::default()
                },
            ),
        ];
        registrations.retain(|registration| self.supports_content_mapper_registration(&registration.id));
        if let Err(err) = self.send_client_request(ctx, lsproto::CLIENT_REGISTER_CAPABILITY_INFO, lsproto::RegistrationParams { registrations }) {
            return Err(Error::wrap("failed to register content mapper text document sync: ", err));
        }

        *content_mapper_extensions_registered = true;
        Ok(())
    }

    // server.go:705
    // RefreshDiagnostics implements project.Client.
    pub fn refresh_diagnostics(&self, ctx: &Context) -> Result<(), Error> {
        if !self.client_capabilities().workspace.diagnostics.refresh_support {
            return Ok(());
        }

        if let Some(err) = ctx.err() {
            return Err(err.into());
        }

        // Fire-and-forget: the client always returns null, and waiting for the response
        // can cause the server to hang if the client is slow or unresponsive.
        // Any response from the client will be silently ignored by the read loop.
        if let Err(err) = self.send_client_request_fire_and_forget(lsproto::WORKSPACE_DIAGNOSTIC_REFRESH_INFO, lsproto::NoParams) {
            return Err(Error::wrap("failed to refresh diagnostics: ", err));
        }

        Ok(())
    }

    // server.go:725
    // PublishDiagnostics implements project.Client.
    pub fn publish_diagnostics(&self, ctx: &Context, params: lsproto::PublishDiagnosticsParams) -> Result<(), Error> {
        self.send_notification(lsproto::TEXT_DOCUMENT_PUBLISH_DIAGNOSTICS_INFO, params)
    }

    // server.go:730
    // SendTelemetry implements project.Client.
    pub fn send_telemetry(&self, ctx: &Context, telemetry: lsproto::TelemetryEvent) -> Result<(), Error> {
        if !self.telemetry_enabled.load(Ordering::SeqCst) {
            panic!("SendTelemetry called with telemetry disabled");
        }
        self.send_notification(lsproto::TELEMETRY_EVENT_INFO, telemetry)
    }

    // server.go:738
    // IsActive implements project.Client.
    pub fn is_active(&self) -> bool {
        let last = self.last_request_time_ms.load(Ordering::SeqCst);
        last == 0 || unix_millis_now().saturating_sub(last) <= 60_000
    }

    // server.go:743
    pub fn refresh_inlay_hints(&self, ctx: &Context) -> Result<(), Error> {
        if !self.client_capabilities().workspace.inlay_hint.refresh_support {
            return Ok(());
        }

        if let Err(err) = self.send_client_request_fire_and_forget(lsproto::WORKSPACE_INLAY_HINT_REFRESH_INFO, lsproto::NoParams) {
            return Err(Error::wrap("failed to refresh inlay hints: ", err));
        }
        Ok(())
    }

    // server.go:754
    pub fn refresh_code_lens(&self, ctx: &Context) -> Result<(), Error> {
        if !self.client_capabilities().workspace.code_lens.refresh_support {
            return Ok(());
        }

        if let Err(err) = self.send_client_request_fire_and_forget(lsproto::WORKSPACE_CODE_LENS_REFRESH_INFO, lsproto::NoParams) {
            return Err(Error::wrap("failed to refresh code lens: ", err));
        }
        Ok(())
    }

    // server.go:766
    // ProgressStart implements project.Client.
    pub fn progress_start(&self, message: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]) {
        if let Some(progress) = self.project_progress.get() {
            progress.start(message, tsrs_diagnostics::stringify_args(args));
        }
    }

    // server.go:773
    // ProgressFinish implements project.Client.
    pub fn progress_finish(&self, message: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]) {
        if let Some(progress) = self.project_progress.get() {
            progress.finish(message, tsrs_diagnostics::stringify_args(args));
        }
    }

    // server.go:780
    // GetLocale implements project.Client.
    pub fn get_locale(&self) -> Locale {
        self.locale.read().unwrap().clone()
    }

    // server.go:787
    // SetLocale implements project.Client.
    pub fn set_locale(&self, locale_string: &str) {
        let mut new_locale = self.init_locale.lock().unwrap().clone();
        if locale_string != "auto" {
            let Some(parsed) = locale_parse(locale_string) else {
                return;
            };
            new_locale = parsed;
        }
        *self.locale.write().unwrap() = new_locale;
    }

    // server.go:801
    pub fn request_configuration(&self, ctx: &Context) -> Result<lsutil::UserPreferences, Error> {
        let caps = lsproto::get_client_capabilities(ctx);
        if !caps.workspace.configuration {
            let opts = self.initialization_options();
            if let Some(user_prefs) = &opts.user_preferences {
                self.logger.logf(format_args!("received formatting options from initialization: {}\n{}", go_type_name(user_prefs), go_sprint_value(user_prefs)));
                if let lsproto::Value::Object(config) = user_prefs {
                    let mut items = OrderedMap::default();
                    items.insert("js/ts".to_string(), lsproto::Value::Object(config.clone()));
                    return Ok(lsutil::parse_user_preferences(&items));
                }
            }
            return Ok(lsutil::new_default_user_preferences());
        }
        let configs = match self.send_client_request(
            ctx,
            lsproto::WORKSPACE_CONFIGURATION_INFO,
            lsproto::ConfigurationParams {
                items: vec![
                    lsproto::ConfigurationItem { section: Some("js/ts".to_string()), ..Default::default() },
                    lsproto::ConfigurationItem { section: Some("typescript".to_string()), ..Default::default() },
                    lsproto::ConfigurationItem { section: Some("javascript".to_string()), ..Default::default() },
                    lsproto::ConfigurationItem { section: Some("editor".to_string()), ..Default::default() },
                ],
                ..Default::default()
            },
        ) {
            Ok(configs) => configs,
            Err(err) => return Err(Error::wrap("configure request failed: ", err)),
        };
        let mut config_map: OrderedMap<String, lsproto::Value> = OrderedMap::default();
        for (i, config) in configs.into_iter().enumerate() {
            match i {
                0 => {
                    config_map.insert("js/ts".to_string(), config);
                }
                1 => {
                    config_map.insert("typescript".to_string(), config);
                }
                2 => {
                    config_map.insert("javascript".to_string(), config);
                }
                3 => {
                    config_map.insert("editor".to_string(), config);
                }
                _ => {}
            }
        }
        let show = |key: &str| config_map.get(key).map(go_sprint_value).unwrap_or_else(|| "<nil>".to_string());
        self.logger.logf(format_args!(
            "received options from workspace/configuration request:\njs/ts: {}\n\ntypescript: {}\n\njavascript: {}\n\neditor: {}\n",
            show("js/ts"),
            show("typescript"),
            show("javascript"),
            show("editor"),
        ));
        Ok(lsutil::parse_user_preferences(&config_map))
    }

    // server.go:859
    pub fn run(self: &Arc<Self>, ctx: &Context) -> Result<(), Error> {
        let (g, ctx) = errGroup::with_context(ctx);
        let _ = self.background_ctx.set(ctx.clone());
        {
            let s = self.clone();
            let ctx = ctx.clone();
            g.go("lsp-dispatch", move || s.dispatch_loop(&ctx));
        }
        {
            let s = self.clone();
            let ctx = ctx.clone();
            g.go("lsp-write", move || s.write_loop(&ctx));
        }

        // Don't run readLoop in the group, as it blocks on stdin read and cannot be cancelled.
        // (Go's group member that waits for ctx.Done() or the read loop's error is folded into the read
        // thread: it reports its error to the group when it returns.)
        {
            let s = self.clone();
            let ctx = ctx.clone();
            let g = g.clone();
            spawn_server_thread("lsp-read", move || {
                let result = s.read_loop(&ctx);
                g.record(result);
            });
        }

        if let Some(err) = g.wait() {
            if !err.is_eof() && ctx.err().is_some() {
                return Err(err);
            }
        }
        Ok(())
    }

    // server.go:883
    fn read_loop(self: &Arc<Self>, ctx: &Context) -> Result<(), Error> {
        let Some(mut reader) = self.r.lock().unwrap().take() else {
            panic!("readLoop started twice");
        };
        loop {
            if let Some(err) = ctx.err() {
                return Err(err.into());
            }
            let (msg, err) = reader.read();
            if let Some(err) = err {
                if err.is_code(ErrorCode::InvalidRequest) || err.is_code(ErrorCode::InvalidParams) {
                    let mut id = None;
                    if err.is_code(ErrorCode::InvalidParams) {
                        if let Some(msg) = &msg {
                            if msg.kind == MessageKind::Request {
                                id = msg.as_request().id.clone();
                            }
                        }
                    }
                    self.send_error(id.as_ref(), err)?;
                    continue;
                }
                return Err(err);
            }
            let msg = msg.unwrap();

            if self.initialize_params.get().is_none() && msg.kind == MessageKind::Request {
                let req = msg.into_request();
                if req.method == Method::Initialize {
                    let params = match req.unmarshal_params::<lsproto::InitializeParams>() {
                        Ok(params) => params,
                        Err(err) => {
                            self.send_error(req.id.as_ref(), err)?;
                            continue;
                        }
                    };
                    let resp = self.handle_initialize(ctx, params, &req)?;
                    self.send_result(req.id.as_ref(), resp)?;
                } else {
                    self.send_error(req.id.as_ref(), ErrorCode::ServerNotInitialized.into())?;
                }
                continue;
            }

            if msg.kind == MessageKind::Response {
                let resp = msg.into_response();
                let mut pending = self.pending_server_requests.lock().unwrap();
                let id = resp.id.clone().expect("response without id");
                if let Some(resp_chan) = pending.remove(&id) {
                    resp_chan.send(resp);
                    resp_chan.close();
                }
            } else {
                let req = msg.into_request();
                if req.method == Method::CancelRequest {
                    if let Ok(params) = req.unmarshal_params::<lsproto::CancelParams>() {
                        self.cancel_request(&params.id);
                    }
                } else {
                    self.request_queue.put(ctx, Arc::new(req))?;
                }
            }
        }
    }

    // server.go:954
    fn cancel_request(&self, raw_id: &lsproto::IntegerOrString) {
        let id = lsproto::new_id(raw_id);
        let mut pending = self.pending_client_requests.lock().unwrap();
        if let Some(pending_req) = pending.remove(&id) {
            pending_req.cancel.call();
        }
    }

    // server.go:968
    fn dispatch_loop(self: &Arc<Self>, ctx: &Context) -> Result<(), Error> {
        let (ctx, lsp_exit) = ctx.with_cancel_cause();
        let _exit_on_return = deferCall(Some({
            let lsp_exit = lsp_exit.clone();
            move || lsp_exit.call(None)
        }));
        loop {
            let req = self.request_queue.get(&ctx)?;

            self.last_request_time_ms.store(unix_millis_now(), Ordering::SeqCst);
            let mut request_ctx = context::with_locale(&ctx, self.get_locale());
            let mut cancel = None;
            if let Some(id) = &req.id {
                let (c, cancel_func) = context::with_request_id(&request_ctx, &id.string()).with_cancel();
                request_ctx = c;
                cancel = Some(cancel_func.clone());
                self.pending_client_requests.lock().unwrap().insert(id.clone(), pendingClientRequest { req: req.clone(), cancel: cancel_func });
            }

            match self.handle_request_or_notification(&request_ctx, &req) {
                Err(err) => {
                    self.handle_error(req.id.as_ref(), err, &lsp_exit);
                    self.remove_request(req.id.as_ref(), cancel.as_ref());
                }
                Ok(Some(do_async_work)) => {
                    let s = self.clone();
                    let lsp_exit = lsp_exit.clone();
                    workerpool::go(move || {
                        if let Err(ls_error) = do_async_work() {
                            s.handle_error(req.id.as_ref(), ls_error, &lsp_exit);
                        }
                        s.remove_request(req.id.as_ref(), cancel.as_ref());
                    });
                }
                Ok(None) => {
                    self.remove_request(req.id.as_ref(), cancel.as_ref());
                }
            }
        }
    }

    // server.go:990 (dispatchLoop's handleError closure)
    fn handle_error(&self, id: Option<&ID>, err: Error, lsp_exit: &CancelCauseFunc) {
        if err.is(ErrorTag::ContextCanceled) {
            if let Err(err) = self.send_error(id, ErrorCode::RequestCancelled.into()) {
                lsp_exit.call(Some(err.message));
            }
        } else if err.is_eof() {
            lsp_exit.call(None);
        } else if let Err(err) = self.send_error(id, err) {
            lsp_exit.call(Some(err.message));
        }
    }

    // server.go:1004 (dispatchLoop's removeRequest closure)
    fn remove_request(&self, id: Option<&ID>, cancel: Option<&CancelFunc>) {
        if let Some(id) = id {
            self.pending_client_requests.lock().unwrap().remove(id);
            if let Some(cancel) = cancel {
                cancel.call();
            }
        }
    }

    // server.go:1029
    pub(crate) fn write_loop(&self, ctx: &Context) -> Result<(), Error> {
        loop {
            let msg = self.outgoing_queue.get(ctx)?;
            let result = self.w.lock().unwrap().write(&msg);
            if let Err(err) = result {
                if err.is(MESSAGE_MARSHAL_ERROR) && msg.kind == MessageKind::Response {
                    let resp = msg.as_response();
                    if resp.id.is_some() && resp.error.is_none() {
                        let id = resp.id.as_ref().unwrap();
                        self.logger.errorf(format_args!("failed to marshal response for request {}: {}", id, err));
                        self.send_error(Some(id), err)?;
                        continue;
                    }
                }
                return Err(Error::wrap("failed to write message: ", err));
            }
        }
    }

    // server.go:1053
    // WARNING: this should only be called in the async portion of a request handler,
    // otherwise a deadlock can occur.
    pub(crate) fn send_client_request<Req: Json, Resp: Json>(&self, ctx: &Context, info: lsproto::RequestInfo<Req, Resp>, params: Req) -> Result<Resp, Error> {
        let id = jsonrpc::new_id_string(&format!("ts{}", self.client_seq.fetch_add(1, Ordering::SeqCst) + 1));
        let req = info.new_request_message(Some(id.clone()), params);

        let response_chan = responseChan::new();
        self.pending_server_requests.lock().unwrap().insert(id.clone(), response_chan.clone());

        let _remove = deferCall(Some(|| {
            let mut pending = self.pending_server_requests.lock().unwrap();
            if let Some(resp_chan) = pending.remove(&id) {
                resp_chan.close();
            }
        }));

        self.send(req.message())?;

        match response_chan.recv(ctx) {
            Err(err) => Err(err.into()),
            Ok(resp) => {
                // A receive from the closed channel without a value is Go's nil response, whose fields
                // would be dereferenced; it cannot happen (the channel is closed after the send or by this
                // function's own cleanup).
                let resp = resp.expect("pending server request closed without a response");
                if let Some(error) = &resp.error {
                    return Err(Error::new(format!("request failed: {}", error.string())));
                }
                info.unmarshal_result(resp.result.as_ref()).map_err(Error::from)
            }
        }
    }

    // server.go:1090
    // sendClientRequestFireAndForget sends a request to the client without waiting for a response.
    // The response, if any, will be silently ignored by the read loop since no pending channel is registered.
    // This means any error returned by the client will not be observed. Use only for requests where the
    // response value is not needed (e.g., the client always returns null).
    pub(crate) fn send_client_request_fire_and_forget<Req: Json, Resp: Json>(&self, info: lsproto::RequestInfo<Req, Resp>, params: Req) -> Result<(), Error> {
        let id = jsonrpc::new_id_string(&format!("ts{}", self.client_seq.fetch_add(1, Ordering::SeqCst) + 1));
        let req = info.new_request_message(Some(id), params);
        self.send(req.message())
    }

    // server.go:1096
    pub(crate) fn send_result(&self, id: Option<&ID>, result: impl Json) -> Result<(), Error> {
        self.send_response(ResponseMessage { id: id.cloned(), result: Some(result.to_json()), ..Default::default() })
    }

    // server.go:1108
    pub(crate) fn send_error(&self, id: Option<&ID>, err: Error) -> Result<(), Error> {
        // Do not send error response for notifications,
        // except for parse errors which may occur before determining if the message is a request or notification.
        if id.is_none() && !err.is_code(ErrorCode::InvalidRequest) {
            self.logger.errorf(format_args!("error handling notification: {}", err));
            return Ok(());
        }
        let code = err.as_error_code().unwrap_or(ErrorCode::InternalError);
        // TODO(jakebailey): error data
        self.send_response(ResponseMessage {
            id: id.cloned(),
            error: Some(jsonrpc::ResponseError { code: code.0, message: err.message, data: None }),
            ..Default::default()
        })
    }

    // server.go:1129
    pub(crate) fn send_notification<Params: Json>(&self, info: lsproto::NotificationInfo<Params>, params: Params) -> Result<(), Error> {
        self.send(info.new_notification_message(params).message())
    }

    // server.go:1133
    fn send_response(&self, resp: ResponseMessage) -> Result<(), Error> {
        self.send(resp.message())
    }

    // server.go:1138
    // send writes a message to the outgoing queue, respecting context cancellation.
    pub(crate) fn send(&self, msg: Message) -> Result<(), Error> {
        self.outgoing_queue.put(&self.background_ctx(), msg).map_err(Error::from)
    }

    // server.go:1144
    // handleRequestOrNotification looks up the handler for the given request or notification, executes its synchronous work
    // and returns any asynchronous work as a function to be executed by the caller.
    fn handle_request_or_notification(self: &Arc<Self>, ctx: &Context, req: &Arc<RequestMessage>) -> Result<Option<asyncWork>, Error> {
        let ctx = lsproto::with_client_capabilities(ctx, self.client_capabilities_arc());

        if let Some(handler) = handlers().0.get(&req.method) {
            let start = Instant::now();
            let result = handler(self, &ctx, req);
            let id_str = match &req.id {
                Some(id) => format!(" ({})", id),
                None => String::new(),
            };
            let do_async_work = match result {
                Err(err) => {
                    if let Some(resp) = content_mapper_fallback_response(req.method, &err) {
                        if !self.logger.is_tracing() {
                            self.logger.info(&format!("handled method '{}'{} in {}", req.method, id_str, go_duration_string(start.elapsed())));
                        }
                        self.send_response(ResponseMessage { id: req.id.clone(), result: Some(resp), ..Default::default() })?;
                        return Ok(None);
                    }
                    if !err.is(USER_FACING_REQUEST_FAILED_ERROR) {
                        self.logger.error(&format!("error handling method '{}'{}: {}", req.method, id_str, err));
                    } else if !self.logger.is_tracing() {
                        self.logger.info(&format!("handled method '{}'{} in {}", req.method, id_str, go_duration_string(start.elapsed())));
                    }
                    return Err(err);
                }
                Ok(do_async_work) => do_async_work,
            };
            if let Some(do_async_work) = do_async_work {
                let s = self.clone();
                let method = req.method;
                return Ok(Some(Box::new(move || {
                    // note: ctx.Err() has to be checked in the async work to allow async handlers to cleanup resources correctly
                    let async_work_err = do_async_work();
                    let is_user_facing = matches!(&async_work_err, Err(err) if err.is(USER_FACING_REQUEST_FAILED_ERROR));
                    let is_real_error = async_work_err.is_err() && !is_user_facing;
                    if is_real_error {
                        s.logger.info(&format!("error handling method '{}'{} in {}", method, id_str, go_duration_string(start.elapsed())));
                    } else if !s.logger.is_tracing() {
                        s.logger.info(&format!("handled method '{}'{} in {}", method, id_str, go_duration_string(start.elapsed())));
                    }
                    async_work_err
                })));
            }
            if !self.logger.is_tracing() {
                self.logger.info(&format!("handled method '{}'{} in {}", req.method, id_str, go_duration_string(start.elapsed())));
            }
            return Ok(None);
        }
        self.logger.warn(&format!("unknown method '{}'", req.method));
        if req.id.is_some() {
            self.send_error(req.id.as_ref(), ErrorCode::InvalidRequest.into())?;
            return Ok(None);
        }
        Ok(None)
    }

    // server.go:1468
    fn get_language_service_and_cross_project_orchestrator(
        self: &Arc<Self>,
        ctx: &Context,
        uri: &lsproto::DocumentUri,
        req: &RequestMessage,
    ) -> Result<(Arc<LanguageService>, Arc<dyn tsrs_ls::CrossProjectOrchestrator>), Error> {
        let (default_project, default_ls, all_projects) = self.session().get_language_service_and_projects_for_file(ctx, uri)?;
        let orchestrator: Arc<dyn tsrs_ls::CrossProjectOrchestrator> = Arc::new(crossProjectOrchestrator {
            server: self.clone(),
            req: Arc::new(req.clone()),
            default_project: default_project.arc().clone(),
            all_projects,
        });
        Ok((default_ls, orchestrator))
    }

    // server.go:1477: `defer s.recover(req)` around `f`. A panic in `f` is logged and answered with an internal
    // error; the work then returns nil like Go's function after a recovered panic.
    pub(crate) fn with_recover(&self, req: &RequestMessage, f: impl FnOnce() -> Result<(), Error>) -> Result<(), Error> {
        match recover_scope(f) {
            Ok(result) => result,
            Err((payload, stack)) => {
                self.recover(req, payload, &stack);
                Ok(())
            }
        }
    }

    // server.go:1477
    fn recover(&self, req: &RequestMessage, r: Box<dyn Any + Send>, stack: &str) {
        let r = panic_value_string(&r);
        self.logger.errorf(format_args!("panic handling request {}: {}\n{}", req.method, r, stack));
        if req.id.is_some() {
            let _ = self.send_error(
                req.id.as_ref(),
                Error::wrap_code(ErrorCode::InternalError, Error::new(format!("panic handling request {}: {}", req.method, r))),
            );
        } else {
            self.logger.error(&format!("unhandled panic in notification{}{}", req.method, r));
        }

        if self.telemetry_enabled.load(Ordering::SeqCst) {
            let _ = self.send_notification(
                lsproto::TELEMETRY_EVENT_INFO,
                lsproto::TelemetryEvent {
                    request_failure_telemetry_event: Some(lsproto::RequestFailureTelemetryEvent {
                        properties: lsproto::RequestFailureTelemetryProperties {
                            error_code: ErrorCode::InternalError.string(),
                            request_method: req.method.0.replace('/', "."),
                            stack: sanitize_stack_trace(stack),
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            );
        }
    }

    // server.go:1501
    fn handle_initialize(self: &Arc<Self>, ctx: &Context, params: lsproto::InitializeParams, _req: &RequestMessage) -> Result<lsproto::InitializeResponse, Error> {
        if self.initialize_params.get().is_some() {
            return Err(ErrorCode::InvalidRequest.into());
        }

        self.init_started.store(true, Ordering::SeqCst);

        let params = Arc::new(params);
        let _ = self.initialize_params.set(params.clone());
        // The spec types initializationOptions as nullable; treat both null and an
        // absent value as empty options so the rest of the server can read fields
        // off s.initializationOptions without nil-checking the container.
        let initialization_options = match &params.initialization_options {
            Some(lsproto::InitializationOptionsOrNull { initialization_options: Some(options), .. }) => options.clone(),
            _ => lsproto::InitializationOptions::default(),
        };
        let _ = self.initialization_options.set(initialization_options);
        let initialization_options = self.initialization_options();
        if let Some(v) = initialization_options.log_verbosity {
            if is_valid_log_verbosity(v) {
                self.logger.set_verbosity(v);
            }
        }
        if let Some(level) = initialization_options.track_flaky_diagnostics {
            let _ = self.flake_logging.set(level);
        }
        let _ = self.client_capabilities.set(Arc::new(params.capabilities.resolve()));
        if self.client_capabilities().window.work_done_progress {
            let _ = self.project_progress.set(new_project_loading_progress(Arc::downgrade(self), self.progress_delay));
        }

        // Go logs json.MarshalIndent(&s.clientCapabilities, "", "\t"); the Resolved* views have no JSON codec in
        // tsrs_lsproto, so the log shows their Debug form.
        self.logger.info(&format!("Resolved client capabilities: {:?}", self.client_capabilities()));

        let mut position_encoding = lsproto::PositionEncodingKind::UTF16;
        if self.client_capabilities().general.position_encodings.contains(&lsproto::PositionEncodingKind::UTF8) {
            position_encoding = lsproto::PositionEncodingKind::UTF8;
        }
        let _ = self.position_encoding.set(position_encoding);

        if let Some(locale) = &params.locale {
            *self.locale.write().unwrap() = locale_parse(locale).unwrap_or(Locale::DEFAULT);
        }
        *self.init_locale.lock().unwrap() = self.get_locale();

        if let Some(start_watchdog) = &self.start_watchdog {
            if let Some(process_id) = params.process_id.integer {
                start_watchdog(process_id);
            }
        }

        let response = lsproto::InitializeResult {
            server_info: Some(lsproto::ServerInfo { name: "typescript".to_string(), version: Some(tsrs_core::version().to_string()) }),
            capabilities: lsproto::ServerCapabilities {
                position_encoding: Some(self.position_encoding()),
                text_document_sync: Some(lsproto::TextDocumentSyncOptionsOrKind {
                    options: Some(lsproto::TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(lsproto::TextDocumentSyncKind::Incremental),
                        save: Some(lsproto::BooleanOrSaveOptions { boolean: Some(true), ..Default::default() }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                hover_provider: Some(lsproto::BooleanOrHoverOptions { boolean: Some(true), ..Default::default() }),
                definition_provider: Some(lsproto::BooleanOrDefinitionOptions { boolean: Some(true), ..Default::default() }),
                type_definition_provider: Some(lsproto::BooleanOrTypeDefinitionOptionsOrTypeDefinitionRegistrationOptions {
                    boolean: Some(true),
                    ..Default::default()
                }),
                references_provider: Some(lsproto::BooleanOrReferenceOptions { boolean: Some(true), ..Default::default() }),
                implementation_provider: Some(lsproto::BooleanOrImplementationOptionsOrImplementationRegistrationOptions {
                    boolean: Some(true),
                    ..Default::default()
                }),
                diagnostic_provider: Some(lsproto::DiagnosticOptionsOrRegistrationOptions {
                    options: Some(lsproto::DiagnosticOptions {
                        identifier: Some("typescript".to_string()),
                        inter_file_dependencies: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                completion_provider: Some(lsproto::CompletionOptions {
                    trigger_characters: Some(lsconsts::strings(&lsconsts::COMPLETION_TRIGGER_CHARACTERS)),
                    resolve_provider: Some(true),
                    completion_item: Some(lsproto::ServerCompletionItemOptions { label_details_support: Some(true) }),
                    ..Default::default()
                }),
                signature_help_provider: Some(lsproto::SignatureHelpOptions {
                    trigger_characters: Some(lsconsts::strings(&lsconsts::SIGNATURE_HELP_TRIGGER_CHARACTERS)),
                    retrigger_characters: Some(lsconsts::strings(&lsconsts::SIGNATURE_HELP_RETRIGGER_CHARACTERS)),
                    ..Default::default()
                }),
                document_formatting_provider: Some(lsproto::BooleanOrDocumentFormattingOptions { boolean: Some(true), ..Default::default() }),
                document_range_formatting_provider: Some(lsproto::BooleanOrDocumentRangeFormattingOptions {
                    boolean: Some(true),
                    ..Default::default()
                }),
                document_on_type_formatting_provider: Some(lsproto::DocumentOnTypeFormattingOptions {
                    first_trigger_character: "{".to_string(),
                    more_trigger_character: Some(vec!["}".to_string(), ";".to_string(), "\n".to_string()]),
                }),
                workspace_symbol_provider: Some(lsproto::BooleanOrWorkspaceSymbolOptions { boolean: Some(true), ..Default::default() }),
                document_symbol_provider: Some(lsproto::BooleanOrDocumentSymbolOptions { boolean: Some(true), ..Default::default() }),
                folding_range_provider: Some(lsproto::BooleanOrFoldingRangeOptionsOrFoldingRangeRegistrationOptions {
                    boolean: Some(true),
                    ..Default::default()
                }),
                rename_provider: Some(lsproto::BooleanOrRenameOptions {
                    rename_options: Some(lsproto::RenameOptions { prepare_provider: Some(true), ..Default::default() }),
                    ..Default::default()
                }),
                document_highlight_provider: Some(lsproto::BooleanOrDocumentHighlightOptions { boolean: Some(true), ..Default::default() }),
                selection_range_provider: Some(lsproto::BooleanOrSelectionRangeOptionsOrSelectionRangeRegistrationOptions {
                    boolean: Some(true),
                    ..Default::default()
                }),
                linked_editing_range_provider: Some(lsproto::BooleanOrLinkedEditingRangeOptionsOrLinkedEditingRangeRegistrationOptions {
                    boolean: Some(true),
                    ..Default::default()
                }),
                inlay_hint_provider: Some(lsproto::BooleanOrInlayHintOptionsOrInlayHintRegistrationOptions {
                    boolean: Some(true),
                    ..Default::default()
                }),
                code_lens_provider: Some(lsproto::CodeLensOptions { resolve_provider: Some(true), ..Default::default() }),
                code_action_provider: Some(lsproto::BooleanOrCodeActionOptions {
                    code_action_options: Some(lsproto::CodeActionOptions {
                        code_action_kinds: Some(supported_code_action_kinds()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                call_hierarchy_provider: Some(lsproto::BooleanOrCallHierarchyOptionsOrCallHierarchyRegistrationOptions {
                    boolean: Some(true),
                    ..Default::default()
                }),
                experimental: Some(lsproto::ExperimentalServerCapabilities {
                    custom_source_definition_provider: Some(true),
                    custom_multi_document_highlight_provider: Some(true),
                }),
                vs_references_provider: Some(true),
                vs_on_auto_insert_provider: Some(lsproto::VSOnAutoInsertOptions { vs_trigger_characters: vec![">".to_string()] }),
                workspace: Some(lsproto::WorkspaceOptions {
                    file_operations: Some(lsproto::FileOperationOptions {
                        will_rename: Some(lsproto::FileOperationRegistrationOptions { filters: file_rename_filters() }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                semantic_tokens_provider: Some(lsproto::SemanticTokensOptionsOrRegistrationOptions {
                    options: Some(lsproto::SemanticTokensOptions {
                        legend: lsconsts::semantic_tokens_legend(&self.client_capabilities().text_document.semantic_tokens),
                        full: Some(lsproto::BooleanOrSemanticTokensFullDelta { boolean: Some(true), ..Default::default() }),
                        range: Some(lsproto::BooleanOrEmptyObject { boolean: Some(true), ..Default::default() }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        };

        Ok(response)
    }

    // server.go:1677
    fn handle_initialized(self: &Arc<Self>, ctx: &Context, _params: lsproto::InitializedParams) -> Result<(), Error> {
        let mut disable_push_diagnostics = false;
        let mut enable_telemetry = false;
        let opts = self.initialization_options();
        if let Some(v) = opts.disable_push_diagnostics {
            disable_push_diagnostics = v;
        }
        if let Some(v) = opts.enable_telemetry {
            enable_telemetry = v;
        }
        let mut run_external_code = false;
        if let Some(v) = opts.run_external_code {
            run_external_code = v;
        }
        let has_dynamic_watch_registration = self.client_capabilities().workspace.did_change_watched_files.dynamic_registration;
        if has_dynamic_watch_registration {
            self.logger.logf(format_args!("file watching: using LSP client-side watching (client supports dynamic registration)"));
            self.watch_enabled.store(true, Ordering::SeqCst);
        } else if builtin_watcher_has_fast_recursive_backend() {
            // The client cannot watch files itself, but the builtin watcher has a
            // backend with efficient recursive watching (Windows or FSEvents), so
            // fall back to watching files in-process.
            // (Phase 4: lspwatcher is not ported, so this branch is never taken; Go logs
            // "file watching: using builtin in-process watcher (client lacks dynamic watch registration)", enables
            // watching and creates the watcher here.)
        } else {
            // The client cannot watch files and the builtin watcher backend lacks
            // efficient recursive watching, so file watching is disabled.
            // (tsrs: the builtin watcher (lspwatcher) is not ported yet, phase 4, so this branch is also taken where
            // Go would watch in-process.)
            self.logger.logf(format_args!(
                "file watching: disabled (client lacks dynamic watch registration and builtin watcher backend is not fast-recursive; the builtin watcher is not ported)"
            ));
        }

        let initialize_params = self.initialize_params.get().unwrap().clone();
        let mut cwd = self.cwd.clone();
        if self.client_capabilities().workspace.workspace_folders
            && matches!(&initialize_params.workspace_folders, Some(lsproto::WorkspaceFoldersOrNull { workspace_folders: Some(folders), .. }) if folders.len() == 1)
        {
            let folders = initialize_params.workspace_folders.as_ref().unwrap().workspace_folders.as_ref().unwrap();
            cwd = lsproto::DocumentUri(folders[0].uri.0.clone()).file_name();
        } else if let Some(root_uri) = &initialize_params.root_uri.document_uri {
            cwd = root_uri.file_name();
        } else if let Some(lsproto::StringOrNull { string: Some(root_path), .. }) = &initialize_params.root_path {
            cwd = root_path.clone();
        }
        if !tspath::path_is_absolute(&cwd) {
            cwd = self.cwd.clone();
        }

        self.telemetry_enabled.store(enable_telemetry, Ordering::SeqCst);

        let client: Arc<dyn project::Client> = self.clone();
        let logger: Arc<dyn logging::Logger> = self.logger_arc();
        let npm_executor: Arc<dyn project::NpmExecutor> = self.clone();
        let session = project::new_session(project::SessionInit {
            background_ctx: lsproto::with_client_capabilities(&self.background_ctx(), self.client_capabilities_arc()),
            options: Arc::new(project::SessionOptions {
                current_directory: cwd,
                default_library_path: self.default_library_path.clone(),
                typings_location: self.typings_location.clone(),
                position_encoding: self.position_encoding(),
                watch_enabled: self.watch_enabled.load(Ordering::SeqCst),
                logging_enabled: true,
                telemetry_enabled: enable_telemetry,
                debounce_delay: Duration::from_millis(500),
                push_diagnostics_enabled: !disable_push_diagnostics,
                run_external_code,
                ..Default::default()
            }),
            fs: self.fs.clone().expect("FS is required"),
            logger: Some(logger),
            client: Some(client),
            npm_executor: Some(npm_executor),
            parse_cache: self.parse_cache.clone(),
            content_mapped_parse_cache: None,
        });
        let _ = self.session.set(session.clone());

        let user_preferences = self.request_configuration(ctx)?;
        session.initialize_with_user_config(user_preferences);

        let result = self.send_client_request(
            ctx,
            lsproto::CLIENT_REGISTER_CAPABILITY_INFO,
            lsproto::RegistrationParams {
                registrations: vec![lsproto::Registration {
                    id: "typescript-config-watch-id".to_string(),
                    register_options: Some(lsproto::RegisterOptions {
                        workspace_did_change_configuration: Some(lsproto::DidChangeConfigurationRegistrationOptions {
                            section: Some(lsproto::StringOrStrings {
                                strings: Some(vec!["js/ts".to_string(), "typescript".to_string(), "javascript".to_string(), "editor".to_string()]),
                                ..Default::default()
                            }),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
            },
        );
        if let Err(err) = result {
            return Err(Error::wrap("failed to register configuration change watcher: ", err));
        }

        // !!! temporary.
        // Remove when we have `handleDidChangeConfiguration`/implicit project config support
        // derived from 'js/ts.implicitProjectConfig.*'.
        let options = *self.compiler_options_for_inferred_projects.lock().unwrap();
        if options.is_some() {
            session.did_change_compiler_options_for_inferred_projects(ctx, options);
        }

        session.start_performance_telemetry();

        self.init_complete.1.call();
        Ok(())
    }

    // server.go:1788
    fn handle_shutdown(self: &Arc<Self>, ctx: &Context, _params: lsproto::NoParams, _req: &RequestMessage) -> Result<lsproto::ShutdownResponse, Error> {
        if let Some(builtin_watcher) = &self.builtin_watcher {
            match *builtin_watcher {}
        }
        self.session().close();
        Ok(lsproto::Null)
    }

    // server.go:1796
    fn handle_exit(self: &Arc<Self>, ctx: &Context, _params: lsproto::NoParams) -> Result<(), Error> {
        Err(Error::tagged(ErrorTag::EOF, "EOF"))
    }

    // server.go:1800
    fn handle_did_change_workspace_configuration(self: &Arc<Self>, ctx: &Context, params: lsproto::DidChangeConfigurationParams) -> Result<(), Error> {
        if let lsproto::Value::Null = params.settings {
            return Ok(());
        } else if let lsproto::Value::Object(settings) = &params.settings {
            self.session().configure(lsutil::parse_user_preferences(settings));
        }
        Ok(())
    }

    // server.go:1809
    fn handle_did_open(self: &Arc<Self>, ctx: &Context, params: lsproto::DidOpenTextDocumentParams) -> Result<(), Error> {
        let doc = params.text_document;
        self.session().did_open_file(ctx, doc.uri, doc.version, doc.text, doc.language_id);
        Ok(())
    }

    // server.go:1814
    fn handle_did_change(self: &Arc<Self>, ctx: &Context, params: lsproto::DidChangeTextDocumentParams) -> Result<(), Error> {
        self.session().did_change_file(ctx, params.text_document.uri, params.text_document.version, params.content_changes);
        Ok(())
    }

    // server.go:1819
    fn handle_did_save(self: &Arc<Self>, ctx: &Context, params: lsproto::DidSaveTextDocumentParams) -> Result<(), Error> {
        self.session().did_save_file(ctx, params.text_document.uri);
        Ok(())
    }

    // server.go:1824
    fn handle_did_close(self: &Arc<Self>, ctx: &Context, params: lsproto::DidCloseTextDocumentParams) -> Result<(), Error> {
        self.session().did_close_file(ctx, params.text_document.uri);
        Ok(())
    }

    // server.go:1829
    fn handle_did_change_watched_files(self: &Arc<Self>, ctx: &Context, params: lsproto::DidChangeWatchedFilesParams) -> Result<(), Error> {
        self.session().did_change_watched_files(ctx, &params.changes);
        Ok(())
    }

    // server.go:1834
    fn handle_set_trace(self: &Arc<Self>, _ctx: &Context, _params: lsproto::SetTraceParams) -> Result<(), Error> {
        // $/setTrace is sent by vscode-languageclient when trace settings change.
        // Server log verbosity is controlled separately by custom/setLogVerbosity,
        // so this handler is intentionally a no-op.
        Ok(())
    }

    // server.go:1841
    fn handle_set_log_verbosity(self: &Arc<Self>, _ctx: &Context, params: lsproto::SetLogVerbosityParams) -> Result<(), Error> {
        if !is_valid_log_verbosity(params.verbosity) {
            return Err(Error::wrap_code(ErrorCode::InvalidParams, Error::new(format!("invalid log verbosity {}", params.verbosity.0))));
        }
        self.logger.set_verbosity(params.verbosity);
        Ok(())
    }

    // server.go:1849
    fn handle_document_diagnostic(
        self: &Arc<Self>,
        ctx: &Context,
        language_service: &Arc<LanguageService>,
        params: lsproto::DocumentDiagnosticParams,
    ) -> Result<lsproto::DocumentDiagnosticResponse, Error> {
        let ctx = context::with_checker_lifetime(ctx, context::CheckerLifetime::Diagnostics);
        if self.flake_logging() == lsproto::DiagnosticFlakeLogLevel::Off {
            return language_service.provide_diagnostics(&ctx, &params.text_document.uri);
        }
        let direct = language_service.provide_diagnostics(&ctx, &params.text_document.uri)?;
        // Go runs a full `Program.Emit` (writing nothing) between the two diagnostics requests to provoke checker
        // state changes; emit is not ported, so the second request runs right after the first.
        let secondary = match language_service.provide_diagnostics(&ctx, &params.text_document.uri) {
            Ok(secondary) => secondary,
            Err(_) => return Ok(direct),
        };
        let empty = Vec::new();
        let direct_items = direct.full_document_diagnostic_report.as_ref().map(|r| &r.items).unwrap_or(&empty);
        let secondary_items = secondary.full_document_diagnostic_report.as_ref().map(|r| &r.items).unwrap_or(&empty);
        let (missing_from_pre, missing_from_post) = lsproto::compare_diagnostics(direct_items, secondary_items);
        if missing_from_pre.is_empty() && missing_from_post.is_empty() {
            return Ok(direct);
        }

        let diff = generate_diagnostic_diff_string(&missing_from_pre, &missing_from_post, lsproto::Diagnostic::as_string);

        self.logger.error(&diff);

        if self.telemetry_enabled.load(Ordering::SeqCst) {
            let sanitized_diff = generate_diagnostic_diff_string(&missing_from_pre, &missing_from_post, lsproto::Diagnostic::code_as_string);
            let _ = self.send_notification(
                lsproto::TELEMETRY_EVENT_INFO,
                lsproto::TelemetryEvent {
                    request_failure_telemetry_event: Some(lsproto::RequestFailureTelemetryEvent {
                        properties: lsproto::RequestFailureTelemetryProperties {
                            error_code: ErrorCode::InternalError.string(),
                            request_method: "textDocument.diagnostic.flakeLog".to_string(),
                            stack: sanitized_diff,
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            );
        }

        if self.flake_logging() == lsproto::DiagnosticFlakeLogLevel::Panic {
            panic!("flaky diagnostic(s) logged:\n{}", diff);
        }
        Ok(direct)
    }

    // server.go:1907
    fn handle_hover(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::HoverParams) -> Result<lsproto::HoverResponse, Error> {
        ls.provide_hover(ctx, &params)
    }

    // server.go:1911
    fn handle_prepare_rename(
        self: &Arc<Self>,
        ctx: &Context,
        language_service: &Arc<LanguageService>,
        params: lsproto::PrepareRenameParams,
    ) -> Result<lsproto::PrepareRenameResponse, Error> {
        let info = language_service.get_rename_info(ctx, "" /*newName*/, &params.text_document.uri, params.position);
        if !info.can_rename {
            return Err(user_facing_request_failed_error(info.localized_error_message));
        }
        Ok(lsproto::PrepareRenameResponse {
            prepare_rename_placeholder: Some(lsproto::PrepareRenamePlaceholder { range: info.trigger_span, placeholder: info.display_name }),
            ..Default::default()
        })
    }

    // server.go:1924
    fn handle_rename(self: &Arc<Self>, ctx: &Context, params: lsproto::RenameParams, req: &RequestMessage) -> Result<lsproto::RenameResponse, Error> {
        let (default_ls, orchestrator) = self.get_language_service_and_cross_project_orchestrator(ctx, &params.text_document.uri, req)?;
        let info = default_ls.get_rename_info(ctx, &params.new_name, &params.text_document.uri, params.position);
        if info.can_rename && !info.file_to_rename.is_empty() {
            // We send a `willRenameFiles` request if the client allows;
            // otherwise we directly compute the edits for renaming the file.
            if tsrs_ls::client_supports_will_rename_files(ctx) {
                let document_changes = vec![lsproto::TextDocumentEditOrCreateFileOrRenameFileOrDeleteFile {
                    rename_file: Some(lsproto::RenameFile {
                        kind: lsproto::StringLiteralRename::default(),
                        old_uri: tsrs_ls::lsconv::file_name_to_document_uri(&info.file_to_rename),
                        new_uri: tsrs_ls::lsconv::file_name_to_document_uri(&info.new_file_name),
                        ..Default::default()
                    }),
                    ..Default::default()
                }];
                return Ok(lsproto::WorkspaceEditOrNull {
                    workspace_edit: Some(lsproto::WorkspaceEdit { document_changes: Some(document_changes), ..Default::default() }),
                });
            }
            let rename_files_params = lsproto::RenameFilesParams {
                files: vec![lsproto::FileRename {
                    old_uri: tsrs_ls::lsconv::file_name_to_document_uri(&info.file_to_rename),
                    new_uri: tsrs_ls::lsconv::file_name_to_document_uri(&info.new_file_name),
                }],
            };
            return self.handle_will_rename_files_worker(ctx, rename_files_params, req, true /*sendRenameFile*/);
        }

        default_ls.provide_rename(ctx, &params, Some(&*orchestrator))
    }

    // server.go:1961
    fn handle_will_rename_files(self: &Arc<Self>, ctx: &Context, params: lsproto::RenameFilesParams, msg: &RequestMessage) -> Result<lsproto::WillRenameFilesResponse, Error> {
        self.handle_will_rename_files_worker(ctx, params, msg, false /*sendRenameFile*/)
    }

    // server.go:1968
    // If `sendRenameFile` is true, the original `willRenameFiles` request is being handled as part of a rename operation
    // where the client doesn't support `willRenameFiles`,
    // so we should include the file rename in the edits we return
    fn handle_will_rename_files_worker(
        self: &Arc<Self>,
        ctx: &Context,
        params: lsproto::RenameFilesParams,
        _req: &RequestMessage,
        send_rename_file: bool,
    ) -> Result<lsproto::WillRenameFilesResponse, Error> {
        if params.files.is_empty() {
            return Ok(lsproto::WillRenameFilesResponse::default());
        }

        let mut uris = Vec::with_capacity(params.files.len());
        for file in &params.files {
            uris.push(file.old_uri.clone());
        }

        if uris.is_empty() {
            return Ok(lsproto::WillRenameFilesResponse::default());
        }

        // The rest of the worker collects `LanguageService.GetEditsForFileRename` results (phase 3).
        Err(not_yet_ported(Method::WorkspaceWillRenameFiles))
    }

    // server.go:2070
    fn handle_signature_help(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::SignatureHelpParams) -> Result<lsproto::SignatureHelpResponse, Error> {
        Err(not_yet_ported(Method::TextDocumentSignatureHelp))
    }

    // server.go:2079
    fn handle_folding_range(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::FoldingRangeParams) -> Result<lsproto::FoldingRangeResponse, Error> {
        ls.provide_folding_range(ctx, &params.text_document.uri)
    }

    // server.go:2083
    fn handle_vs_on_auto_insert(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::VSOnAutoInsertParams) -> Result<lsproto::VSOnAutoInsertResponse, Error> {
        Err(not_yet_ported(Method::TextDocumentVSOnAutoInsert))
    }

    // server.go:2087
    fn handle_linked_editing_range(
        self: &Arc<Self>,
        ctx: &Context,
        ls: &Arc<LanguageService>,
        params: lsproto::LinkedEditingRangeParams,
    ) -> Result<lsproto::LinkedEditingRangeResponse, Error> {
        Err(not_yet_ported(Method::TextDocumentLinkedEditingRange))
    }

    // server.go:2091
    fn handle_definition(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::DefinitionParams) -> Result<lsproto::DefinitionResponse, Error> {
        ls.provide_definition(ctx, &params.text_document.uri, params.position)
    }

    // server.go:2095
    fn handle_source_definition(
        self: &Arc<Self>,
        ctx: &Context,
        ls: &Arc<LanguageService>,
        params: lsproto::TextDocumentPositionParams,
    ) -> Result<lsproto::CustomTextDocumentSourceDefinitionResponse, Error> {
        let resp = ls.provide_source_definition(ctx, &params.text_document.uri, params.position)?;
        Ok(resp)
    }

    // server.go:2103
    fn handle_type_definition(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::TypeDefinitionParams) -> Result<lsproto::TypeDefinitionResponse, Error> {
        ls.provide_type_definition(ctx, &params.text_document.uri, params.position)
    }

    // server.go:2107
    fn handle_completion(self: &Arc<Self>, ctx: &Context, language_service: &Arc<LanguageService>, params: lsproto::CompletionParams) -> Result<lsproto::CompletionResponse, Error> {
        Err(not_yet_ported(Method::TextDocumentCompletion))
    }

    // server.go:2116
    fn handle_completion_item_resolve(self: &Arc<Self>, ctx: &Context, params: lsproto::CompletionItem, req_msg: &RequestMessage) -> Result<lsproto::CompletionResolveResponse, Error> {
        Err(not_yet_ported(Method::CompletionItemResolve))
    }

    // server.go:2129
    fn handle_document_format(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::DocumentFormattingParams) -> Result<lsproto::DocumentFormattingResponse, Error> {
        ls.provide_format_document(ctx, &params.text_document.uri, &params.options)
    }

    // server.go:2137
    fn handle_document_range_format(
        self: &Arc<Self>,
        ctx: &Context,
        ls: &Arc<LanguageService>,
        params: lsproto::DocumentRangeFormattingParams,
    ) -> Result<lsproto::DocumentRangeFormattingResponse, Error> {
        ls.provide_format_document_range(ctx, &params.text_document.uri, &params.options, params.range)
    }

    // server.go:2146
    fn handle_document_on_type_format(
        self: &Arc<Self>,
        ctx: &Context,
        ls: &Arc<LanguageService>,
        params: lsproto::DocumentOnTypeFormattingParams,
    ) -> Result<lsproto::DocumentOnTypeFormattingResponse, Error> {
        ls.provide_format_document_on_type(ctx, &params.text_document.uri, &params.options, params.position, &params.ch)
    }

    // server.go:2156
    fn handle_workspace_symbol(self: &Arc<Self>, ctx: &Context, params: lsproto::WorkspaceSymbolParams, req_msg: &RequestMessage) -> Result<lsproto::WorkspaceSymbolResponse, Error> {
        let mut resp = lsproto::WorkspaceSymbolResponse::default();
        let mut ls_err: Option<Error> = None;
        let mut provide_symbols = |snapshot: &Arc<project::Snapshot>, programs: Vec<&'static tsrs_compiler::Program>| {
            let _ = self.with_recover(req_msg, || {
                match tsrs_ls::provide_workspace_symbols(ctx, &programs, &snapshot.converters(), snapshot.user_preferences(), &params.query) {
                    Ok(r) => resp = r,
                    Err(e) => ls_err = Some(e),
                }
                Ok(())
            });
        };
        if params.text_document.is_some() && self.session().config().workspace_symbols_scope == lsutil::WorkspaceSymbolsScope::CurrentProject {
            let uri = params.text_document.as_ref().unwrap().uri.clone();
            self.session().with_snapshot_for_document(ctx, &uri, |snapshot| {
                // Go maps `ls.Project.GetProgram` (a nil program is dereferenced by ProvideWorkspaceSymbols).
                let programs = snapshot.get_language_service_projects_containing_file(&uri).iter().map(|p| p.get_program().unwrap()).collect();
                provide_symbols(snapshot, programs);
            });
        } else {
            self.session().with_snapshot_loading_project_tree(ctx, None, |snapshot| {
                let programs = snapshot.project_collection.language_service_projects().iter().map(|p| p.get_program().unwrap()).collect();
                provide_symbols(snapshot, programs);
            });
        }
        match ls_err {
            Some(e) => Err(e),
            None => Ok(resp),
        }
    }

    // server.go:2184
    fn handle_document_symbol(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::DocumentSymbolParams) -> Result<lsproto::DocumentSymbolResponse, Error> {
        ls.provide_document_symbols(ctx, &params.text_document.uri)
    }

    // server.go:2188
    fn handle_document_highlight(
        self: &Arc<Self>,
        ctx: &Context,
        ls: &Arc<LanguageService>,
        params: lsproto::DocumentHighlightParams,
    ) -> Result<lsproto::DocumentHighlightResponse, Error> {
        ls.provide_document_highlights(ctx, &params.text_document.uri, params.position)
    }

    // server.go:2192
    fn handle_multi_document_highlight(
        self: &Arc<Self>,
        ctx: &Context,
        ls: &Arc<LanguageService>,
        params: lsproto::MultiDocumentHighlightParams,
    ) -> Result<lsproto::CustomMultiDocumentHighlightResponse, Error> {
        ls.provide_multi_document_highlights(ctx, &params.text_document.uri, params.position, &params.files_to_search)
    }

    // server.go:2196
    fn handle_selection_range(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::SelectionRangeParams) -> Result<lsproto::SelectionRangeResponse, Error> {
        ls.provide_selection_ranges(ctx, &params)
    }

    // server.go:2200
    fn handle_code_action(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::CodeActionParams) -> Result<lsproto::CodeActionResponse, Error> {
        Err(not_yet_ported(Method::TextDocumentCodeAction))
    }

    // server.go:2204
    fn handle_inlay_hint(self: &Arc<Self>, ctx: &Context, language_service: &Arc<LanguageService>, params: lsproto::InlayHintParams) -> Result<lsproto::InlayHintResponse, Error> {
        Err(not_yet_ported(Method::TextDocumentInlayHint))
    }

    // server.go:2212
    fn handle_code_lens(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::CodeLensParams) -> Result<lsproto::CodeLensResponse, Error> {
        Err(not_yet_ported(Method::TextDocumentCodeLens))
    }

    // server.go:2216
    fn handle_code_lens_resolve(self: &Arc<Self>, ctx: &Context, code_lens: lsproto::CodeLens, req_msg: &RequestMessage) -> Result<lsproto::CodeLensResolveResponse, Error> {
        Err(not_yet_ported(Method::CodeLensResolve))
    }

    // server.go:2240
    fn handle_prepare_call_hierarchy(
        self: &Arc<Self>,
        ctx: &Context,
        language_service: &Arc<LanguageService>,
        params: lsproto::CallHierarchyPrepareParams,
    ) -> Result<lsproto::CallHierarchyPrepareResponse, Error> {
        language_service.provide_prepare_call_hierarchy(ctx, &params.text_document.uri, params.position)
    }

    // server.go:2248
    fn handle_call_hierarchy_incoming_calls(
        self: &Arc<Self>,
        ctx: &Context,
        params: lsproto::CallHierarchyIncomingCallsParams,
        req_msg: &RequestMessage,
    ) -> Result<lsproto::CallHierarchyIncomingCallsResponse, Error> {
        let (default_ls, orchestrator) = self.get_language_service_and_cross_project_orchestrator(ctx, &params.item.uri, req_msg)?;
        default_ls.provide_call_hierarchy_incoming_calls(ctx, &params.item, Some(&*orchestrator))
    }

    // server.go:2260
    fn handle_call_hierarchy_outgoing_calls(
        self: &Arc<Self>,
        ctx: &Context,
        params: lsproto::CallHierarchyOutgoingCallsParams,
        _req: &RequestMessage,
    ) -> Result<lsproto::CallHierarchyOutgoingCallsResponse, Error> {
        let language_service = self.session().get_language_service(ctx, &params.item.uri)?;
        language_service.provide_call_hierarchy_outgoing_calls(ctx, &params.item)
    }

    // server.go:2272
    fn handle_semantic_tokens_full(self: &Arc<Self>, ctx: &Context, ls: &Arc<LanguageService>, params: lsproto::SemanticTokensParams) -> Result<lsproto::SemanticTokensResponse, Error> {
        Err(not_yet_ported(Method::TextDocumentSemanticTokensFull))
    }

    // server.go:2276
    fn handle_semantic_tokens_range(
        self: &Arc<Self>,
        ctx: &Context,
        ls: &Arc<LanguageService>,
        params: lsproto::SemanticTokensRangeParams,
    ) -> Result<lsproto::SemanticTokensRangeResponse, Error> {
        Err(not_yet_ported(Method::TextDocumentSemanticTokensRange))
    }

    // server.go:2280 (the `api` package is out of scope)
    fn handle_initialize_api_session(
        self: &Arc<Self>,
        ctx: &Context,
        params: lsproto::InitializeAPISessionParams,
        _req: &RequestMessage,
    ) -> Result<lsproto::CustomInitializeAPISessionResponse, Error> {
        Err(not_yet_ported(Method::CustomInitializeAPISession))
    }

    // server.go:2363
    // !!! temporary; remove when we have `handleDidChangeConfiguration`/implicit project config support
    pub fn set_compiler_options_for_inferred_projects(&self, ctx: &Context, options: Option<P<CompilerOptions>>) {
        *self.compiler_options_for_inferred_projects.lock().unwrap() = options;
        if let Some(session) = self.session.get() {
            session.did_change_compiler_options_for_inferred_projects(ctx, options);
        }
    }

    // server.go:2394 (pprof is out of scope)
    fn handle_run_gc(self: &Arc<Self>, _ctx: &Context, _params: lsproto::NoParams, _req: &RequestMessage) -> Result<lsproto::RunGCResponse, Error> {
        Err(not_yet_ported(Method::CustomRunGC))
    }

    // server.go:2400
    fn handle_save_heap_profile(self: &Arc<Self>, _ctx: &Context, params: lsproto::ProfileParams, _req: &RequestMessage) -> Result<lsproto::SaveHeapProfileResponse, Error> {
        Err(not_yet_ported(Method::CustomSaveHeapProfile))
    }

    // server.go:2409
    fn handle_save_alloc_profile(self: &Arc<Self>, _ctx: &Context, params: lsproto::ProfileParams, _req: &RequestMessage) -> Result<lsproto::SaveAllocProfileResponse, Error> {
        Err(not_yet_ported(Method::CustomSaveAllocProfile))
    }

    // server.go:2418
    fn handle_start_cpu_profile(self: &Arc<Self>, _ctx: &Context, params: lsproto::ProfileParams, _req: &RequestMessage) -> Result<lsproto::StartCPUProfileResponse, Error> {
        Err(not_yet_ported(Method::CustomStartCPUProfile))
    }

    // server.go:2427
    fn handle_stop_cpu_profile(self: &Arc<Self>, _ctx: &Context, _params: lsproto::NoParams, _req: &RequestMessage) -> Result<lsproto::StopCPUProfileResponse, Error> {
        Err(not_yet_ported(Method::CustomStopCPUProfile))
    }

    // server.go:2436
    fn handle_project_info(self: &Arc<Self>, ctx: &Context, params: lsproto::ProjectInfoParams, _req: &RequestMessage) -> Result<lsproto::CustomProjectInfoResponse, Error> {
        let uri = &params.text_document.uri;
        let (default_project, _, _) = self.session().get_language_service_and_projects_for_file(ctx, uri)?;
        let mut config_file_path = String::new();
        if default_project.kind == project::Kind::Configured {
            config_file_path = default_project.config_file_name().to_string();
        }
        Ok(lsproto::ProjectInfoResult { config_file_path })
    }

    // server.go:2451 (content mappers are out of scope)
    fn handle_set_content_mapper_contributions(
        self: &Arc<Self>,
        ctx: &Context,
        params: lsproto::SetContentMapperContributionsParams,
        _req: &RequestMessage,
    ) -> Result<lsproto::CustomSetContentMapperContributionsResponse, Error> {
        Err(not_yet_ported(Method::CustomSetContentMapperContributions))
    }
}

// server.go:323
const CONTENT_MAPPER_DID_OPEN_REGISTRATION_ID: &str = "content-mapper-did-open";
const CONTENT_MAPPER_DID_CHANGE_REGISTRATION_ID: &str = "content-mapper-did-change";
const CONTENT_MAPPER_DID_CLOSE_REGISTRATION_ID: &str = "content-mapper-did-close";
const CONTENT_MAPPER_DIAGNOSTIC_REGISTRATION_ID: &str = "content-mapper-diagnostic";
const CONTENT_MAPPER_HOVER_REGISTRATION_ID: &str = "content-mapper-hover";
const CONTENT_MAPPER_SIGNATURE_HELP_REGISTRATION_ID: &str = "content-mapper-signature-help";
const CONTENT_MAPPER_DEFINITION_REGISTRATION_ID: &str = "content-mapper-definition";
const CONTENT_MAPPER_TYPE_DEFINITION_REGISTRATION_ID: &str = "content-mapper-type-definition";
const CONTENT_MAPPER_IMPLEMENTATION_REGISTRATION_ID: &str = "content-mapper-implementation";
const CONTENT_MAPPER_REFERENCES_REGISTRATION_ID: &str = "content-mapper-references";
const CONTENT_MAPPER_DOCUMENT_HIGHLIGHT_REGISTRATION_ID: &str = "content-mapper-document-highlight";
const CONTENT_MAPPER_COMPLETION_REGISTRATION_ID: &str = "content-mapper-completion";
const CONTENT_MAPPER_RENAME_REGISTRATION_ID: &str = "content-mapper-rename";
const CONTENT_MAPPER_SEMANTIC_TOKENS_REGISTRATION_ID: &str = "content-mapper-semantic-tokens";
const CONTENT_MAPPER_DOCUMENT_SYMBOL_REGISTRATION_ID: &str = "content-mapper-document-symbol";
const CONTENT_MAPPER_FOLDING_RANGE_REGISTRATION_ID: &str = "content-mapper-folding-range";
const CONTENT_MAPPER_SELECTION_RANGE_REGISTRATION_ID: &str = "content-mapper-selection-range";
const CONTENT_MAPPER_INLAY_HINT_REGISTRATION_ID: &str = "content-mapper-inlay-hint";
const CONTENT_MAPPER_CODE_LENS_REGISTRATION_ID: &str = "content-mapper-code-lens";
const CONTENT_MAPPER_CODE_ACTION_REGISTRATION_ID: &str = "content-mapper-code-action";
const CONTENT_MAPPER_FORMATTING_REGISTRATION_ID: &str = "content-mapper-formatting";
const CONTENT_MAPPER_RANGE_FORMATTING_REGISTRATION_ID: &str = "content-mapper-range-formatting";
const CONTENT_MAPPER_ON_TYPE_FORMATTING_REGISTRATION_ID: &str = "content-mapper-on-type-formatting";
const CONTENT_MAPPER_LINKED_EDITING_REGISTRATION_ID: &str = "content-mapper-linked-editing";
const CONTENT_MAPPER_CALL_HIERARCHY_REGISTRATION_ID: &str = "content-mapper-call-hierarchy";
const CONTENT_MAPPER_WILL_RENAME_FILES_REGISTRATION_ID: &str = "content-mapper-will-rename-files";

// server.go:352
fn supported_code_action_kinds() -> Vec<lsproto::CodeActionKind> {
    vec![
        lsproto::CodeActionKind::QuickFix,
        lsproto::CodeActionKind::SourceOrganizeImportsTs,
        lsproto::CodeActionKind::SourceRemoveUnusedImportsTs,
        lsproto::CodeActionKind::SourceSortImportsTs,
        lsproto::CodeActionKind::SourceFixAllTs,
    ]
}

// server.go:1103: `userFacingRequestFailedError` (a string error type that unwraps to ErrorCodeRequestFailed).
pub(crate) const USER_FACING_REQUEST_FAILED_ERROR: ErrorTag = ErrorTag::Sentinel("userFacingRequestFailedError");

pub(crate) fn user_facing_request_failed_error(message: impl Into<String>) -> Error {
    Error { message: message.into(), tags: vec![USER_FACING_REQUEST_FAILED_ERROR, ErrorTag::Code(ErrorCode::RequestFailed)] }
}

// server.go:1198
// contentMapperFallbackResponse returns an empty response for requests made for
// unknown file types not handled by any content mapper. This typically serves a
// short window in time between when the server has unregistered content mapper
// extensions and when the client has stopped sending requests for those file types.
fn content_mapper_fallback_response(method: Method, err: &Error) -> Option<lsproto::Value> {
    if !project::is_err_no_project_for_unknown_script_kind(err) {
        return None;
    }
    match method {
        Method::TextDocumentDiagnostic => Some(
            lsproto::DocumentDiagnosticResponse {
                full_document_diagnostic_report: Some(lsproto::RelatedFullDocumentDiagnosticReport { items: Vec::new(), ..Default::default() }),
                ..Default::default()
            }
            .to_json(),
        ),
        Method::TextDocumentHover
        | Method::TextDocumentSignatureHelp
        | Method::TextDocumentDefinition
        | Method::TextDocumentTypeDefinition
        | Method::TextDocumentImplementation
        | Method::TextDocumentReferences
        | Method::TextDocumentDocumentHighlight
        | Method::TextDocumentCompletion
        | Method::TextDocumentRename => Some(lsproto::Null.to_json()),
        _ => None,
    }
}

// server.go:1227
// handlerMap maps LSP method to a handler function. The handler function executes any work that must be done synchronously
// before other requests/notifications can be processed, and returns any additional work as a function to be executed
// asynchronously after the synchronous work is complete.
pub(crate) type handlerFunc = Box<dyn Fn(&Arc<Server>, &Context, &Arc<RequestMessage>) -> Result<Option<asyncWork>, Error> + Send + Sync>;

pub(crate) struct handlerMap(FxHashMap<Method, handlerFunc>);

// server.go:1229
fn handlers() -> &'static handlerMap {
    static HANDLERS: OnceLock<handlerMap> = OnceLock::new();
    HANDLERS.get_or_init(|| {
        let mut handlers = handlerMap(FxHashMap::default());

        handlers.register_request_handler(lsproto::INITIALIZE_INFO, |s, ctx, params, req| s.handle_initialize(ctx, params, req));
        handlers.register_notification_handler(lsproto::INITIALIZED_INFO, Server::handle_initialized);
        handlers.register_request_handler(lsproto::SHUTDOWN_INFO, Server::handle_shutdown);
        handlers.register_notification_handler(lsproto::EXIT_INFO, Server::handle_exit);

        handlers.register_notification_handler(lsproto::WORKSPACE_DID_CHANGE_CONFIGURATION_INFO, Server::handle_did_change_workspace_configuration);
        handlers.register_notification_handler(lsproto::TEXT_DOCUMENT_DID_OPEN_INFO, Server::handle_did_open);
        handlers.register_notification_handler(lsproto::TEXT_DOCUMENT_DID_CHANGE_INFO, Server::handle_did_change);
        handlers.register_notification_handler(lsproto::TEXT_DOCUMENT_DID_SAVE_INFO, Server::handle_did_save);
        handlers.register_notification_handler(lsproto::TEXT_DOCUMENT_DID_CLOSE_INFO, Server::handle_did_close);
        handlers.register_notification_handler(lsproto::WORKSPACE_DID_CHANGE_WATCHED_FILES_INFO, Server::handle_did_change_watched_files);
        handlers.register_notification_handler(lsproto::SET_TRACE_INFO, Server::handle_set_trace);
        handlers.register_notification_handler(lsproto::CUSTOM_SET_LOG_VERBOSITY_INFO, Server::handle_set_log_verbosity);
        handlers.register_request_handler(lsproto::WORKSPACE_WILL_RENAME_FILES_INFO, Server::handle_will_rename_files);

        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_DIAGNOSTIC_INFO, Server::handle_document_diagnostic);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_HOVER_INFO, Server::handle_hover);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_DEFINITION_INFO, Server::handle_definition);
        handlers.register_language_service_document_request_handler(lsproto::CUSTOM_TEXT_DOCUMENT_SOURCE_DEFINITION_INFO, Server::handle_source_definition);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_TYPE_DEFINITION_INFO, Server::handle_type_definition);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_SIGNATURE_HELP_INFO, Server::handle_signature_help);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_FORMATTING_INFO, Server::handle_document_format);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_RANGE_FORMATTING_INFO, Server::handle_document_range_format);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_ON_TYPE_FORMATTING_INFO, Server::handle_document_on_type_format);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_DOCUMENT_SYMBOL_INFO, Server::handle_document_symbol);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_DOCUMENT_HIGHLIGHT_INFO, Server::handle_document_highlight);
        handlers.register_language_service_document_request_handler(lsproto::CUSTOM_TEXT_DOCUMENT_MULTI_DOCUMENT_HIGHLIGHT_INFO, Server::handle_multi_document_highlight);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_SELECTION_RANGE_INFO, Server::handle_selection_range);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_INLAY_HINT_INFO, Server::handle_inlay_hint);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_CODE_LENS_INFO, Server::handle_code_lens);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_CODE_ACTION_INFO, Server::handle_code_action);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_PREPARE_CALL_HIERARCHY_INFO, Server::handle_prepare_call_hierarchy);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_FOLDING_RANGE_INFO, Server::handle_folding_range);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_PREPARE_RENAME_INFO, Server::handle_prepare_rename);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_LINKED_EDITING_RANGE_INFO, Server::handle_linked_editing_range);

        handlers.register_language_service_with_auto_imports_request_handler(lsproto::TEXT_DOCUMENT_COMPLETION_INFO, Server::handle_completion);
        handlers.register_language_service_with_auto_imports_request_handler(lsproto::TEXT_DOCUMENT_CODE_ACTION_INFO, Server::handle_code_action);

        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_VS_ON_AUTO_INSERT_INFO, Server::handle_vs_on_auto_insert);

        handlers.register_multi_project_reference_request_handler(lsproto::TEXT_DOCUMENT_REFERENCES_INFO, |ls, ctx, params, orchestrator| {
            ls.provide_references(ctx, &params, Some(&*orchestrator))
        });
        handlers.register_multi_project_reference_request_handler(lsproto::TEXT_DOCUMENT_VS_REFERENCES_INFO, |ls, ctx, params, orchestrator| {
            ls.provide_vs_references(ctx, &params, Some(&*orchestrator))
        });
        handlers.register_request_handler(lsproto::TEXT_DOCUMENT_RENAME_INFO, Server::handle_rename);
        handlers.register_multi_project_reference_request_handler(lsproto::TEXT_DOCUMENT_IMPLEMENTATION_INFO, |ls, ctx, params, orchestrator| {
            ls.provide_implementations(ctx, &params, Some(&*orchestrator))
        });

        handlers.register_request_handler(lsproto::CALL_HIERARCHY_INCOMING_CALLS_INFO, Server::handle_call_hierarchy_incoming_calls);
        handlers.register_request_handler(lsproto::CALL_HIERARCHY_OUTGOING_CALLS_INFO, Server::handle_call_hierarchy_outgoing_calls);

        handlers.register_request_handler(lsproto::WORKSPACE_SYMBOL_INFO, Server::handle_workspace_symbol);
        handlers.register_request_handler(lsproto::COMPLETION_ITEM_RESOLVE_INFO, Server::handle_completion_item_resolve);
        handlers.register_request_handler(lsproto::CODE_LENS_RESOLVE_INFO, Server::handle_code_lens_resolve);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_SEMANTIC_TOKENS_FULL_INFO, Server::handle_semantic_tokens_full);
        handlers.register_language_service_document_request_handler(lsproto::TEXT_DOCUMENT_SEMANTIC_TOKENS_RANGE_INFO, Server::handle_semantic_tokens_range);

        // Developer/debugging commands
        handlers.register_request_handler(lsproto::CUSTOM_RUN_GC_INFO, Server::handle_run_gc);
        handlers.register_request_handler(lsproto::CUSTOM_SAVE_HEAP_PROFILE_INFO, Server::handle_save_heap_profile);
        handlers.register_request_handler(lsproto::CUSTOM_SAVE_ALLOC_PROFILE_INFO, Server::handle_save_alloc_profile);
        handlers.register_request_handler(lsproto::CUSTOM_START_CPU_PROFILE_INFO, Server::handle_start_cpu_profile);
        handlers.register_request_handler(lsproto::CUSTOM_STOP_CPU_PROFILE_INFO, Server::handle_stop_cpu_profile);

        handlers.register_request_handler(lsproto::CUSTOM_INITIALIZE_API_SESSION_INFO, Server::handle_initialize_api_session);
        handlers.register_request_handler(lsproto::CUSTOM_PROJECT_INFO_INFO, Server::handle_project_info);
        handlers.register_request_handler(lsproto::CUSTOM_SET_CONTENT_MAPPER_CONTRIBUTIONS_INFO, Server::handle_set_content_mapper_contributions);
        handlers
    })
}

impl handlerMap {
    // server.go:1300
    fn register_notification_handler<Req: Json + Default + 'static>(
        &mut self,
        info: lsproto::NotificationInfo<Req>,
        f: fn(&Arc<Server>, &Context, Req) -> Result<(), Error>,
    ) {
        self.0.insert(
            info.method,
            Box::new(move |s, ctx, req| {
                if s.session.get().is_none() && req.method != Method::Initialized {
                    return Err(ErrorCode::ServerNotInitialized.into());
                }

                let params = req.unmarshal_params::<Req>()?;
                f(s, ctx, params)?;
                match ctx.err() {
                    Some(err) => Err(err.into()),
                    None => Ok(None),
                }
            }),
        );
    }

    // server.go:1317
    fn register_request_handler<Req: Json + Default + 'static, Resp: Json + 'static>(
        &mut self,
        info: lsproto::RequestInfo<Req, Resp>,
        f: fn(&Arc<Server>, &Context, Req, &RequestMessage) -> Result<Resp, Error>,
    ) {
        self.0.insert(
            info.method,
            Box::new(move |s, ctx, req| {
                if s.session.get().is_none() && req.method != Method::Initialize {
                    return Err(ErrorCode::ServerNotInitialized.into());
                }

                let params = req.unmarshal_params::<Req>()?;
                let resp = f(s, ctx, params, req)?;
                if let Some(err) = ctx.err() {
                    return Err(err.into());
                }
                s.send_result(req.id.as_ref(), resp)?;
                Ok(None)
            }),
        );
    }

    // server.go:1341
    fn register_language_service_document_request_handler<Req, Resp>(
        &mut self,
        info: lsproto::RequestInfo<Req, Resp>,
        f: fn(&Arc<Server>, &Context, &Arc<LanguageService>, Req) -> Result<Resp, Error>,
    ) where
        Req: Json + Default + lsproto::HasTextDocumentURI + Send + 'static,
        Resp: Json + 'static,
    {
        self.0.insert(
            info.method,
            Box::new(move |s, ctx, req| {
                let params = req.unmarshal_params::<Req>()?;
                let ls = s.session().get_language_service(ctx, params.text_document_uri())?;
                let (s, ctx, req) = (s.clone(), ctx.clone(), req.clone());
                Ok(Some(Box::new(move || {
                    s.with_recover(&req, || {
                        let resp = f(&s, &ctx, &ls, params);
                        // After any language service request, check if new global diagnostics were
                        // discovered during checking and push updated tsconfig diagnostics if so.
                        s.session().enqueue_publish_global_diagnostics();
                        let resp = resp?;
                        if let Some(err) = ctx.err() {
                            return Err(err.into());
                        }
                        s.send_result(req.id.as_ref(), resp)
                    })
                })))
            }),
        );
    }

    // server.go:1368
    fn register_language_service_with_auto_imports_request_handler<Req, Resp>(
        &mut self,
        info: lsproto::RequestInfo<Req, Resp>,
        f: fn(&Arc<Server>, &Context, &Arc<LanguageService>, Req) -> Result<Resp, Error>,
    ) where
        Req: Json + Default + Clone + lsproto::HasTextDocumentURI + Send + 'static,
        Resp: Json + 'static,
    {
        self.0.insert(
            info.method,
            Box::new(move |s, ctx, req| {
                let params = req.unmarshal_params::<Req>()?;
                let uri = params.text_document_uri().clone();
                s.session().with_language_service_and_snapshot(ctx, &uri, |language_service, snapshot| {
                    let (s, ctx, req) = (s.clone(), ctx.clone(), req.clone());
                    Ok(Some(Box::new(move || {
                        s.with_recover(&req, || {
                            let mut language_service = language_service;
                            let mut resp = f(&s, &ctx, &language_service, params.clone());
                            if matches!(&resp, Err(err) if tsrs_ls::is_err_needs_auto_imports(err)) {
                                language_service = s.session().get_language_service_with_auto_imports(&ctx, &snapshot, params.text_document_uri())?;
                                if let Some(err) = ctx.err() {
                                    return Err(err.into());
                                }
                                resp = f(&s, &ctx, &language_service, params);
                                if matches!(&resp, Err(err) if tsrs_ls::is_err_needs_auto_imports(err)) {
                                    panic!("{} returned ErrNeedsAutoImports even after enabling auto imports", info.method);
                                }
                            }
                            let resp = resp?;
                            if let Some(err) = ctx.err() {
                                return Err(err.into());
                            }
                            s.send_result(req.id.as_ref(), resp)
                        })
                    }) as project::AsyncWork))
                })
            }),
        );
    }

    // server.go:1403
    fn register_multi_project_reference_request_handler<Req, Resp>(
        &mut self,
        info: lsproto::RequestInfo<Req, Resp>,
        f: fn(&LanguageService, &Context, Req, Arc<dyn tsrs_ls::CrossProjectOrchestrator>) -> Result<Resp, Error>,
    ) where
        Req: Json + Default + lsproto::HasTextDocumentPosition + Send + 'static,
        Resp: Json + 'static,
    {
        self.0.insert(
            info.method,
            Box::new(move |s, ctx, req| {
                let params = req.unmarshal_params::<Req>()?;
                // !!! sheetal: multiple projects that contain the file through symlinks
                let (default_ls, orchestrator) = s.get_language_service_and_cross_project_orchestrator(ctx, params.text_document_uri(), req)?;
                let (s, ctx, req) = (s.clone(), ctx.clone(), req.clone());
                Ok(Some(Box::new(move || {
                    s.with_recover(&req, || {
                        let resp = f(&default_ls, &ctx, params, orchestrator)?;
                        if let Some(err) = ctx.err() {
                            return Err(err.into());
                        }
                        s.send_result(req.id.as_ref(), resp)
                    })
                })))
            }),
        );
    }
}

// server.go:1431
struct crossProjectOrchestrator {
    server: Arc<Server>,
    req: Arc<RequestMessage>,
    default_project: Arc<dyn tsrs_ls::Project>,
    all_projects: Vec<Arc<dyn tsrs_ls::Project>>,
}

impl tsrs_ls::CrossProjectOrchestrator for crossProjectOrchestrator {
    // server.go:1440
    fn get_default_project(&self) -> Arc<dyn tsrs_ls::Project> {
        self.default_project.clone()
    }

    // server.go:1444
    fn get_all_projects_for_initial_request(&self) -> Vec<Arc<dyn tsrs_ls::Project>> {
        self.all_projects.clone()
    }

    // server.go:1448
    fn get_language_service_for_project_with_file(&self, ctx: &Context, p: &Arc<dyn tsrs_ls::Project>, uri: &lsproto::DocumentUri) -> Option<Arc<LanguageService>> {
        // Go type-asserts `p.(*project.Project)`; `ls::Project` has no downcast, so the project is found by its
        // id in the current snapshot (the session looks it up by id in a fresh snapshot either way).
        let snapshot = self.server.session().snapshot();
        let project = snapshot.project_collection.get_project(&project::ID(p.id()))?;
        self.server.session().get_language_service_for_project_with_file(ctx, &project, uri)
    }

    // server.go:1452
    fn get_projects_for_file(&self, ctx: &Context, uri: &lsproto::DocumentUri) -> Result<Vec<Arc<dyn tsrs_ls::Project>>, Error> {
        self.server.session().get_projects_for_file(ctx, uri)
    }

    // server.go:1456. Go returns an iterator that loads the snapshot while it is consumed; the projects are
    // collected while the snapshot is held.
    fn get_projects_loading_project_tree(&self, ctx: &Context, requested_project_trees: &Set<Path>) -> Box<dyn Iterator<Item = Arc<dyn tsrs_ls::Project>> + '_> {
        let mut projects = Vec::new();
        self.server.session().with_snapshot_loading_project_tree(ctx, Some(requested_project_trees.clone()), |snapshot| {
            for p in snapshot.project_collection.language_service_projects() {
                projects.push(p.arc().clone() as Arc<dyn tsrs_ls::Project>);
            }
        });
        Box::new(projects.into_iter())
    }
}

// The handlers of methods whose language-service functions are not ported yet (phase 3) and of out-of-scope
// features (content mappers, the api package, pprof) answer with this error.
pub(crate) fn not_yet_ported(method: Method) -> Error {
    Error::wrap_code(ErrorCode::MethodNotFound, Error::new(format!("{} is not ported yet", method)))
}

// server.go:1896
fn generate_diagnostic_diff_string(missing_from_pre: &[&lsproto::Diagnostic], missing_from_post: &[&lsproto::Diagnostic], stringifier: fn(&lsproto::Diagnostic) -> String) -> String {
    let mut b = String::new();
    for elem in missing_from_pre {
        b.push_str(&format!("Diagnostic {} was present after emit but not before emit\n", stringifier(elem)));
    }
    for elem in missing_from_post {
        b.push_str(&format!("Diagnostic {} was present before emit but not after emit\n", stringifier(elem)));
    }
    b
}

// fswatch.Default().HasFastRecursiveBackend(): the builtin watcher (lspwatcher, fswatch) is phase 4, so no
// backend is available yet.
fn builtin_watcher_has_fast_recursive_backend() -> bool {
    false
}

// server.go:99: `_ project.Client = (*Server)(nil)`.
impl project::Client for Server {
    fn watch_files(&self, ctx: &Context, id: project::WatcherID, watchers: Vec<lsproto::FileSystemWatcher>) -> Result<(), Error> {
        Server::watch_files(self, ctx, &id.0, watchers)
    }
    fn unwatch_files(&self, ctx: &Context, id: project::WatcherID) -> Result<(), Error> {
        Server::unwatch_files(self, ctx, &id.0)
    }
    fn register_content_mapper_extensions(&self, ctx: &Context, extensions: Vec<String>) -> Result<(), Error> {
        Server::register_content_mapper_extensions(self, ctx, extensions)
    }
    fn refresh_diagnostics(&self, ctx: &Context) -> Result<(), Error> {
        Server::refresh_diagnostics(self, ctx)
    }
    fn publish_diagnostics(&self, ctx: &Context, params: lsproto::PublishDiagnosticsParams) -> Result<(), Error> {
        Server::publish_diagnostics(self, ctx, params)
    }
    fn refresh_inlay_hints(&self, ctx: &Context) -> Result<(), Error> {
        Server::refresh_inlay_hints(self, ctx)
    }
    fn refresh_code_lens(&self, ctx: &Context) -> Result<(), Error> {
        Server::refresh_code_lens(self, ctx)
    }
    fn progress_start(&self, message: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]) {
        Server::progress_start(self, message, args)
    }
    fn progress_finish(&self, message: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]) {
        Server::progress_finish(self, message, args)
    }
    fn send_telemetry(&self, ctx: &Context, telemetry: lsproto::TelemetryEvent) -> Result<(), Error> {
        Server::send_telemetry(self, ctx, telemetry)
    }
    fn is_active(&self) -> bool {
        Server::is_active(self)
    }
    fn set_locale(&self, locale: &str) {
        Server::set_locale(self, locale)
    }
    fn get_locale(&self) -> Locale {
        Server::get_locale(self)
    }
}

// server.go:98: `_ ata.NpmExecutor = (*Server)(nil)`.
impl project::NpmExecutor for Server {
    // server.go:2371
    // NpmInstall implements ata.NpmExecutor
    fn npm_install(&self, cwd: &str, args: &[String]) -> Result<Vec<u8>, String> {
        match &self.npm_install {
            Some(npm_install) => npm_install(cwd, args),
            None => panic!("NpmInstall called without an npm executor"),
        }
    }
}

// Go's errgroup.WithContext: the first member that returns an error cancels the group's context; Wait returns
// that error after every member has returned.
#[derive(Clone)]
struct errGroup {
    inner: Arc<errGroupInner>,
}

struct errGroupInner {
    first: Mutex<Option<Error>>,
    cancel: CancelFunc,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

impl errGroup {
    fn with_context(ctx: &Context) -> (errGroup, Context) {
        let (ctx, cancel) = ctx.with_cancel();
        (errGroup { inner: Arc::new(errGroupInner { first: Mutex::new(None), cancel, handles: Mutex::new(Vec::new()) }) }, ctx)
    }

    fn record(&self, result: Result<(), Error>) {
        if let Err(err) = result {
            let mut first = self.inner.first.lock().unwrap();
            if first.is_none() {
                *first = Some(err);
                drop(first);
                self.inner.cancel.call();
            }
        }
    }

    fn go(&self, name: &str, f: impl FnOnce() -> Result<(), Error> + Send + 'static) {
        let g = self.clone();
        let handle = spawn_server_thread(name, move || g.record(f()));
        self.inner.handles.lock().unwrap().push(handle);
    }

    fn wait(&self) -> Option<Error> {
        let handles = std::mem::take(&mut *self.inner.handles.lock().unwrap());
        for handle in handles {
            let _ = handle.join();
        }
        self.inner.cancel.call();
        self.inner.first.lock().unwrap().clone()
    }
}

// The server's own threads (read, dispatch, write) get the same 512 MB stacks as the request workers: the
// dispatch thread builds programs (parse + bind) for synchronous handlers. A panic on them ends the process like
// an unrecovered panic in a Go goroutine.
fn spawn_server_thread(name: &str, f: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(workerpool::WORKER_STACK_SIZE)
        .spawn(move || workerpool::run_or_exit(f))
        .expect("failed to spawn a language server thread")
}

// A scope guard for Go's `defer f()`.
struct deferCall<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for deferCall<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

fn unix_millis_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// locale.Parse: English only (docs/LSP.md); any tag parses and is kept as text.
fn locale_parse(s: &str) -> Option<Locale> {
    if s.is_empty() {
        return None;
    }
    Some(Locale(s.to_string()))
}

// Go `fmt.Sprint(r)` of a recovered panic value.
fn panic_value_string(r: &Box<dyn Any + Send>) -> String {
    if let Some(s) = r.downcast_ref::<&str>() {
        return s.to_string();
    }
    if let Some(s) = r.downcast_ref::<String>() {
        return s.clone();
    }
    "panic".to_string()
}

thread_local! {
    static RECOVER_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static PANIC_STACK: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

// Runs `f`, catching a panic like a deferred Go `recover()`: inside the scope the panic hook records the stack
// at the panic site (Go's debug.Stack() in the deferred function) instead of printing the panic.
fn recover_scope<R>(f: impl FnOnce() -> R) -> Result<R, (Box<dyn Any + Send>, String)> {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if RECOVER_DEPTH.with(|d| d.get()) > 0 {
                let stack = std::backtrace::Backtrace::force_capture().to_string();
                PANIC_STACK.with(|s| *s.borrow_mut() = Some(stack));
                return;
            }
            previous(info);
        }));
    });
    RECOVER_DEPTH.with(|d| d.set(d.get() + 1));
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    RECOVER_DEPTH.with(|d| d.set(d.get() - 1));
    result.map_err(|payload| (payload, PANIC_STACK.with(|s| s.borrow_mut().take()).unwrap_or_default()))
}

// Go `fmt.Sprintf("%+v", v)` of a decoded JSON value (`any`): nil, bool, float64, string, []any, map[string]any
// (maps print with sorted keys).
fn go_sprint_value(v: &lsproto::Value) -> String {
    match v {
        lsproto::Value::Null => "<nil>".to_string(),
        lsproto::Value::Bool(b) => b.to_string(),
        lsproto::Value::Number(n) => go_format_float(*n),
        lsproto::Value::String(s) => s.clone(),
        lsproto::Value::Array(items) => format!("[{}]", items.iter().map(go_sprint_value).collect::<Vec<_>>().join(" ")),
        lsproto::Value::Object(members) => {
            let mut keys: Vec<&String> = members.keys().collect();
            keys.sort();
            format!("map[{}]", keys.iter().map(|k| format!("{}:{}", k, go_sprint_value(&members[*k]))).collect::<Vec<_>>().join(" "))
        }
    }
}

// Go `%v` of a float64 (strconv 'g' with the shortest representation; exponent form for large/small values).
fn go_format_float(f: f64) -> String {
    if f == f.trunc() && f.abs() < 1e21 {
        return format!("{}", f as i64);
    }
    let exp = if f == 0.0 { 0 } else { f.abs().log10().floor() as i32 };
    if exp < -4 || exp >= 21 {
        let s = format!("{:e}", f);
        let (mantissa, exponent) = s.split_once('e').unwrap();
        let exponent: i32 = exponent.parse().unwrap();
        return format!("{}e{}{:02}", mantissa, if exponent < 0 { '-' } else { '+' }, exponent.abs());
    }
    format!("{}", f)
}

// Go `%T` of a decoded JSON value.
fn go_type_name(v: &lsproto::Value) -> &'static str {
    match v {
        lsproto::Value::Null => "<nil>",
        lsproto::Value::Bool(_) => "bool",
        lsproto::Value::Number(_) => "float64",
        lsproto::Value::String(_) => "string",
        lsproto::Value::Array(_) => "[]interface {}",
        lsproto::Value::Object(_) => "map[string]interface {}",
    }
}

// Go `time.Duration.String()`.
pub(crate) fn go_duration_string(d: Duration) -> String {
    let nanos = d.as_nanos() as u64;
    if nanos == 0 {
        return "0s".to_string();
    }
    fn frac(v: u64, prec: u32) -> String {
        let pow = 10u64.pow(prec);
        let int = v / pow;
        let mut f = v % pow;
        if f == 0 {
            return int.to_string();
        }
        let mut digits = prec;
        while f % 10 == 0 {
            f /= 10;
            digits -= 1;
        }
        format!("{}.{:0width$}", int, f, width = digits as usize)
    }
    if nanos < 1_000 {
        return format!("{}ns", nanos);
    }
    if nanos < 1_000_000 {
        return format!("{}µs", frac(nanos, 3));
    }
    if nanos < 1_000_000_000 {
        return format!("{}ms", frac(nanos, 6));
    }
    let mut out = String::new();
    let hours = nanos / 3_600_000_000_000;
    let rem = nanos % 3_600_000_000_000;
    let minutes = rem / 60_000_000_000;
    let secs = rem % 60_000_000_000;
    if hours > 0 {
        out.push_str(&format!("{}h", hours));
    }
    if hours > 0 || minutes > 0 {
        out.push_str(&format!("{}m", minutes));
    }
    out.push_str(&format!("{}s", frac(secs, 9)));
    out
}
