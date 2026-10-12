//! Storage sharing for completed immutable payloads; semantic identities and resolution stay in each context.
//!
//! The batch owner keeps this store alive until its contexts and their results are no longer used. Shared records
//! live in shard-owned regions, never in the allocating context's arena, scratch region or a source-file region.
//! No record stores a checker-local type, symbol, signature, mapper or node. Dropping the store frees the records.

use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use hashbrown::HashTable;
use rustc_hash::FxHasher;
use tsrs_core::arena::Region;

use crate::*;

const SHARD_COUNT: usize = 16;

#[derive(Default)]
struct PayloadShard {
    region: Option<Region>,
    strings: HashTable<&'static str>,
    literals: HashTable<P<LiteralValue>>,
    template_texts: HashTable<&'static [&'static str]>,
    tuple_infos: HashTable<&'static [TupleElementInfo]>,
    tuple_metadata: HashTable<P<TupleMetadata>>,
    variances: HashTable<&'static [VarianceFlags]>,
    requested_bytes: u64,
    stored_bytes: u64,
    hits: u64,
    misses: u64,
}

impl PayloadShard {
    fn enter(&mut self) -> tsrs_core::arena::RegionScope {
        self.region
            .get_or_insert_with(|| Region::new_scratch(64 << 10))
            .enter()
    }

    fn record(&mut self, bytes: usize, hit: bool) {
        self.requested_bytes += bytes as u64;
        if hit {
            self.hits += 1;
        } else {
            self.misses += 1;
            self.stored_bytes += bytes as u64;
        }
    }
}

#[derive(Default, Debug)]
pub struct SharedPayloadStats {
    pub requested_bytes: u64,
    pub stored_bytes: u64,
    pub arena_allocated_bytes: u64,
    pub arena_used_bytes: u64,
    pub table_bytes: u64,
    pub hits: u64,
    pub misses: u64,
}

/// An exact-equality interner for one program. Hashes select candidates only; a collision never shares unequal data.
/// Disabled stores allocate in the current context, preserving the ownership of compatibility checkers.
pub struct SharedPayloadStore {
    shards: Option<Box<[Mutex<PayloadShard>; SHARD_COUNT]>>,
}

impl SharedPayloadStore {
    pub fn new(enabled: bool) -> Self {
        Self {
            shards: enabled.then(|| Box::new(std::array::from_fn(|_| Mutex::new(PayloadShard::default())))),
        }
    }

    fn shard(&self, hash: u64) -> Option<&Mutex<PayloadShard>> {
        self.shards
            .as_ref()
            .map(|shards| &shards[hash as usize & (SHARD_COUNT - 1)])
    }

    /// Shared payload requests and storage, excluding local fallbacks and all context-owned identity bindings.
    pub fn stats(&self) -> SharedPayloadStats {
        let mut stats = SharedPayloadStats::default();
        if let Some(shards) = &self.shards {
            for shard in shards.iter() {
                let shard = shard.lock().unwrap();
                stats.requested_bytes += shard.requested_bytes;
                stats.stored_bytes += shard.stored_bytes;
                stats.hits += shard.hits;
                stats.misses += shard.misses;
                stats.table_bytes += (shard.strings.allocation_size()
                    + shard.literals.allocation_size()
                    + shard.template_texts.allocation_size()
                    + shard.tuple_infos.allocation_size()
                    + shard.tuple_metadata.allocation_size()
                    + shard.variances.allocation_size())
                    as u64;
                if let Some(region) = &shard.region {
                    stats.arena_allocated_bytes += region.allocated_bytes() as u64;
                    stats.arena_used_bytes += region.used_bytes() as u64;
                }
            }
        }
        stats
    }

