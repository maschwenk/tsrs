// Port of tsc/internal/api/requestfilesystem (requestfilesystem.go, pathtree.go, filechanges.go): the
// `fileSystem` of createSnapshot / updateSnapshot. A `full` filesystem is canonical and total; a `layer` is
// checked before the session host filesystem. Layers over request filesystems are compacted eagerly.
//
// Differences from Go (documented in docs/NODE_API.md): the request filesystem enters the project snapshot as
// a plain host filesystem, so LSP-overlay rebasing (`WithBaseFileSystem` / `Overlays`) and alias expansion of
// client `fileNotifications` (`ExpandFileChanges`) are not applied; standalone API sessions have no overlays.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use rustc_hash::FxHashSet;
use tsrs_core::json::Value;
use tsrs_core::tspath::{self, Path};
use tsrs_project::FileChangeSummary;
use tsrs_vfs::{Entries, FileInfo, FileMode, FS};

use crate::wire::Params;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Full,
    Layer,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Fallback {
    #[default]
    Inherit,
    Allowed,
    Missing,
}

#[derive(Clone, Debug)]
enum Entry {
    File { file_name: String, content: Arc<str> },
    Symlink(Symlink),
    Directory { directory_name: String, listing: Option<Entries> },
}

#[derive(Clone, Debug)]
struct Symlink {
    link_name: String,
    target: String,
    host: bool,
}

impl Entry {
    fn info(&self) -> Option<FileInfo> {
        match self {
            Entry::File { file_name, content } => Some(FileInfo {
                name: tspath::get_base_file_name(file_name),
                size: content.len() as i64,
                mode: FileMode::from_bits_retain(0o444),
                mod_time: None,
            }),
            Entry::Directory { directory_name, .. } => Some(dir_info(directory_name)),
            Entry::Symlink(_) => None,
        }
    }
}

fn dir_info(name: &str) -> FileInfo {
    FileInfo { name: tspath::get_base_file_name(name), size: 0, mode: FileMode::Dir | FileMode::from_bits_retain(0o555), mod_time: None }
}

#[derive(Clone, Debug, Default)]
struct Node {
    entry: Option<Arc<Entry>>,
    fallback: Fallback,
    children: HashMap<Path, Arc<Node>>,
    has_symlinks: bool,
}

fn ancestors(path: &str) -> Vec<Path> {
    let mut paths = Vec::new();
    let mut p = path.to_string();
    loop {
        paths.push(Path::new(p.clone()));
        let parent = tspath::get_directory_path(&p).to_string();
        if parent == p {
            break;
        }
        p = parent;
    }
    paths.reverse();
    paths
}

impl Node {
    fn replaces_subtree(&self) -> bool {
        matches!(self.entry.as_deref(), Some(Entry::File { .. } | Entry::Symlink(_)))
    }

    fn ensure(&mut self, path: &Path) -> &mut Node {
        let mut node = self;
        for a in ancestors(path) {
            let child = node.children.entry(a).or_default();
            node = Arc::make_mut(child);
        }
        node
    }

    fn lookup(&self, path: &Path) -> (Option<&Node>, Fallback) {
        let mut fallback = Fallback::Inherit;
        let mut node = Some(self);
        for a in ancestors(path) {
            let Some(n) = node else { break };
            if n.fallback != Fallback::Inherit {
                fallback = n.fallback;
            }
            node = n.children.get(&a).map(|c| &**c);
        }
        if let Some(n) = node {
            if n.fallback != Fallback::Inherit {
                fallback = n.fallback;
            }
        }
        (node, fallback)
    }

    fn walk_symlinks(&self, visit: &mut dyn FnMut(&Symlink)) {
        if !self.has_symlinks {
            return;
        }
        for child in self.children.values() {
            if let Some(Entry::Symlink(s)) = child.entry.as_deref() {
                visit(s);
            }
            child.walk_symlinks(visit);
        }
    }

