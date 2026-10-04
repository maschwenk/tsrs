// Port of execute/incremental/referencemap.go.

use std::sync::OnceLock;

use rustc_hash::FxHashMap;
use tsrs_core::collections::{Set, SyncMap};
use tsrs_core::tspath::Path;

#[derive(Default)]
pub struct ReferenceMap {
    references: SyncMap<Path, std::sync::Arc<Set<Path>>>,
    referenced_by: OnceLock<FxHashMap<Path, Set<Path>>>,
}

impl ReferenceMap {
    // referencemap.go:18
    pub(crate) fn store_references(&self, path: Path, refs: Set<Path>) {
        self.references.store(path, std::sync::Arc::new(refs));
    }

    // referencemap.go:18, for a set that other entries share.
    pub(crate) fn store_shared_references(&self, path: Path, refs: std::sync::Arc<Set<Path>>) {
        self.references.store(path, refs);
    }

    // referencemap.go:22
    pub(crate) fn get_references(&self, path: &Path) -> Option<std::sync::Arc<Set<Path>>> {
        self.references.load(path)
    }

    // referencemap.go:27
    pub(crate) fn get_paths_with_references(&self) -> Vec<Path> {
        self.references.keys()
    }

    // referencemap.go:31
    pub(crate) fn get_referenced_by(&self, path: &Path) -> Vec<Path> {
        let referenced_by = self.referenced_by.get_or_init(|| {
            let mut referenced_by: FxHashMap<Path, Set<Path>> = FxHashMap::default();
            self.references.range(|key, value| {
                for r in value.keys() {
                    referenced_by.entry(r.clone()).or_default().add(key.clone());
                }
                true
            });
            referenced_by
        });
        match referenced_by.get(path) {
            Some(refs) => refs.keys().iter().cloned().collect(),
            None => Vec::new(),
        }
    }
}
