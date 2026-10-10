//! Go-style heap pointers for the port.
//!
//! The Go compiler is a graph of long-lived, mutually referencing, identity-compared
//! heap objects (nodes, symbols, types, links). We model that with a leak arena:
//! `P<T>` is a `Copy` pointer to a value that lives for the rest of the process.
//! Equality, hashing and ordering are by identity, like Go pointers. With compressed pointers (the default on
//! unix, `cfg(compressed_ptrs)`) a `P<T>` is a 32-bit handle: the value's offset in 8-byte units from the one
//! reserved range every arena allocates from (`reserve`). Handles are position independent: nothing that stores a
//! `P` (or its `to_bits` / `key`) stores an address.
//!
//! Mutable fields inside arena values use `Cell` / `RefCell`, or `OwnedCell` / `FrozenCell` in
//! objects shared between checker threads.
//!
//! Threading contract ("Threading" in docs/PORTING.md): values are only mutated by the thread that
//! created them (parser/binder per file, checker per checker). After a file is bound its AST and
//! symbols are read-only and shared. `P<T>` is therefore declared `Send + Sync`.

use crate::arena::{self, Arena, Checkpoint};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

thread_local! {
    /// The thread's own arena, set on first use (`own_arena`) and cleared by `release_own_arena`.
    static ARENA: std::cell::Cell<Option<&'static Arena>> = const { std::cell::Cell::new(None) };
}

fn new_thread_arena() -> &'static Arena {
    let arena: &'static Arena = Box::leak(Box::new(Arena::new()));
    #[cfg(any(debug_assertions, feature = "checked-cells"))]
    shared_check::register(arena);
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::register_arena(arena);
    crate::memsplit::register_arena(arena);
    arena
}

macro_rules! profile {
    ($ty:ty, $bytes:expr_2021, $addr:expr_2021) => {
        #[cfg(feature = "alloc-profile")]
        crate::alloc_profile::record(std::panic::Location::caller(), std::any::type_name::<$ty>(), $bytes, $addr);
    };
}

/// The current thread's own arena (never freed): on first use, an arena that a thread done allocating gave back
/// (`release_own_arena`), else a new one.
pub(crate) fn own_arena() -> &'static Arena {
    ARENA.with(|c| match c.get() {
        Some(a) => a,
        None => {
            let a = arena::take_spare_arena().unwrap_or_else(new_thread_arena);
            c.set(Some(a));
            a
        }
    })
}

/// Hands the calling thread's own arena to the next thread that needs one, for a thread that is done allocating (a
/// checker thread at the end of its task, a parse worker once the program is loaded). That thread continues in the
/// arena's current chunk, whose unused end is resident when the chunk has transparent huge pages (the partly used
/// 2 MiB block under the finger), instead of starting a chunk of its own (notes/mem-linux-residency-32.md). The
/// arena's objects stay where they are; the next owner only allocates below the finger and reuses its free lists,
/// whose blocks are dead. An allocation on this thread afterwards takes another arena. Does nothing while a region,
/// thread-arena or scratch scope is open (the scope stack names the arena).
pub fn release_own_arena() {
    let Some(a) = ARENA.with(std::cell::Cell::get) else {
        return;
    };
    if !arena::no_scope_open() {
        return;
    }
    ARENA.with(|c| c.set(None));
    arena::give_spare_arena(a);
}

#[cold]
#[inline(never)]
fn init_current() -> &'static Arena {
    let a = own_arena();
    arena::CURRENT.with(|c| c.set(a));
    a
}

/// The current allocation target: a region entered on this thread (`arena::Region::enter`), else the thread's own
/// arena. A region is not `'static`, but it outlives every allocation made while it is entered by contract (see
/// `arena`), like the thread arena outlives everything.
#[inline]
fn with_arena<R>(f: impl FnOnce(&'static Arena) -> R) -> R {
    #[cfg(feature = "alloc-profile")]
    let _chunk = crate::alloc_profile::ArenaScope::enter();
    let p = arena::CURRENT.with(|c| c.get());
    // SAFETY: non-null `CURRENT` is the thread arena or a region kept alive by the entered scope.
    let a = if p.is_null() { init_current() } else { unsafe { &*p } };
    f(a)
}

/// The thread's scratch region (`arena::Region::enter_scratch`) if one is entered, else the current target
/// (`with_arena`).
#[inline]
fn with_scratch_arena<R>(f: impl FnOnce(&'static Arena) -> R) -> R {
    let s = arena::scratch_arena();
    if s.is_null() {
        return with_arena(f);
    }
    #[cfg(feature = "alloc-profile")]
    let _chunk = crate::alloc_profile::ArenaScope::enter();
    // SAFETY: the scratch region is kept alive (and locked by this thread) by its entered `ScratchScope`.
    f(unsafe { &*s })
}

/// The layout `P::new` allocates for a `T`. Compressed handles count 8-byte units, so every `P` target is 8-aligned
/// and padded to 8 (the padding keeps free-list classes, which assume 8-aligned blocks of `size` bytes, exact).
#[inline]
pub(crate) const fn p_layout<T>() -> std::alloc::Layout {
    let l = std::alloc::Layout::new::<T>();
    if cfg!(compressed_ptrs) {
        match l.align_to(8) {
            Ok(l) => l.pad_to_align(),
            Err(_) => panic!("layout"),
        }
    } else {
        l
    }
}

/// Pointer to an arena value. Never null; use `Option<P<T>>` for Go's nil-able pointers
/// (it is pointer-sized).
#[cfg(not(compressed_ptrs))]
#[repr(transparent)]
pub struct P<T: ?Sized + 'static>(&'static T);

/// Handle of an arena value: its offset from `reserve::base()` in 8-byte units (notes/mem-pointer-compression.md).
/// Never 0; `Option<P<T>>` is 4 bytes too.
#[cfg(compressed_ptrs)]
#[repr(transparent)]
pub struct P<T: ?Sized + 'static>(std::num::NonZeroU32, std::marker::PhantomData<&'static T>);

impl<T> P<T> {
    /// Allocates `value` in the current thread's leak arena. Destructors never run.
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn new(value: T) -> P<T> {
        let p = with_arena(|a| {
            let r = a.alloc_with(p_layout::<T>(), value);
            a.track_drop(std::ptr::from_mut::<T>(r), 1);
            // SAFETY: a fresh block of the current arena, laid out by `p_layout`.
            unsafe { P::from_arena(r) }
        });
        profile!(T, p_layout::<T>().size(), p.addr());
        p
    }

    /// `P::new` in the thread's scratch region if one is entered (`arena::Region::enter_scratch`), even while escaped
    /// from it: for values known to die with the scratch region (a node builder call's state, emit's nodes).
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn new_scratch(value: T) -> P<T> {
        let p = with_scratch_arena(|a| {
            let r = a.alloc_with(p_layout::<T>(), value);
            a.track_drop(std::ptr::from_mut::<T>(r), 1);
            // SAFETY: a fresh block of the arena, laid out by `p_layout`.
            unsafe { P::from_arena(r) }
        });
        profile!(T, p_layout::<T>().size(), p.addr());
        p
    }

    /// `P::new` or `P::new_scratch`.
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn new_in(scratch: bool, value: T) -> P<T> {
        if scratch {
            P::new_scratch(value)
        } else {
            P::new(value)
        }
    }

    /// `P::new` that first takes a block of the right size from the current thread's free list (see `free`).
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn new_recycled(value: T) -> P<T> {
        let class = const { arena::free_class(p_layout::<T>().size(), p_layout::<T>().align()) };
        if class != 0 {
            if let Some((block, a)) = with_arena(|a| a.pop_free(class).map(|b| (b, a))) {
                let p = block.cast::<T>();
                // SAFETY: a dead block of exactly `p_layout::<T>().size()` bytes, 8-aligned (`free_class`).
                unsafe { p.as_ptr().write(value) };
                a.track_drop(p.as_ptr(), 1);
                profile!(T, p_layout::<T>().size(), p.addr().get());
                // SAFETY: as above; the block is now owned by the new value.
                return unsafe { P::from_arena(&*p.as_ptr()) };
            }
        }
        P::new(value)
    }

