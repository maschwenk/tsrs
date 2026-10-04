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
    /// API only (`gojson`): `moduleDetection` and `newLine` numbers decoded from the wire with no Rust variant (Go
    /// keeps any int32), by JSON name. The typed field stays at its default (which behaves like Go's fallback) and
    /// the value is echoed back. (`target`, `module`, `jsx` and `moduleResolution` hold any int32 directly.)
    pub api_unknown_enum_values: Vec<(&'static str, i32)>,

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
    // Go `*int` (64-bit).
    pub max_node_module_js_depth: Option<i64>,

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
    // Go `*int` (64-bit).
    pub checkers: Option<i64>,
}

pub static EMPTY_COMPILER_OPTIONS: LazyLock<CompilerOptions> = LazyLock::new(CompilerOptions::default);

/// `EMPTY_COMPILER_OPTIONS` as an arena pointer (one per process; a compressed `P` cannot point to a static, so
/// compressed builds keep a copy in the arena of the thread that first asks).
pub fn empty_compiler_options() -> crate::P<CompilerOptions> {
    #[cfg(not(feature = "compressed-ptrs"))]
    return crate::P::from_static(&*EMPTY_COMPILER_OPTIONS);
    #[cfg(feature = "compressed-ptrs")]
    {
        static EMPTY: std::sync::OnceLock<crate::P<CompilerOptions>> = std::sync::OnceLock::new();
        *EMPTY.get_or_init(|| {
            let _scope = crate::arena::enter_thread_arena();
            crate::P::new(CompilerOptions::default())
        })
    }
}

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

impl ModuleDetectionKind {
    /// The Go int32 value.
    pub fn value(self) -> i32 {
        self as i32
    }
}

/// Go `type ModuleKind int32`: any int32 is representable (the API can receive values with no named constant, which
/// Go keeps and compiles with). Named values are associated constants, usable as patterns.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ModuleKind(pub i32);

#[allow(non_upper_case_globals)]
impl ModuleKind {
    pub const None: ModuleKind = ModuleKind(0);
    pub const CommonJS: ModuleKind = ModuleKind(1);
    pub const AMD: ModuleKind = ModuleKind(2);
    pub const UMD: ModuleKind = ModuleKind(3);
    pub const System: ModuleKind = ModuleKind(4);
    pub const ES2015: ModuleKind = ModuleKind(5);
    pub const ES2020: ModuleKind = ModuleKind(6);
    pub const ES2022: ModuleKind = ModuleKind(7);
    pub const ESNext: ModuleKind = ModuleKind(99);
    pub const Node16: ModuleKind = ModuleKind(100);
    pub const Node18: ModuleKind = ModuleKind(101);
    pub const Node20: ModuleKind = ModuleKind(102);
    pub const NodeNext: ModuleKind = ModuleKind(199);
    pub const Preserve: ModuleKind = ModuleKind(200);

    /// The Go constant name of a named value.
    pub fn name(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("None"),
            1 => Some("CommonJS"),
            2 => Some("AMD"),
            3 => Some("UMD"),
            4 => Some("System"),
            5 => Some("ES2015"),
            6 => Some("ES2020"),
            7 => Some("ES2022"),
            99 => Some("ESNext"),
            100 => Some("Node16"),
            101 => Some("Node18"),
            102 => Some("Node20"),
            199 => Some("NodeNext"),
            200 => Some("Preserve"),
            _ => None,
        }
    }

    /// Whether this is one of Go's named constants.
    pub fn is_named(self) -> bool {
        self.name().is_some()
    }

    /// The Go int32 value.
    pub fn value(self) -> i32 {
        self.0
    }
}

impl fmt::Debug for ModuleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "ModuleKind({})", self.0),
        }
    }
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

    /// Go stringer: the constant name, or `ModuleKind(<n>)`.
    pub fn string(self) -> std::borrow::Cow<'static, str> {
        match self.name() {
            Some(n) => std::borrow::Cow::Borrowed(n),
            None => std::borrow::Cow::Owned(format!("ModuleKind({})", self.0)),
        }
    }
}

impl fmt::Display for ModuleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

// ModuleKindNone | ModuleKindCommonJS | ModuleKindESNext
pub type ResolutionMode = ModuleKind;

pub const RESOLUTION_MODE_NONE: ResolutionMode = ModuleKind::None;
pub const RESOLUTION_MODE_COMMON_JS: ResolutionMode = ModuleKind::CommonJS;
pub const RESOLUTION_MODE_ESM: ResolutionMode = ModuleKind::ESNext;

/// Go `type ModuleResolutionKind int32`: any int32 is representable (the API can receive values with no named constant; Go keeps
/// them and panics only when a module is actually resolved, see `tsrs_module` resolver). Named values are
/// associated constants, usable as patterns.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ModuleResolutionKind(pub i32);

#[allow(non_upper_case_globals)]
impl ModuleResolutionKind {
    pub const Unknown: ModuleResolutionKind = ModuleResolutionKind(0);
    pub const Classic: ModuleResolutionKind = ModuleResolutionKind(1);
    pub const Node10: ModuleResolutionKind = ModuleResolutionKind(2);
    pub const Node16: ModuleResolutionKind = ModuleResolutionKind(3);
    pub const NodeNext: ModuleResolutionKind = ModuleResolutionKind(99);
    pub const Bundler: ModuleResolutionKind = ModuleResolutionKind(100);

