use crate::*;
use tsrs_core::{PKey, PSlot};

/// All Go link stores (`core.LinkStore`, `nodeLinkStore`, `symbolArenaLinkStore`) map to this one type.
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

const _: () = assert!(std::mem::size_of::<LinkSlot>() == if tsrs_core::COMPRESSED_PTRS { 8 } else { 12 });

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

const _: () = assert!(std::mem::size_of::<ReferenceKindsSlot>() == if tsrs_core::COMPRESSED_PTRS { 8 } else { 16 });

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

/// Links keyed by a node/symbol id (Go `PagedLinkStore`-backed stores). Like Go, the id is looked up by indexing,
/// not hashing: a group of `ID_GROUP` consecutive ids holds 2 bytes per id (the slot's offset from the group's first
/// slot, 0 = no links), found through a vector indexed by group number (one `Option<P<IdGroup>>` per group of the id
/// space below the highest id seen). The groups and the values live in the arena (the values in fixed-size chunks,
/// in first-access order). Stable addresses: chunks are never reallocated, freed or recycled, so a `P<V>` stays valid
/// for the store's lifetime (`value_symbol_links` callers keep links across other accesses).
///
/// Node and symbol ids come from process-wide counters in per-thread blocks of 1,024, so with several checkers one
/// checker's links are dense in its own blocks and spread thinly over the other checkers' (every checker touches
/// most lib symbols, whichever checker numbered them). Groups are small (128 ids, 264 bytes) so that a thinly used
/// block costs little: the 1,024-id pages they replace were 14% full on vscode at 64 checkers, and the ids-to-slots
/// tables took 3.3 MB per checker (notes/mem-64.md). A group whose offsets outgrow 16 bits (a link made more than
/// 65,535 links after the group's first) gets a 4-byte-per-id dense form, as the narrow pages did.
pub struct IdLinkStore<V: 'static> {
    index: Vec<Option<P<IdGroup>>>,
    wide_slots: FxHashMap<u64, u32>, // ids >= 2^32 (long-running processes such as the test runner)
    chunks: Vec<P<PSlot<V>>>,        // first slot of each chunk
    len: u32,
}

/// Slots as `slot - before` (0 = no links); `before` is one less than the first slot the group got (wrapping; slots
/// only grow). When an offset does not fit, `dense` points at the 4-byte form (`slot + 1`, 0 = no links), which is
/// read instead of `slots` from then on.
#[repr(C)]
struct IdGroup {
    before: u32,
    dense: Cell<Option<P<[Cell<u32>; ID_GROUP]>>>,
    slots: [Cell<u16>; ID_GROUP],
}

const _: () = assert!(std::mem::size_of::<IdGroup>() == if tsrs_core::COMPRESSED_PTRS { 264 } else { 272 });

/// Kept for embedders (tsrslint) and the pool, which chose sparse id pages for multi-checker runs: the groups above
/// made the sparse form unnecessary, so these are inert and `TSRS_SPARSE_ID_PAGES` is ignored.
pub fn set_multiple_checkers(_multiple: bool) {}

/// See `set_multiple_checkers`.
pub fn set_sparse_id_pages(_on: bool) {}

const ID_LINK_CHUNK_SHIFT: u32 = 12;
const ID_LINK_CHUNK: usize = 1 << ID_LINK_CHUNK_SHIFT;
const ID_GROUP_SHIFT: u32 = 7;
const ID_GROUP: usize = 1 << ID_GROUP_SHIFT;

impl<V: 'static> Default for IdLinkStore<V> {
    fn default() -> Self {
        IdLinkStore { index: Vec::new(), wide_slots: FxHashMap::default(), chunks: Vec::new(), len: 0 }
    }
}

