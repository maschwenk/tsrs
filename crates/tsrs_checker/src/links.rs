use crate::*;
use tsrs_core::PKey;
use tsrs_core::arena_owner::ArenaKey;

pub use crate::owned_links::{KeyedLinkStore, LinkStore};

/// A link value that holds its own key, for `KeyedLinkStore`.
pub trait KeyedLinks {
    /// The key the store filed the value under (`P::key`); 0 until then. Only the store reads or writes it.
    fn link_key(&self) -> &Cell<PKey>;
}

/// Go `symbolReferenceLinks` (`core.LinkStore[*ast.Symbol, SymbolReferenceLinks]`). The record has one field,
/// `referenceKinds`, a 4-byte `SymbolFlags`, kept in the slot next to the key (8 bytes with compressed pointers, the
/// size of `LinkStore`'s key-and-index slot) instead of in an 8-byte arena slot of its own: -6.5 MB on vscode at one
/// checker (notes/mem-dense-link-tables.md).
///
/// No stable address: the flags move when the table grows, so the store hands out no reference. Go's callers never
/// keep the record: `symbolReferenced` ORs meanings into it and the unused-locals checks read it, each on the spot.
#[derive(Default)]
pub struct SymbolReferenceLinkStore {
    slots: hashbrown::HashTable<ReferenceKindsSlot>,
}

#[derive(Clone, Copy)]
struct ReferenceKindsSlot {
    key: PKey,
    kinds: SymbolFlags,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<ReferenceKindsSlot>() == 16);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<ReferenceKindsSlot>() == 8);

impl SymbolReferenceLinkStore {
    #[inline]
    fn hash(key: PKey) -> u64 {
        use std::hash::BuildHasher;
        rustc_hash::FxBuildHasher.hash_one(key)
    }

    /// Go `c.symbolReferenceLinks.Get(symbol).referenceKinds |= meaning`: creates the record on first use.
    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn add_reference_kinds(&mut self, symbol: P<Symbol>, meaning: SymbolFlags) {
        let key = symbol.key();
        let slot = match self.slots.entry(Self::hash(key), |s| s.key == key, |s| Self::hash(s.key)) {
            hashbrown::hash_table::Entry::Occupied(slot) => slot.into_mut(),
            hashbrown::hash_table::Entry::Vacant(slot) => {
                tsrs_core::sitecount::hit("links", "SymbolReferenceLinks");
                slot.insert(ReferenceKindsSlot { key, kinds: SymbolFlags::None }).into_mut()
            }
        };
        slot.kinds |= meaning;
    }

    /// Go `c.symbolReferenceLinks.Get(symbol).referenceKinds`, without creating a record for a symbol never
    /// referenced (`SymbolFlags::None` either way).
    #[inline]
    pub fn reference_kinds(&self, symbol: P<Symbol>) -> SymbolFlags {
        let key = symbol.key();
        self.slots.find(Self::hash(key), |s| s.key == key).map_or(SymbolFlags::None, |s| s.kinds)
    }

    /// Heap census: the slots (key and flags; nothing in the arena).
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        use crate::heapcensus::HeapSize;
        vec![("slots (key, kinds)", self.slots.heap_stat())]
    }
}

// The semantic node/symbol IDs remain assigned by these wrappers. Storage identities and borrows are owned
// by the safe stores; no record address escapes them.
pub use crate::owned_id_links::{IdLinkStore, InlineIdKey, InlineIdStore};

/// Kept for embedders and the pool; owned compact groups support sparse checker ID blocks directly.
pub fn set_multiple_checkers(_multiple: bool) {}
pub fn set_sparse_id_pages(_on: bool) {}

/// Go `nodeLinkStore`: keyed by the node id, so every access assigns the node its id (`ast.GetNodeId`) the way Go
/// does. Ids are observable (e.g. in cache keys and internal names), so the assignment order must match. The values
/// are stored in owned groups (`InlineIdStore`); its user is `symbol_node_links`.
pub struct NodeLinkStore<V> {
    store: InlineIdStore<V>,
}

impl<V> Default for NodeLinkStore<V> {
    fn default() -> Self {
        NodeLinkStore { store: InlineIdStore::default() }
    }
}

impl<V: Default> NodeLinkStore<V> {
    /// Retain a key across recursive checker work, then borrow again with `at`.
    #[inline]
    pub fn get_key(&mut self, node: P<Node>) -> InlineIdKey<V> {
        self.store.get_key(ast::get_node_id(node).0)
    }
    /// The node's links, created on first use. Access borrows this store.
    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get(&mut self, node: P<Node>) -> &V {
        self.store.get(ast::get_node_id(node).0)
    }
}

impl<V> NodeLinkStore<V> {
    #[inline]
    pub fn at(&self, key: InlineIdKey<V>) -> &V {
        self.store.at(key)
    }

    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        self.store.heap_parts()
    }

    /// `None` if no id of the node's group of 32 has links yet; otherwise the node's value, unset if the node itself
    /// never had links (`InlineIdStore`). Assigns the node an id, as Go's `TryGet` does.
    #[inline]
    pub fn try_get(&self, node: P<Node>) -> Option<&V> {
        self.store.try_get(ast::get_node_id(node).0)
    }

    /// `try_get` that returns `None` for a node without an id instead of assigning one (no side effect).
    #[inline]
    pub fn try_get_if_id_assigned(&self, node: P<Node>) -> Option<&V> {
        self.store.try_get(ast::get_assigned_node_id(node)? as u64)
    }
}

