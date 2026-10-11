//! Filesystem snapshot for one native, nonincremental CLI type check.
//!
//! Adapted from oxc-resolver's Cache/CachedPath/CachedMeta at 2e976e6489ff: one interned path owns
//! both views of its metadata and its lazy filesystem results. Resolution algorithms and the
//! semantic package.json cache remain in tsrs_module. See notes/perf-resolver-path-cache.md.

use std::hash::{Hash, Hasher};
#[cfg(target_os = "macos")]
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock, RwLock, Weak};
use std::time::SystemTime;

use hashbrown::{HashTable, hash_table::Entry};
use rustc_hash::FxHasher;
use tsrs_core::tspath;

use crate::{Entries, FS, FileInfo, FileMode};

const SHARDS: usize = 64;
const INITIALIZED: u8 = 1;
const EXISTS: u8 = 2;
const DIRECTORY: u8 = 4;
const SYMLINK: u8 = 8;
const DIRECTORY_SCAN_AFTER_MISSES: u8 = 8;

// Negative answers alone are inferred from a listing. ASCII case-insensitive matching is
// conservative on both sensitive and insensitive volumes. Directories with non-ASCII entries
// are ineligible: OS Unicode normalization and case folding can differ from Rust's.
struct DirectoryNames(HashTable<Box<str>>);

fn name_hash(name: &str) -> u64 {
    let mut hasher = FxHasher::default();
    for chunk in name.as_bytes().chunks(8) {
        let mut lower = [0; 8];
        lower[..chunk.len()].copy_from_slice(chunk);
        lower.make_ascii_lowercase();
        hasher.write(&lower[..chunk.len()]);
    }
    hasher.finish()
}

impl DirectoryNames {
    fn read(path: &str) -> Option<Self> {
        let mut names = HashTable::new();
        for name in super::directory_names(path)? {
            let hash = name_hash(&name);
            // Case variants may coexist on a sensitive volume. Either name prevents inference.
            names.insert_unique(hash, name, |name| name_hash(name));
        }
        Some(Self(names))
    }

    fn excludes(&self, name: &str) -> bool {
        !matches!(name, "" | "." | "..")
            && name.is_ascii()
            && self.0.find(name_hash(name), |entry| entry.eq_ignore_ascii_case(name)).is_none()
    }
}

#[derive(Default)]
struct CachedMeta {
    link: AtomicU8,
    followed: AtomicU8,
}

const _: () = assert!(std::mem::size_of::<CachedMeta>() == 2);

fn encode(meta: Option<FileMode>) -> u8 {
    meta.map_or(INITIALIZED, |mode| {
        INITIALIZED | EXISTS | if mode.is_dir() { DIRECTORY } else { 0 } | if mode.intersects(FileMode::Symlink) { SYMLINK } else { 0 }
    })
}

fn decode(bits: u8) -> Option<FileMode> {
    if bits & EXISTS == 0 {
        return None;
    }
    let mut mode = FileMode::None;
    if bits & DIRECTORY != 0 {
        mode |= FileMode::Dir;
    }
    if bits & SYMLINK != 0 {
        mode |= FileMode::Symlink;
    }
    Some(mode)
}

fn metadata_or_init(slot: &AtomicU8, operation: &'static str, init: impl FnOnce() -> Option<FileMode>) -> Option<FileMode> {
    // The entire value is in this byte; no other memory is published. As in Oxc, cold probes may race.
    let bits = slot.load(Ordering::Relaxed);
    if bits & INITIALIZED != 0 {
        crate::fsstats::cache(operation, if bits & EXISTS != 0 { "hit-positive" } else { "hit-negative" });
        return decode(bits);
    }
    let meta = init();
    crate::fsstats::cache(operation, if meta.is_some() { "miss-positive" } else { "miss-negative" });
    // The snapshot is immutable, so racing initializers produce the same self-contained value.
    slot.store(encode(meta), Ordering::Relaxed);
    meta
}

fn cached<'a, T>(slot: &'a OnceLock<T>, operation: &'static str, init: impl FnOnce() -> T) -> &'a T {
    let mut initialized = false;
    let result = slot.get_or_init(|| {
        initialized = true;
        init()
    });
    crate::fsstats::cache(operation, if initialized { "miss" } else { "hit" });
    result
}

#[derive(Default)]
struct PathData {
    stat: OnceLock<Option<FileInfo>>,
    entries: OnceLock<Entries>,
    package_json: OnceLock<Option<String>>,
    names: OnceLock<Option<DirectoryNames>>,
    #[cfg(target_os = "macos")]
    link_target: OnceLock<Option<String>>,
}

struct CanonicalPath {
    target: Weak<CachedPath>,
    spelling: Box<str>,
}

struct CachedPath {
    path: Box<str>,
    hash: u64,
    parent: Option<Weak<Self>>,
    normalized: bool,
    meta: CachedMeta,
    missing_children: AtomicU8,
    // None is a failed realpath, distinct from a successful mapping to this entry itself.
    canonicalized: OnceLock<Option<CanonicalPath>>,
    data: OnceLock<Box<PathData>>,
}

