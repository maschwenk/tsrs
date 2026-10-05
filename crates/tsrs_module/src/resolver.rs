use std::fmt;

use tsrs_ast::Diagnostic;
use tsrs_core::collections::{new_set_with_size_hint, OrderedMap, Set};
use tsrs_core::tspath::{self, ComparePathsOptions};
use tsrs_core::{CompilerOptions, ModuleKind, ModuleResolutionKind, Pattern, ResolutionMode, TextRange, Tristate, P};
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;

use crate::cache::{
    get_redirect_config_name, new_resolution_data, ModuleResolutionCache, ModuleResolutionCacheKey, ParsedPatternsCache, ResolutionData,
    TypeRefDirectiveResolutionCache, TypeRefDirectiveResolutionCacheKey,
};
use crate::packagejson::{self, with_package_directory, InfoCache, InfoCacheEntry, InfoCacheEntryExt, JSONValueType, PackageJson, TypeValidatedField, VersionPaths};
use crate::types::{
    Extensions, NodeResolutionFeatures, PackageId, ResolutionHost, ResolvedModule, ResolvedProjectReference, ResolvedTypeReferenceDirective, Resolver,
};
use crate::util::{is_applicable_versioned_types_key, mangle_scoped_package_name, parse_node_module_from_path, parse_package_name, compare_pattern_keys, INFERRED_TYPES_CONTAINING_FILE};

#[derive(Clone, Debug, Default)]
pub(crate) struct Resolved {
    path: String,
    extension: String,
    package_id: PackageId,
    original_path: String,
    resolved_using_ts_extension: bool,
    resolved_using_extra_extensions: bool,
}

// Go's `*resolved`: nil (`continueSearching()`) is `None`, `&resolved{}` (`unresolved()`) is a
// `Some` with an empty path.
trait ResolvedExt {
    fn should_continue_searching(&self) -> bool;
    fn is_resolved(&self) -> bool;
}

impl ResolvedExt for Option<Resolved> {
    fn should_continue_searching(&self) -> bool {
        self.is_none()
    }

    fn is_resolved(&self) -> bool {
        matches!(self, Some(r) if !r.path.is_empty())
    }
}

fn continue_searching() -> Option<Resolved> {
    None
}

fn unresolved() -> Option<Resolved> {
    Some(Resolved::default())
}

type ResolutionKindSpecificLoader<'l, 'r> = &'l mut dyn FnMut(&mut ResolutionState<'r>, Extensions, &str) -> Option<Resolved>;

#[derive(Default)]
pub(crate) struct Tracer {
    traces: Vec<DiagAndArgs>,
}

// Go's `Args []any` are formatted to strings when the trace is recorded.
#[derive(Clone, Debug)]
pub struct DiagAndArgs {
    pub message: &'static Message,
    pub args: Vec<String>,
}

impl Tracer {
    fn write(&mut self, diag: &'static Message, args: &[&dyn fmt::Display]) {
        self.traces.push(DiagAndArgs { message: diag, args: args.iter().map(|a| a.to_string()).collect() });
    }

    fn write_strings(&mut self, diag: &'static Message, args: &[String]) {
        self.traces.push(DiagAndArgs { message: diag, args: args.to_vec() });
    }
}

fn get_traces(t: Option<Tracer>) -> Vec<DiagAndArgs> {
    match t {
        Some(t) => t.traces,
        None => Vec::new(),
    }
}

// `if r.tracer != nil { r.tracer.write(msg, args...) }`; arguments are only evaluated when tracing.
macro_rules! trace {
    ($tracer:expr, $msg:expr $(, $arg:expr)* $(,)?) => {
        if let Some(t) = ($tracer).as_mut() {
            t.write(&$msg, &[$(&$arg as &dyn ::std::fmt::Display),*]);
        }
    };
}

pub(crate) struct ResolutionState<'r> {
    resolver: &'r DefaultResolver,
    tracer: Option<Tracer>,

    // request fields
    name: String,
    containing_directory: String,
    is_config_lookup: bool,
    features: NodeResolutionFeatures,
    esm_mode: bool,
    conditions: Vec<String>,
    extensions: Extensions,
    compiler_options: P<CompilerOptions>,
    resolve_package_directory_only: bool,

    // state fields
    // candidateEndingIsFromConfig is set when the candidate file extension originated from
    // configuration (package.json fields, tsconfig.json paths entries, or wildcard substitutions)
    // rather than from the module specifier written in source code. When true, resolvedUsingTsExtension
    // is suppressed so the checker does not attempt to extract a TS extension from the original specifier.
    candidate_ending_is_from_config: bool,
    resolved_package_directory: bool,
    diagnostics: Vec<P<Diagnostic>>,
}

fn new_resolution_state<'r>(
    name: &str,
    containing_directory: &str,
    is_type_reference_directive: bool,
    resolution_mode: ResolutionMode,
    compiler_options: P<CompilerOptions>,
    redirected_reference: Option<&dyn ResolvedProjectReference>,
    resolver: &'r DefaultResolver,
    trace_builder: Option<Tracer>,
) -> ResolutionState<'r> {
    let mut state = ResolutionState {
        name: name.to_string(),
        containing_directory: containing_directory.to_string(),
        compiler_options: get_compiler_options_with_redirect(compiler_options, redirected_reference),
        resolver,
        tracer: trace_builder,
        is_config_lookup: false,
        features: NodeResolutionFeatures::None,
        esm_mode: false,
        conditions: Vec::new(),
        extensions: Extensions::empty(),
        resolve_package_directory_only: false,
        candidate_ending_is_from_config: false,
        resolved_package_directory: false,
        diagnostics: Vec::new(),
    };

    if is_type_reference_directive {
        state.extensions = Extensions::Declaration;
    } else if compiler_options.no_dts_resolution == Tristate::True {
        state.extensions = Extensions::ImplementationFiles;
    } else {
        state.extensions = Extensions::TypeScript | Extensions::JavaScript | Extensions::Declaration;
    }

    if !is_type_reference_directive && compiler_options.get_resolve_json_module() {
        state.extensions |= Extensions::Json;
    }

    match compiler_options.get_module_resolution_kind() {
        ModuleResolutionKind::Node16 => {
            state.features = NodeResolutionFeatures::Node16Default;
            state.esm_mode = resolution_mode == ModuleKind::ESNext;
            state.conditions = get_conditions(&compiler_options, resolution_mode);
        }
        ModuleResolutionKind::NodeNext => {
            state.features = NodeResolutionFeatures::NodeNextDefault;
            state.esm_mode = resolution_mode == ModuleKind::ESNext;
            state.conditions = get_conditions(&compiler_options, resolution_mode);
        }
        ModuleResolutionKind::Bundler => {
            state.features = get_node_resolution_features(&compiler_options);
            state.conditions = get_conditions(&compiler_options, resolution_mode);
        }
        _ => {}
    }
    state
}

pub fn get_compiler_options_with_redirect(compiler_options: P<CompilerOptions>, redirected_reference: Option<&dyn ResolvedProjectReference>) -> P<CompilerOptions> {
    let Some(redirected_reference) = redirected_reference else {
        return compiler_options;
    };
    if let Some(options_from_redirect) = redirected_reference.compiler_options() {
        return options_from_redirect;
    }
    compiler_options
}

pub struct DefaultResolver {
    pub(crate) resolution_data: P<ResolutionData>,
    host: &'static dyn ResolutionHost,
    // reportDiagnostic: DiagnosticReporter
    pub(crate) module_resolution_cache: ModuleResolutionCache,
    pub(crate) type_ref_directive_resolution_cache: TypeRefDirectiveResolutionCache,

    // Cached representations for `core.CompilerOptions.paths`, keyed by the
    // path mappings themselves. This does not handle other path patterns such
    // as `typesVersions`.
    pub(crate) parsed_patterns_for_paths: ParsedPatternsCache,
}

pub struct ResolverOptions {
    pub host: &'static dyn ResolutionHost,
    pub compiler_options: P<CompilerOptions>,
    pub typings_location: String,
    pub project_name: String,
    pub extra_extensions: Vec<String>,
    pub package_json_cache: Option<P<InfoCache>>,
}

impl ResolverOptions {
    pub fn new(host: &'static dyn ResolutionHost, compiler_options: P<CompilerOptions>) -> ResolverOptions {
        ResolverOptions {
            host,
            compiler_options,
            typings_location: String::new(),
            project_name: String::new(),
            extra_extensions: Vec::new(),
            package_json_cache: None,
        }
    }
}

pub fn new_resolver(opts: ResolverOptions) -> DefaultResolver {
    let host = opts.host;
    DefaultResolver::new_from_resolution_data(new_resolution_data(opts), host)
}

impl DefaultResolver {
    // Go's `(*ResolutionData).NewResolver`.
    pub fn new_from_resolution_data(d: P<ResolutionData>, host: &'static dyn ResolutionHost) -> DefaultResolver {
        DefaultResolver {
            resolution_data: d,
            host,
            module_resolution_cache: ModuleResolutionCache::default(),
            type_ref_directive_resolution_cache: TypeRefDirectiveResolutionCache::default(),
            parsed_patterns_for_paths: ParsedPatternsCache::default(),
        }
    }

    pub fn get_resolution_data(&self) -> P<ResolutionData> {
        self.resolution_data
    }

    fn new_trace_builder(&self) -> Option<Tracer> {
        if self.compiler_options.trace_resolution == Tristate::True {
            return Some(Tracer::default());
        }
        None
    }

    pub fn get_package_scope_for_path(&self, directory: &str) -> Option<P<InfoCacheEntry>> {
        ResolutionState::bare(self, self.compiler_options).get_package_scope_for_path(directory)
    }
}

impl Tracer {
    fn trace_resolution_using_project_reference(&mut self, redirected_reference: Option<&dyn ResolvedProjectReference>) {
        if let Some(redirected_reference) = redirected_reference {
            if redirected_reference.compiler_options().is_some() {
                self.write(&diagnostics::Using_compiler_options_of_project_reference_redirect_0, &[&redirected_reference.config_name()]);
            }
        }
    }
}

impl DefaultResolver {
    pub fn resolve_type_reference_directive(
        &self,
        type_reference_directive_name: &str,
        containing_file: &str,
        resolution_mode: ResolutionMode,
        redirected_reference: Option<&dyn ResolvedProjectReference>,
    ) -> (P<ResolvedTypeReferenceDirective>, Vec<DiagAndArgs>) {
        let containing_directory = tspath::get_directory_path(containing_file);
        let mut trace_builder = self.new_trace_builder();

        let from_inferred_types_containing_file = containing_file.ends_with(INFERRED_TYPES_CONTAINING_FILE);

        let cache_key = TypeRefDirectiveResolutionCacheKey {
            containing_directory: containing_directory.clone(),
            type_reference_name: type_reference_directive_name.to_string(),
            resolution_mode,
            redirect_config_name: get_redirect_config_name(redirected_reference),
            from_inferred_types_containing_file,
        };

        if trace_builder.is_none() {
            if let Some(cached) = self.type_ref_directive_resolution_cache.get(&cache_key) {
                return (cached, Vec::new());
            }
        }

        let compiler_options = get_compiler_options_with_redirect(self.compiler_options, redirected_reference);

        let (type_roots, from_config) = compiler_options.get_effective_type_roots(self.host.get_current_directory());
        if let Some(t) = trace_builder.as_mut() {
            t.write(
                &diagnostics::Resolving_type_reference_directive_0_containing_file_1_root_directory_2,
                &[&type_reference_directive_name, &containing_file, &type_roots.join(",")],
            );
            t.trace_resolution_using_project_reference(redirected_reference);
        }

        let mut state = new_resolution_state(
            type_reference_directive_name,
            &containing_directory,
            true, /*isTypeReferenceDirective*/
            resolution_mode,
            compiler_options,
            redirected_reference,
            self,
            trace_builder,
        );
        let result = P::new(state.resolve_type_reference_directive(&type_roots, from_config, from_inferred_types_containing_file));
        let mut trace_builder = state.tracer.take();

        if let Some(t) = trace_builder.as_mut() {
            t.trace_type_reference_directive_result(type_reference_directive_name, &result);
        }

        self.type_ref_directive_resolution_cache.set(cache_key, result);

        (result, get_traces(trace_builder))
    }

    pub fn resolve_module_name(
        &self,
        module_name: &str,
        containing_file: &str,
        resolution_mode: ResolutionMode,
        redirected_reference: Option<&dyn ResolvedProjectReference>,
    ) -> Result<(P<ResolvedModule>, Vec<DiagAndArgs>), String> {
        let (result, trace) =
            self.resolve_module_name_worker(module_name, containing_file, &tspath::get_directory_path(containing_file), resolution_mode, redirected_reference);
        Ok((result, trace))
    }

    pub fn resolve_module_name_from_directory(
        &self,
        module_name: &str,
        containing_directory: &str,
        resolution_mode: ResolutionMode,
    ) -> Result<(P<ResolvedModule>, Vec<DiagAndArgs>), String> {
        let (result, trace) = self.resolve_module_name_worker(module_name, containing_directory, containing_directory, resolution_mode, None);
        Ok((result, trace))
    }

