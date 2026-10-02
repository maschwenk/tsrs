// Minimal port of Go internal/testutil/projecttestutil (projecttestutil.go + the generated client mock),
// for this crate's tests. ATA helpers (types registry, npm executor) are not ported.

use std::fmt::Display;
use std::sync::{Arc, Mutex};

use tsrs_core::context::{Context, Locale};
use tsrs_core::tspath::{self, Path};
use tsrs_diagnostics::Message;
use tsrs_lsproto as lsproto;
use tsrs_vfs::{bundled, vfstest, FS};

use crate::client::Client;
use crate::logging::{new_test_logger, LogCollector, Logger};
use crate::session::{new_session, Session, SessionInit, SessionOptions};
use crate::watch::WatcherID;

pub(crate) const TestTypingsLocation: &str = "/home/src/Library/Caches/typescript";

#[derive(Clone)]
pub(crate) struct WatchFilesCall {
    pub(crate) id: WatcherID,
    pub(crate) watchers: Vec<lsproto::FileSystemWatcher>,
}

// clientmock_generated.go (call recording only; every call succeeds unless a hook says otherwise)
#[derive(Default)]
pub(crate) struct ClientMock {
    pub(crate) watch_files_calls: Mutex<Vec<WatchFilesCall>>,
    pub(crate) unwatch_files_calls: Mutex<Vec<WatcherID>>,
    pub(crate) refresh_diagnostics_calls: Mutex<usize>,
    pub(crate) refresh_inlay_hints_calls: Mutex<usize>,
    pub(crate) refresh_code_lens_calls: Mutex<usize>,
    pub(crate) publish_diagnostics_calls: Mutex<Vec<lsproto::PublishDiagnosticsParams>>,
    pub(crate) locale: Mutex<String>,
    pub(crate) watch_files_func: Mutex<Option<Box<dyn Fn(&WatcherID) -> Result<(), lsproto::Error> + Send + Sync>>>,
}

impl ClientMock {
    pub(crate) fn watch_files_calls(&self) -> Vec<WatchFilesCall> {
        self.watch_files_calls.lock().unwrap().clone()
    }
    pub(crate) fn unwatch_files_calls(&self) -> Vec<WatcherID> {
        self.unwatch_files_calls.lock().unwrap().clone()
    }
    pub(crate) fn refresh_diagnostics_calls(&self) -> usize {
        *self.refresh_diagnostics_calls.lock().unwrap()
    }
    pub(crate) fn publish_diagnostics_calls(&self) -> Vec<lsproto::PublishDiagnosticsParams> {
        self.publish_diagnostics_calls.lock().unwrap().clone()
    }
}

