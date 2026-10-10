//! Lazy checker maps owned by their containing Rust records. Nil reads preserve the port's semantics.
#![forbid(unsafe_code)]

use crate::packedmap::{PackedKey, PackedMap};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::hash::Hash;

pub struct OwnedMap<K, V>(RefCell<Option<Box<FxHashMap<K, V>>>>);

impl<K, V> Default for OwnedMap<K, V> {
    fn default() -> Self {
        Self(RefCell::new(None))
    }
}

impl<K: Eq + Hash, V: Clone> OwnedMap<K, V> {
    pub fn new_empty() -> Self {
        Self(RefCell::new(Some(Box::default())))
    }

    pub fn make(&self) {
        *self.0.borrow_mut() = Some(Box::default());
    }
    pub fn reset(&self) {
        *self.0.borrow_mut() = None;
    }
    pub fn is_nil(&self) -> bool {
        self.0.borrow().is_none()
    }

    pub fn get<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
    {
        self.0.borrow().as_ref().and_then(|m| m.get(key).cloned())
    }

    pub fn has(&self, key: &K) -> bool {
        self.0.borrow().as_ref().is_some_and(|m| m.contains_key(key))
    }

    pub fn set(&self, key: K, value: V) {
        self.0.borrow_mut().get_or_insert_with(Box::default).insert(key, value);
    }

    pub fn delete(&self, key: &K) {
        if let Some(m) = self.0.borrow_mut().as_mut() {
            m.remove(key);
        }
    }

    pub fn len(&self) -> usize {
        self.0.borrow().as_ref().map_or(0, |m| m.len())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        if let Some(m) = self.0.borrow_mut().as_mut() {
            m.clear();
        }
    }

    /// Preserve the pooled scratch-map capacity policy while releasing its records through ordinary Drop.
    pub fn clear_scratch(&self) {
        if let Some(m) = self.0.borrow_mut().as_mut() {
            let len = m.len();
            if m.capacity() > 64 && m.capacity() > 4 * len {
                **m = FxHashMap::with_capacity_and_hasher(len, Default::default());
            } else {
                m.clear();
            }
        }
    }

    pub fn assign(&self, m: FxHashMap<K, V>) {
        *self.0.borrow_mut() = Some(Box::new(m));
    }

    pub fn entries(&self) -> Vec<(K, V)>
    where
        K: Clone,
    {
        self.0.borrow().as_ref().map_or_else(Vec::new, |m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }
}

pub struct OwnedPackedMap<K, V>(RefCell<Option<Box<PackedMap<K, V>>>>);

impl<K, V> Default for OwnedPackedMap<K, V> {
    fn default() -> Self {
        Self(RefCell::new(None))
    }
}

impl<K: PackedKey, V: Copy> OwnedPackedMap<K, V> {
    pub fn make(&self) {
        *self.0.borrow_mut() = Some(Box::default());
    }
    pub fn is_nil(&self) -> bool {
        self.0.borrow().is_none()
    }
    #[inline]
    pub fn get(&self, key: &K) -> Option<V> {
        self.0.borrow().as_ref().and_then(|m| m.get(key))
    }
    pub fn set(&self, key: K, value: V) {
        self.0.borrow_mut().get_or_insert_with(Box::default).insert(key, value);
    }
    pub fn len(&self) -> usize {
        self.0.borrow().as_ref().map_or(0, |m| m.len())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[cfg(feature = "assignment-stats")]
    pub(crate) fn heap_stat(&self) -> Option<crate::heapcensus::HeapStat> {
        self.0.borrow().as_ref().map(|m| m.heap_stat())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn lazy_maps_preserve_nil_reads_and_release_replaced_values() {
        let map = OwnedMap::<u32, Rc<()>>::default();
        assert!(map.is_nil());
        assert_eq!(map.len(), 0);
        assert!(map.get(&1).is_none());
        map.clear();
        assert!(map.is_nil());
        let value = Rc::new(());
        let weak = Rc::downgrade(&value);
        map.set(1, value);
        assert!(map.has(&1));
        let returned = map.get(&1).unwrap();
        map.make();
        assert!(!map.is_nil());
        assert!(map.is_empty());
        assert!(weak.upgrade().is_some());
        drop(returned);
        assert!(weak.upgrade().is_none());
        map.reset();
        assert!(map.is_nil());
    }

    #[test]
    fn dropping_a_link_record_reclaims_its_map_and_values() {
        let value = Rc::new(());
        let weak = Rc::downgrade(&value);
        static KEY: u32 = 1;
        let links = tsrs_core::LinkStore::<u32, OwnedMap<u32, Rc<()>>>::default();
        links.get(tsrs_core::P::from_static(&KEY)).set(1, value);
        links.clear();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn owned_packed_maps_keep_values_through_rehashing_and_reset() {
        #[derive(Clone, Copy, PartialEq, Eq)]
        struct Key(u32);
        impl PackedKey for Key {
            fn packed_hash(&self) -> u64 {
                self.0 as u64
            }
        }
        let map = OwnedPackedMap::default();
        assert!(map.is_nil());
        assert!(map.get(&Key(0)).is_none());
        for i in 0..5000 {
            map.set(Key(i), i + 1);
        }
        for i in 0..5000 {
            assert_eq!(map.get(&Key(i)), Some(i + 1));
        }
        map.make();
        assert!(map.is_empty());
        assert!(!map.is_nil());
    }
}