    /// `from_static` without its compressed-mode check, for hot paths whose reference is known to come from the arena.
    ///
    /// # Safety
    /// `r` must be an 8-aligned object in the arena: a block allocated with `p_layout` (`P::new`, `alloc`), or a view
    /// of one that starts 8-aligned inside it.
    #[inline]
    pub unsafe fn from_arena(r: &'static T) -> P<T> {
        #[cfg(compressed_ptrs)]
        {
            let off = std::ptr::from_ref::<T>(r).addr().wrapping_sub(crate::reserve::base().addr());
            debug_assert!(off < crate::reserve::RESERVE && off & 7 == 0 && off != 0, "P::from_arena outside the arena");
            // SAFETY: `r` is an arena object (this function's contract), and the reservation's first granule is never
            // handed out, so its handle is not 0.
            P(unsafe { std::num::NonZeroU32::new_unchecked((off >> crate::reserve::UNIT_SHIFT) as u32) }, std::marker::PhantomData)
        }
        #[cfg(not(compressed_ptrs))]
        P(r)
    }

    /// The arena object at `bits`, which `to_bits` returned (tag bits cleared).
    ///
    /// # Safety
    /// `bits` must come from `to_bits` of a live `P<T>` (same `T`, or a type whose value starts there).
    #[inline]
    pub unsafe fn from_bits(bits: usize) -> P<T> {
        #[cfg(compressed_ptrs)]
        {
            debug_assert!(bits != 0 && bits & 7 == 0);
            // SAFETY: `bits` came from `to_bits` of a live `P<T>` (this function's contract), whose handle is not 0.
            P(unsafe { std::num::NonZeroU32::new_unchecked((bits >> crate::reserve::UNIT_SHIFT) as u32) }, std::marker::PhantomData)
        }
        #[cfg(not(compressed_ptrs))]
        // SAFETY: `bits` is the exposed address of a live arena object (this function's contract).
        P(unsafe { &*std::ptr::with_exposed_provenance::<T>(bits) })
    }

    /// The object packed in the low bits of `w` by `pack` (higher bits may hold anything).
    ///
    /// # Safety
    /// The low `PACK_BITS` bits of `w` must come from `pack` of a live `P<T>`.
    #[inline]
    pub unsafe fn unpack(w: u64) -> P<T> {
        #[cfg(compressed_ptrs)]
        // SAFETY: the low bits came from `pack` of a live `P<T>` (this function's contract): its handle, not 0.
        return P(unsafe { std::num::NonZeroU32::new_unchecked(w as u32) }, std::marker::PhantomData);
        #[cfg(all(not(compressed_ptrs), target_pointer_width = "64"))]
        // SAFETY: the low bits are the address of a live arena object, shifted by `pack` (this function's contract).
        return P(unsafe { &*std::ptr::with_exposed_provenance::<T>(((w & PACK_MASK) << 3) as usize) });
        #[cfg(target_pointer_width = "32")]
        // SAFETY: the low bits are the address of a live arena object, unshifted on 32-bit targets (`pack`).
        return P(unsafe { &*std::ptr::with_exposed_provenance::<T>((w & PACK_MASK) as usize) });
    }

    /// `unpack` for an optional pointer (low bits 0 = `None`).
    ///
    /// # Safety
    /// As `unpack`, or the low `PACK_BITS` bits are 0.
    #[inline]
    pub unsafe fn unpack_opt(w: u64) -> Option<P<T>> {
        #[cfg(compressed_ptrs)]
        // SAFETY: nonzero low bits came from `pack` (this function's contract).
        return (w as u32 != 0).then(|| unsafe { P::unpack(w) });
        #[cfg(not(compressed_ptrs))]
        // SAFETY: nonzero low bits came from `pack` (this function's contract).
        return (w & PACK_MASK != 0).then(|| unsafe { P::unpack(w) });
    }

    /// The object whose `key` is `key`.
    ///
    /// # Safety
    /// `key` must come from `key` of a live `P<T>` (same `T`, or a type whose value starts there).
    #[inline]
    pub unsafe fn from_key(key: PKey) -> P<T> {
        #[cfg(compressed_ptrs)]
        // SAFETY: `key` came from `key` of a live `P<T>` (this function's contract): its handle, not 0.
        return P(unsafe { std::num::NonZeroU32::new_unchecked(key) }, std::marker::PhantomData);
        #[cfg(not(compressed_ptrs))]
        // SAFETY: `key` is the exposed address of a live arena object (this function's contract).
        return P(unsafe { &*std::ptr::with_exposed_provenance::<T>(key) });
    }

    /// `from_key` for an optional pointer: 0 is `None`.
    ///
    /// # Safety
    /// As `from_key`.
    #[inline]
    pub unsafe fn from_key_opt(key: PKey) -> Option<P<T>> {
        // SAFETY: a nonzero `key` came from `key` of a live `P<T>` (this function's contract).
        (key != 0).then(|| unsafe { P::from_key(key) })
    }

    /// `from_bits` for an optional pointer: 0 is `None`.
    ///
    /// # Safety
    /// As `from_bits`.
    #[inline]
    pub unsafe fn from_bits_opt(bits: usize) -> Option<P<T>> {
        // SAFETY: nonzero `bits` came from `to_bits` of a live `P<T>` (this function's contract).
        (bits != 0).then(|| unsafe { P::from_bits(bits) })
    }

    /// The `n`-th element after this one in an arena array of `T` (pointer `add`; not named `add`, which would shadow `add` methods of `T`). Compressed: handle arithmetic,
    /// no address round trip (`T`'s size must be a multiple of 8, like every `P` target's).
    ///
    /// # Safety
    /// `self` must be an element of an arena array with at least `n` more elements.
    #[inline]
    pub unsafe fn array_add(self, n: usize) -> P<T> {
        #[cfg(compressed_ptrs)]
        {
            const { assert!(std::mem::size_of::<T>() % 8 == 0, "P::array_add needs 8-byte multiples") };
            let h = self.0.get() + (n * (std::mem::size_of::<T>() >> crate::reserve::UNIT_SHIFT)) as u32;
            // SAFETY: `h` is past a nonzero handle, inside the same array (this function's contract).
            return P(unsafe { std::num::NonZeroU32::new_unchecked(h) }, std::marker::PhantomData);
        }
        #[cfg(not(compressed_ptrs))]
        // SAFETY: the array has at least `n` more elements after `self` (this function's contract).
        P(unsafe { &*std::ptr::from_ref::<T>(self.0).add(n) })
    }

    /// The same object viewed as a `U` (a header at the start of a larger allocation, or the reverse).
    ///
    /// # Safety
    /// A `U` must live at this address.
    #[inline]
    pub unsafe fn cast<U>(self) -> P<U> {
        #[cfg(compressed_ptrs)]
        return P(self.0, std::marker::PhantomData);
        #[cfg(not(compressed_ptrs))]
        // SAFETY: the caller guarantees that a `U` lives at this address.
        P(unsafe { &*(self.0 as *const T).cast::<U>() })
    }

    /// A pointer to an arena object (`P::get` of a live `P`, an object made with `alloc`, or a view of one that
    /// starts 8-aligned inside it). In compressed mode this is checked: anything outside the arena range (a static,
    /// a heap object) or not 8-aligned panics.
    #[inline]
    #[cfg(compressed_ptrs)]
    #[track_caller]
    pub fn from_static(r: &'static T) -> P<T> {
        let off = std::ptr::from_ref::<T>(r).addr().wrapping_sub(crate::reserve::base().addr());
        if off >= crate::reserve::RESERVE || off & 7 != 0 || off == 0 {
            from_static_failed(std::any::type_name::<T>(), std::ptr::from_ref::<T>(r).addr());
        }
        P(std::num::NonZeroU32::new((off >> crate::reserve::UNIT_SHIFT) as u32).unwrap(), std::marker::PhantomData)
    }

