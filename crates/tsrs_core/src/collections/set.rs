use rustc_hash::FxHashSet;
use std::hash::Hash;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Set<T: Hash + Eq> {
    pub m: FxHashSet<T>,
}

impl<T: Hash + Eq> Default for Set<T> {
    fn default() -> Self {
        Set { m: FxHashSet::default() }
    }
}

pub fn new_set_with_size_hint<T: Hash + Eq>(hint: usize) -> Set<T> {
    Set { m: FxHashSet::with_capacity_and_hasher(hint, Default::default()) }
}

impl<T: Hash + Eq> Set<T> {
    pub fn new() -> Set<T> {
        Set { m: FxHashSet::default() }
    }

    pub fn has(&self, key: &T) -> bool {
        self.m.contains(key)
    }

    pub fn add(&mut self, key: T) {
        self.m.insert(key);
    }

    pub fn delete(&mut self, key: &T) {
        self.m.remove(key);
    }

    pub fn len(&self) -> usize {
        self.m.len()
    }

    pub fn keys(&self) -> &FxHashSet<T> {
        &self.m
    }

    pub fn clear(&mut self) {
        self.m.clear();
    }

    // Returns true if the key was not already present in the set.
    pub fn add_if_absent(&mut self, key: T) -> bool {
        self.m.insert(key)
    }

    pub fn union(&mut self, other: &Set<T>)
    where
        T: Clone,
    {
        self.m.extend(other.m.iter().cloned());
    }

    pub fn unioned_with(&self, other: &Set<T>) -> Set<T>
    where
        T: Clone,
    {
        let mut result = self.clone();
        result.union(other);
        result
    }

    pub fn equals(&self, other: &Set<T>) -> bool {
        self.m == other.m
    }

    pub fn is_subset_of(&self, other: &Set<T>) -> bool {
        self.m.iter().all(|k| other.has(k))
    }

    pub fn intersects(&self, other: &Set<T>) -> bool {
        self.m.iter().any(|k| other.has(k))
    }
}

pub fn new_set_from_items<T: Hash + Eq>(items: impl IntoIterator<Item = T>) -> Set<T> {
    let mut s = Set::new();
    for item in items {
        s.add(item);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set() {
        let mut a = new_set_from_items([1, 2, 3]);
        let b = new_set_from_items([3, 4]);
        assert!(a.intersects(&b));
        assert!(!a.is_subset_of(&b));
        assert!(!a.add_if_absent(1));
        assert!(a.add_if_absent(5));
        let u = a.unioned_with(&b);
        assert_eq!(u.len(), 5);
        assert!(b.is_subset_of(&u));
        a.delete(&5);
        assert!(a.equals(&new_set_from_items([1, 2, 3])));
    }
}
