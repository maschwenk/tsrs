//! Go-style heap pointers for the port.
//!
//! The Go compiler is a graph of long-lived, mutually referencing, identity-compared
//! heap objects (nodes, symbols, types, links). We model that with a leak arena:
//! `P<T>` is a `Copy` pointer to a value that lives for the rest of the process.
//! Equality, hashing and ordering are by address, exactly like Go pointers.
//!
//! Mutable fields inside arena values use `Cell` / `RefCell`.
//!
//! Threading contract: values are only mutated by the thread that created them
//! (parser/binder per file, checker per checker). After a file is bound its AST is
//! read-only and may be shared. `P<T>` is therefore declared `Send + Sync`.

use bumpalo::Bump;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

thread_local! {
    static ARENA: &'static Bump = Box::leak(Box::new(Bump::with_capacity(1 << 20)));
}

#[inline]
fn with_arena<R>(f: impl FnOnce(&'static Bump) -> R) -> R {
    ARENA.with(|a| f(a))
}

/// Pointer to an arena value. Never null; use `Option<P<T>>` for Go's nil-able pointers
/// (it is pointer-sized).
#[repr(transparent)]
pub struct P<T: ?Sized + 'static>(&'static T);

impl<T> P<T> {
    /// Allocates `value` in the current thread's leak arena. Destructors never run.
    #[inline]
    pub fn new(value: T) -> P<T> {
        with_arena(|a| P(a.alloc(value)))
    }
}

impl<T: ?Sized> P<T> {
    #[inline]
    pub fn from_static(r: &'static T) -> P<T> {
        P(r)
    }

    /// The underlying `'static` reference (not tied to the borrow of `self`).
    #[inline]
    pub fn get(self) -> &'static T {
        self.0
    }

    #[inline]
    pub fn addr(self) -> usize {
        self.0 as *const T as *const () as usize
    }

    #[inline]
    pub fn ptr_eq(self, other: P<T>) -> bool {
        self.addr() == other.addr()
    }
}

impl<T: ?Sized> Clone for P<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for P<T> {}

impl<T: ?Sized> Deref for P<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.0
    }
}

impl<T: ?Sized> PartialEq for P<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.addr() == other.addr()
    }
}
impl<T: ?Sized> Eq for P<T> {}

impl<T: ?Sized> Hash for P<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.addr())
    }
}

impl<T: ?Sized> PartialOrd for P<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T: ?Sized> Ord for P<T> {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.addr().cmp(&other.addr())
    }
}

/// Prints the address only: arena graphs are cyclic, so a structural Debug would recurse forever.
impl<T: ?Sized> fmt::Debug for P<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P({:#x})", self.addr())
    }
}

// See the threading contract in the module docs.
unsafe impl<T: ?Sized> Send for P<T> {}
unsafe impl<T: ?Sized> Sync for P<T> {}

/// Copies a slice into the arena. Use for Go slices that are stored in long-lived objects.
#[inline]
pub fn alloc_slice<T: Copy>(items: &[T]) -> &'static [T] {
    if items.is_empty() {
        return &[];
    }
    with_arena(|a| &*a.alloc_slice_copy(items))
}

/// Moves a `Vec` of arbitrary (possibly non-`Copy`) items into the arena.
#[inline]
pub fn alloc_vec<T>(items: Vec<T>) -> &'static [T] {
    if items.is_empty() {
        return &[];
    }
    with_arena(|a| &*a.alloc_slice_fill_iter(items))
}

/// Copies a string into the arena.
#[inline]
pub fn alloc_str(s: &str) -> &'static str {
    if s.is_empty() {
        return "";
    }
    with_arena(|a| &*a.alloc_str(s))
}

/// Allocates a plain `&'static T` (for values that do not need pointer identity semantics).
#[inline]
pub fn alloc<T>(value: T) -> &'static T {
    with_arena(|a| &*a.alloc(value))
}

/// Bytes allocated so far by the current thread's arena.
pub fn arena_allocated_bytes() -> usize {
    with_arena(|a| a.allocated_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Thing {
        n: Cell<i32>,
        next: Cell<Option<P<Thing>>>,
    }

    #[test]
    fn identity_and_mutation() {
        let a = P::new(Thing { n: Cell::new(1), next: Cell::new(None) });
        let b = P::new(Thing { n: Cell::new(1), next: Cell::new(None) });
        assert!(a != b);
        assert!(a == a);
        a.next.set(Some(b));
        b.next.set(Some(a));
        a.next.get().unwrap().n.set(5);
        assert_eq!(b.n.get(), 5);
        assert_eq!(std::mem::size_of::<Option<P<Thing>>>(), std::mem::size_of::<usize>());
    }

    #[test]
    fn slices_and_strings() {
        let s = alloc_slice(&[1, 2, 3]);
        assert_eq!(s, &[1, 2, 3]);
        let v = alloc_vec(vec![String::from("a"), String::from("b")]);
        assert_eq!(v.len(), 2);
        assert_eq!(alloc_str("hello"), "hello");
    }
}
