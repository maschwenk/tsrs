use crate::collections::OrderedMap;
use crate::tspath;
use crate::Tristate;
use rustc_hash::FxHashMap;
use std::fmt;
use std::sync::LazyLock;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginImport {
    pub name: String,
}

// CompilerOptions contains the compiler options exposed by the API.
//
// Go `[]string` options are `Option<Vec<String>>` because the Go code distinguishes nil (unset) from empty.
#[derive(Clone, Debug, Default)]
pub struct CompilerOptions {

    pub allow_js: Tristate,
    pub allow_arbitrary_extensions: Tristate,
    pub allow_importing_ts_extensions: Tristate,
    pub allow_non_ts_extensions: Tristate,
    pub allow_umd_global_access: Tristate,
    pub allow_unreachable_code: Tristate,
    pub allow_unused_labels: Tristate,
    pub assume_changes_only_affect_direct_dependencies: Tristate,
    pub check_js: Tristate,
    pub custom_conditions: Option<Vec<String>>,
    pub composite: Tristate,
    pub emit_declaration_only: Tristate,
    pub emit_bom: Tristate,
    pub emit_decorator_metadata: Tristate,
    pub declaration: Tristate,
    pub declaration_dir: String,
    pub declaration_map: Tristate,
    pub deduplicate_packages: Tristate,
    pub disable_size_limit: Tristate,
    pub disable_source_of_project_reference_redirect: Tristate,
    pub disable_solution_searching: Tristate,
    pub disable_referenced_project_load: Tristate,
    pub erasable_syntax_only: Tristate,
    pub exact_optional_property_types: Tristate,
    pub experimental_decorators: Tristate,
    pub force_consistent_casing_in_file_names: Tristate,
    pub isolated_modules: Tristate,
    pub isolated_declarations: Tristate,
    pub ignore_config: Tristate,
    pub ignore_deprecations: String,
    pub import_helpers: Tristate,
    pub inline_source_map: Tristate,
    pub inline_sources: Tristate,
    pub init: Tristate,
    pub incremental: Tristate,
    pub jsx: JsxEmit,
    pub jsx_factory: String,
    pub jsx_fragment_factory: String,
    pub jsx_import_source: String,
    pub lib: Option<Vec<String>>,
    pub lib_replacement: Tristate,
    pub locale: String,
    pub map_root: String,
    pub module: ModuleKind,
    pub module_resolution: ModuleResolutionKind,
    pub module_suffixes: Option<Vec<String>>,
    pub module_detection: ModuleDetectionKind,
    pub new_line: NewLineKind,
    pub no_emit: Tristate,
    pub no_check: Tristate,
    pub no_error_truncation: Tristate,
    pub no_fallthrough_cases_in_switch: Tristate,
    pub no_implicit_any: Tristate,
    pub no_implicit_this: Tristate,
    pub no_implicit_returns: Tristate,
    pub no_emit_helpers: Tristate,
    pub no_lib: Tristate,
    pub no_property_access_from_index_signature: Tristate,
    pub no_unchecked_indexed_access: Tristate,
    pub no_emit_on_error: Tristate,
    pub no_unused_locals: Tristate,
    pub no_unused_parameters: Tristate,
    pub no_resolve: Tristate,
    pub no_implicit_override: Tristate,
    pub no_unchecked_side_effect_imports: Tristate,
    pub out_dir: String,
    pub paths: Option<OrderedMap<String, Vec<String>>>,
    // Plugins are parsed only so tools can report that native TypeScript does not support them.
    pub plugins: Option<Vec<PluginImport>>,
    pub preserve_const_enums: Tristate,
    pub preserve_symlinks: Tristate,
    pub project: String,
    pub resolve_json_module: Tristate,
    pub resolve_package_json_exports: Tristate,
    pub resolve_package_json_imports: Tristate,
    pub remove_comments: Tristate,
    pub rewrite_relative_import_extensions: Tristate,
    pub react_namespace: String,
    pub root_dir: String,
    pub root_dirs: Option<Vec<String>>,
    pub skip_lib_check: Tristate,
    pub stable_type_ordering: Tristate,
    pub strict: Tristate,
    pub strict_bind_call_apply: Tristate,
    pub strict_builtin_iterator_return: Tristate,
    pub strict_function_types: Tristate,
    pub strict_null_checks: Tristate,
    pub strict_property_initialization: Tristate,
    pub strip_internal: Tristate,
    pub skip_default_lib_check: Tristate,
    pub source_map: Tristate,
    pub source_root: String,
    pub suppress_output_path_check: Tristate,
    pub target: ScriptTarget,
    pub trace_resolution: Tristate,
    pub ts_build_info_file: String,
    pub type_roots: Option<Vec<String>>,
    pub types: Option<Vec<String>>,
    pub use_define_for_class_fields: Tristate,
    pub use_unknown_in_catch_variables: Tristate,
    pub verbatim_module_syntax: Tristate,
    pub max_node_module_js_depth: Option<i32>,

