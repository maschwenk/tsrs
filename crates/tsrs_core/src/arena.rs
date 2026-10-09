//! Oxc-backed allocation owners used by the port's pointer compatibility layer.
//!
//! Fixed, no-`Drop` values live in an [`oxc_allocator::Allocator`]. Values which own resources or otherwise need a
//! destructor live in heap sidecars owned by the same [`Arena`]. Dropping a [`Region`] runs those sidecar
//! destructors before releasing the Oxc chunks.
//!
//! The region and scratch APIs remain while callers are migrated to explicit owners. They select an owner; they no
//! longer provide block recycling, compressed-address reservation, or speculative bump-pointer rewinds.

use std::alloc::Layout;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};

use oxc_allocator::Allocator;

/// Kept for allocation-profile output compatibility. Oxc does not maintain per-size free lists.
pub const MAX_FREE_SIZE: usize = 512;
/// Kept for the checked-pointer diagnostic in `ptr`; Oxc-owned blocks are never poisoned or reused individually.
pub const POISON: u8 = 0xA5;

#[cfg(feature = "alloc-profile")]
const CLASSES: usize = MAX_FREE_SIZE / 8 + 1;

struct Sidecar {
    ptr: *mut u8,
    len: usize,
    drop: unsafe fn(*mut u8, usize),
}

// SAFETY: `ptr` names `len` live `T`s allocated as a `Box<T>` or `Box<[T]>`, and this entry is consumed once.
unsafe fn drop_sidecar<T>(ptr: *mut u8, len: usize) {
    if len == 1 {
        // SAFETY: established by the function contract.
        unsafe { drop(Box::from_raw(ptr.cast::<T>())) };
    } else {
        let slice = std::ptr::slice_from_raw_parts_mut(ptr.cast::<T>(), len);
        // SAFETY: established by the function contract.
        unsafe { drop(Box::from_raw(slice)) };
    }
}

/// One Oxc allocator and the heap-owning values paired with it.
pub struct Arena {
    // Sidecars are destroyed by `Drop` before `allocator` releases the fixed data they may refer to.
    sidecars: RefCell<Vec<Sidecar>>,
    allocator: Allocator,
    region: Option<Weak<RegionInner>>,
    registered: bool,
    allocations: RefCell<Vec<(usize, usize)>>,
    sidecar_bytes: Cell<usize>,
    epoch: Cell<u64>,
    #[cfg(feature = "alloc-profile")]
    pub(crate) free_stats: FreeStats,
}

#[cfg(feature = "alloc-profile")]
pub(crate) struct FreeStats {
    pub(crate) blocks: [Cell<u64>; CLASSES],
    pub(crate) peak_blocks: [Cell<u64>; CLASSES],
    pub(crate) reissued: [Cell<u64>; CLASSES],
    pub(crate) misses: [Cell<u64>; CLASSES],
    pub(crate) bytes: Cell<u64>,
    pub(crate) peak_bytes: Cell<u64>,
}

#[cfg(feature = "alloc-profile")]
impl FreeStats {
    const fn new() -> Self {
        Self {
            blocks: [const { Cell::new(0) }; CLASSES],
            peak_blocks: [const { Cell::new(0) }; CLASSES],
            reissued: [const { Cell::new(0) }; CLASSES],
            misses: [const { Cell::new(0) }; CLASSES],
            bytes: Cell::new(0),
            peak_bytes: Cell::new(0),
        }
    }
}

pub(crate) struct Residency {
    pub(crate) capacity: usize,
    pub(crate) used: usize,
    pub(crate) current_unused: Option<(usize, usize)>,
    pub(crate) retired_unused: Vec<(usize, usize)>,
    pub(crate) current_huge: bool,
}

/// A compatibility token for parser speculation. Oxc allocations are retained until their owner is dropped.
#[derive(Clone, Copy)]
pub struct Checkpoint {
    epoch: u64,
}

impl Arena {
    pub(crate) fn new() -> Self {
        Self::with_owner(0, None, false)
    }

