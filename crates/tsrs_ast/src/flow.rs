use std::cell::Cell;

use bitflags::bitflags;
use tsrs_core::{alloc, P};

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

pub struct FlowNode {
    pub flags: Cell<FlowFlags>,
    pub node: Cell<Option<P<Node>>>,            // Associated AST node
    pub antecedent: Cell<Option<P<FlowNode>>>,  // Antecedent for all but FlowLabel
    pub antecedents: Cell<Option<P<FlowList>>>, // Linked list of antecedents for FlowLabel
}

impl FlowNode {
    pub fn flags(&self) -> FlowFlags {
        self.flags.get()
    }
    pub fn node(&self) -> Option<P<Node>> {
        self.node.get()
    }
    pub fn antecedent(&self) -> Option<P<FlowNode>> {
        self.antecedent.get()
    }
    pub fn antecedents(&self) -> Option<P<FlowList>> {
        self.antecedents.get()
    }
}

pub struct FlowList {
    pub flow: P<FlowNode>,
    pub next: Cell<Option<P<FlowList>>>,
}

pub type FlowLabel = FlowNode;

// FlowSwitchClauseData (synthetic AST node for FlowFlagsSwitchClause)

pub struct FlowSwitchClauseData {
    pub switch_statement: P<Node>,
    pub clause_start: i32, // Start index of case/default clause range
    pub clause_end: i32,   // End index of case/default clause range
}

pub fn new_flow_switch_clause_data(switch_statement: P<Node>, clause_start: usize, clause_end: usize) -> P<Node> {
    let node = alloc(FlowSwitchClauseData { switch_statement, clause_start: clause_start as i32, clause_end: clause_end as i32 });
    new_node(Kind::Unknown, NodeData::FlowSwitchClauseData(node), &NodeFactoryHooks::default())
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
    let node = alloc(FlowReduceLabelData { target, antecedents });
    new_node(Kind::Unknown, NodeData::FlowReduceLabelData(node), &NodeFactoryHooks::default())
}
