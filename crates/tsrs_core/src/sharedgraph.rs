//! tsrs-only research prototype (`spike/shared-graph`, `TSRS_SHARED_GRAPH=1`): a frozen seed graph shared by every
//! checker thread.
//!
//! One seed checker runs on a fresh thread arena; when it is done its arena's chunks are *frozen*: marked in a page
//! bitmap (`is_frozen_addr`) and, with `TSRS_SHARED_GRAPH_PROTECT=1` (the default while the prototype is developed),
//! `mprotect`ed read-only, so a missed write is a fault with a backtrace instead of a silent race. Every pool checker
//! is a fork that reads the frozen objects and keeps what it would have written into them in its own `Overlay`, keyed
//! by the address of the cell. `OvCell` is a `Cell` whose writes to a frozen address go to the overlay of the
//! checker the current thread runs (`enter_overlay`).
//!
//! Nothing here is used unless the switch is on: `ANY_FROZEN` stays false and every `OvCell` behaves as a `Cell`.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use rustc_hash::FxHashMap;

/// Whether the prototype is compiled in (`--features shared-graph`). Without it every frozen-object test below is a
/// constant false, so the switch-off build runs main's code paths.
pub const COMPILED_IN: bool = cfg!(feature = "shared-graph");

/// Set once, by `freeze`, before any fork exists (the forks are created on threads spawned after it).
static ANY_FROZEN: AtomicBool = AtomicBool::new(false);

const PAGE_SHIFT: usize = 12;
const SPAN: usize = 32 << 30;
const WORDS: usize = SPAN >> PAGE_SHIFT >> 6;
/// One bit per 4 KiB page of the reservation (1 MiB, zero until a freeze, in .bss).
static FROZEN: [AtomicU64; WORDS] = [const { AtomicU64::new(0) }; WORDS];

/// Overrides: a fork set a frozen cell that already held a non-default value (counted; the inline value still wins).
pub static OVERRIDES: AtomicUsize = AtomicUsize::new(0);

/// The frozen chunks lie in `[FROZEN_LO, FROZEN_LO + FROZEN_SPAN)` (0 and 0 while nothing is frozen).
static FROZEN_LO: AtomicUsize = AtomicUsize::new(0);
static FROZEN_SPAN: AtomicUsize = AtomicUsize::new(0);
/// One bit per 64-byte line of the frozen range: some fork wrote a cell in that line into its overlay. A read of a
/// frozen cell whose line is clean returns the inline value without touching the overlay (no thread-local, no hash).
/// Bits are only ever set (a fork whose overlay lacks the cell falls back to the inline value).
static DIRTY: std::sync::atomic::AtomicPtr<AtomicU64> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
/// Lines marked dirty (stats).
pub static DIRTY_LINES: AtomicUsize = AtomicUsize::new(0);

/// Whether the cell at `addr` is frozen and its line is dirty: the only case a read must consult the overlay.
#[inline(always)]
pub fn dirty(addr: usize) -> bool {
    if !COMPILED_IN {
        return false;
    }
    // Relaxed: the range is written before the forks' threads were spawned.
    let off = addr.wrapping_sub(FROZEN_LO.load(Ordering::Relaxed));
    if off >= FROZEN_SPAN.load(Ordering::Relaxed) {
        return false;
    }
    let line = off >> 6;
    // SAFETY: DIRTY covers the whole span (allocated by `freeze` before FROZEN_SPAN became non-zero).
    let w = unsafe { &*DIRTY.load(Ordering::Relaxed).add(line >> 6) };
    // Relaxed: a stale clear bit only means this thread never wrote the line (its own writes are program-ordered).
    w.load(Ordering::Relaxed) & (1 << (line & 63)) != 0
}

#[inline]
fn mark_dirty(addr: usize) {
    let line = (addr - FROZEN_LO.load(Ordering::Relaxed)) >> 6;
    // SAFETY: as in `dirty`; the caller checked that `addr` is frozen.
    let w = unsafe { &*DIRTY.load(Ordering::Relaxed).add(line >> 6) };
    let bit = 1 << (line & 63);
    if w.load(Ordering::Relaxed) & bit == 0 && w.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        DIRTY_LINES.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline(always)]
