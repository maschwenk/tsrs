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
    pub status: ExitStatus,
    pub times: CompileTimes,
}
