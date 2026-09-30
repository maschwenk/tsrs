// `impl checker.Program for *compiler.Program` (the Go assertion `var _ checker.Program = (*Program)(nil)`).

use rustc_hash::FxHashMap;
use tsrs_ast::{Node, SourceFile, SourceFileMetaData};
use tsrs_checker::{ProjectReferenceCommandLine, SourceOutputAndProjectReference};
use tsrs_core::tspath::Path;
use tsrs_core::{CompilerOptions, ModuleKind, ResolutionMode, P};
use tsrs_module::{ModeAwareCacheKey, ResolvedModule};

use crate::program::Program;

impl tsrs_checker::Program for Program {
    fn options(&self) -> P<CompilerOptions> {
        Program::options(self)
    }

    fn source_files(&self) -> &'static [P<SourceFile>] {
        self.files
    }

    fn bind_source_files(&self) {
        Program::bind_source_files(self)
    }

    fn file_exists(&self, file_name: &str) -> bool {
        Program::file_exists(self, file_name)
    }

    fn get_source_file(&self, file_name: &str) -> Option<P<SourceFile>> {
        Program::get_source_file(self, file_name)
    }

    fn get_source_file_for_resolved_module(&self, file_name: &str) -> Option<P<SourceFile>> {
        Program::get_source_file_for_resolved_module(self, file_name)
    }

    fn get_emit_module_format_of_file(&self, source_file: P<SourceFile>) -> ModuleKind {
        Program::get_emit_module_format_of_file(self, source_file)
    }

    fn get_emit_syntax_for_usage_location(&self, source_file: P<SourceFile>, usage_location: P<Node>) -> ResolutionMode {
        Program::get_emit_syntax_for_usage_location(self, source_file, usage_location)
    }

    fn get_implied_node_format_for_emit(&self, source_file: P<SourceFile>) -> ModuleKind {
        Program::get_implied_node_format_for_emit(self, source_file)
    }

    fn get_resolved_module(&self, current_source_file: P<SourceFile>, module_reference: &str, mode: ResolutionMode) -> Option<P<ResolvedModule>> {
        Program::get_resolved_module(self, current_source_file, module_reference, mode)
    }

    fn get_resolved_modules(&self) -> &FxHashMap<Path, FxHashMap<ModeAwareCacheKey, P<ResolvedModule>>> {
        Program::get_resolved_modules(self)
    }

    fn get_packages_map(&self) -> &FxHashMap<String, bool> {
        Program::get_packages_map(self)
    }

    fn get_source_file_meta_data(&self, path: &Path) -> SourceFileMetaData {
        Program::get_source_file_meta_data(self, path)
    }

    fn get_jsx_runtime_import_specifier(&self, path: &Path) -> (String, Option<P<Node>>) {
        Program::get_jsx_runtime_import_specifier(self, path)
    }

    fn get_import_helpers_import_specifier(&self, path: &Path) -> Option<P<Node>> {
        Program::get_import_helpers_import_specifier(self, path)
    }

    fn source_file_may_be_emitted(&self, source_file: P<SourceFile>, force_dts_emit: bool) -> bool {
        Program::source_file_may_be_emitted(self, source_file, force_dts_emit)
    }

    fn is_source_file_default_library(&self, path: &Path) -> bool {
        Program::is_source_file_default_library(self, path)
    }

    // Project references are not ported: there are never output-dts mappings or redirects.
    fn get_project_reference_from_output_dts(&self, path: &Path) -> Option<&'static SourceOutputAndProjectReference> {
        let _ = Program::get_project_reference_from_output_dts(self, path);
        None
    }

    fn get_project_reference_from_source(&self, path: &Path) -> Option<&'static SourceOutputAndProjectReference> {
        let _ = Program::get_project_reference_from_source(self, path);
        None
    }

    fn get_redirect_for_resolution(&self, file: P<SourceFile>) -> Option<&'static dyn ProjectReferenceCommandLine> {
        let _ = Program::get_redirect_for_resolution(self, file);
        None
    }

    fn common_source_directory(&self) -> String {
        Program::common_source_directory(self).to_string()
    }

    fn use_case_sensitive_file_names(&self) -> bool {
        Program::use_case_sensitive_file_names(self)
    }

    fn get_current_directory(&self) -> String {
        Program::get_current_directory(self).to_string()
    }

    fn get_default_resolution_mode_for_file(&self, file: P<SourceFile>) -> ResolutionMode {
        Program::get_default_resolution_mode_for_file(self, file)
    }

    fn get_mode_for_usage_location(&self, file: P<SourceFile>, module_specifier: P<Node>) -> ResolutionMode {
        Program::get_mode_for_usage_location(self, file, module_specifier)
    }
}