pub fn any_frozen() -> bool {
    // Relaxed: written before the threads that read it were spawned.
    COMPILED_IN && ANY_FROZEN.load(Ordering::Relaxed)
}

#[inline]
pub fn is_frozen_addr(addr: usize) -> bool {
    if !any_frozen() {
        return false;
    }
    is_frozen_addr_slow(addr)
}

#[inline]
fn is_frozen_addr_slow(addr: usize) -> bool {
    if !COMPILED_IN {
        return false;
    }
    // Relaxed: as in `dirty`.
    if addr.wrapping_sub(FROZEN_LO.load(Ordering::Relaxed)) >= FROZEN_SPAN.load(Ordering::Relaxed) {
        return false;
    }
    #[cfg(compressed_ptrs)]
    {
        let off = addr.wrapping_sub(crate::reserve::BASE_ADDR);
        if off >= SPAN {
            return false;
        }
        let page = off >> PAGE_SHIFT;
        // Relaxed: written before the reading threads were spawned.
        FROZEN[page >> 6].load(Ordering::Relaxed) & (1 << (page & 63)) != 0
    }
    #[cfg(not(compressed_ptrs))]
    {
        let _ = addr;
        false
    }
}

/// Whether frozen chunks are `mprotect`ed (`TSRS_SHARED_GRAPH_PROTECT`, default on; `0` turns it off, `log` makes a
/// write fault unprotect the page, print the site once per page, and continue).
pub fn protect_mode() -> u8 {
    static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("TSRS_SHARED_GRAPH_PROTECT").as_deref() {
        Ok("0" | "off") => 0,
        Ok("log") => 2,
        _ => 1,
    })
}

/// Marks `ranges` (start, len; page-aligned chunks of the seed's arena) frozen, and protects them per `protect_mode`.
pub fn freeze(ranges: &[(usize, usize)]) {
    #[cfg(compressed_ptrs)]
    for &(start, len) in ranges {
        let off = start - crate::reserve::BASE_ADDR;
        assert!(off + len <= SPAN, "shared graph: frozen chunk outside the reservation");
        // Only whole 4 KiB pages inside the chunk (region chunks are 16-byte aligned).
        for page in off.div_ceil(1 << PAGE_SHIFT)..((off + len) >> PAGE_SHIFT) {
            // Relaxed: published to the forks by spawning their threads.
            FROZEN[page >> 6].fetch_or(1 << (page & 63), Ordering::Relaxed);
        }
    }
    let lo = ranges.iter().map(|r| r.0).min().unwrap_or(0);
    let hi = ranges.iter().map(|r| r.0 + r.1).max().unwrap_or(0);
    let words = ((hi - lo) >> 6).div_ceil(64) + 1;
    let dirty: &'static mut [AtomicU64] = Box::leak((0..words).map(|_| AtomicU64::new(0)).collect());
    DIRTY.store(dirty.as_mut_ptr(), Ordering::Relaxed);
    FROZEN_LO.store(lo, Ordering::Relaxed);
    FROZEN_SPAN.store(hi - lo, Ordering::Relaxed);
    ANY_FROZEN.store(true, Ordering::Release);
    #[cfg(unix)]
    if protect_mode() != 0 {
        install_fault_handler();
        let page = crate::reserve::page_size();
        for &(start, len) in ranges {
            // Only whole system pages inside the chunk: a 16 KiB page may also hold the neighbouring chunk.
            let (lo, hi) = (start.next_multiple_of(page), (start + len) & !(page - 1));
            if hi <= lo {
                continue;
            }
            // SAFETY: whole pages of a chunk of the reservation, mapped read-write; they become read-only.
            let r = unsafe { libc::mprotect(std::ptr::with_exposed_provenance_mut::<libc::c_void>(lo), hi - lo, libc::PROT_READ) };
            assert!(r == 0, "shared graph: mprotect failed");
        }
    }
}

/// `TSRS_SHARED_GRAPH_LOG_OVERRIDES=1`: prints the stack of every override (`faults.py` groups them).
#[cold]
fn log_site(msg: &[u8]) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("TSRS_SHARED_GRAPH_LOG_OVERRIDES").is_ok_and(|v| v == "1")) {
        return;
    }
    #[cfg(unix)]
    {
        extern "C" {
            fn backtrace(buf: *mut *mut libc::c_void, size: i32) -> i32;
            fn backtrace_symbols_fd(buf: *const *mut libc::c_void, size: i32, fd: i32);
        }
        let mut frames = [std::ptr::null_mut::<libc::c_void>(); 40];
        // SAFETY: writes a buffer to stderr; fills and prints a local frame buffer.
        unsafe {
            libc::write(2, msg.as_ptr().cast(), msg.len());
            let n = backtrace(frames.as_mut_ptr(), 40);
            backtrace_symbols_fd(frames.as_ptr(), n, 2);
        }
    }
}

