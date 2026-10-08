// Port of execute/build/parseCache.go.

use std::hash::Hash;
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;

struct parseCacheEntry<V> {
    value: Mutex<Option<V>>,
}

pub(crate) struct parseCache<K, V> {
    entries: Mutex<FxHashMap<K, Arc<parseCacheEntry<V>>>>,
}

impl<K, V> Default for parseCache<K, V> {
    fn default() -> Self {
        parseCache { entries: Mutex::new(FxHashMap::default()) }
    }
}

// Go compares the value with its zero value (`allowZero`); every V here is an Option.
pub(crate) trait IsZeroValue {
    fn is_zero_value(&self) -> bool;
}

impl<T> IsZeroValue for Option<T> {
    fn is_zero_value(&self) -> bool {
        self.is_none()
    }
}

impl<K: Hash + Eq + Clone, V: Clone + IsZeroValue> parseCache<K, V> {
    // parseCache.go:18
    pub(crate) fn load_or_store(&self, key: &K, parse: impl FnOnce(&K) -> V, allow_zero: bool) -> V {
        let new_entry = Arc::new(parseCacheEntry { value: Mutex::new(None) });
        let mut new_guard = new_entry.value.lock().unwrap();
        let existing = {
            let mut entries = self.entries.lock().unwrap();
            match entries.get(key) {
                Some(entry) => Some(Arc::clone(entry)),
                None => {
                    entries.insert(key.clone(), Arc::clone(&new_entry));
                    None
                }
            }
        };
        if let Some(entry) = existing {
            drop(new_guard);
            let mut guard = entry.value.lock().unwrap();
            if let Some(value) = guard.as_ref() {
                if allow_zero || !value.is_zero_value() {
                    return value.clone();
                }
            }
            let value = parse(key);
            *guard = Some(value.clone());
            return value;
        }
        let value = parse(key);
        *new_guard = Some(value.clone());
        value
    }

    // parseCache.go:34
    #[expect(dead_code, reason = "its only Go caller, Orchestrator.checkTasksForEventChanges (build --watch), is not ported")]
    pub(crate) fn store(&self, key: K, value: V) {
        self.entries.lock().unwrap().insert(key, Arc::new(parseCacheEntry { value: Mutex::new(Some(value)) }));
    }

    // parseCache.go:38
    pub(crate) fn delete(&self, key: &K) {
        self.entries.lock().unwrap().remove(key);
    }

    // parseCache.go:42
    pub(crate) fn reset(&self) {
        self.entries.lock().unwrap().clear();
    }
}
