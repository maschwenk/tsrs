use crate::*;
use tsrs_core::{PKey, PSlot};

/// Go `core.LinkStore` keyed by node or symbol identity. Most of the checker's link stores use this type;
/// `KeyedLinkStore`, `SymbolReferenceLinkStore`, `NodeLinkStore` (Go `nodeLinkStore`) and `SymbolLinkStore`
/// (Go `symbolLinkStore`) below cover the rest. (The node builder's link stores use `tsrs_core::LinkStore`.)
/// Values live in the arena, so `get` hands out a `Copy` pointer whose `Cell` fields are mutated in place.
///
/// Keyed by the key's identity like Go's `map[K]*V`. A slot is the key (`P::key`: its handle with compressed pointers,
/// else its address) and the value's index (8 or 12 bytes, 4-aligned) instead of two pointers, and the values live in fixed-size arena chunks in first-access order (stable
/// addresses, as before). 5.1M links in 26 stores on the private monorepo single.
///
/// Stable addresses: a chunk is never reallocated, freed or recycled, so a `P<V>` handed out stays valid for the
/// store's lifetime. Callers rely on it: they keep links across accesses that create others, and
/// `run_without_resolved_signature_caching` keeps them in a vector across a nested check.
pub struct LinkStore<K: 'static, V: 'static> {
    slots: hashbrown::HashTable<LinkSlot>,
    chunks: Vec<P<PSlot<V>>>, // first slot of each chunk
    len: u32,
    key: std::marker::PhantomData<P<K>>,
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
struct LinkSlot {
    key: PKey,
    index: u32, // the value's position in `chunks`
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<LinkSlot>() == if tsrs_core::COMPRESSED_PTRS { 8 } else { 12 });
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<LinkSlot>() == 8);

const LINK_CHUNK_SHIFT: u32 = 10;
const LINK_CHUNK: usize = 1 << LINK_CHUNK_SHIFT;

impl<K: 'static, V: 'static> Default for LinkStore<K, V> {
    fn default() -> Self {
        LinkStore { slots: hashbrown::HashTable::new(), chunks: Vec::new(), len: 0, key: std::marker::PhantomData }
    }
}

impl<K: 'static, V: 'static> LinkStore<K, V> {
    #[inline]
    fn hash(key: PKey) -> u64 {
        use std::hash::BuildHasher;
        rustc_hash::FxBuildHasher.hash_one(key)
    }

    #[inline]
    fn address(key: P<K>) -> PKey {
        key.key()
    }

    #[inline]
    fn at(&self, index: u32) -> P<V> {
        debug_assert!(index < self.len);
        #[expect(
            clippy::disallowed_methods,
            reason = "bounds checks on the three link-store lookups: +0.6% instructions, four checkers (notes/lint-paydown-compiler.md)"
        )]
        // SAFETY: every stored index is below `len`, and `chunks` holds `LINK_CHUNK` values per started chunk.
        let chunk = unsafe { *self.chunks.get_unchecked((index >> LINK_CHUNK_SHIFT) as usize) };
        // SAFETY: `chunk` is the first slot of a `LINK_CHUNK`-slot array, and the offset is below `LINK_CHUNK`.
        unsafe { PSlot::nth(chunk, index as usize & (LINK_CHUNK - 1)) }
    }

    #[inline]
    fn index(&self, key: P<K>) -> Option<u32> {
        let key = Self::address(key);
        self.slots.find(Self::hash(key), |slot| ({ slot.key }) == key).map(|slot| slot.index)
    }

    #[inline]
    pub fn try_get(&self, key: P<K>) -> Option<P<V>> {
        self.index(key).map(|index| self.at(index))
    }

    #[inline]
    pub fn has(&self, key: P<K>) -> bool {
        self.index(key).is_some()
    }
}

impl<K: 'static, V: 'static> LinkStore<K, V> {
    /// Heap census: the slot table (the values are in the arena).
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        use crate::heapcensus::HeapSize;
        vec![("slots", self.slots.heap_stat()), ("chunk list", self.chunks.heap_stat())]
    }
}