#[cfg(unix)]
fn install_fault_handler() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        for sig in [libc::SIGSEGV, libc::SIGBUS] {
            // SAFETY: installs a handler with a valid sigaction; the handler only reads `si_addr`.
            unsafe {
                let mut sa: libc::sigaction = std::mem::zeroed();
                sa.sa_sigaction = fault_handler as *const () as usize;
                sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&raw mut sa.sa_mask);
                libc::sigaction(sig, &raw const sa, std::ptr::null_mut());
            }
        }
    });
}

#[cfg(unix)]
extern "C" fn fault_handler(sig: i32, info: *mut libc::siginfo_t, _ctx: *mut libc::c_void) {
    // SAFETY: the kernel passes a valid siginfo for SA_SIGINFO handlers.
    let addr = unsafe { (*info).si_addr() }.addr();
    if !is_frozen_addr_slow(addr) {
        // Not ours: the default action (a crash) on return.
        // SAFETY: resets the handler of this signal to the default.
        unsafe { libc::signal(sig, libc::SIG_DFL) };
        return;
    }
    // Debugging aid: the raw frames (execinfo), symbolized by the system without allocating.
    {
        extern "C" {
            fn backtrace(buf: *mut *mut libc::c_void, size: i32) -> i32;
            fn backtrace_symbols_fd(buf: *const *mut libc::c_void, size: i32, fd: i32);
        }
        let mut frames = [std::ptr::null_mut::<libc::c_void>(); 40];
        let msg = b"tsrs shared graph: write to frozen address\n";
        // SAFETY: writes a static buffer to stderr; fills and prints a local frame buffer.
        unsafe {
            libc::write(2, msg.as_ptr().cast(), msg.len());
            let n = backtrace(frames.as_mut_ptr(), 40);
            backtrace_symbols_fd(frames.as_ptr(), n, 2);
        }
        let _ = addr;
    }
    if protect_mode() == 2 {
        let page = addr & !(crate::reserve::page_size() - 1);
        // SAFETY: a page of a frozen chunk; it becomes writable again so the faulting write can complete.
        unsafe { libc::mprotect(std::ptr::with_exposed_provenance_mut::<libc::c_void>(page), crate::reserve::page_size(), libc::PROT_READ | libc::PROT_WRITE) };
        return;
    }
    std::process::abort();
}

/// A checker's private values for frozen cells and its private tables hung off frozen objects.
#[derive(Default)]
pub struct Overlay {
    cells: RefCell<FxHashMap<usize, [u64; 2]>>,
    /// One bit per 64 bytes of the frozen range: some cell in that line has an overlay value (skips the hash lookup).
    lines: RefCell<Vec<u64>>,
    tables: RefCell<FxHashMap<usize, Box<dyn Any>>>,
    id_words: RefCell<Vec<Option<Box<[u64; ID_PAGE]>>>>,
}

impl Overlay {
    #[inline]
    fn line(addr: usize) -> usize {
        // Relaxed: set before the forks' threads were spawned.
        (addr - FROZEN_LO.load(Ordering::Relaxed)) >> 6
    }

    #[inline]
    pub fn get_cell<T: Copy>(&self, addr: usize, v: T) -> T {
        let line = Self::line(addr);
        let has = self.lines.borrow().get(line >> 6).is_some_and(|w| w & (1 << (line & 63)) != 0);
        if !has {
            return v;
        }
        self.cells.borrow().get(&addr).map_or(v, |&b| decode(b))
    }

    pub fn set_cell<T: Copy>(&self, addr: usize, v: T) {
        let line = Self::line(addr);
        {
            let mut lines = self.lines.borrow_mut();
            if lines.is_empty() {
                // Relaxed: as in `line`.
                let span = FROZEN_SPAN.load(Ordering::Relaxed);
                lines.resize((span >> 6).div_ceil(64) + 1, 0);
            }
            lines[line >> 6] |= 1 << (line & 63);
        }
        mark_dirty(addr);
        self.cells.borrow_mut().insert(addr, encode(v));
    }

