// Port of tsc/internal/api/requestfilesystem/{requestfilesystem.go,filechanges.go}: the filesystem a
// client sends with createSnapshot/updateSnapshot (`fileSystem` param). A `full` filesystem replaces
// the host; a `layer` overlays it, with `removedPaths` as negative lookups that block fallback to the
// layer below. Layers over a previous request filesystem are compacted into one path tree.
//
// Integration (core): the snapshot filesystem in tsrs_project must treat a request filesystem as Go's
// project.RebasableFileSystem (layerOverlayFileSystem rebases it over the overlay FS via
// `base_file_system` / `with_base_file_system`), and use it as a FileChangeExpander.

mod pathtree;

use std::sync::Arc;
use std::time::SystemTime;

use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;
use tsrs_core::tspath::{self, Path};
use tsrs_lsproto::DocumentUri;
use tsrs_project::{new_cached_file_handle, FileChangeExpander, FileChangeSummary, FileHandle, FileHandleSource, FsRef, LayeredFileSystem, OverlayMap};
use tsrs_vfs::{Entries, FileInfo, FileMode, FsError, FS};

#[cfg(test)]
use pathtree::ancestors;
use pathtree::{compose, equal_names, merge_entries, node_entries, path_contains, Entry, Fallback, Node, ReqDirectory, ReqFile, ReqSymlink};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Full,
    Layer,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct RequestDirectoryEntries {
    #[serde(default)]
    pub files: Option<Vec<String>>,
    #[serde(default)]
    pub directories: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct RequestSymlink {
    pub target: String,
    #[serde(default)]
    pub host: bool,
}

/// proto `RequestFileSystem` (JSON field names as in the pinned Go struct).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestFileSystemParams {
    pub kind: String,
    #[serde(default)]
    pub files: Option<FxHashMap<String, String>>,
    #[serde(default)]
    pub directories: Option<FxHashMap<String, RequestDirectoryEntries>>,
    #[serde(default)]
    pub symlinks: Option<FxHashMap<String, RequestSymlink>>,
    #[serde(default)]
    pub removed_paths: Option<Vec<String>>,
}

impl RequestFileSystemParams {
    /// Decodes `fileSystem` params with the pinned decoder's strictness: duplicate member names at any
    /// depth, invalid UTF-8 and unpaired surrogate escapes are errors (texts match jsontext). Use this
    /// (or `strictjson::validate` on the whole request) instead of plain serde, which keeps the last
    /// duplicate.
    pub fn from_json(data: &[u8]) -> Result<Option<RequestFileSystemParams>, String> {
        crate::strictjson::validate(data)?;
        serde_json::from_slice(data).map_err(|e| format!("json: {e}"))
    }
}

/// A snapshot filesystem as the API session sees it: either a request filesystem or anything else.
#[derive(Clone)]
pub enum AnyFs {
    Request(Arc<RequestFs>),
    Other(FsRef),
}

impl AnyFs {
    pub fn fs(&self) -> &dyn FS {
        match self {
            AnyFs::Request(r) => &**r,
            AnyFs::Other(f) => f.fs(),
        }
    }

    pub fn overlays(&self) -> Option<OverlayMap> {
        match self {
            AnyFs::Request(r) => Some(r.overlays()),
            AnyFs::Other(FsRef::Layered(l)) => Some(l.overlays()),
            AnyFs::Other(FsRef::Host(_)) => None,
        }
    }
}

/// HasFullFileSystem.
pub fn has_full_file_system(fs: &AnyFs) -> bool {
    matches!(fs, AnyFs::Request(r) if r.kind == Kind::Full)
}

#[derive(Clone)]
pub struct RequestFs {
    kind: Kind,
    base: FsRef,
    current_directory: String,
    case_sensitive: bool,
    paths: Arc<Node>,
}

struct Resolved {
    path: String,
    followed_symlink: bool,
    host: bool,
    ok: bool,
}

#[derive(Default)]
struct Lookup {
    path: String,
    info: Option<FileInfo>,
    /// Some when the lookup falls through to the base filesystem.
    base: bool,
    followed_symlink: bool,
    ok: bool,
}

