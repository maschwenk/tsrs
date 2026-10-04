use rustc_hash::FxHashMap;
use std::hash::Hash;
use std::sync::RwLock;

#[derive(Debug)]
pub struct SyncMap<K: Hash + Eq, V> {
    // A read-write lock: the resolver's and package.json caches are read from every worker during program
    // construction and rarely written. (No caller re-enters the map while holding it, so a waiting writer cannot
    // deadlock a nested read.)
    m: RwLock<FxHashMap<K, V>>,
}

impl<K: Hash + Eq, V> Default for SyncMap<K, V> {
    fn default() -> Self {
        SyncMap { m: RwLock::new(FxHashMap::default()) }
    }
}

impl<K: Hash + Eq + Clone, V: Clone> SyncMap<K, V> {
    pub fn load(&self, key: &K) -> Option<V> {
        self.m.read().unwrap().get(key).cloned()
    }

    pub fn store(&self, key: K, value: V) {
        self.m.write().unwrap().insert(key, value);
    }

    pub fn load_or_store(&self, key: K, value: V) -> (V, bool) {
        if let Some(v) = self.m.read().unwrap().get(&key) {
            return (v.clone(), true);
        }
        let mut m = self.m.write().unwrap();
        if let Some(v) = m.get(&key) {
            return (v.clone(), true);
        }
        m.insert(key, value.clone());
        (value, false)
    }

    pub fn delete(&self, key: &K) {
        self.m.write().unwrap().remove(key);
    }

    pub fn clear(&self) {
        self.m.write().unwrap().clear();
    }

    pub fn range(&self, mut f: impl FnMut(&K, &V) -> bool) {
        let entries: Vec<(K, V)> = self.m.read().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in &entries {
            if !f(k, v) {
                break;
            }
        }
    }

    // Size returns the approximate number of items in the map.
    pub fn size(&self) -> usize {
        self.m.read().unwrap().len()
    }

    pub fn to_map(&self) -> FxHashMap<K, V> {
        self.m.read().unwrap().clone()
    }

    pub fn keys(&self) -> Vec<K> {
        self.m.read().unwrap().keys().cloned().collect()
    }

    pub fn clone_map(&self) -> SyncMap<K, V> {
        SyncMap { m: RwLock::new(self.to_map()) }
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
}
