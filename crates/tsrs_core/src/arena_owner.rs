//! Rust-owned indexed storage for persistent graphs.
//!
//! Edges are typed keys, not pointers. Resolving a key borrows its owner; growing a builder needs an exclusive
//! borrow. Values run their ordinary destructors. Sealed stores inherit `Send` and `Sync` from their values.
//! One builder is one typed table; a graph owns a table for each record type. Slots are never reused.
//! `LocalKey<T>` is a four-byte edge inside a table; `ArenaKey<T>` adds the table's identity for cross-owner edges.
//! Neither key determines the compiler's semantic node/type numbering or iteration order.
//!
//! A reference cannot escape its owner:
//! ```compile_fail
//! use tsrs_core::arena_owner::ArenaBuilder;
//! let value = {
//!     let mut builder = ArenaBuilder::new();
//!     let key = builder.alloc(42);
//!     let owner = builder.seal();
//!     owner.get(key).unwrap()
//! };
//! println!("{value}");
//! ```
//!
//! Allocation cannot invalidate a live borrow:
//! ```compile_fail
//! use tsrs_core::arena_owner::ArenaBuilder;
//! let mut builder = ArenaBuilder::new();
//! let key = builder.alloc(42);
//! let value = builder.get(key).unwrap();
//! builder.alloc(43);
//! println!("{value}");
//! ```
//!
//! Non-`Sync` records cannot be shared between threads:
//! ```compile_fail
//! use std::cell::Cell;
//! use tsrs_core::arena_owner::ArenaBuilder;
//! let mut builder = ArenaBuilder::new();
//! let key = builder.alloc(Cell::new(0));
//! let owner = builder.seal();
//! std::thread::spawn(move || owner.get(key).unwrap().set(1));
//! ```
//!
//! Keys from different record types cannot be mixed:
//! ```compile_fail
//! use tsrs_core::arena_owner::ArenaBuilder;
//! let mut strings = ArenaBuilder::new();
//! let key = strings.alloc(String::from("text"));
//! let numbers = ArenaBuilder::<u32>::new();
//! numbers.get(key);
//! ```

#![forbid(unsafe_code)]

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rustc_hash::FxHashMap;

static NEXT_ARENA_ID: AtomicU32 = AtomicU32::new(1);

/// Process-unique table identity. Exhaustion permanently fails instead of reissuing old identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArenaId(NonZeroU32);

impl ArenaId {
    fn fresh() -> Self {
        // Relaxed: the counter issues identities only; it does not publish graph data.
        let id = NEXT_ARENA_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("arena owner identity space exhausted");
        Self(NonZeroU32::new(id).expect("arena identities start at one"))
    }

    pub fn get(self) -> u32 {
        self.0.get()
    }
}

/// A slot qualified by its owner. Copying a key neither keeps storage alive nor accesses its value.
pub struct ArenaKey<T> {
    arena: ArenaId,
    local: LocalKey<T>,
}

// Eight bytes on all targets (native-pointer width on 64-bit), including optional edges.
const _: () = assert!(std::mem::size_of::<ArenaKey<()>>() == 8);
const _: () = assert!(std::mem::size_of::<Option<ArenaKey<()>>>() == 8);

impl<T> ArenaKey<T> {
    pub fn arena(self) -> ArenaId {
        self.arena
    }

    /// An owner-relative edge. Keep this inside the graph owned by this key's table; use the qualified key
    /// when an edge crosses an ownership boundary.
    pub fn local(self) -> LocalKey<T> {
        self.local
    }
}

impl<T> Clone for ArenaKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for ArenaKey<T> {}
impl<T> PartialEq for ArenaKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.arena == other.arena && self.local == other.local
    }
}
impl<T> Eq for ArenaKey<T> {}
impl<T> Hash for ArenaKey<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.arena.hash(state);
        self.local.hash(state);
    }
}
impl<T> fmt::Debug for ArenaKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArenaKey")
            .field("arena", &self.arena)
            .field("slot", &self.local)
            .finish()
    }
}

/// A compact edge within one typed table. Unlike `ArenaKey`, it carries no owner identity: resolving it against
/// another table of the same type can select an unrelated record. Export qualified keys outside an owner.
pub struct LocalKey<T> {
    slot: NonZeroU32,
    marker: PhantomData<fn() -> T>,
}

const _: () = assert!(std::mem::size_of::<LocalKey<()>>() == 4);
const _: () = assert!(std::mem::size_of::<Option<LocalKey<()>>>() == 4);

impl<T> LocalKey<T> {
    fn index(self) -> usize {
        self.slot.get() as usize - 1
    }
}
impl<T> Copy for LocalKey<T> {}
impl<T> Clone for LocalKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for LocalKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.slot == other.slot
    }
}
impl<T> Eq for LocalKey<T> {}
impl<T> Hash for LocalKey<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.slot.hash(state);
    }
}
impl<T> fmt::Debug for LocalKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.slot.fmt(f)
    }
}

