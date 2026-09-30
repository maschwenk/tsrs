use crate::*;

/// All Go link stores (`core.LinkStore`, `nodeLinkStore`, `symbolArenaLinkStore`) map to this one type.
/// Values live in the arena, so `get` hands out a `Copy` pointer whose `Cell` fields are mutated in place.
pub struct LinkStore<K: 'static, V: 'static> {
    entries: FxHashMap<P<K>, P<V>>,
}

impl<K: 'static, V: 'static> Default for LinkStore<K, V> {
    fn default() -> Self {
        LinkStore { entries: FxHashMap::default() }
    }
}

impl<K: 'static, V: Default + 'static> LinkStore<K, V> {
    /// Returns the links for `key`, creating them on first use.
    #[inline]
    pub fn get(&mut self, key: P<K>) -> P<V> {
        *self.entries.entry(key).or_insert_with(|| P::new(V::default()))
    }

    #[inline]
    pub fn try_get(&self, key: P<K>) -> Option<P<V>> {
        self.entries.get(&key).copied()
    }

    #[inline]
    pub fn has(&self, key: P<K>) -> bool {
        self.entries.contains_key(&key)
    }
}