    pub(crate) fn alloc_string(&self, value: &str) -> &'static str {
        if value.is_empty() {
            return "";
        }
        let hash = payload_hash(value);
        let Some(shard) = self.shard(hash) else {
            return alloc_str(value);
        };
        let mut shard = shard.lock().unwrap();
        if let Some(&stored) = shard.strings.find(hash, |&stored| stored == value) {
            shard.record(value.len(), true);
            return stored;
        }
        shard.record(value.len(), false);
        let _region = shard.enter();
        let stored = alloc_str(value);
        shard
            .strings
            .insert_unique(hash, stored, |stored| payload_hash(*stored));
        stored
    }

    pub(crate) fn literal(&self, value: LiteralValue) -> P<LiteralValue> {
        let hash = literal_hash(value);
        let Some(shard) = self.shard(hash) else {
            return P::new(value);
        };
        let mut shard = shard.lock().unwrap();
        let bytes = std::mem::size_of::<LiteralValue>()
            + match value {
                LiteralValue::String(value) => value.len(),
                LiteralValue::BigInt(value) => value.base10_value.len(),
                _ => 0,
            };
        if let Some(&stored) = shard
            .literals
            .find(hash, |&stored| literal_equal(*stored, value))
        {
            shard.record(bytes, true);
            return stored;
        }
        shard.record(bytes, false);
        let _region = shard.enter();
        // The caller may have obtained these bytes from a checker or leaf-file region. Copy on the miss so the
        // shared record owns its entire payload. Never publish a reference into another context's storage.
        let value = match value {
            LiteralValue::String(value) => LiteralValue::String(alloc_str(value)),
            LiteralValue::BigInt(value) => LiteralValue::BigInt(PseudoBigInt {
                base10_value: alloc_str(value.base10_value),
                ..value
            }),
            value => value,
        };
        let stored = P::new(value);
        shard
            .literals
            .insert_unique(hash, stored, |stored| literal_hash(**stored));
        stored
    }

    /// Borrowed string constructor: a hit does not copy bytes into either the local arena or the shared store.
    pub(crate) fn string_literal(&self, value: &str) -> P<LiteralValue> {
        let hash = string_literal_hash(value);
        let Some(shard) = self.shard(hash) else {
            return P::new(LiteralValue::String(alloc_str(value)));
        };
        let mut shard = shard.lock().unwrap();
        let bytes = std::mem::size_of::<LiteralValue>() + value.len();
        if let Some(&stored) = shard.literals.find(
            hash,
            |&stored| matches!(*stored, LiteralValue::String(text) if text == value),
        ) {
            shard.record(bytes, true);
            return stored;
        }
        shard.record(bytes, false);
        let _region = shard.enter();
        let stored = P::new(LiteralValue::String(alloc_str(value)));
        shard
            .literals
            .insert_unique(hash, stored, |stored| literal_hash(**stored));
        stored
    }

    pub(crate) fn name_list(&self, values: Vec<&'static str>) -> &'static [&'static str] {
        if self.shards.is_none() {
            return alloc_vec(values);
        }
        self.string_list(&values)
    }

    pub(crate) fn string_list(&self, values: &[&str]) -> &'static [&'static str] {
        if values.is_empty() {
            return &[];
        }
        let hash = payload_hash(values);
        let Some(shard) = self.shard(hash) else {
            return alloc_vec(values.iter().map(|value| alloc_str(value)).collect());
        };
        let mut shard = shard.lock().unwrap();
        let bytes =
            std::mem::size_of_val(values) + values.iter().map(|value| value.len()).sum::<usize>();
        if let Some(&stored) = shard.template_texts.find(hash, |&stored| stored == values) {
            shard.record(bytes, true);
            return stored;
        }
        shard.record(bytes, false);
        let _region = shard.enter();
        let stored = alloc_vec(values.iter().map(|value| alloc_str(value)).collect());
        shard
            .template_texts
            .insert_unique(hash, stored, |stored| payload_hash(*stored));
        stored
    }

    pub(crate) fn tuple_infos(&self, values: &[TupleElementInfo]) -> &'static [TupleElementInfo] {
        if values.is_empty() {
            return &[];
        }
        // Labels are local identity bindings, including nodes in leaf regions. Only the pointer-free descriptor
        // can be shared; labeled tuples use the original allocation without a lookup or a shared reference.
        if values
            .iter()
            .any(|value| value.labeled_declaration.is_some())
        {
            return alloc_slice(values);
        }
        let hash = payload_hash(values);
        let Some(shard) = self.shard(hash) else {
            return alloc_slice(values);
        };
        let mut shard = shard.lock().unwrap();
        if let Some(&stored) = shard.tuple_infos.find(hash, |&stored| stored == values) {
            shard.record(std::mem::size_of_val(values), true);
            return stored;
        }
        shard.record(std::mem::size_of_val(values), false);
        let _region = shard.enter();
        let stored = alloc_slice(values);
        shard
            .tuple_infos
            .insert_unique(hash, stored, |stored| payload_hash(*stored));
        stored
    }

    pub(crate) fn tuple_metadata(&self, value: TupleMetadata) -> P<TupleMetadata> {
        let hash = payload_hash(&value);
        let Some(shard) = self.shard(hash) else {
            return P::new(value);
        };
        let mut shard = shard.lock().unwrap();
        if let Some(&stored) = shard.tuple_metadata.find(hash, |&stored| *stored == value) {
            shard.record(std::mem::size_of::<TupleMetadata>(), true);
            return stored;
        }
        shard.record(std::mem::size_of::<TupleMetadata>(), false);
        let _region = shard.enter();
        let stored = P::new(value);
        shard
            .tuple_metadata
            .insert_unique(hash, stored, |stored| payload_hash(stored.get()));
        stored
    }

    pub(crate) fn variances(&self, values: Vec<VarianceFlags>) -> &'static [VarianceFlags] {
        if values.is_empty() {
            return &[];
        }
        let hash = payload_hash(&values);
        let Some(shard) = self.shard(hash) else {
            return alloc_vec(values);
        };
        let mut shard = shard.lock().unwrap();
        if let Some(&stored) = shard.variances.find(hash, |&stored| stored == values) {
            shard.record(std::mem::size_of_val(values.as_slice()), true);
            return stored;
        }
        shard.record(std::mem::size_of_val(values.as_slice()), false);
        let _region = shard.enter();
        let stored = alloc_vec(values);
        shard
            .variances
            .insert_unique(hash, stored, |stored| payload_hash(*stored));
        stored
    }
}