    fn with_owner(first_chunk: usize, region: Option<Weak<RegionInner>>, registered: bool) -> Self {
        Self {
            sidecars: RefCell::new(Vec::new()),
            allocator: if first_chunk == 0 {
                Allocator::new()
            } else {
                Allocator::with_capacity(first_chunk)
            },
            region,
            registered,
            allocations: RefCell::new(Vec::new()),
            sidecar_bytes: Cell::new(0),
            epoch: Cell::new(0),
            #[cfg(feature = "alloc-profile")]
            free_stats: FreeStats::new(),
        }
    }

    fn record_allocation(&self, addr: usize, bytes: usize) {
        if bytes == 0 {
            return;
        }
        self.allocations.borrow_mut().push((addr, bytes));
        if self.registered {
            let region = Weak::clone(self.region.as_ref().expect("registered arena has no owner"));
            REGISTRY
                .write()
                .unwrap()
                .insert(reg_key(addr), (reg_key(addr + bytes), region));
        }
    }

    #[expect(clippy::mut_from_ref, reason = "arena allocation returns fresh exclusive storage")]
    fn alloc_sidecar<T>(&self, value: T) -> &mut T {
        let ptr = Box::into_raw(Box::new(value));
        self.sidecars.borrow_mut().push(Sidecar {
            ptr: ptr.cast(),
            len: 1,
            drop: drop_sidecar::<T>,
        });
        let bytes = std::mem::size_of::<T>();
        self.sidecar_bytes.set(self.sidecar_bytes.get() + bytes);
        self.record_allocation(ptr.addr(), bytes);
        // SAFETY: `ptr` came from `Box::into_raw` and stays owned by `sidecars` until this arena is dropped.
        unsafe { &mut *ptr }
    }

    pub(crate) fn alloc_layout(&self, layout: Layout) -> NonNull<u8> {
        let ptr = self.allocator.alloc_layout(layout);
        self.record_allocation(ptr.as_ptr().addr(), layout.size());
        ptr
    }

    #[expect(clippy::mut_from_ref, reason = "arena allocation returns fresh exclusive storage")]
    pub(crate) fn alloc_with<T>(&self, layout: Layout, value: T) -> &mut T {
        if std::mem::needs_drop::<T>() {
            return self.alloc_sidecar(value);
        }
        let ptr = self.alloc_layout(layout).cast::<T>();
        // SAFETY: `ptr` has the caller-supplied layout for `T`, is fresh, and the Oxc owner outlives the result.
        unsafe {
            ptr.as_ptr().write(value);
            &mut *ptr.as_ptr()
        }
    }