    fn entries(node: Option<&Node>) -> Option<Entries> {
        let node = node?;
        let Some(Entry::Directory { listing, .. }) = node.entry.as_deref() else { return None };
        if let Some(l) = listing {
            return Some(l.clone());
        }
        let mut e = Entries::default();
        for child in node.children.values() {
            match child.entry.as_deref() {
                Some(Entry::File { file_name, .. }) => e.files.push(tspath::get_base_file_name(file_name)),
                Some(Entry::Directory { directory_name, .. }) => e.directories.push(tspath::get_base_file_name(directory_name)),
                _ => {}
            }
        }
        e.files.sort();
        e.directories.sort();
        Some(e)
    }

    fn first_symlink(&self, path: &Path) -> Option<&Symlink> {
        let mut node = Some(self);
        for a in ancestors(path) {
            let n = node?;
            node = n.children.get(&a).map(|c| &**c);
            if let Some(Entry::Symlink(s)) = node.and_then(|n| n.entry.as_deref()) {
                return Some(s);
            }
        }
        None
    }

    fn contains_file_ancestor(&self, path: &Path) -> bool {
        let mut node = Some(self);
        for a in ancestors(path) {
            let Some(n) = node else { return false };
            node = n.children.get(&a).map(|c| &**c);
            if &a != path {
                if let Some(Entry::File { .. }) = node.and_then(|n| n.entry.as_deref()) {
                    return true;
                }
            }
        }
        false
    }
}

fn path_contains(parent: &str, path: &str) -> bool {
    path == parent || path.starts_with(&tspath::ensure_trailing_directory_separator(parent))
}

fn equal_names(a: &str, b: &str, cs: bool) -> bool {
    tspath::get_canonical_file_name(a, cs) == tspath::get_canonical_file_name(b, cs)
}

fn merge_entries(base: &Entries, overlay: &Entries, cs: bool) -> Entries {
    let mut r = base.clone();
    let mut symlinks = r.symlinks.take().unwrap_or_default();
    let delete_symlink = |s: &mut FxHashSet<String>, name: &str| s.retain(|e| !equal_names(e, name, cs));
    for name in &overlay.files {
        r.directories.retain(|v| !equal_names(v, name, cs));
        if !r.files.iter().any(|v| equal_names(v, name, cs)) {
            r.files.push(name.clone());
        }
        delete_symlink(&mut symlinks, name);
    }
    for name in &overlay.directories {
        r.files.retain(|v| !equal_names(v, name, cs));
        if !r.directories.iter().any(|v| equal_names(v, name, cs)) {
            r.directories.push(name.clone());
        }
        delete_symlink(&mut symlinks, name);
    }
    if let Some(o) = &overlay.symlinks {
        symlinks.extend(o.iter().cloned());
    }
    r.symlinks = Some(symlinks);
    r.files.sort();
    r.directories.sort();
    r
}

