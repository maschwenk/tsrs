// Port of tsc/internal/api/requestfilesystem/pathtree.go: the canonical-path trie that stores request
// files, directories, symlinks and per-path fallback (negative lookup) state. Nodes are immutable once
// a request filesystem is built; composition shares unchanged subtrees (Go: maps.Clone of children).

use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_core::tspath::{self, Path};
use tsrs_vfs::Entries;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Fallback {
    #[default]
    Inherit,
    Allowed,
    Missing,
}

#[derive(Debug)]
pub(crate) struct ReqFile {
    pub file_name: String,
    pub content: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ReqSymlink {
    pub link_name: String,
    pub target: String,
    pub host: bool,
}

#[derive(Debug)]
pub(crate) struct ReqDirectory {
    pub directory_name: String,
    pub listing: Option<Arc<Entries>>,
}

#[derive(Clone, Debug)]
pub(crate) enum Entry {
    File(Arc<ReqFile>),
    Symlink(Arc<ReqSymlink>),
    Directory(Arc<ReqDirectory>),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Node {
    pub entry: Option<Entry>,
    pub fallback: Fallback,
    pub children: FxHashMap<Path, Arc<Node>>,
    pub has_symlinks: bool,
}

pub(crate) fn ancestors(path: &str) -> Vec<Path> {
    let mut paths = Vec::new();
    let mut path = path.to_string();
    loop {
        let parent = tspath::get_directory_path(&path);
        let done = parent == path;
        paths.push(Path::new(path));
        if done {
            break;
        }
        path = parent;
    }
    paths.reverse();
    paths
}

pub(crate) fn clone_entries(entries: &Entries) -> Entries {
    entries.clone()
}

impl Node {
    fn replaces_subtree(&self) -> bool {
        matches!(self.entry, Some(Entry::File(_)) | Some(Entry::Symlink(_)))
    }

    pub fn ensure(&mut self, path: &Path) -> &mut Node {
        let mut node = self;
        for ancestor in ancestors(path) {
            let child = node.children.entry(ancestor).or_default();
            node = Arc::make_mut(child);
        }
        node
    }

    pub fn lookup(&self, path: &Path) -> (Option<&Node>, Fallback) {
        let mut fallback = Fallback::Inherit;
        let mut node = Some(self);
        for ancestor in ancestors(path) {
            let Some(n) = node else { break };
            if n.fallback != Fallback::Inherit {
                fallback = n.fallback;
            }
            node = n.children.get(&ancestor).map(|c| &**c);
        }
        if let Some(n) = node {
            if n.fallback != Fallback::Inherit {
                fallback = n.fallback;
            }
        }
        (node, fallback)
    }