    /// The Go constant name of a named value.
    pub fn name(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("Unknown"),
            1 => Some("Classic"),
            2 => Some("Node10"),
            3 => Some("Node16"),
            99 => Some("NodeNext"),
            100 => Some("Bundler"),
            _ => None,
        }
    }
}

impl fmt::Debug for ModuleResolutionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "ModuleResolutionKind({})", self.0),
        }
    }
}

impl ModuleResolutionKind {
    /// The Go int32 value.
    pub fn value(self) -> i32 {
        self.0
    }
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
            _ => panic!("unhandled case in ModuleResolutionKind.String"),
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

impl NewLineKind {
    /// The Go int32 value.
    pub fn value(self) -> i32 {
        self as i32
    }
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

/// Go `type ScriptTarget int32`: any int32 is representable (the API can receive values with no named constant, which
/// Go keeps and compiles with). Named values are associated constants, usable as patterns.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ScriptTarget(pub i32);

#[allow(non_upper_case_globals)]
impl ScriptTarget {
    pub const None: ScriptTarget = ScriptTarget(0);
    pub const ES5: ScriptTarget = ScriptTarget(1);
    pub const ES2015: ScriptTarget = ScriptTarget(2);
    pub const ES2016: ScriptTarget = ScriptTarget(3);
    pub const ES2017: ScriptTarget = ScriptTarget(4);
    pub const ES2018: ScriptTarget = ScriptTarget(5);
    pub const ES2019: ScriptTarget = ScriptTarget(6);
    pub const ES2020: ScriptTarget = ScriptTarget(7);
    pub const ES2021: ScriptTarget = ScriptTarget(8);
    pub const ES2022: ScriptTarget = ScriptTarget(9);
    pub const ES2023: ScriptTarget = ScriptTarget(10);
    pub const ES2024: ScriptTarget = ScriptTarget(11);
    pub const ES2025: ScriptTarget = ScriptTarget(12);
    pub const ES2026: ScriptTarget = ScriptTarget(13);
    pub const ESNext: ScriptTarget = ScriptTarget(99);
    pub const JSON: ScriptTarget = ScriptTarget(100);

    /// The Go constant name of a named value.
    pub fn name(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("None"),
            1 => Some("ES5"),
            2 => Some("ES2015"),
            3 => Some("ES2016"),
            4 => Some("ES2017"),
            5 => Some("ES2018"),
            6 => Some("ES2019"),
            7 => Some("ES2020"),
            8 => Some("ES2021"),
            9 => Some("ES2022"),
            10 => Some("ES2023"),
            11 => Some("ES2024"),
            12 => Some("ES2025"),
            13 => Some("ES2026"),
            99 => Some("ESNext"),
            100 => Some("JSON"),
            _ => None,
        }
    }

    /// Whether this is one of Go's named constants.
    pub fn is_named(self) -> bool {
        self.name().is_some()
    }

    /// The Go int32 value.
    pub fn value(self) -> i32 {
        self.0
    }
}

impl fmt::Debug for ScriptTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "ScriptTarget({})", self.0),
        }
    }
}

impl ScriptTarget {
    pub const Latest: ScriptTarget = ScriptTarget::ESNext;
    pub const LatestStandard: ScriptTarget = ScriptTarget::ES2026;

    /// Go stringer: the constant name, or `ScriptTarget(<n>)`.
    pub fn string(self) -> std::borrow::Cow<'static, str> {
        match self.name() {
            Some(n) => std::borrow::Cow::Borrowed(n),
            None => std::borrow::Cow::Owned(format!("ScriptTarget({})", self.0)),
        }
    }
}

impl fmt::Display for ScriptTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

/// Go `type JsxEmit int32`: any int32 is representable (the API can receive values with no named constant, which
/// Go keeps and compiles with). Named values are associated constants, usable as patterns.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct JsxEmit(pub i32);

#[allow(non_upper_case_globals)]
impl JsxEmit {
    pub const None: JsxEmit = JsxEmit(0);
    pub const Preserve: JsxEmit = JsxEmit(1);
    pub const React: JsxEmit = JsxEmit(2);
    pub const ReactNative: JsxEmit = JsxEmit(3);
    pub const ReactJSX: JsxEmit = JsxEmit(4);
    pub const ReactJSXDev: JsxEmit = JsxEmit(5);

    /// The Go constant name of a named value.
    pub fn name(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("None"),
            1 => Some("Preserve"),
            2 => Some("React"),
            3 => Some("ReactNative"),
            4 => Some("ReactJSX"),
            5 => Some("ReactJSXDev"),
            _ => None,
        }
    }

    /// Whether this is one of Go's named constants.
    pub fn is_named(self) -> bool {
        self.name().is_some()
    }

    /// The Go int32 value.
    pub fn value(self) -> i32 {
        self.0
    }
}

impl fmt::Debug for JsxEmit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "JsxEmit({})", self.0),
        }
    }
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
            _ => panic!("unhandled case in JsxEmit.String"),
        }
    }
}

impl fmt::Display for JsxEmit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.string())
    }
}