    #[expect(clippy::mut_from_ref, reason = "arena allocation returns fresh exclusive storage")]
    pub(crate) fn alloc_slice_copy<T: Copy>(&self, src: &[T]) -> &mut [T] {
        if src.is_empty() {
            return &mut [];
        }
        let layout = Layout::array::<T>(src.len()).expect("arena slice layout");
        let ptr = self.alloc_layout(layout).cast::<T>();
        // SAFETY: `ptr` is fresh storage for exactly `src.len()` `T`s and the regions do not overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), ptr.as_ptr(), src.len());
            std::slice::from_raw_parts_mut(ptr.as_ptr(), src.len())
        }
    }

    #[expect(clippy::mut_from_ref, reason = "arena allocation returns fresh exclusive storage")]
    pub(crate) fn alloc_vec<T>(&self, mut items: Vec<T>) -> &mut [T] {
        if items.is_empty() {
            return &mut [];
        }
        let len = items.len();
        if std::mem::needs_drop::<T>() {
            let ptr = Box::into_raw(items.into_boxed_slice());
            let data = ptr.cast::<T>();
            self.sidecars.borrow_mut().push(Sidecar {
                ptr: data.cast(),
                len,
                drop: drop_sidecar::<T>,
            });
            let bytes = std::mem::size_of::<T>() * len;
            self.sidecar_bytes.set(self.sidecar_bytes.get() + bytes);
            self.record_allocation(data.addr(), bytes);
            // SAFETY: the boxed slice stays owned by `sidecars` until this arena is dropped.
            return unsafe { &mut *ptr };
        }
        let layout = Layout::array::<T>(len).expect("arena vector layout");
        let ptr = self.alloc_layout(layout).cast::<T>();
        // SAFETY: `ptr` is fresh storage for `len` `T`s. Setting the source length to zero transfers ownership.
        unsafe {
            std::ptr::copy_nonoverlapping(items.as_ptr(), ptr.as_ptr(), len);
            items.set_len(0);
            std::slice::from_raw_parts_mut(ptr.as_ptr(), len)
        }
    }

    #[expect(clippy::mut_from_ref, reason = "arena allocation returns fresh exclusive storage")]
    pub(crate) fn alloc_str(&self, value: &str) -> &mut str {
        let bytes = self.alloc_slice_copy(value.as_bytes());
        // SAFETY: `bytes` is copied from a valid UTF-8 string.
        unsafe { std::str::from_utf8_unchecked_mut(bytes) }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.allocator.capacity() + self.sidecar_bytes.get()
    }

    #[cfg(feature = "alloc-profile")]
    pub(crate) fn current_chunk_unused(&self) -> usize {
        self.allocator.capacity().saturating_sub(self.allocator.used_bytes())
    }

    pub(crate) fn residency(&self) -> Residency {
        Residency {
            capacity: self.capacity(),
            used: self.used_bytes(),
            current_unused: None,
            retired_unused: Vec::new(),
            current_huge: false,
        }
    }

    pub(crate) fn used_ranges(&self) -> Vec<(usize, usize)> {
        self.allocations.borrow().clone()
    }

    fn used_bytes(&self) -> usize {
        self.allocator.used_bytes() + self.sidecar_bytes.get()
    }

    pub(crate) fn bump_epoch(&self) {
        self.epoch.set(self.epoch.get().wrapping_add(1));
    }

    pub(crate) fn checkpoint(&self) -> Checkpoint {
        Checkpoint { epoch: self.epoch.get() }
    }

    #[expect(clippy::trivially_copy_pass_by_ref, reason = "compatibility API accepts the parser's checkpoint borrow")]
    pub(crate) fn rewindable_now(&self, cp: &Checkpoint) -> bool {
        let _ = cp.epoch;
        false
    }

    fn contains(&self, addr: usize) -> bool {
        self.allocations
            .borrow()
            .iter()
            .any(|&(start, len)| start <= addr && addr < start + len)
    }

    fn forget_sidecars(&self) {
        self.sidecars.borrow_mut().clear();
        self.sidecar_bytes.set(0);
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        let sidecars = self.sidecars.get_mut();
        for sidecar in sidecars.drain(..).rev() {
            // SAFETY: each sidecar entry owns a distinct live heap allocation and is consumed exactly once.
            unsafe { (sidecar.drop)(sidecar.ptr, sidecar.len) };
        }
    }
}

#[cfg(feature = "alloc-profile")]
pub(crate) fn scrub_stack_for_census() {}

#[cfg(feature = "alloc-profile")]
pub(crate) fn poison_mode() -> bool {
    false
}

/// Oxc does not expose stable partial-reset checkpoints; speculative allocations stay until owner teardown.
pub(crate) fn rewind(_arena: &Arena, _cp: Checkpoint) {}

#[inline]
fn reg_key(addr: usize) -> usize {
    addr + 1
}

static REGISTRY: RwLock<BTreeMap<usize, (usize, Weak<RegionInner>)>> = RwLock::new(BTreeMap::new());
static ANY_REGION: AtomicBool = AtomicBool::new(false);

#[inline]
fn any_region() -> bool {
    // Relaxed: this is a one-way fast-path flag; the registry lock synchronizes the data used after it is true.
    ANY_REGION.load(Ordering::Relaxed)
}

thread_local! {
    pub(crate) static CURRENT: Cell<*const Arena> = const { Cell::new(std::ptr::null()) };
    static SCOPES: RefCell<Vec<(u64, *const Arena)>> = const { RefCell::new(Vec::new()) };
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
    static SCRATCH: Cell<(*const Arena, *const Arena)> = const {
        Cell::new((std::ptr::null(), std::ptr::null()))
    };
}

