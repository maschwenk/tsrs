// Port of execute/incremental/referencemap.go.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use rustc_hash::FxHashMap;
use tsrs_core::collections::{Set, SyncMap};
use tsrs_core::tspath::Path;

#[derive(Default)]
pub struct ReferenceMap {
    references: SyncMap<Path, Arc<Set<Path>>>,
    // Go builds `referencedBy` once (`sync.Once`), from the references stored at the first query. Here that
    // snapshot is `groups`: the files grouped by the reference set they share (files read from a tsbuildinfo
    // share one set per id list). The first `SCAN_QUERIES` queries test each distinct set for the path; after
    // that the inverted map is built from the same snapshot. Either way the answer is the one Go's map gives.
    // A leaf edit asks once or twice, and inverting the ~15 M references of the 38k-file codebase costs ~1 s.
    groups: OnceLock<Vec<(Arc<Set<Path>>, Vec<Path>)>>,
    queries: AtomicUsize,
    // Referenced path -> indices into `groups` of the sets that contain it.
    referenced_by: OnceLock<FxHashMap<Path, Vec<u32>>>,
}

const SCAN_QUERIES: usize = 256;

impl ReferenceMap {
    // referencemap.go:18
    pub(crate) fn store_references(&self, path: Path, refs: Set<Path>) {
        self.references.store(path, Arc::new(refs));
    }

    // referencemap.go:18, for a set that other entries share.
    pub(crate) fn store_shared_references(&self, path: Path, refs: Arc<Set<Path>>) {
        self.references.store(path, refs);
    }

    // referencemap.go:22
    pub(crate) fn get_references(&self, path: &Path) -> Option<Arc<Set<Path>>> {
        self.references.load(path)
    }

    // referencemap.go:27
    pub(crate) fn get_paths_with_references(&self) -> Vec<Path> {
        self.references.keys()
    }

    // referencemap.go:31. Unordered, like Go's `maps.Keys`; every caller sorts it.
    pub(crate) fn get_referenced_by(&self, path: &Path) -> Vec<Path> {
        let groups = self.groups.get_or_init(|| {
            let mut index: FxHashMap<*const Set<Path>, usize> = FxHashMap::default();
            let mut groups: Vec<(Arc<Set<Path>>, Vec<Path>)> = Vec::new();
            self.references.range(|key, value| {
                let i = *index.entry(Arc::as_ptr(value)).or_insert_with(|| {
                    groups.push((value.clone(), Vec::new()));
                    groups.len() - 1
                });
                groups[i].1.push(key.clone());
                true
            });
            groups
        });
        let mut result = Vec::new();
        if self.referenced_by.get().is_none() && self.queries.fetch_add(1, Ordering::Relaxed) < SCAN_QUERIES {
            for (refs, owners) in groups {
                if refs.has(path) {
                    result.extend(owners.iter().cloned());
                }
            }
            return result;
        }
        let referenced_by = self.referenced_by.get_or_init(|| {
            let mut referenced_by: FxHashMap<Path, Vec<u32>> = FxHashMap::default();
            for (i, (refs, _)) in groups.iter().enumerate() {
                for r in refs.keys() {
                    match referenced_by.get_mut(r) {
                        Some(sets) => sets.push(i as u32),
                        None => {
                            referenced_by.insert(r.clone(), vec![i as u32]);
                        }
                    }
                }
            }
            referenced_by
        });
        if let Some(sets) = referenced_by.get(path) {
            for &i in sets {
                result.extend(groups[i as usize].1.iter().cloned());
            }
        }
        result
    }
}
