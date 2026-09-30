use rustc_hash::FxBuildHasher;
use std::hash::Hash;

pub type OrderedSet<T> = indexmap::IndexSet<T, FxBuildHasher>;

pub fn new_ordered_set_with_size_hint<T>(hint: usize) -> OrderedSet<T> {
    OrderedSet::with_capacity_and_hasher(hint, FxBuildHasher)
}

/// Go-named methods for `OrderedSet`. `delete` preserves insertion order like the Go implementation.
pub trait OrderedSetExt<T> {
    fn add(&mut self, value: T);
    fn has(&self, value: &T) -> bool;
    fn delete(&mut self, value: &T) -> bool;
    fn size(&self) -> usize;
}

impl<T: Hash + Eq> OrderedSetExt<T> for OrderedSet<T> {
    fn add(&mut self, value: T) {
        self.insert(value);
    }

    fn has(&self, value: &T) -> bool {
        self.contains(value)
    }

    fn delete(&mut self, value: &T) -> bool {
        self.shift_remove(value)
    }

    fn size(&self) -> usize {
        self.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ordered_set() {
        let mut s: OrderedSet<i32> = OrderedSet::default();
        for i in 0..10 {
            s.add(i);
        }
        s.add(3);
        assert_eq!(s.size(), 10);
        assert!(s.delete(&3));
        assert!(!s.delete(&3));
        assert!(!s.has(&3));
        let v: Vec<i32> = s.iter().copied().collect();
        assert_eq!(v, vec![0, 1, 2, 4, 5, 6, 7, 8, 9]);
    }
}
