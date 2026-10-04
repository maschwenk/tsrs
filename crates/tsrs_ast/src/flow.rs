use std::cell::Cell;

use bitflags::bitflags;
use tsrs_core::{alloc, OwnedCell, PKey, P};

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
/// an antecedent) are never both set, so they share one field (`link`): the antecedent's or the list's `P::key`. A
/// list is marked by bit 0 of the key (pointers: an address, 8-aligned) or, with compressed pointers (a handle uses
/// all 32 bits), by bit 31 of `text_index` (text indices are below 2^20 or `NO_SOURCE_TEXT`). 24 bytes (16 with
/// compressed pointers); 1.9M flow nodes on the private monorepo.
pub struct FlowNode {
    pub flags: OwnedCell<FlowFlags>,
    // Not in Go: the text index of the file whose binder made this flow node (`NO_SOURCE_TEXT` for the checker's);
    // compact identifiers read their text index from their flow node (identifier.rs).
    text_index: OwnedCell<u32>,
    pub node: OwnedCell<Option<P<Node>>>, // Associated AST node
    link: OwnedCell<PKey>,
}

const _: () = assert!(std::mem::size_of::<FlowNode>() == if tsrs_core::COMPRESSED_PTRS { 16 } else { 24 });

const FLOW_LINK_LIST: PKey = 1;
const TEXT_LIST_BIT: u32 = 1 << 31;

/// Census builds: the link word is an antecedent or a list tagged with bit 0 (`crate::census_layouts`).
pub(crate) fn census_layout() {
    let off = std::mem::offset_of!(FlowNode, link);
    tsrs_core::census_layout(std::any::type_name::<FlowNode>(), &[tsrs_core::CensusField::LowTag { off, mask: FLOW_LINK_LIST as u8 }]);
}

impl FlowNode {
    pub fn new(flags: FlowFlags, node: Option<P<Node>>, antecedent: Option<P<FlowNode>>, text_index: u32) -> FlowNode {
        let text_index = if tsrs_core::COMPRESSED_PTRS { text_index & !TEXT_LIST_BIT } else { text_index };
        FlowNode {
            flags: OwnedCell::new(flags),
            text_index: OwnedCell::new(text_index),
            node: OwnedCell::new(node),
            link: OwnedCell::new(P::key_opt(antecedent)),
        }
    }
    pub fn flags(&self) -> FlowFlags {
        self.flags.get()
    }
    pub fn node(&self) -> Option<P<Node>> {
        self.node.get()
    }
    #[inline]
    pub fn text_index(&self) -> u32 {
        let t = self.text_index.get();
        if !tsrs_core::COMPRESSED_PTRS {
            return t;
        }
        let t = t & !TEXT_LIST_BIT;
        if t == crate::NO_SOURCE_TEXT & !TEXT_LIST_BIT {
            crate::NO_SOURCE_TEXT
        } else {
            t
        }
    }
    #[inline]
    fn link_is_list(&self) -> bool {
        if tsrs_core::COMPRESSED_PTRS {
            self.text_index.get() & TEXT_LIST_BIT != 0
        } else {
            self.link.get() & FLOW_LINK_LIST != 0
        }
    }
    /// Go `Antecedent` (antecedent for all but FlowLabel).
    #[inline]
    pub fn antecedent(&self) -> Option<P<FlowNode>> {
        // SAFETY: an untagged link is 0 or the antecedent stored by `new`.
        (!self.link_is_list()).then(|| unsafe { P::from_key_opt(self.link.get()) }).flatten()
    }
    /// Go `Antecedents` (linked list of antecedents for FlowLabel).
    #[inline]
    pub fn antecedents(&self) -> Option<P<FlowList>> {
        let key = if tsrs_core::COMPRESSED_PTRS { self.link.get() } else { self.link.get() & !FLOW_LINK_LIST };
        // SAFETY: a tagged link is the list stored by `set_antecedents`.
        self.link_is_list().then(|| unsafe { P::from_key(key) })
    }
    /// Go `Antecedents = list`. Panics on a flow node that has an antecedent (Go never gives one both).
    pub fn set_antecedents(&self, list: Option<P<FlowList>>) {
        assert!(self.antecedent().is_none(), "flow node with both an antecedent and antecedents");
        if tsrs_core::COMPRESSED_PTRS {
            let t = self.text_index.get() & !TEXT_LIST_BIT;
            self.text_index.set(if list.is_some() { t | TEXT_LIST_BIT } else { t });
            self.link.set(P::key_opt(list));
        } else {
            self.link.set(list.map_or(0, |l| l.key() | FLOW_LINK_LIST));
        }
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
