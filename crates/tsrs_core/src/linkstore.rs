//! Owned link stores for node-building and emit state. Recursive work retains keys and borrows again afterward.
//!
//! A record borrow cannot escape its owning store:
//! ```compile_fail
//! use tsrs_core::{LinkStore, P};
//! static KEY: u32 = 1;
//! let value = {
//!     let store = LinkStore::<u32, String>::default();
//!     store.get(P::from_static(&KEY))
//! };
//! println!("{}", value.len());
//! ```
#![forbid(unsafe_code)]

use crate::arena_owner::{ArenaBuilder, ArenaKey, LocalKey};
use crate::{P, PKey};
use rustc_hash::FxHashMap;
use std::cell::{Ref, RefCell};
use std::marker::PhantomData;

/// Interior mutability supports the node builder's shared receiver. `Ref` checks that insertion or clearing does
/// not overlap a live record borrow. Retain `ArenaKey` across callbacks that may add or clear entries.
pub struct LinkStore<K: 'static, V> {
    entries: RefCell<LinkEntries<V>>,
    key: PhantomData<fn() -> K>,
}

struct LinkEntries<V> {
    index: FxHashMap<PKey, LocalKey<V>>,
    values: ArenaBuilder<V>,
}

impl<K: 'static, V> Default for LinkStore<K, V> {
    fn default() -> Self {
        Self {
            entries: RefCell::new(LinkEntries { index: FxHashMap::default(), values: ArenaBuilder::new() }),
            key: PhantomData,
        }
    }
}

impl<K: 'static, V> LinkStore<K, V> {
    /// Drops the records and the index. Existing keys are invalidated by the fresh table identity.
    pub fn clear(&self) {
        *self.entries.borrow_mut() = LinkEntries { index: FxHashMap::default(), values: ArenaBuilder::new() };
    }

    pub fn has(&self, key: P<K>) -> bool {
        self.entries.borrow().index.contains_key(&key.key())
    }

    pub fn try_get(&self, key: P<K>) -> Option<Ref<'_, V>> {
        let entries = self.entries.borrow();
        let index = *entries.index.get(&key.key())?;
        Some(Ref::map(entries, |entries| entries.values.get_local(index).expect("stored link index")))
    }

    pub fn at(&self, key: ArenaKey<V>) -> Ref<'_, V> {
        Ref::map(self.entries.borrow(), |entries| entries.values.get(key).expect("link key belongs to another store"))
    }
}

impl<K: 'static, V: Default> LinkStore<K, V> {
    pub fn get(&self, key: P<K>) -> Ref<'_, V> {
        let key = self.get_key(key);
        self.at(key)
    }

    pub fn get_key(&self, key: P<K>) -> ArenaKey<V> {
        let mut entries = self.entries.borrow_mut();
        if let Some(&index) = entries.index.get(&key.key()) {
            return entries.values.qualify(index).expect("stored link index");
        }
        let value = entries.values.alloc(V::default());
        entries.index.insert(key.key(), value.local());
        value
    }
}

const PAGE_SHIFT: u64 = 8;
const PAGE_SIZE: usize = 1 << PAGE_SHIFT;
const PAGE_MASK: u64 = PAGE_SIZE as u64 - 1;
const MAX_PAGE_COUNT: u64 = 65536;

type Page<V> = [V; PAGE_SIZE];

/// A paged record key retains its table identity and the position inside its page.
pub struct PagedLinkKey<V> {
    page: ArenaKey<Page<V>>,
    offset: u8,
}

impl<V> Copy for PagedLinkKey<V> {}
impl<V> Clone for PagedLinkKey<V> {
    fn clone(&self) -> Self {
        *self
    }
}

/// A sparse two-level index of Rust-owned pages. The high-key map and low-key vector share one value owner.
pub struct PagedLinkStore<V> {
    entries: RefCell<PagedEntries<V>>,
}

struct PagedEntries<V> {
    page_map: FxHashMap<u64, LocalKey<Page<V>>>,
    page_list: Vec<Option<LocalKey<Page<V>>>>,
    pages: ArenaBuilder<Page<V>>,
}

impl<V> Default for PagedLinkStore<V> {
    fn default() -> Self {
        Self {
            entries: RefCell::new(PagedEntries {
                page_map: FxHashMap::default(),
                page_list: Vec::new(),
                pages: ArenaBuilder::new(),
            }),
        }
    }
}

impl<V> PagedEntries<V> {
    fn page(&self, index: u64) -> Option<LocalKey<Page<V>>> {
        if index < MAX_PAGE_COUNT {
            self.page_list.get(index as usize).copied().flatten()
        } else {
            self.page_map.get(&index).copied()
        }
    }
}

