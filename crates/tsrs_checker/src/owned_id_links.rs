//! Rust-owned specialized link storage. Semantic IDs index compact groups; retained storage keys name an owner.
#![forbid(unsafe_code)]

use rustc_hash::FxHashMap;
use tsrs_core::arena_owner::{ArenaBuilder, ArenaKey, LocalKey};

const ID_GROUP_SHIFT: u32 = 7;
const ID_GROUP: usize = 1 << ID_GROUP_SHIFT;

/// Sparse ID groups preserve the measured two-byte offset representation. Both group data and records run
/// ordinary Rust destructors, and every record access borrows the store.
pub struct IdLinkStore<V> {
    pub(super) index: Vec<Option<Box<IdGroup>>>,
    wide_slots: FxHashMap<u64, u32>,
    values: ArenaBuilder<V>,
}

pub(super) struct IdGroup {
    before: u32,
    pub(super) dense: Option<Box<[u32; ID_GROUP]>>,
    slots: [u16; ID_GROUP],
}

impl<V> Default for IdLinkStore<V> {
    fn default() -> Self {
        Self { index: Vec::new(), wide_slots: FxHashMap::default(), values: ArenaBuilder::new() }
    }
}

impl<V> IdLinkStore<V> {
    #[inline]
    fn slot(&self, id: u64) -> Option<u32> {
        let Ok(id) = u32::try_from(id) else {
            return self.wide_slot(id);
        };
        let group = self.index.get((id >> ID_GROUP_SHIFT) as usize)?.as_ref()?;
        let i = id as usize & (ID_GROUP - 1);
        if let Some(dense) = &group.dense {
            return dense[i].checked_sub(1);
        }
        match group.slots[i] {
            0 => None,
            offset => Some(group.before.wrapping_add(offset as u32)),
        }
    }

    #[cold]
    #[inline(never)]
    fn wide_slot(&self, id: u64) -> Option<u32> {
        self.wide_slots.get(&id).copied()
    }

    #[inline]
    pub fn at(&self, key: ArenaKey<V>) -> &V {
        self.values.get(key).expect("link key belongs to another store")
    }

    #[inline]
    pub fn try_get(&self, id: u64) -> Option<&V> {
        let key = self.values.key_at(self.slot(id)? as usize)?;
        self.values.get(key)
    }

    #[inline]
    pub fn has(&self, id: u64) -> bool {
        self.slot(id).is_some()
    }

    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        use crate::heapcensus::{HeapSize, HeapStat};
        let mut narrow = HeapStat { slot: 2, ..HeapStat::default() };
        let mut dense = HeapStat { slot: 4, ..HeapStat::default() };
        for group in self.index.iter().flatten() {
            narrow.containers += 1;
            narrow.cap += ID_GROUP as u64;
            narrow.bytes += std::mem::size_of::<IdGroup>() as u64;
            if let Some(d) = &group.dense {
                dense.containers += 1;
                dense.cap += ID_GROUP as u64;
                dense.bytes += std::mem::size_of_val(&**d) as u64;
                dense.len += d.iter().filter(|&&s| s != 0).count() as u64;
            } else {
                narrow.len += group.slots.iter().filter(|&&s| s != 0).count() as u64;
            }
        }
        vec![
            ("group index", self.index.heap_stat()),
            ("narrow groups", narrow),
            ("dense groups", dense),
            ("wide ids", self.wide_slots.heap_stat()),
            ("values", values_heap(&self.values)),
        ]
    }
}

impl<V: Default> IdLinkStore<V> {
    #[inline]
    pub fn get(&mut self, id: u64) -> &V {
        let key = self.get_key(id);
        self.at(key)
    }

    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get_key(&mut self, id: u64) -> ArenaKey<V> {
        if let Some(slot) = self.slot(id) {
            return self.values.key_at(slot as usize).expect("stored link index");
        }
        self.create(id)
    }

    #[inline(never)]
    #[cfg_attr(feature = "site-counts", track_caller)]
    fn create(&mut self, id: u64) -> ArenaKey<V> {
        tsrs_core::sitecount::hit("links", std::any::type_name::<V>());
        let slot = u32::try_from(self.values.len()).expect("link slot space exhausted");
        // Default must succeed before any index points to the new record.
        let key = self.values.alloc(V::default());
        let Ok(id) = u32::try_from(id) else {
            self.wide_slots.insert(id, slot);
            return key;
        };
        let group_index = (id >> ID_GROUP_SHIFT) as usize;
        grow_index(&mut self.index, group_index);
        let group = self.index[group_index].get_or_insert_with(|| {
            Box::new(IdGroup { before: slot.wrapping_sub(1), dense: None, slots: [0; ID_GROUP] })
        });
        let i = id as usize & (ID_GROUP - 1);
        if let Some(dense) = &mut group.dense {
            dense[i] = slot + 1;
        } else {
            match u16::try_from(slot.wrapping_sub(group.before)) {
                Ok(offset) if offset != 0 => group.slots[i] = offset,
                _ => {
                    let mut dense = Box::new(std::array::from_fn(|j| match group.slots[j] {
                        0 => 0,
                        offset => group.before.wrapping_add(offset as u32) + 1,
                    }));
                    dense[i] = slot + 1;
                    group.dense = Some(dense);
                }
            }
        }
        key
    }
}