    /// The handle (compressed mode only): the object's offset from `reserve::base()` in 8-byte units.
    #[inline]
    #[cfg(compressed_ptrs)]
    pub fn handle(self) -> u32 {
        self.0.get()
    }
}

#[cfg(compressed_ptrs)]
#[cold]
#[inline(never)]
#[track_caller]
fn from_static_failed(ty: &str, addr: usize) -> ! {
    panic!("P::<{ty}>::from_static({addr:#x}): not an 8-aligned object in the arena range (compressed pointers)")
}

#[cfg(not(compressed_ptrs))]
impl<T: ?Sized> P<T> {
    #[inline]
    pub const fn from_static(r: &'static T) -> P<T> {
        P(r)
    }

    /// The underlying `'static` reference (not tied to the borrow of `self`).
    #[inline]
    pub fn get(self) -> &'static T {
        #[cfg(feature = "alloc-profile")]
        check_not_freed(self.0);
        self.0
    }

    #[inline]
    pub fn addr(self) -> usize {
        self.0 as *const T as *const () as usize
    }

    /// Position-independent bits for packed words: the address here, the offset from the arena base in compressed
    /// mode. 8-aligned and below 2^48 in both (the arena aligns every `P` target to 8; user-space addresses are below
    /// 2^48 on every supported platform), so callers may keep tags in the low 3 and the high 16 bits.
    #[inline]
    pub fn to_bits(self) -> usize {
        (self.0 as *const T as *const ()).expose_provenance()
    }
}

#[cfg(compressed_ptrs)]
impl<T: ?Sized> P<T> {
    /// The value (a `'static` reference, not tied to the borrow of `self`).
    #[inline]
    pub fn get(self) -> &'static T
    where
        T: Sized,
    {
        // SAFETY: a handle is the offset of a live arena object from the base of the reservation it lives in.
        let r = unsafe { &*crate::reserve::base().add((self.0.get() as usize) << crate::reserve::UNIT_SHIFT).cast::<T>() };
        #[cfg(feature = "alloc-profile")]
        check_not_freed(r);
        r
    }

    /// The object's address.
    #[inline]
    pub fn addr(self) -> usize {
        crate::reserve::base().addr() + ((self.0.get() as usize) << crate::reserve::UNIT_SHIFT)
    }

    /// See the pointer-mode `to_bits`: here the byte offset from the base.
    #[inline]
    pub fn to_bits(self) -> usize {
        (self.0.get() as usize) << crate::reserve::UNIT_SHIFT
    }
}

/// Bits of a word that `P::pack` uses (the rest belongs to the caller).
pub const PACK_BITS: u32 = 45;
#[cfg(not(compressed_ptrs))]
const PACK_MASK: u64 = (1 << PACK_BITS) - 1;

/// The narrowest integer that identifies a `P` (`P::key`): the handle in compressed mode, else the address.
#[cfg(compressed_ptrs)]
pub type PKey = u32;
#[cfg(not(compressed_ptrs))]
pub type PKey = usize;

impl<T: ?Sized> P<T> {
    #[inline]
    pub fn ptr_eq(self, other: P<T>) -> bool {
        self == other
    }

    /// Identity as an integer, for tables keyed by object (eq and hash agree with `P`'s) and for packed fields
    /// (`from_key` turns it back; never 0).
    #[inline]
    pub fn key(self) -> PKey {
        #[cfg(compressed_ptrs)]
        return self.0.get();
        #[cfg(not(compressed_ptrs))]
        return (self.0 as *const T as *const ()).expose_provenance();
    }

    /// The pointer in the low `PACK_BITS` (45) bits of a word whose higher bits belong to the caller: the handle with
    /// compressed pointers (bits 32..45 stay 0, so `unpack` reads the low 32 bits and needs no mask), else the
    /// address / 8. 0 for `None` (`pack_opt`).
    #[inline]
    pub fn pack(self) -> u64 {
        #[cfg(compressed_ptrs)]
        return self.0.get() as u64;
        #[cfg(all(not(compressed_ptrs), target_pointer_width = "64"))]
        {
            let a = (self.0 as *const T as *const ()).expose_provenance() as u64;
            assert!(a & 7 == 0 && a >> (PACK_BITS + 3) == 0, "address {a:#x} does not pack in 45 bits");
            a >> 3
        }
        // 32-bit targets: objects may be only 4-aligned, and every address fits in `PACK_BITS` unshifted.
        #[cfg(target_pointer_width = "32")]
        return (self.0 as *const T as *const ()).expose_provenance() as u64;
    }

    #[inline]
    pub fn pack_opt(p: Option<P<T>>) -> u64 {
        p.map_or(0, P::pack)
    }

    /// `key` of an optional pointer, 0 for `None`.
    #[inline]
    pub fn key_opt(p: Option<P<T>>) -> PKey {
        p.map_or(0, P::key)
    }

    /// `to_bits` of an optional pointer, 0 for `None`.
    #[inline]
    pub fn to_bits_opt(p: Option<P<T>>) -> usize {
        p.map_or(0, P::to_bits)
    }
}

impl<T: ?Sized> Clone for P<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for P<T> {}

#[cfg(not(compressed_ptrs))]
impl<T: ?Sized> Deref for P<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        #[cfg(feature = "alloc-profile")]
        check_not_freed(self.0);
        self.0
    }
}

#[cfg(compressed_ptrs)]
impl<T> Deref for P<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.get()
    }
}

/// Profile builds with `TSRS_ARENA_POISON=1`: freed and rewound arena memory is filled with `POISON` and never reused,
/// so a `P` whose target starts with eight poison bytes points into memory the arena gave back. Checked on every
/// dereference, which turns a use after a wrong free into a panic at the use instead of a changed result.
#[cfg(feature = "alloc-profile")]
#[inline]
fn check_not_freed<T: ?Sized>(r: &T) {
    // A compressed handle always names an 8-aligned block (`p_layout`); a reference only when `T` is 8-aligned.
    let aligned = cfg!(compressed_ptrs) || std::mem::align_of_val(r) >= 8;
    if std::mem::size_of_val(r) >= 8 && aligned && crate::arena::poison_mode() {
        // SAFETY: `r` is at least 8 bytes and 8-aligned.
        let w = unsafe { std::ptr::read_volatile(r as *const T as *const u64) };
        assert!(
            w != u64::from_ne_bytes([crate::arena::POISON; 8]),
            "arena: dereference of a freed block at {:p} as {}",
            r as *const T as *const (),
            std::any::type_name::<T>()
        );
    }
}

#[cfg(not(compressed_ptrs))]
impl<T: ?Sized> PartialEq for P<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.addr() == other.addr()
    }
}
#[cfg(compressed_ptrs)]
impl<T: ?Sized> PartialEq for P<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T: ?Sized> Eq for P<T> {}

impl<T: ?Sized> Hash for P<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        #[cfg(not(compressed_ptrs))]
        state.write_usize(self.addr());
        #[cfg(compressed_ptrs)]
        state.write_u32(self.0.get());
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
        #[cfg(not(compressed_ptrs))]
        return self.addr().cmp(&other.addr());
        #[cfg(compressed_ptrs)]
        return self.0.cmp(&other.0);
    }
}

/// Prints the address (the handle in compressed mode) only: arena graphs are cyclic, so a structural Debug would
/// recurse forever.
impl<T: ?Sized> fmt::Debug for P<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(not(compressed_ptrs))]
        return write!(f, "P({:#x})", self.addr());
        #[cfg(compressed_ptrs)]
        return write!(f, "P(#{:#x})", self.0.get());
    }
}

// SAFETY: by decree, under the threading contract in the module docs: an arena value is mutated only by the thread
// that created it, and a file's AST and symbols are read-only once it is bound.
unsafe impl<T: ?Sized> Send for P<T> {}
// SAFETY: as for `Send`.
unsafe impl<T: ?Sized> Sync for P<T> {}

/// An element of an arena array whose elements are handed out as `P<V>` (link-store pages): 8-aligned in compressed
/// mode, where `P` targets must be; the same as `V` otherwise.
#[cfg_attr(compressed_ptrs, repr(C, align(8)))]
#[cfg_attr(not(compressed_ptrs), repr(transparent))]
#[derive(Default)]
pub struct PSlot<V>(pub V);