impl<V: 'static> IdLinkStore<V> {
    #[inline]
    fn at(&self, slot: u32) -> P<V> {
        debug_assert!(slot < self.len);
        #[expect(clippy::disallowed_methods, reason = "measured with the other link-store lookups (see LinkStore::at)")]
        // SAFETY: every stored slot is below `len`; chunks hold `ID_LINK_CHUNK` values per started chunk.
        unsafe { PSlot::nth(*self.chunks.get_unchecked((slot >> ID_LINK_CHUNK_SHIFT) as usize), slot as usize & (ID_LINK_CHUNK - 1)) }
    }

    #[inline]
    fn slot(&self, id: u64) -> Option<u32> {
        match u32::try_from(id) {
            Ok(id) => self.narrow_slot(id),
            Err(_) => self.wide_slot(id),
        }
    }

    #[inline]
    fn narrow_slot(&self, id: u32) -> Option<u32> {
        let group = (*self.index.get((id >> ID_GROUP_SHIFT) as usize)?)?;
        let i = id as usize & (ID_GROUP - 1);
        // Narrow groups first: nearly all groups are narrow.
        if let Some(dense) = group.dense.get() {
            return Self::dense_slot(dense, i);
        }
        match group.slots[i].get() {
            0 => None,
            offset => Some(group.before.wrapping_add(offset as u32)),
        }
    }

    #[inline(never)]
    fn dense_slot(dense: P<[Cell<u32>; ID_GROUP]>, i: usize) -> Option<u32> {
        dense[i].get().checked_sub(1)
    }

    // Out of line: keeps the hash lookup out of every inlined `get`.
    #[cold]
    #[inline(never)]
    fn wide_slot(&self, id: u64) -> Option<u32> {
        self.wide_slots.get(&id).copied()
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

impl<V: 'static> IdLinkStore<V> {
    /// Heap census: the group index and the wide-id map (the groups and the values are in the arena; the groups are
    /// reported with their arena bytes so that the id-to-slot cost stays visible).
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        use crate::heapcensus::{HeapSize, HeapStat};
        let mut narrow = HeapStat { slot: 2, ..HeapStat::default() };
        let mut dense = HeapStat { slot: 4, ..HeapStat::default() };
        for group in self.index.iter().flatten() {
            narrow.containers += 1;
            narrow.cap += ID_GROUP as u64;
            narrow.bytes += std::mem::size_of::<IdGroup>() as u64;
            if let Some(d) = group.dense.get() {
                dense.containers += 1;
                dense.cap += ID_GROUP as u64;
                dense.bytes += std::mem::size_of::<[Cell<u32>; ID_GROUP]>() as u64;
                dense.len += d.iter().filter(|s| s.get() != 0).count() as u64;
            } else {
                narrow.len += group.slots.iter().filter(|s| s.get() != 0).count() as u64;
            }
        }
        vec![
            ("group index", self.index.heap_stat()),
            ("narrow groups (arena)", narrow),
            ("dense groups (arena)", dense),
            ("wide ids", self.wide_slots.heap_stat()),
            ("chunk list", self.chunks.heap_stat()),
        ]
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
            self.chunks.push(PSlot::first(alloc_vec((0..ID_LINK_CHUNK).map(|_| PSlot(V::default())).collect())));
        }
        self.len += 1;
        if id <= u32::MAX as u64 {
            let group_index = (id >> ID_GROUP_SHIFT) as usize;
            if group_index >= self.index.len() {
                // Grow by a quarter, not double: ids keep arriving in every checker's blocks for the whole run, so the
                // index ends near the top of the id space in every checker (a checker's index is a few hundred KB).
                let need = group_index + 1;
                if need > self.index.capacity() {
                    self.index.reserve_exact((need - self.index.len()).max(self.index.len() / 4));
                }
                self.index.resize(need, None);
            }
            let i = id as usize & (ID_GROUP - 1);
            let group = *self.index[group_index].get_or_insert_with(|| {
                P::new(IdGroup { before: slot.wrapping_sub(1), dense: Cell::new(None), slots: [const { Cell::new(0) }; ID_GROUP] })
            });
            if let Some(dense) = group.dense.get() {
                dense[i].set(slot + 1);
            } else {
                match u16::try_from(slot.wrapping_sub(group.before)) {
                    Ok(offset) if offset != 0 => group.slots[i].set(offset),
                    _ => {
                        let dense: [Cell<u32>; ID_GROUP] = std::array::from_fn(|j| {
                            Cell::new(match group.slots[j].get() {
                                0 => 0,
                                offset => group.before.wrapping_add(offset as u32) + 1,
                            })
                        });
                        dense[i].set(slot + 1);
                        group.dense.set(Some(P::new(dense)));
                    }
                }
            }
        } else {
            self.wide_slots.insert(id, slot);
        }
        self.at(slot)
    }
}

/// Links keyed by a node or symbol id whose value is small (a 4-byte handle), stored in the table itself: no slot, no
/// offset, no separate value chunk. Ids map through two levels that follow the per-thread id blocks of 1,024
/// (`ast::use_id_blocks`): `blocks[id >> 10]` points at an arena table of 32 group pointers, and a group is an arena
/// array of 32 values (`[V; 32]`, 128 bytes for a 4-byte `V`), allocated on the first access to one of its ids and
/// default (unset) until written. A checker's own id blocks fill their groups; the blocks other checkers numbered get
/// a 128-byte table and only the groups they touch (notes/mem-dense-link-tables.md: 0.98 / 0.67 of a group's ids have
/// links at 1 / 32 checkers on vscode, against 0.98 / 0.41 for 128-id groups).
///
/// Stable addresses: groups and tables are arena objects that are never freed, moved or recycled (like `IdLinkStore`'s
/// groups and chunks), so a value handed out stays where it is for the store's lifetime. Callers keep the reference
/// across later accesses that create other links (`get_resolved_symbol` holds it across `resolve_name`). Values are
/// handed out as `&'static V`, not `P<V>`: a cell of a 4-byte value is 4-aligned, and a `P` handle names 8-byte units.
///
/// Unlike `IdLinkStore`, the store cannot tell an id whose links were never created from one whose links are unset:
/// both read as `V::default()`. That is the same answer for every reader of a value whose fields are all unset by
/// default (`SymbolNodeLinks`).
pub struct InlineIdStore<V: 'static> {
    blocks: Vec<Option<P<InlineBlock<V>>>>,
    wide: FxHashMap<u64, P<V>>, // ids >= 2^32 (long-running processes such as the test runner)
}