    // Deprecated: Do not use outside of options parsing and validation.
    pub allow_synthetic_default_imports: Tristate,
    // Deprecated: Do not use outside of options parsing and validation.
    pub always_strict: Tristate,
    // Deprecated: Do not use outside of options parsing and validation.
    pub base_url: String,
    // Deprecated: Do not use outside of options parsing and validation.
    pub downlevel_iteration: Tristate,
    // Deprecated: Do not use outside of options parsing and validation.
    pub es_module_interop: Tristate,
    // Deprecated: Do not use outside of options parsing and validation.
    pub out_file: String,

    // Internal fields
    pub config_file_path: String,
    pub no_dts_resolution: Tristate,
    pub paths_base_path: String,
    pub diagnostics: Tristate,
    pub extended_diagnostics: Tristate,
    pub generate_cpu_profile: String,
    pub generate_trace: String,
    pub list_emitted_files: Tristate,
    pub list_files: Tristate,
    pub explain_files: Tristate,
    pub list_files_only: Tristate,
    pub no_emit_for_js_files: Tristate,
    pub preserve_watch_output: Tristate,
    pub pretty: Tristate,
    pub version: Tristate,
    pub watch: Tristate,
    pub show_config: Tristate,
    pub build: Tristate,
    pub help: Tristate,
    pub all: Tristate,
    pub run_external_code: Tristate,

    pub pprof_dir: String,
    pub single_threaded: Tristate,
    pub quiet: Tristate,
    pub checkers: Option<i32>,
}

pub static EMPTY_COMPILER_OPTIONS: LazyLock<CompilerOptions> = LazyLock::new(CompilerOptions::default);

impl CompilerOptions {
    pub fn get_emit_script_target(&self) -> ScriptTarget {
        if self.target != ScriptTarget::None {
            return self.target;
        }
        ScriptTarget::LatestStandard
    }

    pub fn get_emit_module_kind(&self) -> ModuleKind {
        if self.module != ModuleKind::None {
            return self.module;
        }

        let target = self.get_emit_script_target();
        if target == ScriptTarget::ESNext {
            return ModuleKind::ESNext;
        }
        if target >= ScriptTarget::ES2022 {
            return ModuleKind::ES2022;
        }
        if target >= ScriptTarget::ES2020 {
            return ModuleKind::ES2020;
        }
        if target >= ScriptTarget::ES2015 {
            return ModuleKind::ES2015;
        }
        ModuleKind::CommonJS
    }

    pub fn get_module_resolution_kind(&self) -> ModuleResolutionKind {
        match self.module_resolution {
            ModuleResolutionKind::Unknown | ModuleResolutionKind::Classic | ModuleResolutionKind::Node10 => {
                match self.get_emit_module_kind() {
                    ModuleKind::Node16 | ModuleKind::Node18 | ModuleKind::Node20 => ModuleResolutionKind::Node16,
                    ModuleKind::NodeNext => ModuleResolutionKind::NodeNext,
                    _ => ModuleResolutionKind::Bundler,
                }
            }
            _ => self.module_resolution,
        }
    }

    pub fn get_emit_module_detection_kind(&self) -> ModuleDetectionKind {
        if self.module_detection != ModuleDetectionKind::None {
            return self.module_detection;
        }
        let module_kind = self.get_emit_module_kind();
        if ModuleKind::Node16 <= module_kind && module_kind <= ModuleKind::NodeNext {
            return ModuleDetectionKind::Force;
        }
        ModuleDetectionKind::Auto
    }

