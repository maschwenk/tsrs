// Only the parts of Go's emitter.go that type checking depends on. Emit itself is not ported.

use tsrs_ast::{self as ast, SourceFile};
use tsrs_core::tspath::{self, ComparePathsOptions};
use tsrs_core::P;

use crate::outputpaths;
use crate::program::Program;

pub(crate) fn source_file_may_be_emitted(source_file: P<SourceFile>, host: &Program, force_dts_emit: bool, force_js_emit: bool) -> bool {
    // TODO: move this to outputpaths?
    let options = host.options();
    // Js files are emitted only if option is enabled
    if !force_js_emit && options.no_emit_for_js_files.is_true() && ast::is_source_file_js(source_file) {
        return false;
    }

    // Declaration files are not emitted
    if source_file.is_declaration_file.get() {
        return false;
    }

    // Source file from node_modules are not emitted
    if host.is_source_file_from_external_library(source_file) {
        return false;
    }

    // forcing dts emit => file needs to be emitted
    if force_dts_emit || force_js_emit {
        return true;
    }

    // Check other conditions for file emit
    // Source files from referenced projects are not emitted
    if host.get_project_reference_from_source(&source_file.path()).is_some() {
        return false;
    }

    // Any non json file should be emitted
    if !ast::is_json_source_file(source_file) {
        return true;
    }

    // Json file is not emitted if outDir is not specified
    if options.out_dir.is_empty() {
        return false;
    }

    // Otherwise, if rootDir is specified or a config file exists, we know the common source directory and can check if the file would be emitted in the same location
    if !options.root_dir.is_empty() || !options.config_file_path.is_empty() {
        let common_dir = tspath::get_normalized_absolute_path(
            &outputpaths::get_common_source_directory(
                &options,
                Vec::new,
                host.get_current_directory(),
                host.use_case_sensitive_file_names(),
                None,
            ),
            host.get_current_directory(),
        );
        let output_path = outputpaths::get_source_file_path_in_new_dir_worker(
            source_file.file_name(),
            &options.out_dir,
            host.get_current_directory(),
            &common_dir,
            host.use_case_sensitive_file_names(),
        );
        if tspath::compare_paths(
            source_file.file_name(),
            &output_path,
            &ComparePathsOptions {
                use_case_sensitive_file_names: host.use_case_sensitive_file_names(),
                current_directory: host.get_current_directory().to_string(),
            },
        ) == 0
        {
            return false;
        }
    }

    true
}

pub(crate) fn get_source_files_to_emit(
    host: &Program,
    target_source_files: Option<&[P<SourceFile>]>,
    force_dts_emit: bool,
    force_js_emit: bool,
) -> Vec<P<SourceFile>> {
    let target_source_files = target_source_files.unwrap_or_else(|| host.source_files());
    target_source_files.iter().copied().filter(|&f| source_file_may_be_emitted(f, host, force_dts_emit, force_js_emit)).collect()
}
