use std::hash::Hash;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};

pub struct MapBuilder<K, VBase, VBuilder> {
    base: Arc<FxHashMap<K, VBase>>,
    dirty: FxHashMap<K, VBuilder>,
    deleted: Option<FxHashSet<K>>,

    to_builder: Box<dyn Fn(&VBase) -> VBuilder + Send + Sync>,
    build: Box<dyn Fn(VBuilder) -> VBase + Send + Sync>,
}

// mapbuilder.go:14
pub fn new_map_builder<K, VBase, VBuilder>(
    base: Arc<FxHashMap<K, VBase>>,
    to_builder: impl Fn(&VBase) -> VBuilder + Send + Sync + 'static,
    build: impl Fn(VBuilder) -> VBase + Send + Sync + 'static,
) -> MapBuilder<K, VBase, VBuilder> {
    MapBuilder { base, dirty: FxHashMap::default(), deleted: None, to_builder: Box::new(to_builder), build: Box::new(build) }
}

impl<K: Hash + Eq + Clone, VBase: Clone, VBuilder: Clone> MapBuilder<K, VBase, VBuilder> {
    // mapbuilder.go:27
    pub fn set(&mut self, key: K, value: VBuilder) {
        if let Some(deleted) = &mut self.deleted {
            deleted.remove(&key);
        }
        self.dirty.insert(key, value);
    }

    // mapbuilder.go:32
    pub fn delete(&mut self, key: K) {
        self.dirty.remove(&key);
        self.deleted.get_or_insert_with(FxHashSet::default).insert(key);
    }

    // mapbuilder.go:40
    pub fn clear(&mut self) {
        self.dirty = FxHashMap::default();
        let mut deleted = FxHashSet::with_capacity_and_hasher(self.base.len(), Default::default());
        for key in self.base.keys() {
            deleted.insert(key.clone());
        }
        self.deleted = Some(deleted);
    }

    // mapbuilder.go:48
    pub fn has(&self, key: &K) -> bool {
        if self.deleted.as_ref().is_some_and(|d| d.contains(key)) {
            return false;
        }
        if self.dirty.contains_key(key) {
            return true;
        }
        self.base.contains_key(key)
    }

    // mapbuilder.go:59
    pub fn build(&self) -> Arc<FxHashMap<K, VBase>> {
        if self.dirty.is_empty() && self.deleted.as_ref().is_none_or(|d| d.is_empty()) {
            return self.base.clone();
        }
        let mut result: FxHashMap<K, VBase> = (*self.base).clone();
        if let Some(deleted) = &self.deleted {
            for key in deleted {
                result.remove(key);
            }
        }
        for (key, value) in &self.dirty {
            result.insert(key.clone(), (self.build)(value.clone()));
        }
        Arc::new(result)
    }

    pub fn to_builder(&self, value: &VBase) -> VBuilder {
        (self.to_builder)(value)
    }
}
