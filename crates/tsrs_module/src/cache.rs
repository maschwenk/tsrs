use std::ops::Deref;

use rustc_hash::FxHashMap;
use tsrs_core::collections::{OrderedMap, SyncMap};
use tsrs_core::tspath::Path;
use tsrs_core::{CompilerOptions, ResolutionMode, P};

use crate::packagejson::{self, InfoCache, InfoCacheEntry};
use crate::resolver::{try_parse_patterns, DefaultResolver, ParsedPatterns, ResolverOptions};
use crate::types::{ModeAwareCacheKey, ResolutionHost, ResolvedModule, ResolvedProjectReference, ResolvedTypeReferenceDirective};

pub type ModeAwareCache<T> = FxHashMap<ModeAwareCacheKey, T>;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ModuleResolutionCacheKey {
    pub(crate) containing_directory: String,
    pub(crate) module_name: String,
    pub(crate) resolution_mode: ResolutionMode,
    pub(crate) redirect_config_name: String,
}

#[derive(Default)]
pub(crate) struct ModuleResolutionCache {
    cache: SyncMap<ModuleResolutionCacheKey, P<ResolvedModule>>,
}

impl ModuleResolutionCache {
    pub(crate) fn size(&self) -> usize {
        self.cache.size()
    }

    pub(crate) fn get(&self, key: &ModuleResolutionCacheKey) -> Option<P<ResolvedModule>> {
        self.cache.load(key)
    }

    pub(crate) fn set(&self, key: ModuleResolutionCacheKey, value: P<ResolvedModule>) {
        self.cache.load_or_store(key, value);
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct TypeRefDirectiveResolutionCacheKey {
    pub(crate) containing_directory: String,
    pub(crate) type_reference_name: String,
    pub(crate) resolution_mode: ResolutionMode,
    pub(crate) redirect_config_name: String,
    pub(crate) from_inferred_types_containing_file: bool,
}

#[derive(Default)]
pub(crate) struct TypeRefDirectiveResolutionCache {
    cache: SyncMap<TypeRefDirectiveResolutionCacheKey, P<ResolvedTypeReferenceDirective>>,
}

impl TypeRefDirectiveResolutionCache {
    pub(crate) fn size(&self) -> usize {
        self.cache.size()
    }

    pub(crate) fn get(&self, key: &TypeRefDirectiveResolutionCacheKey) -> Option<P<ResolvedTypeReferenceDirective>> {
        self.cache.load(key)
    }

    pub(crate) fn set(&self, key: TypeRefDirectiveResolutionCacheKey, value: P<ResolvedTypeReferenceDirective>) {
        self.cache.store(key, value);
    }
}

// Keyed by the identity (address) of the path mappings, like Go's pointer key.
#[derive(Default)]
pub(crate) struct ParsedPatternsCache {
    cache: SyncMap<usize, P<ParsedPatterns>>,
}

impl ParsedPatternsCache {
    pub(crate) fn size(&self) -> usize {
        self.cache.size()
    }

    pub(crate) fn get(&self, path_mappings: Option<&OrderedMap<String, Vec<String>>>) -> P<ParsedPatterns> {
        let key = path_mappings.map_or(0, |m| m as *const OrderedMap<String, Vec<String>> as usize);
        match self.cache.load(&key) {
            Some(patterns) => patterns,
            None => self.cache.load_or_store(key, P::new(try_parse_patterns(path_mappings))).0,
        }
    }
}

pub struct ResolutionData {
    pub(crate) compiler_options: P<CompilerOptions>,
    pub(crate) typings_location: String,
    pub(crate) project_name: String,
    pub(crate) extra_extensions: Vec<String>,

    pub(crate) package_json_info_cache: P<InfoCache>,
}

pub(crate) fn new_resolution_data(opts: ResolverOptions) -> P<ResolutionData> {
    let package_json_info_cache = match opts.package_json_cache {
        Some(cache) => cache,
        None => P::new(packagejson::new_info_cache(opts.host.get_current_directory(), opts.host.fs().use_case_sensitive_file_names())),
    };
    P::new(ResolutionData {
        compiler_options: opts.compiler_options,
        typings_location: opts.typings_location,
        project_name: opts.project_name,
        extra_extensions: opts.extra_extensions,
        package_json_info_cache,
    })
}

impl ResolutionData {
    // Clone copies the package-json cache table without copying its entries.
    pub fn clone_data(&self) -> P<ResolutionData> {
        P::new(ResolutionData {
            compiler_options: self.compiler_options,
            typings_location: self.typings_location.clone(),
            project_name: self.project_name.clone(),
            extra_extensions: self.extra_extensions.clone(),
            package_json_info_cache: P::new(self.package_json_info_cache.clone_cache()),
        })
    }

    pub fn package_json_cache_entries(&self, f: impl FnMut(&Path, &P<InfoCacheEntry>) -> bool) {
        self.package_json_info_cache.range(f);
    }

    pub fn new_resolver(&'static self, host: &'static dyn ResolutionHost) -> DefaultResolver {
        DefaultResolver::new_from_resolution_data(P::from_static(self), host)
    }
}

pub(crate) fn get_redirect_config_name(redirect: Option<&dyn ResolvedProjectReference>) -> String {
    match redirect {
        None => String::new(),
        Some(redirect) => redirect.config_name().to_string(),
    }
}

impl Deref for DefaultResolver {
    type Target = ResolutionData;
    fn deref(&self) -> &ResolutionData {
        &self.resolution_data
    }
}