    /// All symlinks in the tree, ordered by canonical link path (Go iterates a map; sorting keeps
    /// alias discovery deterministic).
    pub fn symlinks(&self) -> Vec<(Path, Arc<ReqSymlink>)> {
        let mut out = Vec::new();
        self.collect_symlinks(&mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn collect_symlinks(&self, out: &mut Vec<(Path, Arc<ReqSymlink>)>) {
        if !self.has_symlinks {
            return;
        }
        for (path, child) in &self.children {
            if let Some(Entry::Symlink(s)) = &child.entry {
                out.push((path.clone(), s.clone()));
            }
            child.collect_symlinks(out);
        }
    }

    pub fn first_symlink(&self, path: &Path) -> Option<(Path, Arc<ReqSymlink>)> {
        let mut node = Some(self);
        for ancestor in ancestors(path) {
            let n = node?;
            node = n.children.get(&ancestor).map(|c| &**c);
            if let Some(Node { entry: Some(Entry::Symlink(s)), .. }) = node {
                return Some((ancestor, s.clone()));
            }
        }
        None
    }

    pub fn contains_file_ancestor(&self, path: &Path) -> bool {
        let mut node = Some(self);
        for ancestor in ancestors(path) {
            let Some(n) = node else { return false };
            node = n.children.get(&ancestor).map(|c| &**c);
            if ancestor != *path {
                if let Some(Node { entry: Some(Entry::File(_)), .. }) = node {
                    return true;
                }
            }
        }
        false
    }
}

/// requestPathNode.entries for a possibly-missing node.
pub(crate) fn node_entries(node: Option<&Node>) -> (Entries, bool) {
    let Some(node) = node else { return (Entries::default(), false) };
    let Some(Entry::Directory(directory)) = &node.entry else { return (Entries::default(), false) };
    if let Some(listing) = &directory.listing {
        return (clone_entries(listing), true);
    }
    let mut entries = Entries::default();
    for child in node.children.values() {
        match &child.entry {
            Some(Entry::File(f)) => entries.files.push(tspath::get_base_file_name(&f.file_name)),
            Some(Entry::Directory(d)) => entries.directories.push(tspath::get_base_file_name(&d.directory_name)),
            _ => {}
        }
    }
    entries.files.sort();
    entries.directories.sort();
    (entries, true)
}

pub(crate) fn equal_names(left: &str, right: &str, case_sensitive: bool) -> bool {
    tspath::get_canonical_file_name(left, case_sensitive) == tspath::get_canonical_file_name(right, case_sensitive)
}

/// requestfilesystem.go mergeEntries.
pub(crate) fn merge_entries(base: &Entries, overlay: &Entries, case_sensitive: bool) -> Entries {
    let eq = |a: &str, b: &str| equal_names(a, b, case_sensitive);
    let mut result = clone_entries(base);
    let mut symlinks = result.symlinks.take().unwrap_or_default();
    let delete_symlink = |symlinks: &mut FxHashSet<String>, name: &str| symlinks.retain(|existing| !eq(existing, name));
    for name in &overlay.files {
        result.directories.retain(|v| !eq(v, name));
        if !result.files.iter().any(|v| eq(v, name)) {
            result.files.push(name.clone());
        }
        delete_symlink(&mut symlinks, name);
    }
    for name in &overlay.directories {
        result.files.retain(|v| !eq(v, name));
        if !result.directories.iter().any(|v| eq(v, name)) {
            result.directories.push(name.clone());
        }
        delete_symlink(&mut symlinks, name);
    }
    if let Some(overlay_links) = &overlay.symlinks {
        symlinks.extend(overlay_links.iter().cloned());
    }
    result.symlinks = Some(symlinks);
    result.files.sort();
    result.directories.sort();
    result
}

/// pathtree.go composeRequestPaths.
pub(crate) fn compose(base: Option<&Arc<Node>>, overlay: Option<&Arc<Node>>, fallback: Fallback, case_sensitive: bool) -> Option<Arc<Node>> {
    let Some(overlay) = overlay else { return base.cloned() };
    let mut fallback = fallback;
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
    let previous_directory = match &result.entry {
        Some(Entry::Directory(d)) => Some(d.clone()),
        _ => None,
    };
    let overlay_directory = match &overlay.entry {
        Some(Entry::Directory(d)) => Some(d.clone()),
        _ => None,
    };
    if let Some(entry) = &overlay.entry {
        result.entry = Some(entry.clone());
        if let (Some(od), Some(pd)) = (&overlay_directory, &previous_directory) {
            if od.listing.is_none() {
                result.entry = Some(Entry::Directory(Arc::new(ReqDirectory {
                    directory_name: od.directory_name.clone(),
                    listing: pd.listing.clone(),
                })));
            }
        }
    }
    for (path, child) in &overlay.children {
        let composed = compose(result.children.get(path), Some(child), fallback, case_sensitive);
        if let Some(composed) = composed {
            result.children.insert(path.clone(), composed);
        }
    }
    if let Some(Entry::Directory(directory)) = &result.entry {
        if let Some(listing) = &directory.listing {
            if overlay_directory.as_ref().is_none_or(|od| od.listing.is_none()) {
                let eq = |a: &str, b: &str| equal_names(a, b, case_sensitive);
                let mut entries = clone_entries(listing);
                let mut paths: Vec<&Path> = overlay.children.keys().collect();
                paths.sort();
                for path in paths {
                    let child = &overlay.children[path];
                    let name = tspath::get_base_file_name(path);
                    if child.fallback == Fallback::Missing || child.replaces_subtree() {
                        entries.files.retain(|e| !eq(e, &name));
                        entries.directories.retain(|e| !eq(e, &name));
                        if let Some(links) = &mut entries.symlinks {
                            links.retain(|e| !eq(e, &name));
                        }
                    }
                    match &child.entry {
                        Some(Entry::File(f)) => {
                            let add = Entries { files: vec![tspath::get_base_file_name(&f.file_name)], ..Default::default() };
                            entries = merge_entries(&entries, &add, case_sensitive);
                        }
                        Some(Entry::Directory(d)) => {
                            let add = Entries { directories: vec![tspath::get_base_file_name(&d.directory_name)], ..Default::default() };
                            entries = merge_entries(&entries, &add, case_sensitive);
                        }
                        _ => {}
                    }
                }
                let name = directory.directory_name.clone();
                result.entry = Some(Entry::Directory(Arc::new(ReqDirectory { directory_name: name, listing: Some(Arc::new(entries)) })));
            }
        }
    }
    result.has_symlinks = matches!(result.entry, Some(Entry::Symlink(_))) || result.children.values().any(|c| c.has_symlinks);
    Some(Arc::new(result))
}

pub(crate) fn path_contains(parent: &str, path: &str) -> bool {
    path == parent || path.starts_with(&tspath::ensure_trailing_directory_separator(parent))
}
