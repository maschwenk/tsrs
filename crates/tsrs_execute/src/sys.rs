// Port of cmd/tsc/sys.go (osSys).

use std::io::{IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tsrs_core::tspath;
use tsrs_vfs::{bundled, osvfs, FS};

use crate::tsc::System;

pub struct osSys {
    writer: Mutex<std::io::BufWriter<std::io::Stdout>>,
    fs: Arc<dyn FS>,
    default_library_path: String,
    cwd: String,
    start: Instant,
}

impl System for osSys {
    fn since_start(&self) -> Duration {
        self.start.elapsed()
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

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
        std::io::stdout().is_terminal()
    }

    fn get_environment_variable(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

pub fn new_system() -> osSys {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("Error getting current directory: {}", err);
            std::process::exit(crate::tsc::ExitStatus::InvalidProject_OutputsSkipped as i32);
        }
    };

    osSys {
        cwd: tspath::normalize_path(&cwd.to_string_lossy()),
        fs: Arc::new(bundled::wrap_fs(osvfs::fs())),
        default_library_path: default_library_path(),
        writer: Mutex::new(std::io::BufWriter::new(std::io::stdout())),
        start: Instant::now(),
    }
}

// Go's embedded build reads the default libraries from `bundled:///libs`; its `noembed` build (the npm `tsgo`)
// reads them from the executable's directory, which changes path-sorted tsbuildinfo fields. tsrs-only:
// TSRS_LIB_PATH=<dir containing lib.d.ts> behaves like the noembed build (the emit oracles point it at tsgo's
// directory when comparing tsbuildinfo with the npm tsgo).
fn default_library_path() -> String {
    if let Ok(dir) = std::env::var("TSRS_LIB_PATH") {
        if !dir.is_empty() {
            return tspath::normalize_path(&dir);
        }
    }
    bundled::lib_path()
}
