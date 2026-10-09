//! Shared-graph prototype (`tsrs_core::sharedgraph`): a fork's interning map over the frozen seed's. A lookup tries
//! the fork's own map, then the seed's (read only: the seed checker is frozen and never written again); inserts go
//! to the fork's own map; a removed key that the seed holds is remembered so the seed's entry stays hidden. Without a
//! base (the switch off, or the seed itself) it is the plain map plus one `None` test per miss.

use std::hash::Hash;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::heapcensus::{HeapSize, HeapStat};
use crate::packedmap::{PackedKey, PackedMap};

pub struct Based<M: 'static, K: 'static = ()> {
    pub own: M,
    base: Option<&'static M>,
    /// Keys removed from the fork that the base may hold (only maps with `remove`).
    removed: FxHashSet<K>,
}

impl<M: Default, K> Default for Based<M, K> {
    fn default() -> Self {
        Based { own: M::default(), base: None, removed: FxHashSet::default() }
    }
}

impl<M: Default, K> Based<M, K> {
    /// An empty map that reads through to `base`'s (a frozen seed's) own map.
    pub fn over(base: &'static Based<M, K>) -> Self {
        Based { own: M::default(), base: Some(&base.own), removed: FxHashSet::default() }
    }

    #[inline]
    pub fn base(&self) -> Option<&'static M> {
        self.base
    }
}

impl<M: HeapSize, K> HeapSize for Based<M, K> {
    /// The fork's own map only (the base is counted once, in the seed).
    fn heap_stat(&self) -> HeapStat {
        self.own.heap_stat()
    }
}

impl<K: PackedKey, V: Copy> Based<PackedMap<K, V>> {
    #[inline]
    pub fn get(&self, key: &K) -> Option<V> {
        match self.own.get(key) {
            Some(v) => Some(v),
            None => self.base.filter(|_| tsrs_core::sharedgraph::COMPILED_IN).and_then(|b| b.get(key)),
        }
    }

    #[inline]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.own.insert(key, value)
    }

    /// The base's values, then the fork's own.
    pub fn values(&self) -> impl Iterator<Item = V> + '_ {
        self.base.into_iter().flat_map(|b| b.values()).chain(self.own.values())
    }

    pub(crate) fn heap_stat(&self) -> HeapStat {
        self.own.heap_stat()
    }
}

/// A key of a two-level map that can tell it is not one of the seed's objects (strategy E, spike/shared-graph-seed):
/// then the base cannot hold it and the second probe is skipped.
pub trait BaseKey {
    #[inline]
    fn maybe_in_base(&self) -> bool {
        true
    }
}

impl BaseKey for crate::checker::UnionOfUnionKey {}

/// A type the seed created lies in its frozen region; a fork's own types never do.
impl BaseKey for tsrs_core::P<crate::types::Type> {
    #[inline]
    fn maybe_in_base(&self) -> bool {
        tsrs_core::sharedgraph::is_frozen_addr(self.addr())
    }
}

impl<K: Hash + Eq + Clone + BaseKey, V> Based<FxHashMap<K, V>, K> {
    #[inline]
    pub fn get(&self, key: &K) -> Option<&V> {
        match self.own.get(key) {
            Some(v) => Some(v),
            None => self.base_get(key),
        }
    }

    /// The base's entry for `key`, unless the fork removed it.
    #[inline]
    pub fn base_get(&self, key: &K) -> Option<&'static V> {
        let b = self.base.filter(|_| tsrs_core::sharedgraph::COMPILED_IN)?;
        if !key.maybe_in_base() {
            return None;
        }
        if !self.removed.is_empty() && self.removed.contains(key) {
            return None;
        }
        b.get(key)
    }

    #[inline]
    pub fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    #[inline]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if !self.removed.is_empty() {
            self.removed.remove(&key);
        }
        self.own.insert(key, value)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        if key.maybe_in_base() && self.base.filter(|_| tsrs_core::sharedgraph::COMPILED_IN).is_some_and(|b| b.contains_key(key)) {
            self.removed.insert(key.clone());
        }
        self.own.remove(key)
    }

    /// The fork's own values (heap census: what this checker holds).
    pub fn values(&self) -> impl Iterator<Item = &V> + '_ {
        self.own.values()
    }
}