impl<K: 'static, V: Default + 'static> LinkStore<K, V> {
    /// Returns the links for `key`, creating them on first use.
    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get(&mut self, key: P<K>) -> P<V> {
        let key = Self::address(key);
        let index = match self.slots.entry(Self::hash(key), |slot| ({ slot.key }) == key, |slot| Self::hash(slot.key)) {
            hashbrown::hash_table::Entry::Occupied(slot) => slot.get().index,
            hashbrown::hash_table::Entry::Vacant(slot) => {
                tsrs_core::sitecount::hit("links", std::any::type_name::<V>());
                let index = self.len;
                slot.insert(LinkSlot { key, index });
                if index as usize % LINK_CHUNK == 0 {
                    self.chunks.push(PSlot::first(alloc_vec((0..LINK_CHUNK).map(|_| PSlot(V::default())).collect())));
                }
                self.len += 1;
                index
            }
        };
        self.at(index)
    }
}

/// A link value that holds its own key, for `KeyedLinkStore`.
pub trait KeyedLinks {
    /// The key the store filed the value under (`P::key`); 0 until then. Only the store reads or writes it.
    fn link_key(&self) -> &Cell<PKey>;
}

/// `LinkStore` for values with room for their key: a bucket holds only the value's index (4 bytes plus hashbrown's
/// control byte, against 8 + 1), and a probe compares the key the value holds. `signature_links` and `type_node_links`
/// keep the key in what was padding (`SignatureLinks` 12 -> 16 bytes in a 16-byte slot, `TypeNodeLinks` 20 -> 24 in
/// 24, compressed pointers), so the values do not grow and the tables shrink by 4/9 (notes/mem-dense-link-tables.md:
/// 18.0 -> 10.0 MiB each on vscode at one checker). Go's `LinkStore` maps the key's identity the same way.
///
/// The key is written before the index is inserted, so a probe never sees a value without its key (and a fresh value's
/// key 0 is never a handle: the reservation never hands out offset 0). A hit loads the value the caller reads next; a
/// false positive of hashbrown's 7-bit tag costs one value load; a rehash reads each value's key once.
///
/// Stable addresses: as `LinkStore`, the values sit in arena chunks that are never reallocated, freed or recycled, so
/// a `P<V>` stays valid for the store's lifetime while the table of indexes grows and rehashes around it.
pub struct KeyedLinkStore<K: 'static, V: 'static> {
    slots: hashbrown::HashTable<u32>, // the value's position in `chunks`
    chunks: Vec<P<PSlot<V>>>,         // first slot of each chunk
    len: u32,
    key: std::marker::PhantomData<P<K>>,
}

impl<K: 'static, V: 'static> Default for KeyedLinkStore<K, V> {
    fn default() -> Self {
        KeyedLinkStore { slots: hashbrown::HashTable::new(), chunks: Vec::new(), len: 0, key: std::marker::PhantomData }
    }
}

/// The value at `index` of a store's chunks (`LinkStore::at`, as a function so that a probe closure can borrow the
/// chunks while the table is borrowed mutably).
#[inline]
fn keyed_at<V>(chunks: &[P<PSlot<V>>], index: u32) -> P<V> {
    #[expect(clippy::disallowed_methods, reason = "measured with the other link-store lookups (see LinkStore::at)")]
    // SAFETY: every stored index is below the store's `len`, and `chunks` holds `LINK_CHUNK` values per started chunk.
    let chunk = unsafe { *chunks.get_unchecked((index >> LINK_CHUNK_SHIFT) as usize) };
    // SAFETY: `chunk` is the first slot of a `LINK_CHUNK`-slot array, and the offset is below `LINK_CHUNK`.
    unsafe { PSlot::nth(chunk, index as usize & (LINK_CHUNK - 1)) }
}

impl<K: 'static, V: KeyedLinks + 'static> KeyedLinkStore<K, V> {
    #[inline]
    fn hash(key: PKey) -> u64 {
        use std::hash::BuildHasher;
        rustc_hash::FxBuildHasher.hash_one(key)
    }

