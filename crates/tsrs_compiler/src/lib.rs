mod checker_program;
mod programregions;
mod checkerpool;
mod checkerpool_stats;
pub mod diagnosticwriter;
mod emitter;
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
mod program_emit;
mod programlike;
mod splitcheck;
mod sharedgraph;
mod projectreferencedtsfakinghost;
mod projectreferencefilemapper;
mod projectreferenceparser;
#[cfg(test)]
mod program_test;
#[cfg(test)]
mod contentmapper_test;
#[cfg(test)]
mod modulespecifiers_oracle_test;

pub use checkerpool::{assignment_stats_enabled, checker_count_upper_bound, set_checker_assignment_from_cli, set_checker_cost_cache_from_cli, parse_memory_size, set_max_memory_from_cli, use_go_default_checker_count, Checker, CheckerHandle, CheckerPool, Context, PooledChecker};
pub use emitter::EmitOnly;
pub use programlike::{get_diagnostics_of_any_program_like, ProgramLike};
pub use program_emit::{combine_emit_results, handle_no_emit_options, EmitOptions, EmitResult, SourceMapEmitResult, WriteFile, WriteFileData};
pub use file_include::FileIncludeReason;
pub use programregions::ProgramRegions;
pub use fileregions::{bind_in_file_regions, enable as enable_file_regions, enable_lazy_dts, lazy_dts_allowed, leaf_settings_from_env, stats_report as leaf_stats_report, LeafMode, LeafSettings};
pub use fileloader::{
    content_mapper_initialization_diagnostic, content_mapper_project_diagnostic, content_mapper_project_error_diagnostic, DuplicateSourceFile, LibFile,
};
pub use host::{get_content_mapped_source_files_with, new_cached_fs_compiler_host, new_compiler_host, CompilerHost, TraceFn};
pub use program::{
    filter_no_emit_semantic_diagnostics, free_program, free_unshared_program, shared_program_data, SharedProgramData, get_diagnostics_of_any_program, new_program, sort_and_deduplicate_diagnostics, CreateCheckerPool,
    CreateModuleResolver, Program, ProgramConfig, ProgramOptions, worker_pool,
};
