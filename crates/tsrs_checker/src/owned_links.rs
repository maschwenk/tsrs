//! Indexed checker link storage; no references outlive a borrow of this store.
#![forbid(unsafe_code)]

use tsrs_core::arena_owner::{ArenaBuilder, ArenaKey, LocalKey};
use tsrs_core::{P, PKey};

/// Checker-owned links, indexed by the identity of their AST or symbol key.
///
/// Records are ordinary Rust values in first-access order. Access borrows the store; callers that recurse into
/// the checker keep a typed storage key and resolve again afterwards, so vector growth cannot invalidate a reference.
/// The key side still uses legacy identities until the AST and symbol stores migrate.
pub struct LinkStore<K: 'static, V> {
    slots: hashbrown::HashTable<LinkSlot<V>>,
    values: ArenaBuilder<V>,
    key: std::marker::PhantomData<fn() -> K>,
}

#[repr(C, packed(4))]
struct LinkSlot<V> {
    key: PKey,
    index: LocalKey<V>,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<LinkSlot<()>>() == 12);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<LinkSlot<()>>() == 8);

impl<K: 'static, V> Default for LinkStore<K, V> {
    fn default() -> Self {
        Self {
            slots: hashbrown::HashTable::new(),
            values: ArenaBuilder::new(),
            key: std::marker::PhantomData,
        }
    }
}

impl<K: 'static, V> LinkStore<K, V> {
    #[inline]
    fn hash(key: PKey) -> u64 {
        use std::hash::BuildHasher;
        rustc_hash::FxBuildHasher.hash_one(key)
    }

    #[inline]
    fn index(&self, key: P<K>) -> Option<LocalKey<V>> {
        let key = key.key();
        self.slots
            .find(Self::hash(key), |slot| ({ slot.key }) == key)
            .map(|slot| slot.index)
    }

    #[inline]
    pub fn try_get(&self, key: P<K>) -> Option<&V> {
        self.index(key)
            .and_then(|index| self.values.get_local(index))
    }

    #[inline]
    pub fn has(&self, key: P<K>) -> bool {
        self.index(key).is_some()
    }

    /// Resolve a retained key after a recursive checker call. Foreign keys cannot select another store's value.
    #[inline]
    pub fn at(&self, key: ArenaKey<V>) -> &V {
        self.values
            .get(key)
            .expect("link key belongs to another store")
    }

    /// Both the index and the records are owned heap storage.
    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        use crate::heapcensus::HeapSize;
        let slot = std::mem::size_of::<V>() as u64;
        let values = crate::heapcensus::HeapStat {
            containers: 1,
            len: self.values.len() as u64,
            cap: self.values.capacity() as u64,
            slot,
            bytes: self.values.capacity() as u64 * slot,
        };
        vec![("slots", self.slots.heap_stat()), ("values", values)]
    }
}

impl<K: 'static, V: Default> LinkStore<K, V> {
    #[inline]
    pub fn get(&mut self, key: P<K>) -> &V {
        let key = self.get_key(key);
        self.at(key)
    }

    /// Returns the links for `key`, creating them on first use.
    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get_key(&mut self, key: P<K>) -> ArenaKey<V> {
        let key = key.key();
        let index = match self.slots.entry(
            Self::hash(key),
            |slot| ({ slot.key }) == key,
            |slot| Self::hash(slot.key),
        ) {
            hashbrown::hash_table::Entry::Occupied(slot) => slot.get().index,
            hashbrown::hash_table::Entry::Vacant(slot) => {
                tsrs_core::sitecount::hit("links", std::any::type_name::<V>());
                // Construct before publishing the index: a panicking Default leaves the table unchanged.
                let index = self.values.alloc(V::default()).local();
                slot.insert(LinkSlot { key, index });
                index
            }
        };
        self.values
            .qualify(index)
            .expect("link index belongs to this store")
    }
}

/// Compact link table for values that already carry their lookup key. Hash buckets hold four-byte local IDs;
/// values and borrowed access obey the same ownership rules as `LinkStore`.
pub struct KeyedLinkStore<K: 'static, V> {
    slots: hashbrown::HashTable<LocalKey<V>>,
    values: ArenaBuilder<V>,
    key: std::marker::PhantomData<fn() -> K>,
}

impl<K: 'static, V> Default for KeyedLinkStore<K, V> {
    fn default() -> Self {
        Self {
            slots: hashbrown::HashTable::new(),
            values: ArenaBuilder::new(),
            key: std::marker::PhantomData,
        }
    }
}

