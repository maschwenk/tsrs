use hashbrown::hash_table::Entry;
pub use hashbrown::Equivalent;
use hashbrown::HashTable;
use rustc_hash::{FxHashMap, FxHasher};
use std::borrow::Borrow;
use std::hash::{Hash, Hasher};
use std::sync::RwLock;

/// Shards per map. Program construction reads and writes the resolver's and package.json caches from every worker
/// (vscode: 390k acquisitions in a 0.14 s phase at 64 threads); one lock serialized them on its cache line and in
/// the kernel's futex queues (notes/perf-frontend-64.md). Keys spread over the shards by their hash, so a shard
/// sees 1/64 of the traffic; the same hash probes the shard's table, so a lookup hashes once.
const SHARDS: usize = 64;

#[derive(Debug)]
pub struct SyncMap<K: Hash + Eq, V> {
    // Read-write locks: the caches are read from every worker and rarely written. (No caller re-enters the map while
    // holding a shard, so a waiting writer cannot deadlock a nested read.)
    shards: Box<[RwLock<HashTable<(K, V)>>]>,
}

impl<K: Hash + Eq, V> Default for SyncMap<K, V> {
    fn default() -> Self {
        SyncMap { shards: (0..SHARDS).map(|_| RwLock::new(HashTable::new())).collect() }
    }
}

#[inline]
fn hash_of<Q: Hash + ?Sized>(key: &Q) -> u64 {
    let mut h = FxHasher::default();
    key.hash(&mut h);
    h.finish()
}

#[inline]
fn read<K, V>(shard: &RwLock<HashTable<(K, V)>>) -> std::sync::RwLockReadGuard<'_, HashTable<(K, V)>> {
    crate::festats::timed_counted(crate::festats::Cat::LockSyncMap, crate::festats::Cat::SyncMapOps, || shard.read().unwrap())
}

#[inline]
fn write<K, V>(shard: &RwLock<HashTable<(K, V)>>) -> std::sync::RwLockWriteGuard<'_, HashTable<(K, V)>> {
    crate::festats::timed_counted(crate::festats::Cat::LockSyncMap, crate::festats::Cat::SyncMapOps, || shard.write().unwrap())
}

impl<K: Hash + Eq, V> SyncMap<K, V> {
    #[inline]
    fn shard(&self, hash: u64) -> &RwLock<HashTable<(K, V)>> {
        // The top bits: Fx mixes upward (rotate-multiply), so they depend on every input byte.
        &self.shards[(hash >> (u64::BITS - SHARDS.trailing_zeros())) as usize]
    }
}