    // Go's unexported `resolveModuleName` (renamed to avoid clashing with the exported method).
    fn resolve_module_name_worker(
        &self,
        module_name: &str,
        containing_file: &str,
        containing_directory: &str,
        resolution_mode: ResolutionMode,
        redirected_reference: Option<&dyn ResolvedProjectReference>,
    ) -> (P<ResolvedModule>, Vec<DiagAndArgs>) {
        let mut trace_builder = self.new_trace_builder();

        let cache_key = ModuleResolutionCacheKey {
            containing_directory: containing_directory.to_string(),
            module_name: module_name.to_string(),
            resolution_mode,
            redirect_config_name: get_redirect_config_name(redirected_reference),
        };

        if trace_builder.is_none() {
            if let Some(cached) = self.module_resolution_cache.get(&cache_key) {
                return (cached, Vec::new());
            }
        }

        let compiler_options = get_compiler_options_with_redirect(self.compiler_options, redirected_reference);
        if let Some(t) = trace_builder.as_mut() {
            t.write(&diagnostics::Resolving_module_0_from_1, &[&module_name, &containing_file]);
            t.trace_resolution_using_project_reference(redirected_reference);
        }

        let module_resolution = compiler_options.get_module_resolution_kind();
        if compiler_options.module_resolution != module_resolution {
            trace!(trace_builder, diagnostics::Module_resolution_kind_is_not_specified_using_0, module_resolution.string());
        } else {
            trace!(trace_builder, diagnostics::Explicitly_specified_module_resolution_kind_Colon_0, module_resolution.string());
        }

        let result: ResolvedModule;
        match module_resolution {
            ModuleResolutionKind::Node16 | ModuleResolutionKind::NodeNext | ModuleResolutionKind::Bundler => {
                let mut state = new_resolution_state(
                    module_name,
                    containing_directory,
                    false, /*isTypeReferenceDirective*/
                    resolution_mode,
                    compiler_options,
                    redirected_reference,
                    self,
                    trace_builder,
                );
                result = state.resolve_node_like();
                trace_builder = state.tracer.take();
            }
            _ => panic!("Unexpected moduleResolution: {}", module_resolution.value()),
        }

        if let Some(t) = trace_builder.as_mut() {
            if result.is_resolved() {
                if !result.package_id.name.is_empty() {
                    t.write(
                        &diagnostics::Module_name_0_was_successfully_resolved_to_1_with_Package_ID_2,
                        &[&module_name, &result.resolved_file_name, &result.package_id],
                    );
                } else {
                    t.write(&diagnostics::Module_name_0_was_successfully_resolved_to_1, &[&module_name, &result.resolved_file_name]);
                }
            } else {
                t.write(&diagnostics::Module_name_0_was_not_resolved, &[&module_name]);
            }
        }

        let final_result = P::new(self.try_resolve_from_typings_location(module_name, containing_directory, result, &mut trace_builder));
        self.module_resolution_cache.set(cache_key, final_result);

        (final_result, get_traces(trace_builder))
    }

    pub fn resolve_package_directory(
        &self,
        module_name: &str,
        containing_file: &str,
        resolution_mode: ResolutionMode,
        redirected_reference: Option<&dyn ResolvedProjectReference>,
    ) -> Option<P<ResolvedModule>> {
        let compiler_options = get_compiler_options_with_redirect(self.compiler_options, redirected_reference);
        let containing_directory = tspath::get_directory_path(containing_file);
        let mut state = new_resolution_state(
            module_name,
            &containing_directory,
            false, /*isTypeReferenceDirective*/
            resolution_mode,
            compiler_options,
            redirected_reference,
            self,
            None,
        );
        state.resolve_package_directory_only = true;
        let result = state.load_module_from_nearest_node_modules_directory(false /*typesScopeOnly*/);
        if result.as_ref().is_some_and(|r| !r.path.is_empty()) {
            return Some(P::new(state.create_resolved_module_handling_symlink(result)));
        }
        None
    }

    fn try_resolve_from_typings_location(
        &self,
        module_name: &str,
        containing_directory: &str,
        original_result: ResolvedModule,
        trace_builder: &mut Option<Tracer>,
    ) -> ResolvedModule {
        if self.typings_location.is_empty()
            || tspath::is_external_module_name_relative(module_name)
            || (!original_result.resolved_file_name.is_empty()
                && tspath::extension_is_one_of(original_result.extension, tspath::SUPPORTED_TS_EXTENSIONS_WITH_JSON_FLAT))
        {
            return original_result;
        }

        let mut state = new_resolution_state(
            module_name,
            containing_directory,
            false,            /*isTypeReferenceDirective*/
            ModuleKind::None, // resolutionMode,
            self.compiler_options,
            None, // redirectedReference,
            self,
            trace_builder.take(),
        );
        trace!(
            state.tracer,
            diagnostics::Auto_discovery_for_typings_is_enabled_in_project_0_Running_extra_resolution_pass_for_module_1_using_cache_location_2,
            self.project_name,
            module_name,
            self.typings_location
        );
        let global_resolved = state.load_module_from_immediate_node_modules_directory(Extensions::Declaration, &self.typings_location, false);
        if global_resolved.is_none() {
            *trace_builder = state.tracer.take();
            return original_result;
        }
        let mut result = state.create_resolved_module(global_resolved, true);
        *trace_builder = state.tracer.take();
        let mut diags = original_result.resolution_diagnostics.to_vec();
        diags.extend_from_slice(result.resolution_diagnostics);
        result.resolution_diagnostics = tsrs_core::alloc_vec(diags);
        result
    }

    fn resolve_config(&self, module_name: &str, containing_file: &str) -> ResolvedModule {
        let containing_directory = tspath::get_directory_path(containing_file);
        let mut state = new_resolution_state(
            module_name,
            &containing_directory,
            false, /*isTypeReferenceDirective*/
            ModuleKind::CommonJS,
            self.compiler_options,
            None,
            self,
            None,
        );
        state.is_config_lookup = true;
        state.extensions = Extensions::Json;
        state.resolve_node_like()
    }
}

impl Resolver for DefaultResolver {
    fn resolve_module_name(
        &self,
        module_name: &str,
        containing_file: &str,
        resolution_mode: ResolutionMode,
        redirected_reference: Option<&dyn ResolvedProjectReference>,
    ) -> Result<(P<ResolvedModule>, Vec<DiagAndArgs>), String> {
        DefaultResolver::resolve_module_name(self, module_name, containing_file, resolution_mode, redirected_reference)
    }

    fn resolve_module_name_from_directory(
        &self,
        module_name: &str,
        containing_directory: &str,
        resolution_mode: ResolutionMode,
    ) -> Result<(P<ResolvedModule>, Vec<DiagAndArgs>), String> {
        DefaultResolver::resolve_module_name_from_directory(self, module_name, containing_directory, resolution_mode)
    }

    fn resolve_type_reference_directive(
        &self,
        type_reference_directive_name: &str,
        containing_file: &str,
        resolution_mode: ResolutionMode,
        redirected_reference: Option<&dyn ResolvedProjectReference>,
    ) -> (P<ResolvedTypeReferenceDirective>, Vec<DiagAndArgs>) {
        DefaultResolver::resolve_type_reference_directive(self, type_reference_directive_name, containing_file, resolution_mode, redirected_reference)
    }

    fn get_resolution_data(&self) -> P<ResolutionData> {
        self.resolution_data
    }
}

impl Tracer {
    fn trace_type_reference_directive_result(&mut self, type_reference_directive_name: &str, result: &ResolvedTypeReferenceDirective) {
        if !result.is_resolved() {
            self.write(&diagnostics::Type_reference_directive_0_was_not_resolved, &[&type_reference_directive_name]);
        } else if !result.package_id.name.is_empty() {
            self.write(
                &diagnostics::Type_reference_directive_0_was_successfully_resolved_to_1_with_Package_ID_2_primary_Colon_3,
                &[&type_reference_directive_name, &result.resolved_file_name, &result.package_id, &result.primary],
            );
        } else {
            self.write(
                &diagnostics::Type_reference_directive_0_was_successfully_resolved_to_1_primary_Colon_2,
                &[&type_reference_directive_name, &result.resolved_file_name, &result.primary],
            );
        }
    }
}

impl<'r> ResolutionState<'r> {
    // Go's `&resolutionState{compilerOptions: ..., resolver: r}` literals.
    fn bare(resolver: &'r DefaultResolver, compiler_options: P<CompilerOptions>) -> ResolutionState<'r> {
        ResolutionState {
            resolver,
            tracer: None,
            name: String::new(),
            containing_directory: String::new(),
            is_config_lookup: false,
            features: NodeResolutionFeatures::None,
            esm_mode: false,
            conditions: Vec::new(),
            extensions: Extensions::empty(),
            compiler_options,
            resolve_package_directory_only: false,
            candidate_ending_is_from_config: false,
            resolved_package_directory: false,
            diagnostics: Vec::new(),
        }
    }

    fn fs(&self) -> &'r dyn tsrs_vfs::FS {
        self.resolver.host.fs()
    }

