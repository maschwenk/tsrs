//! Non-function declarations of `flow.go`.

use std::sync::LazyLock;

use crate::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FlowType {
    pub t: Option<P<Type>>,
    pub incomplete: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SharedFlow {
    pub flow: P<FlowNode>,
    pub flow_type: FlowType,
    /// Flow memo (flowmemo.rs): the taint sources and flags of the frame that computed the value, and the height of
    /// the sub-walk it stands for.
    pub transient: u32,
    pub reference: u32,
    pub height: u16,
    pub flags: u8,
}

/// Arena handle (`P<FlowState>`), pooled through `Checker::free_flow_state` like Go.
#[derive(Default)]
pub struct FlowState {
    pub reference: Cell<Option<P<Node>>>,
    pub declared_type: Cell<Option<P<Type>>>,
    pub initial_type: Cell<Option<P<Type>>>,
    pub flow_container: Cell<Option<P<Node>>>,
    pub ref_key: Cell<CacheHashKey>,
    pub depth: Cell<i32>,
    pub shared_flow_start: Cell<i32>,
    pub reduce_labels: RefCell<Vec<P<ast::FlowReduceLabelData>>>,
    pub next: Cell<Option<P<FlowState>>>,
    // Flow memo (flowmemo.rs), per walk.
    /// The memo key; `memo_key_state` is 0 before it is computed, 1 if the reference has none, 2 if `memo_key` is it.
    pub memo_key: Cell<u128>,
    pub memo_key_state: Cell<u8>,
    /// Taint source of everything read from the reference node beyond its key.
    pub walk_floor: Cell<u32>,
    /// The walk returned a memo result instead of walking a sub-graph.
    pub memo_used: Cell<bool>,
    /// The walk reached the depth limit after using the memo and unwinds to walk again without it.
    pub memo_aborted: Cell<bool>,
    /// The walk is that second walk: it fills the memo but does not consult it.
    pub memo_faithful: Cell<bool>,
    /// `sharedFlows` entries of this walk whose value is transient.
    pub impure_shared: Cell<u32>,
}

pub static typeofNEFacts: LazyLock<FxHashMap<&'static str, TypeFacts>> = LazyLock::new(|| {
    FxHashMap::from_iter([
        ("string", TypeFacts::TypeofNEString),
        ("number", TypeFacts::TypeofNENumber),
        ("bigint", TypeFacts::TypeofNEBigInt),
        ("boolean", TypeFacts::TypeofNEBoolean),
        ("symbol", TypeFacts::TypeofNESymbol),
        ("undefined", TypeFacts::NEUndefined),
        ("object", TypeFacts::TypeofNEObject),
        ("function", TypeFacts::TypeofNEFunction),
    ])
});

pub const nonDottedNameCacheKey: CacheHashKey = CacheHashKey::hash_string_128("?");