const INLINE_GROUP_SHIFT: u32 = 5;
const INLINE_GROUP: usize = 1 << INLINE_GROUP_SHIFT;
/// One second-level table per id block (`ast::use_id_blocks` takes 1,024 ids at a time).
const INLINE_BLOCK_SHIFT: u32 = 10;
const INLINE_BLOCK_GROUPS: usize = 1 << (INLINE_BLOCK_SHIFT - INLINE_GROUP_SHIFT);

type InlineGroup<V> = [V; INLINE_GROUP];
type InlineBlock<V> = [Cell<Option<P<InlineGroup<V>>>>; INLINE_BLOCK_GROUPS];

impl<V: 'static> Default for InlineIdStore<V> {
    fn default() -> Self {
        InlineIdStore { blocks: Vec::new(), wide: FxHashMap::default() }
    }
}

impl<V: 'static> InlineIdStore<V> {
    #[inline]
    fn narrow(&self, id: u32) -> Option<&'static V> {
        let block = (*self.blocks.get((id >> INLINE_BLOCK_SHIFT) as usize)?)?;
        let group = block.get()[(id >> INLINE_GROUP_SHIFT) as usize & (INLINE_BLOCK_GROUPS - 1)].get()?;
        Some(&group.get()[id as usize & (INLINE_GROUP - 1)])
    }

    #[inline]
    pub fn try_get(&self, id: u64) -> Option<&'static V> {
        match u32::try_from(id) {
            Ok(id) => self.narrow(id),
            Err(_) => self.wide_get(id),
        }
    }

    // Out of line: keeps the hash lookup out of every inlined `try_get`.
    #[cold]
    #[inline(never)]
    fn wide_get(&self, id: u64) -> Option<&'static V> {
        self.wide.get(&id).map(|v| v.get())
    }

    /// Heap census: the block index (the tables and groups are in the arena; reported with their arena bytes so that
    /// the id-to-value cost stays visible).
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        use crate::heapcensus::{HeapSize, HeapStat};
        let mut tables = HeapStat { slot: 4, ..HeapStat::default() };
        let mut groups = HeapStat { slot: std::mem::size_of::<V>() as u64, ..HeapStat::default() };
        for block in self.blocks.iter().flatten() {
            tables.containers += 1;
            tables.cap += INLINE_BLOCK_GROUPS as u64;
            tables.bytes += std::mem::size_of::<InlineBlock<V>>() as u64;
            for group in block.get().iter().filter_map(|g| g.get()) {
                tables.len += 1;
                groups.containers += 1;
                groups.len += INLINE_GROUP as u64;
                groups.cap += INLINE_GROUP as u64;
                groups.bytes += std::mem::size_of_val(group.get()) as u64;
            }
        }
        vec![
            ("block index", self.blocks.heap_stat()),
            ("block tables (arena)", tables),
            ("value groups (arena)", groups),
            ("wide ids", self.wide.heap_stat()),
        ]
    }
}

impl<V: Default + 'static> InlineIdStore<V> {
    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get(&mut self, id: u64) -> &'static V {
        if let Ok(narrow) = u32::try_from(id) {
            if let Some(value) = self.narrow(narrow) {
                return value;
            }
        }
        self.create(id)
    }

    /// The value of an id whose group does not exist yet (or a wide id): allocates the block table and the group.
    #[inline(never)]
    #[cfg_attr(feature = "site-counts", track_caller)]
    fn create(&mut self, id: u64) -> &'static V {
        tsrs_core::sitecount::hit("links", std::any::type_name::<V>());
        let Ok(id) = u32::try_from(id) else {
            return self.wide.entry(id).or_insert_with(|| P::new(V::default())).get();
        };
        let b = (id >> INLINE_BLOCK_SHIFT) as usize;
        if b >= self.blocks.len() {
            // Grow by a quarter, not double: ids keep arriving in every checker's blocks for the whole run, so the
            // index ends near the top of the id space in every checker (4 bytes per 1,024 ids).
            let need = b + 1;
            if need > self.blocks.capacity() {
                self.blocks.reserve_exact((need - self.blocks.len()).max(self.blocks.len() / 4));
            }
            self.blocks.resize(need, None);
        }
        let block = *self.blocks[b].get_or_insert_with(|| P::new(std::array::from_fn(|_| Cell::new(None))));
        let cell = &block.get()[(id >> INLINE_GROUP_SHIFT) as usize & (INLINE_BLOCK_GROUPS - 1)];
        let group = match cell.get() {
            Some(group) => group,
            None => {
                let group = P::new(std::array::from_fn(|_| V::default()));
                cell.set(Some(group));
                group
            }
        };
        &group.get()[id as usize & (INLINE_GROUP - 1)]
    }
}

