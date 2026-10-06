use crate::*;
use tsrs_core::{PKey, PSlot};

/// All Go link stores (`core.LinkStore`, `nodeLinkStore`, `symbolArenaLinkStore`) map to this one type.
/// Values live in the arena, so `get` hands out a `Copy` pointer whose `Cell` fields are mutated in place.
///
/// Keyed by the key's identity like Go's `map[K]*V`. A slot is the key (`P::key`: its handle with compressed pointers,
/// else its address) and the value's index (8 or 12 bytes, 4-aligned) instead of two pointers, and the values live in fixed-size arena chunks in first-access order (stable
/// addresses, as before). 5.1M links in 26 stores on the private monorepo single.
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

/// Links keyed by a node/symbol id (Go `PagedLinkStore`-backed stores). Like Go, the id is looked up by indexing,
/// not hashing: a group of `ID_GROUP` consecutive ids holds 2 bytes per id (the slot's offset from the group's first
/// slot, 0 = no links), found through a vector indexed by group number (one `Option<P<IdGroup>>` per group of the id
/// space below the highest id seen). The groups and the values live in the arena (the values in fixed-size chunks,
/// in first-access order; stable addresses, `P<V>` handed out as before).
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
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        self.store.heap_parts()
    }

    #[inline]
    pub fn try_get(&self, node: P<Node>) -> Option<P<V>> {
        self.store.try_get(ast::get_node_id(node).0)
    }

    #[inline]
    pub fn has(&self, node: P<Node>) -> bool {
        self.store.has(ast::get_node_id(node).0)
    }

    /// `try_get` that returns `None` for a node without an id instead of assigning one (no side effect).
    #[inline]
    pub fn try_get_if_id_assigned(&self, node: P<Node>) -> Option<P<V>> {
        let id = ast::get_assigned_node_id(node)?;
        self.store.narrow_slot(id).map(|slot| self.store.at(slot))
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
