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

#[cfg(feature = "assignment-stats")]
impl<K: 'static, V: 'static> LinkStore<K, V> {
    /// The keys' addresses (census, notes/mem-shared-base.md).
    pub fn stats_keys(&self) -> Vec<u64> {
        self.slots.iter().map(|slot| ({ slot.key }) as u64).collect()
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
///
/// Node and symbol ids come from process-wide counters, so with several checkers one checker's ids are spread
/// thinly over the whole id space (on the private monorepo with 4 checkers each checker holds links for ~25% of
/// the ids of the pages it touches, and the pages cost ~4x what one checker's do). With `TSRS_SPARSE_ID_PAGES=1`
/// (notes/mem-shared-base.md) a page starts sparse: a bitmap of the ids present plus their slots in id order, and
/// becomes a dense page once it is `ID_PAGE_DENSE_AT` full. Slots and their first-access order are the same in both
/// forms; only the lookup structure differs.
pub struct IdLinkStore<V: 'static> {
    pages: Vec<Option<IdPage>>,
    wide_slots: FxHashMap<u64, u32>, // ids >= 2^32 (long-running processes such as the test runner)
    chunks: Vec<&'static [V]>,
    len: u32,
    sparse: bool,
}

enum IdPage {
    Dense(Box<[u32; ID_PAGE]>),
    Sparse(Box<SparseIdPage>),
}

/// The ids of a page that have links (bit `i % 64` of word `i / 64`), the number of set bits before each word, and
/// the slots of the present ids in id order.
struct SparseIdPage {
    bits: [u64; ID_PAGE / 64],
    before: [u16; ID_PAGE / 64],
    slots: Vec<u32>,
}

impl SparseIdPage {
    #[inline]
    fn rank(&self, i: usize) -> Option<usize> {
        let word = self.bits[i >> 6];
        let bit = 1u64 << (i & 63);
        (word & bit != 0).then(|| self.before[i >> 6] as usize + (word & (bit - 1)).count_ones() as usize)
    }

    #[inline]
    fn slot(&self, i: usize) -> Option<u32> {
        // SAFETY: `rank` counts set bits, and `slots` holds one entry per set bit.
        self.rank(i).map(|r| unsafe { *self.slots.get_unchecked(r) })
    }

    fn insert(&mut self, i: usize, slot: u32) {
        let w = i >> 6;
        let bit = 1u64 << (i & 63);
        debug_assert!(self.bits[w] & bit == 0);
        let rank = self.before[w] as usize + (self.bits[w] & (bit - 1)).count_ones() as usize;
        self.bits[w] |= bit;
        for before in &mut self.before[w + 1..] {
            *before += 1;
        }
        if self.slots.len() == self.slots.capacity() {
            // Grow by a quarter: the slack stays small next to the 4 bytes per entry.
            self.slots.reserve_exact((self.slots.len() / 4).max(8));
        }
        self.slots.insert(rank, slot);
    }

    fn to_dense(&self) -> Box<[u32; ID_PAGE]> {
        let mut dense = Box::new([0u32; ID_PAGE]);
        let mut r = 0;
        for (w, &word) in self.bits.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                dense[w * 64 + b] = self.slots[r] + 1;
                r += 1;
                bits &= bits - 1;
            }
        }
        dense
    }
}

/// A sparse page becomes dense at this many entries (its size is then about that of a dense page).
const ID_PAGE_DENSE_AT: usize = 768;

static MULTIPLE_CHECKERS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called by a checker pool before it creates its checkers: sparse pages pay off only when several checkers
/// share the id space (a single checker's pages are dense).
pub fn set_multiple_checkers(multiple: bool) {
    MULTIPLE_CHECKERS.store(multiple, std::sync::atomic::Ordering::Relaxed);
}

fn sparse_id_pages() -> bool {
    static SPARSE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    MULTIPLE_CHECKERS.load(std::sync::atomic::Ordering::Relaxed)
        && *SPARSE.get_or_init(|| std::env::var("TSRS_SPARSE_ID_PAGES").is_ok_and(|v| v == "1"))
}

const ID_LINK_CHUNK_SHIFT: u32 = 12;
const ID_LINK_CHUNK: usize = 1 << ID_LINK_CHUNK_SHIFT;
const ID_PAGE_SHIFT: u32 = 10;
const ID_PAGE: usize = 1 << ID_PAGE_SHIFT;