impl<K: Hash + Eq + Clone, V: Clone> SyncMap<K, V> {
    /// The value under `key`; `key` may be a borrowed form of `K` (`str` for `String`), hashing the same.
    pub fn load<Q>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let hash = hash_of(key);
        let m = read(self.shard(hash));
        m.find(hash, |(k, _)| k.borrow() == key).map(|(_, v)| v.clone())
    }

    /// Probe a composite owned key without constructing its strings. As with `HashMap`, equivalent keys must
    /// hash identically; `Equivalent` supplies equality when `Borrow` cannot express a composite borrowed view.
    pub fn load_equivalent<Q: Hash + Equivalent<K> + ?Sized>(&self, key: &Q) -> Option<V> {
        let hash = hash_of(key);
        let m = read(self.shard(hash));
        m.find(hash, |(k, _)| key.equivalent(k)).map(|(_, v)| v.clone())
    }

    pub fn store(&self, key: K, value: V) {
        let hash = hash_of(&key);
        let mut m = write(self.shard(hash));
        match m.entry(hash, |(k, _)| *k == key, |(k, _)| hash_of(k)) {
            Entry::Occupied(mut e) => e.get_mut().1 = value,
            Entry::Vacant(e) => {
                e.insert((key, value));
            }
        }
    }

    pub fn load_or_store(&self, key: K, value: V) -> (V, bool) {
        let hash = hash_of(&key);
        let shard = self.shard(hash);
        if let Some((_, v)) = read(shard).find(hash, |(k, _)| *k == key) {
            return (v.clone(), true);
        }
        let mut m = write(shard);
        match m.entry(hash, |(k, _)| *k == key, |(k, _)| hash_of(k)) {
            Entry::Occupied(e) => (e.get().1.clone(), true),
            Entry::Vacant(e) => {
                e.insert((key, value.clone()));
                (value, false)
            }
        }
    }

    pub fn delete(&self, key: &K) {
        let hash = hash_of(key);
        let mut m = self.shard(hash).write().unwrap();
        if let Ok(e) = m.find_entry(hash, |(k, _)| k == key) {
            e.remove();
        }
    }

    pub fn clear(&self) {
        for shard in &self.shards {
            shard.write().unwrap().clear();
        }
    }

    pub fn range(&self, mut f: impl FnMut(&K, &V) -> bool) {
        // Go's sync.Map.Range visits a snapshot in no particular order; this visits the shards in index order.
        let entries: Vec<(K, V)> = self.shards.iter().flat_map(|s| s.read().unwrap().iter().cloned().collect::<Vec<_>>()).collect();
        for (k, v) in &entries {
            if !f(k, v) {
                break;
            }
        }
    }

    // Size returns the approximate number of items in the map.
    pub fn size(&self) -> usize {
        self.shards.iter().map(|s| s.read().unwrap().len()).sum()
    }

    pub fn to_map(&self) -> FxHashMap<K, V> {
        let mut out = FxHashMap::with_capacity_and_hasher(self.size(), Default::default());
        for shard in &self.shards {
            out.extend(shard.read().unwrap().iter().cloned());
        }
        out
    }

    pub fn keys(&self) -> Vec<K> {
        self.shards.iter().flat_map(|s| s.read().unwrap().iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()).collect()
    }

    pub fn clone_map(&self) -> SyncMap<K, V> {
        let clone = SyncMap::default();
        for (i, shard) in self.shards.iter().enumerate() {
            clone.shards[i].write().unwrap().clone_from(&shard.read().unwrap());
        }
        clone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_map() {
        let m: SyncMap<&str, i32> = SyncMap::default();
        assert_eq!(m.load_or_store("a", 1), (1, false));
        assert_eq!(m.load_or_store("a", 2), (1, true));
        assert_eq!(m.load(&"a"), Some(1));
        m.store("b", 3);
        m.store("b", 4);
        assert_eq!(m.load(&"b"), Some(4));
        assert_eq!(m.size(), 2);
        let c = m.clone_map();
        m.delete(&"a");
        assert_eq!(m.load(&"a"), None);
        assert_eq!(c.load(&"a"), Some(1));
    }

    #[test]
    fn borrowed_lookup_hashes_like_the_key() {
        let m: SyncMap<String, u32> = SyncMap::default();
        for i in 0..1000 {
            m.store(format!("/dir/{i}/file.ts"), i);
        }
        assert!((0..1000).all(|i| m.load(format!("/dir/{i}/file.ts").as_str()) == Some(i)));
        assert_eq!(m.load("/dir/none"), None);
    }

    #[test]
    fn shards_cover_every_key() {
        let m: SyncMap<u64, u64> = SyncMap::default();
        for i in 0..10_000 {
            m.store(i, i * 2);
        }
        assert_eq!(m.size(), 10_000);
        assert!((0..10_000).all(|i| m.load(&i) == Some(i * 2)));
        let mut keys = m.keys();
        keys.sort_unstable();
        assert_eq!(keys, (0..10_000).collect::<Vec<_>>());
        assert_eq!(m.to_map().len(), 10_000);
        let mut seen = 0;
        m.range(|_, _| {
            seen += 1;
            true
        });
        assert_eq!(seen, 10_000);
        m.clear();
        assert_eq!(m.size(), 0);
    }
}
