use std::hash::Hash;

use rustc_hash::FxHashMap;

// util.go:5
pub fn clone_map_if_nil<K: Hash + Eq + Clone, V: Clone, T>(
    dirty: &T,
    original: Option<&T>,
    get_map: impl Fn(&T) -> Option<&FxHashMap<K, V>>,
) -> FxHashMap<K, V> {
    let Some(dirty_map) = get_map(dirty) else {
        let Some(original) = original else {
            return FxHashMap::default();
        };
        let Some(original_map) = get_map(original) else {
            return FxHashMap::default();
        };
        return original_map.clone();
    };
    dirty_map.clone()
}