    pub fn get_resolve_package_json_exports(&self) -> bool {
        self.resolve_package_json_exports.is_true_or_unknown()
    }

    pub fn get_resolve_package_json_imports(&self) -> bool {
        self.resolve_package_json_imports.is_true_or_unknown()
    }

    pub fn get_allow_importing_ts_extensions(&self) -> bool {
        self.allow_importing_ts_extensions.is_true() || self.rewrite_relative_import_extensions.is_true()
    }

    pub fn allow_importing_ts_extensions_from(&self, file_name: &str) -> bool {
        self.get_allow_importing_ts_extensions() || tspath::is_declaration_file_name(file_name)
    }

    pub fn get_resolve_json_module(&self) -> bool {
        if self.resolve_json_module != Tristate::Unknown {
            return self.resolve_json_module == Tristate::True;
        }
        match self.get_emit_module_kind() {
            // TODO in 6.0: add Node16/Node18
            ModuleKind::Node20 | ModuleKind::NodeNext => return true,
            _ => {}
        }
        self.get_module_resolution_kind() == ModuleResolutionKind::Bundler
    }

    pub fn should_preserve_const_enums(&self) -> bool {
        self.preserve_const_enums == Tristate::True || self.get_isolated_modules()
    }

    pub fn get_allow_js(&self) -> bool {
        if self.allow_js != Tristate::Unknown {
            return self.allow_js == Tristate::True;
        }
        self.check_js == Tristate::True
    }

    pub fn get_jsx_transform_enabled(&self) -> bool {
        let jsx = self.jsx;
        jsx == JsxEmit::React || jsx == JsxEmit::ReactJSX || jsx == JsxEmit::ReactJSXDev
    }

    pub fn get_strict_option_value(&self, value: Tristate) -> bool {
        if value != Tristate::Unknown {
            return value == Tristate::True;
        }
        self.strict != Tristate::False
    }

    pub fn get_effective_type_roots(&self, current_directory: &str) -> (Vec<String>, bool) {
        if let Some(type_roots) = &self.type_roots {
            return (type_roots.clone(), true);
        }
        let base_dir = if !self.config_file_path.is_empty() {
            tspath::get_directory_path(&self.config_file_path)
        } else {
            if current_directory.is_empty() {
                // This was accounted for in the TS codebase, but only for third-party API usage
                // where the module resolution host does not provide a getCurrentDirectory().
                panic!("cannot get effective type roots without a config file path or current directory");
            }
            current_directory.to_string()
        };

        let mut type_roots = Vec::with_capacity(base_dir.matches('/').count());
        tspath::for_each_ancestor_directory(&base_dir, |dir| -> Option<()> {
            type_roots.push(tspath::combine_paths(dir, &["node_modules", "@types"]));
            None
        });
        (type_roots, false)
    }

    // UsesWildcardTypes returns true if this option's types array includes "*"
    pub fn uses_wildcard_types(&self) -> bool {
        self.types.as_ref().is_some_and(|types| types.iter().any(|t| t == "*"))
    }

    pub fn get_isolated_modules(&self) -> bool {
        self.isolated_modules == Tristate::True || self.verbatim_module_syntax == Tristate::True
    }

    pub fn is_incremental(&self) -> bool {
        self.incremental.is_true() || self.composite.is_true()
    }

    pub fn get_emit_standard_class_fields(&self) -> bool {
        self.use_define_for_class_fields != Tristate::False && self.get_emit_script_target() >= ScriptTarget::ES2022
    }

    pub fn get_use_define_for_class_fields(&self) -> bool {
        if self.use_define_for_class_fields == Tristate::Unknown {
            return self.get_emit_script_target() >= ScriptTarget::ES2022;
        }
        self.use_define_for_class_fields == Tristate::True
    }

    pub fn get_emit_declarations(&self) -> bool {
        self.declaration.is_true() || self.composite.is_true()
    }

    pub fn get_are_declaration_maps_enabled(&self) -> bool {
        self.declaration_map == Tristate::True && self.get_emit_declarations()
    }

    pub fn has_json_module_emit_enabled(&self) -> bool {
        !matches!(self.get_emit_module_kind(), ModuleKind::System | ModuleKind::UMD)
    }

