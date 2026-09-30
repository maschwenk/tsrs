// Port of compiler/emitHost.go: the host the declaration transformer runs against. Only the declaration-emit
// surface is ported (the JS emit parts of Go's `printer.EmitHost` are not needed to compute diagnostics).
//
// Go `newEmitHost` acquires the file's checker from the pool and keeps it locked until `done()`; here the caller
// holds the checker (`CheckerGuard`/`&mut Checker`) and lends it to `checker_slot` for the duration of the
// transform, and the host's `Resolver` borrows it from there (see tsrs_declarations::Resolver).

use tsrs_ast::{FileReference, ModifierFlags, Node, SourceFile};
use tsrs_checker::CheckerSlot;
use tsrs_core::tspath::Path;
use tsrs_core::{ResolutionMode, P};
use tsrs_declarations::{DeclarationEmitHost, Resolver};
use tsrs_module::symlinks::KnownSymlinks;
use tsrs_module::{packagejson, ResolvedModule};
use tsrs_tsoptions::outputpaths::{self, OutputPaths, OutputPathsHost};

use crate::emitter;
use crate::program::Program;

pub(crate) struct EmitHost {
    program: &'static Program,
    emit_resolver: Resolver,
}

// emitHost.go:38 (the checker acquisition is the caller's; see above)
pub(crate) fn new_emit_host(program: &'static Program, emit_resolver: P<tsrs_checker::EmitResolver>, checker_slot: P<CheckerSlot>) -> &'static EmitHost {
    P::new(EmitHost { program, emit_resolver: Resolver::new(emit_resolver, checker_slot) }).get()
}

impl OutputPathsHost for EmitHost {
    fn common_source_directory(&self) -> String {
        OutputPathsHost::common_source_directory(self.program)
    }

    fn content_mapper_extensions(&self) -> Vec<String> {
        OutputPathsHost::content_mapper_extensions(self.program)
    }

    fn get_current_directory(&self) -> &str {
        OutputPathsHost::get_current_directory(self.program)
    }

    fn use_case_sensitive_file_names(&self) -> bool {
        OutputPathsHost::use_case_sensitive_file_names(self.program)
    }
}

impl tsrs_modulespecifiers::ModuleSpecifierGenerationHost for EmitHost {
    fn get_symlink_cache(&self) -> P<KnownSymlinks> {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_symlink_cache(self.program)
    }

    fn get_global_typings_cache_location(&self) -> String {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_global_typings_cache_location(self.program)
    }

    fn get_project_reference_from_source(&self, path: &Path) -> Option<P<tsrs_tsoptions::SourceOutputAndProjectReference>> {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_project_reference_from_source(self.program, path)
    }

    fn get_redirect_targets(&self, path: &Path) -> Vec<String> {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_redirect_targets(self.program, path)
    }

    fn get_source_of_project_reference_if_output_included(&self, file: P<SourceFile>) -> String {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_source_of_project_reference_if_output_included(self.program, file)
    }

    fn file_exists(&self, path: &str) -> bool {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::file_exists(self.program, path)
    }

    fn get_nearest_ancestor_directory_with_package_json(&self, dirname: &str) -> String {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_nearest_ancestor_directory_with_package_json(self.program, dirname)
    }

    fn get_package_json_info(&self, pkg_json_path: &str) -> Option<P<packagejson::InfoCacheEntry>> {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_package_json_info(self.program, pkg_json_path)
    }

    fn get_default_resolution_mode_for_file(&self, file: P<SourceFile>) -> ResolutionMode {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_default_resolution_mode_for_file(self.program, file)
    }

    fn get_resolved_module_from_module_specifier(&self, file: P<SourceFile>, module_specifier: P<Node>) -> Option<P<ResolvedModule>> {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_resolved_module_from_module_specifier(self.program, file, module_specifier)
    }

    fn get_mode_for_usage_location(&self, file: P<SourceFile>, module_specifier: P<Node>) -> ResolutionMode {
        tsrs_modulespecifiers::ModuleSpecifierGenerationHost::get_mode_for_usage_location(self.program, file, module_specifier)
    }
}

impl DeclarationEmitHost for EmitHost {
    // emitHost.go:103
    fn get_source_file_from_reference(&self, origin: P<SourceFile>, ref_: P<FileReference>) -> Option<P<SourceFile>> {
        self.program.get_source_file_from_reference(origin, ref_)
    }

    // emitHost.go:94
    fn get_output_paths_for(&self, file: P<SourceFile>, force_dts_paths: bool) -> OutputPaths {
        // TODO: cache
        outputpaths::get_output_paths_for(file, &self.program.options(), self, outputpaths::ForceEmitPaths { dts: force_dts_paths, ..Default::default() })
    }

    // emitHost.go:99
    fn source_file_may_be_emitted(&self, file: P<SourceFile>, force_dts_emit: bool) -> bool {
        emitter::source_file_may_be_emitted(file, self.program, force_dts_emit, false)
    }

    // emitHost.go:90
    fn get_effective_declaration_flags(&self, node: P<Node>, flags: ModifierFlags) -> ModifierFlags {
        self.get_emit_resolver().get_effective_declaration_flags(node, flags)
    }

    // emitHost.go:120 (GetEmitResolver)
    fn get_emit_resolver(&self) -> Resolver {
        self.emit_resolver
    }
}
