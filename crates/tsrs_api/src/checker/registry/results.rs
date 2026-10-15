use std::fmt;

use tsrs_checker::{Signature, Type};
use tsrs_compiler::ProgramLease;
use tsrs_core::P;

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
