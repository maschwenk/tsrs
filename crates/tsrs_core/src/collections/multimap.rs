use rustc_hash::FxHashMap;
use std::hash::Hash;

#[derive(Clone, Debug)]
pub struct MultiMap<K: Hash + Eq, V> {
    pub m: FxHashMap<K, Vec<V>>,
}

impl<K: Hash + Eq, V> Default for MultiMap<K, V> {
    fn default() -> Self {
        MultiMap { m: FxHashMap::default() }
    }
}

pub fn new_multi_map_with_size_hint<K: Hash + Eq, V>(hint: usize) -> MultiMap<K, V> {
    MultiMap { m: FxHashMap::with_capacity_and_hasher(hint, Default::default()) }
}

pub fn group_by<K: Hash + Eq, V: Clone>(items: &[V], mut group_id: impl FnMut(&V) -> K) -> MultiMap<K, V> {
    let mut m = MultiMap::default();
    for item in items {
        m.add(group_id(item), item.clone());
    }
    m
}

impl<K: Hash + Eq, V> MultiMap<K, V> {
    pub fn has(&self, key: &K) -> bool {
        self.m.contains_key(key)
    }

    pub fn get<Q: Hash + Eq + ?Sized>(&self, key: &Q) -> &[V]
    where
        K: std::borrow::Borrow<Q>,
    {
        self.m.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn add(&mut self, key: K, value: V) {
        self.m.entry(key).or_default().push(value);
    }

    pub fn remove(&mut self, key: &K, value: &V)
    where
        V: PartialEq,
    {
        if let Some(values) = self.m.get_mut(key) {
            if let Some(i) = values.iter().position(|v| v == value) {
                if values.len() == 1 {
                    self.m.remove(key);
                } else {
                    values.remove(i);
                }
            }
        }
    }

    pub fn remove_all(&mut self, key: &K) {
        self.m.remove(key);
    }

    pub fn len(&self) -> usize {
        self.m.len()
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.m.keys()
    }

    pub fn values(&self) -> impl Iterator<Item = &Vec<V>> {
        self.m.values()
    }

    pub fn clear(&mut self) {
        self.m.clear();
    }
}