/// Go `symbolArenaLinkStore`: keyed by the symbol id, so every access assigns the symbol its id
/// (`ast.GetSymbolId`) the way Go does. Symbol ids are observable (the internal names of unique-symbol-keyed
/// properties embed them, and the node builder counts their length toward truncation), so the assignment order
/// must match.
pub struct SymbolArenaLinkStore<V> {
    store: IdLinkStore<V>,
}

impl<V> Default for SymbolArenaLinkStore<V> {
    fn default() -> Self {
        SymbolArenaLinkStore { store: IdLinkStore::default() }
    }
}

impl<V: Default> SymbolArenaLinkStore<V> {
    #[inline]
    pub fn get_key(&mut self, symbol: P<Symbol>) -> ArenaKey<V> {
        self.store.get_key(ast::get_symbol_id(symbol).0)
    }

    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get(&mut self, symbol: P<Symbol>) -> &V {
        self.store.get(ast::get_symbol_id(symbol).0)
    }
}

impl<V> SymbolArenaLinkStore<V> {
    #[inline]
    pub fn at(&self, key: ArenaKey<V>) -> &V {
        self.store.at(key)
    }

    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        self.store.heap_parts()
    }

    #[inline]
    pub fn try_get(&self, symbol: P<Symbol>) -> Option<&V> {
        self.store.try_get(ast::get_symbol_id(symbol).0)
    }

    /// `try_get` that returns `None` for a symbol without an id instead of assigning one (no side effect, no call:
    /// for fast paths whose fallback does the `get`).
    #[inline]
    pub fn try_get_if_id_assigned(&self, symbol: P<Symbol>) -> Option<&V> {
        let id = ast::get_assigned_symbol_id(symbol)?;
        self.store.try_get(id as u64)
    }

    #[inline]
    pub fn has(&self, symbol: P<Symbol>) -> bool {
        self.store.has(ast::get_symbol_id(symbol).0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A narrow group stores slot offsets from its first slot in 16 bits. A group that gets a link again after more
    // than 2^16 other links must get its dense form and keep every earlier id's slot.
    #[test]
    fn narrow_groups_turn_dense_without_losing_slots() {
        let mut store: IdLinkStore<Cell<u64>> = IdLinkStore::default();
        let mut ids: Vec<u64> = vec![5, 7];
        ids.extend((0..70_000u64).map(|i| 128 + i));
        ids.extend([9, 127]);
        for &id in &ids {
            store.get(id).set(id + 1);
        }
        assert!(store.index[0].as_ref().unwrap().dense.as_ref().is_some());
        assert!(store.index[1].as_ref().unwrap().dense.as_ref().is_none());
        for &id in &ids {
            assert_eq!(store.try_get(id).map(|v| (*v).get()), Some(id + 1), "id {id}");
        }
        for id in [0u64, 6, 8, 10, 126, 70_200] {
            assert!(store.try_get(id).is_none(), "id {id}");
        }
    }

    fn scrambled(mut ids: Vec<u64>, seed: u64) -> Vec<u64> {
        let mut x = seed;
        for i in (1..ids.len()).rev() {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ids.swap(i, (x >> 33) as usize % (i + 1));
        }
        ids
    }

    // Inline values: an id must reach its own cell through both levels wherever it falls in its group and block, and a
    // retained keys must keep selecting their values while later IDs grow the block index and value vectors.
    // An ID outside every allocated group reads as absent.
    #[test]
    fn inline_ids_reach_their_own_owned_cells() {
        let mut store: InlineIdStore<Cell<u32>> = InlineIdStore::default();
        let mut ids: Vec<u64> = (0..4000u64).map(|i| (i * 7919) % 300_000).collect();
        ids.extend([0, 1, 31, 32, 33, 1023, 1024, 1025, 65_535, 65_536, 65_537, (1 << 20) - 1, 1 << 20, (1 << 24) + 5]);
        let ids = scrambled(ids, 12345);
        let mut expected: FxHashMap<u64, u32> = FxHashMap::default();
        let mut held = Vec::new();
        for (n, &id) in ids.iter().enumerate() {
            let value = store.get(id);
            if value.get() == 0 {
                value.set(n as u32 + 1);
                expected.insert(id, n as u32 + 1);
            }
            held.push((id, store.get_key(id)));
        }
        for &(id, key) in &held {
            assert_eq!(store.at(key).get(), expected[&id], "id {id} changed identity");
        }
        for id in (0..300_000u64).chain([(1 << 20) - 1, 1 << 20, (1 << 24) + 5]) {
            assert_eq!(store.try_get(id).map_or(0, |v| v.get()), expected.get(&id).copied().unwrap_or(0), "id {id}");
        }
        assert!(store.try_get(5_000_000).is_none());
        assert!(store.try_get((1 << 24) + 64).is_none()); // same block as (1 << 24) + 5, another group
        assert!(store.try_get(1 << 40).is_none());
        store.get(1 << 40).set(7);
        assert_eq!(store.try_get(1 << 40).map(|v| v.get()), Some(7));
    }

    // The block index grows by a quarter, once per new top block: values written before every growth must still be
    // found after it, through the new index.
    #[test]
    fn inline_index_growth_keeps_every_value() {
        let mut store: InlineIdStore<Cell<u32>> = InlineIdStore::default();
        for b in 0..3000u64 {
            store.get(b * 1024 + b % 1024).set(b as u32 + 1);
        }
        for b in 0..3000u64 {
            assert_eq!(store.try_get(b * 1024 + b % 1024).map(|v| v.get()), Some(b as u32 + 1), "block {b}");
            assert_eq!(store.try_get(b * 1024 + (b + 32) % 1024).map(|v| v.get()), None, "block {b}");
        }
    }

    #[derive(Default)]
    struct Keyed {
        n: Cell<u32>,
        key: Cell<PKey>,
    }

    impl KeyedLinks for Keyed {
        fn link_key(&self) -> &Cell<PKey> {
            &self.key
        }
    }

    // The table holds only indexes and finds a key in its record: through ~15 rehashes (60,000 keys
    // from an empty table), every retained key must keep reaching its own record, a
    // key never inserted must miss (about one probe in 128 meets a matching 7-bit tag and must be rejected by the key
    // in the value), and the value must hold its key.
    #[test]
    fn keyed_store_finds_each_key_through_rehashes() {
        let keys: Vec<P<u64>> = (0..60_000u64).map(P::new).collect();
        let order: Vec<u64> = scrambled((0..keys.len() as u64).collect(), 777);
        let mut store: KeyedLinkStore<u64, Keyed> = KeyedLinkStore::default();
        let mut held = vec![None; keys.len()];
        for &i in &order {
            let value = store.get_key(keys[i as usize]);
            assert_eq!(store.at(value).n.get(), 0);
            store.at(value).n.set(i as u32 + 1);
            held[i as usize] = Some(value);
        }
        for (i, &key) in keys.iter().enumerate() {
            let retained = held[i].unwrap();
            assert_eq!(store.get_key(key), retained, "key {i} changed identity");
            let value = store.try_get(key).unwrap();
            assert!(std::ptr::eq(value, store.at(retained)));
            assert_eq!(value.n.get(), i as u32 + 1);
            assert_eq!(value.key.get(), key.key());
        }
        for absent in (0..10_000u64).map(P::new) {
            assert!(store.try_get(absent).is_none());
            assert!(!store.has(absent));
        }
    }

    // Reference kinds live in the slot: meanings must accumulate per symbol through the table's growth, and a symbol
    // never referenced must read as `None` without getting a record.
    #[test]
    fn reference_kinds_accumulate_in_their_slots() {
        let symbols: Vec<P<Symbol>> = (0..20_000).map(|_| P::new(Symbol::default())).collect();
        let mut store = SymbolReferenceLinkStore::default();
        for (i, &symbol) in symbols.iter().enumerate().filter(|(i, _)| i % 3 != 0) {
            store.add_reference_kinds(symbol, SymbolFlags::Value);
            if i % 2 == 0 {
                store.add_reference_kinds(symbol, SymbolFlags::Type);
            }
        }
        for (i, &symbol) in symbols.iter().enumerate() {
            let expected = match (i % 3 != 0, i % 2 == 0) {
                (false, _) => SymbolFlags::None,
                (true, false) => SymbolFlags::Value,
                (true, true) => SymbolFlags::Value | SymbolFlags::Type,
            };
            assert_eq!(store.reference_kinds(symbol), expected, "symbol {i}");
        }
        assert_eq!(store.slots.len(), symbols.len() - symbols.len().div_ceil(3));
    }

    // A wrong index hands out another id's links: the checker would read a foreign symbol's type without any visible
    // error. Every id must map to the slot it was given, in a scrambled order across many groups.
    #[test]
    fn groups_map_ids_to_their_slots() {
        let mut store: IdLinkStore<Cell<u32>> = IdLinkStore::default();
        let mut ids: Vec<u64> = (0..3000u64).map(|i| (i * 7919) % 40_000).collect();
        let mut x = 12345u64;
        for i in (1..ids.len()).rev() {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ids.swap(i, (x >> 33) as usize % (i + 1));
        }
        let mut expected: FxHashMap<u64, u32> = FxHashMap::default();
        for (n, &id) in ids.iter().enumerate() {
            let links = store.get(id);
            if links.get() == 0 {
                links.set(n as u32 + 1);
                expected.insert(id, n as u32 + 1);
            }
        }
        for id in 0..40_000u64 {
            assert_eq!(store.try_get(id).map(|v| (*v).get()), expected.get(&id).copied(), "id {id}");
        }
        assert!(store.try_get(1 << 40).is_none());
        store.get(1 << 40).set(7);
        assert_eq!(store.try_get(1 << 40).map(|v| (*v).get()), Some(7));
    }
}