impl CachedPath {
    fn data(&self) -> &PathData {
        self.data.get_or_init(|| Box::new(PathData::default()))
    }

    fn link_metadata(&self) -> Option<FileMode> {
        metadata_or_init(&self.meta.link, "link metadata", || {
            // Raw realpath spellings can contain literal backslashes, whose physical parent
            // differs from tspath's lexical parent. Only normalized metadata keys may infer absence.
            let parent = if self.normalized { self.parent.as_ref().and_then(Weak::upgrade) } else { None };
            if let Some(parent) = &parent {
                // These are self-contained atomic values; the listing itself is published by
                // OnceLock. Relaxed reads here do not publish any other memory.
                let followed = parent.meta.followed.load(Ordering::Relaxed);
                let link = parent.meta.link.load(Ordering::Relaxed);
                let directory = INITIALIZED | EXISTS | DIRECTORY;
                let known_directory = followed & directory == directory || link & directory == directory;
                // This counter is only a heuristic threshold, with no dependent memory to acquire.
                if known_directory && parent.missing_children.load(Ordering::Relaxed) >= DIRECTORY_SCAN_AFTER_MISSES {
                    if let Some(names) = cached(&parent.data().names, "directory names", || DirectoryNames::read(&parent.path)) {
                        let name = self.path.rsplit('/').next().unwrap_or_default();
                        if names.excludes(name) {
                            crate::fsstats::cache("directory names", "negative");
                            return None;
                        }
                    }
                }
            }
            let result = super::symlink_file_type(&self.path);
            if result.as_ref().is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) {
                if let Some(parent) = parent {
                    // This bounded count only selects when to try a listing. It publishes no
                    // filesystem facts; errors other than NotFound do not advance it.
                    let _ = parent.missing_children.try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                        (count < DIRECTORY_SCAN_AFTER_MISSES).then(|| count + 1)
                    });
                }
            }
            result.ok()
        })
    }

    #[cfg(target_os = "macos")]
    fn read_link(&self) -> io::Result<String> {
        cached(&self.data().link_target, "readlink", || super::read_link(&self.path).ok())
            .clone()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

struct ResolverFsCache {
    paths: Box<[RwLock<HashTable<Arc<CachedPath>>>]>,
}

/// A fresh filesystem snapshot for a read-only compilation with stable inputs. It must not be
/// reused across compilations or used by external content mappers, build mode, or a server.
pub fn for_compilation() -> impl FS {
    ResolverFsCache::new()
}

impl ResolverFsCache {
    fn new() -> Self {
        Self { paths: (0..SHARDS).map(|_| RwLock::new(HashTable::new())).collect() }
    }

    fn intern(&self, path: &str) -> Arc<CachedPath> {
        let mut hasher = FxHasher::default();
        path.hash(&mut hasher);
        self.intern_with_hash(path, hasher.finish())
    }

    fn intern_with_hash(&self, path: &str, hash: u64) -> Arc<CachedPath> {
        let shard = &self.paths[(hash >> (u64::BITS - SHARDS.trailing_zeros())) as usize];
        if let Some(entry) = shard.read().unwrap().find(hash, |entry| &*entry.path == path) {
            crate::fsstats::cache("intern", "hit");
            return Arc::clone(entry);
        }
        // Intern parents without a shard guard: a parent can hash to this same shard.
        let parent_path = tspath::get_directory_path(path);
        let parent = (parent_path != path && !parent_path.is_empty()).then(|| self.intern(&parent_path));
        let root_len = tspath::get_root_length(path);
        let normalized = !path.contains('\\') && (root_len == path.len() || crate::internal::valid_path(&path[root_len..]));
        let entry = Arc::new(CachedPath {
            path: path.into(),
            hash,
            parent: parent.as_ref().map(Arc::downgrade),
            normalized,
            meta: CachedMeta::default(),
            missing_children: AtomicU8::new(0),
            canonicalized: OnceLock::new(),
            data: OnceLock::new(),
        });
        match shard.write().unwrap().entry(hash, |entry| &*entry.path == path, |entry| entry.hash) {
            Entry::Occupied(existing) => {
                crate::fsstats::cache("intern", "race");
                Arc::clone(existing.get())
            }
            Entry::Vacant(vacant) => {
                crate::fsstats::cache("intern", "miss");
                vacant.insert(Arc::clone(&entry));
                entry
            }
        }
    }

    fn metadata_path(&self, path: &str) -> Arc<CachedPath> {
        let entry = self.intern(path);
        let _ = crate::internal::root_length(path);
        if entry.normalized {
            return entry;
        }
        // Common::split_path normalizes metadata/read paths lexically. Realpath intentionally
        // interns the raw spelling instead, because symlink/.. has physical traversal semantics.
        let normalized = tspath::normalize_path(path);
        let root_len = tspath::get_root_length(&normalized);
        let normalized =
            if root_len == normalized.len() { normalized.as_str() } else { tspath::remove_trailing_directory_separator(&normalized) };
        self.intern(normalized)
    }

    fn followed_metadata(&self, path: &Arc<CachedPath>) -> Option<FileMode> {
        metadata_or_init(&path.meta.followed, "followed metadata", || {
            match path.link_metadata() {
                Some(mode) if mode.intersects(FileMode::Symlink) => {
                    // Linux O_PATH applies the same kernel symlink limit as stat. The macOS
                    // realpath walker accepts up to 255 links, so its successful target cannot
                    // answer a followed metadata probe whose kernel traversal would fail.
                    #[cfg(target_os = "linux")]
                    {
                        let target = self.canonicalize(path).and_then(|target| target.link_metadata());
                        if let Some(mode) = target.filter(|mode| !mode.intersects(FileMode::Symlink)) {
                            return Some(mode);
                        }
                    }
                    super::count_stat_call();
                    super::metadata_file_type(&path.path).ok()
                }
                other => other,
            }
        })
    }

    fn canonicalize(&self, path: &Arc<CachedPath>) -> Option<Arc<CachedPath>> {
        if let Some(canonical) = path.canonicalized.get() {
            crate::fsstats::cache("realpath", if canonical.is_some() { "hit-positive" } else { "hit-negative" });
            return canonical.as_ref().map(|canonical| canonical.target.upgrade().unwrap_or_else(|| self.intern(&canonical.spelling)));
        }
        // Do not hold a OnceLock initializer while walking symlinks: recursive paths and
        // concurrent callers may traverse one another. Publish the finished result afterward.
        #[cfg(target_os = "linux")]
        let resolved = super::native_realpath(&path.path);
        #[cfg(target_os = "macos")]
        let resolved = super::walk_symlinks_with(
            &path.path,
            |prefix| self.intern(prefix).link_metadata().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound)),
            |prefix| self.intern(prefix).read_link(),
        );
        let resolved = resolved.and_then(|path| super::abs(&path)).ok().map(|path| self.intern(&path));
        crate::fsstats::cache("realpath", if resolved.is_some() { "miss-positive" } else { "miss-negative" });
        let _ = path
            .canonicalized
            .set(resolved.as_ref().map(|target| CanonicalPath { target: Arc::downgrade(target), spelling: target.path.clone() }));
        resolved
    }
}