/// NewForUpdate. `params == None` returns `base` unchanged. For a `layer`, file-change notifications
/// (changed/created/deleted, including symlink aliases and dropped overlays) are added to `changes`.
pub fn new_for_update(
    params: Option<&RequestFileSystemParams>,
    base: &AnyFs,
    current_directory: &str,
    changes: &mut FileChangeSummary,
) -> Result<AnyFs, String> {
    let Some(params) = params else { return Ok(base.clone()) };
    let kind = parse_kind(&params.kind)?;
    let base_fs = match (kind, base) {
        (Kind::Full, AnyFs::Request(r)) => AnyFs::Other(r.base.clone()),
        _ => base.clone(),
    };
    let placeholder = match &base_fs {
        AnyFs::Request(r) => r.base.clone(),
        AnyFs::Other(f) => f.clone(),
    };
    let mut fs = new_worker(params, kind, placeholder, base_fs.fs().use_case_sensitive_file_names(), current_directory)?;
    if let AnyFs::Request(r) = &base_fs {
        fs = fs.apply_to(r);
    }
    if kind == Kind::Layer {
        add_file_changes(changes, params, &base_fs, &fs, current_directory);
    }
    Ok(AnyFs::Request(Arc::new(fs)))
}

fn parse_kind(kind: &str) -> Result<Kind, String> {
    match kind {
        "full" => Ok(Kind::Full),
        "layer" => Ok(Kind::Layer),
        other => Err(format!("unknown request filesystem kind {}", serde_json::to_string(other).unwrap())),
    }
}

fn file_info(entry: &Entry) -> Option<FileInfo> {
    match entry {
        Entry::File(f) => Some(FileInfo {
            name: tspath::get_base_file_name(&f.file_name),
            size: f.content.len() as i64,
            mode: FileMode::from_bits_retain(0o444),
            mod_time: None,
        }),
        Entry::Directory(d) => Some(dir_info(&d.directory_name)),
        Entry::Symlink(_) => None,
    }
}

fn dir_info(name: &str) -> FileInfo {
    FileInfo { name: tspath::get_base_file_name(name), size: 0, mode: FileMode::Dir | FileMode::from_bits_retain(0o555), mod_time: None }
}

fn new_worker(params: &RequestFileSystemParams, kind: Kind, base: FsRef, case_sensitive: bool, cwd: &str) -> Result<RequestFs, String> {
    let mut result = RequestFs { kind, base, current_directory: cwd.to_string(), case_sensitive, paths: Arc::new(Node::default()) };
    let mut root = Node::default();
    result.register_directory(&mut root, cwd);

    let empty_files = FxHashMap::default();
    let files = params.files.as_ref().unwrap_or(&empty_files);
    let mut file_names: Vec<&String> = files.keys().collect();
    file_names.sort();
    for file_name in file_names {
        let content = &files[file_name];
        let absolute = result.to_absolute_path(file_name);
        let path = result.to_path(&absolute);
        let node = root.ensure(&path);
        if let Some(Entry::File(existing)) = &node.entry {
            return Err(format!("duplicate request filesystem file path {:?} and {:?}", existing.file_name, absolute));
        }
        node.entry = Some(Entry::File(Arc::new(ReqFile { file_name: absolute.clone(), content: content.clone() })));
        result.register_directory(&mut root, &tspath::get_directory_path(&absolute));
    }

    let mut listed = Vec::new();
    if let Some(directories) = &params.directories {
        let mut seen = FxHashSet::default();
        let mut names: Vec<&String> = directories.keys().collect();
        names.sort();
        for directory_name in names {
            let entries = &directories[directory_name];
            let absolute = result.to_absolute_path(directory_name);
            let path = result.to_path(&absolute);
            if !seen.insert(path.clone()) {
                return Err(format!("duplicate request filesystem directory path {absolute:?}"));
            }
            let node = root.ensure(&path);
            if !matches!(node.entry, Some(Entry::File(_))) {
                node.entry = Some(Entry::Directory(Arc::new(ReqDirectory {
                    directory_name: absolute.clone(),
                    listing: Some(Arc::new(Entries {
                        files: entries.files.clone().unwrap_or_default(),
                        directories: entries.directories.clone().unwrap_or_default(),
                        symlinks: None,
                    })),
                })));
            }
            result.register_directory(&mut root, &tspath::get_directory_path(&absolute));
            for child in entries.directories.iter().flatten() {
                listed.push(tspath::combine_paths(&absolute, &[child]));
            }
        }
    }

    if let Some(symlinks) = &params.symlinks {
        let mut names: Vec<&String> = symlinks.keys().collect();
        names.sort();
        for link_name in &names {
            let dir = tspath::get_directory_path(&result.to_absolute_path(link_name));
            result.register_directory(&mut root, &dir);
        }
        let mut seen: FxHashMap<Path, String> = FxHashMap::default();
        for link_name in names {
            let symlink = &symlinks[link_name];
            let absolute = result.to_absolute_path(link_name);
            let path = result.to_path(&absolute);
            if let Some(existing) = seen.get(&path) {
                return Err(format!("duplicate request filesystem symlink path {existing:?} and {absolute:?}"));
            }
            seen.insert(path.clone(), absolute.clone());
            let target_directory = tspath::get_directory_path(&absolute);
            let target = result.to_absolute_path_from(&symlink.target, &target_directory);
            let node = root.ensure(&path);
            if node.entry.is_none() {
                node.entry = Some(Entry::Symlink(Arc::new(ReqSymlink { link_name: absolute, target, host: symlink.host })));
            }
        }
    }
    for directory_name in listed {
        result.register_directory(&mut root, &directory_name);
    }
    for path in params.removed_paths.iter().flatten() {
        let p = result.to_path(&result.to_absolute_path(path));
        root.ensure(&p).fallback = Fallback::Missing;
    }
    result.paths = compose(None, Some(&Arc::new(root)), Fallback::Allowed, case_sensitive).expect("overlay is present");
    Ok(result)
}