impl<V: Default> PagedLinkStore<V> {
    pub fn get(&self, key: u64) -> Ref<'_, V> {
        let key = self.get_key(key);
        self.at(key)
    }

    pub fn get_key(&self, key: u64) -> PagedLinkKey<V> {
        let mut entries = self.entries.borrow_mut();
        let page_index = key >> PAGE_SHIFT;
        let page = match entries.page(page_index) {
            Some(page) => page,
            None => {
                let page = entries.pages.alloc(std::array::from_fn(|_| V::default())).local();
                if page_index < MAX_PAGE_COUNT {
                    if page_index as usize >= entries.page_list.len() {
                        entries.page_list.resize(page_index as usize + 1, None);
                    }
                    entries.page_list[page_index as usize] = Some(page);
                } else {
                    entries.page_map.insert(page_index, page);
                }
                page
            }
        };
        PagedLinkKey { page: entries.pages.qualify(page).expect("stored link page"), offset: (key & PAGE_MASK) as u8 }
    }
}

impl<V> PagedLinkStore<V> {
    pub fn has(&self, key: u64) -> bool {
        self.entries.borrow().page(key >> PAGE_SHIFT).is_some()
    }

    pub fn at(&self, key: PagedLinkKey<V>) -> Ref<'_, V> {
        Ref::map(self.entries.borrow(), |entries| {
            &entries.pages.get(key.page).expect("link key belongs to another store")[key.offset as usize]
        })
    }

    pub fn try_get(&self, key: u64) -> Option<Ref<'_, V>> {
        let entries = self.entries.borrow();
        let page = entries.page(key >> PAGE_SHIFT)?;
        Some(Ref::map(entries, |entries| {
            &entries.pages.get_local(page).expect("stored link page")[(key & PAGE_MASK) as usize]
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Links {
        n: Cell<i32>,
    }

    static KEY: i32 = 1;

    #[test]
    fn link_store() {
        let store: LinkStore<i32, Links> = LinkStore::default();
        let k = P::from_static(&KEY);
        assert!(!store.has(k));
        store.get(k).n.set(5);
        assert_eq!(store.try_get(k).unwrap().n.get(), 5);
        assert_eq!(store.get_key(k), store.get_key(k));

        let paged: PagedLinkStore<Links> = PagedLinkStore::default();
        assert!(!paged.has(3));
        paged.get(3).n.set(7);
        assert!(paged.has(4)); // same page
        assert_eq!(paged.get(3).n.get(), 7);
        paged.get(1 << 40).n.set(9);
        assert_eq!(paged.try_get(1 << 40).unwrap().n.get(), 9);
    }

    #[test]
    fn clear_drops_records_and_invalidates_keys() {
        let value = Rc::new(());
        let weak = Rc::downgrade(&value);
        let store: LinkStore<i32, RefCell<Option<Rc<()>>>> = LinkStore::default();
        let key = store.get_key(P::from_static(&KEY));
        *store.at(key).borrow_mut() = Some(value);
        store.clear();
        assert!(weak.upgrade().is_none());
        let replacement = store.get_key(P::from_static(&KEY));
        assert_ne!(key, replacement);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store.at(key))).is_err());
    }

    #[test]
    fn retained_page_keys_survive_sparse_growth_and_drop_resources() {
        let value = Rc::new(());
        let weak = Rc::downgrade(&value);
        let store: PagedLinkStore<RefCell<Option<Rc<()>>>> = PagedLinkStore::default();
        let key = store.get_key(255);
        *store.at(key).borrow_mut() = Some(value);
        for page in 1..500 {
            store.get_key(page * PAGE_SIZE as u64);
        }
        store.get_key(u64::MAX);
        assert!(store.at(key).borrow().is_some());
        assert!(store.has(u64::MAX));
        assert!(!store.has((MAX_PAGE_COUNT + 1) * PAGE_SIZE as u64));
        drop(store);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn filling_a_low_page_gap_keeps_existing_high_pages() {
        let store = PagedLinkStore::<Links>::default();
        let high = store.get_key(500 * PAGE_SIZE as u64);
        store.at(high).n.set(42);
        store.get(0).n.set(1);
        assert_eq!(store.try_get(500 * PAGE_SIZE as u64).unwrap().n.get(), 42);
        assert_eq!(store.get(0).n.get(), 1);
        assert!(store.try_get(PAGE_SIZE as u64).is_none());
    }

    #[test]
    #[should_panic(expected = "link key belongs to another store")]
    fn foreign_page_key_is_rejected() {
        let first: PagedLinkStore<Links> = PagedLinkStore::default();
        let second: PagedLinkStore<Links> = PagedLinkStore::default();
        let key = first.get_key(0);
        second.get_key(0);
        second.at(key);
    }
}