const INLINE_GROUP_SHIFT: u32 = 5;
const INLINE_GROUP: usize = 1 << INLINE_GROUP_SHIFT;
const INLINE_BLOCK_SHIFT: u32 = 10;
const INLINE_BLOCK_GROUPS: usize = 1 << (INLINE_BLOCK_SHIFT - INLINE_GROUP_SHIFT);
type InlineGroup<V> = [V; INLINE_GROUP];
type InlineBlock<V> = [Option<LocalKey<InlineGroup<V>>>; INLINE_BLOCK_GROUPS];

/// A retained inline record identity. Its private fields ensure offsets originate in the owning store.
pub struct InlineIdKey<V>(InlineLocation<V>);

enum InlineLocation<V> {
    Group { key: ArenaKey<InlineGroup<V>>, offset: u8 },
    Wide(ArenaKey<V>),
}

impl<V> Copy for InlineLocation<V> {}
impl<V> Clone for InlineLocation<V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<V> Copy for InlineIdKey<V> {}
impl<V> Clone for InlineIdKey<V> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Two-level sparse groups own their values. A borrow cannot outlive the store or overlap allocation.
pub struct InlineIdStore<V> {
    blocks: Vec<Option<Box<InlineBlock<V>>>>,
    groups: ArenaBuilder<InlineGroup<V>>,
    wide: FxHashMap<u64, LocalKey<V>>,
    wide_values: ArenaBuilder<V>,
}

impl<V> Default for InlineIdStore<V> {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            groups: ArenaBuilder::new(),
            wide: FxHashMap::default(),
            wide_values: ArenaBuilder::new(),
        }
    }
}

impl<V> InlineIdStore<V> {
    #[inline]
    fn group(&self, id: u32) -> Option<LocalKey<InlineGroup<V>>> {
        let block = self.blocks.get((id >> INLINE_BLOCK_SHIFT) as usize)?.as_ref()?;
        block[(id >> INLINE_GROUP_SHIFT) as usize & (INLINE_BLOCK_GROUPS - 1)]
    }

    #[inline]
    pub fn at(&self, key: InlineIdKey<V>) -> &V {
        match key.0 {
            InlineLocation::Group { key, offset } => {
                &self.groups.get(key).expect("link key belongs to another store")[offset as usize]
            }
            InlineLocation::Wide(key) => self.wide_values.get(key).expect("link key belongs to another store"),
        }
    }

    #[inline]
    pub fn try_get(&self, id: u64) -> Option<&V> {
        let Ok(id) = u32::try_from(id) else {
            return self.wide_get(id);
        };
        let group = self.groups.get_local(self.group(id)?)?;
        Some(&group[id as usize & (INLINE_GROUP - 1)])
    }

    #[cold]
    #[inline(never)]
    fn wide_get(&self, id: u64) -> Option<&V> {
        self.wide_values.get_local(*self.wide.get(&id)?)
    }

    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        use crate::heapcensus::{HeapSize, HeapStat};
        let mut tables = HeapStat { slot: 4, ..HeapStat::default() };
        for block in self.blocks.iter().flatten() {
            tables.containers += 1;
            tables.cap += INLINE_BLOCK_GROUPS as u64;
            tables.bytes += std::mem::size_of_val(&**block) as u64;
            tables.len += block.iter().flatten().count() as u64;
        }
        vec![
            ("block index", self.blocks.heap_stat()),
            ("block tables", tables),
            ("value groups", values_heap(&self.groups)),
            ("wide ids", self.wide.heap_stat()),
            ("wide values", values_heap(&self.wide_values)),
        ]
    }
}

impl<V: Default> InlineIdStore<V> {
    #[inline]
    pub fn get(&mut self, id: u64) -> &V {
        let key = self.get_key(id);
        self.at(key)
    }

    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get_key(&mut self, id: u64) -> InlineIdKey<V> {
        let Ok(id) = u32::try_from(id) else {
            return self.wide_key(id);
        };
        let group = match self.group(id) {
            Some(group) => group,
            None => self.create_group(id),
        };
        InlineIdKey(InlineLocation::Group {
            key: self.groups.qualify(group).expect("stored inline group"),
            offset: (id as usize & (INLINE_GROUP - 1)) as u8,
        })
    }