    pub fn cell_count(&self) -> usize {
        self.cells.borrow().len()
    }
    /// Pages of object-flag words (256 per page).
    pub fn id_word_pages(&self) -> usize {
        self.id_words.borrow().iter().filter(|p| p.is_some()).count()
    }
    pub fn table_count(&self) -> usize {
        self.tables.borrow().len()
    }

    /// The private table keyed by `addr` (a frozen object), made by `make` on first use.
    pub fn table<T: 'static>(&self, addr: usize, make: impl FnOnce() -> T) -> &'static T {
        let mut tables = self.tables.borrow_mut();
        let b = tables.entry(addr).or_insert_with(|| Box::new(make()));
        let t: &T = b.downcast_ref::<T>().expect("shared graph: overlay table of another type");
        // SAFETY: a table is never removed or replaced while the overlay lives, and the overlay lives as long as its
        // checker; boxes do not move when the map grows.
        unsafe { &*std::ptr::from_ref(t) }
    }

    pub fn try_table<T: 'static>(&self, addr: usize) -> Option<&'static T> {
        let tables = self.tables.borrow();
        let t: &T = tables.get(&addr)?.downcast_ref::<T>()?;
        // SAFETY: as in `table`.
        Some(unsafe { &*std::ptr::from_ref(t) })
    }
}

thread_local! {
    static CURRENT: Cell<*const Overlay> = const { Cell::new(std::ptr::null()) };
}

/// Makes `overlay` the one `OvCell`s on this thread use. The pool calls it wherever it locks a checker.
pub fn enter_overlay(overlay: &Overlay) {
    CURRENT.with(|c| c.set(std::ptr::from_ref(overlay)));
}

/// The overlay of the checker this thread runs, if any. Without one (the main thread sorting diagnostics after the
/// pass), frozen objects read as the seed left them.
#[inline]
pub fn try_current_overlay() -> Option<&'static Overlay> {
    let p = CURRENT.with(Cell::get);
    // SAFETY: set by `enter_overlay` from a checker that outlives its use on this thread.
    (!p.is_null()).then(|| unsafe { &*p })
}

pub fn current_overlay() -> &'static Overlay {
    let p = CURRENT.with(Cell::get);
    assert!(!p.is_null(), "shared graph: a frozen object was touched on a thread without a checker overlay");
    // SAFETY: set by `enter_overlay` from a checker that outlives its use on this thread.
    unsafe { &*p }
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

/// A `Cell` for a lazily filled field of an arena object that may be frozen (see the module docs).
#[repr(transparent)]
pub struct OvCell<T>(Cell<T>);

impl<T: Copy + Default + PartialEq> Default for OvCell<T> {
    fn default() -> Self {
        OvCell(Cell::new(T::default()))
    }
}

impl<T: Copy + std::fmt::Debug> std::fmt::Debug for OvCell<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

impl<T: Copy + Default + PartialEq> OvCell<T> {
    #[inline]
    pub const fn new(v: T) -> Self {
        OvCell(Cell::new(v))
    }

    /// A set value is returned as is (a fork's write over a value the seed set is an override: counted, not seen);
    /// an unset one is looked up in the overlay only if its line is dirty.
    #[inline]
    pub fn get(&self) -> T {
        let v = self.0.get();
        if v != T::default() || !dirty(std::ptr::from_ref(self).addr()) {
            return v;
        }
        overlay_get_frozen(std::ptr::from_ref(self).addr(), v)
    }

    #[inline]
    pub fn set(&self, v: T) {
        if any_frozen() && self.set_frozen(v) {
            return;
        }
        self.0.set(v);
    }

