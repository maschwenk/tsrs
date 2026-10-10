//! Non-function declarations of `inference.go`.

use crate::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct InferenceKey {
    pub s: TypeId,
    pub t: TypeId,
}

/// Qualified scratch-state edge; records belong to the checker.
pub type InferenceStateKey = tsrs_core::arena_owner::ArenaKey<InferenceState>;

/// Rust-owned scratch record, reused in LIFO order through a vector of keys.
#[derive(Default)]
pub struct InferenceState {
    pub inferences: RefCell<Vec<InferenceInfoKey>>,
    pub original_source: Cell<Option<P<Type>>>,
    pub original_target: Cell<Option<P<Type>>>,
    pub priority: Cell<InferencePriority>,
    pub inference_priority: Cell<InferencePriority>,
    pub contravariant: Cell<bool>,
    pub bivariant: Cell<bool>,
    pub expanding_flags: Cell<ExpandingFlags>,
    pub propagation_type: Cell<Option<P<Type>>>,
    pub visited: OwnedMap<InferenceKey, InferencePriority>,
    pub source_stack: RefCell<Vec<P<Type>>>,
    pub target_stack: RefCell<Vec<P<Type>>>,
    /// The walk called `clear_cached_inferences` (the inference memo, infermemo.rs).
    pub cleared_inferences: Cell<bool>,
}
