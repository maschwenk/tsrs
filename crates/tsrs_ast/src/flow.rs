use std::cell::Cell;

use bitflags::bitflags;
use tsrs_core::{alloc, OwnedCell, P};

use crate::ast::{new_node, Node, NodeFactoryHooks};
use crate::generated::NodeData;
use crate::kind::Kind;

// FlowFlags

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct FlowFlags: u32 {
        const None = 0;
        const Unreachable = 1 << 0; // Unreachable code
        const Start = 1 << 1; // Start of flow graph
        const BranchLabel = 1 << 2; // Non-looping junction
        const LoopLabel = 1 << 3; // Looping junction
        const Assignment = 1 << 4; // Assignment
        const TrueCondition = 1 << 5; // Condition known to be true
        const FalseCondition = 1 << 6; // Condition known to be false
        const SwitchClause = 1 << 7; // Switch statement clause
        const ArrayMutation = 1 << 8; // Potential array mutation
        const Call = 1 << 9; // Potential assertion call
        const ReduceLabel = 1 << 10; // Temporarily reduce antecedents of label
        const Referenced = 1 << 11; // Referenced as antecedent once
        const Shared = 1 << 12; // Referenced as antecedent more than once
        const Label = Self::BranchLabel.bits() | Self::LoopLabel.bits();
        const Condition = Self::TrueCondition.bits() | Self::FalseCondition.bits();
    }
}

// FlowNode

/// Go's `Antecedent` (all but labels; set at creation) and `Antecedents` (labels; Go's binder creates labels without
/// an antecedent) are never both set, so they share one word (`link`): the antecedent, or the list with bit 0 set.
/// 24 bytes; 1.9M flow nodes on the private monorepo.
pub struct FlowNode {
    pub flags: OwnedCell<FlowFlags>,
    // Not in Go: the text index of the file whose binder made this flow node (`NO_SOURCE_TEXT` for the checker's);
    // compact identifiers read their text index from their flow node (identifier.rs).
    pub text_index: u32,
    pub node: OwnedCell<Option<P<Node>>>, // Associated AST node
    link: OwnedCell<*const ()>,
}

const _: () = assert!(std::mem::size_of::<FlowNode>() == 24);

const FLOW_LINK_LIST: usize = 1;

impl FlowNode {
    pub fn new(flags: FlowFlags, node: Option<P<Node>>, antecedent: Option<P<FlowNode>>, text_index: u32) -> FlowNode {
        let link = antecedent.map_or(std::ptr::null(), |a| (a.get() as *const FlowNode).cast::<()>());
        FlowNode { flags: OwnedCell::new(flags), text_index, node: OwnedCell::new(node), link: OwnedCell::new(link) }
    }
    pub fn flags(&self) -> FlowFlags {
        self.flags.get()
    }
    pub fn node(&self) -> Option<P<Node>> {
        self.node.get()
    }
    /// Go `Antecedent` (antecedent for all but FlowLabel).
    #[inline]
    pub fn antecedent(&self) -> Option<P<FlowNode>> {
        let p = self.link.get();
        // SAFETY: an untagged non-null link is the antecedent stored by `new`.
        (!p.is_null() && p.addr() & FLOW_LINK_LIST == 0).then(|| P::from_static(unsafe { &*p.cast::<FlowNode>() }))
    }
    /// Go `Antecedents` (linked list of antecedents for FlowLabel).
    #[inline]
    pub fn antecedents(&self) -> Option<P<FlowList>> {
        let p = self.link.get();
        // SAFETY: a tagged link is the list stored by `set_antecedents`.
        (p.addr() & FLOW_LINK_LIST != 0).then(|| P::from_static(unsafe { &*p.map_addr(|a| a & !FLOW_LINK_LIST).cast::<FlowList>() }))
    }
    /// Go `Antecedents = list`. Panics on a flow node that has an antecedent (Go never gives one both).
    pub fn set_antecedents(&self, list: Option<P<FlowList>>) {
        assert!(self.antecedent().is_none(), "flow node with both an antecedent and antecedents");
        self.link.set(list.map_or(std::ptr::null(), |l| (l.get() as *const FlowList).cast::<()>().map_addr(|a| a | FLOW_LINK_LIST)));
    }
}

pub struct FlowList {
    pub flow: P<FlowNode>,
    pub next: OwnedCell<Option<P<FlowList>>>,
}

pub type FlowLabel = FlowNode;

// FlowSwitchClauseData (synthetic AST node for FlowFlagsSwitchClause)

pub struct FlowSwitchClauseData {
    pub switch_statement: P<Node>,
    pub clause_start: i32, // Start index of case/default clause range
    pub clause_end: i32,   // End index of case/default clause range
}

pub fn new_flow_switch_clause_data(switch_statement: P<Node>, clause_start: usize, clause_end: usize) -> P<Node> {
    let data = FlowSwitchClauseData { switch_statement, clause_start: clause_start as i32, clause_end: clause_end as i32 };
    new_node(Kind::Unknown, data, &NodeFactoryHooks::default())
}

impl FlowSwitchClauseData {
    pub fn is_empty(&self) -> bool {
        self.clause_start == self.clause_end
    }
}

// FlowReduceLabelData (synthetic AST node for FlowFlagsReduceLabel)

pub struct FlowReduceLabelData {
    pub target: P<FlowLabel>,             // Target label
    pub antecedents: Option<P<FlowList>>, // Temporary antecedent list
}

pub fn new_flow_reduce_label_data(target: P<FlowLabel>, antecedents: Option<P<FlowList>>) -> P<Node> {
    new_node(Kind::Unknown, FlowReduceLabelData { target, antecedents }, &NodeFactoryHooks::default())
}
