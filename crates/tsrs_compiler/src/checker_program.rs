// `impl checker.Program for *compiler.Program` (the Go assertion `var _ checker.Program = (*Program)(nil)`).
// Rust checkers retain the data separately from the program's pool.

use std::sync::Arc;

use rustc_hash::FxHashMap;
use tsrs_ast::{Node, SourceFile, SourceFileMetaData};
use tsrs_tsoptions::{ParsedCommandLine, SourceOutputAndProjectReference};
use tsrs_core::tspath::Path;
use tsrs_core::{CompilerOptions, ModuleKind, ResolutionMode, P};
use tsrs_module::symlinks::KnownSymlinks;
use tsrs_module::{packagejson, ModeAwareCacheKey, ResolvedModule};

use crate::program::{Program, ProgramData};

impl tsrs_checker::Program for ProgramData {
    fn options(&self) -> P<CompilerOptions> {
        ProgramData::options(self)
    }

    fn source_files(&self) -> Arc<[P<SourceFile>]> {
        Arc::clone(&self.files)
    }

    fn bind_source_files(&self) {
        ProgramData::bind_source_files(self)
    }

    fn file_exists(&self, file_name: &str) -> bool {
        ProgramData::file_exists(self, file_name)
    }

    fn get_source_file(&self, file_name: &str) -> Option<P<SourceFile>> {
        ProgramData::get_source_file(self, file_name)
    }

    fn get_source_file_for_resolved_module(&self, file_name: &str) -> Option<P<SourceFile>> {
        ProgramData::get_source_file_for_resolved_module(self, file_name)
    }

    fn get_emit_module_format_of_file(&self, source_file: P<SourceFile>) -> ModuleKind {
        ProgramData::get_emit_module_format_of_file(self, source_file)
    }

    fn get_emit_syntax_for_usage_location(&self, source_file: P<SourceFile>, usage_location: P<Node>) -> ResolutionMode {
        ProgramData::get_emit_syntax_for_usage_location(self, source_file, usage_location)
    }

    fn get_implied_node_format_for_emit(&self, source_file: P<SourceFile>) -> ModuleKind {
        ProgramData::get_implied_node_format_for_emit(self, source_file)
    }

    fn get_resolved_module(&self, current_source_file: P<SourceFile>, module_reference: &str, mode: ResolutionMode) -> Option<P<ResolvedModule>> {
        ProgramData::get_resolved_module(self, current_source_file, module_reference, mode)
    }

    fn get_resolved_modules(&self) -> &FxHashMap<Path, FxHashMap<ModeAwareCacheKey, P<ResolvedModule>>> {
        ProgramData::get_resolved_modules(self)
    }

    fn get_packages_map(&self) -> &FxHashMap<String, bool> {
        ProgramData::get_packages_map(self)
    }

    fn get_source_file_meta_data(&self, path: &Path) -> SourceFileMetaData {
        ProgramData::get_source_file_meta_data(self, path).clone()
    }

    fn get_jsx_runtime_import_specifier(&self, path: &Path) -> (String, Option<P<Node>>) {
        ProgramData::get_jsx_runtime_import_specifier(self, path)
    }

    fn get_import_helpers_import_specifier(&self, path: &Path) -> Option<P<Node>> {
        ProgramData::get_import_helpers_import_specifier(self, path)
    }

    fn source_file_may_be_emitted(&self, source_file: P<SourceFile>, force_dts_emit: bool) -> bool {
        ProgramData::source_file_may_be_emitted(self, source_file, force_dts_emit)
    }

    fn is_source_file_default_library(&self, path: &Path) -> bool {
        ProgramData::is_source_file_default_library(self, path)
    }

    fn get_project_reference_from_output_dts(&self, path: &Path) -> Option<P<SourceOutputAndProjectReference>> {
        ProgramData::get_project_reference_from_output_dts(self, path)
    }

    fn get_project_reference_from_source(&self, path: &Path) -> Option<P<SourceOutputAndProjectReference>> {
        ProgramData::get_project_reference_from_source(self, path)
    }

    fn get_redirect_for_resolution(&self, file: P<SourceFile>) -> Option<P<ParsedCommandLine>> {
        ProgramData::get_redirect_for_resolution(self, file)
    }

    fn common_source_directory(&self) -> String {
        ProgramData::common_source_directory(self).to_string()
    }

    fn use_case_sensitive_file_names(&self) -> bool {
        ProgramData::use_case_sensitive_file_names(self)
    }

    fn get_current_directory(&self) -> String {
        ProgramData::get_current_directory(self).to_string()
    }

    fn get_default_resolution_mode_for_file(&self, file: P<SourceFile>) -> ResolutionMode {
        ProgramData::get_default_resolution_mode_for_file(self, file)
    }

    fn get_mode_for_usage_location(&self, file: P<SourceFile>, module_specifier: P<Node>) -> ResolutionMode {
        ProgramData::get_mode_for_usage_location(self, file, module_specifier)
    }