/// Go `composeRequestPaths`.
fn compose(base: Option<&Arc<Node>>, overlay: Option<&Arc<Node>>, mut fallback: Fallback, cs: bool) -> Option<Arc<Node>> {
    let Some(overlay) = overlay else { return base.cloned() };
    let mut base = base;
    if overlay.fallback != Fallback::Inherit {
        fallback = overlay.fallback;
        base = None;
    }
    if overlay.replaces_subtree() {
        base = None;
    }
    let mut result: Node = base.map(|b| (**b).clone()).unwrap_or_default();
    if overlay.fallback != Fallback::Inherit || overlay.replaces_subtree() {
        result.fallback = fallback;
    }
    let previous_listing = match result.entry.as_deref() {
        Some(Entry::Directory { listing, .. }) => Some(listing.clone()),
        _ => None,
    };
    let overlay_dir = match overlay.entry.as_deref() {
        Some(Entry::Directory { directory_name, listing }) => Some((directory_name.clone(), listing.clone())),
        _ => None,
    };
    if let Some(entry) = &overlay.entry {
        result.entry = Some(entry.clone());
        if let (Some((name, None)), Some(prev)) = (&overlay_dir, &previous_listing) {
            result.entry = Some(Arc::new(Entry::Directory { directory_name: name.clone(), listing: prev.clone() }));
        }
    }
    for (path, child) in &overlay.children {
        let composed = compose(result.children.get(path), Some(child), fallback, cs);
        if let Some(c) = composed {
            result.children.insert(path.clone(), c);
        }
    }
    let overlay_has_listing = matches!(&overlay_dir, Some((_, Some(_))));
    if let Some(Entry::Directory { directory_name, listing: Some(listing) }) = result.entry.as_deref() {
        if !overlay_has_listing {
            let mut entries = listing.clone();
            for (path, child) in &overlay.children {
                let name = tspath::get_base_file_name(path);
                if child.fallback == Fallback::Missing || child.replaces_subtree() {
                    entries.files.retain(|e| !equal_names(e, &name, cs));
                    entries.directories.retain(|e| !equal_names(e, &name, cs));
                    if let Some(s) = &mut entries.symlinks {
                        s.retain(|e| !equal_names(e, &name, cs));
                    }
                }
                match child.entry.as_deref() {
                    Some(Entry::File { file_name, .. }) => {
                        entries = merge_entries(&entries, &Entries { files: vec![tspath::get_base_file_name(file_name)], ..Default::default() }, cs)
                    }
                    Some(Entry::Directory { directory_name, .. }) => {
                        entries = merge_entries(&entries, &Entries { directories: vec![tspath::get_base_file_name(directory_name)], ..Default::default() }, cs)
                    }
                    _ => {}
                }
            }
            let name = directory_name.clone();
            result.entry = Some(Arc::new(Entry::Directory { directory_name: name, listing: Some(entries) }));
        }
    }
    result.has_symlinks = matches!(result.entry.as_deref(), Some(Entry::Symlink(_))) || result.children.values().any(|c| c.has_symlinks);
    Some(Arc::new(result))
}

/// Parsed `RequestFileSystem` params.
pub struct RequestParams {
    pub kind: Kind,
    files: Vec<(String, String)>,
    directories: Vec<(String, Entries)>,
    symlinks: Vec<(String, String, bool)>,
    removed_paths: Vec<String>,
}

impl RequestParams {
    pub fn parse(v: &Value) -> Result<RequestParams, String> {
        let p = Params(v);
        let o = p.object().map_err(|e| e.message)?;
        let kind = match o.get("kind") {
            Some(Value::String(k)) if k == "full" => Kind::Full,
            Some(Value::String(k)) if k == "layer" => Kind::Layer,
            Some(Value::String(k)) => return Err(format!("unknown request filesystem kind {k:?}")),
            _ => return Err("unknown request filesystem kind \"\"".to_string()),
        };
        let str_list = |v: &Value, what: &str| -> Result<Vec<String>, String> {
            match v {
                Value::Null => Ok(Vec::new()),
                Value::Array(a) => a.iter().map(|x| if let Value::String(s) = x { Ok(s.clone()) } else { Err(format!("{what} must contain strings")) }).collect(),
                _ => Err(format!("{what} must be an array")),
            }
        };
        let mut files = Vec::new();
        match p.get("files") {
            Value::Null => {}
            Value::Object(m) => {
                for (k, v) in m.iter() {
                    match v {
                        Value::String(c) => files.push((k.clone(), c.clone())),
                        _ => return Err("fileSystem.files values must be strings".to_string()),
                    }
                }
            }
            _ => return Err("fileSystem.files must be an object".to_string()),
        }
        let mut directories = Vec::new();
        match p.get("directories") {
            Value::Null => {}
            Value::Object(m) => {
                for (k, v) in m.iter() {
                    let d = Params(v);
                    directories.push((
                        k.clone(),
                        Entries { files: str_list(d.get("files"), "directory files")?, directories: str_list(d.get("directories"), "directory directories")?, symlinks: None },
                    ));
                }
            }
            _ => return Err("fileSystem.directories must be an object".to_string()),
        }
        let mut symlinks = Vec::new();
        match p.get("symlinks") {
            Value::Null => {}
            Value::Object(m) => {
                for (k, v) in m.iter() {
                    let s = Params(v);
                    let target = match s.get("target") {
                        Value::String(t) => t.clone(),
                        _ => return Err("symlink target must be a string".to_string()),
                    };
                    symlinks.push((k.clone(), target, matches!(s.get("host"), Value::Bool(true))));
                }
            }
            _ => return Err("fileSystem.symlinks must be an object".to_string()),
        }
        Ok(RequestParams { kind, files, directories, symlinks, removed_paths: str_list(p.get("removedPaths"), "removedPaths")? })
    }
}

