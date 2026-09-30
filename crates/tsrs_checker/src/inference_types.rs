//! Non-function declarations of `inference.go`.

use crate::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct InferenceKey {
    pub s: TypeId,
    pub t: TypeId,
}

/// Arena handle (`P<InferenceState>`), pooled through `Checker::freeinference_state` like Go.
#[derive(Default)]
pub struct InferenceState {
    pub inferences: Cell<&'static [P<InferenceInfo>]>,
    pub original_source: Cell<Option<P<Type>>>,
    pub original_target: Cell<Option<P<Type>>>,
    pub priority: Cell<InferencePriority>,
    pub inference_priority: Cell<InferencePriority>,
    pub contravariant: Cell<bool>,
    pub bivariant: Cell<bool>,
    pub expanding_flags: Cell<ExpandingFlags>,
    pub propagation_type: Cell<Option<P<Type>>>,
    pub visited: GoMap<InferenceKey, InferencePriority>,
    pub source_stack: RefCell<Vec<P<Type>>>,
    pub target_stack: RefCell<Vec<P<Type>>>,
    pub next: Cell<Option<P<InferenceState>>>,
}
