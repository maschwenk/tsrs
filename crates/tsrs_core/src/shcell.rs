//! spike/r1-read-path (research, never merged): `ShCell` and its variants, the cells of the lazily filled fields that a
//! fork of the shared type layer would write on frozen objects (notes/design-shared-type-layer.md section 5.2: the
//! spike's `OvCell` with the constant window test of `shwindow` instead of its loads of globals). A set value reads as
//! is; an unset one and every write take one window test. Nothing is in the window on this branch, so the cold paths,
//! which keep a fork's values in a per-thread side table keyed by the cell's address, never run.

use crate::shwindow::shared_ref;

// The side table a fork would keep for the lazy fields it fills on shared objects (by the cell's address). Never
// filled here; the cold paths below read and write it as the layer would.
thread_local! {
    static SIDE: std::cell::RefCell<rustc_hash::FxHashMap<usize, [u64; 2]>> = std::cell::RefCell::new(rustc_hash::FxHashMap::default());
}

#[inline]
fn encode<T: Copy>(v: T) -> [u64; 2] {
    const { assert!(std::mem::size_of::<T>() <= 16) };
    let mut out = [0u64; 2];
    // SAFETY: T fits in 16 bytes; a plain byte copy of a Copy value.
    unsafe { std::ptr::copy_nonoverlapping(std::ptr::from_ref(&v).cast::<u8>(), out.as_mut_ptr().cast::<u8>(), std::mem::size_of::<T>()) };
    out
}

#[inline]
fn decode<T: Copy>(bits: [u64; 2]) -> T {
    // SAFETY: `bits` was made by `encode::<T>` of a valid T.
    unsafe { std::ptr::read_unaligned(bits.as_ptr().cast::<T>()) }
}

/// A shared object's lazy field as this thread's checker filled it, else `v` (the shared value).
#[cold]
#[inline(never)]
pub fn side_get<T: Copy>(addr: usize, v: T) -> T {
    SIDE.with(|s| s.borrow().get(&addr).map_or(v, |&b| decode(b)))
}

/// Records this thread's checker's value of a shared object's lazy field.
#[cold]
#[inline(never)]
pub fn side_set<T: Copy>(addr: usize, v: T) {
    SIDE.with(|s| {
        s.borrow_mut().insert(addr, encode(v));
    });
}

/// A `Cell` for a lazily filled field of an arena object that the shared layer may hold (the fields a fork writes on a
/// frozen object). A set value reads as is; an unset one (the default) and every write take one window test.
#[repr(transparent)]
pub struct ShCell<T>(std::cell::Cell<T>);

impl<T: Copy + Default + PartialEq> Default for ShCell<T> {
    #[inline]
    fn default() -> Self {
        ShCell(std::cell::Cell::new(T::default()))
    }
}

impl<T: Copy + std::fmt::Debug> std::fmt::Debug for ShCell<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

impl<T: Copy + Default + PartialEq> ShCell<T> {
    #[inline]
    pub const fn new(v: T) -> Self {
        ShCell(std::cell::Cell::new(v))
    }

    #[inline]
    pub fn get(&self) -> T {
        let v = self.0.get();
        if v != T::default() || !shared_ref(self) {
            return v;
        }
        side_get(std::ptr::from_ref(self).addr(), v)
    }

    #[inline]
    pub fn set(&self, v: T) {
        if shared_ref(self) {
            return side_set(std::ptr::from_ref(self).addr(), v);
        }
        self.0.set(v)
    }

    #[inline]
    pub fn replace(&self, v: T) -> T {
        let old = self.get();
        self.set(v);
        old
    }

    #[inline]
    pub fn take(&self) -> T {
        self.replace(T::default())
    }
}

/// `ShCell` for an `i32` whose unset value is -1 (`Signature.resolved_min_argument_count`).
#[repr(transparent)]
pub struct ShCountCell(std::cell::Cell<i32>);

impl ShCountCell {
    #[inline]
    pub const fn new(v: i32) -> Self {
        ShCountCell(std::cell::Cell::new(v))
    }

    #[inline]
    pub fn get(&self) -> i32 {
        let v = self.0.get();
        if v != -1 || !shared_ref(self) {
            return v;
        }
        side_get(std::ptr::from_ref(self).addr(), v)
    }

    #[inline]
    pub fn set(&self, v: i32) {
        if shared_ref(self) {
            return side_set(std::ptr::from_ref(self).addr(), v);
        }
        self.0.set(v)
    }
}

impl Default for ShCountCell {
    #[inline]
    fn default() -> Self {
        ShCountCell::new(0)
    }
}

/// `ShCell` for a pointer word with a tag in bit 0 (a rare tail): unset while the pointer part is null.
pub struct ShTaggedPtrCell<T>(std::cell::Cell<*const T>);

impl<T> ShTaggedPtrCell<T> {
    #[inline]
    pub const fn new(p: *const T) -> Self {
        ShTaggedPtrCell(std::cell::Cell::new(p))
    }

    #[inline]
    pub fn get(&self) -> *const T {
        let p = self.0.get();
        if p.addr() > 1 || !shared_ref(self) {
            return p;
        }
        side_get(std::ptr::from_ref(self).addr(), p)
    }

    #[inline]
    pub fn set(&self, p: *const T) {
        if shared_ref(self) {
            return side_set(std::ptr::from_ref(self).addr(), p);
        }
        self.0.set(p)
    }
}

/// `OptionThinSliceCell` whose unset value is `None`, as `ShCell`.
pub struct ShOptionThinSliceCell<T: 'static>(std::cell::Cell<Option<crate::ThinSlice<T>>>);

impl<T> ShOptionThinSliceCell<T> {
    #[inline]
    pub fn new(s: Option<&'static [T]>) -> Self {
        ShOptionThinSliceCell(std::cell::Cell::new(s.map(crate::ThinSlice::new)))
    }

    #[inline]
    pub fn get(&self) -> Option<&'static [T]> {
        match self.0.get() {
            Some(s) => Some(s.get()),
            None if !shared_ref(self) => None,
            None => side_get(std::ptr::from_ref(self).addr(), None::<crate::ThinSlice<T>>).map(crate::ThinSlice::get),
        }
    }

    #[inline]
    pub fn set(&self, s: Option<&'static [T]>) {
        let v = s.map(crate::ThinSlice::new);
        if shared_ref(self) {
            return side_set(std::ptr::from_ref(self).addr(), v);
        }
        self.0.set(v)
    }
}

impl<T> Default for ShOptionThinSliceCell<T> {
    fn default() -> Self {
        ShOptionThinSliceCell::new(None)
    }
}

/// `StrCell` whose unset value is "", as `ShCell`.
pub struct ShStrCell(std::cell::Cell<crate::PackedStr>);

impl ShStrCell {
    #[inline]
    pub fn new(s: &'static str) -> Self {
        ShStrCell(std::cell::Cell::new(crate::PackedStr::new(s)))
    }

    #[inline]
    pub fn get(&self) -> &'static str {
        let s = self.0.get().as_str();
        if !s.is_empty() || !shared_ref(self) {
            return s;
        }
        side_get(std::ptr::from_ref(self).addr(), self.0.get()).as_str()
    }

    #[inline]
    pub fn set(&self, s: &'static str) {
        let v = crate::PackedStr::new(s);
        if shared_ref(self) {
            return side_set(std::ptr::from_ref(self).addr(), v);
        }
        self.0.set(v)
    }
}

impl Default for ShStrCell {
    fn default() -> Self {
        ShStrCell::new("")
    }
}
