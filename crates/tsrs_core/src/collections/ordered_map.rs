use rustc_hash::FxBuildHasher;
use std::hash::Hash;

pub type OrderedMap<K, V> = indexmap::IndexMap<K, V, FxBuildHasher>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapEntry<K, V> {
    pub key: K,
    pub value: V,
}

pub fn new_ordered_map_with_size_hint<K, V>(hint: usize) -> OrderedMap<K, V> {
    OrderedMap::with_capacity_and_hasher(hint, FxBuildHasher)
}

pub fn new_ordered_map_from_list<K: Hash + Eq, V>(items: Vec<MapEntry<K, V>>) -> OrderedMap<K, V> {
    let mut m = new_ordered_map_with_size_hint(items.len());
    for entry in items {
        m.set(entry.key, entry.value);
    }
    m
}

/// Go-named methods for `OrderedMap`. `delete` preserves insertion order like the Go implementation.
pub trait OrderedMapExt<K, V> {
    fn set(&mut self, key: K, value: V);
    fn get_or_zero(&self, key: &K) -> V
    where
        V: Default + Clone;
    fn entry_at(&self, index: usize) -> Option<(&K, &V)>;
    fn has(&self, key: &K) -> bool;
    fn delete(&mut self, key: &K) -> Option<V>;
    fn size(&self) -> usize;
}

impl<K: Hash + Eq, V> OrderedMapExt<K, V> for OrderedMap<K, V> {
    fn set(&mut self, key: K, value: V) {
        self.insert(key, value);
    }

    fn get_or_zero(&self, key: &K) -> V
    where
        V: Default + Clone,
    {
        self.get(key).cloned().unwrap_or_default()
    }

    fn entry_at(&self, index: usize) -> Option<(&K, &V)> {
        self.get_index(index)
    }

    fn has(&self, key: &K) -> bool {
        self.contains_key(key)
    }

    fn delete(&mut self, key: &K) -> Option<V> {
        self.shift_remove(key)
    }

    fn size(&self) -> usize {
        self.len()
    }
}

pub fn diff_ordered_maps<K: Hash + Eq, V: PartialEq>(
    m1: &OrderedMap<K, V>,
    m2: &OrderedMap<K, V>,
    on_added: impl FnMut(&K, &V),
    on_removed: impl FnMut(&K, &V),
    on_modified: impl FnMut(&K, &V, &V),
) {
    diff_ordered_maps_func(m1, m2, |a, b| a == b, on_added, on_removed, on_modified)
}

pub fn diff_ordered_maps_func<K: Hash + Eq, V>(
    m1: &OrderedMap<K, V>,
    m2: &OrderedMap<K, V>,
    mut equal_values: impl FnMut(&V, &V) -> bool,
    mut on_added: impl FnMut(&K, &V),
    mut on_removed: impl FnMut(&K, &V),
    mut on_modified: impl FnMut(&K, &V, &V),
) {
    for (k, v2) in m2 {
        if !m1.contains_key(k) {
            on_added(k, v2);
        }
    }
    for (k, v1) in m1 {
        if let Some(v2) = m2.get(k) {
            if !equal_values(v1, v2) {
                on_modified(k, v1, v2);
            }
        } else {
            on_removed(k, v1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad_int(n: i32) -> String {
        format!("{n:10}")
    }

    #[test]
    fn test_ordered_map() {
        let mut m: OrderedMap<i32, String> = OrderedMap::default();
        assert!(!m.has(&1));
        const N: i32 = 1000;
        const START: i32 = 1;
        const END: i32 = START + N;
        // Seed the map with ascending keys and values for easier testing.
        for i in START..END {
            m.set(i, pad_int(i));
        }
        assert_eq!(m.size(), N as usize);
        // Attempt to overwrite existing keys in reverse order.
        for i in (START..END).rev() {
            m.set(i, pad_int(i));
        }
        assert_eq!(m.size(), N as usize);
        for i in START..END {
            assert_eq!(m.get(&i), Some(&pad_int(i)));
        }
        for (k, v) in &m {
            assert_eq!(*v, pad_int(*k));
        }
        let keys: Vec<i32> = m.keys().copied().collect();
        assert_eq!(keys.len(), N as usize);
        assert!(keys.is_sorted());
        let values: Vec<&String> = m.values().collect();
        assert!(values.is_sorted());
        assert_eq!(m.keys().next(), Some(&START));
        assert_eq!(m.values().next(), Some(&pad_int(START)));
        for i in START + 1..END {
            assert_eq!(m.delete(&i), Some(pad_int(i)));
            assert!(!m.has(&i));
            assert_eq!(m.get_or_zero(&i), "");
            assert_eq!(m.delete(&i), None);
        }
        assert_eq!(m.size(), 1);
        assert!(m.has(&START));
        assert_eq!(m.delete(&START), Some(pad_int(START)));
        assert_eq!(m.size(), 0);
    }

    #[test]
    fn test_ordered_map_clone() {
        let mut m: OrderedMap<i32, &str> = OrderedMap::default();
        m.set(1, "one");
        m.set(2, "two");
        let clone = m.clone();
        assert_eq!(clone.size(), 2);
        m.delete(&1);
        assert_eq!(m.size(), 1);
        assert_eq!(clone.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(clone.values().copied().collect::<Vec<_>>(), vec!["one", "two"]);
        m.clear();
        assert_eq!(m.size(), 0);
    }

    #[test]
    fn test_diff() {
        let m1 = new_ordered_map_from_list(vec![
            MapEntry { key: "a", value: 1 },
            MapEntry { key: "b", value: 2 },
        ]);
        let m2 = new_ordered_map_from_list(vec![
            MapEntry { key: "b", value: 3 },
            MapEntry { key: "c", value: 4 },
        ]);
        let mut log = Vec::new();
        let log_cell = std::cell::RefCell::new(&mut log);
        diff_ordered_maps(
            &m1,
            &m2,
            |k, v| log_cell.borrow_mut().push(format!("+{k}{v}")),
            |k, v| log_cell.borrow_mut().push(format!("-{k}{v}")),
            |k, a, b| log_cell.borrow_mut().push(format!("~{k}{a}{b}")),
        );
        assert_eq!(log, vec!["+c4", "-a1", "~b23"]);
    }
}