/// Exclusive construction state for a typed graph table. Cycles use keys rather than references into the vector.
pub struct ArenaBuilder<T> {
    id: ArenaId,
    values: Vec<T>,
}

impl<T> ArenaBuilder<T> {
    pub fn new() -> Self {
        Self {
            id: ArenaId::fresh(),
            values: Vec::new(),
        }
    }

    /// Capacity is a number of records, not bytes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            id: ArenaId::fresh(),
            values: Vec::with_capacity(capacity),
        }
    }

    pub fn id(&self) -> ArenaId {
        self.id
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn alloc(&mut self, value: T) -> ArenaKey<T> {
        let index = u32::try_from(self.values.len()).expect("arena slot space exhausted");
        let slot = index
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .expect("arena slot space exhausted");
        self.values.push(value);
        ArenaKey {
            arena: self.id,
            local: LocalKey {
                slot,
                marker: PhantomData,
            },
        }
    }

    pub fn get(&self, key: ArenaKey<T>) -> Option<&T> {
        (key.arena == self.id)
            .then(|| self.get_local(key.local))
            .flatten()
    }

    pub fn get_mut(&mut self, key: ArenaKey<T>) -> Option<&mut T> {
        if key.arena != self.id {
            return None;
        }
        self.values.get_mut(key.local.index())
    }

    /// Resolve an edge stored inside this table's graph.
    pub fn get_local(&self, key: LocalKey<T>) -> Option<&T> {
        self.values.get(key.index())
    }

    /// Recover a qualified key from an owner-maintained numeric index (for compact paged indices).
    /// The index is checked against this table; it does not carry a foreign owner's identity.
    pub fn key_at(&self, index: usize) -> Option<ArenaKey<T>> {
        self.values.get(index).map(|_| ArenaKey {
            arena: self.id,
            local: LocalKey {
                slot: NonZeroU32::new(index as u32 + 1).expect("allocated arena slot"),
                marker: PhantomData,
            },
        })
    }

    /// Qualify an edge from this table's graph for use outside the owner.
    pub fn qualify(&self, key: LocalKey<T>) -> Option<ArenaKey<T>> {
        self.get_local(key).map(|_| ArenaKey {
            arena: self.id,
            local: key,
        })
    }

    pub fn capacity(&self) -> usize {
        self.values.capacity()
    }

    /// Ends construction; no allocation or mutable record access is exposed by the sealed table.
    pub fn seal(self) -> Arc<SealedArena<T>> {
        Arc::new(SealedArena {
            id: self.id,
            values: self.values.into_boxed_slice(),
        })
    }

    pub fn finish(self, root: ArenaKey<T>) -> OwnedRoot<T> {
        assert!(self.get(root).is_some(), "root belongs to another arena");
        OwnedRoot {
            owner: self.seal(),
            root,
        }
    }
}

impl<T> Default for ArenaBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Read-only owned storage. The last owner runs each record's destructor, including resource-owning fields.
pub struct SealedArena<T> {
    id: ArenaId,
    values: Box<[T]>,
}

impl<T> SealedArena<T> {
    pub fn id(&self) -> ArenaId {
        self.id
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Record storage only; allocations owned by fields of `T` are not included.
    pub fn storage_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.values)
    }

    pub fn get(&self, key: ArenaKey<T>) -> Option<&T> {
        (key.arena == self.id)
            .then(|| self.get_local(key.local))
            .flatten()
    }

    /// Resolve an edge stored inside this table's graph.
    pub fn get_local(&self, key: LocalKey<T>) -> Option<&T> {
        self.values.get(key.index())
    }
}

/// A root and the strong owner that keeps every record in its table alive.
pub struct OwnedRoot<T> {
    owner: Arc<SealedArena<T>>,
    root: ArenaKey<T>,
}

impl<T> OwnedRoot<T> {
    pub fn owner(&self) -> &Arc<SealedArena<T>> {
        &self.owner
    }
    pub fn key(&self) -> ArenaKey<T> {
        self.root
    }
    pub fn get(&self) -> &T {
        self.owner
            .get(self.root)
            .expect("owned root belongs to its arena")
    }
}

impl<T> Clone for OwnedRoot<T> {
    fn clone(&self) -> Self {
        Self {
            owner: Arc::clone(&self.owner),
            root: self.root,
        }
    }
}

/// Resource state paired with a root. State is dropped before the graph it may identify.
pub struct OwnedGraph<T, S> {
    state: S,
    root: OwnedRoot<T>,
}

impl<T, S> OwnedGraph<T, S> {
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
}

/// A snapshot's strong references to contributing tables of one record type.
pub struct ArenaGroup<T> {
    owners: FxHashMap<ArenaId, Arc<SealedArena<T>>>,
}

impl<T> ArenaGroup<T> {
    pub fn new() -> Self {
        Self {
            owners: FxHashMap::default(),
        }
    }

    pub fn insert(&mut self, owner: Arc<SealedArena<T>>) {
        self.owners.entry(owner.id).or_insert(owner);
    }

    pub fn get(&self, key: ArenaKey<T>) -> Option<&T> {
        self.owners.get(&key.arena)?.get(key)
    }
}

