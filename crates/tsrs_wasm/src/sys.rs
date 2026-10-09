// The WebAssembly build's `System` (native tsrs's `osSys` over the host file system): stdout is WASI fd 1, the
// environment is WASI's environ, the current directory and the tty flag come from the request.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tsrs_ast::Diagnostic;
use tsrs_core::{tspath, P};
use tsrs_execute::tsc::System;
use tsrs_vfs::{bundled, FS};

use crate::host::HostFs;

type Sink = Box<dyn Fn(P<Diagnostic>) + Send + Sync>;

pub struct WasmSys {
    writer: Mutex<Box<dyn Write + Send>>,
    fs: Arc<dyn FS>,
    default_library_path: String,
    cwd: String,
    tty: bool,
    start: Instant,
    sink: Option<Sink>,
}

impl WasmSys {
    /// `diagnostics`: collect diagnostics there (the JSON reply) instead of printing them.
    /// `out`: where tsc's text goes (stdout, which is WASI fd 1 in the module).
    pub fn new(cwd: &str, case_sensitive: bool, tty: bool, diagnostics: Option<Arc<Mutex<Vec<P<Diagnostic>>>>>, out: Box<dyn Write + Send>) -> WasmSys {
        WasmSys {
            writer: Mutex::new(out),
            fs: Arc::new(bundled::wrap_fs(HostFs::new(case_sensitive))),
            default_library_path: default_library_path(),
            cwd: tspath::normalize_path(cwd),
            tty,
            start: Instant::now(),
            sink: diagnostics.map(|list| Box::new(move |d| list.lock().unwrap().push(d)) as Sink),
        }
    }
}

impl System for WasmSys {
    fn fs(&self) -> Arc<dyn FS> {
        Arc::clone(&self.fs)
    }

    fn default_library_path(&self) -> &str {
        &self.default_library_path
    }

    fn get_current_directory(&self) -> &str {
        &self.cwd
    }

    fn write(&self, text: &str) {
        let _ = self.writer.lock().unwrap().write_all(text.as_bytes());
    }

    fn flush(&self) {
        let _ = self.writer.lock().unwrap().flush();
    }

    fn write_output_is_tty(&self) -> bool {
        self.tty
    }

    fn get_environment_variable(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn since_start(&self) -> Duration {
        self.start.elapsed()
    }

    // Go's osSys.Spawn (cmd/tsc/sys.go:68), which WASI cannot carry out: the mapper's initialization then fails.
    fn spawn(&self, command: &[String], dir: &str, stderr: Option<Box<dyn Write + Send>>) -> Result<tsrs_execute::tsc::ReadWriteCloser, String> {
        tsrs_contentmapper::spawn_process(command, dir, stderr)
    }

    fn diagnostic_sink(&self) -> Option<&(dyn Fn(P<Diagnostic>) + Sync)> {
        self.sink.as_deref().map(|f| f as &(dyn Fn(P<Diagnostic>) + Sync))
    }
}

// As native `sys.rs`: the embedded libraries, or TSRS_LIB_PATH.
fn default_library_path() -> String {
    if let Ok(dir) = std::env::var("TSRS_LIB_PATH") {
        if !dir.is_empty() {
            return tspath::normalize_path(&dir);
        }
    }
    bundled::lib_path()
}