    pub fn get_paths_base_path(&self, current_directory: &str) -> String {
        if self.paths.as_ref().map_or(0, |p| p.len()) == 0 {
            return String::new();
        }
        if !self.paths_base_path.is_empty() {
            return self.paths_base_path.clone();
        }
        current_directory.to_string()
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum ModuleDetectionKind {
    #[default]
    None = 0,
    Auto = 1,
    Legacy = 2,
    Force = 3,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum ModuleKind {
    #[default]
    None = 0,
    CommonJS = 1,
    // Deprecated: Do not use outside of options parsing and validation.
    AMD = 2,
    // Deprecated: Do not use outside of options parsing and validation.
    UMD = 3,
    // Deprecated: Do not use outside of options parsing and validation.
    System = 4,
    // NOTE: ES module kinds should be contiguous to more easily check whether a module kind is *any* ES module kind.
    //       Non-ES module kinds should not come between ES2015 (the earliest ES module kind) and ESNext (the last ES
    //       module kind).
    ES2015 = 5,
    ES2020 = 6,
    ES2022 = 7,
    ESNext = 99,
    // Node16+ is an amalgam of commonjs (albeit updated) and es2022+, and represents a distinct module system from es2020/esnext
    Node16 = 100,
    Node18 = 101,
    Node20 = 102,
    NodeNext = 199,
    // Emit as written
    Preserve = 200,
}

impl ModuleKind {
    // ResolutionMode constants (ResolutionMode is an alias of ModuleKind).
    pub const ESM: ModuleKind = ModuleKind::ESNext;

    pub fn is_non_node_esm(self) -> bool {
        self >= ModuleKind::ES2015 && self <= ModuleKind::ESNext
    }

    pub fn supports_import_attributes(self) -> bool {
        ModuleKind::Node18 <= self && self <= ModuleKind::NodeNext || self == ModuleKind::Preserve || self == ModuleKind::ESNext
    }

    pub fn string(self) -> &'static str {
        match self {
            ModuleKind::None => "None",
            ModuleKind::CommonJS => "CommonJS",
            ModuleKind::AMD => "AMD",
            ModuleKind::UMD => "UMD",
            ModuleKind::System => "System",
            ModuleKind::ES2015 => "ES2015",
            ModuleKind::ES2020 => "ES2020",
            ModuleKind::ES2022 => "ES2022",
            ModuleKind::ESNext => "ESNext",
            ModuleKind::Node16 => "Node16",
            ModuleKind::Node18 => "Node18",
            ModuleKind::Node20 => "Node20",
            ModuleKind::NodeNext => "NodeNext",
            ModuleKind::Preserve => "Preserve",
        }
    }
}

impl fmt::Display for ModuleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.string())
    }
}

// ModuleKindNone | ModuleKindCommonJS | ModuleKindESNext
pub type ResolutionMode = ModuleKind;

pub const RESOLUTION_MODE_NONE: ResolutionMode = ModuleKind::None;
pub const RESOLUTION_MODE_COMMON_JS: ResolutionMode = ModuleKind::CommonJS;
pub const RESOLUTION_MODE_ESM: ResolutionMode = ModuleKind::ESNext;

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum ModuleResolutionKind {
    #[default]
    Unknown = 0,
    // Deprecated: Do not use outside of options parsing and validation.
    Classic = 1,
    // Deprecated: Do not use outside of options parsing and validation.
    Node10 = 2,
    // Starting with node16, node's module resolver has significant departures from traditional cjs resolution
    // to better support ECMAScript modules and their use within node - however more features are still being added.
    // TypeScript's Node ESM support was introduced after Node 12 went end-of-life, and Node 14 is the earliest stable
    // version that supports both pattern trailers - *but*, Node 16 is the first version that also supports ECMAScript 2022.
    // In turn, we offer both a `NodeNext` moving resolution target, and a `Node16` version-anchored resolution target
    Node16 = 3,
    NodeNext = 99, // Not simply `Node16` so that compiled code linked against TS can use the `Next` value reliably (same as with `ModuleKind`)
    Bundler = 100,
}