struct OwnerLock {
    state: Mutex<(Option<std::thread::ThreadId>, u32, u32)>,
    released: Condvar,
}

impl OwnerLock {
    fn lock(&self) {
        let me = std::thread::current().id();
        let mut state = self.state.lock().unwrap();
        loop {
            match state.0 {
                None => {
                    state.0 = Some(me);
                    state.1 = 1;
                    return;
                }
                Some(owner) if owner == me => {
                    state.1 += 1;
                    return;
                }
                Some(_) => {
                    state.2 += 1;
                    state = self.released.wait(state).unwrap();
                    state.2 -= 1;
                }
            }
        }
    }

    fn unlock(&self) {
        let mut state = self.state.lock().unwrap();
        state.1 -= 1;
        if state.1 == 0 {
            state.0 = None;
            let waiting = state.2 != 0;
            drop(state);
            if waiting {
                self.released.notify_one();
            }
        }
    }
}

pub(crate) struct RegionInner {
    // Callbacks must run before `arena` drops sidecars and Oxc chunks.
    on_free: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    arena: Box<Arena>,
    lock: OwnerLock,
    owners: Mutex<Vec<usize>>,
    retire: AtomicBool,
}

#[expect(clippy::non_send_fields_in_send_ty, reason = "allocation is serialized by the region owner lock")]
// SAFETY: allocation and sidecar mutation happen only while the owning thread holds `lock`.
unsafe impl Send for RegionInner {}
// SAFETY: as for `Send`; readers reach allocated values, not the allocator's mutable cursor state.
unsafe impl Sync for RegionInner {}

/// A freeable Oxc allocation owner. Clones share ownership; the last handle tears it down.
#[derive(Clone)]
pub struct Region(Arc<RegionInner>);

impl Region {
    pub fn new(first_chunk: usize) -> Self {
        // Relaxed: this only disables the no-region fast path; registry access is synchronized by its lock.
        ANY_REGION.store(true, Ordering::Relaxed);
        Self::new_in(first_chunk, true)
    }

    pub fn new_scratch(first_chunk: usize) -> Self {
        Self::new_in(first_chunk, false)
    }

    pub fn new_scratch_in_large_slabs(first_chunk: usize) -> Self {
        Self::new_in(first_chunk, false)
    }

    fn new_in(first_chunk: usize, registered: bool) -> Self {
        Self(Arc::new_cyclic(|weak| RegionInner {
            on_free: Mutex::new(Vec::new()),
            arena: Box::new(Arena::with_owner(first_chunk, Some(Weak::clone(weak)), registered)),
            lock: OwnerLock {
                state: Mutex::new((None, 0, 0)),
                released: Condvar::new(),
            },
            owners: Mutex::new(Vec::new()),
            retire: AtomicBool::new(false),
        }))
    }

    pub fn retire_on_free(&self) {
        // Relaxed: the last `Arc` release orders this flag write before `RegionInner::drop` reads it.
        self.0.retire.store(true, Ordering::Relaxed);
    }

    pub fn enter(&self) -> RegionScope {
        self.0.lock.lock();
        RegionScope::push(&raw const *self.0.arena, Some(self.clone()))
    }

    pub fn containing(addr: usize) -> Option<Self> {
        if !any_region() {
            return None;
        }
        let registry = REGISTRY.read().unwrap();
        let (_, (end, owner)) = registry.range(..=reg_key(addr)).next_back()?;
        (reg_key(addr) < *end).then(|| owner.upgrade().map(Self)).flatten()
    }

    pub fn containing_all(addrs: impl Iterator<Item = usize>) -> Vec<Option<Self>> {
        if !any_region() {
            return addrs.map(|_| None).collect();
        }
        let registry = REGISTRY.read().unwrap();
        addrs
            .map(|addr| {
                let (_, (end, owner)) = registry.range(..=reg_key(addr)).next_back()?;
                (reg_key(addr) < *end).then(|| owner.upgrade().map(Self)).flatten()
            })
            .collect()
    }