/// Go `requestFileSystem`.
#[derive(Clone)]
pub struct RequestFileSystem {
    pub kind: Kind,
    base: Arc<dyn FS>,
    current_directory: String,
    cs: bool,
    paths: Arc<Node>,
}

impl std::fmt::Debug for RequestFileSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RequestFileSystem({:?})", self.kind)
    }
}

struct Lookup {
    path: String,
    info: Option<FileInfo>,
    entry: Option<Arc<Entry>>,
    use_base: bool,
    followed_symlink: bool,
}

struct Resolved {
    path: String,
    followed_symlink: bool,
    host: bool,
}

/// Go `NewForUpdate`. `base_request` is the base snapshot's request filesystem, if any.
pub fn new_for_update(
    params: &RequestParams,
    host: Arc<dyn FS>,
    base_request: Option<&RequestFileSystem>,
    current_directory: &str,
    file_changes: &mut FileChangeSummary,
) -> Result<RequestFileSystem, String> {
    // Full filesystems never build on a previous request filesystem; layers compact onto it.
    let layered_base = if params.kind == Kind::Full { None } else { base_request };
    let base_fs: Arc<dyn FS> = host;
    let mut fs = RequestFileSystem::new_worker(params, base_fs, current_directory)?;
    if let Some(b) = layered_base {
        fs.paths = compose(Some(&b.paths), Some(&fs.paths), Fallback::Allowed, fs.cs).unwrap_or_default();
        fs.kind = b.kind;
        fs.base = b.base.clone();
    }
    if params.kind == Kind::Layer {
        add_file_changes(file_changes, params, layered_base, &fs, current_directory);
    }
    Ok(fs)
}