impl<V> PSlot<V> {
    /// The pointer to the element of an arena array.
    #[inline]
    #[cfg_attr(compressed_ptrs, track_caller)]
    pub fn as_p(&'static self) -> P<V> {
        // SAFETY: slots live only in arena arrays (`alloc_vec`), 8-aligned in compressed mode.
        unsafe { P::from_arena(&self.0) }
    }

    /// The first slot of an arena array of slots, for `P::array_add` indexing (`nth`).
    #[inline]
    pub fn first(slots: &'static [PSlot<V>]) -> P<PSlot<V>> {
        // SAFETY: as in `as_p`; the array is not empty.
        unsafe { P::from_arena(&slots[0]) }
    }

    /// The `i`-th value of the array whose first slot is `first`.
    ///
    /// # Safety
    /// `first` comes from `first` of an array with more than `i` slots.
    #[inline]
    pub unsafe fn nth(first: P<PSlot<V>>, i: usize) -> P<V> {
        // SAFETY: the array has more than `i` slots (this function's contract), and `PSlot` is `repr(C)` /
        // `repr(transparent)` with the value first, so a `V` lives at the slot's address.
        unsafe { first.array_add(i).cast::<V>() }
    }
}

/// A pointer to a `static` with `P`'s identity semantics (eq / hash / order by address): for Go package-level
/// `*T` variables such as the emit helpers, which a compressed `P` cannot point to.
#[repr(transparent)]
pub struct SP<T: 'static>(&'static T);

impl<T> SP<T> {
    #[inline]
    pub const fn from_static(r: &'static T) -> SP<T> {
        SP(r)
    }
    #[inline]
    pub fn get(self) -> &'static T {
        self.0
    }
    #[inline]
    pub fn addr(self) -> usize {
        std::ptr::from_ref::<T>(self.0) as usize
    }
}

impl<T> Clone for SP<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for SP<T> {}
impl<T> Deref for SP<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        self.0
    }
}
impl<T> PartialEq for SP<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}
impl<T> Eq for SP<T> {}
impl<T> Hash for SP<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.addr())
    }
}
impl<T> fmt::Debug for SP<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SP({:#x})", self.addr())
    }
}
// SAFETY: an `SP` is a `&'static T` with `P`'s identity semantics and falls under the same threading contract.
unsafe impl<T> Send for SP<T> {}
// SAFETY: as for `Send`.
unsafe impl<T> Sync for SP<T> {}

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
        crate::usebits::mark(self.a.as_ptr() as usize);
        // SAFETY: built from a `&'static [A]` of this length.
        unsafe { std::slice::from_raw_parts(self.a.as_ptr(), self.a_len as usize) }
    }
    #[inline]
    pub fn second(&self) -> &'static [B] {
        crate::usebits::mark(self.b.as_ptr() as usize);
        // SAFETY: built from a `&'static [B]` of this length.
        unsafe { std::slice::from_raw_parts(self.b.as_ptr(), self.b_len as usize) }
    }
}

// SAFETY: two `&'static` slices of arena or static data, under `P`'s threading contract.
unsafe impl<A, B> Send for SlicePair<A, B> {}
// SAFETY: as for `Send`; the pair is never written through.
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
        crate::usebits::mark(self.0.as_ptr() as usize);
        // SAFETY: the pointer is the data pointer of a `&'static [T]` of length `len` (this function's contract).
        unsafe { std::slice::from_raw_parts(self.0.as_ptr(), len) }
    }
}

// SAFETY: the data pointer of a `&'static [T]` of arena or static data, under `P`'s threading contract.
unsafe impl<T> Send for StaticSlicePtr<T> {}
// SAFETY: as for `Send`; the slice is never written through.
unsafe impl<T> Sync for StaticSlicePtr<T> {}

/// A `&'static str` in 8 bytes: the data pointer in the low 48 bits and the length in the high 16. A string of
/// `u16::MAX` bytes or more (or one whose address does not fit in 48 bits) is copied into the arena after a `u32`
/// length, and the length bits hold `u16::MAX`. `as_str` returns the same text (for short strings, the same slice).
#[derive(Clone, Copy)]
#[cfg(target_pointer_width = "64")]
pub struct PackedStr(std::ptr::NonNull<u8>);

/// 32-bit targets (wasm32): a fat `&'static str` is already 8 bytes, so `PackedStr` is the reference itself.
#[derive(Clone, Copy)]
#[cfg(target_pointer_width = "32")]
pub struct PackedStr(&'static str);

#[cfg(target_pointer_width = "32")]
impl PackedStr {
    #[inline]
    pub fn new(s: &'static str) -> PackedStr {
        PackedStr(s)
    }

    #[inline]
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

#[cfg(target_pointer_width = "64")]
const PACKED_STR_LEN_SHIFT: u32 = 48;
#[cfg(target_pointer_width = "64")]
const PACKED_STR_LONG: usize = u16::MAX as usize;

#[cfg(target_pointer_width = "64")]
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
        let p = with_arena(|a| a.alloc_layout(layout));
        profile!(PackedStr, layout.size(), p.addr().get());
        // SAFETY: `p` points to `4 + len` fresh bytes, 4-byte aligned.
        unsafe {
            p.cast::<u32>().as_ptr().write(len);
            std::ptr::copy_nonoverlapping(s.as_ptr(), p.as_ptr().add(4), s.len());
        }
        assert!(p.addr().get() >> PACKED_STR_LEN_SHIFT == 0, "arena address above 2^48");
        PackedStr(p.map_addr(|a| a | (PACKED_STR_LONG << PACKED_STR_LEN_SHIFT)))
    }

    #[inline]
    #[expect(
        clippy::disallowed_methods,
        reason = "from_utf8 here and in OwnedStrCell::get: +5% instructions, one checker (notes/lint-paydown-compiler.md)"
    )]
    pub fn as_str(self) -> &'static str {
        let len = self.0.addr().get() >> PACKED_STR_LEN_SHIFT;
        let p = self.0.as_ptr().map_addr(|a| a & ((1 << PACKED_STR_LEN_SHIFT) - 1));
        // SAFETY: built by `new` from a `&'static str` of this length, or by `new_long` (length prefix + bytes).
        unsafe {
            let (p, len) = if len == PACKED_STR_LONG { (p.add(4).cast_const(), p.cast::<u32>().read_unaligned() as usize) } else { (p.cast_const(), len) };
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, len))
        }
    }
}

// SAFETY: a `PackedStr` stands for a `&'static str` (immutable bytes), which is `Send`.
unsafe impl Send for PackedStr {}
// SAFETY: a `&'static str` is `Sync`.
unsafe impl Sync for PackedStr {}

impl fmt::Debug for PackedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}

/// A `&'static [T]` in one word: the data pointer in the low 48 bits and the length in the high 16 (like
/// `PackedStr`). A slice of 2^16 elements or more (or one whose address does not fit in 48 bits) is kept as a
/// pointer to an arena copy of the `&'static [T]` reference itself, tagged with bit 0 (`T` is at least 2-aligned, so
/// a data pointer never has it), so `get` returns exactly the slice that was packed (same pointer and length) in
/// either form. The word is never zero (a slice's data pointer, even an empty one's, is non-null), so
/// `Option<ThinSlice<T>>` is one word too.
#[cfg(target_pointer_width = "64")]
pub struct ThinSlice<T: 'static>(std::ptr::NonNull<()>, std::marker::PhantomData<&'static [T]>);

/// 32-bit targets (wasm32): the data pointer and the length, 8 bytes like a fat `&'static [T]` (and `Option` of it
/// uses the non-null niche). Every slice fits, so the only long form is `from_ref`'s: the address of the `&'static [T]`
/// it reads, tagged with bit 0 (`T` is at least 2-aligned, so a data pointer never has it).
#[cfg(target_pointer_width = "32")]
pub struct ThinSlice<T: 'static>(std::ptr::NonNull<()>, usize, std::marker::PhantomData<&'static [T]>);