impl RequestFs {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// RebasableFileSystem.BaseFileSystem.
    pub fn base_file_system(&self) -> &FsRef {
        &self.base
    }

    /// RebasableFileSystem.WithBaseFileSystem.
    pub fn with_base_file_system(&self, base: FsRef) -> Arc<RequestFs> {
        let mut clone = self.clone();
        clone.base = base;
        Arc::new(clone)
    }

    fn apply_to(mut self, base: &RequestFs) -> RequestFs {
        self.paths = compose(Some(&base.paths), Some(&self.paths), Fallback::Allowed, self.case_sensitive).expect("overlay is present");
        self.kind = base.kind;
        self.base = base.base.clone();
        self
    }

    fn base_fs(&self) -> &dyn FS {
        self.base.fs()
    }

    fn blocks_fallback(&self, path: &str) -> bool {
        self.paths.lookup(&self.to_path(path)).1 == Fallback::Missing
    }

    fn to_absolute_path(&self, path: &str) -> String {
        self.to_absolute_path_from(path, &self.current_directory)
    }

    fn to_absolute_path_from(&self, path: &str, cwd: &str) -> String {
        let absolute = tspath::get_normalized_absolute_path(path, cwd);
        if tspath::is_disk_path_root(&absolute) {
            return absolute;
        }
        tspath::remove_trailing_directory_separator(&absolute).to_string()
    }

    fn to_path(&self, path: &str) -> Path {
        tspath::to_path(path, &self.current_directory, self.case_sensitive)
    }

    fn register_directory(&self, root: &mut Node, directory_name: &str) {
        let mut name = self.to_absolute_path(directory_name);
        loop {
            let node = root.ensure(&self.to_path(&name));
            if node.entry.is_some() {
                return;
            }
            node.entry = Some(Entry::Directory(Arc::new(ReqDirectory { directory_name: name.clone(), listing: None })));
            let parent = tspath::get_directory_path(&name);
            if parent == name {
                return;
            }
            name = parent;
        }
    }