fn payload_hash(value: &(impl Hash + ?Sized)) -> u64 {
    let mut hash = FxHasher::default();
    value.hash(&mut hash);
    hash.finish()
}

fn literal_hash(value: LiteralValue) -> u64 {
    let mut hash = FxHasher::default();
    std::mem::discriminant(&value).hash(&mut hash);
    match value {
        LiteralValue::String(value) => value.hash(&mut hash),
        // Storage equivalence preserves every bit: Number's semantic equality deliberately equates the signed
        // zeros and makes NaN unequal to itself, neither of which is the right equality for immutable storage.
        LiteralValue::Number(value) => value.0.to_bits().hash(&mut hash),
        LiteralValue::Boolean(value) => value.hash(&mut hash),
        LiteralValue::BigInt(value) => value.hash(&mut hash),
    }
    hash.finish()
}

fn string_literal_hash(value: &str) -> u64 {
    let mut hash = FxHasher::default();
    std::mem::discriminant(&LiteralValue::String("")).hash(&mut hash);
    value.hash(&mut hash);
    hash.finish()
}

fn literal_equal(left: LiteralValue, right: LiteralValue) -> bool {
    match (left, right) {
        (LiteralValue::Number(left), LiteralValue::Number(right)) => {
            left.0.to_bits() == right.0.to_bits()
        }
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_storage_without_merging_numeric_values() {
        let store = SharedPayloadStore::new(true);
        let positive = store.literal(LiteralValue::Number(Number(0.0)));
        let negative = store.literal(LiteralValue::Number(Number(-0.0)));
        assert_ne!(positive, negative);
        assert_eq!(positive, store.literal(LiteralValue::Number(Number(0.0))));
        let nan = LiteralValue::Number(Number(f64::from_bits(0x7ff8_0000_0000_0001)));
        assert_eq!(store.literal(nan), store.literal(nan));
        assert_ne!(
            store.literal(nan),
            store.literal(LiteralValue::Number(Number(f64::from_bits(
                0x7ff8_0000_0000_0002
            ))))
        );
        assert_ne!(
            store.literal(LiteralValue::String("1")),
            store.literal(LiteralValue::Number(Number(1.0)))
        );
    }

    #[test]
    fn concurrent_contexts_publish_one_complete_payload() {
        let store = SharedPayloadStore::new(true);
        std::thread::scope(|scope| {
            let store = &store;
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(move || {
                        let literal = store.literal(LiteralValue::String("member"));
                        let texts = store.string_list(&["prefix", "suffix"]);
                        let tuples = store.tuple_infos(&[TupleElementInfo {
                            flags: ElementFlags::Required,
                            labeled_declaration: None,
                        }]);
                        let variances = store.variances(vec![
                            VarianceFlags::Covariant,
                            VarianceFlags::Contravariant,
                        ]);
                        (
                            literal,
                            texts.as_ptr() as usize,
                            tuples.as_ptr() as usize,
                            variances.as_ptr() as usize,
                        )
                    })
                })
                .collect();
            let results: Vec<_> = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect();
            assert!(results.iter().all(|value| value == &results[0]));
        });
    }

    #[test]
    fn payloads_outlive_the_allocating_context_region() {
        let store = SharedPayloadStore::new(true);
        let literal;
        let texts;
        {
            let local = Region::new_scratch(4096);
            let _local = local.enter();
            literal = store.literal(LiteralValue::String(alloc_str("context-owned bytes")));
            texts = store.string_list(&[alloc_str("local prefix"), "suffix"]);
        }
        assert_eq!(*literal, LiteralValue::String("context-owned bytes"));
        assert_eq!(texts, &["local prefix", "suffix"]);
        assert_eq!(
            literal,
            store.literal(LiteralValue::String("context-owned bytes"))
        );
    }

    #[test]
    fn compatibility_store_keeps_distinct_allocations() {
        let store = SharedPayloadStore::new(false);
        let value = LiteralValue::Number(Number(42.0));
        assert_ne!(store.literal(value), store.literal(value));
        let texts = ["a", "b"];
        assert_ne!(
            store.string_list(&texts).as_ptr(),
            store.string_list(&texts).as_ptr()
        );
    }

    #[test]
    fn shared_literal_payload_keeps_local_identity_and_links() {
        let store = SharedPayloadStore::new(true);
        let first = LiteralType::default();
        first
            .value
            .set_payload(Some(store.string_literal("same value")));
        let second = LiteralType::default();
        second
            .value
            .set_payload(Some(store.literal(LiteralValue::String("same value"))));
        let first = Type::alloc(
            TypeFlags::StringLiteral,
            ObjectFlags::None,
            TypeId(1),
            first,
        );
        let second = Type::alloc(
            TypeFlags::StringLiteral,
            ObjectFlags::None,
            TypeId(2),
            second,
        );
        assert!(first != second);
        assert_ne!(first.id, second.id);
        assert_eq!(
            first.as_literal_type().value(),
            second.as_literal_type().value()
        );
        first.as_literal_type().regular_type.set(Some(first));
        first.as_literal_type().fresh_type.set(Some(second));
        assert!(second.as_literal_type().regular_type.get().is_none());
        assert!(second.as_literal_type().fresh_type.get().is_none());
        assert_eq!(store.stats().misses, 1);
        assert_eq!(store.stats().hits, 1);
    }

    #[test]
    fn labeled_tuple_metadata_stays_in_the_local_region() {
        let store = SharedPayloadStore::new(true);
        let local = Region::new_scratch(4096);
        let _local = local.enter();
        let factory = ast::NodeFactory::new(Default::default());
        let label = factory.new_identifier("label");
        let info = [TupleElementInfo {
            flags: ElementFlags::Required,
            labeled_declaration: Some(label),
        }];
        let first = store.tuple_infos(&info);
        let second = store.tuple_infos(&info);
        assert_eq!(first, second);
        assert_ne!(first.as_ptr(), second.as_ptr());
        assert_eq!(store.stats().misses, 0);
        assert_eq!(store.stats().arena_allocated_bytes, 0);
    }

    #[test]
    fn a_full_hash_collision_still_compares_literal_values() {
        let store = SharedPayloadStore::new(true);
        let wanted = LiteralValue::String("wanted");
        let hash = literal_hash(wanted);
        let other;
        {
            let mut shard = store.shard(hash).unwrap().lock().unwrap();
            let _region = shard.enter();
            other = P::new(LiteralValue::String(alloc_str("different")));
            // Force a full collision so the production lookup must reject the unequal candidate. There are only
            // two entries, so inserting the requested value does not grow or rehash this three-slot table.
            shard.literals.insert_unique(hash, other, |_| hash);
        }
        let value = store.literal(wanted);
        assert_ne!(value, other);
        assert_eq!(*value, wanted);
        assert_eq!(store.literal(wanted), value);
    }
}