#[cfg(target_pointer_width = "32")]
impl<T> ThinSlice<T> {
    const ALIGNED: () = assert!(std::mem::align_of::<T>() >= 2, "ThinSlice needs bit 0 of the data pointer");

    #[inline]
    pub fn new(s: &'static [T]) -> Self {
        let () = Self::ALIGNED;
        ThinSlice(std::ptr::NonNull::from(s).cast::<()>(), s.len(), std::marker::PhantomData)
    }

    /// A long-form slice that reads `*r` (as on 64-bit targets).
    pub fn from_ref(r: &'static &'static [T]) -> Self {
        let () = Self::ALIGNED;
        ThinSlice(std::ptr::NonNull::from(r).cast::<()>().map_addr(|a| a | THIN_LONG_TAG), 0, std::marker::PhantomData)
    }

    /// Whether this is the long form (`from_ref`).
    #[inline]
    pub fn is_long(self) -> bool {
        self.0.addr().get() & THIN_LONG_TAG != 0
    }

    /// The `&'static [T]` a long-form slice reads (`None` for the short form).
    #[inline]
    pub fn long_ref(self) -> Option<&'static &'static [T]> {
        // SAFETY: a long form was built by `from_ref` from a `&'static &'static [T]`.
        self.is_long().then(|| unsafe { &*(self.0.as_ptr().map_addr(|a| a & !THIN_LONG_TAG) as *const &'static [T]) })
    }

    #[inline]
    pub fn get(self) -> &'static [T] {
        if let Some(r) = self.long_ref() {
            return *r;
        }
        // SAFETY: built by `new` from a `&'static [T]` of this length.
        unsafe { std::slice::from_raw_parts(self.0.as_ptr() as *const T, self.1) }
    }
}

#[cfg(target_pointer_width = "64")]
const THIN_LEN_SHIFT: u32 = 48;
#[cfg(target_pointer_width = "64")]
const THIN_ADDR_MASK: usize = (1 << THIN_LEN_SHIFT) - 1;
const THIN_LONG_TAG: usize = 1;

impl<T> Clone for ThinSlice<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for ThinSlice<T> {}

#[cfg(target_pointer_width = "64")]
impl<T> ThinSlice<T> {
    const ALIGNED: () = assert!(std::mem::align_of::<T>() >= 2, "ThinSlice needs bit 0 of the data pointer");

    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn new(s: &'static [T]) -> Self {
        let () = Self::ALIGNED;
        let p = std::ptr::NonNull::from(s).cast::<()>();
        if (p.addr().get() >> THIN_LEN_SHIFT) | (s.len() >> (usize::BITS - THIN_LEN_SHIFT)) == 0 {
            return ThinSlice(p.map_addr(|a| a | (s.len() << THIN_LEN_SHIFT)), std::marker::PhantomData);
        }
        Self::new_long(s)
    }

    #[cold]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    fn new_long(s: &'static [T]) -> Self {
        let r = P::new(s);
        let p = std::ptr::NonNull::from(r.get()).cast::<()>();
        assert!(p.addr().get() >> THIN_LEN_SHIFT == 0, "arena address above 2^48");
        ThinSlice(p.map_addr(|a| a | THIN_LONG_TAG), std::marker::PhantomData)
    }

    /// A long-form slice that reads `*r` (a `&'static [T]` at a stable address that the owner may give a special
    /// meaning, such as `tsrs_ast`'s lazily parsed node lists, which check `is_long` before `get`).
    pub fn from_ref(r: &'static &'static [T]) -> Self {
        let p = std::ptr::NonNull::from(r).cast::<()>();
        assert!(p.addr().get() >> THIN_LEN_SHIFT == 0, "arena address above 2^48");
        ThinSlice(p.map_addr(|a| a | THIN_LONG_TAG), std::marker::PhantomData)
    }

    /// Whether this is the long form (`new_long`, `from_ref`).
    #[inline]
    pub fn is_long(self) -> bool {
        self.0.addr().get() & THIN_LONG_TAG != 0
    }

    /// The `&'static [T]` a long-form slice reads (`None` for the short form).
    #[inline]
    pub fn long_ref(self) -> Option<&'static &'static [T]> {
        // SAFETY: a long form was built by `new_long` or `from_ref` from a `&'static &'static [T]`.
        self.is_long().then(|| unsafe { &*(self.0.as_ptr().map_addr(|a| a & !THIN_LONG_TAG) as *const &'static [T]) })
    }

    /// The slice; records a use of the block that holds its elements (`usebits`).
    #[inline]
    pub fn get(self) -> &'static [T] {
        let s = self.peek();
        crate::usebits::mark_slice(s);
        s
    }

    /// `get` without recording a use (`usebits`).
    #[inline]
    pub fn peek(self) -> &'static [T] {
        let w = self.0.addr().get();
        if w & THIN_LONG_TAG != 0 {
            return self.get_long();
        }
        let p = self.0.as_ptr().map_addr(|a| a & THIN_ADDR_MASK);
        // SAFETY: built by `new` from a `&'static [T]` of this length.
        unsafe { std::slice::from_raw_parts(p as *const T, w >> THIN_LEN_SHIFT) }
    }

    #[cold]
    fn get_long(self) -> &'static [T] {
        // SAFETY: built by `new_long`: a tagged pointer to the `&'static [T]` itself.
        unsafe { *(self.0.as_ptr().map_addr(|a| a & !THIN_LONG_TAG) as *const &'static [T]) }
    }
}

// SAFETY: a `ThinSlice` stands for a `&'static [T]` of arena or static data, under `P`'s threading contract.
unsafe impl<T> Send for ThinSlice<T> {}
// SAFETY: as for `Send`; the slice is never written through.
unsafe impl<T> Sync for ThinSlice<T> {}

impl<T: fmt::Debug> fmt::Debug for ThinSlice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

/// A `Cell<&'static [T]>` in one word (a `ThinSlice`). `get` returns exactly the slice last `set` (same pointer and
/// length).
pub struct ThinSliceCell<T: 'static>(std::cell::Cell<ThinSlice<T>>);

impl<T> ThinSliceCell<T> {
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn new(s: &'static [T]) -> Self {
        ThinSliceCell(std::cell::Cell::new(ThinSlice::new(s)))
    }
    #[inline]
    pub fn get(&self) -> &'static [T] {
        crate::usebits::mark_ptr(self);
        self.0.get().get()
    }
    /// `get` without recording a use (`usebits`).
    #[inline]
    pub fn peek(&self) -> &'static [T] {
        self.0.get().peek()
    }
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn set(&self, s: &'static [T]) {
        crate::usebits::mark_write(self as *const Self as usize);
        self.0.set(ThinSlice::new(s))
    }
}

impl<T> Default for ThinSliceCell<T> {
    fn default() -> Self {
        ThinSliceCell::new(&[])
    }
}

/// A `Cell<Option<&'static [T]>>` in one word, like `ThinSliceCell` (for Go slices whose nil differs from empty).
pub struct OptionThinSliceCell<T: 'static>(std::cell::Cell<Option<ThinSlice<T>>>);

impl<T> OptionThinSliceCell<T> {
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn new(s: Option<&'static [T]>) -> Self {
        OptionThinSliceCell(std::cell::Cell::new(s.map(ThinSlice::new)))
    }
    #[inline]
    pub fn get(&self) -> Option<&'static [T]> {
        crate::usebits::mark_ptr(self);
        self.0.get().map(ThinSlice::get)
    }
    /// `get` without recording a use (`usebits`).
    #[inline]
    pub fn peek(&self) -> Option<&'static [T]> {
        self.0.get().map(ThinSlice::peek)
    }
    #[inline]
    #[cfg_attr(feature = "alloc-profile", track_caller)]
    pub fn set(&self, s: Option<&'static [T]>) {
        crate::usebits::mark_write(self as *const Self as usize);
        self.0.set(s.map(ThinSlice::new))
    }
}

impl<T> Default for OptionThinSliceCell<T> {
    fn default() -> Self {
        OptionThinSliceCell::new(None)
    }
}