impl RequestFileSystem {
    fn new_worker(params: &RequestParams, base: Arc<dyn FS>, current_directory: &str) -> Result<RequestFileSystem, String> {
        let cs = base.use_case_sensitive_file_names();
        let mut r = RequestFileSystem { kind: params.kind, base, current_directory: current_directory.to_string(), cs, paths: Arc::new(Node::default()) };
        let mut root = Node::default();
        r.register_directory(&mut root, current_directory);
        let mut files: Vec<&(String, String)> = params.files.iter().collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (file_name, content) in files {
            let abs = r.to_absolute_path(file_name);
            let path = r.to_path(&abs);
            let node = root.ensure(&path);
            if let Some(Entry::File { file_name: existing, .. }) = node.entry.as_deref() {
                return Err(format!("duplicate request filesystem file path {existing:?} and {abs:?}"));
            }
            node.entry = Some(Arc::new(Entry::File { file_name: abs.clone(), content: Arc::from(content.as_str()) }));
            r.register_directory(&mut root, &tspath::get_directory_path(&abs));
        }
        let mut seen_dirs = FxHashSet::default();
        let mut listed = Vec::new();
        for (dir_name, entries) in &params.directories {
            let abs = r.to_absolute_path(dir_name);
            let path = r.to_path(&abs);
            if !seen_dirs.insert(path.clone()) {
                return Err(format!("duplicate request filesystem directory path {abs:?}"));
            }
            let node = root.ensure(&path);
            if !matches!(node.entry.as_deref(), Some(Entry::File { .. })) {
                node.entry = Some(Arc::new(Entry::Directory {
                    directory_name: abs.clone(),
                    listing: Some(Entries { files: entries.files.clone(), directories: entries.directories.clone(), symlinks: None }),
                }));
            }
            r.register_directory(&mut root, &tspath::get_directory_path(&abs));
            for child in &entries.directories {
                listed.push(tspath::combine_paths(&abs, &[child]));
            }
        }
        for (link, _, _) in &params.symlinks {
            let abs = r.to_absolute_path(link);
            r.register_directory(&mut root, &tspath::get_directory_path(&abs));
        }
        let mut seen_links: HashMap<Path, String> = HashMap::new();
        for (link, target, host) in &params.symlinks {
            let abs = r.to_absolute_path(link);
            let path = r.to_path(&abs);
            if let Some(existing) = seen_links.get(&path) {
                return Err(format!("duplicate request filesystem symlink path {existing:?} and {abs:?}"));
            }
            seen_links.insert(path.clone(), abs.clone());
            let abs_target = r.to_absolute_path_from(target, &tspath::get_directory_path(&abs));
            let node = root.ensure(&path);
            if node.entry.is_none() {
                node.entry = Some(Arc::new(Entry::Symlink(Symlink { link_name: abs, target: abs_target, host: *host })));
            }
        }
        for d in listed {
            r.register_directory(&mut root, &d);
        }
        for p in &params.removed_paths {
            let path = r.to_path(&r.to_absolute_path(p));
            root.ensure(&path).fallback = Fallback::Missing;
        }
        r.paths = compose(None, Some(&Arc::new(root)), Fallback::Allowed, cs).unwrap_or_default();
        Ok(r)
    }

    pub fn is_full(&self) -> bool {
        self.kind == Kind::Full
    }

    fn to_absolute_path(&self, path: &str) -> String {
        self.to_absolute_path_from(path, &self.current_directory)
    }

    fn to_absolute_path_from(&self, path: &str, cwd: &str) -> String {
        let abs = tspath::get_normalized_absolute_path(path, cwd);
        if tspath::is_disk_path_root(&abs) {
            abs
        } else {
            tspath::remove_trailing_directory_separator(&abs).to_string()
        }
    }

    fn to_path(&self, path: &str) -> Path {
        tspath::to_path(path, &self.current_directory, self.cs)
    }

    fn register_directory(&self, root: &mut Node, directory_name: &str) {
        let mut name = self.to_absolute_path(directory_name);
        loop {
            let node = root.ensure(&self.to_path(&name));
            if node.entry.is_some() {
                return;
            }
            node.entry = Some(Arc::new(Entry::Directory { directory_name: name.clone(), listing: None }));
            let parent = tspath::get_directory_path(&name).to_string();
            if parent == name {
                return;
            }
            name = parent;
        }
    }

    fn blocks_fallback(&self, path: &str) -> bool {
        self.paths.lookup(&self.to_path(path)).1 == Fallback::Missing
    }

    fn resolve_path(&self, path: &str) -> Option<Resolved> {
        let mut r = Resolved { path: self.to_absolute_path(path), followed_symlink: false, host: false };
        let mut seen = FxHashSet::default();
        loop {
            let canonical = self.to_path(&r.path);
            if self.paths.contains_file_ancestor(&canonical) {
                return None;
            }
            let Some(link) = self.paths.first_symlink(&canonical) else {
                r.host = self.is_host_path(&r.path);
                return Some(r);
            };
            if !seen.insert(self.to_path(&link.link_name)) {
                return None;
            }
            r.followed_symlink = true;
            let suffix = tspath::trim_file_path_prefix(&r.path, &link.link_name, self.cs)?;
            r.path = self.to_absolute_path(&tspath::combine_paths(&link.target, &[suffix.trim_start_matches('/')]));
            if link.host {
                r.host = true;
                return Some(r);
            }
        }
    }