pub static MODULE_KIND_TO_MODULE_RESOLUTION_KIND: LazyLock<FxHashMap<ModuleKind, ModuleResolutionKind>> = LazyLock::new(|| {
    let mut m = FxHashMap::default();
    m.insert(ModuleKind::Node16, ModuleResolutionKind::Node16);
    m.insert(ModuleKind::NodeNext, ModuleResolutionKind::NodeNext);
    m
});

impl ModuleResolutionKind {
    // We don't use stringer on this for now, because these values
    // are user-facing in --traceResolution, and stringer currently
    // lacks the ability to remove the "ModuleResolutionKind" prefix
    // when generating code for multiple types into the same output
    // file. Additionally, since there's no TS equivalent of
    // `ModuleResolutionKindUnknown`, we want to panic on that case,
    // as it probably represents a mistake when porting TS to Go.
    pub fn string(self) -> &'static str {
        match self {
            ModuleResolutionKind::Unknown => panic!("should not use zero value of ModuleResolutionKind"),
            ModuleResolutionKind::Classic => "Classic",
            ModuleResolutionKind::Node10 => "Node10",
            ModuleResolutionKind::Node16 => "Node16",
            ModuleResolutionKind::NodeNext => "NodeNext",
            ModuleResolutionKind::Bundler => "Bundler",
        }
    }
}

impl fmt::Display for ModuleResolutionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.string())
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum NewLineKind {
    #[default]
    None = 0,
    CRLF = 1,
    LF = 2,
}

pub fn get_new_line_kind(s: &str) -> NewLineKind {
    match s {
        "\r\n" => NewLineKind::CRLF,
        "\n" => NewLineKind::LF,
        _ => NewLineKind::None,
    }
}

impl NewLineKind {
    pub fn get_new_line_character(self) -> &'static str {
        match self {
            NewLineKind::CRLF => "\r\n",
            _ => "\n",
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum ScriptTarget {
    #[default]
    None = 0,
    // Deprecated: Do not use outside of options parsing and validation.
    ES5 = 1,
    ES2015 = 2,
    ES2016 = 3,
    ES2017 = 4,
    ES2018 = 5,
    ES2019 = 6,
    ES2020 = 7,
    ES2021 = 8,
    ES2022 = 9,
    ES2023 = 10,
    ES2024 = 11,
    ES2025 = 12,
    ES2026 = 13,
    ESNext = 99,
    JSON = 100,
}

impl ScriptTarget {
    pub const Latest: ScriptTarget = ScriptTarget::ESNext;
    pub const LatestStandard: ScriptTarget = ScriptTarget::ES2026;

    pub fn string(self) -> &'static str {
        match self {
            ScriptTarget::None => "None",
            ScriptTarget::ES5 => "ES5",
            ScriptTarget::ES2015 => "ES2015",
            ScriptTarget::ES2016 => "ES2016",
            ScriptTarget::ES2017 => "ES2017",
            ScriptTarget::ES2018 => "ES2018",
            ScriptTarget::ES2019 => "ES2019",
            ScriptTarget::ES2020 => "ES2020",
            ScriptTarget::ES2021 => "ES2021",
            ScriptTarget::ES2022 => "ES2022",
            ScriptTarget::ES2023 => "ES2023",
            ScriptTarget::ES2024 => "ES2024",
            ScriptTarget::ES2025 => "ES2025",
            ScriptTarget::ES2026 => "ES2026",
            ScriptTarget::ESNext => "ESNext",
            ScriptTarget::JSON => "JSON",
        }
    }
}

impl fmt::Display for ScriptTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.string())
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum JsxEmit {
    #[default]
    None = 0,
    Preserve = 1,
    React = 2,
    ReactNative = 3,
    ReactJSX = 4,
    ReactJSXDev = 5,
}

impl JsxEmit {
    pub fn string(self) -> &'static str {
        match self {
            JsxEmit::None => panic!("should not use zero value of JsxEmit"),
            JsxEmit::Preserve => "preserve",
            JsxEmit::ReactNative => "react-native",
            JsxEmit::React => "react",
            JsxEmit::ReactJSX => "react-jsx",
            JsxEmit::ReactJSXDev => "react-jsxdev",
        }
    }
}

impl fmt::Display for JsxEmit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.string())
    }
}
