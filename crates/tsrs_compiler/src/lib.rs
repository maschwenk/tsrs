#[cfg(feature = "checker")]
mod checker_program;
mod checkerpool;
#[cfg(feature = "checker")]
mod checkerpool_stats;
pub mod diagnosticwriter;
mod emitter;
#[cfg(feature = "checker")]
mod emithost;
mod file_include;
mod fileloader;
mod filesparser;
mod host;
mod includeprocessor;
mod outputpaths;
mod processing_diagnostic;
mod program;
mod projectreferencedtsfakinghost;
mod projectreferencefilemapper;
mod projectreferenceparser;
#[cfg(all(test, feature = "checker"))]
mod program_test;
#[cfg(all(test, feature = "checker"))]
mod modulespecifiers_oracle_test;

pub use checkerpool::{assignment_stats_enabled, set_checker_assignment_from_cli, set_checker_cost_cache_from_cli, Checker, CheckerHandle, CheckerPool, Context, PooledChecker};
pub use file_include::FileIncludeReason;
pub use fileloader::{DuplicateSourceFile, LibFile};
pub use host::{new_cached_fs_compiler_host, new_compiler_host, CompilerHost, TraceFn};
pub use program::{
    filter_no_emit_semantic_diagnostics, get_diagnostics_of_any_program, new_program, sort_and_deduplicate_diagnostics, CreateCheckerPool,
    CreateModuleResolver, Program, ProgramConfig, ProgramOptions,
};
