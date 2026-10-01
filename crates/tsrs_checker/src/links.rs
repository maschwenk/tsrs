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

/// Links keyed by a node/symbol id (Go `PagedLinkStore`-backed stores). Go pages by id; ids are process-wide and
/// shared by all checkers, so per-checker pages would be dense in every checker. This store maps the id to a slot
/// in fixed-size value chunks instead: 9 bytes per map slot (u32 id -> u32 slot) rather than 17 for a pointer-keyed
/// map with a separate arena allocation per value.
pub struct IdLinkStore<V: 'static> {
    slots: FxHashMap<u32, u32>,
    wide_slots: FxHashMap<u64, u32>, // ids >= 2^32 (long-running processes such as the test runner)
    chunks: Vec<&'static [V]>,
    len: u32,
}

const ID_LINK_CHUNK_SHIFT: u32 = 12;
const ID_LINK_CHUNK: usize = 1 << ID_LINK_CHUNK_SHIFT;

impl<V: 'static> Default for IdLinkStore<V> {
    fn default() -> Self {
        IdLinkStore { slots: FxHashMap::default(), wide_slots: FxHashMap::default(), chunks: Vec::new(), len: 0 }
    }
}

impl<V: 'static> IdLinkStore<V> {
    #[inline]
    fn at(&self, slot: u32) -> P<V> {
        P::from_static(&self.chunks[(slot >> ID_LINK_CHUNK_SHIFT) as usize][slot as usize & (ID_LINK_CHUNK - 1)])
    }

    #[inline]
    fn slot(&self, id: u64) -> Option<u32> {
        if id <= u32::MAX as u64 {
            self.slots.get(&(id as u32)).copied()
        } else {
            self.wide_slots.get(&id).copied()
        }
    }

    #[inline]
    pub fn try_get(&self, id: u64) -> Option<P<V>> {
        self.slot(id).map(|slot| self.at(slot))
    }

    #[inline]
    pub fn has(&self, id: u64) -> bool {
        self.slot(id).is_some()
    }
}

impl<V: Default + 'static> IdLinkStore<V> {
    #[inline]
    pub fn get(&mut self, id: u64) -> P<V> {
        if let Some(slot) = self.slot(id) {
            return self.at(slot);
        }
        let slot = self.len;
        if slot as usize % ID_LINK_CHUNK == 0 {
            self.chunks.push(alloc_vec((0..ID_LINK_CHUNK).map(|_| V::default()).collect()));
        }
        self.len += 1;
        if id <= u32::MAX as u64 {
            self.slots.insert(id as u32, slot);
        } else {
            self.wide_slots.insert(id, slot);
        }
        self.at(slot)
    }
}

/// Go `nodeLinkStore`: keyed by the node id, so every access assigns the node its id (`ast.GetNodeId`) the way Go
/// does. Ids are observable (e.g. in cache keys and internal names), so the assignment order must match.
pub struct NodeLinkStore<V: 'static> {
    store: IdLinkStore<V>,
}

impl<V: 'static> Default for NodeLinkStore<V> {
    fn default() -> Self {
        NodeLinkStore { store: IdLinkStore::default() }
    }
}

impl<V: Default + 'static> NodeLinkStore<V> {
    #[inline]
    pub fn get(&mut self, node: P<Node>) -> P<V> {
        self.store.get(ast::get_node_id(node).0)
    }
}

impl<V: 'static> NodeLinkStore<V> {
    #[inline]
    pub fn try_get(&self, node: P<Node>) -> Option<P<V>> {
        self.store.try_get(ast::get_node_id(node).0)
    }

    #[inline]
    pub fn has(&self, node: P<Node>) -> bool {
        self.store.has(ast::get_node_id(node).0)
    }
}

/// Go `symbolArenaLinkStore`: keyed by the symbol id, so every access assigns the symbol its id
/// (`ast.GetSymbolId`) the way Go does. Symbol ids are observable (the internal names of unique-symbol-keyed
/// properties embed them, and the node builder counts their length toward truncation), so the assignment order
/// must match.
pub struct SymbolArenaLinkStore<V: 'static> {
    store: IdLinkStore<V>,
}

impl<V: 'static> Default for SymbolArenaLinkStore<V> {
    fn default() -> Self {
        SymbolArenaLinkStore { store: IdLinkStore::default() }
    }
}

impl<V: Default + 'static> SymbolArenaLinkStore<V> {
    #[inline]
    pub fn get(&mut self, symbol: P<Symbol>) -> P<V> {
        self.store.get(ast::get_symbol_id(symbol).0)
    }
}

impl<V: 'static> SymbolArenaLinkStore<V> {
    #[inline]
    pub fn try_get(&self, symbol: P<Symbol>) -> Option<P<V>> {
        self.store.try_get(ast::get_symbol_id(symbol).0)
    }

    #[inline]
    pub fn has(&self, symbol: P<Symbol>) -> bool {
        self.store.has(ast::get_symbol_id(symbol).0)
    }
}