    fn resolve_path(&self, path: &str) -> Resolved {
        let mut result = Resolved { path: self.to_absolute_path(path), followed_symlink: false, host: false, ok: true };
        let mut seen = FxHashSet::default();
        loop {
            let canonical = self.to_path(&result.path);
            if self.paths.contains_file_ancestor(&canonical) {
                result.ok = false;
                return result;
            }
            let Some((match_path, symlink)) = self.paths.first_symlink(&canonical) else {
                result.host = self.is_host_path(&result.path);
                return result;
            };
            if !seen.insert(match_path) {
                result.ok = false;
                return result;
            }
            result.followed_symlink = true;
            let Some(suffix) = tspath::trim_file_path_prefix(&result.path, &symlink.link_name, self.case_sensitive) else {
                result.ok = false;
                return result;
            };
            let suffix = suffix.strip_prefix('/').unwrap_or(suffix).to_string();
            result.path = self.to_absolute_path(&tspath::combine_paths(&symlink.target, &[&suffix]));
            if symlink.host {
                result.host = true;
                return result;
            }
        }
    }

    fn is_host_path(&self, path: &str) -> bool {
        let canonical = self.to_path(path);
        self.paths.symlinks().iter().any(|(_, s)| s.host && path_contains(&self.to_path(&s.target), &canonical))
    }

    /// aliasesForPath: every symlinked spelling of `path` that resolves.
    pub fn aliases_for_path(&self, path: &str) -> Vec<String> {
        let symlinks = self.paths.symlinks();
        let mut seen: FxHashSet<Path> = FxHashSet::default();
        seen.insert(self.to_path(path));
        let mut queue = std::collections::VecDeque::from([self.to_absolute_path(path)]);
        let mut aliases = Vec::new();
        while let Some(candidate) = queue.pop_front() {
            for (_, symlink) in &symlinks {
                let Some(suffix) = tspath::trim_file_path_prefix(&candidate, &symlink.target, self.case_sensitive) else { continue };
                if !suffix.is_empty() && !tspath::has_trailing_directory_separator(&symlink.target) && !suffix.starts_with('/') {
                    continue;
                }
                let suffix = suffix.strip_prefix('/').unwrap_or(suffix);
                let alias = self.to_absolute_path(&tspath::combine_paths(&symlink.link_name, &[suffix]));
                let alias_path = self.to_path(&alias);
                if seen.contains(&alias_path) {
                    continue;
                }
                if !self.resolve_path(&alias).ok {
                    continue;
                }
                seen.insert(alias_path);
                aliases.push(alias.clone());
                queue.push_back(alias);
            }
        }
        aliases
    }

    fn local_path_info(&self, path: &str) -> (Option<FileInfo>, Fallback) {
        let (node, fallback) = self.paths.lookup(&self.to_path(path));
        (node.and_then(|n| n.entry.as_ref()).and_then(file_info), fallback)
    }

    fn local_file(&self, path: &str) -> Option<Arc<ReqFile>> {
        match self.paths.lookup(&self.to_path(path)).0.and_then(|n| n.entry.as_ref()) {
            Some(Entry::File(f)) => Some(f.clone()),
            _ => None,
        }
    }

    fn lookup_path(&self, path: &str) -> Lookup {
        let absolute = self.to_absolute_path(path);
        let (info, path_fallback) = self.local_path_info(&absolute);
        if info.is_some() {
            return Lookup { path: absolute, info, ok: true, ..Default::default() };
        }
        if path_fallback == Fallback::Missing {
            return Lookup::default();
        }
        let resolved = self.resolve_path(path);
        if !resolved.ok {
            return Lookup::default();
        }
        let mut result = Lookup { path: resolved.path.clone(), followed_symlink: resolved.followed_symlink, ok: true, ..Default::default() };
        let (resolved_info, resolved_fallback) = self.local_path_info(&resolved.path);
        if !resolved.host && resolved_info.is_some() {
            result.info = resolved_info;
        } else if resolved.host || self.kind == Kind::Layer {
            if resolved_fallback == Fallback::Missing {
                return Lookup::default();
            }
            result.base = true;
        }
        result
    }

    fn mutation_path(&self, path: &str) -> Option<String> {
        if self.kind != Kind::Layer {
            return None;
        }
        let resolved = self.resolve_path(path);
        resolved.ok.then_some(resolved.path)
    }