    #[inline]
    pub fn try_get(&self, key: P<K>) -> Option<P<V>> {
        let key = key.key();
        let chunks = &self.chunks;
        self.slots.find(Self::hash(key), |&i| keyed_at(chunks, i).link_key().get() == key).map(|&i| keyed_at(chunks, i))
    }

    #[inline]
    pub fn has(&self, key: P<K>) -> bool {
        self.try_get(key).is_some()
    }

    /// Heap census: the table of indexes (the values are in the arena).
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        use crate::heapcensus::HeapSize;
        vec![("index slots", self.slots.heap_stat()), ("chunk list", self.chunks.heap_stat())]
    }
}

impl<K: 'static, V: KeyedLinks + Default + 'static> KeyedLinkStore<K, V> {
    /// Returns the links for `key`, creating them on first use.
    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get(&mut self, key: P<K>) -> P<V> {
        let key = key.key();
        let chunks = &self.chunks;
        match self.slots.entry(
            Self::hash(key),
            |&i| keyed_at(chunks, i).link_key().get() == key,
            |&i| Self::hash(keyed_at(chunks, i).link_key().get()),
        ) {
            hashbrown::hash_table::Entry::Occupied(slot) => keyed_at(&self.chunks, *slot.get()),
            hashbrown::hash_table::Entry::Vacant(slot) => {
                tsrs_core::sitecount::hit("links", std::any::type_name::<V>());
                let index = self.len;
                if index as usize % LINK_CHUNK == 0 {
                    self.chunks.push(PSlot::first(alloc_vec((0..LINK_CHUNK).map(|_| PSlot(V::default())).collect())));
                }
                self.len += 1;
                let value = keyed_at(&self.chunks, index);
                value.link_key().set(key);
                slot.insert(index);
                value
            }
        }
    }
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
const _: () = assert!(std::mem::size_of::<ReferenceKindsSlot>() == if tsrs_core::COMPRESSED_PTRS { 8 } else { 16 });
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

// When possible, nodeLinkStore and symbolLinkStore store Node and Symbol links in an efficient and densely
// packed paged array store. When a Node or Symbol has not yet been assigned an ID, the store provides one
// from a generator that produces sequences of IDs in a reserved range of the ID space. The generator grabs
// chunks of 256 IDs from a central atomic counter, ensuring that each block of 256 IDs is consecutive and
// causing Node or Symbol links to be densely packed within a single page.

const MAX_PAGE_LINK_COUNT: u64 = 0x100_0000; // 16M

#[inline]
fn page_link_id(id: u64) -> Option<u64> {
    id.checked_sub(ast::BLOCK_ID_OFFSET).filter(|&id| id < MAX_PAGE_LINK_COUNT)
}

// Kept for embedders and the pool; dense link stores choose their storage from the id's range.
pub fn set_multiple_checkers(_multiple: bool) {}
pub fn set_sparse_id_pages(_on: bool) {}

fn id_link_heap_parts<V>(links: &tsrs_core::LinkStore<u64, V>, pages: &tsrs_core::PagedLinkStore<V>) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
    use crate::heapcensus::HeapStat;
    let (len, cap) = links.storage_usage();
    let (page_len, page_cap) = pages.page_index_usage();
    let page_slot = std::mem::size_of::<Option<P<[Cell<Option<P<V>>>; tsrs_core::LINK_PAGE_SIZE]>>>() as u64;
    vec![
        ("fallback ids", HeapStat::table(len, cap, std::mem::size_of::<(u64, P<V>)>())),
        ("page index", HeapStat { containers: 1, len: page_len as u64, cap: page_cap as u64, slot: page_slot, bytes: page_cap as u64 * page_slot }),
    ]
}

#[derive(Default)]
pub struct NodeLinkStore<V: 'static> {
    gen_: RefCell<ast::NodeIdGenerator>,
    links: tsrs_core::LinkStore<u64, V>,
    pages: tsrs_core::PagedLinkStore<V>,
}