impl<K: 'static, V: crate::KeyedLinks> KeyedLinkStore<K, V> {
    #[inline]
    fn hash(key: PKey) -> u64 {
        use std::hash::BuildHasher;
        rustc_hash::FxBuildHasher.hash_one(key)
    }

    #[inline]
    pub fn try_get(&self, key: P<K>) -> Option<&V> {
        let key = key.key();
        let index = self.slots.find(Self::hash(key), |&index| {
            self.values
                .get_local(index)
                .expect("stored link index")
                .link_key()
                .get()
                == key
        })?;
        self.values.get_local(*index)
    }

    #[inline]
    pub fn has(&self, key: P<K>) -> bool {
        self.try_get(key).is_some()
    }

    #[inline]
    pub fn at(&self, key: ArenaKey<V>) -> &V {
        self.values
            .get(key)
            .expect("link key belongs to another store")
    }

    pub fn heap_parts(&self) -> Vec<(&'static str, crate::heapcensus::HeapStat)> {
        use crate::heapcensus::HeapSize;
        let slot = std::mem::size_of::<V>() as u64;
        let values = crate::heapcensus::HeapStat {
            containers: 1,
            len: self.values.len() as u64,
            cap: self.values.capacity() as u64,
            slot,
            bytes: self.values.capacity() as u64 * slot,
        };
        vec![("index slots", self.slots.heap_stat()), ("values", values)]
    }
}

impl<K: 'static, V: crate::KeyedLinks + Default> KeyedLinkStore<K, V> {
    #[inline]
    pub fn get(&mut self, key: P<K>) -> &V {
        let key = self.get_key(key);
        self.at(key)
    }

    #[inline]
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub fn get_key(&mut self, key: P<K>) -> ArenaKey<V> {
        let key = key.key();
        let values = &self.values;
        let index = match self.slots.entry(
            Self::hash(key),
            |&index| {
                values
                    .get_local(index)
                    .expect("stored link index")
                    .link_key()
                    .get()
                    == key
            },
            |&index| {
                Self::hash(
                    values
                        .get_local(index)
                        .expect("stored link index")
                        .link_key()
                        .get(),
                )
            },
        ) {
            hashbrown::hash_table::Entry::Occupied(slot) => *slot.get(),
            hashbrown::hash_table::Entry::Vacant(slot) => {
                tsrs_core::sitecount::hit("links", std::any::type_name::<V>());
                let value = V::default();
                value.link_key().set(key);
                let index = self.values.alloc(value).local();
                slot.insert(index);
                index
            }
        };
        self.values
            .qualify(index)
            .expect("link index belongs to this store")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    static KEYS: [u32; 4096] = [0; 4096];

    #[test]
    fn retained_keys_survive_growth_and_rehashing() {
        let mut store: LinkStore<u32, Cell<u32>> = LinkStore::default();
        let first = store.get_key(P::from_static(&KEYS[0]));
        store.at(first).set(42);
        for (i, key) in KEYS.iter().enumerate().skip(1) {
            store.get(P::from_static(key)).set(i as u32);
        }
        assert_eq!(store.at(first).get(), 42);
        assert_eq!(store.get_key(P::from_static(&KEYS[0])), first);
        for (i, key) in KEYS.iter().enumerate().skip(1) {
            assert_eq!(store.try_get(P::from_static(key)).unwrap().get(), i as u32);
        }
    }

    #[test]
    #[should_panic(expected = "link key belongs to another store")]
    fn foreign_storage_keys_are_rejected() {
        let mut first: LinkStore<u32, Cell<u32>> = LinkStore::default();
        let key = first.get_key(P::from_static(&KEYS[0]));
        let mut second: LinkStore<u32, Cell<u32>> = LinkStore::default();
        second.get(P::from_static(&KEYS[0]));
        second.at(key);
    }

    #[test]
    fn resource_fields_drop_with_the_store() {
        #[derive(Default)]
        struct Links(Cell<Option<Rc<()>>>);
        let value = Rc::new(());
        let weak = Rc::downgrade(&value);
        let mut store: LinkStore<u32, Links> = LinkStore::default();
        store.get(P::from_static(&KEYS[0])).0.set(Some(value));
        assert!(weak.upgrade().is_some());
        drop(store);
        assert!(weak.upgrade().is_none());
    }
}
