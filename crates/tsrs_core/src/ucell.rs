//! `Cell` / `RefCell` for the fields of arena objects. In normal builds these are exactly `std::cell::{Cell,
//! RefCell}`. In the alloc-profile build they are transparent wrappers whose reads (`get`, `replace`, `take`,
//! `borrow`, ...) record a use for the use census (`usebits`); `peek` reads without recording (for code that only
//! inspects a field in order to write it, such as the mode checks of packed setters).

#[cfg(not(feature = "alloc-profile"))]
pub use std::cell::{Cell, RefCell};

/// `Cell::get` without recording a use (same as `get` outside the alloc-profile build).
#[cfg(not(feature = "alloc-profile"))]
pub trait Peek<T> {
    fn peek(&self) -> T;
}

#[cfg(not(feature = "alloc-profile"))]
impl<T: Copy> Peek<T> for Cell<T> {
    #[inline(always)]
    fn peek(&self) -> T {
        self.get()
    }
}

#[cfg(feature = "alloc-profile")]
pub use imp::{Cell, Peek, RefCell};

#[cfg(feature = "alloc-profile")]
mod imp {
    use crate::usebits::{mark_ptr, mark_value};
    #[inline(always)]
    fn mark_write<T: ?Sized>(p: &T) {
        crate::usebits::mark_write(p as *const T as *const () as usize);
    }
    use std::fmt;
    use std::ops::Deref;

    #[repr(transparent)]
    #[derive(Default)]
    pub struct Cell<T>(std::cell::Cell<T>);

    pub trait Peek<T> {
        fn peek(&self) -> T;
    }

    impl<T: Copy> Peek<T> for Cell<T> {
        #[inline(always)]
        fn peek(&self) -> T {
            self.0.get()
        }
    }

    impl<T> Cell<T> {
        #[inline(always)]
        pub const fn new(value: T) -> Cell<T> {
            Cell(std::cell::Cell::new(value))
        }
        #[inline(always)]
        pub fn set(&self, value: T) {
            mark_write(self);
            self.0.set(value)
        }
        #[inline(always)]
        pub fn replace(&self, value: T) -> T {
            mark_write(self);
            mark_ptr(self);
            let v = self.0.replace(value);
            mark_value(&v);
            v
        }
        #[inline(always)]
        pub fn into_inner(self) -> T {
            self.0.into_inner()
        }
        #[inline(always)]
        pub fn get_mut(&mut self) -> &mut T {
            self.0.get_mut()
        }
        #[inline(always)]
        pub fn swap(&self, other: &Cell<T>) {
            self.0.swap(&other.0)
        }
        #[inline(always)]
        pub fn as_ptr(&self) -> *mut T {
            self.0.as_ptr()
        }
    }

    impl<T: Copy> Cell<T> {
        #[inline(always)]
        pub fn get(&self) -> T {
            mark_ptr(self);
            let v = self.0.get();
            mark_value(&v);
            v
        }
        #[inline(always)]
        pub fn update(&self, f: impl FnOnce(T) -> T) {
            mark_write(self);
            mark_ptr(self);
            self.0.set(f(self.0.get()));
        }
    }

    impl<T: Default> Cell<T> {
        #[inline(always)]
        pub fn take(&self) -> T {
            mark_write(self);
            mark_ptr(self);
            let v = self.0.take();
            mark_value(&v);
            v
        }
    }

    impl<T: Copy> Clone for Cell<T> {
        #[inline]
        fn clone(&self) -> Cell<T> {
            Cell::new(self.get())
        }
    }

    impl<T: Copy + fmt::Debug> fmt::Debug for Cell<T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }

    impl<T: Copy + PartialEq> PartialEq for Cell<T> {
        fn eq(&self, other: &Cell<T>) -> bool {
            self.get() == other.get()
        }
    }
    impl<T: Copy + Eq> Eq for Cell<T> {}

    impl<T> From<T> for Cell<T> {
        fn from(t: T) -> Cell<T> {
            Cell::new(t)
        }
    }

    #[repr(transparent)]
    #[derive(Default)]
    pub struct RefCell<T: ?Sized>(std::cell::RefCell<T>);

    impl<T> RefCell<T> {
        #[inline(always)]
        pub const fn new(value: T) -> RefCell<T> {
            RefCell(std::cell::RefCell::new(value))
        }
        #[inline(always)]
        pub fn into_inner(self) -> T {
            self.0.into_inner()
        }
        #[inline(always)]
        pub fn replace(&self, t: T) -> T {
            mark_write(self);
            mark_ptr(self);
            self.0.replace(t)
        }
        #[inline(always)]
        pub fn replace_with<F: FnOnce(&mut T) -> T>(&self, f: F) -> T {
            mark_ptr(self);
            self.0.replace_with(f)
        }
        #[inline(always)]
        pub fn swap(&self, other: &RefCell<T>) {
            self.0.swap(&other.0)
        }
    }

    impl<T: Default> RefCell<T> {
        #[inline(always)]
        pub fn take(&self) -> T {
            mark_ptr(self);
            self.0.take()
        }
    }

    impl<T: ?Sized> RefCell<T> {
        #[inline(always)]
        #[track_caller]
        pub fn borrow(&self) -> std::cell::Ref<'_, T> {
            mark_ptr(self);
            self.0.borrow()
        }
        #[inline(always)]
        pub fn try_borrow(&self) -> Result<std::cell::Ref<'_, T>, std::cell::BorrowError> {
            mark_ptr(self);
            self.0.try_borrow()
        }
        /// A write; not a use (code that reads through `borrow_mut` reads what it wrote or is rare).
        #[inline(always)]
        #[track_caller]
        pub fn borrow_mut(&self) -> std::cell::RefMut<'_, T> {
            mark_write(self);
            self.0.borrow_mut()
        }
        #[inline(always)]
        pub fn try_borrow_mut(&self) -> Result<std::cell::RefMut<'_, T>, std::cell::BorrowMutError> {
            self.0.try_borrow_mut()
        }
        #[inline(always)]
        pub fn get_mut(&mut self) -> &mut T {
            self.0.get_mut()
        }
        #[inline(always)]
        pub fn as_ptr(&self) -> *mut T {
            self.0.as_ptr()
        }
    }

    impl<T: Clone> Clone for RefCell<T> {
        fn clone(&self) -> RefCell<T> {
            RefCell::new(self.borrow().clone())
        }
    }

    impl<T: ?Sized + fmt::Debug> fmt::Debug for RefCell<T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }

    /// For helpers written against `&std::cell::Cell<T>` (reads through them are not recorded).
    impl<T> Deref for Cell<T> {
        type Target = std::cell::Cell<T>;
        #[inline(always)]
        fn deref(&self) -> &std::cell::Cell<T> {
            &self.0
        }
    }
}
