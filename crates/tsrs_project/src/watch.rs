use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once, RwLock};

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_core::collections::{Set, SyncSet};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_ls::lsconv;
use tsrs_lsproto as lsproto;

pub(crate) const minWatchLocationDepth: usize = 2;

pub(crate) const allWatchKinds: lsproto::WatchKind = lsproto::WatchKind(1 | 2 | 4);

#[derive(Clone, PartialEq, Eq, Hash)]
struct fileSystemWatcherKey {
    pattern: String,
    kind: lsproto::WatchKind,
}

struct fileSystemWatcherValue {
    count: i32,
    id: WatcherID,
}

// watchRegistry tracks the current watch globs and how many individual
// WatchedFiles reference each glob. It provides ref-count helpers so callers
// don't manipulate the map directly.
//
// All methods are safe for concurrent use; locking is handled internally.
pub(crate) struct watchRegistry {
    mu: Mutex<watchRegistryState>,
}

struct watchRegistryState {
    entries: FxHashMap<fileSystemWatcherKey, fileSystemWatcherValue>,
    pending: FxHashSet<WatcherID>,
}

// watch.go:43
pub(crate) fn new_watch_registry() -> watchRegistry {
    watchRegistry { mu: Mutex::new(watchRegistryState { entries: FxHashMap::default(), pending: FxHashSet::default() }) }
}

impl watchRegistry {
    // watch.go:53
    // Acquire increments the ref count for a watcher. If this is the first
    // reference (count goes from 0 to 1), it returns true so the caller knows
    // to register the watcher with the client.
    pub(crate) fn acquire(&self, watcher: &lsproto::FileSystemWatcher, id: WatcherID) -> bool {
        let mut st = self.mu.lock().unwrap();
        let key = to_file_system_watcher_key(watcher);
        let value = st.entries.entry(key).or_insert_with(|| fileSystemWatcherValue { count: 0, id });
        value.count += 1;
        value.count == 1
    }

    // watch.go:69
    // Release decrements the ref count for a watcher. If no references remain,
    // the entry is removed and the function returns the WatcherID and true so
    // the caller knows to unregister the watcher from the client.
    pub(crate) fn release(&self, watcher: &lsproto::FileSystemWatcher) -> (WatcherID, bool) {
        let mut st = self.mu.lock().unwrap();
        let key = to_file_system_watcher_key(watcher);
        let Some(value) = st.entries.get_mut(&key) else {
            return (WatcherID::default(), false);
        };
        if value.count <= 1 {
            let id = value.id.clone();
            st.entries.remove(&key);
            return (id, true);
        }
        value.count -= 1;
        (WatcherID::default(), false)
    }

    // watch.go:86
    // MarkPending records that a watcher's registration failed and needs retry.
    pub(crate) fn mark_pending(&self, id: WatcherID) {
        self.mu.lock().unwrap().pending.insert(id);
    }

    // watch.go:93
    // ClearPending removes a watcher from the pending set after successful registration.
    pub(crate) fn clear_pending(&self, id: &WatcherID) {
        self.mu.lock().unwrap().pending.remove(id);
    }

