use std::hash::Hash;
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashSet;
use tsrs_core::collections::SyncMap;

struct ownerCacheEntry<V> {
    mu: Mutex<ownerCacheEntryState<V>>,
}

struct ownerCacheEntryState<V> {
    value: Option<V>,
    owners: FxHashSet<u64>,
}

// OwnerCache is like RefCountCache, but each entry tracks the set of its
// owners instead of a count. We use this to associate extended config cache
// entries with each snapshot that contains them, since the same config can
// be Acquired multiple times during config parsing while only appearing once in
// the ParsedCommandLine's list of extended files. When updating this code, check
// if the same changes should be made to RefCountCache as well.
pub struct OwnerCache<K: Hash + Eq, V> {
    entries: SyncMap<K, Arc<ownerCacheEntry<V>>>,
}

// ownercache.go:28
pub fn new_owner_cache<K: Hash + Eq, V>() -> OwnerCache<K, V> {
    OwnerCache { entries: SyncMap::default() }
}

impl<K: Hash + Eq + Clone, V: Clone> OwnerCache<K, V> {
    // ownercache.go:38. Loading borrows its inputs only during this call; neither callback is retained.
    pub fn load_and_acquire(&self, identity: &K, owner: u64, parse: impl FnOnce() -> V, is_expired: impl FnOnce(&V) -> bool) -> V {
        self.load_or_store_locked_entry(&identity, |entry, loaded| {
            if !loaded || is_expired(entry.value.as_ref().unwrap()) {
                entry.value = Some(parse());
            }
            entry.owners.insert(owner);
            entry.value.clone().unwrap()
        })
    }

    // ownercache.go:48
    pub fn acquire(&self, identity: &K, owner: u64, value: V) {
        self.load_or_store_locked_entry(&identity, |entry, loaded| {
            if !loaded {
                entry.value = Some(value);
            }
            entry.owners.insert(owner);
        })
    }

    // ownercache.go:60
    // AddOwner adds an owner to an existing live entry. The entry must exist
    // and have at least one current owner; callers must ensure the entry is
    // kept alive (e.g. via snapshot ref counting).
    pub fn add_owner(&self, identity: &K, owner: u64) {
        let Some(entry) = self.entries.load(identity) else {
            panic!("OwnerCache.AddOwner: entry not found");
        };
        let mut st = entry.mu.lock().unwrap();
        if st.owners.is_empty() {
            panic!("OwnerCache.AddOwner: entry has no owners");
        }
        st.owners.insert(owner);
    }

    // ownercache.go:73
    pub fn has(&self, identity: &K) -> bool {
        self.entries.load(identity).is_some()
    }

    // ownercache.go:78
    pub fn release(&self, identity: &K, owner: u64) {
        let Some(entry) = self.entries.load(identity) else {
            return;
        };
        let mut st = entry.mu.lock().unwrap();
        st.owners.remove(&owner);
        if st.owners.is_empty() {
            self.entries.delete(identity);
        }
    }

    // ownercache.go:91
    // Go returns the entry locked and the caller unlocks it with `defer`; here the caller's code runs as `f`
    // while the lock is held.
    fn load_or_store_locked_entry<R>(&self, key: &K, f: impl FnOnce(&mut ownerCacheEntryState<V>, bool) -> R) -> R {
        loop {
            let entry = Arc::new(ownerCacheEntry { mu: Mutex::new(ownerCacheEntryState { value: None, owners: FxHashSet::default() }) });
            let mut guard = entry.mu.lock().unwrap();
            let (existing, loaded) = self.entries.load_or_store(key.clone(), Arc::clone(&entry));
            if loaded {
                drop(guard);
                let mut existing_guard = existing.mu.lock().unwrap();
                if existing_guard.owners.is_empty() {
                    continue;
                }
                return f(&mut existing_guard, true);
            }
            return f(&mut guard, false);
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.size()
    }
}
