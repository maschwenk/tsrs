use crate::*;

/// All Go link stores (`core.LinkStore`, `nodeLinkStore`, `symbolArenaLinkStore`) map to this one type.
/// Values live in the arena, so `get` hands out a `Copy` pointer whose `Cell` fields are mutated in place.
///
/// Keyed by the key's address like Go's `map[K]*V`. A slot is the key address and the value's index (12 bytes,
/// 4-aligned) instead of two pointers, and the values live in fixed-size arena chunks in first-access order (stable
/// addresses, as before). 5.1M links in 26 stores on the private monorepo single.
pub struct LinkStore<K: 'static, V: 'static> {
    slots: hashbrown::HashTable<LinkSlot>,
    chunks: Vec<&'static [V]>,
    len: u32,
    key: std::marker::PhantomData<P<K>>,
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
struct LinkSlot {
    key: usize, // the key's address
    index: u32, // the value's position in `chunks`
}

const _: () = assert!(std::mem::size_of::<LinkSlot>() == 12);

const LINK_CHUNK_SHIFT: u32 = 10;
const LINK_CHUNK: usize = 1 << LINK_CHUNK_SHIFT;

impl<K: 'static, V: 'static> Default for LinkStore<K, V> {
    fn default() -> Self {
        LinkStore { slots: hashbrown::HashTable::new(), chunks: Vec::new(), len: 0, key: std::marker::PhantomData }
    }
}

impl<K: 'static, V: 'static> LinkStore<K, V> {
    #[inline]
    fn hash(key: usize) -> u64 {
        use std::hash::BuildHasher;
        rustc_hash::FxBuildHasher.hash_one(key)
    }

    #[inline]
    fn address(key: P<K>) -> usize {
        (key.get() as *const K).addr()
    }

    #[inline]
    fn at(&self, index: u32) -> P<V> {
        debug_assert!(index < self.len);
        // SAFETY: every stored index is below `len`, and `chunks` holds `LINK_CHUNK` values per started chunk.
        let chunk = unsafe { self.chunks.get_unchecked((index >> LINK_CHUNK_SHIFT) as usize) };
        P::from_static(unsafe { chunk.get_unchecked(index as usize & (LINK_CHUNK - 1)) })
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
                    self.chunks.push(alloc_vec((0..LINK_CHUNK).map(|_| V::default()).collect()));
                }
                self.len += 1;
                index
            }
        };
        self.at(index)
    }
}

/// Links keyed by a node/symbol id (Go `PagedLinkStore`-backed stores). Like Go, the id is looked up in pages of
/// `ID_PAGE` consecutive ids (4 bytes per id: slot + 1, 0 = no links), found by indexing a vector by page number
/// (8 bytes per page of the id space below the highest id seen, ~0.2 MB for the private monorepo's 26M symbol ids; pages without
/// links stay unallocated). The values live in fixed-size chunks in the arena (stable addresses, `P<V>` handed out
/// as before), in first-access order.
pub struct IdLinkStore<V: 'static> {
    pages: Vec<Option<Box<[u32; ID_PAGE]>>>,
    wide_slots: FxHashMap<u64, u32>, // ids >= 2^32 (long-running processes such as the test runner)
    chunks: Vec<&'static [V]>,
    len: u32,
}

const ID_LINK_CHUNK_SHIFT: u32 = 12;
const ID_LINK_CHUNK: usize = 1 << ID_LINK_CHUNK_SHIFT;
const ID_PAGE_SHIFT: u32 = 10;
const ID_PAGE: usize = 1 << ID_PAGE_SHIFT;

impl<V: 'static> Default for IdLinkStore<V> {
    fn default() -> Self {
        IdLinkStore { pages: Vec::new(), wide_slots: FxHashMap::default(), chunks: Vec::new(), len: 0 }
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
            let page = self.pages.get((id >> ID_PAGE_SHIFT) as usize)?.as_deref()?;
            page[id as usize & (ID_PAGE - 1)].checked_sub(1)
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
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get(&mut self, id: u64) -> P<V> {
        if let Some(slot) = self.slot(id) {
            return self.at(slot);
        }
        self.create(id)
    }

    #[inline(never)]
    #[cfg_attr(feature = "site-counts", track_caller)]
    fn create(&mut self, id: u64) -> P<V> {
        tsrs_core::sitecount::hit("links", std::any::type_name::<V>());
        let slot = self.len;
        if slot as usize % ID_LINK_CHUNK == 0 {
            self.chunks.push(alloc_vec((0..ID_LINK_CHUNK).map(|_| V::default()).collect()));
        }
        self.len += 1;
        if id <= u32::MAX as u64 {
            let page_index = (id >> ID_PAGE_SHIFT) as usize;
            if page_index >= self.pages.len() {
                self.pages.resize_with(page_index + 1, || None);
            }
            let page = self.pages[page_index].get_or_insert_with(|| Box::new([0; ID_PAGE]));
            page[id as usize & (ID_PAGE - 1)] = slot + 1;
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
    #[cfg_attr(feature = "site-counts", track_caller)]
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
    #[cfg_attr(feature = "site-counts", track_caller)]
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