impl<V: 'static> Default for IdLinkStore<V> {
    fn default() -> Self {
        IdLinkStore { pages: Vec::new(), wide_slots: FxHashMap::default(), chunks: Vec::new(), len: 0, sparse: sparse_id_pages() }
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
            match self.pages.get((id >> ID_PAGE_SHIFT) as usize)?.as_ref()? {
                IdPage::Dense(page) => page[id as usize & (ID_PAGE - 1)].checked_sub(1),
                IdPage::Sparse(page) => page.slot(id as usize & (ID_PAGE - 1)),
            }
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

#[cfg(feature = "assignment-stats")]
impl<V: 'static> IdLinkStore<V> {
    /// The ids that have links (census, notes/mem-shared-base.md).
    pub fn stats_keys(&self) -> Vec<u64> {
        let mut keys: Vec<u64> = self.wide_slots.keys().copied().collect();
        for (page_index, page) in self.pages.iter().enumerate() {
            let page = match page {
                Some(IdPage::Dense(page)) => &**page,
                Some(IdPage::Sparse(page)) => &*page.to_dense(),
                None => continue,
            };
            for (i, &slot) in page.iter().enumerate() {
                if slot != 0 {
                    keys.push(((page_index << ID_PAGE_SHIFT) + i) as u64);
                }
            }
        }
        keys
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
            let sparse = self.sparse;
            let page = self.pages[page_index].get_or_insert_with(|| {
                if sparse {
                    IdPage::Sparse(Box::new(SparseIdPage { bits: [0; ID_PAGE / 64], before: [0; ID_PAGE / 64], slots: Vec::new() }))
                } else {
                    IdPage::Dense(Box::new([0; ID_PAGE]))
                }
            });
            let i = id as usize & (ID_PAGE - 1);
            match page {
                IdPage::Dense(page) => page[i] = slot + 1,
                IdPage::Sparse(sparse_page) => {
                    sparse_page.insert(i, slot);
                    if sparse_page.slots.len() >= ID_PAGE_DENSE_AT {
                        *page = IdPage::Dense(sparse_page.to_dense());
                    }
                }
            }
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

#[cfg(feature = "assignment-stats")]
impl<V: 'static> NodeLinkStore<V> {
    pub fn stats_keys(&self) -> Vec<u64> {
        self.store.stats_keys()
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

#[cfg(feature = "assignment-stats")]
impl<V: 'static> SymbolArenaLinkStore<V> {
    pub fn stats_keys(&self) -> Vec<u64> {
        self.store.stats_keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A wrong rank in a sparse page hands out another id's links: the checker would read a foreign symbol's type
    // without any visible error. Every id must map to the slot it was given, before and after densification.
    #[test]
    fn sparse_pages_map_ids_like_dense_pages() {
        let mut sparse: IdLinkStore<Cell<u32>> = IdLinkStore { sparse: true, ..IdLinkStore::default() };
        let mut dense: IdLinkStore<Cell<u32>> = IdLinkStore { sparse: false, ..IdLinkStore::default() };
        // Ids of two pages in a scrambled order: page 0 stays sparse (300 ids), page 3 becomes dense (900 ids).
        let mut ids: Vec<u64> = (0..300u64).map(|i| (i * 337) % 1024).chain((0..900u64).map(|i| 3 * 1024 + (i * 613) % 1024)).collect();
        let mut x = 12345u64;
        for i in (1..ids.len()).rev() {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ids.swap(i, (x >> 33) as usize % (i + 1));
        }
        for &id in &ids {
            let s = sparse.get(id);
            let d = dense.get(id);
            s.set(id as u32 + 1);
            d.set(id as u32 + 1);
        }
        assert!(matches!(sparse.pages[0], Some(IdPage::Sparse(_))));
        assert!(matches!(sparse.pages[3], Some(IdPage::Dense(_))));
        for id in 0..4 * 1024u64 {
            assert_eq!(sparse.slot(id), dense.slot(id), "id {id}");
            if let Some(links) = sparse.try_get(id) {
                assert_eq!(links.get().get(), id as u32 + 1);
            }
        }
    }
}