    /// Writes to the overlay if the cell is frozen (returns true).
    #[cold]
    #[inline(never)]
    fn set_frozen(&self, v: T) -> bool {
        let addr = std::ptr::from_ref(self).addr();
        if !is_frozen_addr_slow(addr) {
            return false;
        }
        if self.0.get() != T::default() && self.0.get() != v {
            OVERRIDES.fetch_add(1, Ordering::Relaxed);
            log_site(b"tsrs shared graph: override\n");
        }
        current_overlay().set_cell(addr, v);
        true
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

impl<T: Copy + Default + PartialEq> Clone for OvCell<T> {
    fn clone(&self) -> Self {
        OvCell(Cell::new(self.get()))
    }
}

/// Reads a `RefCell` without writing its borrow flag when it is frozen (a frozen object is never borrowed mutably).
#[inline]
pub fn with_ref<T, R>(c: &RefCell<T>, f: impl FnOnce(&T) -> R) -> R {
    if any_frozen() && is_frozen_addr_slow(std::ptr::from_ref(c).addr()) {
        // SAFETY: frozen memory is never written after the freeze, so no mutable borrow exists or can start.
        return f(unsafe { c.try_borrow_unguarded() }.expect("shared graph: frozen RefCell borrowed mutably"));
    }
    f(&c.borrow())
}

/// Whether the object at `addr` is frozen (one load while nothing is).
#[inline]
pub fn frozen<T: ?Sized>(r: &T) -> bool {
    any_frozen() && is_frozen_addr_slow(std::ptr::from_ref(r).cast::<u8>().addr())
}

/// A word for a raw pointer kept in an `OvCell` (`OvCell<RawWord>`): the address, provenance exposed.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct RawWord(pub usize);

impl RawWord {
    #[inline]
    pub fn from_ptr<T>(p: *const T) -> RawWord {
        RawWord(p.expose_provenance())
    }
    #[inline]
    pub fn ptr<T>(self) -> *const T {
        std::ptr::with_exposed_provenance(self.0)
    }
}

/// `ThinSliceCell` whose writes to a frozen address go to the overlay.
pub struct OvThinSliceCell<T: 'static>(OvCell<crate::ThinSlice<T>>);

impl<T> OvThinSliceCell<T> {
    #[inline]
    pub fn new(s: &'static [T]) -> Self {
        OvThinSliceCell(OvCell::new(crate::ThinSlice::new(s)))
    }
    #[inline]
    pub fn get(&self) -> &'static [T] {
        self.0.get().get()
    }
    #[inline]
    pub fn set(&self, s: &'static [T]) {
        self.0.set(crate::ThinSlice::new(s))
    }
}

impl<T> Default for OvThinSliceCell<T> {
    fn default() -> Self {
        OvThinSliceCell::new(&[])
    }
}

/// `OptionThinSliceCell` whose writes to a frozen address go to the overlay.
pub struct OvOptionThinSliceCell<T: 'static>(OvCell<Option<crate::ThinSlice<T>>>);

impl<T> OvOptionThinSliceCell<T> {
    #[inline]
    pub fn new(s: Option<&'static [T]>) -> Self {
        OvOptionThinSliceCell(OvCell::new(s.map(crate::ThinSlice::new)))
    }
    #[inline]
    pub fn get(&self) -> Option<&'static [T]> {
        self.0.get().map(crate::ThinSlice::get)
    }
    #[inline]
    pub fn set(&self, s: Option<&'static [T]>) {
        self.0.set(s.map(crate::ThinSlice::new))
    }
}

impl<T> Default for OvOptionThinSliceCell<T> {
    fn default() -> Self {
        OvOptionThinSliceCell::new(None)
    }
}

const ID_PAGE: usize = 256;

impl Overlay {
    /// A word kept by object id (frozen types' object flags): `None` if never set.
    #[inline]
    pub fn id_word(&self, id: u32) -> Option<u64> {
        let pages = self.id_words.borrow();
        let page = pages.get(id as usize / ID_PAGE)?.as_ref()?;
        let w = page[id as usize % ID_PAGE];
        (w >> 63 != 0).then_some(w & !(1 << 63))
    }

    fn set_id_word(&self, id: u32, value: u64) {
        let mut pages = self.id_words.borrow_mut();
        let p = id as usize / ID_PAGE;
        if pages.len() <= p {
            pages.resize_with(p + 1, || None);
        }
        pages[p].get_or_insert_with(|| Box::new([0; ID_PAGE]))[id as usize % ID_PAGE] = value | (1 << 63);
    }
}

/// A frozen type's object flags written by a fork: kept by type id in the fork's overlay; the flags cell's line is
/// marked dirty so reads look there.
pub fn set_id_word_frozen(cell_addr: usize, id: u32, value: u64) {
    mark_dirty(cell_addr);
    current_overlay().set_id_word(id, value);
}

