use crate::P;
use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};
use std::hash::Hash;

// Links store
//
// Values are arena-allocated and returned as `P<V>`; callers mutate them through `Cell` fields.
pub struct LinkStore<K: 'static, V: 'static> {
    entries: RefCell<FxHashMap<K, P<V>>>,
    /// Values go to the thread's scratch region (`P::new_scratch`): the store of an object that dies with it.
    scratch: bool,
}

impl<K, V> Default for LinkStore<K, V> {
    fn default() -> Self {
        LinkStore { entries: RefCell::new(FxHashMap::default()), scratch: false }
    }
}

impl<K, V> LinkStore<K, V> {
    /// A store whose values are allocated in the thread's scratch region when one is entered (see `scratch`).
    pub fn new_scratch(scratch: bool) -> Self {
        LinkStore { entries: RefCell::new(FxHashMap::default()), scratch }
    }

    /// Forgets every entry (and releases the table, so no stale entry stays in its memory).
    pub fn clear(&self) {
        *self.entries.borrow_mut() = FxHashMap::default();
    }

    pub fn storage_usage(&self) -> (usize, usize) {
        let entries = self.entries.borrow();
        (entries.len(), entries.capacity())
    }
}

impl<K: Copy + Eq + Hash, V: Default> LinkStore<K, V> {
    pub fn get(&self, key: K) -> P<V> {
        if let Some(&value) = self.entries.borrow().get(&key) {
            return value;
        }
        let value = P::new_in(self.scratch, V::default());
        self.entries.borrow_mut().insert(key, value);
        value
    }
}

impl<K: Copy + Eq + Hash, V> LinkStore<K, V> {
    pub fn has(&self, key: K) -> bool {
        self.entries.borrow().contains_key(&key)
    }

    pub fn try_get(&self, key: K) -> Option<P<V>> {
        self.entries.borrow().get(&key).copied()
    }
}

pub const LINK_PAGE_SHIFT: u64 = 8;
pub const LINK_PAGE_SIZE: usize = 1 << LINK_PAGE_SHIFT;
pub const LINK_PAGE_MASK: u64 = LINK_PAGE_SIZE as u64 - 1;

type LinkPage<V> = [Cell<Option<P<V>>>; LINK_PAGE_SIZE];

// PagedLinkStore implements a sparse-array-like structure for storing elements keyed by dense uint64 keys.
// Elements are allocated in an arena, element references are stored in fixed-size pages of 256 entries, and
// an index of pages is maintained in a growable list.
pub struct PagedLinkStore<V: 'static> {
    pages: RefCell<Vec<Option<P<LinkPage<V>>>>>,
}

impl<V> Default for PagedLinkStore<V> {
    fn default() -> Self {
        PagedLinkStore { pages: RefCell::new(Vec::new()) }
    }
}

impl<V: Default> PagedLinkStore<V> {
    pub fn get(&self, key: u64) -> P<V> {
        let page_index = (key >> LINK_PAGE_SHIFT) as usize;
        let mut pages = self.pages.borrow_mut();
        if page_index >= pages.len() {
            // Grow the length of the list to pageIndex+1
            pages.resize(page_index + 1, None);
        }
        let page = *pages[page_index].get_or_insert_with(|| P::new(std::array::from_fn(|_| Cell::new(None))));
        let slot = &page[(key & LINK_PAGE_MASK) as usize];
        match slot.get() {
            Some(link) => link,
            None => {
                let link = P::new(V::default());
                slot.set(Some(link));
                link
            }
        }
    }
}

impl<V> PagedLinkStore<V> {
    pub fn has(&self, key: u64) -> bool {
        self.try_get(key).is_some()
    }

    pub fn try_get(&self, key: u64) -> Option<P<V>> {
        let page_index = (key >> LINK_PAGE_SHIFT) as usize;
        let page = (*self.pages.borrow().get(page_index)?)?;
        page[(key & LINK_PAGE_MASK) as usize].get()
    }

    pub fn page_index_usage(&self) -> (usize, usize) {
        let pages = self.pages.borrow();
        (pages.len(), pages.capacity())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct Links {
        n: Cell<i32>,
    }

    #[test]
    fn link_store() {
        let store: LinkStore<P<i32>, Links> = LinkStore::default();
        let k = P::new(1);
        assert!(!store.has(k));
        store.get(k).n.set(5);
        assert_eq!(store.try_get(k).unwrap().n.get(), 5);
        assert!(store.get(k) == store.get(k));

        let paged: PagedLinkStore<Links> = PagedLinkStore::default();
        assert!(!paged.has(3));
        paged.get(3).n.set(7);
        assert!(!paged.has(4)); // same page, but no link has been allocated
        assert_eq!(paged.get(3).n.get(), 7);
        let held = paged.get(3);
        paged.get((1 << 24) - 1).n.set(9);
        assert_eq!(paged.try_get((1 << 24) - 1).unwrap().n.get(), 9);
        assert!(!paged.has((1 << 24) - 2));
        assert!(!paged.has(1 << 24));
        assert_eq!(paged.get(3), held);

        let ids: LinkStore<u64, Links> = LinkStore::default();
        ids.get(1 << 48).n.set(11);
        assert_eq!(ids.try_get(1 << 48).unwrap().n.get(), 11);
    }
}
