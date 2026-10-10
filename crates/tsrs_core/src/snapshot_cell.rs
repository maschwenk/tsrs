//! Owned values whose reads produce independent snapshots instead of arena references.

#![forbid(unsafe_code)]

use std::cell::{Ref, RefCell};

#[derive(Default)]
pub struct SnapshotCell<T>(RefCell<T>);

impl<T> SnapshotCell<T> {
    pub fn new(value: T) -> Self {
        Self(RefCell::new(value))
    }

    pub fn set(&self, value: T) {
        let previous = self.0.replace(value);
        drop(previous);
    }

    pub fn borrow(&self) -> Ref<'_, T> {
        self.0.borrow()
    }
}

impl<T: Clone> SnapshotCell<T> {
    pub fn get(&self) -> T {
        self.0.borrow().clone()
    }
}
