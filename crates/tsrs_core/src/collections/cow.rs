use rustc_hash::FxHashMap;
use std::hash::Hash;
use std::rc::Rc;

// CopyOnWriteMap is a map that defers cloning of an inherited backing map
// until the first mutation, and supports nested scopes that share the parent's
// map for reads but get their own clone on write.
//
// The zero value is an empty map ready to use.
#[derive(Clone, Debug)]
pub struct CopyOnWriteMap<K: Hash + Eq + Clone, V: Clone> {
    m: Rc<FxHashMap<K, V>>,
}

impl<K: Hash + Eq + Clone, V: Clone> Default for CopyOnWriteMap<K, V> {
    fn default() -> Self {
        CopyOnWriteMap { m: Rc::new(FxHashMap::default()) }
    }
}

/// Saved state returned by `enter_scope`; pass it to `exit_scope` to restore the map.
pub struct CopyOnWriteScope<K: Hash + Eq + Clone, V: Clone> {
    saved: Rc<FxHashMap<K, V>>,
}

impl<K: Hash + Eq + Clone, V: Clone> CopyOnWriteMap<K, V> {
    pub fn get(&self, k: &K) -> Option<&V> {
        self.m.get(k)
    }

    pub fn has(&self, k: &K) -> bool {
        self.m.contains_key(k)
    }

    // Set assigns v to k, cloning the inherited backing map first if necessary.
    pub fn set(&mut self, k: K, v: V) {
        Rc::make_mut(&mut self.m).insert(k, v);
    }

    // EnterScope returns a value that restores this map to its current state.
    // While the scope is active, the map shares its current backing storage with
    // the parent scope: reads see the inherited entries, and the first mutation
    // transparently clones the storage so the parent's view is not modified.
    pub fn enter_scope(&mut self) -> CopyOnWriteScope<K, V> {
        CopyOnWriteScope { saved: Rc::clone(&self.m) }
    }

    pub fn exit_scope(&mut self, scope: CopyOnWriteScope<K, V>) {
        self.m = scope.saved;
    }
}

#[derive(Clone, Debug)]
pub struct CopyOnWriteSet<K: Hash + Eq + Clone> {
    m: CopyOnWriteMap<K, ()>,
}

impl<K: Hash + Eq + Clone> Default for CopyOnWriteSet<K> {
    fn default() -> Self {
        CopyOnWriteSet { m: CopyOnWriteMap::default() }
    }
}

impl<K: Hash + Eq + Clone> CopyOnWriteSet<K> {
    pub fn has(&self, k: &K) -> bool {
        self.m.has(k)
    }

    pub fn add(&mut self, k: K) {
        self.m.set(k, ());
    }

    pub fn enter_scope(&mut self) -> CopyOnWriteScope<K, ()> {
        self.m.enter_scope()
    }

    pub fn exit_scope(&mut self, scope: CopyOnWriteScope<K, ()>) {
        self.m.exit_scope(scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes() {
        let mut s: CopyOnWriteSet<i32> = CopyOnWriteSet::default();
        s.add(1);
        let scope = s.enter_scope();
        s.add(2);
        assert!(s.has(&1) && s.has(&2));
        s.exit_scope(scope);
        assert!(s.has(&1) && !s.has(&2));
    }
}