impl<T: Copy + Default> Default for OvExact<T> {
    fn default() -> Self {
        OvExact(Cell::new(T::default()))
    }
}

/// `OvCell` for a word whose unwritten value is not its default (a tag bit set at creation): a frozen cell always
/// looks in the overlay first (one hash lookup per read of a frozen cell).
#[repr(transparent)]
pub struct OvExact<T>(Cell<T>);

impl<T: Copy> OvExact<T> {
    #[inline]
    pub const fn new(v: T) -> Self {
        OvExact(Cell::new(v))
    }
    #[inline]
    pub fn get(&self) -> T {
        let v = self.0.get();
        if !dirty(std::ptr::from_ref(self).addr()) {
            return v;
        }
        overlay_get_frozen(std::ptr::from_ref(self).addr(), v)
    }
    #[inline]
    pub fn set(&self, v: T) {
        if any_frozen() {
            let addr = std::ptr::from_ref(self).addr();
            if is_frozen_addr_slow(addr) {
                        current_overlay().set_cell(addr, v);
                return;
            }
        }
        self.0.set(v);
    }
}

#[cold]
#[inline(never)]
fn overlay_get_frozen<T: Copy>(addr: usize, v: T) -> T {
    try_current_overlay().map_or(v, |o| o.get_cell(addr, v))
}

/// Writes `v` to the current overlay if the cell at `cell` is frozen (returns true).
#[inline]
pub fn overlay_set_if_frozen<C, T: Copy>(cell: &C, v: T) -> bool {
    let addr = std::ptr::from_ref(cell).addr();
    if !is_frozen_addr_slow(addr) {
        return false;
    }
    current_overlay().set_cell(addr, v);
    true
}

/// Writes by forks to `OwnedCell`s of frozen symbols and nodes (discovery counter).
pub static OWNED_WRITES: AtomicUsize = AtomicUsize::new(0);

/// Object-flag bits forks changed on frozen types (stats).
pub static FLAG_BITS: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];

pub fn count_flag_bits(changed: u32) {
    for (i, c) in FLAG_BITS.iter().enumerate() {
        if changed & (1 << i) != 0 {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Prints the stack under `TSRS_SHARED_GRAPH_LOG_OWNED=1` (discovery of fork writes to frozen objects).
#[cold]
pub fn log_owned_site() {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("TSRS_SHARED_GRAPH_LOG_OWNED").is_ok_and(|v| v == "1")) {
        return;
    }
    #[cfg(unix)]
    {
        extern "C" {
            fn backtrace(buf: *mut *mut libc::c_void, size: i32) -> i32;
            fn backtrace_symbols_fd(buf: *const *mut libc::c_void, size: i32, fd: i32);
        }
        let mut frames = [std::ptr::null_mut::<libc::c_void>(); 40];
        let msg = b"tsrs shared graph: owned write\n";
        // SAFETY: writes a static buffer to stderr; fills and prints a local frame buffer.
        unsafe {
            libc::write(2, msg.as_ptr().cast(), msg.len());
            let n = backtrace(frames.as_mut_ptr(), 40);
            backtrace_symbols_fd(frames.as_ptr(), n, 2);
        }
    }
}

/// `OwnedCell::set` on a frozen cell: counted and logged (`TSRS_SHARED_GRAPH_LOG_OWNED=1`), kept in the overlay.
#[cold]
#[inline(never)]
pub fn owned_set_if_frozen<C, T: Copy>(cell: &C, v: T) -> bool {
    let addr = std::ptr::from_ref(cell).addr();
    if !is_frozen_addr_slow(addr) {
        return false;
    }
    OWNED_WRITES.fetch_add(1, Ordering::Relaxed);
    log_owned_site();
    current_overlay().set_cell(addr, v);
    true
}

thread_local! {
    static SEED_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Marks the calling thread as the seed checker's: objects it creates get their ids at once (a fork must never
/// write an id into a frozen object).
pub fn set_seed_thread(on: bool) {
    SEED_THREAD.with(|s| s.set(on));
}

#[inline]
pub fn on_seed_thread() -> bool {
    COMPILED_IN && SEED_THREAD.with(Cell::get)
}

