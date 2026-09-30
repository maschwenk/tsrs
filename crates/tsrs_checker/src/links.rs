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

/// Go `nodeLinkStore`: keyed by the node id, so every access assigns the node its id (`ast.GetNodeId`) the way Go
/// does. Ids are observable (e.g. in cache keys and internal names), so the assignment order must match.
pub struct NodeLinkStore<V: 'static> {
    store: LinkStore<Node, V>,
}

impl<V: 'static> Default for NodeLinkStore<V> {
    fn default() -> Self {
        NodeLinkStore { store: LinkStore::default() }
    }
}

impl<V: Default + 'static> NodeLinkStore<V> {
    #[inline]
    pub fn get(&mut self, node: P<Node>) -> P<V> {
        ast::get_node_id(node);
        self.store.get(node)
    }

    #[inline]
    pub fn try_get(&self, node: P<Node>) -> Option<P<V>> {
        ast::get_node_id(node);
        self.store.try_get(node)
    }

    #[inline]
    pub fn has(&self, node: P<Node>) -> bool {
        ast::get_node_id(node);
        self.store.has(node)
    }
}

/// Go `symbolArenaLinkStore`: keyed by the symbol id, so every access assigns the symbol its id
/// (`ast.GetSymbolId`) the way Go does. Symbol ids are observable (the internal names of unique-symbol-keyed
/// properties embed them, and the node builder counts their length toward truncation), so the assignment order
/// must match.
pub struct SymbolArenaLinkStore<V: 'static> {
    store: LinkStore<Symbol, V>,
}

impl<V: 'static> Default for SymbolArenaLinkStore<V> {
    fn default() -> Self {
        SymbolArenaLinkStore { store: LinkStore::default() }
    }
}

impl<V: Default + 'static> SymbolArenaLinkStore<V> {
    #[inline]
    pub fn get(&mut self, symbol: P<Symbol>) -> P<V> {
        ast::get_symbol_id(symbol);
        self.store.get(symbol)
    }

    #[inline]
    pub fn try_get(&self, symbol: P<Symbol>) -> Option<P<V>> {
        ast::get_symbol_id(symbol);
        self.store.try_get(symbol)
    }

    #[inline]
    pub fn has(&self, symbol: P<Symbol>) -> bool {
        ast::get_symbol_id(symbol);
        self.store.has(symbol)
    }
}