    fn as_module_specifier_generation_host(self: Arc<Self>) -> Arc<dyn tsrs_modulespecifiers::ModuleSpecifierGenerationHost> {
        self
    }
}

// Go `*compiler.Program` satisfies `modulespecifiers.ModuleSpecifierGenerationHost` structurally (its supertrait
// `OutputPathsHost` is implemented in program.rs); the node builder reaches it through
// `checker::Program::as_module_specifier_generation_host`.
impl tsrs_modulespecifiers::ModuleSpecifierGenerationHost for ProgramData {
    fn get_symlink_cache(&self) -> P<KnownSymlinks> {
        ProgramData::get_symlink_cache(self)
    }

    fn get_global_typings_cache_location(&self) -> String {
        ProgramData::get_global_typings_cache_location(self).to_string()
    }

    fn get_project_reference_from_source(&self, path: &Path) -> Option<P<tsrs_tsoptions::SourceOutputAndProjectReference>> {
        ProgramData::get_project_reference_from_source(self, path)
    }

    fn get_redirect_targets(&self, path: &Path) -> Vec<String> {
        ProgramData::get_redirect_targets(self, path).to_vec()
    }

    fn get_source_of_project_reference_if_output_included(&self, file: P<SourceFile>) -> String {
        ProgramData::get_source_of_project_reference_if_output_included(self, file.file_name(), &file.path())
    }

    fn file_exists(&self, path: &str) -> bool {
        ProgramData::file_exists(self, path)
    }

    fn get_nearest_ancestor_directory_with_package_json(&self, dirname: &str) -> String {
        ProgramData::get_nearest_ancestor_directory_with_package_json(self, dirname)
    }

    fn get_package_json_info(&self, pkg_json_path: &str) -> Option<P<packagejson::InfoCacheEntry>> {
        ProgramData::get_package_json_info(self, pkg_json_path)
    }

    fn get_default_resolution_mode_for_file(&self, file: P<SourceFile>) -> ResolutionMode {
        ProgramData::get_default_resolution_mode_for_file(self, file)
    }

    fn get_resolved_module_from_module_specifier(&self, file: P<SourceFile>, module_specifier: P<Node>) -> Option<P<ResolvedModule>> {
        ProgramData::get_resolved_module_from_module_specifier(self, file, module_specifier)
    }

    fn get_mode_for_usage_location(&self, file: P<SourceFile>, module_specifier: P<Node>) -> ResolutionMode {
        ProgramData::get_mode_for_usage_location(self, file, module_specifier)
    }

    fn exports_module_name_cache(&self, options: &tsrs_core::CompilerOptions) -> Option<&tsrs_modulespecifiers::ExportsModuleNameCache> {
        std::ptr::eq(options, &raw const *self.options()).then_some(&self.exports_module_name_cache)
    }
}

impl tsrs_modulespecifiers::ModuleSpecifierGenerationHost for Program {
    fn get_symlink_cache(&self) -> P<KnownSymlinks> {
        ProgramData::get_symlink_cache(self)
    }

    fn get_global_typings_cache_location(&self) -> String {
        ProgramData::get_global_typings_cache_location(self).to_string()
    }

    fn get_project_reference_from_source(&self, path: &Path) -> Option<P<tsrs_tsoptions::SourceOutputAndProjectReference>> {
        ProgramData::get_project_reference_from_source(self, path)
    }

    fn get_redirect_targets(&self, path: &Path) -> Vec<String> {
        ProgramData::get_redirect_targets(self, path).to_vec()
    }

    fn get_source_of_project_reference_if_output_included(&self, file: P<SourceFile>) -> String {
        ProgramData::get_source_of_project_reference_if_output_included(self, file.file_name(), &file.path())
    }

    fn file_exists(&self, path: &str) -> bool {
        ProgramData::file_exists(self, path)
    }

    fn get_nearest_ancestor_directory_with_package_json(&self, dirname: &str) -> String {
        ProgramData::get_nearest_ancestor_directory_with_package_json(self, dirname)
    }

    fn get_package_json_info(&self, pkg_json_path: &str) -> Option<P<packagejson::InfoCacheEntry>> {
        ProgramData::get_package_json_info(self, pkg_json_path)
    }

    fn get_default_resolution_mode_for_file(&self, file: P<SourceFile>) -> ResolutionMode {
        ProgramData::get_default_resolution_mode_for_file(self, file)
    }

    fn get_resolved_module_from_module_specifier(&self, file: P<SourceFile>, module_specifier: P<Node>) -> Option<P<ResolvedModule>> {
        ProgramData::get_resolved_module_from_module_specifier(self, file, module_specifier)
    }

    fn get_mode_for_usage_location(&self, file: P<SourceFile>, module_specifier: P<Node>) -> ResolutionMode {
        ProgramData::get_mode_for_usage_location(self, file, module_specifier)
    }

    fn exports_module_name_cache(&self, options: &tsrs_core::CompilerOptions) -> Option<&tsrs_modulespecifiers::ExportsModuleNameCache> {
        std::ptr::eq(options, &raw const *self.options()).then_some(&self.exports_module_name_cache)
    }
}
