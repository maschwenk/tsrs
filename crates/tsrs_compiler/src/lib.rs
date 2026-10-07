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
mod fileregions;
mod filesparser;
mod host;
mod includeprocessor;
mod affinity;
mod outputpaths;
mod processing_diagnostic;
mod program;
#[cfg(feature = "checker")]
mod program_emit;
#[cfg(feature = "checker")]
mod programlike;
mod splitcheck;
mod projectreferencedtsfakinghost;
mod projectreferencefilemapper;
mod projectreferenceparser;
#[cfg(all(test, feature = "checker"))]
mod program_test;
#[cfg(all(test, feature = "checker"))]
mod modulespecifiers_oracle_test;

pub use checkerpool::{assignment_stats_enabled, checker_count_upper_bound, set_checker_assignment_from_cli, set_checker_cost_cache_from_cli, use_go_default_checker_count, Checker, CheckerHandle, CheckerPool, Context, PooledChecker};
#[cfg(feature = "checker")]
pub use emitter::EmitOnly;
#[cfg(feature = "checker")]
pub use programlike::{get_diagnostics_of_any_program_like, ProgramLike};
#[cfg(feature = "checker")]
pub use program_emit::{combine_emit_results, handle_no_emit_options, EmitOptions, EmitResult, SourceMapEmitResult, WriteFile, WriteFileData};
pub use file_include::FileIncludeReason;
pub use fileregions::{enable as enable_file_regions, leaf_settings_from_env, stats_report as leaf_stats_report, LeafMode, LeafSettings};
pub use fileloader::{DuplicateSourceFile, LibFile};
pub use host::{new_cached_fs_compiler_host, new_compiler_host, CompilerHost, TraceFn};
pub use program::{
    filter_no_emit_semantic_diagnostics, free_program, free_unshared_program, shared_program_data, SharedProgramData, get_diagnostics_of_any_program, new_program, sort_and_deduplicate_diagnostics, CreateCheckerPool,
    CreateModuleResolver, Program, ProgramConfig, ProgramOptions, worker_pool,
};