    fn is_host_path(&self, path: &str) -> bool {
        let canonical = self.to_path(path);
        let mut found = false;
        self.paths.walk_symlinks(&mut |s| {
            if s.host && path_contains(&self.to_path(&s.target), &canonical) {
                found = true;
            }
        });
        found
    }

    fn aliases_for_path(&self, path: &str) -> Vec<String> {
        let mut symlinks = Vec::new();
        self.paths.walk_symlinks(&mut |s| symlinks.push(s.clone()));
        let mut seen = FxHashSet::default();
        seen.insert(self.to_path(path));
        let mut queue = std::collections::VecDeque::from([self.to_absolute_path(path)]);
        let mut aliases = Vec::new();
        while let Some(candidate) = queue.pop_front() {
            for s in &symlinks {
                let Some(suffix) = tspath::trim_file_path_prefix(&candidate, &s.target, self.cs) else { continue };
                if !suffix.is_empty() && !tspath::has_trailing_directory_separator(&s.target) && !suffix.starts_with('/') {
                    continue;
                }
                let alias = self.to_absolute_path(&tspath::combine_paths(&s.link_name, &[suffix.trim_start_matches('/')]));
                let alias_path = self.to_path(&alias);
                if seen.contains(&alias_path) || self.resolve_path(&alias).is_none() {
                    continue;
                }
                seen.insert(alias_path);
                aliases.push(alias.clone());
                queue.push_back(alias);
            }
        }
        aliases
    }

    fn local(&self, path: &str) -> (Option<Arc<Entry>>, Fallback) {
        let (node, fallback) = self.paths.lookup(&self.to_path(path));
        let entry = node.and_then(|n| n.entry.clone()).filter(|e| !matches!(**e, Entry::Symlink(_)));
        (entry, fallback)
    }

    fn lookup_path(&self, path: &str) -> Option<Lookup> {
        let abs = self.to_absolute_path(path);
        let (entry, fb) = self.local(&abs);
        if let Some(e) = entry {
            return Some(Lookup { path: abs, info: e.info(), entry: Some(e), use_base: false, followed_symlink: false });
        }
        if fb == Fallback::Missing {
            return None;
        }
        let resolved = self.resolve_path(path)?;
        let mut result = Lookup { path: resolved.path.clone(), info: None, entry: None, use_base: false, followed_symlink: resolved.followed_symlink };
        let (rentry, rfb) = self.local(&resolved.path);
        if !resolved.host && rentry.is_some() {
            let e = rentry.unwrap();
            result.info = e.info();
            result.entry = Some(e);
        } else if resolved.host || self.kind == Kind::Layer {
            if rfb == Fallback::Missing {
                return None;
            }
            result.use_base = true;
        }
        Some(result)
    }

    fn mutation_path(&self, path: &str) -> Option<String> {
        if self.kind != Kind::Layer {
            return None;
        }
        self.resolve_path(path).map(|r| r.path)
    }

    fn get_local_entries(&self, dir: &str) -> (Option<Entries>, bool) {
        let (node, _) = self.paths.lookup(&self.to_path(dir));
        let explicit = matches!(node.and_then(|n| n.entry.as_deref()), Some(Entry::Directory { listing: Some(_), .. }));
        (Node::entries(node), explicit)
    }

    fn remove_entries(&self, dir: &str, entries: &Entries) -> Entries {
        let mut r = entries.clone();
        r.files.retain(|n| !self.blocks_fallback(&tspath::combine_paths(dir, &[n])));
        r.directories.retain(|n| !self.blocks_fallback(&tspath::combine_paths(dir, &[n])));
        if let Some(s) = &mut r.symlinks {
            s.retain(|n| !self.blocks_fallback(&tspath::combine_paths(dir, &[n])));
        }
        r
    }

