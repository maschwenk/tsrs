//! `FrozenCell<T>`: interior mutability for arena objects that are shared between checker threads.
//!
//! Go's AST nodes and binder symbols are written while a file is parsed and bound and are then read
//! concurrently by every checker, with no synchronization: they are frozen by convention. `RefCell`
//! cannot express that, because its borrow counter is written even by `borrow()`, so concurrent readers
//! race on it. `FrozenCell` has the same `borrow()` / `borrow_mut()` surface without a runtime flag.
//!
//! Contract (see "Threading" in docs/PORTING.md):
//! - `borrow_mut()` is only called by the thread that owns the object: the parser/binder while the file
//!   is being built, or a checker on objects it created itself (transient symbols, its own tables).
//!   Once a file is bound, its nodes and binder symbols are never written again.
//! - A `borrow_mut()` guard is never held across a call that may read or write the same cell (same rule
//!   as for `RefCell`).
//!
//! Builds with `debug_assertions` or the `checked-cells` feature keep an atomic borrow counter and panic
//! on aliasing violations, like `RefCell` but without a data race; release builds do no bookkeeping.

use std::cell::UnsafeCell;
use std::fmt;
use std::ops::{Deref, DerefMut};

#[cfg(any(debug_assertions, feature = "checked-cells"))]
use std::sync::atomic::{AtomicIsize, Ordering};

pub struct FrozenCell<T> {
    value: UnsafeCell<T>,
    #[cfg(any(debug_assertions, feature = "checked-cells"))]
    state: AtomicIsize, // > 0: shared borrows, -1: exclusive borrow
}

// SAFETY: shared access is read-only except through `borrow_mut`, whose callers follow the ownership
// contract in the module docs (only the owning thread writes, and never while other threads can read).
unsafe impl<T: Send + Sync> Sync for FrozenCell<T> {}
unsafe impl<T: Send> Send for FrozenCell<T> {}

impl<T> FrozenCell<T> {
    #[inline]
    pub const fn new(value: T) -> FrozenCell<T> {
        FrozenCell {
            value: UnsafeCell::new(value),
            #[cfg(any(debug_assertions, feature = "checked-cells"))]
            state: AtomicIsize::new(0),
        }
    }

    #[inline]
    pub fn borrow(&self) -> FrozenRef<'_, T> {
        #[cfg(any(debug_assertions, feature = "checked-cells"))]
        {
            let prev = self.state.fetch_add(1, Ordering::Acquire);
            if prev < 0 {
                self.state.fetch_sub(1, Ordering::Release);
                panic!("FrozenCell already mutably borrowed");
            }
        }
        // SAFETY: see the module contract; no exclusive borrow is live.
        FrozenRef {
            value: unsafe { &*self.value.get() },
            #[cfg(any(debug_assertions, feature = "checked-cells"))]
            state: &self.state,
        }
    }

    #[inline]
    pub fn borrow_mut(&self) -> FrozenRefMut<'_, T> {
        #[cfg(any(debug_assertions, feature = "checked-cells"))]
        {
            if self.state.compare_exchange(0, -1, Ordering::Acquire, Ordering::Relaxed).is_err() {
                panic!("FrozenCell already borrowed");
            }
        }
        // SAFETY: see the module contract; only the owning thread writes, and no other borrow is live.
        FrozenRefMut {
            value: unsafe { &mut *self.value.get() },
            #[cfg(any(debug_assertions, feature = "checked-cells"))]
            state: &self.state,
        }
    }

    /// Replaces the value (owner thread only, like `borrow_mut`).
    #[inline]
    pub fn replace(&self, value: T) -> T {
        std::mem::replace(&mut *self.borrow_mut(), value)
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    #[inline]
    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: Default> Default for FrozenCell<T> {
    #[inline]
    fn default() -> Self {
        FrozenCell::new(T::default())
    }
}

impl<T: fmt::Debug> fmt::Debug for FrozenCell<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("FrozenCell").field(&*self.borrow()).finish()
    }
}

pub struct FrozenRef<'a, T> {
    value: &'a T,
    #[cfg(any(debug_assertions, feature = "checked-cells"))]
    state: &'a AtomicIsize,
}

impl<T> Deref for FrozenRef<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

#[cfg(any(debug_assertions, feature = "checked-cells"))]
impl<T> Drop for FrozenRef<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.state.fetch_sub(1, Ordering::Release);
    }
}

pub struct FrozenRefMut<'a, T> {
    value: &'a mut T,
    #[cfg(any(debug_assertions, feature = "checked-cells"))]
    state: &'a AtomicIsize,
}

impl<T> Deref for FrozenRefMut<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> DerefMut for FrozenRefMut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

#[cfg(any(debug_assertions, feature = "checked-cells"))]
impl<T> Drop for FrozenRefMut<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.state.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrow_and_mutate() {
        let c = FrozenCell::new(vec![1]);
        c.borrow_mut().push(2);
        assert_eq!(&*c.borrow(), &[1, 2]);
        let a = c.borrow();
        let b = c.borrow();
        assert_eq!(a.len() + b.len(), 4);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn aliasing_is_detected() {
        let c = FrozenCell::new(vec![1]);
        let _r = c.borrow();
        c.borrow_mut().push(2);
    }
}