    pub fn on_free(&self, callback: Box<dyn FnOnce() + Send>) {
        self.0.on_free.lock().unwrap().push(callback);
    }

    pub fn adopt_owner(&self, addr: usize) {
        self.0.owners.lock().unwrap().push(addr);
        REGISTRY
            .write()
            .unwrap()
            .insert(reg_key(addr), (reg_key(addr + 1), Arc::downgrade(&self.0)));
    }

    pub fn allocated_bytes(&self) -> usize {
        self.0.arena.capacity()
    }

    pub fn trim(&self) {
        // Oxc retains the current chunk for reuse until owner teardown.
    }

    pub fn used_bytes(&self) -> usize {
        self.0.arena.used_bytes()
    }

    pub fn drop_entries(&self) -> usize {
        self.0.arena.sidecars.borrow().len()
    }

    pub fn forget_drops(&self) {
        let _scope = self.enter();
        self.0.arena.forget_sidecars();
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Region({:p})", Arc::as_ptr(&self.0))
    }
}

impl Drop for RegionInner {
    fn drop(&mut self) {
        for callback in self.on_free.get_mut().unwrap().drain(..) {
            callback();
        }
        let mut registry = REGISTRY.write().unwrap();
        if self.arena.registered {
            for &(addr, _) in self.arena.allocations.borrow().iter() {
                registry.remove(&reg_key(addr));
            }
        }
        for addr in self.owners.get_mut().unwrap().drain(..) {
            registry.remove(&reg_key(addr));
        }
        // Relaxed: the last `Arc` release orders the preceding `retire_on_free` write before this destructor.
        if self.retire.load(Ordering::Relaxed) {
            // Relaxed: diagnostic counters do not publish data and may be read after worker joins.
            RETIRED_CALLS.fetch_add(1, Ordering::Relaxed);
            // Relaxed: as above.
            RETIRED_BYTES.fetch_add(self.arena.capacity(), Ordering::Relaxed);
        }
    }
}

pub struct RegionScope {
    token: u64,
    region: Option<Region>,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl RegionScope {
    fn push(arena: *const Arena, region: Option<Region>) -> Self {
        let token = NEXT_TOKEN.with(|next| {
            let token = next.get();
            next.set(token + 1);
            token
        });
        SCOPES.with(|scopes| scopes.borrow_mut().push((token, arena)));
        CURRENT.with(|current| current.set(arena));
        Self {
            token,
            region,
            _not_send: std::marker::PhantomData,
        }
    }
}

impl Drop for RegionScope {
    fn drop(&mut self) {
        SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            if let Some(index) = scopes.iter().rposition(|&(token, _)| token == self.token) {
                scopes.remove(index);
            }
            CURRENT.with(|current| {
                current.set(scopes.last().map_or(std::ptr::null(), |&(_, arena)| arena));
            });
        });
        if let Some(region) = &self.region {
            region.0.lock.unlock();
        }
    }
}

struct SpareArena(&'static Arena);
// SAFETY: a spare arena has no allocating thread; the mutex transfers it to the next owner.
unsafe impl Send for SpareArena {}

static SPARE_ARENAS: Mutex<Vec<SpareArena>> = Mutex::new(Vec::new());

pub(crate) fn take_spare_arena() -> Option<&'static Arena> {
    SPARE_ARENAS.lock().unwrap().pop().map(|arena| arena.0)
}

pub(crate) fn no_scope_open() -> bool {
    SCOPES.with(|scopes| scopes.borrow().is_empty()) && SCRATCH.with(|scratch| scratch.get().0.is_null())
}

pub(crate) fn give_spare_arena(arena: &'static Arena) {
    CURRENT.with(|current| current.set(std::ptr::null()));
    SPARE_ARENAS.lock().unwrap().push(SpareArena(arena));
}

pub fn trim_own_arena_tail() {}

pub fn current_region() -> Option<Region> {
    let arena = CURRENT.with(|current| current.get());
    if arena.is_null() {
        return None;
    }
    // SAFETY: a non-null current arena is kept alive by a scope or is the leaked thread arena.
    unsafe { &*arena }.region.as_ref().and_then(Weak::upgrade).map(Region)
}