const _: () = assert!(std::mem::size_of::<ThinSliceCell<u16>>() == 8 && std::mem::size_of::<OptionThinSliceCell<u16>>() == 8);

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
        crate::usebits::mark_ptr(self);
        let s = self.peek();
        crate::usebits::mark_slice(s);
        s
    }
    /// `get` without recording a use (`usebits`).
    #[inline]
    pub fn peek(&self) -> &'static [T] {
        let p = self.0.get();
        // SAFETY: built from a `&'static [T]` of this length.
        unsafe { std::slice::from_raw_parts(p.ptr.as_ptr(), p.len as usize) }
    }
    #[inline]
    pub fn set(&self, s: &'static [T]) {
        crate::usebits::mark_write(self as *const Self as usize);
        self.0.set(Self::pack(s))
    }
}

impl<T> Default for SliceCell<T> {
    fn default() -> Self {
        SliceCell::new(&[])
    }
}

#[repr(C, packed(4))]
struct PackedOptionSlice<T: 'static> {
    ptr: *const T, // null = None (a slice's data pointer, even an empty one's, is never null)
    len: u32,
}

impl<T> Clone for PackedOptionSlice<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for PackedOptionSlice<T> {}

/// A `Cell<Option<&'static [T]>>` in 12 bytes with 4-byte alignment, like `SliceCell` (for Go slices whose nil
/// differs from empty).
pub struct OptionSliceCell<T: 'static>(std::cell::Cell<PackedOptionSlice<T>>);

impl<T> OptionSliceCell<T> {
    #[inline]
    pub fn new(s: Option<&'static [T]>) -> Self {
        OptionSliceCell(std::cell::Cell::new(Self::pack(s)))
    }
    #[inline]
    fn pack(s: Option<&'static [T]>) -> PackedOptionSlice<T> {
        match s {
            Some(s) => PackedOptionSlice { ptr: s.as_ptr(), len: u32::try_from(s.len()).expect("slice longer than u32::MAX") },
            None => PackedOptionSlice { ptr: std::ptr::null(), len: 0 },
        }
    }
    #[inline]
    pub fn get(&self) -> Option<&'static [T]> {
        crate::usebits::mark_ptr(self);
        let s = self.peek();
        if let Some(s) = s {
            crate::usebits::mark_slice(s);
        }
        s
    }
    /// `get` without recording a use (`usebits`).
    #[inline]
    pub fn peek(&self) -> Option<&'static [T]> {
        let p = self.0.get();
        // SAFETY: a non-null pointer was built from a `&'static [T]` of this length.
        (!p.ptr.is_null()).then(|| unsafe { std::slice::from_raw_parts(p.ptr, p.len as usize) })
    }
    #[inline]
    pub fn set(&self, s: Option<&'static [T]>) {
        crate::usebits::mark_write(self as *const Self as usize);
        self.0.set(Self::pack(s))
    }
}

impl<T> Default for OptionSliceCell<T> {
    fn default() -> Self {
        OptionSliceCell::new(None)
    }
}


/// `Cell<&'static [T]>` for structs of handles: a 12-byte 4-aligned `SliceCell` packs with the 4-byte fields around
/// it next to 8-byte pointers, an 8-byte `ThinSliceCell` packs tighter next to 4-byte handles (compressed pointers).
#[cfg(not(compressed_ptrs))]
pub type PSliceCell<T> = SliceCell<T>;
#[cfg(compressed_ptrs)]
pub type PSliceCell<T> = ThinSliceCell<T>;

/// A `Cell<&'static str>` in 8 bytes (a `PackedStr`).
pub struct StrCell(std::cell::Cell<PackedStr>);

impl StrCell {
    #[inline]
    pub fn new(s: &'static str) -> Self {
        StrCell(std::cell::Cell::new(PackedStr::new(s)))
    }
    #[inline]
    pub fn get(&self) -> &'static str {
        crate::usebits::mark_ptr(self);
        let s = self.0.get().as_str();
        crate::usebits::mark(s.as_ptr() as usize);
        s
    }
    #[inline]
    pub fn set(&self, s: &'static str) {
        self.0.set(PackedStr::new(s))
    }
}

impl Default for StrCell {
    fn default() -> Self {
        StrCell::new("")
    }
}

/// Copies a slice into the arena. Use for Go slices that are stored in long-lived objects.
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_slice<T: Copy>(items: &[T]) -> &'static [T] {
    if items.is_empty() {
        return &[];
    }
    let s: &'static [T] = with_arena(|a| &*a.alloc_slice_copy(items));
    profile!([T], std::mem::size_of_val(items), s.as_ptr() as usize);
    s
}

/// `alloc_slice` in the thread's scratch region if one is entered (see `P::new_scratch`), else like `alloc_slice`.
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_slice_scratch<T: Copy>(items: &[T]) -> &'static [T] {
    if items.is_empty() {
        return &[];
    }
    let s: &'static [T] = with_scratch_arena(|a| &*a.alloc_slice_copy(items));
    profile!([T], std::mem::size_of_val(items), s.as_ptr() as usize);
    s
}

/// `alloc_vec` in the thread's scratch region if one is entered (see `P::new_scratch`), else like `alloc_vec`.
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_vec_scratch<T>(items: Vec<T>) -> &'static [T] {
    if items.is_empty() {
        return &[];
    }
    #[cfg(feature = "alloc-profile")] // read only by profile!, before `items` is moved into the arena
    let bytes = std::mem::size_of_val(&items[..]);
    let s: &'static [T] = with_scratch_arena(|a| {
        let s = a.alloc_vec(items);
        a.track_drop(s.as_mut_ptr(), s.len());
        &*s
    });
    profile!([T], bytes, s.as_ptr() as usize);
    s
}

/// `alloc_slice` of bytes, 4-aligned (for a length prefix read as `u32`).
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_slice_aligned4(items: &[u8]) -> &'static [u8] {
    let layout = std::alloc::Layout::from_size_align(items.len().max(1), 4).expect("arena slice layout");
    let p = with_arena(|a| a.alloc_layout(layout));
    profile!([u8], layout.size(), p.addr().get());
    // SAFETY: fresh memory for `items.len()` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(items.as_ptr(), p.as_ptr(), items.len());
        std::slice::from_raw_parts(p.as_ptr(), items.len())
    }
}

/// Moves a `Vec` of arbitrary (possibly non-`Copy`) items into the arena.
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_vec<T>(items: Vec<T>) -> &'static [T] {
    if items.is_empty() {
        return &[];
    }
    #[cfg(feature = "alloc-profile")] // read only by profile!, before `items` is moved into the arena
    let bytes = std::mem::size_of_val(&items[..]);
    let s: &'static [T] = with_arena(|a| {
        let s = a.alloc_vec(items);
        a.track_drop(s.as_mut_ptr(), s.len());
        &*s
    });
    profile!([T], bytes, s.as_ptr() as usize);
    s
}

/// `alloc_slice` that first takes a block of the right size from the current thread's free list (see `free`).
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_slice_recycled<T: Copy>(items: &[T]) -> &'static [T] {
    if items.is_empty() {
        return &[];
    }
    let class = arena::free_class(std::mem::size_of_val(items), std::mem::align_of::<T>());
    if class != 0 {
        if let Some(block) = with_arena(|a| a.pop_free(class)) {
            let p = block.cast::<T>();
            // SAFETY: a dead block of exactly `size_of_val(items)` bytes, 8-aligned; `T: Copy`.
            let s: &'static [T] = unsafe {
                std::ptr::copy_nonoverlapping(items.as_ptr(), p.as_ptr(), items.len());
                std::slice::from_raw_parts(p.as_ptr(), items.len())
            };
            profile!([T], std::mem::size_of_val(items), s.as_ptr() as usize);
            return s;
        }
    }
    alloc_slice(items)
}

