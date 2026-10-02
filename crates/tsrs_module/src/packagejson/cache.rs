use std::ops::Deref;
use std::sync::{LazyLock, OnceLock};

use tsrs_core::collections::{new_ordered_map_with_size_hint, OrderedMap, SyncMap};
use tsrs_core::semver;
use tsrs_core::tspath::{self, Path};
use tsrs_core::P;
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;

use super::jsonvalue::JSONValueType;
use super::packagejson::Fields;

static TYPE_SCRIPT_VERSION: LazyLock<semver::Version> = LazyLock::new(|| semver::must_parse(tsrs_core::version()));

#[derive(Debug)]
pub struct PackageJson {
    pub fields: Fields,
    pub parseable: bool,
    version_paths: OnceLock<(VersionPaths, Vec<DiagnosticAndArgs>)>,
}

#[derive(Clone, Debug)]
struct DiagnosticAndArgs {
    message: &'static Message,
    args: Vec<String>,
}

impl Deref for PackageJson {
    type Target = Fields;
    fn deref(&self) -> &Fields {
        &self.fields
    }
}

impl PackageJson {
    pub fn new(fields: Fields, parseable: bool) -> PackageJson {
        PackageJson { fields, parseable, version_paths: OnceLock::new() }
    }

    // `trace` is Go's `func(m *diagnostics.Message, args ...any)`; arguments arrive pre-formatted.
    pub fn get_version_paths(&self, trace: Option<&mut dyn FnMut(&'static Message, &[String])>) -> &VersionPaths {
        let (version_paths, version_traces) = self.version_paths.get_or_init(|| self.compute_version_paths());
        if let Some(trace) = trace {
            for msg in version_traces {
                trace(msg.message, &msg.args);
            }
        }
        version_paths
    }

    fn compute_version_paths(&self) -> (VersionPaths, Vec<DiagnosticAndArgs>) {
        let mut version_traces = Vec::new();
        let mut trace = |message: &'static Message, args: Vec<String>| version_traces.push(DiagnosticAndArgs { message, args });
        let types_versions = &self.fields.types_versions;
        if types_versions.type_() == JSONValueType::NotPresent {
            trace(&diagnostics::X_package_json_does_not_have_a_0_field, vec!["typesVersions".to_string()]);
            return (VersionPaths::default(), version_traces);
        }
        if types_versions.type_() != JSONValueType::Object {
            trace(
                &diagnostics::Expected_type_of_0_field_in_package_json_to_be_1_got_2,
                vec!["typesVersions".to_string(), "object".to_string(), types_versions.type_().to_string()],
            );
            return (VersionPaths::default(), version_traces);
        }

        trace(
            &diagnostics::X_package_json_has_a_typesVersions_field_with_version_specific_path_mappings,
            vec!["typesVersions".to_string()],
        );

        for (key, value) in types_versions.as_object() {
            let Some(key_range) = semver::try_parse_version_range(key) else {
                trace(&diagnostics::X_package_json_has_a_typesVersions_entry_0_that_is_not_a_valid_semver_range, vec![key.clone()]);
                continue;
            };
            if key_range.test(&TYPE_SCRIPT_VERSION) {
                if value.type_() != JSONValueType::Object {
                    trace(
                        &diagnostics::Expected_type_of_0_field_in_package_json_to_be_1_got_2,
                        vec![format!("typesVersions['{}']", key), "object".to_string(), value.type_().to_string()],
                    );
                    return (VersionPaths::default(), version_traces);
                }
                let paths_json = value.as_object();
                let mut paths = new_ordered_map_with_size_hint(paths_json.len());
                for (key, value) in paths_json {
                    if value.type_() != JSONValueType::Array {
                        continue;
                    }
                    let mut slice = vec![String::new(); value.as_array().len()];
                    for (i, path) in value.as_array().iter().enumerate() {
                        if path.type_() != JSONValueType::String {
                            continue;
                        }
                        slice[i] = path.as_string().to_string();
                    }
                    paths.insert(key.clone(), slice);
                }
                return (VersionPaths { version: key.clone(), paths: Some(paths) }, version_traces);
            }
        }

        trace(
            &diagnostics::X_package_json_does_not_have_a_typesVersions_entry_that_matches_version_0,
            vec![tsrs_core::version_major_minor().to_string()],
        );
        (VersionPaths::default(), version_traces)
    }
}

// Go keeps the raw `pathsJSON` object and converts it on the first `GetPaths` call; the conversion
// is done up front here.
#[derive(Clone, Debug, Default)]
pub struct VersionPaths {
    pub version: String,
    paths: Option<OrderedMap<String, Vec<String>>>,
}

impl VersionPaths {
    pub fn exists(&self) -> bool {
        !self.version.is_empty() && self.paths.is_some()
    }

