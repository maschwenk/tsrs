//! Go-style heap pointers for the port.
//!
//! The Go compiler is a graph of long-lived, mutually referencing, identity-compared
//! heap objects (nodes, symbols, types, links). We model that with a leak arena:
//! `P<T>` is a `Copy` pointer to a value that lives for the rest of the process.
//! Equality, hashing and ordering are by address, exactly like Go pointers.
//!
//! Mutable fields inside arena values use `Cell` / `RefCell`, or `OwnedCell` / `FrozenCell` in
//! objects shared between checker threads.
//!
//! Threading contract ("Threading" in docs/PORTING.md): values are only mutated by the thread that
//! created them (parser/binder per file, checker per checker). After a file is bound its AST and
//! symbols are read-only and shared. `P<T>` is therefore declared `Send + Sync`.

use bumpalo::Bump;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

thread_local! {
    static ARENA: &'static Bump = {
        let arena: &'static Bump = Box::leak(Box::new(Bump::with_capacity(1 << 20)));
        #[cfg(any(debug_assertions, feature = "checked-cells"))]
        shared_check::register(arena);
        #[cfg(feature = "alloc-profile")]
        crate::alloc_profile::register_arena(arena);
        arena
    };
}

macro_rules! profile {
    ($ty:ty, $bytes:expr) => {
        #[cfg(feature = "alloc-profile")]
        crate::alloc_profile::record(std::panic::Location::caller(), std::any::type_name::<$ty>(), $bytes);
    };
}

#[inline]
fn with_arena<R>(f: impl FnOnce(&'static Bump) -> R) -> R {
    #[cfg(feature = "alloc-profile")]
    let _chunk = crate::alloc_profile::ArenaScope::enter();
    ARENA.with(|a| f(a))
}

/// Pointer to an arena value. Never null; use `Option<P<T>>` for Go's nil-able pointers
/// (it is pointer-sized).
#[repr(transparent)]
pub struct P<T: ?Sized + 'static>(&'static T);

impl<T> P<T> {
    /// Allocates `value` in the current thread's leak arena. Destructors never run.
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn new(value: T) -> P<T> {
        profile!(T, std::mem::size_of::<T>());
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

/// Two `&'static` slices in 24 bytes (u32 lengths) instead of 32, so an enum variant holding both still fits in a
/// 32-byte enum (Go slice headers are larger, but Go's mapper structs are separate allocations per kind).
pub struct SlicePair<A: 'static, B: 'static> {
    a: std::ptr::NonNull<A>,
    b: std::ptr::NonNull<B>,
    a_len: u32,
    b_len: u32,
}

impl<A, B> Clone for SlicePair<A, B> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<A, B> Copy for SlicePair<A, B> {}

impl<A, B> SlicePair<A, B> {
    #[inline]
    pub fn new(a: &'static [A], b: &'static [B]) -> Self {
        assert!(a.len() <= u32::MAX as usize && b.len() <= u32::MAX as usize);
        SlicePair {
            a: std::ptr::NonNull::from(a).cast(),
            b: std::ptr::NonNull::from(b).cast(),
            a_len: a.len() as u32,
            b_len: b.len() as u32,
        }
    }
    #[inline]
    pub fn first(&self) -> &'static [A] {
        // SAFETY: built from a `&'static [A]` of this length.
        unsafe { std::slice::from_raw_parts(self.a.as_ptr(), self.a_len as usize) }
    }
    #[inline]
    pub fn second(&self) -> &'static [B] {
        // SAFETY: built from a `&'static [B]` of this length.
        unsafe { std::slice::from_raw_parts(self.b.as_ptr(), self.b_len as usize) }
    }
}

// Same contract as `P`.
unsafe impl<A, B> Send for SlicePair<A, B> {}
unsafe impl<A, B> Sync for SlicePair<A, B> {}

/// The data pointer of a `&'static [T]` whose length is stored next to it by the owner (so an enum variant can
/// keep the length in the space after its tag instead of a 16-byte slice reference).
pub struct StaticSlicePtr<T: 'static>(std::ptr::NonNull<T>);

impl<T> Clone for StaticSlicePtr<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for StaticSlicePtr<T> {}

impl<T> StaticSlicePtr<T> {
    #[inline]
    pub fn new(s: &'static [T]) -> Self {
        StaticSlicePtr(std::ptr::NonNull::from(s).cast())
    }
    /// The slice back.
    ///
    /// # Safety
    /// `len` must be the length of the slice this pointer was built from.
    #[inline]
    pub unsafe fn slice(self, len: usize) -> &'static [T] {
        std::slice::from_raw_parts(self.0.as_ptr(), len)
    }
}