impl<V: Default + 'static> NodeLinkStore<V> {
    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get(&mut self, node: P<Node>) -> P<V> {
        let id = self.gen_.get_mut().get_node_id(node).0;
        match page_link_id(id) {
            Some(id) => self.pages.get(id),
            None => self.links.get(id),
        }
    }
}

impl<V: 'static> NodeLinkStore<V> {
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        id_link_heap_parts(&self.links, &self.pages)
    }

    #[inline]
    pub fn try_get(&self, node: P<Node>) -> Option<P<V>> {
        let id = self.gen_.borrow_mut().get_node_id(node).0;
        self.try_get_id(id)
    }

    #[inline]
    pub fn has(&self, node: P<Node>) -> bool {
        self.try_get(node).is_some()
    }

    #[inline]
    fn try_get_id(&self, id: u64) -> Option<P<V>> {
        match page_link_id(id) {
            Some(id) => self.pages.try_get(id),
            None => self.links.try_get(id),
        }
    }

    // Fast paths may look up existing links without assigning an id.
    #[inline]
    pub fn try_get_if_id_assigned(&self, node: P<Node>) -> Option<P<V>> {
        self.try_get_id(ast::get_assigned_node_id(node)?)
    }
}

#[derive(Default)]
pub struct SymbolLinkStore<V: 'static> {
    gen_: RefCell<ast::SymbolIdGenerator>,
    links: tsrs_core::LinkStore<u64, V>,
    pages: tsrs_core::PagedLinkStore<V>,
}

impl<V: Default + 'static> SymbolLinkStore<V> {
    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get(&mut self, symbol: P<Symbol>) -> P<V> {
        let id = self.gen_.get_mut().get_symbol_id(symbol).0;
        match page_link_id(id) {
            Some(id) => self.pages.get(id),
            None => self.links.get(id),
        }
    }
}