    fn current_directory(&self) -> &'r str {
        self.resolver.host.get_current_directory()
    }

    fn resolve_type_reference_directive(&mut self, type_roots: &[String], from_config: bool, from_inferred_types_containing_file: bool) -> ResolvedTypeReferenceDirective {
        // Primary lookup
        if !type_roots.is_empty() {
            trace!(self.tracer, diagnostics::Resolving_with_primary_search_path_0, type_roots.join(", "));
            for type_root in type_roots {
                let candidate = self.get_candidate_from_type_root(type_root);
                let directory_exists = self.fs().directory_exists(type_root);
                if !directory_exists {
                    trace!(self.tracer, diagnostics::Directory_0_does_not_exist_skipping_all_lookups_in_it, type_root);
                    continue;
                }
                if from_config {
                    // Custom typeRoots resolve as file or directory just like we do modules
                    let mut resolved_from_file = self.load_module_from_file(Extensions::Declaration, &candidate);
                    if let Some(resolved) = resolved_from_file.as_mut() {
                        let package_directory = parse_node_module_from_path(&resolved.path, false);
                        if !package_directory.is_empty() {
                            let info = self.get_package_json_info(&package_directory);
                            resolved.package_id = self.get_package_id(&resolved.path, info);
                        }
                        return self.create_resolved_type_reference_directive(resolved_from_file, true /*primary*/);
                    }
                }
                let resolved_from_directory = self.load_node_module_from_directory(Extensions::Declaration, &candidate, true /*considerPackageJson*/);
                if !resolved_from_directory.should_continue_searching() {
                    return self.create_resolved_type_reference_directive(resolved_from_directory, true /*primary*/);
                }
            }
        } else {
            trace!(self.tracer, diagnostics::Root_directory_cannot_be_determined_skipping_primary_search_paths);
        }

        // Secondary lookup
        let mut resolved = None;
        if !from_config || !from_inferred_types_containing_file {
            trace!(self.tracer, diagnostics::Looking_up_in_node_modules_folder_initial_location_0, self.containing_directory);
            if !tspath::is_external_module_name_relative(&self.name) {
                resolved = self.load_module_from_nearest_node_modules_directory(false /*typesScopeOnly*/);
            } else {
                let candidate = normalize_path_for_cjs_resolution(&self.containing_directory, &self.name);
                resolved = self.node_load_module_by_relative_name(Extensions::Declaration, &candidate, true /*considerPackageJson*/);
            }
        } else {
            trace!(self.tracer, diagnostics::Resolving_type_reference_directive_for_program_that_specifies_custom_typeRoots_skipping_lookup_in_node_modules_folder);
        }
        self.create_resolved_type_reference_directive(resolved, false /*primary*/)
    }

    fn get_candidate_from_type_root(&mut self, type_root: &str) -> String {
        let mut name_for_lookup = self.name.clone();
        if type_root.ends_with("/node_modules/@types") || type_root.ends_with("/node_modules/@types/") {
            let name = self.name.clone();
            name_for_lookup = self.mangle_scoped_package_name(&name);
        }
        tspath::combine_paths(type_root, &[&name_for_lookup])
    }

    fn mangle_scoped_package_name(&mut self, name: &str) -> String {
        let mangled = mangle_scoped_package_name(name);
        if mangled != name {
            trace!(self.tracer, diagnostics::Scoped_package_detected_looking_in_0, mangled);
        }
        mangled
    }

    // resolveFromTypeRoot tries to resolve a module name from the configured typeRoots.
    // This is used as a fallback after node_modules resolution fails, for declaration file lookups.
    // Returns nil if typeRoots is not configured or if no matching module is found in any typeRoot directory.
    fn resolve_from_type_root(&mut self) -> Option<Resolved> {
        let compiler_options = self.compiler_options.get();
        let Some(type_roots) = compiler_options.type_roots.as_ref() else {
            return None;
        };
        for type_root in type_roots {
            let candidate = self.get_candidate_from_type_root(type_root);
            let directory_exists = self.fs().directory_exists(type_root);
            if !directory_exists {
                trace!(self.tracer, diagnostics::Directory_0_does_not_exist_skipping_all_lookups_in_it, type_root);
                continue;
            }
            let mut resolved_from_file = self.load_module_from_file(Extensions::Declaration, &candidate);
            if let Some(resolved) = resolved_from_file.as_mut() {
                let package_directory = parse_node_module_from_path(&resolved.path, false);
                if !package_directory.is_empty() {
                    let info = self.get_package_json_info(&package_directory);
                    resolved.package_id = self.get_package_id(&resolved.path, info);
                }
                return resolved_from_file;
            }
            let resolved = self.load_node_module_from_directory(Extensions::Declaration, &candidate, true /*considerPackageJson*/);
            if !resolved.should_continue_searching() {
                return resolved;
            }
        }
        None
    }

    fn get_package_scope_for_path(&mut self, directory: &str) -> Option<P<InfoCacheEntry>> {
        let typings_location = self.resolver.typings_location.clone();
        tspath::for_each_ancestor_directory_stopping_at_global_cache(&typings_location, directory, |directory| {
            if let Some(result) = self.get_package_json_info(directory) {
                return Some(Some(result));
            }
            None
        })
    }

    fn resolve_node_like(&mut self) -> ResolvedModule {
        if let Some(t) = self.tracer.as_mut() {
            let conditions = self.conditions.iter().map(|c| format!("'{}'", c)).collect::<Vec<_>>().join(", ");
            if self.esm_mode {
                t.write(&diagnostics::Resolving_in_0_mode_with_conditions_1, &[&"ESM", &conditions]);
            } else {
                t.write(&diagnostics::Resolving_in_0_mode_with_conditions_1, &[&"CJS", &conditions]);
            }
        }
        let mut result = self.resolve_node_like_worker();
        if self.resolved_package_directory
            && !self.is_config_lookup
            && self.features.intersects(NodeResolutionFeatures::Exports)
            && self.extensions.intersects(Extensions::TypeScript | Extensions::Declaration)
            && !tspath::is_external_module_name_relative(&self.name)
            && result.is_resolved()
            && result.is_external_library_import
            && !extension_is_ok(Extensions::TypeScript | Extensions::Declaration, result.extension)
            && self.conditions.iter().any(|c| c == "import")
        {
            trace!(
                self.tracer,
                diagnostics::Resolution_of_non_relative_name_failed_trying_with_modern_Node_resolution_features_disabled_to_see_if_npm_library_needs_configuration_update
            );
            self.features &= !NodeResolutionFeatures::Exports;
            self.extensions &= Extensions::TypeScript | Extensions::Declaration;
            let diagnostics_count = self.diagnostics.len();
            let diagnostic_result = self.resolve_node_like_worker();
            if diagnostic_result.is_resolved() && diagnostic_result.is_external_library_import {
                result.alternate_result = diagnostic_result.resolved_file_name;
            }
            self.diagnostics.truncate(diagnostics_count);
        }
        result
    }

    fn resolve_node_like_worker(&mut self) -> ResolvedModule {
        let resolved = self.try_load_module_using_optional_resolution_settings();
        if !resolved.should_continue_searching() {
            return self.create_resolved_module_handling_symlink(resolved);
        }

        if !tspath::is_external_module_name_relative(&self.name) {
            if self.features.intersects(NodeResolutionFeatures::Imports) && self.name.starts_with('#') {
                let resolved = self.load_module_from_imports();
                if !resolved.should_continue_searching() {
                    return self.create_resolved_module_handling_symlink(resolved);
                }
            }
            if self.features.intersects(NodeResolutionFeatures::SelfName) {
                let resolved = self.load_module_from_self_name_reference();
                if !resolved.should_continue_searching() {
                    return self.create_resolved_module_handling_symlink(resolved);
                }
            }
            if self.name.contains(':') {
                trace!(self.tracer, diagnostics::Skipping_module_0_that_looks_like_an_absolute_URI_target_file_types_Colon_1, self.name, self.extensions);
                return self.create_resolved_module(None, false);
            }
            trace!(self.tracer, diagnostics::Loading_module_0_from_node_modules_folder_target_file_types_Colon_1, self.name, self.extensions);
            let resolved = self.load_module_from_nearest_node_modules_directory(false /*typesScopeOnly*/);
            if !resolved.should_continue_searching() {
                return self.create_resolved_module_handling_symlink(resolved);
            }
            if self.extensions.intersects(Extensions::Declaration) {
                let resolved = self.resolve_from_type_root();
                if !resolved.should_continue_searching() {
                    return self.create_resolved_module_handling_symlink(resolved);
                }
            }
        } else {
            let candidate = normalize_path_for_cjs_resolution(&self.containing_directory, &self.name);
            let resolved = self.node_load_module_by_relative_name(self.extensions, &candidate, true);
            let is_external_library_import = resolved.as_ref().is_some_and(|r| r.path.contains("/node_modules/"));
            return self.create_resolved_module(resolved, is_external_library_import);
        }
        self.create_resolved_module(None, false)
    }

    fn load_module_from_self_name_reference(&mut self) -> Option<Resolved> {
        let directory_path = tspath::get_normalized_absolute_path(&self.containing_directory, self.current_directory());
        let scope = self.get_package_scope_for_path(&directory_path);
        if !scope.exists() || scope.unwrap().contents.unwrap().exports.is_falsy() {
            // !!! falsy check seems wrong?
            return continue_searching();
        }
        let contents = scope.unwrap().contents.unwrap().get();
        let Some(name) = contents.name.get_value() else {
            return continue_searching();
        };
        let parts = tspath::get_path_components(&self.name, "");
        let name_parts = tspath::get_path_components(name, "");
        if parts.len() < name_parts.len() || name_parts[..] != parts[..name_parts.len()] {
            return continue_searching();
        }
        let trailing_parts = &parts[name_parts.len()..];
        let subpath = if !trailing_parts.is_empty() {
            let trailing: Vec<&str> = trailing_parts.iter().map(|s| s.as_str()).collect();
            tspath::combine_paths(".", &trailing)
        } else {
            ".".to_string()
        };
        // Maybe TODO: splitting extensions into two priorities should be unnecessary, except
        // https://github.com/microsoft/TypeScript/issues/50762 makes the behavior different.
        // As long as that bug exists, we need to do two passes here in self-name loading
        // in order to be consistent with (non-self) library-name loading in
        // `loadModuleFromNearestNodeModulesDirectoryWorker`, which uses two passes in order
        // to prioritize `@types` packages higher up the directory tree over untyped
        // implementation packages. See the selfNameModuleAugmentation.ts test for why this
        // matters.
        //
        // However, there's an exception. If the user has `allowJs` and `declaration`, we need
        // to ensure that self-name imports of their own package can resolve back to their
        // input JS files via `tryLoadInputFileForPath` at a higher priority than their output
        // declaration files, so we need to do a single pass with all extensions for that case.
        if self.compiler_options.get_allow_js() && !self.containing_directory.contains("/node_modules/") {
            return self.load_module_from_exports(scope, self.extensions, &subpath);
        }
        let priority_extensions = self.extensions & (Extensions::TypeScript | Extensions::Declaration);
        let secondary_extensions = self.extensions & !(Extensions::TypeScript | Extensions::Declaration);
        let resolved = self.load_module_from_exports(scope, priority_extensions, &subpath);
        if !resolved.should_continue_searching() {
            return resolved;
        }
        self.load_module_from_exports(scope, secondary_extensions, &subpath)
    }

    fn load_module_from_imports(&mut self) -> Option<Resolved> {
        if self.name == "#" || (self.name.starts_with("#/") && !self.features.intersects(NodeResolutionFeatures::ImportsPatternRoot)) {
            trace!(self.tracer, diagnostics::Invalid_import_specifier_0_has_no_possible_resolutions, self.name);
            return continue_searching();
        }
        let directory_path = tspath::get_normalized_absolute_path(&self.containing_directory, self.current_directory());
        let scope = self.get_package_scope_for_path(&directory_path);
        if !scope.exists() {
            trace!(self.tracer, diagnostics::Directory_0_has_no_containing_package_json_scope_Imports_will_not_resolve, directory_path);
            return continue_searching();
        }
        let scope_entry = scope.unwrap();
        let contents = scope_entry.contents.unwrap().get();
        if contents.imports.type_() != JSONValueType::Object {
            // !!! Old compiler only checks for undefined, but then assumes `imports` is an object if present.
            // Maybe should have a new diagnostic for imports of an invalid type. Also, array should be handled?
            trace!(self.tracer, diagnostics::X_package_json_scope_0_has_no_imports_defined, scope_entry.package_directory);
            return continue_searching();
        }

        let name = self.name.clone();
        let result = self.load_module_from_exports_or_imports(self.extensions, &name, contents.imports.as_object(), scope_entry, true /*isImports*/);
        if !result.should_continue_searching() {
            return result;
        }

        trace!(self.tracer, diagnostics::Import_specifier_0_does_not_exist_in_package_json_scope_at_path_1, self.name, scope_entry.package_directory);
        continue_searching()
    }

    fn load_module_from_exports(&mut self, package_info: Option<P<InfoCacheEntry>>, ext: Extensions, subpath: &str) -> Option<Resolved> {
        // !!! This is ported exactly, but the falsy check seems wrong
        if !package_info.exists() || package_info.unwrap().contents.unwrap().exports.is_falsy() {
            return continue_searching();
        }
        let package_info = package_info.unwrap();
        let exports = &package_info.contents.unwrap().get().exports;

        if subpath == "." {
            let mut main_export = None;
            match exports.type_() {
                JSONValueType::String | JSONValueType::Array => main_export = Some(exports),
                JSONValueType::Object => {
                    if exports.is_conditions() {
                        main_export = Some(exports);
                    } else if let Some(dot) = exports.as_object().get(".") {
                        main_export = Some(dot);
                    }
                }
                _ => {}
            }
            if let Some(main_export) = main_export {
                if main_export.type_() != JSONValueType::NotPresent {
                    return self.load_module_from_target_export_or_import(ext, subpath, package_info, false /*isImports*/, main_export, "", false /*isPattern*/, ".");
                }
            }
        } else if exports.type_() == JSONValueType::Object && exports.is_subpaths() {
            let result = self.load_module_from_exports_or_imports(ext, subpath, exports.as_object(), package_info, false /*isImports*/);
            if !result.should_continue_searching() {
                return result;
            }
        }

        trace!(self.tracer, diagnostics::Export_specifier_0_does_not_exist_in_package_json_scope_at_path_1, subpath, package_info.package_directory);
        continue_searching()
    }

    fn load_module_from_exports_or_imports(
        &mut self,
        extensions: Extensions,
        module_name: &str,
        lookup_table: &'static OrderedMap<String, packagejson::ExportsOrImports>,
        scope: P<InfoCacheEntry>,
        is_imports: bool,
    ) -> Option<Resolved> {
        if !module_name.ends_with('/') && !module_name.contains('*') {
            if let Some(target) = lookup_table.get(module_name) {
                return self.load_module_from_target_export_or_import(extensions, module_name, scope, is_imports, target, "", false /*isPattern*/, module_name);
            }
        }

        let mut expanding_keys: Vec<&'static str> = Vec::with_capacity(lookup_table.len());
        for key in lookup_table.keys() {
            if key.matches('*').count() == 1 || key.ends_with('/') {
                expanding_keys.push(key);
            }
        }
        expanding_keys.sort_by(|a, b| compare_pattern_keys(a, b).cmp(&0));

        for potential_target in expanding_keys {
            if self.features.intersects(NodeResolutionFeatures::ExportsPatternTrailers) && matches_pattern_with_trailer(potential_target, module_name) {
                let target = &lookup_table[potential_target];
                let star_pos = potential_target.find('*').unwrap();
                let subpath = &module_name[potential_target[..star_pos].len()..module_name.len() - (potential_target.len() - 1 - star_pos)];
                return self.load_module_from_target_export_or_import(extensions, module_name, scope, is_imports, target, subpath, true, potential_target);
            } else if potential_target.ends_with('*') && module_name.starts_with(&potential_target[..potential_target.len() - 1]) {
                let target = &lookup_table[potential_target];
                let subpath = &module_name[potential_target.len() - 1..];
                return self.load_module_from_target_export_or_import(extensions, module_name, scope, is_imports, target, subpath, true, potential_target);
            } else if module_name.starts_with(potential_target) {
                let target = &lookup_table[potential_target];
                let subpath = &module_name[potential_target.len()..];
                return self.load_module_from_target_export_or_import(extensions, module_name, scope, is_imports, target, subpath, false, potential_target);
            }
        }

        continue_searching()
    }

    fn load_module_from_target_export_or_import(
        &mut self,
        extensions: Extensions,
        module_name: &str,
        scope: P<InfoCacheEntry>,
        is_imports: bool,
        target: &'static packagejson::ExportsOrImports,
        subpath: &str,
        is_pattern: bool,
        key: &str,
    ) -> Option<Resolved> {
        match target.type_() {
            JSONValueType::String => {
                let target_string = target.as_string();
                if !is_pattern && !subpath.is_empty() && !target_string.ends_with('/') {
                    trace!(self.tracer, diagnostics::X_package_json_scope_0_has_invalid_type_for_target_of_specifier_1, scope.package_directory, module_name);
                    return continue_searching();
                }
                if !target_string.starts_with("./") {
                    if is_imports && !target_string.starts_with("../") && !target_string.starts_with('/') && !tspath::is_rooted_disk_path(target_string) {
                        let combined_lookup = if is_pattern { target_string.replace('*', subpath) } else { format!("{}{}", target_string, subpath) };
                        let scope_containing_directory = tspath::ensure_trailing_directory_separator(scope.package_directory);
                        if let Some(t) = self.tracer.as_mut() {
                            t.write(&diagnostics::Using_0_subpath_1_with_target_2, &[&"imports", &key, &combined_lookup]);
                            t.write(&diagnostics::Resolving_module_0_from_1, &[&combined_lookup, &scope_containing_directory]);
                        }
                        let name = std::mem::replace(&mut self.name, combined_lookup);
                        let containing_directory = std::mem::replace(&mut self.containing_directory, scope_containing_directory);
                        let result = self.resolve_node_like();
                        self.name = name;
                        self.containing_directory = containing_directory;
                        if result.is_resolved() {
                            return Some(Resolved {
                                path: result.resolved_file_name.to_string(),
                                extension: result.extension.to_string(),
                                package_id: result.package_id,
                                original_path: result.original_path.to_string(),
                                resolved_using_ts_extension: result.resolved_using_ts_extension,
                                resolved_using_extra_extensions: false,
                            });
                        }
                        return continue_searching();
                    }
                    trace!(self.tracer, diagnostics::X_package_json_scope_0_has_invalid_type_for_target_of_specifier_1, scope.package_directory, module_name);
                    return continue_searching();
                }
                let parts = if tspath::path_is_relative(target_string) {
                    tspath::get_path_components(target_string, "")[1..].to_vec()
                } else {
                    tspath::get_path_components(target_string, "")
                };
                let parts_after_first = &parts[1..];
                if parts_after_first.iter().any(|p| p == ".." || p == "." || p == "node_modules") {
                    trace!(self.tracer, diagnostics::X_package_json_scope_0_has_invalid_type_for_target_of_specifier_1, scope.package_directory, module_name);
                    return continue_searching();
                }
                let resolved_target = tspath::combine_paths(scope.package_directory, &[target_string]);
                // TODO: Assert that `resolvedTarget` is actually within the package directory? That's what the spec says.... but I'm not sure we need
                // to be in the business of validating everyone's import and export map correctness.
                let subpath_parts = tspath::get_path_components(subpath, "");
                if subpath_parts.iter().any(|p| p == ".." || p == "." || p == "node_modules") {
                    trace!(self.tracer, diagnostics::X_package_json_scope_0_has_invalid_type_for_target_of_specifier_1, scope.package_directory, module_name);
                    return continue_searching();
                }

                if let Some(t) = self.tracer.as_mut() {
                    let message_target = if is_pattern { target_string.replace('*', subpath) } else { format!("{}{}", target_string, subpath) };
                    t.write(&diagnostics::Using_0_subpath_1_with_target_2, &[&if is_imports { "imports" } else { "exports" }, &key, &message_target]);
                }
                let final_path = if is_pattern {
                    tspath::get_normalized_absolute_path(&resolved_target.replace('*', subpath), self.current_directory())
                } else {
                    tspath::get_normalized_absolute_path(&format!("{}{}", resolved_target, subpath), self.current_directory())
                };
                let mut input_link =
                    self.try_load_input_file_for_path(&final_path, subpath, &tspath::combine_paths(scope.package_directory, &["package.json"]), is_imports);
                if let Some(link) = input_link.as_mut() {
                    link.package_id = self.get_package_id(&link.path, Some(scope));
                    return input_link;
                }
                let mut result = self.load_file_name_from_package_json_field(extensions, &final_path, target_string);
                if let Some(r) = result.as_mut() {
                    r.package_id = self.get_package_id(&r.path, Some(scope));
                    return result;
                }
                continue_searching()
            }
            JSONValueType::Object => {
                trace!(self.tracer, diagnostics::Entering_conditional_exports);
                for (condition, sub_target) in target.as_object() {
                    if self.condition_matches(condition) {
                        trace!(self.tracer, diagnostics::Matched_0_condition_1, if is_imports { "imports" } else { "exports" }, condition);
                        let result = self.load_module_from_target_export_or_import(extensions, module_name, scope, is_imports, sub_target, subpath, is_pattern, key);
                        if !result.should_continue_searching() {
                            if result.is_resolved() {
                                trace!(self.tracer, diagnostics::Resolved_under_condition_0, condition);
                            }
                            trace!(self.tracer, diagnostics::Exiting_conditional_exports);
                            return result;
                        } else {
                            trace!(self.tracer, diagnostics::Failed_to_resolve_under_condition_0, condition);
                        }
                    } else {
                        trace!(self.tracer, diagnostics::Saw_non_matching_condition_0, condition);
                    }
                }
                trace!(self.tracer, diagnostics::Exiting_conditional_exports);
                continue_searching()
            }
            JSONValueType::Array => {
                if target.as_array().is_empty() {
                    trace!(self.tracer, diagnostics::X_package_json_scope_0_has_invalid_type_for_target_of_specifier_1, scope.package_directory, module_name);
                    return continue_searching();
                }
                for elem in target.as_array() {
                    let result = self.load_module_from_target_export_or_import(extensions, module_name, scope, is_imports, elem, subpath, is_pattern, key);
                    if !result.should_continue_searching() {
                        return result;
                    }
                }
                trace!(self.tracer, diagnostics::X_package_json_scope_0_has_invalid_type_for_target_of_specifier_1, scope.package_directory, module_name);
                continue_searching()
            }
            JSONValueType::Null => {
                trace!(self.tracer, diagnostics::X_package_json_scope_0_explicitly_maps_specifier_1_to_null, scope.package_directory, module_name);
                unresolved()
            }
            _ => {
                trace!(self.tracer, diagnostics::X_package_json_scope_0_has_invalid_type_for_target_of_specifier_1, scope.package_directory, module_name);
                continue_searching()
            }
        }
    }

    fn try_load_input_file_for_path(&mut self, final_path: &str, entry: &str, package_path: &str, is_imports: bool) -> Option<Resolved> {
        let compiler_options = self.compiler_options.get();
        // Replace any references to outputs for files in the program with the input files to support package self-names used with outDir
        if !self.is_config_lookup
            && (!compiler_options.declaration_dir.is_empty() || !compiler_options.out_dir.is_empty())
            && !final_path.contains("/node_modules/")
            && (compiler_options.config_file_path.is_empty()
                || tspath::contains_path(
                    &tspath::get_directory_path(package_path),
                    &compiler_options.config_file_path,
                    &ComparePathsOptions {
                        use_case_sensitive_file_names: self.fs().use_case_sensitive_file_names(),
                        current_directory: self.current_directory().to_string(),
                    },
                ))
        {
            // Note: this differs from Strada's tryLoadInputFileForPath in that it
            // does not attempt to perform "guesses", instead requring a clear root indicator.

            let root_dir: String;
            if !compiler_options.root_dir.is_empty() {
                // A `rootDir` compiler option strongly indicates the root location
                root_dir = compiler_options.root_dir.clone();
            } else if !compiler_options.config_file_path.is_empty() {
                // When no explicit rootDir is set, treat the config file's directory as the project root, which establishes the common source directory, so no other locations need to be checked.
                root_dir = tspath::get_directory_path(&compiler_options.config_file_path);
            } else {
                // replace empty string with `.` - the reverse of the operation done when entries are built - so main entrypoint errors don't look weird
                let entry_arg = if entry.is_empty() { "." } else { entry };
                let diagnostic = tsrs_ast::new_diagnostic(
                    None,
                    TextRange::default(),
                    if is_imports {
                        &diagnostics::The_project_root_is_ambiguous_but_is_required_to_resolve_import_map_entry_0_in_file_1_Supply_the_rootDir_compiler_option_to_disambiguate
                    } else {
                        &diagnostics::The_project_root_is_ambiguous_but_is_required_to_resolve_export_map_entry_0_in_file_1_Supply_the_rootDir_compiler_option_to_disambiguate
                    },
                    &[&entry_arg, &package_path],
                );
                self.diagnostics.push(diagnostic);
                return unresolved();
            }

            let candidate_directories = self.get_output_directories_for_base_directory(&root_dir);
            for candidate_dir in &candidate_directories {
                if tspath::contains_path(
                    candidate_dir,
                    final_path,
                    &ComparePathsOptions {
                        use_case_sensitive_file_names: self.fs().use_case_sensitive_file_names(),
                        current_directory: self.current_directory().to_string(),
                    },
                ) {
                    // The matched export is looking up something in either the out declaration or js dir, now map the written path back into the source dir and source extension
                    let path_fragment = if final_path.len() > candidate_dir.len() {
                        &final_path[candidate_dir.len() + 1..] // +1 to also remove directory separator
                    } else {
                        ""
                    };
                    let possible_input_base = tspath::combine_paths(&root_dir, &[path_fragment]);
                    let js_and_dts_extensions = [
                        tspath::EXTENSION_MJS,
                        tspath::EXTENSION_CJS,
                        tspath::EXTENSION_JS,
                        tspath::EXTENSION_JSON,
                        tspath::EXTENSION_DMTS,
                        tspath::EXTENSION_DCTS,
                        tspath::EXTENSION_DTS,
                    ];
                    for ext in js_and_dts_extensions {
                        if tspath::file_extension_is(&possible_input_base, ext) {
                            let input_exts = tspath::get_possible_original_input_extension_for_extension(&possible_input_base);
                            for possible_ext in &input_exts {
                                if !extension_is_ok(self.extensions, possible_ext) {
                                    continue;
                                }
                                let possible_input_with_input_extension = tspath::change_extension(&possible_input_base, possible_ext);
                                if self.fs().file_exists(&possible_input_with_input_extension) {
                                    let resolved = self.load_file_name_from_package_json_field(self.extensions, &possible_input_with_input_extension, "");
                                    if !resolved.should_continue_searching() {
                                        return resolved;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        continue_searching()
    }

    fn get_output_directories_for_base_directory(&self, common_source_dir_guess: &str) -> Vec<String> {
        let compiler_options = self.compiler_options.get();
        // Config file output paths are processed to be relative to the host's current directory, while
        // otherwise the paths are resolved relative to the common source dir the compiler puts together
        let current_dir = if !compiler_options.config_file_path.is_empty() { self.current_directory() } else { common_source_dir_guess };
        let mut candidate_directories = Vec::new();
        if !compiler_options.declaration_dir.is_empty() {
            candidate_directories.push(tspath::get_normalized_absolute_path(
                &tspath::combine_paths(current_dir, &[&compiler_options.declaration_dir]),
                self.current_directory(),
            ));
        }
        if !compiler_options.out_dir.is_empty() && compiler_options.out_dir != compiler_options.declaration_dir {
            candidate_directories
                .push(tspath::get_normalized_absolute_path(&tspath::combine_paths(current_dir, &[&compiler_options.out_dir]), self.current_directory()));
        }
        candidate_directories
    }

    fn load_module_from_nearest_node_modules_directory(&mut self, types_scope_only: bool) -> Option<Resolved> {
        let mode = if self.esm_mode || self.condition_matches("import") { ModuleKind::ESNext } else { ModuleKind::CommonJS };
        // Do (up to) two passes through node_modules:
        //   1. For each ancestor node_modules directory, try to find:
        //      i.  TS/DTS files in the implementation package
        //      ii. DTS files in the @types package
        //   2. For each ancestor node_modules directory, try to find:
        //      i.  JS files in the implementation package
        let priority_extensions = self.extensions & (Extensions::TypeScript | Extensions::Declaration);
        let secondary_extensions = self.extensions & !(Extensions::TypeScript | Extensions::Declaration);
        // (1)
        if !priority_extensions.is_empty() {
            trace!(self.tracer, diagnostics::Searching_all_ancestor_node_modules_directories_for_preferred_extensions_Colon_0, priority_extensions);
            let result = self.load_module_from_nearest_node_modules_directory_worker(priority_extensions, mode, types_scope_only);
            if !result.should_continue_searching() {
                return result;
            }
        }
        // (2)
        if !secondary_extensions.is_empty() && !types_scope_only {
            trace!(self.tracer, diagnostics::Searching_all_ancestor_node_modules_directories_for_fallback_extensions_Colon_0, secondary_extensions);
            return self.load_module_from_nearest_node_modules_directory_worker(secondary_extensions, mode, types_scope_only);
        }
        continue_searching()
    }

    fn load_module_from_nearest_node_modules_directory_worker(&mut self, ext: Extensions, _mode: ResolutionMode, types_scope_only: bool) -> Option<Resolved> {
        let containing_directory = self.containing_directory.clone();
        tspath::for_each_ancestor_directory(&containing_directory, |directory| {
            // !!! stop at global cache
            if tspath::get_base_file_name(directory) != "node_modules" {
                return self.load_module_from_immediate_node_modules_directory(ext, directory, types_scope_only);
            }
            continue_searching()
        })
    }

    fn load_module_from_immediate_node_modules_directory(&mut self, extensions: Extensions, directory: &str, types_scope_only: bool) -> Option<Resolved> {
        let node_modules_folder = tspath::combine_paths(directory, &["node_modules"]);
        if !self.fs().directory_exists(&node_modules_folder) {
            trace!(self.tracer, diagnostics::Directory_0_does_not_exist_skipping_all_lookups_in_it, node_modules_folder);
            return continue_searching();
        }

        if !types_scope_only {
            let name = self.name.clone();
            let package_result = self.load_module_from_specific_node_modules_directory(extensions, &name, &node_modules_folder);
            if !package_result.should_continue_searching() {
                return package_result;
            }
        }

        if extensions.intersects(Extensions::Declaration) {
            let node_modules_at_types = tspath::combine_paths(&node_modules_folder, &["@types"]);
            if !self.fs().directory_exists(&node_modules_at_types) {
                trace!(self.tracer, diagnostics::Directory_0_does_not_exist_skipping_all_lookups_in_it, node_modules_at_types);
                return continue_searching();
            }
            let name = self.name.clone();
            let mangled = self.mangle_scoped_package_name(&name);
            return self.load_module_from_specific_node_modules_directory(Extensions::Declaration, &mangled, &node_modules_at_types);
        }

        continue_searching()
    }

    fn load_module_from_specific_node_modules_directory(&mut self, ext: Extensions, module_name: &str, node_modules_directory: &str) -> Option<Resolved> {
        // Strip any trailing directory separator so that imports like `pkg/` and `pkg`
        // produce identical `candidate` and `packageDirectory` strings. Otherwise the
        // `package.json` info cache (which is keyed by normalized path but stores the
        // caller's `PackageDirectory` verbatim) can hand back, under concurrent
        // inserts, an entry whose `PackageDirectory` doesn't match `candidate`,
        // causing `loadNodeModuleFromDirectoryWorker`'s `ComparePaths(candidate, ...)`
        // check to fail and skip loading the package's `main`/`types` entry.
        // https://github.com/microsoft/TypeScript/tsc/issues/3526
        let candidate =
            tspath::remove_trailing_directory_separator(&tspath::normalize_path(&tspath::combine_paths(node_modules_directory, &[module_name]))).to_string();
        let (package_name, rest) = parse_package_name(module_name);
        let rest = rest.to_string();
        let mut package_directory = tspath::combine_paths(node_modules_directory, &[package_name]);
        if package_name.is_empty() {
            package_directory.clone_from(&candidate);
        }

        if self.resolve_package_directory_only {
            if self.fs().directory_exists(&package_directory) {
                return Some(Resolved { path: package_directory, ..Default::default() });
            }
            return continue_searching();
        }

        let mut root_package_info: Option<P<InfoCacheEntry>> = None;
        // First look for a nested package.json, as in `node_modules/foo/bar/package.json`
        let mut package_info = self.get_package_json_info(&candidate);
        // But only if we're not respecting export maps (if we are, we might redirect around this location)
        if !rest.is_empty() && package_info.exists() {
            if self.features.intersects(NodeResolutionFeatures::Exports) {
                root_package_info = self.get_package_json_info(&package_directory);
            }
            if !root_package_info.exists() || root_package_info.unwrap().contents.unwrap().exports.type_() == JSONValueType::NotPresent {
                let from_file = self.load_module_from_file(ext, &candidate);
                if !from_file.should_continue_searching() {
                    return from_file;
                }

                let mut from_directory = self.load_node_module_from_directory_worker(ext, &candidate, package_info);
                if let Some(d) = from_directory.as_mut() {
                    d.package_id = self.get_package_id(&d.path, package_info);
                    return from_directory;
                }
            }
        }

        let loader = |r: &mut ResolutionState<'r>, extensions: Extensions, candidate: &str, package_info: Option<P<InfoCacheEntry>>| -> Option<Resolved> {
            if !rest.is_empty() || !r.esm_mode {
                let mut from_file = r.load_module_from_file(extensions, candidate);
                if let Some(f) = from_file.as_mut() {
                    f.package_id = r.get_package_id(&f.path, package_info);
                    return from_file;
                }
            }
            let mut from_directory = r.load_node_module_from_directory_worker(extensions, candidate, package_info);
            if let Some(d) = from_directory.as_mut() {
                d.package_id = r.get_package_id(&d.path, package_info);
                return from_directory;
            }
            if rest.is_empty()
                && package_info.exists()
                && matches!(package_info.unwrap().contents.unwrap().exports.type_(), JSONValueType::NotPresent | JSONValueType::Null)
                && r.esm_mode
            {
                // EsmMode disables index lookup in `loadNodeModuleFromDirectoryWorker` generally, however non-relative package resolutions still assume
                // a default `index.js` entrypoint if no `main` or `exports` are present
                let mut index_result = r.load_module_from_file(extensions, &tspath::combine_paths(candidate, &["index.js"]));
                if let Some(i) = index_result.as_mut() {
                    i.package_id = r.get_package_id(&i.path, package_info);
                    return index_result;
                }
            }
            continue_searching()
        };

        if !rest.is_empty() {
            package_info = root_package_info;
            if package_info.is_none() {
                // Previous `packageInfo` may have been from a nested package.json; ensure we have the one from the package root now.
                package_info = self.get_package_json_info(&package_directory);
            }
        }
        if let Some(info) = package_info {
            self.resolved_package_directory = true;
            if self.features.intersects(NodeResolutionFeatures::Exports) && info.exists() && !info.contents.unwrap().exports.is_falsy() {
                // package exports are higher priority than file/directory/typesVersions lookups and (and, if there's exports present*, blocks them)
                // *Well, weirdly enough a top-level `"exports": null` does NOT block fallback resolution.
                // https://github.com/microsoft/TypeScript/pull/49327
                return self.load_module_from_exports(package_info, ext, &tspath::combine_paths(".", &[&rest]));
            }
            if !rest.is_empty() && info.exists() {
                let version_paths = self.get_version_paths(info.contents.unwrap());
                if version_paths.exists() {
                    trace!(
                        self.tracer,
                        diagnostics::X_package_json_has_a_typesVersions_entry_0_that_matches_compiler_version_1_looking_for_a_pattern_to_match_module_name_2,
                        version_paths.version,
                        tsrs_core::version(),
                        rest
                    );
                    let paths = version_paths.get_paths();
                    let path_patterns = try_parse_patterns(paths);
                    let from_paths = self.try_load_module_using_paths(ext, &rest, &package_directory, paths.unwrap(), &path_patterns, &mut |r, extensions, candidate| {
                        loader(r, extensions, candidate, package_info)
                    });
                    if !from_paths.should_continue_searching() {
                        return from_paths;
                    }
                }
            }
        }
        loader(self, ext, &candidate, package_info)
    }

    fn create_resolved_module_handling_symlink(&mut self, mut resolved: Option<Resolved>) -> ResolvedModule {
        let is_external_library_import = resolved.as_ref().is_some_and(|r| r.path.contains("/node_modules/"));
        if self.compiler_options.preserve_symlinks != Tristate::True && is_external_library_import && !tspath::is_external_module_name_relative(&self.name) {
            let r = resolved.as_mut().unwrap();
            if r.original_path.is_empty() {
                let (original_path, resolved_file_name) = self.get_original_and_resolved_file_name(&r.path);
                if !original_path.is_empty() {
                    r.path = resolved_file_name;
                    r.original_path = original_path;
                }
            }
        }
        self.create_resolved_module(resolved, is_external_library_import)
    }

    fn create_resolved_module(&self, resolved: Option<Resolved>, is_external_library_import: bool) -> ResolvedModule {
        let mut resolved_module = ResolvedModule { resolution_diagnostics: tsrs_core::alloc_slice(&self.diagnostics), ..Default::default() };

        if let Some(resolved) = resolved {
            resolved_module.resolved_file_name = alloc_string(&resolved.path);
            resolved_module.original_path = alloc_string(&resolved.original_path);
            resolved_module.is_external_library_import = is_external_library_import;
            resolved_module.resolved_using_ts_extension = resolved.resolved_using_ts_extension;
            resolved_module.resolved_using_extra_extensions = resolved.resolved_using_extra_extensions;
            resolved_module.extension = static_extension(&resolved.extension);
            resolved_module.package_id = resolved.package_id;
        }
        resolved_module
    }

    fn create_resolved_type_reference_directive(&mut self, resolved: Option<Resolved>, primary: bool) -> ResolvedTypeReferenceDirective {
        let mut resolved_type_reference_directive =
            ResolvedTypeReferenceDirective { resolution_diagnostics: tsrs_core::alloc_slice(&self.diagnostics), ..Default::default() };

        if resolved.is_resolved() {
            let resolved = resolved.unwrap();
            if !tspath::extension_is_ts(&resolved.extension) {
                panic!("expected a TypeScript file extension");
            }
            resolved_type_reference_directive.resolved_file_name = alloc_string(&resolved.path);
            resolved_type_reference_directive.primary = primary;
            resolved_type_reference_directive.package_id = resolved.package_id;
            resolved_type_reference_directive.is_external_library_import = resolved.path.contains("/node_modules/");

            if self.compiler_options.preserve_symlinks != Tristate::True {
                let (original_path, resolved_file_name) = self.get_original_and_resolved_file_name(&resolved.path);
                if !original_path.is_empty() {
                    resolved_type_reference_directive.resolved_file_name = alloc_string(&resolved_file_name);
                    resolved_type_reference_directive.original_path = alloc_string(&original_path);
                }
            }
        }
        resolved_type_reference_directive
    }

    fn get_original_and_resolved_file_name(&mut self, file_name: &str) -> (String, String) {
        let resolved_file_name = self.real_path(file_name);
        let compare_paths_options = ComparePathsOptions {
            use_case_sensitive_file_names: self.fs().use_case_sensitive_file_names(),
            current_directory: self.current_directory().to_string(),
        };
        if tspath::compare_paths(file_name, &resolved_file_name, &compare_paths_options) == 0 {
            // If the fileName and realpath are differing only in casing, prefer fileName
            // so that we can issue correct errors for casing under forceConsistentCasingInFileNames
            return (String::new(), file_name.to_string());
        }
        (file_name.to_string(), resolved_file_name)
    }

    fn try_load_module_using_optional_resolution_settings(&mut self) -> Option<Resolved> {
        let resolved = self.try_load_module_using_paths_if_eligible();
        if !resolved.should_continue_searching() {
            return resolved;
        }

        if !tspath::is_external_module_name_relative(&self.name) {
            // No more tryLoadModuleUsingBaseUrl.
            continue_searching()
        } else {
            self.try_load_module_using_root_dirs()
        }
    }

    fn get_parsed_patterns_for_paths(&self) -> P<ParsedPatterns> {
        self.resolver.get_parsed_patterns_for_paths(self.compiler_options.get())
    }

    fn try_load_module_using_paths_if_eligible(&mut self) -> Option<Resolved> {
        let compiler_options = self.compiler_options.get();
        if compiler_options.paths.as_ref().is_some_and(|p| !p.is_empty()) && !tspath::path_is_relative(&self.name) {
            trace!(self.tracer, diagnostics::X_paths_option_is_specified_looking_for_a_pattern_to_match_module_name_0, self.name);
        } else {
            return continue_searching();
        }
        let base_directory = compiler_options.get_paths_base_path(self.current_directory());
        let path_patterns = self.get_parsed_patterns_for_paths();
        let name = self.name.clone();
        self.try_load_module_using_paths(
            self.extensions,
            &name,
            &base_directory,
            compiler_options.paths.as_ref().unwrap(),
            &path_patterns,
            &mut |r, extensions, candidate| r.node_load_module_by_relative_name(extensions, candidate, true /*considerPackageJson*/),
        )
    }

    fn try_load_module_using_paths(
        &mut self,
        extensions: Extensions,
        module_name: &str,
        containing_directory: &str,
        paths: &OrderedMap<String, Vec<String>>,
        path_patterns: &ParsedPatterns,
        loader: ResolutionKindSpecificLoader<'_, 'r>,
    ) -> Option<Resolved> {
        let matched_pattern = match_pattern_or_exact(path_patterns, module_name);
        if matched_pattern.is_valid() {
            let matched_star = matched_pattern.matched_text(module_name);
            trace!(self.tracer, diagnostics::Module_name_0_matched_pattern_1, module_name, matched_pattern.text);
            let empty = Vec::new();
            for subst in paths.get(matched_pattern.text.as_str()).unwrap_or(&empty) {
                let path = subst.replacen('*', &matched_star, 1);
                let candidate = tspath::normalize_path(&tspath::combine_paths(containing_directory, &[&path]));
                trace!(self.tracer, diagnostics::Trying_substitution_0_candidate_module_location_Colon_1, subst, path);
                // A path mapping may have an extension
                let extension_from_subst = tspath::try_get_extension_from_path(subst);
                if !extension_from_subst.is_empty() {
                    if let Some(path) = self.try_file(&candidate) {
                        return Some(Resolved { path, extension: extension_from_subst.to_string(), ..Default::default() });
                    }
                }
                // When the substitution path has an explicit extension, the extension came from the
                // paths config, not the module specifier. Suppress resolvedUsingTsExtension in that case.
                let save_candidate_ending_is_from_config = self.candidate_ending_is_from_config;
                if !extension_from_subst.is_empty() {
                    self.candidate_ending_is_from_config = true;
                }
                let resolved = loader(self, extensions, &candidate);
                self.candidate_ending_is_from_config = save_candidate_ending_is_from_config;
                if !resolved.should_continue_searching() {
                    return resolved;
                }
            }
        }
        continue_searching()
    }

    fn try_load_module_using_root_dirs(&mut self) -> Option<Resolved> {
        let compiler_options = self.compiler_options.get();
        let root_dirs: &[String] = compiler_options.root_dirs.as_deref().unwrap_or(&[]);
        if root_dirs.is_empty() {
            return continue_searching();
        }

        trace!(self.tracer, diagnostics::X_rootDirs_option_is_set_using_it_to_resolve_relative_module_name_0, self.name);

        let candidate = tspath::normalize_path(&tspath::combine_paths(&self.containing_directory, &[&self.name]));

        let mut matched_root_dir: Option<&String> = None;
        let mut matched_normalized_prefix = String::new();
        for root_dir in root_dirs {
            // rootDirs are expected to be absolute
            // in case of tsconfig.json this will happen automatically - compiler will expand relative names
            // using location of tsconfig.json as base location
            let mut normalized_root = tspath::normalize_path(root_dir);
            if !normalized_root.ends_with('/') {
                normalized_root.push('/');
            }
            let is_longest_matching_prefix =
                candidate.starts_with(&normalized_root) && (matched_normalized_prefix.is_empty() || matched_normalized_prefix.len() < normalized_root.len());

            trace!(self.tracer, diagnostics::Checking_if_0_is_the_longest_matching_prefix_for_1_2, normalized_root, candidate, is_longest_matching_prefix);

            if is_longest_matching_prefix {
                matched_normalized_prefix = normalized_root;
                matched_root_dir = Some(root_dir);
            }
        }

        if !matched_normalized_prefix.is_empty() {
            trace!(self.tracer, diagnostics::Longest_matching_prefix_for_0_is_1, candidate, matched_normalized_prefix);
            let suffix = &candidate[matched_normalized_prefix.len()..];

            // first - try to load from a initial location
            trace!(self.tracer, diagnostics::Loading_0_from_the_root_dir_1_candidate_location_2, suffix, matched_normalized_prefix, candidate);
            let loader = |r: &mut Self, extensions: Extensions, candidate: &str| r.node_load_module_by_relative_name(extensions, candidate, true /*considerPackageJson*/);
            let extensions = self.extensions;
            let resolved_file_name = loader(self, extensions, &candidate);
            if !resolved_file_name.should_continue_searching() {
                return resolved_file_name;
            }

            trace!(self.tracer, diagnostics::Trying_other_entries_in_rootDirs);
            // then try to resolve using remaining entries in rootDirs
            for root_dir in root_dirs {
                if Some(root_dir) == matched_root_dir {
                    // skip the initially matched entry
                    continue;
                }
                let candidate = tspath::combine_paths(&tspath::normalize_path(root_dir), &[suffix]);
                trace!(self.tracer, diagnostics::Loading_0_from_the_root_dir_1_candidate_location_2, suffix, root_dir, candidate);
                let resolved_file_name = loader(self, extensions, &candidate);
                if !resolved_file_name.should_continue_searching() {
                    return resolved_file_name;
                }
            }
            trace!(self.tracer, diagnostics::Module_resolution_using_rootDirs_has_failed);
        }
        continue_searching()
    }

    fn node_load_module_by_relative_name(&mut self, extensions: Extensions, candidate: &str, consider_package_json: bool) -> Option<Resolved> {
        trace!(self.tracer, diagnostics::Loading_module_as_file_Slash_folder_candidate_module_location_0_target_file_types_Colon_1, candidate, extensions);
        if !tspath::has_trailing_directory_separator(candidate) {
            let parent_of_candidate = tspath::get_directory_path(candidate);
            if !self.fs().directory_exists(&parent_of_candidate) {
                trace!(self.tracer, diagnostics::Directory_0_does_not_exist_skipping_all_lookups_in_it, parent_of_candidate);
                return continue_searching();
            }
            let mut resolved_from_file = self.load_module_from_file(extensions, candidate);
            if let Some(resolved) = resolved_from_file.as_mut() {
                if consider_package_json {
                    let package_directory = parse_node_module_from_path(&resolved.path, false /*isFolder*/);
                    if !package_directory.is_empty() {
                        let info = self.get_package_json_info(&package_directory);
                        resolved.package_id = self.get_package_id(&resolved.path, info);
                    }
                }
                return resolved_from_file;
            }
        }
        if !self.fs().directory_exists(candidate) {
            trace!(self.tracer, diagnostics::Directory_0_does_not_exist_skipping_all_lookups_in_it, candidate);
            return continue_searching();
        }
        // esm mode relative imports shouldn't do any directory lookups (either inside `package.json`
        // files or implicit `index.js`es). This is a notable departure from cjs norms, where `./foo/pkg`
        // could have been redirected by `./foo/pkg/package.json` to an arbitrary location!
        if !self.esm_mode {
            return self.load_node_module_from_directory(extensions, candidate, consider_package_json);
        }
        continue_searching()
    }

    fn load_module_from_file(&mut self, extensions: Extensions, candidate: &str) -> Option<Resolved> {
        // ./foo.js -> ./foo.ts
        let resolved_by_replacing_extension = self.load_module_from_file_no_implicit_extensions(extensions, candidate);
        if resolved_by_replacing_extension.is_some() {
            return resolved_by_replacing_extension;
        }

        // ./foo -> ./foo.ts
        if !self.esm_mode {
            return self.try_adding_extensions(candidate, extensions, "");
        }

        continue_searching()
    }

    fn load_module_from_file_no_implicit_extensions(&mut self, extensions: Extensions, candidate: &str) -> Option<Resolved> {
        let base = tspath::get_base_file_name(candidate);
        if !base.contains('.') {
            return continue_searching(); // extensionless import, no lookups performed, since we don't support extensionless files
        }
        let mut extensionless = tspath::remove_file_extension(candidate);
        if extensionless == candidate {
            // Once TS native extensions are handled, handle arbitrary extensions for declaration file mapping
            let extra_extensions: Vec<&str> = self.resolver.extra_extensions.iter().map(|s| s.as_str()).collect();
            let mut extension = tspath::get_longest_extension_from_path(candidate, &extra_extensions, false);
            if extension.is_empty() {
                extension = candidate[candidate.rfind('.').unwrap()..].to_string();
            }
            extensionless = tspath::remove_extension(candidate, &extension);
        }

        let extension = &candidate[extensionless.len()..];
        trace!(self.tracer, diagnostics::File_name_0_has_a_1_extension_stripping_it, candidate, extension);
        self.try_adding_extensions(extensionless, extensions, extension)
    }

    fn try_adding_extensions(&mut self, extensionless: &str, extensions: Extensions, original_extension: &str) -> Option<Resolved> {
        let directory = tspath::get_directory_path(extensionless);
        if !directory.is_empty() && !self.fs().directory_exists(&directory) {
            return continue_searching();
        }

        match original_extension {
            tspath::EXTENSION_MJS | tspath::EXTENSION_MTS | tspath::EXTENSION_DMTS => {
                if extensions.intersects(Extensions::TypeScript) {
                    let resolved = self.try_extension(
                        tspath::EXTENSION_MTS,
                        extensionless,
                        original_extension == tspath::EXTENSION_MTS || original_extension == tspath::EXTENSION_DMTS,
                    );
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::Declaration) {
                    let resolved = self.try_extension(
                        tspath::EXTENSION_DMTS,
                        extensionless,
                        original_extension == tspath::EXTENSION_MTS || original_extension == tspath::EXTENSION_DMTS,
                    );
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::JavaScript) {
                    let resolved = self.try_extension(tspath::EXTENSION_MJS, extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                continue_searching()
            }
            tspath::EXTENSION_CJS | tspath::EXTENSION_CTS | tspath::EXTENSION_DCTS => {
                if extensions.intersects(Extensions::TypeScript) {
                    let resolved = self.try_extension(
                        tspath::EXTENSION_CTS,
                        extensionless,
                        original_extension == tspath::EXTENSION_CTS || original_extension == tspath::EXTENSION_DCTS,
                    );
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::Declaration) {
                    let resolved = self.try_extension(
                        tspath::EXTENSION_DCTS,
                        extensionless,
                        original_extension == tspath::EXTENSION_CTS || original_extension == tspath::EXTENSION_DCTS,
                    );
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::JavaScript) {
                    let resolved = self.try_extension(tspath::EXTENSION_CJS, extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                continue_searching()
            }
            tspath::EXTENSION_JSON => {
                if extensions.intersects(Extensions::Declaration) {
                    let resolved = self.try_extension(".d.json.ts", extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::Json) {
                    let resolved = self.try_extension(tspath::EXTENSION_JSON, extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                continue_searching()
            }
            tspath::EXTENSION_TSX | tspath::EXTENSION_JSX => {
                // basically idendical to the ts/js case below, but prefers matching tsx and jsx files exactly before falling back to the ts or js file path
                // (historically, we disallow having both a a.ts and a.tsx file in the same compilation, since their outputs clash)
                // TODO: We should probably error if `"./a.tsx"` resolved to `"./a.ts"`, right?
                if extensions.intersects(Extensions::TypeScript) {
                    let resolved = self.try_extension(tspath::EXTENSION_TSX, extensionless, original_extension == tspath::EXTENSION_TSX);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                    let resolved = self.try_extension(tspath::EXTENSION_TS, extensionless, original_extension == tspath::EXTENSION_TSX);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::Declaration) {
                    let resolved = self.try_extension(tspath::EXTENSION_DTS, extensionless, original_extension == tspath::EXTENSION_TSX);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::JavaScript) {
                    let resolved = self.try_extension(tspath::EXTENSION_JSX, extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                    let resolved = self.try_extension(tspath::EXTENSION_JS, extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                continue_searching()
            }
            tspath::EXTENSION_TS | tspath::EXTENSION_DTS | tspath::EXTENSION_JS | "" => {
                if extensions.intersects(Extensions::TypeScript) {
                    let resolved = self.try_extension(
                        tspath::EXTENSION_TS,
                        extensionless,
                        original_extension == tspath::EXTENSION_TS || original_extension == tspath::EXTENSION_DTS,
                    );
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                    let resolved = self.try_extension(
                        tspath::EXTENSION_TSX,
                        extensionless,
                        original_extension == tspath::EXTENSION_TS || original_extension == tspath::EXTENSION_DTS,
                    );
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::Declaration) {
                    let resolved = self.try_extension(
                        tspath::EXTENSION_DTS,
                        extensionless,
                        original_extension == tspath::EXTENSION_TS || original_extension == tspath::EXTENSION_DTS,
                    );
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::JavaScript) {
                    let resolved = self.try_extension(tspath::EXTENSION_JS, extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                    let resolved = self.try_extension(tspath::EXTENSION_JSX, extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                if self.is_config_lookup {
                    let resolved = self.try_extension(tspath::EXTENSION_JSON, extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                continue_searching()
            }
            _ => {
                if self.resolver.extra_extensions.iter().any(|e| e == original_extension) {
                    // A fully specified import of an extraExtension resolves directly to the file.
                    let mut resolved = self.try_extension(original_extension, extensionless, false);
                    if let Some(r) = resolved.as_mut() {
                        r.resolved_using_extra_extensions = true;
                        return resolved;
                    }
                }
                if extensions.intersects(Extensions::Declaration) && !tspath::is_declaration_file_name(&format!("{}{}", extensionless, original_extension)) {
                    let resolved = self.try_extension(&format!(".d{}.ts", original_extension), extensionless, false);
                    if !resolved.should_continue_searching() {
                        return resolved;
                    }
                }
                continue_searching()
            }
        }
    }

    fn try_extension(&mut self, extension: &str, extensionless: &str, resolved_using_ts_extension: bool) -> Option<Resolved> {
        let file_name = format!("{}{}", extensionless, extension);
        if let Some(path) = self.try_file(&file_name) {
            return Some(Resolved {
                path,
                extension: extension.to_string(),
                resolved_using_ts_extension: !self.candidate_ending_is_from_config && resolved_using_ts_extension,
                ..Default::default()
            });
        }
        continue_searching()
    }

    // Go returns `(fileName, ok)`; the name is only used when `ok`.
    fn try_file(&mut self, file_name: &str) -> Option<String> {
        let compiler_options = self.compiler_options.get();
        let module_suffixes: &[String] = compiler_options.module_suffixes.as_deref().unwrap_or(&[]);
        if module_suffixes.is_empty() {
            if self.try_file_lookup(file_name) {
                return Some(file_name.to_string());
            }
            return None;
        }

        let ext = tspath::try_get_extension_from_path(file_name);
        let file_name_no_extension = tspath::remove_extension(file_name, ext);
        for suffix in module_suffixes {
            let path = format!("{}{}{}", file_name_no_extension, suffix, ext);
            if self.try_file_lookup(&path) {
                return Some(path);
            }
        }
        None
    }

    fn try_file_lookup(&mut self, file_name: &str) -> bool {
        if self.fs().file_exists(file_name) {
            trace!(self.tracer, diagnostics::File_0_exists_use_it_as_a_name_resolution_result, file_name);
            return true;
        } else {
            trace!(self.tracer, diagnostics::File_0_does_not_exist, file_name);
        }
        false
    }

    fn load_node_module_from_directory(&mut self, extensions: Extensions, candidate: &str, consider_package_json: bool) -> Option<Resolved> {
        let mut package_info = None;
        if consider_package_json {
            package_info = self.get_package_json_info(candidate);
        }

        self.load_node_module_from_directory_worker(extensions, candidate, package_info)
    }

    fn load_node_module_from_directory_worker(&mut self, ext: Extensions, candidate: &str, package_info: Option<P<InfoCacheEntry>>) -> Option<Resolved> {
        let mut package_file = String::new();
        let mut version_paths: Option<&'static VersionPaths> = None;
        if package_info.exists() {
            let info = package_info.unwrap();
            version_paths = Some(self.get_version_paths(info.contents.unwrap()));
            if tspath::compare_paths(
                candidate,
                info.package_directory,
                &ComparePathsOptions { use_case_sensitive_file_names: self.fs().use_case_sensitive_file_names(), current_directory: String::new() },
            ) == 0
            {
                if let Some(file) = self.get_package_file(ext, package_info) {
                    package_file = file;
                }
            }
        }

        let loader = |r: &mut Self, extensions: Extensions, candidate: &str, package_file: &str| -> Option<Resolved> {
            let from_file = r.load_file_name_from_package_json_field(extensions, candidate, package_file);
            if !from_file.should_continue_searching() {
                return from_file;
            }

            // Even if `extensions == extensionsDeclaration`, we can still look up a .ts file as a result of package.json "types"
            // !!! should we not set this before the filename lookup above?
            let mut expanded_extensions = extensions;
            if extensions == Extensions::Declaration {
                expanded_extensions = Extensions::TypeScript | Extensions::Declaration;
            }

            // Disable `esmMode` for the resolution of the package path for CJS-mode packages (so the `main` field can omit extensions)
            let save_esm_mode = r.esm_mode;
            let save_candidate_ending_is_from_config = r.candidate_ending_is_from_config;
            r.candidate_ending_is_from_config = true;
            if package_info.exists() && package_info.unwrap().contents.unwrap().type_.value != "module" {
                r.esm_mode = false;
            }
            let result = r.node_load_module_by_relative_name(expanded_extensions, candidate, false /*considerPackageJson*/);
            r.esm_mode = save_esm_mode;
            r.candidate_ending_is_from_config = save_candidate_ending_is_from_config;
            result
        };

        let index_path = if self.is_config_lookup {
            tspath::combine_paths(candidate, &["tsconfig"])
        } else {
            tspath::combine_paths(candidate, &["index"])
        };

        if let Some(version_paths) = version_paths.filter(|v| v.exists()) {
            if package_file.is_empty() || tspath::contains_path(candidate, &package_file, &ComparePathsOptions::default()) {
                let module_name = if !package_file.is_empty() {
                    tspath::get_relative_path_from_directory(candidate, &package_file, &ComparePathsOptions::default())
                } else {
                    tspath::get_relative_path_from_directory(candidate, &index_path, &ComparePathsOptions::default())
                };
                trace!(
                    self.tracer,
                    diagnostics::X_package_json_has_a_typesVersions_entry_0_that_matches_compiler_version_1_looking_for_a_pattern_to_match_module_name_2,
                    version_paths.version,
                    tsrs_core::version(),
                    module_name
                );
                let paths = version_paths.get_paths();
                let path_patterns = try_parse_patterns(paths);
                let result = self.try_load_module_using_paths(ext, &module_name, candidate, paths.unwrap(), &path_patterns, &mut |r, extensions, candidate| {
                    loader(r, extensions, candidate, &package_file)
                });
                if let Some(result) = result {
                    if !result.package_id.name.is_empty() {
                        // !!! are these asserts really necessary?
                        panic!("expected packageId to be empty");
                    }
                    return Some(result);
                }
            }
        }

        if !package_file.is_empty() {
            let package_file_result = loader(self, ext, &package_file, &package_file);
            if let Some(package_file_result) = package_file_result {
                if !package_file_result.package_id.name.is_empty() {
                    // !!! are these asserts really necessary?
                    panic!("expected packageId to be empty");
                }
                return Some(package_file_result);
            }
        }

        // ESM mode resolutions don't do package 'index' lookups
        if !self.esm_mode {
            if !self.fs().directory_exists(candidate) {
                return continue_searching();
            }
            return self.load_module_from_file(ext, &index_path);
        }
        continue_searching()
    }

    // This function is only ever called with paths written in package.json files - never
    // module specifiers written in source files - and so it always allows the
    // candidate to end with a TS extension (but will also try substituting a JS extension for a TS extension).
    fn load_file_name_from_package_json_field(&mut self, extensions: Extensions, candidate: &str, package_json_value: &str) -> Option<Resolved> {
        if extensions.intersects(Extensions::TypeScript) && tspath::has_implementation_ts_file_extension(candidate)
            || extensions.intersects(Extensions::Declaration) && tspath::is_declaration_file_name(candidate)
        {
            if let Some(path) = self.try_file(candidate) {
                let extension = tspath::try_extract_ts_extension(&path);
                // resolvedUsingTsExtension should be true when the pattern ends with * and the
                // candidate file ends in a TS extension. This means the * matched a TS extension
                // from the module specifier. For example:
                // - import "pkg/foo.ts" with pattern "./*" -> true
                // - import "pkg/foo.ts.omg" with pattern "./*.omg" -> true (star matched .ts)
                // - import "pkg/foo" with pattern "./*.ts" -> false (extension in pattern, not specifier)
                let resolved_using_ts_extension = package_json_value.ends_with('*') && !extension.is_empty();
                return Some(Resolved { path, extension: extension.to_string(), resolved_using_ts_extension, ..Default::default() });
            }
            return continue_searching();
        }

        if self.is_config_lookup && extensions.intersects(Extensions::Json) && tspath::file_extension_is(candidate, tspath::EXTENSION_JSON) {
            if let Some(path) = self.try_file(candidate) {
                return Some(Resolved { path, extension: tspath::EXTENSION_JSON.to_string(), ..Default::default() });
            }
        }

        self.load_module_from_file_no_implicit_extensions(extensions, candidate)
    }

    // Go returns `(file, ok)`.
    fn get_package_file(&mut self, extensions: Extensions, package_info: Option<P<InfoCacheEntry>>) -> Option<String> {
        if !package_info.exists() {
            return None;
        }
        let package_info = package_info.unwrap();
        let contents = package_info.contents.unwrap().get();
        if self.is_config_lookup {
            return self.get_package_json_path_field("tsconfig", &contents.tsconfig, package_info.package_directory);
        }
        if extensions.intersects(Extensions::Declaration) {
            if let Some(package_file) = self.get_package_json_path_field("typings", &contents.typings, package_info.package_directory) {
                return Some(package_file);
            }
            if let Some(package_file) = self.get_package_json_path_field("types", &contents.types, package_info.package_directory) {
                return Some(package_file);
            }
        }
        if extensions.intersects(Extensions::ImplementationFiles | Extensions::Declaration) {
            return self.get_package_json_path_field("main", &contents.main, package_info.package_directory);
        }
        None
    }

    fn get_package_json_info(&mut self, package_directory: &str) -> Option<P<InfoCacheEntry>> {
        let package_json_path = tspath::combine_paths(package_directory, &["package.json"]);

        if let Some(existing) = InfoCache::get(&self.resolver.package_json_info_cache, &package_json_path) {
            if existing.contents.is_some() {
                trace!(self.tracer, diagnostics::File_0_exists_according_to_earlier_cached_lookups, package_json_path);
                return Some(with_package_directory(existing, package_directory));
            } else {
                if existing.directory_exists {
                    trace!(self.tracer, diagnostics::File_0_does_not_exist_according_to_earlier_cached_lookups, package_json_path);
                }
                return None;
            }
        }

        let directory_exists = self.fs().directory_exists(package_directory);
        if directory_exists && self.fs().file_exists(&package_json_path) {
            // Ignore error
            let contents = self.fs().read_file(&package_json_path).unwrap_or_default();
            let parsed = packagejson::parse(&contents);
            trace!(self.tracer, diagnostics::Found_package_json_at_0, package_json_path);
            let parseable = parsed.is_ok();
            let owner = tsrs_core::arena::enter_table_owner(self.resolver.package_json_info_cache.owner_addr());
            let result = P::new(InfoCacheEntry {
                package_directory: tsrs_core::alloc_str(package_directory),
                directory_exists: true,
                contents: Some(P::new(PackageJson::new(parsed.unwrap_or_default(), parseable))),
            });
            drop(owner);
            let result = self.resolver.package_json_info_cache.set(&package_json_path, result);
            return Some(with_package_directory(result, package_directory));
        } else {
            if directory_exists {
                trace!(self.tracer, diagnostics::File_0_does_not_exist, package_json_path);
            }
            let owner = tsrs_core::arena::enter_table_owner(self.resolver.package_json_info_cache.owner_addr());
            let entry = P::new(InfoCacheEntry { package_directory: tsrs_core::alloc_str(package_directory), directory_exists, contents: None });
            drop(owner);
            self.resolver.package_json_info_cache.set(&package_json_path, entry);
        }
        None
    }

    fn get_package_id(&mut self, resolved_file_name: &str, package_info: Option<P<InfoCacheEntry>>) -> PackageId {
        if package_info.exists() {
            let package_info = package_info.unwrap();
            let package_json_content = package_info.contents.unwrap().get();
            if let Some(name) = package_json_content.name.get_value() {
                if let Some(version) = package_json_content.version.get_value() {
                    let mut sub_module_name = "";
                    if resolved_file_name.len() > package_info.package_directory.len() {
                        sub_module_name = tsrs_core::alloc_str(&resolved_file_name[package_info.package_directory.len() + 1..]);
                    }
                    return PackageId {
                        name: name.as_str(),
                        version: version.as_str(),
                        sub_module_name,
                        peer_dependencies: alloc_string(&self.read_package_json_peer_dependencies(package_info)),
                    };
                }
            }
        }
        PackageId::default()
    }

    fn read_package_json_peer_dependencies(&mut self, package_json_info: P<InfoCacheEntry>) -> String {
        let peer_dependencies = &package_json_info.contents.unwrap().get().peer_dependencies;
        let ok = self.validate_package_json_field("peerDependencies", peer_dependencies);
        if !ok || peer_dependencies.value.is_empty() {
            return String::new();
        }
        trace!(self.tracer, diagnostics::X_package_json_has_a_peerDependencies_field);
        let package_directory = self.real_path(package_json_info.package_directory);
        let Some(node_modules_index) = package_directory.rfind("/node_modules") else {
            return String::new();
        };
        let node_modules = format!("{}/", &package_directory[..node_modules_index + "/node_modules".len()]);
        let mut names: Vec<&String> = peer_dependencies.value.keys().collect();
        names.sort();
        let mut builder = String::new();
        for name in names {
            let peer_package_json = self.get_package_json_info(&format!("{}{}", node_modules, name));
            if peer_package_json.exists() {
                let version = &peer_package_json.unwrap().contents.unwrap().get().version.value;
                builder.push('+');
                builder.push_str(name);
                builder.push('@');
                builder.push_str(version);
                trace!(self.tracer, diagnostics::Found_peerDependency_0_with_1_version, name, version);
            } else {
                trace!(self.tracer, diagnostics::Failed_to_find_peerDependency_0, name);
            }
        }
        builder
    }

    fn real_path(&mut self, path: &str) -> String {
        let rp = tspath::normalize_path(&self.fs().realpath(path));
        trace!(self.tracer, diagnostics::Resolving_real_path_for_0_result_1, path, rp);
        rp
    }

    fn validate_package_json_field(&mut self, field_name: &str, field: &dyn TypeValidatedField) -> bool {
        if field.is_present() {
            if field.is_valid() {
                return true;
            }
            trace!(self.tracer, diagnostics::Expected_type_of_0_field_in_package_json_to_be_1_got_2, field_name, field.expected_json_type(), field.actual_json_type());
        }
        trace!(self.tracer, diagnostics::X_package_json_does_not_have_a_0_field, field_name);
        false
    }

    // Go returns `(path, ok)`.
    fn get_package_json_path_field(&mut self, field_name: &str, field: &packagejson::Expected<String>, directory: &str) -> Option<String> {
        if !self.validate_package_json_field(field_name, field) {
            return None;
        }
        if field.value.is_empty() {
            trace!(self.tracer, diagnostics::X_package_json_had_a_falsy_0_field, field_name);
            return None;
        }
        let path = tspath::normalize_path(&tspath::combine_paths(directory, &[&field.value]));
        trace!(self.tracer, diagnostics::X_package_json_has_0_field_1_that_references_2, field_name, field.value, path);
        Some(path)
    }

    fn condition_matches(&self, condition: &str) -> bool {
        if condition == "default" || self.conditions.iter().any(|c| c == condition) {
            return true;
        }
        if !self.conditions.iter().any(|c| c == "types") {
            return false; // only apply versioned types conditions if the types condition is applied
        }
        is_applicable_versioned_types_key(condition)
    }

    // Go passes `r.getTraceFunc()` to `PackageJson.GetVersionPaths`.
    fn get_version_paths(&mut self, contents: P<PackageJson>) -> &'static VersionPaths {
        match self.tracer.as_mut() {
            Some(t) => {
                let mut trace = |m: &'static Message, args: &[String]| t.write_strings(m, args);
                contents.get().get_version_paths(Some(&mut trace as &mut dyn FnMut(&'static Message, &[String])))
            }
            None => contents.get().get_version_paths(None),
        }
    }
}

pub fn get_conditions(options: &CompilerOptions, resolution_mode: ResolutionMode) -> Vec<String> {
    let module_resolution = options.get_module_resolution_kind();
    let mut resolution_mode = resolution_mode;
    if resolution_mode == ModuleKind::None && module_resolution == ModuleResolutionKind::Bundler {
        resolution_mode = ModuleKind::ESNext;
    }
    let custom_conditions: &[String] = options.custom_conditions.as_deref().unwrap_or(&[]);
    let mut conditions = Vec::with_capacity(3 + custom_conditions.len());
    if resolution_mode == ModuleKind::ESNext {
        conditions.push("import".to_string());
    } else {
        conditions.push("require".to_string());
    }

    if options.no_dts_resolution != Tristate::True {
        conditions.push("types".to_string());
    }
    if module_resolution != ModuleResolutionKind::Bundler {
        conditions.push("node".to_string());
    }
    conditions.extend(custom_conditions.iter().cloned());
    conditions
}

fn get_node_resolution_features(options: &CompilerOptions) -> NodeResolutionFeatures {
    let mut features = NodeResolutionFeatures::None;

    match options.get_module_resolution_kind() {
        ModuleResolutionKind::Node16 => features = NodeResolutionFeatures::Node16Default,
        ModuleResolutionKind::NodeNext => features = NodeResolutionFeatures::NodeNextDefault,
        ModuleResolutionKind::Bundler => features = NodeResolutionFeatures::BundlerDefault,
        _ => {}
    }
    if options.resolve_package_json_exports == Tristate::True {
        features |= NodeResolutionFeatures::Exports;
    } else if options.resolve_package_json_exports == Tristate::False {
        features &= !NodeResolutionFeatures::Exports;
    }
    if options.resolve_package_json_imports == Tristate::True {
        features |= NodeResolutionFeatures::Imports;
    } else if options.resolve_package_json_imports == Tristate::False {
        features &= !NodeResolutionFeatures::Imports;
    }
    features
}

pub(crate) fn move_to_next_directory_separator_if_available(path: &str, prev_separator_index: usize, is_folder: bool) -> usize {
    let offset = prev_separator_index + 1;
    let mut next_separator_index = None;
    if offset <= path.len() {
        next_separator_index = path[offset..].find('/');
    }
    match next_separator_index {
        None => {
            if is_folder {
                return path.len();
            }
            prev_separator_index
        }
        Some(i) => i + offset,
    }
}

#[derive(Debug, Default)]
pub struct ParsedPatterns {
    matchable_string_set: Set<String>,
    patterns: Vec<Pattern>,
}

impl DefaultResolver {
    pub(crate) fn get_parsed_patterns_for_paths(&self, compiler_options: &CompilerOptions) -> P<ParsedPatterns> {
        self.parsed_patterns_for_paths.get(compiler_options.paths.as_ref())
    }
}

pub fn try_parse_patterns(path_mappings: Option<&OrderedMap<String, Vec<String>>>) -> ParsedPatterns {
    let paths: Vec<&String> = match path_mappings {
        Some(m) => m.keys().collect(),
        None => Vec::new(),
    };

    let mut num_patterns = 0;
    let mut num_matchables = 0;
    for path in &paths {
        let pattern = tsrs_core::try_parse_pattern(path);
        if pattern.is_valid() {
            if pattern.star_index == -1 {
                num_matchables += 1;
            } else {
                num_patterns += 1;
            }
        }
    }

    let mut patterns = Vec::new();
    let mut matchable_string_set = Set::new();
    if num_patterns != 0 {
        patterns = Vec::with_capacity(num_patterns);
    }
    if num_matchables != 0 {
        matchable_string_set = new_set_with_size_hint(num_matchables);
    }

    for path in paths {
        let pattern = tsrs_core::try_parse_pattern(path);
        if pattern.is_valid() {
            if pattern.star_index == -1 {
                matchable_string_set.add(path.clone());
            } else {
                patterns.push(pattern);
            }
        }
    }
    ParsedPatterns { matchable_string_set, patterns }
}

pub fn match_pattern_or_exact(patterns: &ParsedPatterns, candidate: &str) -> Pattern {
    if patterns.matchable_string_set.has(&candidate.to_string()) {
        return Pattern { text: candidate.to_string(), star_index: -1 };
    }
    if patterns.patterns.is_empty() {
        return Pattern::default();
    }
    tsrs_core::find_best_pattern_match(&patterns.patterns, |p| p.clone(), candidate).unwrap_or_default()
}

// If you import from "." inside a containing directory "/foo", the result of `tspath.NormalizePath`
// would be "/foo", but this loses the information that `foo` is a directory and we intended
// to look inside of it. The Node CommonJS resolution algorithm doesn't call this out
// (https://nodejs.org/api/modules.html#all-together), but it seems that module paths ending
// in `.` are actually normalized to `./` before proceeding with the resolution algorithm.
fn normalize_path_for_cjs_resolution(containing_directory: &str, module_name: &str) -> String {
    let combined = tspath::combine_paths(containing_directory, &[module_name]);
    let parts = tspath::get_path_components(&combined, "");
    let last_part = &parts[parts.len() - 1];
    if last_part == "." || last_part == ".." {
        return tspath::ensure_trailing_directory_separator(&tspath::normalize_path(&combined));
    }
    tspath::normalize_path(&combined)
}

fn matches_pattern_with_trailer(target: &str, name: &str) -> bool {
    if target.ends_with('*') {
        return false;
    }
    let Some((before, after)) = target.split_once('*') else {
        return false;
    };
    name.starts_with(before) && name.ends_with(after)
}

/** True if `extension` is one of the supported `extensions`. */
fn extension_is_ok(extensions: Extensions, extension: &str) -> bool {
    (extensions.intersects(Extensions::JavaScript)
        && (extension == tspath::EXTENSION_JS || extension == tspath::EXTENSION_JSX || extension == tspath::EXTENSION_MJS || extension == tspath::EXTENSION_CJS))
        || (extensions.intersects(Extensions::TypeScript)
            && (extension == tspath::EXTENSION_TS || extension == tspath::EXTENSION_TSX || extension == tspath::EXTENSION_MTS || extension == tspath::EXTENSION_CTS))
        || (extensions.intersects(Extensions::Declaration)
            && (extension == tspath::EXTENSION_DTS || extension == tspath::EXTENSION_DMTS || extension == tspath::EXTENSION_DCTS))
        || (extensions.intersects(Extensions::Json) && extension == tspath::EXTENSION_JSON)
}

pub fn resolve_config(module_name: &str, containing_file: &str, host: &'static dyn ResolutionHost) -> P<ResolvedModule> {
    let resolver = new_resolver(ResolverOptions::new(
        host,
        P::new(CompilerOptions { module_resolution: ModuleResolutionKind::NodeNext, ..Default::default() }),
    ));
    P::new(resolver.resolve_config(module_name, containing_file))
}

pub fn get_automatic_type_directive_names(options: &CompilerOptions, host: &dyn ResolutionHost) -> Vec<String> {
    if !options.uses_wildcard_types() {
        if let Some(types) = &options.types {
            return types.clone();
        }
        return Vec::new();
    }

    // Walk the primary type lookup locations
    let mut wildcard_matches = Vec::new();
    let (type_roots, _) = options.get_effective_type_roots(host.get_current_directory());
    for root in &type_roots {
        if host.fs().directory_exists(root) {
            for type_directive_path in host.fs().get_accessible_entries(root).directories {
                let normalized = tspath::normalize_path(&type_directive_path);
                let package_json_path = tspath::combine_paths(root, &[&normalized, "package.json"]);
                let mut is_not_needed_package = false;
                if host.fs().file_exists(&package_json_path) {
                    let contents = host.fs().read_file(&package_json_path).unwrap_or_default();
                    let package_json_content = packagejson::parse(&contents).unwrap_or_default();
                    // `types-publisher` sometimes creates packages with `"typings": null` for packages that don't provide their own types.
                    // See `createNotNeededPackageJSON` in the types-publisher` repo.
                    is_not_needed_package = package_json_content.typings.null;
                }
                if !is_not_needed_package {
                    let base_file_name = tspath::get_base_file_name(&normalized);
                    if !base_file_name.starts_with('.') {
                        wildcard_matches.push(base_file_name);
                    }
                }
            }
        }
    }

    // Order potentially matters in program construction, so substitute
    // in the wildcard in the position it was specified in the types array
    let mut result = Vec::new();
    for t in options.types.as_deref().unwrap_or(&[]) {
        if t == "*" {
            result.extend(wildcard_matches.iter().cloned());
        } else {
            result.push(t.clone());
        }
    }
    tsrs_core::deduplicate(&result).into_owned()
}

fn alloc_string(s: &str) -> &'static str {
    if s.is_empty() {
        return "";
    }
    tsrs_core::alloc_str(s)
}

// Extensions are nearly always one of the well-known constants; avoid leaking a copy for those.
fn static_extension(extension: &str) -> &'static str {
    const KNOWN: &[&str] = &[
        tspath::EXTENSION_TS,
        tspath::EXTENSION_TSX,
        tspath::EXTENSION_DTS,
        tspath::EXTENSION_JS,
        tspath::EXTENSION_JSX,
        tspath::EXTENSION_JSON,
        tspath::EXTENSION_MJS,
        tspath::EXTENSION_MTS,
        tspath::EXTENSION_DMTS,
        tspath::EXTENSION_CJS,
        tspath::EXTENSION_CTS,
        tspath::EXTENSION_DCTS,
    ];
    for known in KNOWN {
        if *known == extension {
            return known;
        }
    }
    alloc_string(extension)
}

// resolver.go:2126
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Ending {
    // EndingFixed indicates that the module specifier cannot be changed without changing its resolution.
    #[default]
    Fixed,
    // EndingExtensionChangeable indicates that the module specifier's extension portion was inferred from a
    // file on disk, so an interchangeable one could be used instead (e.g. replacing .d.ts with .js).
    ExtensionChangeable,
    // EndingChangeable indicates that the module specifier's file name and extension portion were inferred
    // from a file on disk without being matched as part of an 'exports' pattern, so can be changed according
    // to the importer's module resolution rules (e.g. an /index.d.ts may be dropped entirely in CommonJS settings).
    Changeable,
}

// resolver.go:2139
#[derive(Clone, Debug, Default)]
pub struct ResolvedEntrypoint {
    // OriginalFileName is the symlink path if the entrypoint was discovered at a symlink. Empty otherwise.
    pub original_file_name: String,
    // ResolvedFileName is the real path to the entrypoint file.
    pub resolved_file_name: String,
    pub module_specifier: String,
    // Ending indicates whether the file name and extension portion of ModuleSpecifier is fixed or can be changed.
    pub ending: Ending,
    // IncludeConditions are the conditions that a resolver must have to reach this entrypoint.
    pub include_conditions: Option<Set<String>>,
    // ExcludeConditions are the conditions that a resolver must not have to reach this entrypoint.
    pub exclude_conditions: Option<Set<String>>,
}

impl ResolvedEntrypoint {
    // resolver.go:2154
    pub fn symlink_or_realpath(&self) -> &str {
        if !self.original_file_name.is_empty() {
            return &self.original_file_name;
        }
        &self.resolved_file_name
    }
}

impl DefaultResolver {
    // resolver.go:2161
    pub fn get_entrypoints_from_package_json_info(
        &self,
        package_json: P<InfoCacheEntry>,
        package_name: &str,
        enable_directory_search: bool,
    ) -> Option<Vec<ResolvedEntrypoint>> {
        let extensions = Extensions::TypeScript | Extensions::Declaration;
        let features = NodeResolutionFeatures::All;
        let mut state = ResolutionState::bare(self, self.compiler_options);
        state.extensions = extensions;
        state.features = features;
        if package_json.exists() && package_json.contents.unwrap().exports.is_present() {
            let entrypoints = state.load_entrypoints_from_export_map(package_json, package_name, &package_json.contents.unwrap().exports);
            return Some(entrypoints);
        }

        let mut result: Vec<ResolvedEntrypoint> = Vec::new();
        let main_resolution = state.load_node_module_from_directory_worker(extensions, package_json.package_directory, Some(package_json));

        if main_resolution.is_resolved() {
            result.push(self.create_resolved_entrypoint_handling_symlink(&main_resolution.as_ref().unwrap().path, package_name, None, None, Ending::Fixed));
        }

        if enable_directory_search {
            let other_files = tsrs_vfs::vfsmatch::read_directory(
                self.host.fs(),
                self.host.get_current_directory(),
                package_json.package_directory,
                &extensions.array(),
                &["node_modules"],
                &["**/*"],
                tsrs_vfs::vfsmatch::UNLIMITED_DEPTH,
            );

            let compare_paths_options = ComparePathsOptions { use_case_sensitive_file_names: self.host.fs().use_case_sensitive_file_names(), ..Default::default() };
            for file in &other_files {
                if main_resolution.is_resolved() && tspath::compare_paths(file, &main_resolution.as_ref().unwrap().path, &compare_paths_options) == 0 {
                    continue;
                }

                result.push(self.create_resolved_entrypoint_handling_symlink(
                    file,
                    &tspath::resolve_path(package_name, &[&tspath::get_relative_path_from_directory(package_json.package_directory, file, &compare_paths_options)]),
                    None,
                    None,
                    Ending::Changeable,
                ));
            }
        }

        if !result.is_empty() {
            return Some(result);
        }
        None
    }

    // resolver.go:2220
    fn create_resolved_entrypoint_handling_symlink(
        &self,
        file_name: &str,
        module_specifier: &str,
        include_conditions: Option<Set<String>>,
        exclude_conditions: Option<Set<String>>,
        ending: Ending,
    ) -> ResolvedEntrypoint {
        let mut original_file_name = String::new();
        let mut resolved_file_name = file_name.to_string();
        let real_path = self.host.fs().realpath(file_name);
        if real_path != file_name {
            original_file_name = file_name.to_string();
            resolved_file_name = real_path;
        }
        ResolvedEntrypoint {
            original_file_name,
            resolved_file_name,
            module_specifier: module_specifier.to_string(),
            include_conditions,
            exclude_conditions,
            ending,
        }
    }
}

impl ResolutionState<'_> {
    // resolver.go:2237
    fn load_entrypoints_from_export_map(
        &mut self,
        package_json: P<InfoCacheEntry>,
        package_name: &str,
        exports: &packagejson::ExportsOrImports,
    ) -> Vec<ResolvedEntrypoint> {
        let mut entrypoints: Vec<ResolvedEntrypoint> = Vec::new();

        match exports.type_() {
            JSONValueType::Array => {
                for element in exports.as_array() {
                    self.load_entrypoints_from_target_exports(package_json, package_name, &mut entrypoints, ".", None, None, element);
                }
            }
            JSONValueType::Object => {
                if exports.is_subpaths() {
                    for (subpath, export) in exports.as_object() {
                        self.load_entrypoints_from_target_exports(package_json, package_name, &mut entrypoints, subpath, None, None, export);
                    }
                } else {
                    self.load_entrypoints_from_target_exports(package_json, package_name, &mut entrypoints, ".", None, None, exports);
                }
            }
            _ => self.load_entrypoints_from_target_exports(package_json, package_name, &mut entrypoints, ".", None, None, exports),
        }

        entrypoints
    }

    // resolver.go:2245 (Go's recursive closure `loadEntrypointsFromTargetExports`)
    fn load_entrypoints_from_target_exports(
        &mut self,
        package_json: P<InfoCacheEntry>,
        package_name: &str,
        entrypoints: &mut Vec<ResolvedEntrypoint>,
        subpath: &str,
        include_conditions: Option<Set<String>>,
        mut exclude_conditions: Option<Set<String>>,
        exports: &packagejson::ExportsOrImports,
    ) {
        if exports.type_() == JSONValueType::String && exports.as_string().starts_with("./") {
            let exports_str = exports.as_string();
            if exports_str.contains('*') {
                if exports_str.find('*') != exports_str.rfind('*') {
                    return;
                }
                let pattern_path = tspath::resolve_path(package_json.package_directory, &[exports_str]);
                let (leading_slice, trailing_slice) = pattern_path.split_once('*').unwrap_or((&pattern_path, ""));
                let case_sensitive = self.resolver.host.fs().use_case_sensitive_file_names();
                let files = tsrs_vfs::vfsmatch::read_directory(
                    self.resolver.host.fs(),
                    self.resolver.host.get_current_directory(),
                    package_json.package_directory,
                    &self.extensions.array(),
                    &[] as &[&str],
                    &[tspath::change_full_extension(&exports_str.replacen('*', "**/*", 1), ".*")],
                    tsrs_vfs::vfsmatch::UNLIMITED_DEPTH,
                );
                for file in &files {
                    let Some(matched_star) = self.get_matched_star_for_pattern_entrypoint(file, leading_slice, trailing_slice, case_sensitive) else {
                        continue;
                    };
                    let module_specifier = tspath::resolve_path(package_name, &[&subpath.replacen('*', &matched_star, 1)]);
                    entrypoints.push(self.resolver.create_resolved_entrypoint_handling_symlink(
                        file,
                        &module_specifier,
                        include_conditions.clone(),
                        exclude_conditions.clone(),
                        if exports_str.ends_with('*') { Ending::ExtensionChangeable } else { Ending::Fixed },
                    ));
                }
            } else {
                let parts_after_first = &tspath::get_path_components(exports_str, "")[2..];
                if parts_after_first.iter().any(|p| p == "..") || parts_after_first.iter().any(|p| p == ".") || parts_after_first.iter().any(|p| p == "node_modules") {
                    return;
                }
                let resolved_target = tspath::resolve_path(package_json.package_directory, &[exports_str]);
                let result = self.load_file_name_from_package_json_field(self.extensions, &resolved_target, exports_str);
                if result.is_resolved() {
                    entrypoints.push(self.resolver.create_resolved_entrypoint_handling_symlink(
                        &result.unwrap().path,
                        &tspath::resolve_path(package_name, &[subpath]),
                        include_conditions,
                        exclude_conditions.clone(),
                        if exports_str.ends_with('*') { Ending::ExtensionChangeable } else { Ending::Fixed },
                    ));
                }
            }
        } else if exports.type_() == JSONValueType::Array {
            for element in exports.as_array() {
                self.load_entrypoints_from_target_exports(
                    package_json,
                    package_name,
                    entrypoints,
                    subpath,
                    include_conditions.clone(),
                    exclude_conditions.clone(),
                    element,
                );
            }
        } else if exports.type_() == JSONValueType::Object {
            let mut prev_conditions: Vec<String> = Vec::new();
            for (condition, export) in exports.as_object() {
                if exclude_conditions.as_ref().is_some_and(|e| e.has(condition)) {
                    continue;
                }

                let condition_always_matches = condition == "default" || condition == "types" || is_applicable_versioned_types_key(condition);
                let mut new_include_conditions = include_conditions.clone();
                if !condition_always_matches {
                    // Go clones the (possibly nil) sets; a nil set clones to nil.
                    let mut set = new_include_conditions.unwrap_or_default();
                    set.add(condition.clone());
                    new_include_conditions = Some(set);
                    for prev_condition in &prev_conditions {
                        exclude_conditions.get_or_insert_with(Set::new).add(prev_condition.clone());
                    }
                }

                prev_conditions.push(condition.clone());
                self.load_entrypoints_from_target_exports(
                    package_json,
                    package_name,
                    entrypoints,
                    subpath,
                    new_include_conditions,
                    exclude_conditions.clone(),
                    export,
                );
                if condition_always_matches {
                    break;
                }
            }
        }
    }

    // resolver.go:2352
    fn get_matched_star_for_pattern_entrypoint(&self, file: &str, leading_slice: &str, trailing_slice: &str, case_sensitive: bool) -> Option<String> {
        if tsrs_core::stringutil::has_prefix_and_suffix_without_overlap(file, leading_slice, trailing_slice, case_sensitive) {
            return Some(file[leading_slice.len()..file.len() - trailing_slice.len()].to_string());
        }

        let js_extension = crate::util::try_get_js_extension_for_file(file, &self.compiler_options);
        if !js_extension.is_empty() {
            let swapped = tspath::change_full_extension(file, js_extension);
            if tsrs_core::stringutil::has_prefix_and_suffix_without_overlap(&swapped, leading_slice, trailing_slice, case_sensitive) {
                return Some(swapped[leading_slice.len()..swapped.len() - trailing_slice.len()].to_string());
            }
        }

        None
    }
}