    pub fn get_paths(&self) -> Option<&OrderedMap<String, Vec<String>>> {
        if !self.exists() {
            return None;
        }
        self.paths.as_ref()
    }
}

#[derive(Debug)]
pub struct InfoCacheEntry {
    pub package_directory: &'static str,
    pub directory_exists: bool,
    pub contents: Option<P<PackageJson>>,
}

// Go's nil-receiver methods on `*InfoCacheEntry`.
pub trait InfoCacheEntryExt {
    fn exists(&self) -> bool;
    fn get_contents(&self) -> Option<P<PackageJson>>;
    fn get_directory(&self) -> &'static str;
}

impl InfoCacheEntryExt for Option<P<InfoCacheEntry>> {
    fn exists(&self) -> bool {
        matches!(self, Some(p) if p.contents.is_some())
    }

    fn get_contents(&self) -> Option<P<PackageJson>> {
        self.and_then(|p| p.contents)
    }

    fn get_directory(&self) -> &'static str {
        match self {
            Some(p) => p.package_directory,
            None => "",
        }
    }
}

impl InfoCacheEntry {
    pub fn exists(&self) -> bool {
        self.contents.is_some()
    }

    pub fn get_contents(&self) -> Option<P<PackageJson>> {
        self.contents
    }

    pub fn get_directory(&self) -> &'static str {
        self.package_directory
    }
}

// WithPackageDirectory returns an entry whose PackageDirectory matches the
// caller's value. The package.json info cache is keyed by the canonical
// path of the package.json file, but multiple callers may look up the same
// package.json using directory paths that differ only by a trailing
// separator (e.g. "node_modules/preact/compat" vs
// "node_modules/preact/compat/"). Because the cache uses first-writer-wins
// semantics, a later caller may receive an entry whose PackageDirectory
// doesn't match its own candidate path. Downstream code compares the
// candidate against PackageDirectory, so we must return a corrected
// shallow copy when they diverge.
// See https://github.com/microsoft/TypeScript/pull/50740.
pub fn with_package_directory(p: P<InfoCacheEntry>, package_directory: &str) -> P<InfoCacheEntry> {
    if p.package_directory == package_directory {
        return p;
    }
    P::new(InfoCacheEntry {
        package_directory: tsrs_core::alloc_str(package_directory),
        directory_exists: p.directory_exists,
        contents: p.contents,
    })
}

#[derive(Debug)]
pub struct InfoCache {
    cache: SyncMap<Path, P<InfoCacheEntry>>,
    current_directory: String,
    use_case_sensitive_file_names: bool,
    // Memory regions (docs/LSP.md "Memory plan"), not in Go: the address of the cache this one was cloned from,
    // transitively (0: none). Clones share their entries, so new entries live with the original (`owner_addr`).
    origin: usize,
}

pub fn new_info_cache(current_directory: &str, use_case_sensitive_file_names: bool) -> InfoCache {
    InfoCache { cache: SyncMap::default(), current_directory: current_directory.to_string(), use_case_sensitive_file_names, origin: 0 }
}

impl InfoCache {
    // The address that decides where a new entry is allocated (`arena::enter_table_owner`): the original cache's. A
    // program's clones copy their predecessor's entries (`ResolutionData::clone_data`), so an entry added to one of
    // them must outlive it: entries stay in the region that allocates the original cache (a full build's region,
    // which every clone keeps; an auto-import update's scratch region) and go to the never-freed thread arena when
    // added from anywhere else (a clone's construction, a checker, a parse worker).
    pub fn owner_addr(&self) -> usize {
        if self.origin != 0 {
            return self.origin;
        }
        self as *const InfoCache as usize
    }

    pub fn clone_cache(&self) -> InfoCache {
        let mut clone = new_info_cache(&self.current_directory, self.use_case_sensitive_file_names);
        clone.origin = self.owner_addr();
        self.cache.range(|key, value| {
            clone.cache.store(key.clone(), *value);
            true
        });
        clone
    }

    pub fn get(&self, package_json_path: &str) -> Option<P<InfoCacheEntry>> {
        let key = tspath::to_path(package_json_path, &self.current_directory, self.use_case_sensitive_file_names);
        self.cache.load(&key)
    }

    pub fn set(&self, package_json_path: &str, info: P<InfoCacheEntry>) -> P<InfoCacheEntry> {
        let key = tspath::to_path(package_json_path, &self.current_directory, self.use_case_sensitive_file_names);
        let (actual, _) = self.cache.load_or_store(key, info);
        actual
    }

    pub fn range(&self, f: impl FnMut(&Path, &P<InfoCacheEntry>) -> bool) {
        self.cache.range(f);
    }
}