// Same contract as `P`.
unsafe impl<T> Send for StaticSlicePtr<T> {}
unsafe impl<T> Sync for StaticSlicePtr<T> {}

/// A `&'static str` in 8 bytes: the data pointer in the low 48 bits and the length in the high 16. A string of
/// `u16::MAX` bytes or more (or one whose address does not fit in 48 bits) is copied into the arena after a `u32`
/// length, and the length bits hold `u16::MAX`. `as_str` returns the same text (for short strings, the same slice).
#[derive(Clone, Copy)]
pub struct PackedStr(std::ptr::NonNull<u8>);

const PACKED_STR_LEN_SHIFT: u32 = 48;
const PACKED_STR_LONG: usize = u16::MAX as usize;

impl PackedStr {
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn new(s: &'static str) -> PackedStr {
        let p = std::ptr::NonNull::from(s).cast::<u8>();
        if s.len() < PACKED_STR_LONG && p.addr().get() >> PACKED_STR_LEN_SHIFT == 0 {
            return PackedStr(p.map_addr(|a| a | (s.len() << PACKED_STR_LEN_SHIFT)));
        }
        PackedStr::new_long(s)
    }

    #[cold]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    fn new_long(s: &str) -> PackedStr {
        let len = u32::try_from(s.len()).expect("string longer than u32::MAX");
        let layout = std::alloc::Layout::from_size_align(4 + s.len(), 4).unwrap();
        profile!(PackedStr, layout.size());
        let p = with_arena(|a| a.alloc_layout(layout));
        // SAFETY: `p` points to `4 + len` fresh bytes, 4-byte aligned.
        unsafe {
            p.cast::<u32>().as_ptr().write(len);
            std::ptr::copy_nonoverlapping(s.as_ptr(), p.as_ptr().add(4), s.len());
        }
        assert!(p.addr().get() >> PACKED_STR_LEN_SHIFT == 0, "arena address above 2^48");
        PackedStr(p.map_addr(|a| a | (PACKED_STR_LONG << PACKED_STR_LEN_SHIFT)))
    }

    #[inline]
    pub fn as_str(self) -> &'static str {
        let len = self.0.addr().get() >> PACKED_STR_LEN_SHIFT;
        let p = self.0.as_ptr().map_addr(|a| a & ((1 << PACKED_STR_LEN_SHIFT) - 1));
        // SAFETY: built by `new` from a `&'static str` of this length, or by `new_long` (length prefix + bytes).
        unsafe {
            let (p, len) = if len == PACKED_STR_LONG { (p.add(4) as *const u8, *(p as *const u32) as usize) } else { (p as *const u8, len) };
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, len))
        }
    }
}

// Same contract as `P`.
unsafe impl Send for PackedStr {}
unsafe impl Sync for PackedStr {}

impl fmt::Debug for PackedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

#[repr(C, packed(4))]
struct PackedSlice<T: 'static> {
    ptr: std::ptr::NonNull<T>,
    len: u32,
}

impl<T> Clone for PackedSlice<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for PackedSlice<T> {}

/// A `Cell<&'static [T]>` in 12 bytes with 4-byte alignment (data pointer + `u32` length), so a struct can pack
/// it with other 4-byte fields instead of padding a 16-byte slice reference. `get` returns exactly the slice
/// last `set` (same pointer and length).
pub struct SliceCell<T: 'static>(std::cell::Cell<PackedSlice<T>>);

impl<T> SliceCell<T> {
    #[inline]
    pub fn new(s: &'static [T]) -> Self {
        SliceCell(std::cell::Cell::new(Self::pack(s)))
    }
    #[inline]
    fn pack(s: &'static [T]) -> PackedSlice<T> {
        PackedSlice { ptr: std::ptr::NonNull::from(s).cast(), len: u32::try_from(s.len()).expect("slice longer than u32::MAX") }
    }
    #[inline]
    pub fn get(&self) -> &'static [T] {
        let p = self.0.get();
        // SAFETY: built from a `&'static [T]` of this length.
        unsafe { std::slice::from_raw_parts(p.ptr.as_ptr(), p.len as usize) }
    }
    #[inline]
    pub fn set(&self, s: &'static [T]) {
        self.0.set(Self::pack(s))
    }
}

impl<T> Default for SliceCell<T> {
    fn default() -> Self {
        SliceCell::new(&[])
    }
}