/// Go `nodeLinkStore`: keyed by the node id, so every access assigns the node its id (`ast.GetNodeId`) the way Go
/// does. Ids are observable (e.g. in cache keys and internal names), so the assignment order must match. The values
/// are stored inline (`InlineIdStore`): its one user, `symbol_node_links`, holds a 4-byte symbol handle per node.
pub struct NodeLinkStore<V: 'static> {
    store: InlineIdStore<V>,
}

impl<V: 'static> Default for NodeLinkStore<V> {
    fn default() -> Self {
        NodeLinkStore { store: InlineIdStore::default() }
    }
}

impl<V: Default + 'static> NodeLinkStore<V> {
    /// The node's links, created on first use. The reference stays valid for the store's lifetime (`InlineIdStore`).
    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get(&mut self, node: P<Node>) -> &'static V {
        self.store.get(ast::get_node_id(node).0)
    }
}

impl<V: 'static> NodeLinkStore<V> {
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        self.store.heap_parts()
    }

    /// `None` if no id of the node's group of 32 has links yet; otherwise the node's value, unset if the node itself
    /// never had links (`InlineIdStore`). Assigns the node an id, as Go's `TryGet` does.
    #[inline]
    pub fn try_get(&self, node: P<Node>) -> Option<&'static V> {
        self.store.try_get(ast::get_node_id(node).0)
    }

    /// `try_get` that returns `None` for a node without an id instead of assigning one (no side effect).
    #[inline]
    pub fn try_get_if_id_assigned(&self, node: P<Node>) -> Option<&'static V> {
        self.store.narrow(ast::get_assigned_node_id(node)?)
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
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        self.store.heap_parts()
    }

    #[inline]
    pub fn try_get(&self, symbol: P<Symbol>) -> Option<P<V>> {
        self.store.try_get(ast::get_symbol_id(symbol).0)
    }

    /// `try_get` that returns `None` for a symbol without an id instead of assigning one (no side effect, no call:
    /// for fast paths whose fallback does the `get`).
    #[inline]
    pub fn try_get_if_id_assigned(&self, symbol: P<Symbol>) -> Option<P<V>> {
        let id = ast::get_assigned_symbol_id(symbol)?;
        self.store.narrow_slot(id).map(|slot| self.store.at(slot))
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
        assert!(store.index[0].unwrap().dense.get().is_some());
        assert!(store.index[1].unwrap().dense.get().is_none());
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
    // cell must stay where it is while later ids allocate tables and groups and grow the block index (callers keep
    // the reference across other accesses). An id outside every allocated group reads as absent.
    #[test]
    fn inline_ids_reach_their_own_stable_cells() {
        let mut store: InlineIdStore<Cell<u32>> = InlineIdStore::default();
        let mut ids: Vec<u64> = (0..4000u64).map(|i| (i * 7919) % 300_000).collect();
        ids.extend([0, 1, 31, 32, 33, 1023, 1024, 1025, 65_535, 65_536, 65_537, (1 << 20) - 1, 1 << 20, (1 << 24) + 5]);
        let ids = scrambled(ids, 12345);
        let mut expected: FxHashMap<u64, u32> = FxHashMap::default();
        let mut held: Vec<(u64, *const Cell<u32>)> = Vec::new();
        for (n, &id) in ids.iter().enumerate() {
            let value = store.get(id);
            if value.get() == 0 {
                value.set(n as u32 + 1);
                expected.insert(id, n as u32 + 1);
            }
            held.push((id, std::ptr::from_ref(value)));
        }
        for &(id, address) in &held {
            assert!(std::ptr::eq(store.get(id), address), "id {id} moved");
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
            if links.get().get() == 0 {
                links.set(n as u32 + 1);
                expected.insert(id, n as u32 + 1);
            }
        }
        for id in 0..40_000u64 {
            assert_eq!(store.try_get(id).map(|v| (*v).get()), expected.get(&id).copied(), "id {id}");
        }
        assert_eq!(store.try_get(1 << 40), None);
        store.get(1 << 40).set(7);
        assert_eq!(store.try_get(1 << 40).map(|v| (*v).get()), Some(7));
    }
}