impl FS for ResolverFsCache {
    fn use_case_sensitive_file_names(&self) -> bool {
        super::fs().use_case_sensitive_file_names()
    }

    fn file_exists(&self, path: &str) -> bool {
        self.followed_metadata(&self.metadata_path(path)).is_some_and(|mode| !mode.is_dir())
    }

    fn directory_exists(&self, path: &str) -> bool {
        self.followed_metadata(&self.metadata_path(path)).is_some_and(FileMode::is_dir)
    }

    fn read_file(&self, path: &str) -> Option<String> {
        if !path.ends_with("/package.json") && !path.ends_with('/') && !path.contains('\\') {
            crate::fsstats::cache("read file", "bypass");
            return super::fs().read_file(path);
        }
        let path = self.metadata_path(path);
        if !path.path.ends_with("/package.json") {
            crate::fsstats::cache("read file", "bypass");
            return super::fs().read_file(&path.path);
        }
        // Like Oxc, keep a directory's package data on its path record. This is raw text only;
        // the resolver still owns package parsing, first-writer semantics, and trace messages.
        let directory = path.parent.as_ref().and_then(Weak::upgrade).unwrap_or_else(|| Arc::clone(&path));
        cached(&directory.data().package_json, "package.json", || super::fs().read_file(&path.path)).clone()
    }

    fn stat(&self, path: &str) -> Option<FileInfo> {
        let path = self.metadata_path(path);
        cached(&path.data().stat, "stat", || {
            let info = super::fs().stat(&path.path);
            // A full stat also supplies the self-contained followed classification.
            path.meta.followed.store(encode(info.as_ref().map(|info| info.mode)), Ordering::Relaxed);
            info
        })
        .clone()
    }

    fn get_accessible_entries(&self, path: &str) -> Entries {
        let path = self.metadata_path(path);
        cached(&path.data().entries, "directory entries", || super::fs().get_accessible_entries(&path.path)).clone()
    }

    fn realpath(&self, path: &str) -> String {
        let _ = crate::internal::root_length(path);
        self.canonicalize(&self.intern(path)).map_or_else(|| path.to_string(), |path| tspath::normalize_slashes(&path.path))
    }

    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        super::fs().write_file(path, data)
    }

    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        super::fs().append_file(path, data)
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        super::fs().remove(path)
    }

    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        super::fs().chtimes(path, a_time, m_time)
    }
}

#[cfg(test)]
#[path = "resolvercache_test.rs"]
mod tests;
