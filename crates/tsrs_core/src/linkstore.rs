use crate::P;
use rustc_hash::FxHashMap;
use std::cell::RefCell;

// Links store
//
// Values are arena-allocated and returned as `P<V>`; callers mutate them through `Cell` fields.
pub struct LinkStore<K: 'static, V: 'static> {
    entries: RefCell<FxHashMap<P<K>, P<V>>>,
}

impl<K, V> Default for LinkStore<K, V> {
    fn default() -> Self {
        LinkStore { entries: RefCell::new(FxHashMap::default()) }
    }
}

impl<K, V: Default> LinkStore<K, V> {
    pub fn get(&self, key: P<K>) -> P<V> {
        if let Some(&value) = self.entries.borrow().get(&key) {
            return value;
        }
        let value = P::new(V::default());
        self.entries.borrow_mut().insert(key, value);
        value
    }
}

impl<K, V> LinkStore<K, V> {
    pub fn has(&self, key: P<K>) -> bool {
        self.entries.borrow().contains_key(&key)
    }

    pub fn try_get(&self, key: P<K>) -> Option<P<V>> {
        self.entries.borrow().get(&key).copied()
    }
}

const PAGE_SHIFT: u64 = 8;
const PAGE_SIZE: usize = 1 << PAGE_SHIFT;
const PAGE_MASK: u64 = PAGE_SIZE as u64 - 1;
const MAX_PAGE_COUNT: u64 = 65536;

// Implements a sparse-array-like structure for storing elements keyed by dense uint64 keys. Elements are
// stored in fixed-size pages of 256 entries and an index of pages is maintained in an array for lower valued
// page indices and a map for higher valued page indices.
pub struct PagedLinkStore<V: 'static> {
    page_map: RefCell<FxHashMap<u64, &'static [V]>>, // Page map for page indices above maxPageCount
    page_list: RefCell<Vec<Option<&'static [V]>>>,  // Page table for page indices below maxPageCount
}

impl<V> Default for PagedLinkStore<V> {
    fn default() -> Self {
        PagedLinkStore { page_map: RefCell::new(FxHashMap::default()), page_list: RefCell::new(Vec::new()) }
    }
}

impl<V: Default> PagedLinkStore<V> {
    fn new_page() -> &'static [V] {
        crate::alloc_vec((0..PAGE_SIZE).map(|_| V::default()).collect())
    }

    pub fn get(&self, key: u64) -> P<V> {
        let page_index = key >> PAGE_SHIFT;
        let page = if page_index < MAX_PAGE_COUNT {
            let mut list = self.page_list.borrow_mut();
            if page_index as usize >= list.len() {
                // Grow the length of the list to pageIndex+1
                list.resize(page_index as usize + 1, None);
            }
            match list[page_index as usize] {
                Some(page) => page,
                None => {
                    let page = Self::new_page();
                    list[page_index as usize] = Some(page);
                    page
                }
            }
        } else {
            let mut map = self.page_map.borrow_mut();
            *map.entry(page_index).or_insert_with(Self::new_page)
        };
        P::from_static(&page[(key & PAGE_MASK) as usize])
    }
}

impl<V> PagedLinkStore<V> {
    pub fn has(&self, key: u64) -> bool {
        self.try_get(key).is_some()
    }

    pub fn try_get(&self, key: u64) -> Option<P<V>> {
        let page_index = key >> PAGE_SHIFT;
        let page = if page_index < MAX_PAGE_COUNT {
            self.page_list.borrow().get(page_index as usize).copied().flatten()
        } else {
            self.page_map.borrow().get(&page_index).copied()
        };
        page.map(|page| P::from_static(&page[(key & PAGE_MASK) as usize]))
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
        let store: LinkStore<i32, Links> = LinkStore::default();
        let k = P::new(1);
        assert!(!store.has(k));
        store.get(k).n.set(5);
        assert_eq!(store.try_get(k).unwrap().n.get(), 5);
        assert!(store.get(k) == store.get(k));

        let paged: PagedLinkStore<Links> = PagedLinkStore::default();
        assert!(!paged.has(3));
        paged.get(3).n.set(7);
        assert!(paged.has(4)); // same page
        assert_eq!(paged.get(3).n.get(), 7);
        paged.get(1 << 40).n.set(9);
        assert_eq!(paged.try_get(1 << 40).unwrap().n.get(), 9);
    }
}