impl<V: 'static> SymbolLinkStore<V> {
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        id_link_heap_parts(&self.links, &self.pages)
    }

    #[inline]
    pub fn try_get(&self, symbol: P<Symbol>) -> Option<P<V>> {
        let id = self.gen_.borrow_mut().get_symbol_id(symbol).0;
        self.try_get_id(id)
    }

    #[inline]
    fn try_get_id(&self, id: u64) -> Option<P<V>> {
        match page_link_id(id) {
            Some(id) => self.pages.try_get(id),
            None => self.links.try_get(id),
        }
    }

    // Fast paths may look up existing links without assigning an id.
    #[inline]
    pub fn try_get_if_id_assigned(&self, symbol: P<Symbol>) -> Option<P<V>> {
        self.try_get_id(ast::get_assigned_symbol_id(symbol)?)
    }

    #[inline]
    pub fn has(&self, symbol: P<Symbol>) -> bool {
        self.try_get(symbol).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_stores_keep_unallocated_links_absent() {
        let factory = ast::NodeFactory::default();
        let node = factory.new_token(Kind::ThisKeyword);
        let symbol = Symbol::new(SymbolFlags::None, "dense");
        let mut nodes: NodeLinkStore<Cell<u32>> = NodeLinkStore::default();
        let mut symbols: SymbolLinkStore<Cell<u32>> = SymbolLinkStore::default();
        assert!(nodes.try_get_if_id_assigned(node).is_none());
        assert!(symbols.try_get_if_id_assigned(symbol).is_none());
        assert!(ast::get_assigned_node_id(node).is_none());
        assert!(ast::get_assigned_symbol_id(symbol).is_none());
        assert!(!nodes.has(node));
        assert!(!symbols.has(symbol));
        assert!(ast::get_node_id(node).0 >= ast::BLOCK_ID_OFFSET);
        assert!(ast::get_symbol_id(symbol).0 >= ast::BLOCK_ID_OFFSET);
        let node_links = nodes.get(node);
        let symbol_links = symbols.get(symbol);
        node_links.set(3);
        symbol_links.set(5);
        let next_node = factory.new_token(Kind::ThisKeyword);
        let next_symbol = Symbol::new(SymbolFlags::None, "next");
        assert!(nodes.try_get(next_node).is_none());
        assert!(symbols.try_get(next_symbol).is_none());
        for _ in 0..1000 {
            nodes.get(factory.new_token(Kind::ThisKeyword));
            symbols.get(Symbol::new(SymbolFlags::None, "growth"));
        }
        assert!(nodes.try_get_if_id_assigned(node) == Some(node_links));
        assert!(symbols.try_get_if_id_assigned(symbol) == Some(symbol_links));
        assert_eq!(node_links.get().get(), 3);
        assert_eq!(symbol_links.get().get(), 5);
        let mut other_nodes: NodeLinkStore<Cell<u32>> = NodeLinkStore::default();
        let mut other_symbols: SymbolLinkStore<Cell<u32>> = SymbolLinkStore::default();
        assert!(other_nodes.try_get(node).is_none());
        assert!(other_symbols.try_get(symbol).is_none());
        assert_eq!(other_nodes.get(node).get().get(), 0);
        assert_eq!(other_symbols.get(symbol).get().get(), 0);
    }

    #[test]
    fn preassigned_and_out_of_range_ids_use_fallback_links() {
        let node = ast::NodeFactory::default().new_token(Kind::ThisKeyword);
        let symbol = Symbol::new(SymbolFlags::None, "ordinary");
        let node_id = ast::get_node_id(node).0;
        let symbol_id = ast::get_symbol_id(symbol).0;
        let mut nodes: NodeLinkStore<Cell<u32>> = NodeLinkStore::default();
        let mut symbols: SymbolLinkStore<Cell<u32>> = SymbolLinkStore::default();
        nodes.get(node).set(7);
        symbols.get(symbol).set(11);
        assert!(nodes.links.has(node_id));
        assert!(symbols.links.has(symbol_id));
        assert_eq!(nodes.try_get(node).unwrap().get().get(), 7);
        assert_eq!(symbols.try_get(symbol).unwrap().get().get(), 11);
        let limit = ast::BLOCK_ID_OFFSET + MAX_PAGE_LINK_COUNT;
        assert_eq!(page_link_id(ast::BLOCK_ID_OFFSET - 1), None);
        assert_eq!(page_link_id(ast::BLOCK_ID_OFFSET), Some(0));
        assert_eq!(page_link_id(limit - 1), Some(MAX_PAGE_LINK_COUNT - 1));
        assert_eq!(page_link_id(limit), None);
        assert_eq!(page_link_id(u64::MAX), None);
        nodes.links.get(limit).set(13);
        symbols.links.get(limit).set(17);
        assert_eq!(nodes.try_get_id(limit).unwrap().get().get(), 13);
        assert_eq!(symbols.try_get_id(limit).unwrap().get().get(), 17);
    }

    fn scrambled(mut ids: Vec<u64>, seed: u64) -> Vec<u64> {
        let mut x = seed;
        for i in (1..ids.len()).rev() {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ids.swap(i, (x >> 33) as usize % (i + 1));
        }
        ids
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

    // The table holds only indexes and finds a key in the value it points at: through ~15 rehashes (60,000 keys
    // from an empty table, 59 chunks of values), every key must keep reaching its own value at its first address, a
    // key never inserted must miss (about one probe in 128 meets a matching 7-bit tag and must be rejected by the key
    // in the value), and the value must hold its key.
    #[test]
    fn keyed_store_finds_each_key_through_rehashes() {
        let keys: Vec<P<u64>> = (0..60_000u64).map(P::new).collect();
        let order: Vec<u64> = scrambled((0..keys.len() as u64).collect(), 777);
        let mut store: KeyedLinkStore<u64, Keyed> = KeyedLinkStore::default();
        let mut held: Vec<Option<P<Keyed>>> = vec![None; keys.len()];
        for &i in &order {
            let value = store.get(keys[i as usize]);
            assert_eq!(value.n.get(), 0);
            value.n.set(i as u32 + 1);
            held[i as usize] = Some(value);
        }
        for (i, &key) in keys.iter().enumerate() {
            let value = store.try_get(key).unwrap();
            assert!(Some(value) == held[i], "key {i} moved");
            assert!(store.get(key) == value);
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

}
