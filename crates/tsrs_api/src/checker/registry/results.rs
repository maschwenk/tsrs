use std::fmt;
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;
use tsrs_ast::Symbol;
use tsrs_checker::{Signature, Type};
use tsrs_compiler::ProgramLease;
use tsrs_core::P;
use tsrs_project::{CheckerDataLease, Snapshot, SnapshotLease};

/// Symbols are snapshot-wide and can come from several projects and auto-import snapshots. Keep
/// those owners together, independently of the registry maps, so resolved handles cannot form a
/// cycle back to the registry. Release temporary checker data before dropping its program owners.
#[derive(Default)]
pub(in crate::checker) struct SymbolOwner {
    pub(in crate::checker) checker_data: CheckerDataLease,
    snapshots: Mutex<FxHashMap<usize, SnapshotLease>>,
}

impl SymbolOwner {
    pub(super) fn retain_snapshot(&self, snapshot: &Arc<Snapshot>) {
        let mut snapshots = self.snapshots.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // IDs are local to a snapshot host. The retained Arc makes object identity stable here.
        snapshots.entry(Arc::as_ptr(snapshot).addr()).or_insert_with(|| snapshot.lease());
    }
}

/// A registered symbol retains all source snapshots and checker data admitted to its registry.
/// It deliberately does not expose the graph's unowned pointers through `Deref`.
#[derive(Clone)]
pub struct SymbolHandle {
    pub(super) value: P<Symbol>,
    pub(super) owner: Arc<SymbolOwner>,
}

impl SymbolHandle {
    pub fn id(&self) -> u64 { tsrs_ast::get_symbol_id(self.value).0 }

    pub(in crate::checker) fn belongs_to(&self, owner: &Arc<SymbolOwner>) -> bool { Arc::ptr_eq(&self.owner, owner) }

    /// # Safety
    /// Keep this handle or its symbol owner alive for every use of the pointer and its graph.
    /// Mutating checker operations still require the appropriate checker to be acquired exclusively.
    pub(in crate::checker) unsafe fn as_raw(&self) -> P<Symbol> { self.value }
}

impl fmt::Debug for SymbolHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SymbolHandle").field("id", &self.id()).finish_non_exhaustive()
    }
}

/// The persistent API checker is pinned until its program owner drops, even after cancellation.
/// Temporary language-service checkers do not have this guarantee.
#[derive(Clone)]
pub(in crate::checker) struct ResultOwner {
    program: ProgramLease,
    pub(in crate::checker) checker_id: u32,
}

impl ResultOwner {
    /// # Safety
    /// `checker_id` must identify a persistent API checker belonging to `program`, whose results
    /// remain allocated until the last program lease drops (including after checker replacement).
    pub(in crate::checker) unsafe fn new(program: &ProgramLease, checker_id: u32) -> Self {
        Self { program: program.clone(), checker_id }
    }

    pub(in crate::checker) fn matches(&self, other: &Self) -> bool {
        self.checker_id == other.checker_id && std::ptr::eq(self.program.program(), other.program.program())
    }
}

/// A registered type that keeps its program and checker region alive independently of the snapshot.
/// It exposes scalar metadata only: legacy `Type` accessors can return unowned arena pointers.
#[derive(Clone)]
pub struct TypeHandle {
    pub(super) value: P<Type>,
    pub(super) owner: ResultOwner,
}

impl TypeHandle {
    pub fn id(&self) -> u32 { self.value.id().0 }

    pub(in crate::checker) fn belongs_to(&self, owner: &ResultOwner) -> bool { self.owner.matches(owner) }

    /// # Safety
    /// Keep this handle or a matching result owner alive for every use of the pointer and its graph.
    /// Checker operations additionally require the originating checker to be acquired exclusively.
    pub(in crate::checker) unsafe fn as_raw(&self) -> P<Type> { self.value }
}

impl fmt::Debug for TypeHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypeHandle").field("id", &self.id()).field("checker_id", &self.owner.checker_id).finish_non_exhaustive()
    }
}

/// A registered signature with the same ownership boundary as [`TypeHandle`]. Following its target
/// retains the original checker owner; it never adopts the registry's current checker identity.
#[derive(Clone)]
pub struct SignatureHandle {
    pub(super) value: P<Signature>,
    pub(super) owner: ResultOwner,
}

impl SignatureHandle {
    pub fn id(&self) -> u64 { self.value.id().0 as u64 }

    pub fn target(&self) -> Option<Self> {
        self.value.target().map(|value| Self { value, owner: self.owner.clone() })
    }

    pub(in crate::checker) fn belongs_to(&self, owner: &ResultOwner) -> bool { self.owner.matches(owner) }

    /// # Safety
    /// Keep this handle or a matching result owner alive for every use of the pointer and its graph.
    /// Checker operations additionally require the originating checker to be acquired exclusively.
    pub(in crate::checker) unsafe fn as_raw(&self) -> P<Signature> { self.value }
}

impl fmt::Debug for SignatureHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignatureHandle").field("id", &self.id()).field("checker_id", &self.owner.checker_id).finish_non_exhaustive()
    }
}
