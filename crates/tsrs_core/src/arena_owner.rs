//! Explicit owners and scoped references for the `oxc_allocator` migration.
//!
//! An [`ArenaBuilder`] is exclusive construction state. [`ArenaBuilder::seal`] permanently removes allocation
//! access, after which the allocation may be shared through [`SealedArena`]. Values do not expose raw references:
//! stored references are [`ArenaKey`]s, and resolving a key borrows the owner that keeps its allocation alive.

use std::{
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    num::NonZeroU64,
    ops::Deref,
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use oxc_allocator::Allocator;
use rustc_hash::FxHashMap;

static NEXT_ARENA_ID: AtomicU64 = AtomicU64::new(1);

/// Process-unique identity of an arena owner. Identities are never reused, so a stale key cannot name a later arena.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArenaId(NonZeroU64);

impl ArenaId {
    fn fresh() -> Self {
        // Relaxed: the counter only issues unique identities; it publishes no arena state.
        let id = NEXT_ARENA_ID.fetch_add(1, Ordering::Relaxed);
        let Some(id) = NonZeroU64::new(id) else {
            panic!("arena owner identity space exhausted");
        };
        Self(id)
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Debug for ArenaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ArenaId").field(&self.get()).finish()
    }
}

/// Exclusive construction state for one Oxc allocation.
pub struct ArenaBuilder {
    id: ArenaId,
    allocator: Allocator,
}

impl ArenaBuilder {
    pub fn new() -> Self {
        Self {
            id: ArenaId::fresh(),
            allocator: Allocator::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            id: ArenaId::fresh(),
            allocator: Allocator::with_capacity(capacity),
        }
    }

    pub fn id(&self) -> ArenaId {
        self.id
    }

    /// Allocates a value that needs no destructor. `oxc_allocator` enforces the no-`Drop` rule at compile time.
    pub fn alloc<T>(&self, value: T) -> ArenaKey<T> {
        ArenaKey::new(self.id, NonNull::from(self.allocator.alloc(value)))
    }

    pub fn alloc_slice_copy<T: Copy>(&self, values: &[T]) -> ArenaSlice<T> {
        let values = self.allocator.alloc_slice_copy(values);
        ArenaSlice {
            arena: self.id,
            ptr: NonNull::from(&mut *values).cast(),
            len: values.len(),
            marker: PhantomData,
        }
    }

    pub fn alloc_str(&self, value: &str) -> ArenaStr {
        let value = self.allocator.alloc_str(value);
        ArenaStr {
            arena: self.id,
            ptr: NonNull::from(value).cast(),
            len: value.len(),
        }
    }

    /// Ends construction. The returned owner exposes no way to allocate or reset its allocator.
    pub fn seal(self) -> Arc<SealedArena> {
        Arc::new(SealedArena {
            id: self.id,
            allocator: self.allocator,
        })
    }

    /// Seals the arena together with a root allocated by this builder.
    pub fn finish<T>(self, root: ArenaKey<T>) -> OwnedRoot<T> {
        assert_eq!(root.arena, self.id, "root belongs to another arena");
        OwnedRoot {
            owner: self.seal(),
            root,
        }
    }
}

impl Default for ArenaBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// An Oxc allocator after construction has ended.
///
/// No method returns `&Allocator`, so allocation and reset cannot race with readers. Its chunks stay at stable
/// addresses until the last `Arc<SealedArena>` is dropped.
pub struct SealedArena {
    id: ArenaId,
    allocator: Allocator,
}

// SAFETY: `Allocator` mutates cursor cells only while allocating or resetting. `SealedArena` is constructed by
// consuming `ArenaBuilder`, exposes neither operation, and only performs read-only byte-count queries thereafter.
unsafe impl Sync for SealedArena {}

impl SealedArena {
    pub fn id(&self) -> ArenaId {
        self.id
    }

    pub fn used_bytes(&self) -> usize {
        self.allocator.used_bytes()
    }

    pub fn capacity(&self) -> usize {
        self.allocator.capacity()
    }

    pub fn scope(&self) -> ArenaScope<'_> {
        ArenaScope { owner: self }
    }
}

/// A typed address whose owner must be borrowed before it can be dereferenced.
#[repr(C)]
pub struct ArenaKey<T: ?Sized> {
    arena: ArenaId,
    ptr: NonNull<T>,
    marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> ArenaKey<T> {
    fn new(arena: ArenaId, ptr: NonNull<T>) -> Self {
        Self {
            arena,
            ptr,
            marker: PhantomData,
        }
    }

    pub fn arena(self) -> ArenaId {
        self.arena
    }

    pub fn addr(self) -> usize {
        self.ptr.as_ptr().cast::<()>() as usize
    }
}

impl<T: ?Sized> Clone for ArenaKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for ArenaKey<T> {}

impl<T: ?Sized> PartialEq for ArenaKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.arena == other.arena && std::ptr::addr_eq(self.ptr.as_ptr(), other.ptr.as_ptr())
    }
}