/// Gives a dead arena block back to the current thread's free list for its size (8-byte multiples up to
/// `arena::MAX_FREE_SIZE`, alignment <= 8; other sizes are ignored). Use the `free!` / `free_slice!` macros, which
/// pass the block as an address: a `P<T>` / `&T` argument would be a live, protected reference to the memory while
/// it is overwritten. Destructors do not run. The caller has proved that nothing reachable or live points to the
/// block: the next `new_recycled` / `alloc_slice_recycled` of that size reuses it. With `TSRS_ARENA_POISON=1` the
/// block is filled with `arena::POISON` instead, and in the census build (`TSRS_CENSUS=1`) it is recorded, never
/// reused, and the census checks at exit that it is unreachable.
///
/// A function that frees a block must not have received it as a reference parameter (`&T`, `&[T]`): a reference
/// parameter is a protected borrow for the whole call (LLVM `noalias readonly`), so writing the free-list link into
/// the block is undefined behaviour, and LLVM may delete that write. It did, in `recycle_mapper_with_targets`,
/// once `panic = "abort"` made the calls around it `nounwind`: the block stayed on the free list with its old
/// contents as the link, and the next allocation of that size crashed (notes/fix-arena-recycle-uaf.md). Pass the
/// block as a `P<T>`, as an address, or as a raw slice (`free_slice_ptr`).
///
/// # Safety
/// No live object, local or cache may point into the block afterwards, no function on the stack may hold it as a
/// reference parameter, and it must be an arena block of exactly `size` bytes.
#[inline]
pub unsafe fn free_raw(addr: usize, size: usize, align: usize, needs_drop: bool) {
    // SAFETY: nothing points into the block afterwards (this function's contract).
    with_arena(|a| unsafe { arena::free_block(a, addr, size, align, needs_drop) });
}

/// `free_slice!` for a slice passed as a raw pointer: the form a function takes when it frees a slice its caller
/// made (see `free_raw`: a `&[T]` parameter would be a protected borrow of the memory being freed). Empty slices are
/// ignored.
///
/// # Safety
/// As for `free_raw`: `s` is a whole arena slice that nothing uses afterwards.
#[inline]
pub unsafe fn free_slice_ptr<T>(s: *const [T]) {
    if s.len() != 0 {
        // SAFETY: the caller's contract (`free_raw`).
        unsafe { free_raw(s.cast::<T>() as usize, s.len() * std::mem::size_of::<T>(), std::mem::align_of::<T>(), std::mem::needs_drop::<T>()) };
    }
}

#[doc(hidden)]
#[inline]
pub fn layout_of_pointee<T: ?Sized>(p: &T) -> (usize, usize, bool) {
    (std::mem::size_of_val(p), std::mem::align_of_val(p), std::mem::needs_drop::<T>())
}

#[doc(hidden)]
#[inline]
pub fn layout_of_p<T>(_: P<T>) -> (usize, usize, bool) {
    (p_layout::<T>().size(), p_layout::<T>().align(), std::mem::needs_drop::<T>())
}

/// `free!(p)`: gives the arena object `p: P<T>` back (`free_raw`). Unsafe: see `free_raw`.
#[macro_export]
macro_rules! free {
    ($p:expr_2021) => {{
        let p = $p;
        let (size, align, needs_drop) = $crate::ptr::layout_of_p(p);
        $crate::ptr::free_raw(p.addr(), size, align, needs_drop)
    }};
}

/// `free_slice!(s)`: gives the arena slice `s: &'static [T]` back (empty slices are ignored; it must not be a static
/// or a sub-slice). Unsafe: see `free_raw`.
#[macro_export]
macro_rules! free_slice {
    ($s:expr_2021) => {{
        let s = $s;
        if !s.is_empty() {
            let (size, align, needs_drop) = $crate::ptr::layout_of_pointee(s);
            $crate::ptr::free_raw(s.as_ptr() as usize, size, align, needs_drop)
        }
    }};
}

/// One field of an arena type that the census's strong mark (alloc-profile builds, `TSRS_CENSUS=1`, the would-free
/// check) would misread as a plain 48-bit pointer, or miss. Offsets are from the start of the arena block. The crate
/// that owns the type registers them with `census_layout`, computed with `offset_of!`, so they follow layout changes.
#[derive(Clone, Copy, Debug)]
pub enum CensusField {
    /// `len` bytes at `off` never hold a pointer: scalar fields, padding, header words, enum payload that some variants
    /// leave uninitialized (copied from the stack with stale words). No scan step that overlaps them is a reference.
    NoPointer { off: usize, len: usize },
    /// The word at `off` keeps an address in its low 48 bits and flag bits above them.
    Tagged { off: usize },
    /// The word at `off` keeps an address / 8 in its low 45 bits when bit `word >> 62` is set in `modes`, else none.
    X8 { off: usize, modes: u8 },
    /// A slice's pointer word at `ptr`: a reference only while the length word at `len` is non-zero (an empty slice's
    /// pointer is dangling and may equal the address of the next block).
    Slice { ptr: usize, len: usize },
    /// A `ThinSlice` word (also `ThinSliceCell` / `OptionThinSliceCell`): the list's address with its length in the
    /// top 16 bits (no reference while the length is 0: an empty slice's pointer may be dangling), or with bit 0 set
    /// the address of the out-of-line `&[T]` record of a long list.
    Thin { off: usize },
    /// The word at `off` keeps an address with tag bits `mask` (low bits of an aligned address) set.
    LowTag { off: usize, mask: u8 },
}

impl CensusField {
    /// `NoPointer` for the scalar field at `off` and the padding after it, up to the next field (`offsets`: every field
    /// offset of the struct) or the struct's end (`size`); `base` is the struct's offset in the arena block.
    pub fn scalar(base: usize, off: usize, offsets: &[usize], size: usize) -> CensusField {
        let end = offsets.iter().copied().filter(|&o| o > off).min().unwrap_or(size);
        CensusField::NoPointer { off: base + off, len: end - off }
    }

    /// `NoPointer` for every byte of `[0, size)` outside the 8-byte words at `words` (a header whose other words hold
    /// flags and ids); `base` as in `scalar`.
    pub fn all_but(base: usize, size: usize, words: &[usize]) -> Vec<CensusField> {
        let mut words = words.to_vec();
        words.sort_unstable();
        let mut out = Vec::new();
        let mut at = 0;
        for w in words.into_iter().chain([size]) {
            if w > at {
                out.push(CensusField::NoPointer { off: base + at, len: w - at });
            }
            at = w + 8;
        }
        out
    }

    /// The (pointer, length) word offsets of `Option<&'static [T]>` (a slice reference with the `None` niche).
    pub fn slice_words() -> (usize, usize) {
        static ONE: [u64; 1] = [0];
        let s: Option<&'static [u64]> = Some(&ONE[..]);
        // SAFETY: `Option<&[u64]>` is two words: the pointer (non-null, the niche) and the length.
        let w: [usize; 2] = unsafe { std::mem::transmute(s) };
        if w[0] == ONE.as_ptr() as usize {
            (0, 8)
        } else {
            (8, 0)
        }
    }
}

/// Whether the census records (alloc-profile build with `TSRS_CENSUS=1`); a constant `false` in other builds, so
/// census-only work behind it compiles to nothing.
#[inline]
pub fn census_recording() -> bool {
    #[cfg(feature = "alloc-profile")]
    return crate::alloc_profile::census::recording();
    #[cfg(not(feature = "alloc-profile"))]
    false
}

/// Census builds: registers the fields of the arena type named `type_name` (`std::any::type_name` of the allocated
/// value, or a prefix ending in `<` for every instance of a generic, such as `TypeAlloc<`) that the strong mark must
/// read specially. Call before the census runs. Compiled to nothing otherwise.
#[inline]
pub fn census_layout(type_name: &'static str, fields: &[CensusField]) {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::census::register_layout(type_name, fields);
    #[cfg(not(feature = "alloc-profile"))]
    let _ = (type_name, fields);
}

/// Census builds (`TSRS_CENSUS=1`): zeroes the unused capacity of a pooled vector after elements were removed, so
/// stale pointers there do not count as references in the would-free check. Compiled to nothing otherwise.
#[inline]
pub fn census_scrub_slack<T>(v: &mut Vec<T>) {
    #[cfg(feature = "alloc-profile")]
    if crate::alloc_profile::census::recording() {
        for slot in v.spare_capacity_mut() {
            *slot = std::mem::MaybeUninit::zeroed();
        }
    }
    #[cfg(not(feature = "alloc-profile"))]
    let _ = v;
}

