
use bitflags::bitflags;
use tsrs_core::{OwnedCell, PKey, P};

use crate::ast::{new_node, Node, NodeFactoryHooks};
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
/// an antecedent and gives antecedents only to labels) are never both set, so they share one field (`link`): a node
/// whose flags have `Label` holds its antecedents, any other node the antecedent's `P::key`. Label bits never change
/// after creation (only `Referenced` / `Shared` are added). 24 bytes, 16 with compressed pointers; 1.9M flow nodes on
/// the private monorepo.
///
/// A label's antecedents are a run in the per-file edge array the binder writes when it finishes the file
/// (`FlowEdgeRun`, notes/mem-flow-compaction.md); while the file is being bound the link holds the binder's own slot
/// for the label instead (`label_slot`), and only the binder reads it.
pub struct FlowNode {
    pub flags: OwnedCell<FlowFlags>,
    // Not in Go: the text index of the file whose binder made this flow node (`NO_SOURCE_TEXT` for the checker's);
    // compact identifiers read their text index from their flow node (identifier.rs).
    pub text_index: u32,
    pub node: OwnedCell<Option<P<Node>>>, // Associated AST node
    link: OwnedCell<PKey>,
}

const _: () = assert!(std::mem::size_of::<FlowNode>() == if tsrs_core::COMPRESSED_PTRS { 16 } else { 24 });

/// Census builds: the link word is a plain pointer (`crate::census_layouts`): the antecedent, or the label's edge run
/// (an interior pointer into the file's edge array).
pub(crate) fn census_layout() {}

impl FlowNode {
    pub fn new(flags: FlowFlags, node: Option<P<Node>>, antecedent: Option<P<FlowNode>>, text_index: u32) -> FlowNode {
        assert!(antecedent.is_none() || !flags.intersects(FlowFlags::Label), "flow label created with an antecedent");
        FlowNode { flags: OwnedCell::new(flags), text_index, node: OwnedCell::new(node), link: OwnedCell::new(P::key_opt(antecedent)) }
    }
    pub fn flags(&self) -> FlowFlags {
        self.flags.get()
    }
    pub fn node(&self) -> Option<P<Node>> {
        self.node.get()
    }
    #[inline]
    pub fn text_index(&self) -> u32 {
        self.text_index
    }
    /// Go `Antecedent` (antecedent for all but FlowLabel).
    #[inline]
    pub fn antecedent(&self) -> Option<P<FlowNode>> {
        // SAFETY: a non-label's link is 0 or the antecedent stored by `new` / `replace_antecedent`.
        (!self.flags.get().intersects(FlowFlags::Label)).then(|| unsafe { P::from_key_opt(self.link.get()) }).flatten()
    }
    /// Points a non-label at `antecedent` instead. Not in Go: the binder gives a bodyless signature its own Start
    /// when the file's shared one turned out to be referenced (binder.rs `unshare_flow_start`).
    pub fn replace_antecedent(&self, antecedent: P<FlowNode>) {
        assert!(!self.flags.get().intersects(FlowFlags::Label), "antecedent on a flow label");
        self.link.set(antecedent.key());
    }
    /// Go `Antecedents` (antecedents of a FlowLabel, in the order the binder added them; empty for Go's nil and for
    /// a node that is not a label). Only for bound files: see `label_slot`.
    #[inline]
    pub fn antecedents(&self) -> &'static [P<FlowNode>] {
        if !self.flags.get().intersects(FlowFlags::Label) {
            return &[];
        }
        // SAFETY: a bound file's label link is 0 or the run stored by `set_antecedent_run`.
        match unsafe { P::<FlowEdgeRun>::from_key_opt(self.link.get()) } {
            Some(run) => run.get().edges(),
            None => &[],
        }
    }
    /// The binder's slot for this label while its file is being bound (0 until the binder gives it one).
    pub fn label_slot(&self) -> PKey {
        debug_assert!(self.flags.get().intersects(FlowFlags::Label));
        self.link.get()
    }
    pub fn set_label_slot(&self, slot: PKey) {
        assert!(self.flags.get().intersects(FlowFlags::Label), "antecedents on a flow node that is not a label");
        self.link.set(slot);
    }
    /// Go `Antecedents = list`, done once per label when the binder finishes the file.
    pub fn set_antecedent_run(&self, run: Option<P<FlowEdgeRun>>) {
        assert!(self.flags.get().intersects(FlowFlags::Label), "antecedents on a flow node that is not a label");
        self.link.set(P::key_opt(run));
    }
}