    fn filter_local_entries(&self, dir: &str, entries: Entries) -> Entries {
        let keep = |name: &str| -> bool {
            let file_name = tspath::combine_paths(dir, &[name]);
            if self.local(&file_name).0.is_some() {
                return true;
            }
            !self.blocks_fallback(&file_name)
        };
        let mut r = entries;
        r.files.retain(|n| keep(n));
        r.directories.retain(|n| keep(n));
        if let Some(s) = &mut r.symlinks {
            s.retain(|n| keep(n));
        }
        r
    }

    fn add_symlink_entries(&self, dir: &str, entries: Entries) -> Entries {
        let mut r = entries;
        let mut symlinks = r.symlinks.take().unwrap_or_default();
        let mut links = Vec::new();
        if let (Some(node), _) = self.paths.lookup(&self.to_path(dir)) {
            for child in node.children.values() {
                if let Some(Entry::Symlink(s)) = child.entry.as_deref() {
                    links.push(s.clone());
                }
            }
        }
        if links.is_empty() {
            r.symlinks = Some(symlinks);
            return r;
        }
        for s in links {
            let name = tspath::get_base_file_name(&s.link_name);
            r.files.retain(|v| !equal_names(v, &name, self.cs));
            r.directories.retain(|v| !equal_names(v, &name, self.cs));
            symlinks.retain(|v| !equal_names(v, &name, self.cs));
            if self.directory_exists(&s.link_name) {
                r.directories.push(name.clone());
                symlinks.insert(name);
            } else if self.file_exists(&s.link_name) {
                r.files.push(name.clone());
                symlinks.insert(name);
            }
        }
        r.files.sort();
        r.directories.sort();
        r.symlinks = Some(symlinks);
        r
    }
}

impl FS for RequestFileSystem {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.cs
    }
    fn file_exists(&self, path: &str) -> bool {
        let Some(l) = self.lookup_path(path) else { return false };
        if l.info.as_ref().is_some_and(|i| i.mode.is_dir()) {
            return false;
        }
        l.info.is_some() || l.use_base && self.base.file_exists(&l.path)
    }
    fn read_file(&self, path: &str) -> Option<String> {
        let l = self.lookup_path(path)?;
        if l.info.as_ref().is_some_and(|i| i.mode.is_dir()) {
            return None;
        }
        if l.use_base {
            return self.base.read_file(&l.path);
        }
        match l.entry.as_deref() {
            Some(Entry::File { content, .. }) => Some(content.to_string()),
            _ => None,
        }
    }
    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        let p = self.mutation_path(path).ok_or_else(|| "invalid argument".to_string())?;
        self.base.write_file(&p, data)
    }
    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        let p = self.mutation_path(path).ok_or_else(|| "invalid argument".to_string())?;
        self.base.append_file(&p, data)
    }
    fn remove(&self, path: &str) -> Result<(), String> {
        let p = self.mutation_path(path).ok_or_else(|| "invalid argument".to_string())?;
        self.base.remove(&p)
    }
    fn chtimes(&self, path: &str, a: SystemTime, m: SystemTime) -> Result<(), String> {
        let p = self.mutation_path(path).ok_or_else(|| "invalid argument".to_string())?;
        self.base.chtimes(&p, a, m)
    }
    fn directory_exists(&self, path: &str) -> bool {
        let Some(l) = self.lookup_path(path) else { return false };
        if l.info.as_ref().is_some_and(|i| !i.mode.is_dir()) {
            return false;
        }
        l.info.is_some() || l.use_base && self.base.directory_exists(&l.path)
    }
    fn get_accessible_entries(&self, directory_name: &str) -> Entries {
        let empty = Entries { symlinks: Some(FxHashSet::default()), ..Default::default() };
        let Some(l) = self.lookup_path(directory_name) else { return empty };
        if l.info.as_ref().is_some_and(|i| !i.mode.is_dir()) {
            return empty;
        }
        let result = if l.use_base {
            self.remove_entries(&l.path, &self.base.get_accessible_entries(&l.path))
        } else {
            let (local, explicit) = self.get_local_entries(&l.path);
            let local = local.unwrap_or_default();
            let mut result = local.clone();
            if self.kind == Kind::Layer && !explicit && !self.blocks_fallback(directory_name) && !self.blocks_fallback(&l.path) {
                result = self.remove_entries(&l.path, &self.base.get_accessible_entries(&l.path));
                result = merge_entries(&result, &local, self.cs);
            }
            self.add_symlink_entries(&l.path, result)
        };
        self.filter_local_entries(directory_name, result)
    }
    fn stat(&self, path: &str) -> Option<FileInfo> {
        let l = self.lookup_path(path)?;
        if l.use_base {
            if let Some(i) = self.base.stat(&l.path) {
                return Some(i);
            }
            if self.base.directory_exists(&l.path) {
                return Some(dir_info(&l.path));
            }
            if self.base.file_exists(&l.path) {
                return Some(FileInfo { name: tspath::get_base_file_name(&l.path), size: 0, mode: FileMode::from_bits_retain(0o444), mod_time: None });
            }
            return None;
        }
        l.info
    }
    fn realpath(&self, path: &str) -> String {
        let Some(l) = self.lookup_path(path) else { return path.to_string() };
        if l.use_base {
            return self.base.realpath(&l.path);
        }
        if l.info.is_some() || !l.followed_symlink {
            return l.path;
        }
        path.to_string()
    }
}

