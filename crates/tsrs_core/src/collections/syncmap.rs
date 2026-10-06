use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};
use std::sync::RwLock;

/// Shards per map. Program construction reads and writes the resolver's and package.json caches from every worker
/// (vscode: 390k acquisitions in a 0.14 s phase at 64 threads); one lock serialized them on its cache line
/// (notes/perf-frontend-64.md). Keys spread over the shards by their hash, so a shard sees 1/64 of the traffic.
const SHARDS: usize = 64;

#[derive(Debug)]
pub struct SyncMap<K: Hash + Eq, V> {
    // Read-write locks: the caches are read from every worker and rarely written. (No caller re-enters the map while
    // holding a shard, so a waiting writer cannot deadlock a nested read.)
    shards: Box<[RwLock<FxHashMap<K, V>>]>,
}

impl<K: Hash + Eq, V> Default for SyncMap<K, V> {
    fn default() -> Self {
        SyncMap { shards: (0..SHARDS).map(|_| RwLock::new(FxHashMap::default())).collect() }
    }
}

impl<K: Hash + Eq, V> SyncMap<K, V> {
    #[inline]
    fn shard(&self, key: &K) -> &RwLock<FxHashMap<K, V>> {
        let mut h = FxHasher::default();
        key.hash(&mut h);
        // The top bits: Fx mixes upward (rotate-multiply), so they depend on every input byte.
        &self.shards[(h.finish() >> (u64::BITS - SHARDS.trailing_zeros())) as usize]
    }
}

impl<K: Hash + Eq + Clone, V: Clone> SyncMap<K, V> {
    pub fn load(&self, key: &K) -> Option<V> {
        let m = crate::festats::timed_counted(crate::festats::Cat::LockSyncMap, crate::festats::Cat::SyncMapOps, || self.shard(key).read().unwrap());
        m.get(key).cloned()
    }

    pub fn store(&self, key: K, value: V) {
        let mut m = crate::festats::timed_counted(crate::festats::Cat::LockSyncMap, crate::festats::Cat::SyncMapOps, || self.shard(&key).write().unwrap());
        m.insert(key, value);
    }

    pub fn load_or_store(&self, key: K, value: V) -> (V, bool) {
        let shard = self.shard(&key);
        if let Some(v) = crate::festats::timed_counted(crate::festats::Cat::LockSyncMap, crate::festats::Cat::SyncMapOps, || shard.read().unwrap()).get(&key)
        {
            return (v.clone(), true);
        }
        let mut m = crate::festats::timed_counted(crate::festats::Cat::LockSyncMap, crate::festats::Cat::SyncMapOps, || shard.write().unwrap());
        if let Some(v) = m.get(&key) {
            return (v.clone(), true);
        }
        m.insert(key, value.clone());
        (value, false)
    }

    pub fn delete(&self, key: &K) {
        self.shard(key).write().unwrap().remove(key);
    }

    pub fn clear(&self) {
        for shard in &self.shards {
            shard.write().unwrap().clear();
        }
    }

    pub fn range(&self, mut f: impl FnMut(&K, &V) -> bool) {
        // Go's sync.Map.Range visits a snapshot in no particular order; this visits the shards in index order.
        let entries: Vec<(K, V)> = self.shards.iter().flat_map(|s| s.read().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>()).collect();
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
            out.extend(shard.read().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        out
    }

    pub fn keys(&self) -> Vec<K> {
        self.shards.iter().flat_map(|s| s.read().unwrap().keys().cloned().collect::<Vec<_>>()).collect()
    }

    pub fn clone_map(&self) -> SyncMap<K, V> {
        let clone = SyncMap::default();
        for (i, shard) in self.shards.iter().enumerate() {
            *clone.shards[i].write().unwrap() = shard.read().unwrap().clone();
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
        assert_eq!(m.size(), 2);
        let c = m.clone_map();
        m.delete(&"a");
        assert_eq!(m.load(&"a"), None);
        assert_eq!(c.load(&"a"), Some(1));
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