/// The antecedents of one flow label: a length word followed by that many `P<FlowNode>`, inside the file's edge array
/// (`FlowEdgeWord`). Replaces Go's linked `FlowList` (8 bytes a cell with compressed pointers) with 4 bytes an edge
/// plus a length word, and padding to the 8-byte alignment a `P` target needs.
#[repr(C)]
pub struct FlowEdgeRun {
    len: PKey,
}

impl FlowEdgeRun {
    #[inline]
    pub fn edges(&'static self) -> &'static [P<FlowNode>] {
        // SAFETY: `write_flow_edge_runs` wrote `len` flow node keys (each a valid `P<FlowNode>`, which is a
        // transparent non-zero key) right after the length word, in the same arena block.
        unsafe { std::slice::from_raw_parts(std::ptr::from_ref(self).add(1).cast::<P<FlowNode>>(), self.len as usize) }
    }
}

/// One 8-byte word of a file's flow edge array (its own type so the allocation profile shows the array as a row).
#[derive(Clone, Copy)]
pub struct FlowEdgeWord(#[expect(dead_code, reason = "read through `FlowEdgeRun`, never as a word")] u64);

/// Writes the antecedent runs of a file's labels into one arena array and points each label at its run. `labels`
/// holds each label with its antecedents in order; a label with none keeps no run.
pub fn write_flow_edge_runs(labels: &[(P<FlowNode>, &[P<FlowNode>])]) {
    const KEY: usize = std::mem::size_of::<PKey>();
    // Runs start 8-aligned (a `P` target): pad to an even number of words with compressed (4-byte) keys.
    let pad = |n: usize| (n * KEY).next_multiple_of(8) / KEY;
    let total: usize = labels.iter().filter(|(_, e)| !e.is_empty()).map(|(_, e)| pad(1 + e.len())).sum();
    if total == 0 {
        return;
    }
    let mut keys: Vec<PKey> = Vec::with_capacity(total);
    for (_, edges) in labels.iter().filter(|(_, e)| !e.is_empty()) {
        let start = keys.len();
        keys.push(edges.len() as PKey);
        keys.extend(edges.iter().map(|e| e.key()));
        keys.resize(start + pad(1 + edges.len()), 0);
    }
    let words: Vec<FlowEdgeWord> = keys
        .as_slice()
        .chunks_exact(8 / KEY)
        .map(|c| {
            let mut b = [0u8; 8];
            for (i, k) in c.iter().enumerate() {
                b[i * KEY..(i + 1) * KEY].copy_from_slice(&k.to_ne_bytes());
            }
            FlowEdgeWord(u64::from_ne_bytes(b))
        })
        .collect();
    let array = tsrs_core::alloc_slice(&words);
    let mut at = 0;
    for &(label, edges) in labels {
        if edges.is_empty() {
            continue;
        }
        // SAFETY: a word of the fresh arena array (runs are padded to whole words) where a run starts.
        let run = unsafe { P::from_arena(&*array.as_ptr().add(at * KEY / 8).cast::<FlowEdgeRun>()) };
        label.set_antecedent_run(Some(run));
        at += pad(1 + edges.len());
    }
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

/// Go keeps the antecedent list of the label the reduce label was made from (`antecedents`): the binder only appends
/// to a label's list, and these labels get no antecedents after the reduce label is made, so that label's final
/// antecedents are the list Go reads.
pub struct FlowReduceLabelData {
    pub target: P<FlowLabel>,      // Target label
    pub antecedents: P<FlowLabel>, // Label whose antecedents are the temporary antecedent list
}

impl FlowReduceLabelData {
    /// Go `Antecedents` (the temporary antecedent list).
    pub fn antecedents(&self) -> &'static [P<FlowNode>] {
        self.antecedents.antecedents()
    }
}

pub fn new_flow_reduce_label_data(target: P<FlowLabel>, antecedents: P<FlowLabel>) -> P<Node> {
    new_node(Kind::Unknown, FlowReduceLabelData { target, antecedents }, &NodeFactoryHooks::default())
}
