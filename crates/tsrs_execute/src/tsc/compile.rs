use std::sync::Arc;
use std::time::{Duration, Instant};

use tsrs_ast::Diagnostic;
use tsrs_core::P;
use tsrs_vfs::FS;

pub trait System: Sync {
    fn fs(&self) -> Arc<dyn FS>;
    fn default_library_path(&self) -> &str;
    fn get_current_directory(&self) -> &str;
    fn write(&self, text: &str);
    fn flush(&self);
    fn write_output_is_tty(&self) -> bool;
    fn get_environment_variable(&self, name: &str) -> Option<String>;

    fn now(&self) -> Instant;
    fn since_start(&self) -> Duration;

    // Go `sys.Now()` as a wall-clock time (time stamps written by --build).
    fn now_time(&self) -> std::time::SystemTime {
        std::time::SystemTime::now()
    }

    // Go `sys.Now().Format("03:04:05 PM")` (build status lines), in local time.
    fn format_time_now(&self) -> String {
        format_local_time_03_04_05_pm(self.now_time())
    }

    /// tsrs-only: where diagnostics go instead of the text reporters (the WebAssembly build's JSON diagnostics).
    /// When it is set, the reporters pass each diagnostic to it, the error summary prints nothing, and `tsc -b` is
    /// refused. The CLI and the tsctests harness have none.
    fn diagnostic_sink(&self) -> Option<&(dyn Fn(P<Diagnostic>) + Sync)> {
        None
    }
}

#[cfg(unix)]
pub fn format_local_time_03_04_05_pm(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as libc::time_t).unwrap_or(0);
    // SAFETY: `libc::tm` is C integers (and, on some platforms, a nullable `char *`), for which all zeros is valid.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to live locals; localtime_r reads `secs` and writes only into `tm`.
    unsafe { libc::localtime_r(&raw const secs, &raw mut tm) };
    let hour12 = match tm.tm_hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{:02}:{:02}:{:02} {}", hour12, tm.tm_min, tm.tm_sec, if tm.tm_hour < 12 { "AM" } else { "PM" })
}

/// Targets without `localtime_r` (WASI has no time zone): UTC, as Go's js/wasm reports it.
#[cfg(not(unix))]
pub fn format_local_time_03_04_05_pm(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) % 86_400;
    let (hour, min, sec) = (secs / 3600, secs / 60 % 60, secs % 60);
    let hour12 = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{:02}:{:02}:{:02} {}", hour12, min, sec, if hour < 12 { "AM" } else { "PM" })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(i32)]
pub enum ExitStatus {
    Success = 0,
    DiagnosticsPresent_OutputsSkipped = 1,
    DiagnosticsPresent_OutputsGenerated = 2,
    InvalidProject_OutputsSkipped = 3,
    ProjectReferenceCycle_OutputsSkipped = 4,
    NotImplemented = 5,
}

// Go `tsc.CommandLineTesting`: hooks of the tsctests harness (execute/tsctests); the CLI passes none.
pub trait CommandLineTesting: Sync {
    fn on_list_files_start(&self, _w: &dyn Fn(&str)) {}
    fn on_list_files_end(&self, _w: &dyn Fn(&str)) {}
    fn on_statistics_start(&self, _w: &dyn Fn(&str)) {}
    fn on_statistics_end(&self, _w: &dyn Fn(&str)) {}
    fn on_build_status_report_start(&self, _w: &dyn Fn(&str)) {}
    fn on_build_status_report_end(&self, _w: &dyn Fn(&str)) {}
    fn on_emitted_files(&self, _result: Option<&tsrs_compiler::EmitResult>, _m_times_cache: Option<&MTimesCache>) {}
    fn on_program(&self, _program: P<tsrs_incremental::Program>) {}
    // Go `GetTrace(w, locale)`: the trace function for a program's compiler host, writing to `w`.
    fn get_trace(&'static self, w: SyncWriter, is_sys_writer: bool) -> Box<tsrs_compiler::TraceFn>;
}

pub type SyncWriter = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

// The build host's mtime cache (Go `*collections.SyncMap[tspath.Path, time.Time]`, zero time = None).
pub type MTimesCache = std::sync::Arc<std::sync::Mutex<rustc_hash::FxHashMap<tsrs_core::tspath::Path, Option<std::time::SystemTime>>>>;

// emit.go:21 GetTraceWithWriterFromSys
pub fn get_trace_with_writer_from_sys(w: SyncWriter, is_sys_writer: bool, testing: Option<&'static dyn CommandLineTesting>) -> Box<tsrs_compiler::TraceFn> {
    match testing {
        None => Box::new(move |msg: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]| {
            w(&format!("{}\n", msg.localize(args)));
        }),
        Some(testing) => testing.get_trace(w, is_sys_writer),
    }
}

pub struct CommandLineResult {
    pub status: ExitStatus,
}

#[derive(Default, Clone, Copy)]
pub struct CompileTimes {
    pub config_time: Duration,
    pub parse_time: Duration,
    pub bind_time: Duration,
    pub check_time: Duration,
    pub total_time: Duration,
    pub emit_time: Duration,
    pub build_info_read_time: Duration,
    pub changes_compute_time: Duration,
}

pub struct CompileAndEmitResult {
    pub diagnostics: Vec<P<Diagnostic>>,
    pub emit_skipped: bool,
    // Go `EmitResult.EmittedFiles`.
    pub emitted_files: Vec<String>,
    pub status: ExitStatus,
    pub times: CompileTimes,
}
