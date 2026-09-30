#[cfg(feature = "checker")]
mod checker_program;
mod checkerpool;
pub mod diagnosticwriter;
mod emitter;
mod file_include;
mod fileloader;
mod filesparser;
mod host;
mod includeprocessor;
mod outputpaths;
mod processing_diagnostic;
mod program;
mod projectreferencefilemapper;
#[cfg(all(test, feature = "checker"))]
mod modulespecifiers_oracle_test;

pub use checkerpool::CheckerGuard;
pub use file_include::FileIncludeReason;
pub use fileloader::LibFile;
pub use host::{new_cached_fs_compiler_host, new_compiler_host, CompilerHost, TraceFn};
pub use program::{
    filter_no_emit_semantic_diagnostics, get_diagnostics_of_any_program, new_program, sort_and_deduplicate_diagnostics, CreateModuleResolver,
    Program, ProgramConfig, ProgramOptions,
};