pub fn enter_thread_arena() -> RegionScope {
    RegionScope::push(std::ptr::from_ref(crate::ptr::own_arena()), None)
}

#[inline]
pub fn enter_owner(addr: usize) -> Option<RegionScope> {
    if !any_region() {
        return escape_scratch_unless_inside(addr);
    }
    Some(match Region::containing(addr) {
        Some(region) => region.enter(),
        None => enter_thread_arena(),
    })
}

#[inline]
pub fn enter_table_owner(addr: usize) -> Option<RegionScope> {
    if !any_region() {
        return escape_scratch_unless_inside(addr);
    }
    let current = CURRENT.with(|arena| arena.get());
    match Region::containing(addr) {
        Some(region) if std::ptr::eq(&raw const *region.0.arena, current) => None,
        _ => Some(enter_thread_arena()),
    }
}

pub struct ScratchScope {
    saved: (*const Arena, *const Arena),
    _scope: RegionScope,
}

impl Region {
    pub fn enter_scratch(&self) -> ScratchScope {
        let outer = CURRENT.with(|current| current.get());
        let outer = if outer.is_null() {
            std::ptr::from_ref(crate::ptr::own_arena())
        } else {
            outer
        };
        let scope = self.enter();
        let saved = SCRATCH.with(|scratch| scratch.replace((&raw const *self.0.arena, outer)));
        ScratchScope {
            saved,
            _scope: scope,
        }
    }
}

impl Drop for ScratchScope {
    fn drop(&mut self) {
        SCRATCH.with(|scratch| scratch.set(self.saved));
    }
}

#[inline]
pub fn escape_scratch() -> Option<RegionScope> {
    let (scratch, outer) = SCRATCH.with(|state| state.get());
    if scratch.is_null() || CURRENT.with(|current| current.get()) == outer {
        return None;
    }
    Some(RegionScope::push(outer, None))
}

#[inline]
fn escape_scratch_unless_inside(addr: usize) -> Option<RegionScope> {
    if scratch_arena().is_null() || scratch_contains(addr) {
        None
    } else {
        escape_scratch()
    }
}

#[inline]
pub fn scratch_active() -> bool {
    !SCRATCH.with(|scratch| scratch.get().0.is_null())
}

pub fn scratch_contains(addr: usize) -> bool {
    let arena = scratch_arena();
    if arena.is_null() {
        return false;
    }
    // SAFETY: the scratch scope keeps the arena alive and serializes allocation on this thread.
    unsafe { &*arena }.contains(addr)
}

#[inline]
pub(crate) fn scratch_arena() -> *const Arena {
    SCRATCH.with(|scratch| scratch.get().0)
}

static RETIRED_CALLS: AtomicUsize = AtomicUsize::new(0);
static RETIRED_BYTES: AtomicUsize = AtomicUsize::new(0);

pub fn flush_retired() {}

pub fn retired_stats() -> (usize, usize) {
    (
        // Relaxed: diagnostic counters are independent and do not publish allocator state.
        RETIRED_CALLS.load(Ordering::Relaxed),
        // Relaxed: as above.
        RETIRED_BYTES.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::P;

    use super::Region;

    #[test]
    fn region_finds_interior_addresses_and_releases_sidecars() {
        struct Dropped(Arc<AtomicUsize>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let dropped = Arc::new(AtomicUsize::new(0));
        let region = Region::new(4096);
        let value = {
            let _scope = region.enter();
            P::new(Dropped(Arc::clone(&dropped)))
        };
        assert!(Region::containing(value.addr()).is_some());
        drop(region);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(Region::containing(value.addr()).is_none());
    }

    #[test]
    fn scratch_routes_explicit_scratch_allocations() {
        let scratch = Region::new_scratch(4096);
        let addr = {
            let _scope = scratch.enter_scratch();
            let value = P::new_scratch(1_u64);
            assert!(super::scratch_contains(value.addr()));
            value.addr()
        };
        assert!(!super::scratch_contains(addr));
    }
}
