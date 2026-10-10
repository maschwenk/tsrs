//! Hash maps of small `Copy` keys and values whose slots are packed to 4-byte alignment (notes/mem-checker-heap.md).
//!
//! The checker's caches keyed by a `CacheHashKey` (Go's 128-bit `xxh3` key) store a 16-byte key and a 4-byte type
//! handle; as a `(K, V)` tuple in an `FxHashMap` that slot is padded to 24 bytes. `PackedMap` stores the same pair in
//! 20 bytes, and hashes a `CacheHashKey` by its own low word (it is already a hash) instead of running Fx over both
//! words. Same contents, same lookups; only the table's layout differs. Nothing iterates these maps in an order
//! that reaches output (`values` is for counts and tests).

use crate::*;

/// A key `PackedMap` can hash.
pub trait PackedKey: Copy + Eq {
    fn packed_hash(&self) -> u64;
}

impl PackedKey for CacheHashKey {
    #[inline]
    fn packed_hash(&self) -> u64 {
        // The key is the output of xxh3-128: its low word is already uniformly distributed.
        self.lo
    }
}

impl PackedKey for CachedSignatureKey {
    #[inline]
    fn packed_hash(&self) -> u64 {
        use std::hash::BuildHasher;
        // Hash the qualified key; storage addresses are no longer signature identities.
        self.key.lo ^ rustc_hash::FxBuildHasher.hash_one(self.sig)
    }
}

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
struct PackedSlot<K, V> {
    key: K,
    value: V,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<PackedSlot<CacheHashKey, P<Type>>>() == 24);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<PackedSlot<CacheHashKey, P<Type>>>() == 20);

pub struct PackedMap<K, V> {
    table: hashbrown::HashTable<PackedSlot<K, V>>,
}

impl<K, V> Default for PackedMap<K, V> {
    fn default() -> Self {
        PackedMap { table: hashbrown::HashTable::new() }
    }
}

impl<K: PackedKey, V: Copy> PackedMap<K, V> {
    pub fn new() -> Self {
        PackedMap::default()
    }

    /// A map holding one entry, with the capacity `FxHashMap::from_iter([(key, value)])` has.
    pub fn from_one(key: K, value: V) -> Self {
        let mut m = PackedMap { table: hashbrown::HashTable::with_capacity(1) };
        m.insert(key, value);
        m
    }

    #[inline]
    pub fn get(&self, key: &K) -> Option<V> {
        let key = *key;
        self.table.find(key.packed_hash(), |slot| ({ slot.key }) == key).map(|slot| slot.value)
    }

    #[inline]
    pub fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    /// Like `HashMap::insert`: returns the value the key had.
    #[inline]
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self.table.entry(key.packed_hash(), |slot| ({ slot.key }) == key, |slot| ({ slot.key }).packed_hash()) {
            hashbrown::hash_table::Entry::Occupied(mut slot) => {
                let old = slot.get().value;
                slot.get_mut().value = value;
                Some(old)
            }
            hashbrown::hash_table::Entry::Vacant(slot) => {
                slot.insert(PackedSlot { key, value });
                None
            }
        }
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.table.capacity()
    }

    pub fn clear(&mut self) {
        self.table.clear();
    }

    /// The values, in table order (counts and tests only: the order is not observable).
    pub fn values(&self) -> impl Iterator<Item = V> + '_ {
        self.table.iter().map(|slot| slot.value)
    }

    pub(crate) fn heap_stat(&self) -> crate::heapcensus::HeapStat {
        crate::heapcensus::HeapStat::table(self.table.len(), self.table.capacity(), std::mem::size_of::<PackedSlot<K, V>>())
    }
}

/// Go `c.stringLiteralTypes` (`map[string]*Type`): the literal types by their value. The key was a heap `String`
/// copy of the text the literal type already owns; the table stores only the type and compares its value.
#[derive(Default)]
pub struct StringLiteralTypes {
    table: hashbrown::HashTable<P<Type>>,
}

impl StringLiteralTypes {
    #[inline]
    fn hash(value: &str) -> u64 {
        use std::hash::BuildHasher;
        rustc_hash::FxBuildHasher.hash_one(value)
    }

    #[inline]
    fn value_of(t: &Type) -> std::cell::Ref<'_, str> {
        std::cell::Ref::map(t.as_literal_type().value.borrow(), |value| match value.as_ref() {
            Some(LiteralValue::String(s)) => s.as_ref(),
            _ => unreachable!("string literal cache entry without a string value"),
        })
    }

    #[inline]
    pub fn get(&self, value: &str) -> Option<P<Type>> {
        self.table.find(Self::hash(value), |&t| &*Self::value_of(&t) == value).copied()
    }

    /// Adds a string literal type whose value is not in the table yet.
    pub fn insert_new(&mut self, t: P<Type>) {
        let value = Self::value_of(&t);
        debug_assert!(self.get(&value).is_none());
        self.table.insert_unique(Self::hash(&value), t, |&t| Self::hash(&Self::value_of(&t)));
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    pub(crate) fn heap_stat(&self) -> crate::heapcensus::HeapStat {
        crate::heapcensus::HeapStat::table(self.table.len(), self.table.capacity(), std::mem::size_of::<P<Type>>())
    }
}