    #[cold]
    #[inline(never)]
    #[cfg_attr(feature = "site-counts", track_caller)]
    fn wide_key(&mut self, id: u64) -> InlineIdKey<V> {
        let index = *self.wide.entry(id).or_insert_with(|| {
            tsrs_core::sitecount::hit("links", std::any::type_name::<V>());
            self.wide_values.alloc(V::default()).local()
        });
        InlineIdKey(InlineLocation::Wide(self.wide_values.qualify(index).expect("stored wide inline value")))
    }

    #[inline(never)]
    #[cfg_attr(feature = "site-counts", track_caller)]
    fn create_group(&mut self, id: u32) -> LocalKey<InlineGroup<V>> {
        tsrs_core::sitecount::hit("links", std::any::type_name::<V>());
        let values = std::array::from_fn(|_| V::default());
        let b = (id >> INLINE_BLOCK_SHIFT) as usize;
        grow_index(&mut self.blocks, b);
        let block = self.blocks[b].get_or_insert_with(|| Box::new([None; INLINE_BLOCK_GROUPS]));
        let group = self.groups.alloc(values).local();
        block[(id >> INLINE_GROUP_SHIFT) as usize & (INLINE_BLOCK_GROUPS - 1)] = Some(group);
        group
    }
}

fn grow_index<T>(index: &mut Vec<Option<T>>, at: usize) {
    if at >= index.len() {
        let need = at + 1;
        if need > index.capacity() {
            index.reserve_exact((need - index.len()).max(index.len() / 4));
        }
        index.resize_with(need, || None);
    }
}

fn values_heap<V>(values: &ArenaBuilder<V>) -> crate::heapcensus::HeapStat {
    let slot = std::mem::size_of::<V>() as u64;
    crate::heapcensus::HeapStat {
        containers: 1,
        len: values.len() as u64,
        cap: values.capacity() as u64,
        slot,
        bytes: values.capacity() as u64 * slot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[test]
    fn id_keys_survive_dense_conversion_and_wide_ids() {
        let mut store = IdLinkStore::<Cell<u32>>::default();
        let first = store.get_key(3);
        store.at(first).set(42);
        for id in 128..70_000 {
            store.get(id).set(id as u32);
        }
        store.get(5).set(43);
        assert!(store.index[0].as_ref().unwrap().dense.is_some());
        assert_eq!(store.at(first).get(), 42);
        assert_eq!(store.get_key(3), first);
        for id in [u32::MAX as u64 + 1, u64::MAX] {
            let key = store.get_key(id);
            store.at(key).set(44);
            assert_eq!(store.try_get(id).unwrap().get(), 44);
        }
    }

    #[test]
    fn specialized_stores_drop_narrow_and_wide_resources() {
        let value = Rc::new(());
        let weak = Rc::downgrade(&value);
        let mut ids = IdLinkStore::<RefCell<Option<Rc<()>>>>::default();
        let mut inline = InlineIdStore::<RefCell<Option<Rc<()>>>>::default();
        *ids.get(3).borrow_mut() = Some(Rc::clone(&value));
        *ids.get(u64::MAX).borrow_mut() = Some(Rc::clone(&value));
        *inline.get(3).borrow_mut() = Some(Rc::clone(&value));
        *inline.get(u64::MAX).borrow_mut() = Some(value);
        drop(ids);
        assert!(weak.upgrade().is_some());
        drop(inline);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    #[should_panic(expected = "link key belongs to another store")]
    fn foreign_id_key_is_rejected() {
        let mut first = IdLinkStore::<Cell<u32>>::default();
        let mut second = IdLinkStore::<Cell<u32>>::default();
        let key = first.get_key(3);
        second.get_key(3);
        second.at(key);
    }

    #[test]
    fn foreign_inline_keys_are_rejected_in_both_representations() {
        let mut first = InlineIdStore::<Cell<u32>>::default();
        let mut second = InlineIdStore::<Cell<u32>>::default();
        for id in [3, u64::MAX] {
            let key = first.get_key(id);
            second.get_key(id);
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| second.at(key))).is_err());
        }
    }

    #[test]
    fn partial_group_initialization_drops_values_and_publishes_no_key() {
        thread_local! {
            static CALLS: Cell<usize> = const { Cell::new(0) };
            static DROPS: Cell<usize> = const { Cell::new(0) };
        }
        struct Value;
        impl Default for Value {
            fn default() -> Self {
                let n = CALLS.get();
                CALLS.set(n + 1);
                assert!(n != 3, "abandoned construction");
                Self
            }
        }
        impl Drop for Value {
            fn drop(&mut self) {
                DROPS.set(DROPS.get() + 1);
            }
        }
        let mut store = InlineIdStore::<Value>::default();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.get_key(3))).is_err());
        assert_eq!(DROPS.get(), 3);
        assert!(store.try_get(3).is_none());
        store.get(3);
        assert_eq!(DROPS.get(), 3);
        drop(store);
        assert_eq!(DROPS.get(), 3 + INLINE_GROUP);
    }
}