impl<T: ?Sized> Eq for ArenaKey<T> {}

impl<T: ?Sized> Hash for ArenaKey<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.arena.hash(state);
        self.ptr.as_ptr().cast::<()>().hash(state);
    }
}

impl<T: ?Sized> fmt::Debug for ArenaKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArenaKey")
            .field("arena", &self.arena)
            .field("ptr", &self.ptr)
            .finish()
    }
}

// SAFETY: an `ArenaKey` cannot dereference its pointer. Thread access requires an `ArenaScope`, whose owner keeps the
// allocation live; normal `T: Send + Sync` bounds govern access to the resolved value.
unsafe impl<T: ?Sized + Send + Sync> Send for ArenaKey<T> {}
// SAFETY: as for `Send`; sharing the inert identity does not access `T`.
unsafe impl<T: ?Sized + Send + Sync> Sync for ArenaKey<T> {}

/// A slice stored in an arena. It is inert until resolved through a matching scope.
#[derive(Clone, Copy)]
pub struct ArenaSlice<T> {
    arena: ArenaId,
    ptr: NonNull<T>,
    len: usize,
    marker: PhantomData<fn() -> T>,
}

impl<T> ArenaSlice<T> {
    pub fn arena(self) -> ArenaId {
        self.arena
    }

    pub fn len(self) -> usize {
        self.len
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

// SAFETY: `ArenaSlice` has the same inert-key and scoped-resolution contract as `ArenaKey`.
unsafe impl<T: Send + Sync> Send for ArenaSlice<T> {}
// SAFETY: as for `Send`.
unsafe impl<T: Send + Sync> Sync for ArenaSlice<T> {}

/// A UTF-8 string stored in an arena. It is inert until resolved through a matching scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArenaStr {
    arena: ArenaId,
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: bytes are initialized once and immutable after sealing; access requires a live matching scope.
unsafe impl Send for ArenaStr {}
// SAFETY: as for `Send`.
unsafe impl Sync for ArenaStr {}

/// Borrow of one sealed owner. Resolved references cannot outlive this value's borrow.
#[derive(Clone, Copy)]
pub struct ArenaScope<'a> {
    owner: &'a SealedArena,
}

impl<'a> ArenaScope<'a> {
    pub fn id(self) -> ArenaId {
        self.owner.id
    }

    #[track_caller]
    pub fn resolve<T: ?Sized>(self, key: ArenaKey<T>) -> ArenaRef<'a, T> {
        assert_eq!(
            key.arena, self.owner.id,
            "arena key resolved by the wrong owner"
        );
        // SAFETY: `ArenaKey` constructors are private and only accept pointers returned by this owner's allocator.
        // Matching process-unique ids prove this is that owner, whose borrow keeps every chunk alive for `'a`.
        ArenaRef {
            arena: key.arena,
            value: unsafe {
                // SAFETY: established above.
                key.ptr.as_ref()
            },
        }
    }

    #[track_caller]
    pub fn resolve_slice<T>(self, value: &ArenaSlice<T>) -> &'a [T] {
        assert_eq!(
            value.arena, self.owner.id,
            "arena slice resolved by the wrong owner"
        );
        // SAFETY: the private constructor records a slice returned by this allocator; the owner is live for `'a`.
        unsafe { std::slice::from_raw_parts(value.ptr.as_ptr(), value.len) }
    }

    #[track_caller]
    pub fn resolve_str(self, value: ArenaStr) -> &'a str {
        assert_eq!(
            value.arena, self.owner.id,
            "arena string resolved by the wrong owner"
        );
        // SAFETY: the private constructor records a `str` returned by this allocator, which is live for `'a`.
        let bytes = unsafe { std::slice::from_raw_parts(value.ptr.as_ptr(), value.len) };
        std::str::from_utf8(bytes).expect("arena string was allocated from valid UTF-8")
    }
}

/// A resolved, owner-bounded arena reference with pointer-identity equality.
#[derive(Clone, Copy)]
pub struct ArenaRef<'a, T: ?Sized> {
    arena: ArenaId,
    value: &'a T,
}

impl<T: ?Sized> ArenaRef<'_, T> {
    pub fn arena(self) -> ArenaId {
        self.arena
    }

    pub fn addr(self) -> usize {
        std::ptr::from_ref(self.value).cast::<()>() as usize
    }
}