    fn filter_local_entries(&self, directory_name: &str, entries: Entries) -> Entries {
        let mut result = entries;
        let keep = |name: &String| {
            let file_name = tspath::combine_paths(directory_name, &[name]);
            if self.local_path_info(&file_name).0.is_some() {
                return true;
            }
            !self.blocks_fallback(&file_name)
        };
        result.files.retain(keep);
        result.directories.retain(keep);
        if let Some(links) = &mut result.symlinks {
            links.retain(keep);
        }
        result
    }

    fn get_local_entries(&self, directory_name: &str) -> (Entries, bool) {
        let (node, _) = self.paths.lookup(&self.to_path(directory_name));
        let (entries, _) = node_entries(node);
        let explicit = matches!(node.and_then(|n| n.entry.as_ref()), Some(Entry::Directory(d)) if d.listing.is_some());
        (entries, explicit)
    }

    fn remove_entries(&self, directory_name: &str, entries: Entries) -> Entries {
        let mut result = entries;
        let keep = |name: &String| !self.blocks_fallback(&tspath::combine_paths(directory_name, &[name]));
        result.files.retain(keep);
        result.directories.retain(keep);
        if let Some(links) = &mut result.symlinks {
            links.retain(keep);
        }
        result
    }

    fn add_symlink_entries(&self, directory_name: &str, entries: Entries) -> Entries {
        let mut result = entries;
        let mut result_links = result.symlinks.take().unwrap_or_default();
        let (node, _) = self.paths.lookup(&self.to_path(directory_name));
        let mut links: Vec<(Path, Arc<ReqSymlink>)> = Vec::new();
        if let Some(node) = node {
            for (path, child) in &node.children {
                if let Some(Entry::Symlink(s)) = &child.entry {
                    links.push((path.clone(), s.clone()));
                }
            }
        }
        links.sort_by(|a, b| a.0.cmp(&b.0));
        let eq = |a: &str, b: &str| equal_names(a, b, self.case_sensitive);
        for (_, symlink) in &links {
            let name = tspath::get_base_file_name(&symlink.link_name);
            result.files.retain(|c| !eq(c, &name));
            result.directories.retain(|c| !eq(c, &name));
            result_links.retain(|c| !eq(c, &name));
            if self.directory_exists(&symlink.link_name) {
                result.directories.push(name.clone());
                result_links.insert(name);
            } else if self.file_exists(&symlink.link_name) {
                result.files.push(name.clone());
                result_links.insert(name);
            }
        }
        result.symlinks = Some(result_links);
        if !links.is_empty() {
            result.files.sort();
            result.directories.sort();
        }
        result
    }
}