/// Go `addFileChanges`.
fn add_file_changes(summary: &mut FileChangeSummary, request: &RequestParams, base_request: Option<&RequestFileSystem>, _fs: &RequestFileSystem, cwd: &str) {
    // `baseFS` in Go is the base snapshot filesystem: the previous request filesystem or the host.
    let base_fs: &dyn FS = match base_request {
        Some(b) => b,
        None => &*_fs.base,
    };
    let cs = base_fs.use_case_sensitive_file_names();
    let to_path = |f: &str| tspath::to_path(f, cwd, cs);
    let uri = |f: &str| tsrs_ls::lsconv::file_name_to_document_uri(f);
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
    let add_with_aliases = |summary: &mut FileChangeSummary, file_name: &str, deleted: bool| {
        add_change(summary, file_name, deleted);
        if let Some(b) = base_request {
            for alias in b.aliases_for_path(file_name) {
                add_change(summary, &alias, deleted);
            }
        }
    };
    let mut overlay_files = FxHashSet::default();
    for (f, _) in &request.files {
        let abs = tspath::get_normalized_absolute_path(f, cwd);
        overlay_files.insert(to_path(&abs));
        add_with_aliases(summary, &abs, false);
    }
    for r in &request.removed_paths {
        let abs = tspath::get_normalized_absolute_path(r, cwd);
        if overlay_files.contains(&to_path(&abs)) {
            continue;
        }
        add_with_aliases(summary, &abs, true);
    }
    let mut add_replacement = |summary: &mut FileChangeSummary, p: &str| {
        let abs = tspath::get_normalized_absolute_path(p, cwd);
        add_with_aliases(summary, &abs, true);
        summary.created.add(uri(&abs));
        if let Some(b) = base_request {
            for alias in b.aliases_for_path(&abs) {
                summary.created.add(uri(&alias));
            }
        }
    };
    for (d, _) in &request.directories {
        add_replacement(summary, d);
    }
    for (l, _, _) in &request.symlinks {
        add_replacement(summary, l);
    }
    if summary.changed.len() + summary.created.len() + summary.deleted.len() > 0 {
        summary.includes_watch_change_outside_node_modules = true;
    }
}
