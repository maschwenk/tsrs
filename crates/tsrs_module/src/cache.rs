use std::ops::Deref;

use rustc_hash::FxHashMap;
use tsrs_core::collections::{Equivalent, OrderedMap, SyncMap};
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

impl Equivalent<ModuleResolutionCacheKey> for (&str, &str, ResolutionMode, &str) {
    fn equivalent(&self, key: &ModuleResolutionCacheKey) -> bool {
        self.0 == key.containing_directory && self.1 == key.module_name && self.2 == key.resolution_mode && self.3 == key.redirect_config_name
    }
}

#[derive(Default)]
pub(crate) struct ModuleResolutionCache {
    cache: SyncMap<ModuleResolutionCacheKey, P<ResolvedModule>>,
}

impl ModuleResolutionCache {
    #[cfg(test)]
    pub(crate) fn size(&self) -> usize {
        self.cache.size()
    }

    pub(crate) fn get(&self, key: (&str, &str, ResolutionMode, &str)) -> Option<P<ResolvedModule>> {
        self.cache.load_equivalent(&key)
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

impl Equivalent<TypeRefDirectiveResolutionCacheKey> for (&str, &str, ResolutionMode, &str, bool) {
    fn equivalent(&self, key: &TypeRefDirectiveResolutionCacheKey) -> bool {
        self.0 == key.containing_directory
            && self.1 == key.type_reference_name
            && self.2 == key.resolution_mode
            && self.3 == key.redirect_config_name
            && self.4 == key.from_inferred_types_containing_file
    }
}

#[derive(Default)]
pub(crate) struct TypeRefDirectiveResolutionCache {
    cache: SyncMap<TypeRefDirectiveResolutionCacheKey, P<ResolvedTypeReferenceDirective>>,
}

impl TypeRefDirectiveResolutionCache {
    #[cfg(test)]
    pub(crate) fn size(&self) -> usize {
        self.cache.size()
    }

    pub(crate) fn get(&self, key: (&str, &str, ResolutionMode, &str, bool)) -> Option<P<ResolvedTypeReferenceDirective>> {
        self.cache.load_equivalent(&key)
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
    #[cfg(test)]
    pub(crate) fn size(&self) -> usize {
        self.cache.size()
    }

    pub(crate) fn get(&self, path_mappings: Option<&OrderedMap<String, Vec<String>>>) -> P<ParsedPatterns> {
        let key = path_mappings.map_or(0, |m| std::ptr::from_ref::<OrderedMap<String, Vec<String>>>(m) as usize);
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

pub(crate) fn get_redirect_config_name(redirect: Option<&dyn ResolvedProjectReference>) -> &str {
    match redirect {
        None => "",
        Some(redirect) => redirect.config_name(),
    }
}

impl Deref for DefaultResolver {
    type Target = ResolutionData;
    fn deref(&self) -> &ResolutionData {
        &self.resolution_data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsrs_core::ModuleKind;

    #[test]
    fn borrowed_queries_keep_all_cache_key_fields() {
        let modules = ModuleResolutionCache::default();
        let result = P::new(ResolvedModule::default());
        modules.set(
            ModuleResolutionCacheKey {
                containing_directory: "/é/src".into(),
                module_name: "pkg".into(),
                resolution_mode: ModuleKind::ESNext,
                redirect_config_name: "/config.json".into(),
            },
            result,
        );
        assert_eq!(modules.get(("/é/src", "pkg", ModuleKind::ESNext, "/config.json")), Some(result));
        for query in [
            ("/other", "pkg", ModuleKind::ESNext, "/config.json"),
            ("/é/src", "other", ModuleKind::ESNext, "/config.json"),
            ("/é/src", "pkg", ModuleKind::CommonJS, "/config.json"),
            ("/é/src", "pkg", ModuleKind::ESNext, "/other.json"),
        ] {
            assert!(modules.get(query).is_none());
        }
        let types = TypeRefDirectiveResolutionCache::default();
        let result = P::new(ResolvedTypeReferenceDirective::default());
        types.set(
            TypeRefDirectiveResolutionCacheKey {
                containing_directory: "/src".into(),
                type_reference_name: "pkg".into(),
                resolution_mode: ModuleKind::CommonJS,
                redirect_config_name: "/config.json".into(),
                from_inferred_types_containing_file: true,
            },
            result,
        );
        assert_eq!(types.get(("/src", "pkg", ModuleKind::CommonJS, "/config.json", true)), Some(result));
        for query in [
            ("/other", "pkg", ModuleKind::CommonJS, "/config.json", true),
            ("/src", "other", ModuleKind::CommonJS, "/config.json", true),
            ("/src", "pkg", ModuleKind::ESNext, "/config.json", true),
            ("/src", "pkg", ModuleKind::CommonJS, "/other.json", true),
            ("/src", "pkg", ModuleKind::CommonJS, "/config.json", false),
        ] {
            assert!(types.get(query).is_none());
        }
    }
}