/// Copies a slice into the arena. Use for Go slices that are stored in long-lived objects.
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_slice<T: Copy>(items: &[T]) -> &'static [T] {
    if items.is_empty() {
        return &[];
    }
    profile!([T], std::mem::size_of_val(items));
    with_arena(|a| &*a.alloc_slice_copy(items))
}

/// Moves a `Vec` of arbitrary (possibly non-`Copy`) items into the arena.
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_vec<T>(items: Vec<T>) -> &'static [T] {
    if items.is_empty() {
        return &[];
    }
    profile!([T], std::mem::size_of_val(&items[..]));
    with_arena(|a| &*a.alloc_slice_fill_iter(items))
}

/// Copies a string into the arena.
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_str(s: &str) -> &'static str {
    if s.is_empty() {
        return "";
    }
    profile!(str, s.len());
    with_arena(|a| &*a.alloc_str(s))
}

/// Allocates a plain `&'static T` (for values that do not need pointer identity semantics).
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc<T>(value: T) -> &'static T {
    profile!(T, std::mem::size_of::<T>());
    with_arena(|a| &*a.alloc(value))
}

/// Prints the allocation profile (no-op unless built with `--features tsrs_core/alloc-profile`).
pub fn alloc_profile_dump() {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::dump();
}

/// Bytes allocated so far by the current thread's arena.
pub fn arena_allocated_bytes() -> usize {
    with_arena(|a| a.allocated_bytes())
}

/// Debug aid for the threading contract (checked builds only, opt-in with `TSRS_CHECK_SHARED=1`):
/// `freeze_shared_objects` records every arena allocation made so far (the parsed and bound program) as
/// shared, and `assert_not_shared` panics when a checker writes to such an object. Release builds without
/// `checked-cells` compile both to nothing.
pub mod shared_check {
    use bumpalo::Bump;
    use std::sync::{Mutex, OnceLock, RwLock};

    struct ArenaRef(&'static Bump);
    // SAFETY: only used to read chunk bounds while the owning threads are idle.
    unsafe impl Send for ArenaRef {}

    static ARENAS: Mutex<Vec<ArenaRef>> = Mutex::new(Vec::new());
    static FROZEN: RwLock<Vec<(usize, usize)>> = RwLock::new(Vec::new());

    #[cfg(any(debug_assertions, feature = "checked-cells"))]
    pub(super) fn register(arena: &'static Bump) {
        ARENAS.lock().unwrap().push(ArenaRef(arena));
    }

    #[inline]
    pub fn enabled() -> bool {
        if !cfg!(any(debug_assertions, feature = "checked-cells")) {
            return false;
        }
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var("TSRS_CHECK_SHARED").is_ok_and(|v| v == "1"))
    }

    /// Forgets the recorded shared objects (a new program is about to be bound).
    pub fn thaw() {
        FROZEN.write().unwrap().clear();
    }

    /// Call while no other thread allocates (between binding and checking).
    pub fn freeze_shared_objects() {
        if !enabled() {
            return;
        }
        let mut ranges = Vec::new();
        for arena in ARENAS.lock().unwrap().iter() {
            // SAFETY: the owning threads are idle; the chunks are only read.
            for (ptr, len) in unsafe { arena.0.iter_allocated_chunks_raw() } {
                ranges.push((ptr as usize, ptr as usize + len));
            }
        }
        ranges.sort_unstable();
        *FROZEN.write().unwrap() = ranges;
    }

    #[inline]
    pub fn assert_not_shared<T: ?Sized>(object: &T, what: &str) {
        if !enabled() {
            return;
        }
        let addr = object as *const T as *const () as usize;
        let frozen = FROZEN.read().unwrap();
        let i = frozen.partition_point(|&(start, _)| start <= addr);
        if i > 0 && addr < frozen[i - 1].1 {
            panic!("write to shared {what} at {addr:#x} after binding (threading contract, docs/PORTING.md)");
        }
    }
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

    #[test]
    fn packed_str() {
        let short: &'static str = "identifier";
        assert!(std::ptr::eq(PackedStr::new(short).as_str(), short));
        assert_eq!(PackedStr::new("").as_str(), "");
        let long: &'static str = Box::leak("x".repeat(u16::MAX as usize + 3).into_boxed_str());
        assert_eq!(PackedStr::new(long).as_str(), long);
        let exact: &'static str = Box::leak("é".repeat(u16::MAX as usize / 2 + 1).into_boxed_str());
        assert_eq!(PackedStr::new(exact).as_str(), exact);
    }
}