    // watch.go:100
    // IsPending returns true if the watcher needs retry due to a previous failure.
    pub(crate) fn is_pending(&self, id: &WatcherID) -> bool {
        self.mu.lock().unwrap().pending.contains(id)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PatternsAndIgnored {
    pub(crate) directories_outside_workspace: Vec<String>,
    pub(crate) patterns_inside_workspace: Vec<String>,
    pub(crate) ignored: FxHashSet<String>,
}

// watch.go:119
// toFileSystemWatcherKey produces a deduplication key for a file system watcher.
// Note: this key is a simple string concatenation of the base and pattern, so
// structurally different watchers (Pattern vs RelativePattern, URI vs WorkspaceFolder)
// could theoretically collide. In practice, workspace watchers use plain Pattern
// with filesystem paths while outside-workspace watchers use RelativePattern with
// file:// URIs, so collisions don't occur.
fn to_file_system_watcher_key(w: &lsproto::FileSystemWatcher) -> fileSystemWatcherKey {
    let kind = w.kind.unwrap_or(allWatchKinds);
    let mut pattern = String::new();
    if let Some(p) = &w.glob_pattern.pattern {
        pattern = p.clone();
    } else if let Some(relative_pattern) = &w.glob_pattern.relative_pattern {
        let mut base = String::new();
        if let Some(uri) = &relative_pattern.base_uri.uri {
            base = uri.0.clone();
        } else if relative_pattern.base_uri.workspace_folder.is_some() {
            panic!("workspace folder-based relative patterns not implemented");
        }
        pattern = format!("{}/{}", base, relative_pattern.pattern);
    }
    fileSystemWatcherKey { pattern, kind }
}

// watch.go:139
pub(crate) fn file_system_watcher_glob_string(w: &lsproto::FileSystemWatcher) -> String {
    if let Some(p) = &w.glob_pattern.pattern {
        return p.clone();
    }
    if let Some(relative_pattern) = &w.glob_pattern.relative_pattern {
        let mut base = String::new();
        if let Some(uri) = &relative_pattern.base_uri.uri {
            base = uri.0.clone();
        } else if relative_pattern.base_uri.workspace_folder.is_some() {
            panic!("workspace folder-based relative patterns not implemented");
        }
        return format!("{}/{}", base, relative_pattern.pattern);
    }
    String::new()
}

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct WatcherID(pub String);

impl std::fmt::Display for WatcherID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

static watcherID: AtomicU64 = AtomicU64::new(0);

fn next_watcher_id() -> u64 {
    watcherID.fetch_add(1, Ordering::Relaxed) + 1
}

pub type ComputeGlobPatterns<T> = Arc<dyn Fn(&T) -> PatternsAndIgnored + Send + Sync>;

// Go `*WatchedFiles[T]` is shared between snapshots: `Arc<WatchedFiles<T>>`, nil = `None`.
pub struct WatchedFiles<T> {
    name: String,
    watch_kind: lsproto::WatchKind,
    has_relative_pattern_capability: bool,
    compute_glob_patterns: ComputeGlobPatterns<T>,

    mu: RwLock<watchedFilesState>,
    input: T,
    compute_watchers_once: Once,
}

struct watchedFilesState {
    workspace_watchers: Vec<lsproto::FileSystemWatcher>,
    outside_workspace_watchers: Vec<lsproto::FileSystemWatcher>,
    ignored: FxHashSet<String>,
    id: u64,
}

// watch.go:174
pub fn new_watched_files<T: Default>(
    name: String,
    watch_kind: lsproto::WatchKind,
    has_relative_pattern_capability: bool,
    compute_glob_patterns: ComputeGlobPatterns<T>,
) -> Arc<WatchedFiles<T>> {
    Arc::new(WatchedFiles {
        name,
        watch_kind,
        has_relative_pattern_capability,
        compute_glob_patterns,
        mu: RwLock::new(watchedFilesState {
            workspace_watchers: Vec::new(),
            outside_workspace_watchers: Vec::new(),
            ignored: FxHashSet::default(),
            id: next_watcher_id(),
        }),
        input: T::default(),
        compute_watchers_once: Once::new(),
    })
}

// watch.go:186
// NewWatchedFilesForPaths creates a watcher for exact file paths, routing files outside the workspace
// through directory-based external watchers so clients can use URI-based RelativePatterns when supported.
pub fn new_watched_files_for_paths(
    name: String,
    watch_kind: lsproto::WatchKind,
    has_relative_pattern_capability: bool,
    workspace_directory: &str,
    current_directory: &str,
    use_case_sensitive_file_names: bool,
) -> Arc<WatchedFiles<Vec<String>>> {
    let compare_paths_options = ComparePathsOptions { current_directory: current_directory.to_string(), use_case_sensitive_file_names };
    let workspace_directory = workspace_directory.to_string();
    new_watched_files(
        name,
        watch_kind,
        has_relative_pattern_capability,
        Arc::new(move |files: &Vec<String>| {
            let mut result = PatternsAndIgnored::default();
            for file in files {
                if tspath::contains_path(&workspace_directory, file, &compare_paths_options) {
                    result.patterns_inside_workspace.push(file.clone());
                } else {
                    result.directories_outside_workspace.push(tspath::get_directory_path(file));
                }
            }
            result
        }),
    )
}

#[derive(Clone, Debug, Default)]
pub struct Watchers {
    pub watcher_id: WatcherID,
    pub workspace_watchers: Vec<lsproto::FileSystemWatcher>,
    pub outside_workspace_watchers: Vec<lsproto::FileSystemWatcher>,
    pub ignored_paths: FxHashSet<String>,
}

fn sorted_compact(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

impl<T> WatchedFiles<T> {
    // watch.go:218
    pub fn watchers(&self) -> Watchers {
        self.compute_watchers_once.call_once(|| {
            let mut st = self.mu.write().unwrap();
            let result = (self.compute_glob_patterns)(&self.input);
            let globs = sorted_compact(result.patterns_inside_workspace);

            // ignored is only used for logging and doesn't affect watcher identity
            st.ignored = result.ignored;
            let mut changed = false;
            let same_globs = st.workspace_watchers.len() == globs.len()
                && st.workspace_watchers.iter().zip(&globs).all(|(a, b)| a.glob_pattern.pattern.as_ref().unwrap() == b);
            if !same_globs {
                st.workspace_watchers = globs
                    .iter()
                    .map(|glob| lsproto::FileSystemWatcher {
                        glob_pattern: lsproto::PatternOrRelativePattern { pattern: Some(glob.clone()), ..Default::default() },
                        kind: Some(self.watch_kind),
                    })
                    .collect();
                changed = true;
            }
            let dirs_outside = sorted_compact(result.directories_outside_workspace);
            let same_dirs = st.outside_workspace_watchers.len() == dirs_outside.len()
                && st
                    .outside_workspace_watchers
                    .iter()
                    .zip(&dirs_outside)
                    .all(|(a, b)| file_system_watcher_glob_string(a) == recursive_directory_glob_pattern(b, self.has_relative_pattern_capability));
            if !same_dirs {
                st.outside_workspace_watchers =
                    dirs_outside.iter().map(|dir| new_recursive_directory_watcher(dir, self.watch_kind, self.has_relative_pattern_capability)).collect();
                changed = true;
            }
            if changed {
                st.id = next_watcher_id();
            }
        });

        let st = self.mu.read().unwrap();
        Watchers {
            watcher_id: WatcherID(format!("{} watcher {}", self.name, st.id)),
            workspace_watchers: st.workspace_watchers.clone(),
            outside_workspace_watchers: st.outside_workspace_watchers.clone(),
            ignored_paths: st.ignored.clone(),
        }
    }

    // watch.go:273
    pub fn name(&self) -> &str {
        &self.name
    }

    // watch.go:277
    pub fn watch_kind(&self) -> lsproto::WatchKind {
        self.watch_kind
    }

    pub fn input(&self) -> &T {
        &self.input
    }

    // watch.go:281
    pub fn clone_with(&self, input: T) -> Arc<WatchedFiles<T>> {
        let st = self.mu.read().unwrap();
        Arc::new(WatchedFiles {
            name: self.name.clone(),
            watch_kind: self.watch_kind,
            has_relative_pattern_capability: self.has_relative_pattern_capability,
            compute_glob_patterns: Arc::clone(&self.compute_glob_patterns),
            mu: RwLock::new(watchedFilesState {
                workspace_watchers: st.workspace_watchers.clone(),
                outside_workspace_watchers: st.outside_workspace_watchers.clone(),
                ignored: FxHashSet::default(),
                id: 0,
            }),
            input,
            compute_watchers_once: Once::new(),
        })
    }
}

// Go's nil-receiver methods on `*WatchedFiles[T]`.
pub trait WatchedFilesExt<T> {
    // watch.go:266
    fn id(&self) -> WatcherID;
    // watch.go:281
    fn clone_with(&self, input: T) -> Option<Arc<WatchedFiles<T>>>;
}

impl<T> WatchedFilesExt<T> for Option<Arc<WatchedFiles<T>>> {
    fn id(&self) -> WatcherID {
        match self {
            None => WatcherID::default(),
            Some(w) => w.watchers().watcher_id,
        }
    }

    fn clone_with(&self, input: T) -> Option<Arc<WatchedFiles<T>>> {
        self.as_ref().map(|w| w.clone_with(input))
    }
}

// watch.go:298
pub(crate) fn create_resolution_lookup_glob_mapper(
    workspace_directory: &str,
    lib_directory: &str,
    current_directory: &str,
    use_case_sensitive_file_names: bool,
) -> ComputeGlobPatterns<Option<Arc<SyncSet<Path>>>> {
    let workspace_directory_path = tspath::to_path(workspace_directory, current_directory, use_case_sensitive_file_names);
    let current_directory_path = tspath::to_path(current_directory, current_directory, use_case_sensitive_file_names);
    let lib_directory_path = tspath::to_path(lib_directory, current_directory, use_case_sensitive_file_names);

    Arc::new(move |data: &Option<Arc<SyncSet<Path>>>| {
        let mut ignored = FxHashSet::default();
        let mut seen_dirs: Set<Path> = Set::default();
        let (mut include_workspace, mut include_root, mut include_lib) = (false, false, false);
        let mut node_modules_directories: Set<Path> = Set::default();
        let mut external_directories: Set<Path> = Set::default();

        if let Some(data) = data {
            data.range(|path| {
                if tspath::is_dynamic_file_name(path) {
                    return true;
                }
                // Assuming all of the input paths are file paths, we can avoid
                // duplicate work by only taking one file per dir, since their outputs
                // will always be the same.
                if !seen_dirs.add_if_absent(path.get_directory_path()) {
                    return true;
                }

                if workspace_directory_path.contains_path(path) {
                    include_workspace = true;
                } else if current_directory_path.contains_path(path) {
                    include_root = true;
                } else if lib_directory_path.contains_path(path) {
                    include_lib = true;
                } else if let Some(idx) = path.find("/node_modules/") {
                    node_modules_directories.add(Path::from(&path[..idx + "/node_modules".len()]));
                } else {
                    external_directories.add(path.get_directory_path());
                }
                true
            });
        }

        let mut globs = Vec::new();
        if include_workspace {
            globs.push(get_recursive_glob_pattern(&workspace_directory_path));
        }
        if include_root {
            globs.push(get_recursive_glob_pattern(&current_directory_path));
        }
        if include_lib {
            globs.push(get_recursive_glob_pattern(&lib_directory_path));
        }
        if node_modules_directories.len() > 0 {
            let mut node_modules_globs: Vec<String> = node_modules_directories.keys().iter().map(|dir| get_recursive_glob_pattern(dir)).collect();
            node_modules_globs.sort();
            globs.extend(node_modules_globs);
        }
        let mut outside_dirs = Vec::new();
        if external_directories.len() > 0 {
            let external_dir_strings: Vec<String> = external_directories.keys().iter().map(|dir| dir.to_string()).collect();
            let (mut external_directory_parents, ignored_external_dirs) = tspath::get_common_parents(
                &external_dir_strings,
                minWatchLocationDepth,
                get_path_components_for_watching,
                // Already using tspath.Path
                &ComparePathsOptions { use_case_sensitive_file_names: true, current_directory: String::new() },
            );
            external_directory_parents.sort();
            ignored = ignored_external_dirs;
            outside_dirs = external_directory_parents;
        }

        PatternsAndIgnored { directories_outside_workspace: outside_dirs, patterns_inside_workspace: globs, ignored }
    })
}

// watch.go:380
pub(crate) fn get_typings_locations_globs(
    typings_files: &[String],
    typings_location: &str,
    workspace_directory: &str,
    current_directory: &str,
    use_case_sensitive_file_names: bool,
) -> PatternsAndIgnored {
    let (mut include_typings_location, mut include_workspace) = (false, false);
    let mut external_directories: FxHashMap<Path, String> = FxHashMap::default();
    let mut globs: FxHashMap<Path, String> = FxHashMap::default();
    let compare_paths_options = ComparePathsOptions { current_directory: current_directory.to_string(), use_case_sensitive_file_names };
    for file in typings_files {
        if tspath::contains_path(typings_location, file, &compare_paths_options) {
            include_typings_location = true;
        } else if !tspath::contains_path(workspace_directory, file, &compare_paths_options) {
            let directory = tspath::get_directory_path(file);
            external_directories.insert(tspath::to_path(&directory, current_directory, use_case_sensitive_file_names), directory);
        } else {
            include_workspace = true;
        }
    }
    let external: Vec<String> = external_directories.values().cloned().collect();
    let (mut external_directory_parents, ignored) =
        tspath::get_common_parents(&external, minWatchLocationDepth, get_path_components_for_watching, &compare_paths_options);
    external_directory_parents.sort();
    if include_workspace {
        globs.insert(tspath::to_path(workspace_directory, current_directory, use_case_sensitive_file_names), get_recursive_glob_pattern(workspace_directory));
    }
    if include_typings_location {
        globs.insert(tspath::to_path(typings_location, current_directory, use_case_sensitive_file_names), get_recursive_glob_pattern(typings_location));
    }
    PatternsAndIgnored {
        directories_outside_workspace: external_directory_parents,
        patterns_inside_workspace: globs.into_values().collect(),
        ignored,
    }
}

// watch.go:424
pub(crate) fn get_path_components_for_watching(path: &str, current_directory: &str) -> Vec<String> {
    let components = tspath::get_path_components(path, current_directory);
    let root_length = perceived_os_root_length_for_watching(&components);
    if root_length <= 1 {
        return components;
    }
    let rest: Vec<&str> = components[1..root_length].iter().map(|s| s.as_str()).collect();
    let new_root = tspath::combine_paths(&components[0], &rest);
    let mut result = vec![new_root];
    result.extend(components[root_length..].iter().cloned());
    result
}

// watch.go:434
fn perceived_os_root_length_for_watching(path_components: &[String]) -> usize {
    let length = path_components.len();
    if length <= 1 {
        return length;
    }
    if path_components[0].starts_with("//") {
        // Group UNC roots (//server/share) into a single component
        return 2;
    }
    let c0 = path_components[0].as_bytes();
    if c0.len() == 3 && tspath::is_volume_character(c0[0]) && c0[1] == b':' && c0[2] == b'/' {
        // Windows-style volume
        if path_components[1].eq_ignore_ascii_case("users") {
            // Group C:/Users/username into a single component
            return 3.min(length);
        }
        return 1;
    }
    if path_components[1] == "home" {
        // Group /home/username into a single component
        return 3.min(length);
    }
    1
}

// watch.go:458
pub(crate) fn get_recursive_glob_pattern(directory: &str) -> String {
    format!("{}/{}", tspath::remove_trailing_directory_separator(directory), "**/*")
}

// watch.go:464
// recursiveDirectoryGlobPattern returns the string form of a recursive watcher
// for the given directory that would be produced by newRecursiveDirectoryWatcher.
fn recursive_directory_glob_pattern(directory: &str, use_relative_pattern: bool) -> String {
    if use_relative_pattern {
        return lsconv::file_name_to_document_uri(directory).0 + "/**/*";
    }
    get_recursive_glob_pattern(directory)
}

// watch.go:474
// newRecursiveDirectoryWatcher creates a FileSystemWatcher for recursively
// watching a directory. When useRelativePattern is true, a RelativePattern with
// a file:// base URI is used; otherwise a plain glob Pattern is used.
fn new_recursive_directory_watcher(directory: &str, kind: lsproto::WatchKind, use_relative_pattern: bool) -> lsproto::FileSystemWatcher {
    if use_relative_pattern {
        let base_uri = lsproto::URI(lsconv::file_name_to_document_uri(directory).0);
        return lsproto::FileSystemWatcher {
            glob_pattern: lsproto::PatternOrRelativePattern {
                relative_pattern: Some(lsproto::RelativePattern {
                    base_uri: lsproto::WorkspaceFolderOrURI { uri: Some(base_uri), ..Default::default() },
                    pattern: "**/*".to_string(),
                }),
                ..Default::default()
            },
            kind: Some(kind),
        };
    }
    let glob = get_recursive_glob_pattern(directory);
    lsproto::FileSystemWatcher {
        glob_pattern: lsproto::PatternOrRelativePattern { pattern: Some(glob), ..Default::default() },
        kind: Some(kind),
    }
}