impl Client for ClientMock {
    fn watch_files(&self, _ctx: &Context, id: WatcherID, watchers: Vec<lsproto::FileSystemWatcher>) -> Result<(), lsproto::Error> {
        self.watch_files_calls.lock().unwrap().push(WatchFilesCall { id: id.clone(), watchers });
        if let Some(f) = &*self.watch_files_func.lock().unwrap() {
            return f(&id);
        }
        Ok(())
    }
    fn unwatch_files(&self, _ctx: &Context, id: WatcherID) -> Result<(), lsproto::Error> {
        self.unwatch_files_calls.lock().unwrap().push(id);
        Ok(())
    }
    fn register_content_mapper_extensions(&self, _ctx: &Context, _extensions: Vec<String>) -> Result<(), lsproto::Error> {
        Ok(())
    }
    fn refresh_diagnostics(&self, _ctx: &Context) -> Result<(), lsproto::Error> {
        *self.refresh_diagnostics_calls.lock().unwrap() += 1;
        Ok(())
    }
    fn publish_diagnostics(&self, _ctx: &Context, params: lsproto::PublishDiagnosticsParams) -> Result<(), lsproto::Error> {
        self.publish_diagnostics_calls.lock().unwrap().push(params);
        Ok(())
    }
    fn refresh_inlay_hints(&self, _ctx: &Context) -> Result<(), lsproto::Error> {
        *self.refresh_inlay_hints_calls.lock().unwrap() += 1;
        Ok(())
    }
    fn refresh_code_lens(&self, _ctx: &Context) -> Result<(), lsproto::Error> {
        *self.refresh_code_lens_calls.lock().unwrap() += 1;
        Ok(())
    }
    fn progress_start(&self, _message: &'static Message, _args: &[&dyn Display]) {}
    fn progress_finish(&self, _message: &'static Message, _args: &[&dyn Display]) {}
    fn send_telemetry(&self, _ctx: &Context, _telemetry: lsproto::TelemetryEvent) -> Result<(), lsproto::Error> {
        Ok(())
    }
    fn is_active(&self) -> bool {
        true
    }
    fn set_locale(&self, locale: &str) {
        *self.locale.lock().unwrap() = locale.to_string();
    }
    fn get_locale(&self) -> Locale {
        Locale(self.locale.lock().unwrap().clone())
    }
}

pub(crate) struct SessionUtils {
    current_directory: String,
    fs_from_file_map: Arc<tsrs_vfs::iovfs::IoVFS<vfstest::MapFS>>,
    fs: Arc<dyn FS>,
    client: Arc<ClientMock>,
    logger: Arc<dyn LogCollector>,
}

impl SessionUtils {
    pub(crate) fn map_fs(&self) -> &vfstest::MapFS {
        self.fs_from_file_map.fsys()
    }

    // projecttestutil.go:127
    // WatchesFile reports whether any registered file watcher would match the given
    // file path. It handles both absolute glob patterns and relative patterns with
    // a base URI. On case-insensitive file systems the paths in glob patterns are
    // lowercased, so callers should pass the lowercased path.
    pub(crate) fn watches_file(&self, file_path: &str) -> bool {
        for call in self.client.watch_files_calls() {
            for watcher in &call.watchers {
                if let Some(pattern) = &watcher.glob_pattern.pattern {
                    if let Ok(g) = tsrs_core::glob::parse(pattern) {
                        if g.match_(file_path) {
                            return true;
                        }
                    }
                } else if let Some(rp) = &watcher.glob_pattern.relative_pattern {
                    let base_uri = rp.base_uri.uri.as_ref().unwrap().0.clone();
                    // Convert base URI (e.g. "file:///home/projects") to a directory path
                    // with trailing separator for proper prefix matching on path boundaries.
                    let base_dir = tspath::ensure_trailing_directory_separator(&lsproto::DocumentUri(base_uri).file_name());
                    if let Some(relative_path) = file_path.strip_prefix(base_dir.as_str()) {
                        if let Ok(g) = tsrs_core::glob::parse(&rp.pattern) {
                            if g.match_(relative_path) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    pub(crate) fn client(&self) -> &Arc<ClientMock> {
        &self.client
    }

    pub(crate) fn to_path(&self, file_name: &str) -> Path {
        tspath::to_path(file_name, &self.current_directory, self.fs.use_case_sensitive_file_names())
    }

    pub(crate) fn fs(&self) -> &Arc<dyn FS> {
        &self.fs
    }

    pub(crate) fn logs(&self) -> String {
        self.logger.string()
    }
}

pub(crate) fn default_session_options() -> SessionOptions {
    SessionOptions {
        current_directory: "/".to_string(),
        default_library_path: bundled::lib_path(),
        typings_location: TestTypingsLocation.to_string(),
        position_encoding: lsproto::PositionEncodingKind::UTF8,
        watch_enabled: true,
        logging_enabled: true,
        push_diagnostics_enabled: true,
        ..Default::default()
    }
}

// projecttestutil.go:225
pub(crate) fn setup(files: &[(&str, &str)]) -> (Arc<Session>, SessionUtils) {
    setup_with_options(files, None)
}

// projecttestutil.go:265
pub(crate) fn setup_with_options(files: &[(&str, &str)], options: Option<SessionOptions>) -> (Arc<Session>, SessionUtils) {
    let (init, session_utils) = get_session_init_options(files, options);
    (new_session(init), session_utils)
}

// projecttestutil.go:285
pub(crate) fn get_session_init_options(files: &[(&str, &str)], options: Option<SessionOptions>) -> (SessionInit, SessionUtils) {
    let fs_from_file_map = Arc::new(vfstest::from_map(
        files.iter().map(|(k, v)| match v.strip_prefix("symlink:") {
            Some(target) => (k.to_string(), vfstest::symlink(target)),
            None => (k.to_string(), vfstest::MapFile::from(v.to_string())),
        }),
        false, /*useCaseSensitiveFileNames*/
    ));
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(fs_from_file_map.clone()));
    let client_mock = Arc::new(ClientMock::default());
    let logger = new_test_logger();
    let session_utils = SessionUtils { fs_from_file_map, current_directory: "/".to_string(), fs: fs.clone(), client: client_mock.clone(), logger: logger.clone() };

    // Use provided options or create default ones
    let options = options.unwrap_or_else(default_session_options);

    let logger: Arc<dyn Logger> = logger;
    (
        SessionInit {
            background_ctx: Context::background(),
            options: Arc::new(options),
            fs,
            client: Some(client_mock),
            logger: Some(logger),
            npm_executor: None,
            parse_cache: None,
            content_mapped_parse_cache: None,
        },
        session_utils,
    )
}