impl<T> Default for ArenaGroup<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Node {
        text: String,
        next: Option<ArenaKey<Node>>,
    }

    #[test]
    fn cycles_survive_growth_and_sealing() {
        let mut builder = ArenaBuilder::with_capacity(1);
        let first = builder.alloc(Node {
            text: "first".into(),
            next: None,
        });
        let second = builder.alloc(Node {
            text: "second".into(),
            next: Some(first),
        });
        builder.get_mut(first).unwrap().next = Some(second);
        for _ in 0..1024 {
            builder.alloc(Node {
                text: String::new(),
                next: Some(first),
            });
        }
        let root = builder.finish(first);
        let next = root.owner().get(root.get().next.unwrap()).unwrap();
        assert_eq!(next.text, "second");
        assert_eq!(next.next, Some(first));
    }

    #[test]
    fn local_edges_are_compact_and_resolve_after_sealing() {
        struct LocalNode {
            next: Option<LocalKey<LocalNode>>,
        }
        let mut builder = ArenaBuilder::new();
        let first = builder.alloc(LocalNode { next: None });
        let second = builder.alloc(LocalNode {
            next: Some(first.local()),
        });
        builder.get_mut(first).unwrap().next = Some(second.local());
        assert_eq!(builder.qualify(second.local()), Some(second));
        let root = builder.finish(first);
        let next = root.owner().get_local(root.get().next.unwrap()).unwrap();
        assert_eq!(next.next, Some(first.local()));
    }

    #[test]
    fn foreign_and_expired_keys_cannot_resolve() {
        let mut first = ArenaBuilder::new();
        let key = first.alloc(1_u32);
        let mut second = ArenaBuilder::new();
        second.alloc(2_u32);
        assert!(second.get(key).is_none());
        assert!(second.get_mut(key).is_none());
        drop(first);
        assert!(second.seal().get(key).is_none());
        assert!(ArenaGroup::<u32>::new().get(key).is_none());
    }

    struct Dropped(Arc<AtomicUsize>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn snapshots_keep_shared_values_until_last_owner() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut builder = ArenaBuilder::new();
        let key = builder.alloc(Dropped(Arc::clone(&drops)));
        let owner = builder.seal();
        let mut old = ArenaGroup::new();
        old.insert(Arc::clone(&owner));
        let mut new = ArenaGroup::new();
        new.insert(owner);
        drop(old);
        assert!(new.get(key).is_some());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(new);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn abandoned_construction_drops_resources() {
        let drops = Arc::new(AtomicUsize::new(0));
        let result = std::panic::catch_unwind({
            let drops = Arc::clone(&drops);
            move || {
                let mut builder = ArenaBuilder::new();
                builder.alloc(Dropped(drops));
                panic!("cancelled construction");
            }
        });
        assert!(result.is_err());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn concurrent_readers_need_no_custom_thread_traits() {
        let mut builder = ArenaBuilder::new();
        let key = builder.alloc(String::from("shared"));
        let owner = builder.seal();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let owner = &owner;
                scope.spawn(move || assert_eq!(owner.get(key).unwrap(), "shared"));
            }
        });
    }

    #[test]
    fn lazy_graph_is_initialized_once_and_drops_with_its_retained_snapshot() {
        struct File {
            text: String,
            members: std::sync::OnceLock<OwnedRoot<String>>,
        }
        let initializations = AtomicUsize::new(0);
        let mut builder = ArenaBuilder::new();
        let key = builder.alloc(File {
            text: "member".into(),
            members: std::sync::OnceLock::new(),
        });
        let old_snapshot = builder.finish(key);
        let retained_snapshot = old_snapshot.clone();
        let weak_file = Arc::downgrade(old_snapshot.owner());
        drop(old_snapshot);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let file = retained_snapshot.get();
                let initializations = &initializations;
                scope.spawn(move || {
                    let members = file.members.get_or_init(|| {
                        initializations.fetch_add(1, Ordering::SeqCst);
                        let mut builder = ArenaBuilder::new();
                        let key = builder.alloc(file.text.clone());
                        builder.finish(key)
                    });
                    assert_eq!(members.get(), "member");
                });
            }
        });
        assert_eq!(initializations.load(Ordering::SeqCst), 1);
        let weak_members = Arc::downgrade(retained_snapshot.get().members.get().unwrap().owner());
        drop(retained_snapshot);
        assert!(weak_file.upgrade().is_none());
        assert!(weak_members.upgrade().is_none());
    }

    #[test]
    fn sidecar_drops_before_graph() {
        struct State(std::sync::Weak<SealedArena<u32>>);
        impl Drop for State {
            fn drop(&mut self) {
                assert!(self.0.upgrade().is_some());
            }
        }
        let mut builder = ArenaBuilder::new();
        let key = builder.alloc(1_u32);
        let root = builder.finish(key);
        let weak = Arc::downgrade(root.owner());
        drop(OwnedGraph::new(root, State(weak.clone())));
        assert!(weak.upgrade().is_none());
    }
}
