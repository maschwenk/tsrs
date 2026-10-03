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
}

pub fn format_local_time_03_04_05_pm(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as libc::time_t).unwrap_or(0);
    // SAFETY: localtime_r writes only into the provided tm.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    let hour12 = match tm.tm_hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{:02}:{:02}:{:02} {}", hour12, tm.tm_min, tm.tm_sec, if tm.tm_hour < 12 { "AM" } else { "PM" })
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
    fn on_emitted_files(&self, _result: Option<&tsrs_compiler::EmitResult>) {}
    fn on_program(&self, _program: P<tsrs_incremental::Program>) {}
    // Go `GetTrace(w, locale)`: the trace function for a program's compiler host, writing to `w`.
    fn get_trace(&self, w: &(dyn Fn(&str) + Sync)) -> Option<Box<dyn Fn(&str) + Send + Sync>> {
        let _ = w;
        None
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