impl<T: ?Sized> Deref for ArenaRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<T: ?Sized> PartialEq for ArenaRef<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        self.arena == other.arena && std::ptr::eq(self.value, other.value)
    }
}

impl<T: ?Sized> Eq for ArenaRef<'_, T> {}

impl<T: ?Sized> Hash for ArenaRef<'_, T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.arena.hash(state);
        std::ptr::from_ref(self.value).cast::<()>().hash(state);
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for ArenaRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// A root and the strong owner that keeps it alive.
pub struct OwnedRoot<T: ?Sized> {
    owner: Arc<SealedArena>,
    root: ArenaKey<T>,
}

/// Heap state paired with an arena root. `state` is declared first so its destructors run while arena references
/// are still valid; the root's strong owner is released afterwards.
pub struct OwnedGraph<T: ?Sized, S> {
    state: S,
    root: OwnedRoot<T>,
}

impl<T: ?Sized, S> OwnedGraph<T, S> {
    pub fn new(root: OwnedRoot<T>, state: S) -> Self {
        Self { state, root }
    }

    pub fn state(&self) -> &S {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut S {
        &mut self.state
    }

    pub fn root(&self) -> &OwnedRoot<T> {
        &self.root
    }

    pub fn with<R>(&self, f: impl for<'a> FnOnce(&S, ArenaRef<'a, T>) -> R) -> R {
        self.root.with(|root| f(&self.state, root))
    }
}

impl<T: ?Sized> OwnedRoot<T> {
    pub fn owner(&self) -> &Arc<SealedArena> {
        &self.owner
    }

    pub fn key(&self) -> ArenaKey<T> {
        self.root
    }

    pub fn with<R>(&self, f: impl for<'a> FnOnce(ArenaRef<'a, T>) -> R) -> R {
        f(self.owner.scope().resolve(self.root))
    }
}

impl<T: ?Sized> Clone for OwnedRoot<T> {
    fn clone(&self) -> Self {
        Self {
            owner: Arc::clone(&self.owner),
            root: self.root,
        }
    }
}

/// Strong owners used by a program or checker to resolve keys from several arenas.
#[derive(Default)]
pub struct ArenaGroup {
    owners: FxHashMap<ArenaId, Arc<SealedArena>>,
}

impl ArenaGroup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, owner: Arc<SealedArena>) {
        self.owners.entry(owner.id).or_insert(owner);
    }

    pub fn scope(&self) -> ArenaGroupScope<'_> {
        ArenaGroupScope {
            owners: &self.owners,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ArenaGroupScope<'a> {
    owners: &'a FxHashMap<ArenaId, Arc<SealedArena>>,
}

impl<'a> ArenaGroupScope<'a> {
    pub fn resolve<T: ?Sized>(self, key: ArenaKey<T>) -> Option<ArenaRef<'a, T>> {
        let owner = self.owners.get(&key.arena)?;
        Some(owner.scope().resolve(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_bounded_values_and_slices() {
        let builder = ArenaBuilder::new();
        let text = builder.alloc_str("source");
        let values = builder.alloc_slice_copy(&[1_u32, 2, 3]);
        let root = builder.alloc(41_u64);
        let root = builder.finish(root);

        root.with(|value| {
            assert_eq!(*value, 41);
            let scope = root.owner.scope();
            assert_eq!(scope.resolve_str(text), "source");
            assert_eq!(scope.resolve_slice(&values), &[1, 2, 3]);
        });
    }

    #[test]
    #[should_panic(expected = "wrong owner")]
    fn key_cannot_be_resolved_by_another_owner() {
        let first = ArenaBuilder::new();
        let key = first.alloc(1_u32);
        let _first = first.seal();
        let second = ArenaBuilder::new().seal();
        let _ = second.scope().resolve(key);
    }

    #[test]
    fn group_keeps_foreign_owners_alive() {
        let first = ArenaBuilder::new();
        let key = first.alloc(7_u32);
        let first = first.seal();
        let weak = Arc::downgrade(&first);
        let mut group = ArenaGroup::new();
        group.insert(first);

        assert_eq!(*group.scope().resolve(key).unwrap(), 7);
        assert!(weak.upgrade().is_some());
    }

    #[test]
    fn sidecar_drops_before_arena_owner() {
        struct State(std::sync::Weak<SealedArena>);

        impl Drop for State {
            fn drop(&mut self) {
                assert!(
                    self.0.upgrade().is_some(),
                    "arena dropped before its sidecar"
                );
            }
        }

        let builder = ArenaBuilder::new();
        let root = builder.alloc(1_u32);
        let root = builder.finish(root);
        let weak = Arc::downgrade(root.owner());
        drop(OwnedGraph::new(root, State(weak.clone())));
        assert!(weak.upgrade().is_none());
    }
}