impl FS for RequestFs {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.case_sensitive
    }

    fn file_exists(&self, path: &str) -> bool {
        let lookup = self.lookup_path(path);
        if !lookup.ok || lookup.info.as_ref().is_some_and(|i| i.is_dir()) {
            return false;
        }
        lookup.info.is_some() || lookup.base && self.base_fs().file_exists(&lookup.path)
    }

    fn read_file(&self, path: &str) -> Option<String> {
        let lookup = self.lookup_path(path);
        if !lookup.ok || lookup.info.as_ref().is_some_and(|i| i.is_dir()) {
            return None;
        }
        if lookup.base {
            return self.base_fs().read_file(&lookup.path);
        }
        self.local_file(&lookup.path).map(|f| f.content.clone())
    }

    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        match self.mutation_path(path) {
            Some(p) => self.base_fs().write_file(&p, data),
            None => Err(FsError::Invalid.to_string()),
        }
    }

    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        match self.mutation_path(path) {
            Some(p) => self.base_fs().append_file(&p, data),
            None => Err(FsError::Invalid.to_string()),
        }
    }

    fn remove(&self, path: &str) -> Result<(), String> {
        match self.mutation_path(path) {
            Some(p) => self.base_fs().remove(&p),
            None => Err(FsError::Invalid.to_string()),
        }
    }

    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        match self.mutation_path(path) {
            Some(p) => self.base_fs().chtimes(&p, a_time, m_time),
            None => Err(FsError::Invalid.to_string()),
        }
    }

    fn directory_exists(&self, path: &str) -> bool {
        let lookup = self.lookup_path(path);
        if !lookup.ok || lookup.info.as_ref().is_some_and(|i| !i.is_dir()) {
            return false;
        }
        lookup.info.is_some() || lookup.base && self.base_fs().directory_exists(&lookup.path)
    }

    fn get_accessible_entries(&self, directory_name: &str) -> Entries {
        let lookup = self.lookup_path(directory_name);
        if !lookup.ok || lookup.info.as_ref().is_some_and(|i| !i.is_dir()) {
            return Entries { symlinks: Some(FxHashSet::default()), ..Default::default() };
        }
        let result = if lookup.base {
            self.remove_entries(&lookup.path, self.base_fs().get_accessible_entries(&lookup.path))
        } else {
            let (local, explicit) = self.get_local_entries(&lookup.path);
            let mut result = local.clone();
            if self.kind == Kind::Layer && !explicit && !self.blocks_fallback(directory_name) && !self.blocks_fallback(&lookup.path) {
                result = self.remove_entries(&lookup.path, self.base_fs().get_accessible_entries(&lookup.path));
                result = merge_entries(&result, &local, self.case_sensitive);
            }
            self.add_symlink_entries(&lookup.path, result)
        };
        self.filter_local_entries(directory_name, result)
    }

    fn stat(&self, path: &str) -> Option<FileInfo> {
        let lookup = self.lookup_path(path);
        if !lookup.ok {
            return None;
        }
        if lookup.base {
            let fs = self.base_fs();
            if let Some(info) = fs.stat(&lookup.path) {
                return Some(info);
            }
            if fs.directory_exists(&lookup.path) {
                return Some(dir_info(&lookup.path));
            }
            if fs.file_exists(&lookup.path) {
                return Some(FileInfo {
                    name: tspath::get_base_file_name(&lookup.path),
                    size: 0,
                    mode: FileMode::from_bits_retain(0o444),
                    mod_time: None,
                });
            }
            return None;
        }
        lookup.info
    }

    fn realpath(&self, path: &str) -> String {
        let lookup = self.lookup_path(path);
        if !lookup.ok {
            return path.to_string();
        }
        if lookup.base {
            return self.base_fs().realpath(&lookup.path);
        }
        if lookup.info.is_some() || !lookup.followed_symlink {
            return lookup.path;
        }
        path.to_string()
    }
}

impl FileHandleSource for RequestFs {
    fn get_file(&self, file_name: &str) -> Option<Arc<dyn FileHandle>> {
        self.get_file_by_path(file_name, &self.to_path(file_name))
    }

    fn get_file_by_path(&self, file_name: &str, _path: &Path) -> Option<Arc<dyn FileHandle>> {
        let lookup = self.lookup_path(file_name);
        if !lookup.ok || lookup.info.as_ref().is_some_and(|i| i.is_dir()) {
            return None;
        }
        if lookup.base {
            if let FsRef::Layered(source) = &self.base {
                return source.get_file(&lookup.path);
            }
            return self.base_fs().read_file(&lookup.path).map(|content| new_cached_file_handle(file_name, &content));
        }
        self.local_file(&lookup.path).map(|f| new_cached_file_handle(file_name, &f.content))
    }
}

impl LayeredFileSystem for RequestFs {
    /// Overlays of the base that this filesystem still exposes at the same path.
    fn overlays(&self) -> OverlayMap {
        let FsRef::Layered(base) = &self.base else { return OverlayMap::default() };
        let mut result = FxHashMap::default();
        for (path, overlay) in base.overlays().iter() {
            let lookup = self.lookup_path(overlay.file_name());
            if !lookup.base || self.to_path(&lookup.path) != *path {
                continue;
            }
            result.insert(path.clone(), overlay.clone());
        }
        Arc::new(result)
    }

    fn as_file_change_expander(&self) -> Option<&dyn FileChangeExpander> {
        Some(self)
    }
}

fn uri(file_name: &str) -> DocumentUri {
    tsrs_ls::lsconv::file_name_to_document_uri(file_name)
}

