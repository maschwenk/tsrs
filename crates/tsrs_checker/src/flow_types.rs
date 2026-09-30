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