/// Census builds (`TSRS_CENSUS=1`): replaces a cleared pooled table with a new one. Clearing a hash table resets only
/// its control bytes, so the removed entries stay in its buckets, and an entry type with uninitialized bytes (an enum
/// whose variants differ in size) keeps stale words there that the census reads as references. Compiled to nothing
/// otherwise.
#[inline]
pub fn census_reset<T: Default>(t: &mut T) {
    #[cfg(feature = "alloc-profile")]
    if crate::alloc_profile::census::recording() {
        *t = T::default();
    }
    #[cfg(not(feature = "alloc-profile"))]
    let _ = t;
}

/// Census builds (`TSRS_CENSUS=1`): a `None` option keeps uninitialized payload bytes (copied from the stack slot or
/// heap block the value was built in), which the conservative census reads as references. Zeroes them, for options
/// kept for the rest of a session (process-wide caches, long-lived tables). Compiled to nothing otherwise.
#[inline]
pub fn census_scrub_none<T>(opt: &mut Option<T>) {
    #[cfg(feature = "alloc-profile")]
    if crate::alloc_profile::census::recording() && opt.is_none() {
        let p: *mut Option<T> = opt;
        // SAFETY: `*p` is `None` and owns nothing; it is overwritten with zeros and then with `None` again before
        // anything reads it, so it holds a valid `None` afterwards whatever the type's niche encoding is.
        unsafe {
            std::ptr::write_bytes(p.cast::<u8>(), 0, std::mem::size_of::<Option<T>>());
            p.write(None);
        }
    }
    #[cfg(not(feature = "alloc-profile"))]
    let _ = opt;
}

/// Census builds (`TSRS_CENSUS=1`): clears the stack area the caller's next callees will use, so values built there
/// (a struct with an unset `OnceLock`, padding) do not carry stale words that look like references. Compiled to
/// nothing otherwise.
#[inline]
pub fn census_scrub_stack() {
    #[cfg(feature = "alloc-profile")]
    if crate::alloc_profile::census::recording() {
        crate::arena::scrub_stack_for_census();
    }
}

/// The current thread's arena position, for discarding a speculative parse (`arena_rewind`).
#[inline]
pub fn arena_checkpoint() -> Checkpoint {
    with_arena(|a| a.checkpoint())
}

/// Discards everything the current thread allocated in the arena since `cp`, if nothing can point to it: the
/// caller guarantees that its own data does not, and the arena skips the rewind when a chunk was added or a free /
/// `arena_pin` happened since `cp` (see `arena`). Checkpoints nest: rewind in reverse order.
#[inline]
pub fn arena_rewind(cp: Checkpoint) {
    with_arena(|a| arena::rewind(a, cp));
}

/// Whether `arena_rewind(cp)` would discard what was allocated since `cp` (same chunk, no free or pin since).
#[inline]
pub fn arena_rewindable(cp: &Checkpoint) -> bool {
    with_arena(|a| a.rewindable_now(cp))
}

/// Declares that data allocated since the innermost open checkpoint may now be referenced from a structure that
/// outlives it (a cache, a list that is not rolled back), so no open checkpoint may be rewound.
#[inline]
pub fn arena_pin() {
    with_arena(|a| a.bump_epoch());
}

/// Copies a string into the arena.
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_str(s: &str) -> &'static str {
    if s.is_empty() {
        return "";
    }
    let r: &'static str = with_arena(|a| &*a.alloc_str(s));
    profile!(str, s.len(), r.as_ptr() as usize);
    r
}

/// `alloc_str` in the thread's scratch region if one is entered (see `P::new_scratch`), else like `alloc_str`.
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc_str_scratch(s: &str) -> &'static str {
    if s.is_empty() {
        return "";
    }
    let r: &'static str = with_scratch_arena(|a| &*a.alloc_str(s));
    profile!(str, s.len(), r.as_ptr() as usize);
    r
}

/// Allocates a plain `&'static T` (for values that do not need pointer identity semantics).
#[inline]
#[cfg_attr(feature = "alloc-profile", track_caller)]
pub fn alloc<T>(value: T) -> &'static T {
    let r: &'static T = with_arena(|a| {
        let r = a.alloc_with(p_layout::<T>(), value);
        a.track_drop(std::ptr::from_mut::<T>(r), 1);
        &*r
    });
    profile!(T, p_layout::<T>().size(), r as *const T as usize);
    r
}

/// Prints the allocation profile (no-op unless built with `--features tsrs_core/alloc-profile`).
pub fn alloc_profile_dump() {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::dump();
}

/// Bytes allocated so far by the current thread's arena.
pub fn arena_allocated_bytes() -> usize {
    with_arena(|a| a.capacity())
}

/// Bytes in use in the current allocation target (the thread's arena or an entered region), for measurements.
pub fn arena_used_bytes() -> usize {
    with_arena(|a| a.used_ranges().iter().map(|&(_, len)| len).sum())
}

/// Compressed pointers: (bytes of the reserved range handed out as chunks now, how far into it chunks were ever
/// carved). `None` with plain pointers.
pub fn reserve_stats() -> Option<(usize, usize)> {
    #[cfg(compressed_ptrs)]
    return Some((crate::reserve::reserved_in_use(), crate::reserve::reserved_high_water()));
    #[cfg(not(compressed_ptrs))]
    None
}

/// Debug aid for the threading contract (checked builds only, opt-in with `TSRS_CHECK_SHARED=1`):
/// `freeze_shared_objects` records every arena allocation made so far (the parsed and bound program) as
/// shared, and `assert_not_shared` panics when a checker writes to such an object. Release builds without
/// `checked-cells` compile both to nothing.
pub mod shared_check {
    use crate::arena::Arena;
    use std::sync::{Mutex, OnceLock, RwLock};

    struct ArenaRef(&'static Arena);
    // SAFETY: only used to read chunk bounds while the owning threads are idle.
    unsafe impl Send for ArenaRef {}

    static ARENAS: Mutex<Vec<ArenaRef>> = Mutex::new(Vec::new());
    static FROZEN: RwLock<Vec<(usize, usize)>> = RwLock::new(Vec::new());

    #[cfg(any(debug_assertions, feature = "checked-cells"))]
    pub(super) fn register(arena: &'static Arena) {
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
            // The owning threads are idle; the chunk list is only read.
            for (ptr, len) in arena.0.used_ranges() {
                ranges.push((ptr, ptr + len));
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
        let addr = std::ptr::from_ref::<T>(object).cast::<()>() as usize;
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
        assert_eq!(std::mem::size_of::<Option<P<Thing>>>(), if crate::COMPRESSED_PTRS { 4 } else { std::mem::size_of::<usize>() });
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
    fn option_slice_cell() {
        let c: OptionSliceCell<i32> = OptionSliceCell::default();
        assert_eq!(c.get(), None);
        c.set(Some(&[]));
        assert_eq!(c.get(), Some(&[][..]));
        c.set(Some(alloc_slice(&[1, 2])));
        assert_eq!(c.get(), Some(&[1, 2][..]));
    }

    #[test]
    fn thin_slices_keep_identity() {
        for len in [0usize, 1, 3, u16::MAX as usize - 1, u16::MAX as usize, u16::MAX as usize + 1, 200_000] {
            let s: &'static [u32] = Box::leak((0..len as u32).collect::<Vec<_>>().into_boxed_slice());
            let c = SliceCell::new(s);
            assert!(std::ptr::eq(c.get(), s), "len {len}");
            let o: OptionSliceCell<u32> = OptionSliceCell::default();
            o.set(Some(s));
            assert!(std::ptr::eq(o.get().unwrap(), s), "len {len}");
            o.set(None);
            assert_eq!(o.get(), None);
        }
        let empty: &'static [u64] = &[];
        assert!(std::ptr::eq(SliceCell::new(empty).get(), empty));
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