impl FileChangeExpander for RequestFs {
    fn expand_file_changes(&self, mut summary: FileChangeSummary) -> FileChangeSummary {
        let expand = |uris: &mut tsrs_core::collections::Set<DocumentUri>| {
            let mut keys: Vec<DocumentUri> = uris.keys().iter().cloned().collect();
            keys.sort_by(|a, b| a.0.cmp(&b.0));
            for key in keys {
                for alias in self.aliases_for_path(&key.file_name()) {
                    uris.add(uri(&alias));
                }
            }
        };
        expand(&mut summary.changed);
        expand(&mut summary.created);
        expand(&mut summary.deleted);
        summary
    }
}

fn sorted(mut keys: Vec<&String>) -> Vec<&String> {
    keys.sort();
    keys
}

/// filechanges.go addFileChanges.
fn add_file_changes(summary: &mut FileChangeSummary, request: &RequestFileSystemParams, base: &AnyFs, fs: &RequestFs, cwd: &str) {
    let base_fs = base.fs();
    let case_sensitive = base_fs.use_case_sensitive_file_names();
    let to_path = |f: &str| tspath::to_path(f, cwd, case_sensitive);
    let base_request = match base {
        AnyFs::Request(r) => Some(r.clone()),
        AnyFs::Other(_) => None,
    };
    let add_change = |summary: &mut FileChangeSummary, file_name: &str, deleted: bool| {
        let u = uri(file_name);
        if deleted {
            if base_fs.file_exists(file_name) || base_fs.directory_exists(file_name) {
                summary.deleted.add(u);
            }
            return;
        }
        if base_fs.file_exists(file_name) {
            summary.changed.add(u);
        } else {
            summary.created.add(u);
        }
    };
    let add_change_and_aliases = |summary: &mut FileChangeSummary, file_name: &str, deleted: bool| {
        add_change(summary, file_name, deleted);
        if let Some(r) = &base_request {
            for alias in r.aliases_for_path(file_name) {
                add_change(summary, &alias, deleted);
            }
        }
    };
    let mut overlay_files = FxHashSet::default();
    if let Some(files) = &request.files {
        for file_name in sorted(files.keys().collect()) {
            let absolute = tspath::get_normalized_absolute_path(file_name, cwd);
            overlay_files.insert(to_path(&absolute));
            add_change_and_aliases(summary, &absolute, false);
        }
    }
    for removed in request.removed_paths.iter().flatten() {
        let absolute = tspath::get_normalized_absolute_path(removed, cwd);
        if overlay_files.contains(&to_path(&absolute)) {
            continue;
        }
        add_change_and_aliases(summary, &absolute, true);
    }
    let add_replacement = |summary: &mut FileChangeSummary, path: &str| {
        let absolute = tspath::get_normalized_absolute_path(path, cwd);
        add_change_and_aliases(summary, &absolute, true);
        summary.created.add(uri(&absolute));
        if let Some(r) = &base_request {
            for alias in r.aliases_for_path(&absolute) {
                summary.created.add(uri(&alias));
            }
        }
    };
    if let Some(directories) = &request.directories {
        for d in sorted(directories.keys().collect()) {
            add_replacement(summary, d);
        }
    }
    if let Some(symlinks) = &request.symlinks {
        for l in sorted(symlinks.keys().collect()) {
            add_replacement(summary, l);
        }
    }
    if let Some(base_overlays) = base.overlays() {
        let overlays = fs.overlays();
        for (path, overlay) in base_overlays.iter() {
            if overlays.contains_key(path) {
                continue;
            }
            let u = uri(overlay.file_name());
            if summary.closed.has(&u) {
                continue;
            }
            summary.created.delete(&u);
            if fs.file_exists(overlay.file_name()) {
                summary.deleted.delete(&u);
                summary.changed.add(u);
            } else {
                summary.changed.delete(&u);
                summary.deleted.add(u);
            }
        }
    }
    if summary.changed.len() + summary.created.len() + summary.deleted.len() > 0 {
        summary.includes_watch_change_outside_node_modules = true;
    }
}

#[cfg(test)]
pub(crate) fn debug_ancestors(path: &str) -> Vec<String> {
    ancestors(path).into_iter().map(|p| p.to_string()).collect()
}
