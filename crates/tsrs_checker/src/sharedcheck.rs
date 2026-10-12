//! Program-owned input and immutable payload storage for independent checking contexts.
//!
//! A context retains the identities, resolutions and history of one sequential checker. Sharing this input and
//! completed payloads never makes another context's cached semantic answer visible to it.

use std::sync::Arc;

use crate::*;
use crate::shareddata::SharedPayloadStore;

pub struct SharedCheckData {
    pub program: &'static dyn Program,
    pub compiler_options: P<CompilerOptions>,
    pub files: &'static [P<SourceFile>],
    pub file_index_map: FxHashMap<P<SourceFile>, usize>,
    pub language_version: ScriptTarget,
    pub module_kind: ModuleKind,
    pub module_resolution_kind: ModuleResolutionKind,
    pub legacy_decorators: bool,
    pub emit_standard_class_fields: bool,
    pub strict_null_checks: bool,
    pub strict_function_types: bool,
    pub strict_bind_call_apply: bool,
    pub strict_property_initialization: bool,
    pub strict_builtin_iterator_return: bool,
    pub no_implicit_any: bool,
    pub no_implicit_this: bool,
    pub use_unknown_in_catch_variables: bool,
    pub exact_optional_property_types: bool,
    pub can_collect_symbol_alias_accessibility_data: bool,
    pub array_variances: &'static [VarianceFlags],
    pub payloads: SharedPayloadStore,
}

impl SharedCheckData {
    /// Shared by all evaluation contexts of one batch. Bind before publishing the program's input.
    pub fn new(program: &'static dyn Program) -> Arc<Self> {
        Self::with_sharing(program, true)
    }

    /// Compatibility checkers keep payloads in their own allocation region and retain their own input snapshot.
    pub(crate) fn private(program: &'static dyn Program) -> Arc<Self> {
        Self::with_sharing(program, false)
    }

    fn with_sharing(program: &'static dyn Program, share_payloads: bool) -> Arc<Self> {
        program.bind_source_files();
        let compiler_options = program.options();
        let files = program.source_files();
        Arc::new(Self {
            program,
            compiler_options,
            files,
            file_index_map: create_file_index_map(files),
            language_version: compiler_options.get_emit_script_target(),
            module_kind: compiler_options.get_emit_module_kind(),
            module_resolution_kind: compiler_options.get_module_resolution_kind(),
            legacy_decorators: compiler_options.experimental_decorators == Tristate::True,
            emit_standard_class_fields: compiler_options.get_emit_standard_class_fields(),
            strict_null_checks: compiler_options.get_strict_option_value(compiler_options.strict_null_checks),
            strict_function_types: compiler_options.get_strict_option_value(compiler_options.strict_function_types),
            strict_bind_call_apply: compiler_options.get_strict_option_value(compiler_options.strict_bind_call_apply),
            strict_property_initialization: compiler_options.get_strict_option_value(compiler_options.strict_property_initialization),
            strict_builtin_iterator_return: compiler_options.get_strict_option_value(compiler_options.strict_builtin_iterator_return),
            no_implicit_any: compiler_options.get_strict_option_value(compiler_options.no_implicit_any),
            no_implicit_this: compiler_options.get_strict_option_value(compiler_options.no_implicit_this),
            use_unknown_in_catch_variables: compiler_options.get_strict_option_value(compiler_options.use_unknown_in_catch_variables),
            exact_optional_property_types: compiler_options.exact_optional_property_types == Tristate::True,
            can_collect_symbol_alias_accessibility_data: compiler_options.verbatim_module_syntax.is_false_or_unknown(),
            array_variances: &[VarianceFlags::Covariant],
            payloads: SharedPayloadStore::new(share_payloads),
        })
    }
}
