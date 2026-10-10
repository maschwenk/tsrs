// Not in Go: what can narrow a reference in each flow graph (notes/perf-flow-skip-unnarrowed.md).
//
// A flow graph is the part of a file's control flow that starts at one `FlowFlags::Start` node: one per control flow
// container that is not an immediately invoked function. The binder gives every flow node it makes the index of its
// graph (the bits of `FlowNode::flags` from `FLOW_GRAPH_SHIFT` up) and records, per graph, the reference paths that the
// nodes the checker's flow walk inspects may narrow (conditions, switches, assignments, assertion-candidate calls),
// closed over the variables they may alias, with the earliest position at which each is mentioned. A walk for a
// reference whose path none of the graphs it can pass through mentions where it can reach finds nothing that narrows
// it, and the checker skips it (`Checker::flow_walk_skippable`).

use std::hash::Hasher;
use std::sync::atomic::{AtomicPtr, Ordering::Relaxed};

use rustc_hash::FxHasher;
use tsrs_core::P;

use crate::ast::Node;
use crate::flow::FlowNode;
use crate::identifier::NO_SOURCE_TEXT;

/// The flag bits of `FlowFlags` end below this; a binder flow node keeps its graph index above it (0: no graph).
pub const FLOW_GRAPH_SHIFT: u32 = 13;
/// The largest graph index; later graphs of a file get 0.
pub const FLOW_GRAPH_MAX: u32 = (1 << (32 - FLOW_GRAPH_SHIFT)) - 1;

/// The key of a name in a `FlowNameIndex`: a hash of its text (`this` and `super` for those keywords), odd. Two names
/// with the same key only make a walk run that could have been skipped.
#[inline]
pub fn flow_name_key(text: &str) -> u32 {
    let mut h = FxHasher::default();
    h.write(text.as_bytes());
    let v = h.finish();
    (v ^ (v >> 32)) as u32 | 1
}

/// The key of the reference path `prefix.name` (`name` a `flow_name_key`), or of `prefix[<any name>]` with
/// `FLOW_WILDCARD`; even, so a key of a single name (odd) is told from the key of a longer path.
#[inline]
pub fn flow_path_key(prefix: u32, name: u32) -> u32 {
    (prefix.rotate_left(7) ^ name).wrapping_mul(0x9E37_79B1) & !1
}

/// Whether `key` is the key of a single name (an identifier, `this` or `super`), which only a variable can alias.
#[inline]
pub fn flow_key_is_name(key: u32) -> bool {
    key & 1 != 0
}

/// The name of an element access whose argument is not a string literal: the checker may resolve it to any name.
pub const FLOW_WILDCARD: u32 = 0x5bd1_e995;

/// One flow graph. `FlowNameIndex::names[start..start + len]` (sorted) are the keys of every prefix of every reference
/// path in its conditions and switches and of what those may alias; `names[start + len..start + len + assigned]`
/// (sorted) the exact paths of its assignment targets and declared names; `positions` has, for each, the earliest
/// position of a node that mentions it. Then come `spans` (start, end) pairs: the outermost loops and immediately
/// invoked functions (sorted, disjoint), inside which a walk may reach nodes after its start.
/// `calls[calls_start..calls_start + calls]` (sorted by key, then position) are the paths of its assertion-candidate
/// calls with the calls. `nodes` is how many flow nodes the binder made in it (saturating), `container` what its `Start`
/// node continues to (`FlowNode::node` of that node), if anything. A graph with more of any than a `u16` holds has
/// `len == u16::MAX` and is never skipped.
#[derive(Clone, Copy)]
pub struct FlowGraphNames {
    pub start: u32,
    pub calls_start: u32,
    pub container: Option<P<Node>>,
    pub len: u16,
    pub assigned: u16,
    pub calls: u16,
    pub spans: u16,
    pub nodes: u16,
}

/// A path an assertion-candidate call mentions.
#[derive(Clone, Copy)]
pub struct FlowCallMention {
    pub key: u32,
    pub call: P<Node>,
}

/// A variable of a global script (top level or in a namespace) whose initializer a condition in another file may
/// inline: its name and the paths it may narrow (`FlowNameIndex::names[start..start + len]`).
#[derive(Clone, Copy)]
pub struct FlowGlobalAlias {
    pub name: u32,
    pub start: u32,
    pub len: u32,
}

/// A file's flow graphs, indexed by graph (index 0 unused).
pub struct FlowNameIndex {
    pub graphs: &'static [FlowGraphNames],
    pub names: &'static [u32],
    pub positions: &'static [u32],
    pub calls: &'static [FlowCallMention],
    pub global_aliases: &'static [FlowGlobalAlias],
}

impl FlowNameIndex {
    /// The mentioned keys and the earliest position of each.
    #[inline]
    pub fn graph_names(&self, graph: &FlowGraphNames) -> (&'static [u32], &'static [u32]) {
        let range = graph.start as usize..graph.start as usize + graph.len as usize;
        (&self.names[range.clone()], &self.positions[range])
    }

    /// The assigned keys and the earliest position of each.
    #[inline]
    pub fn graph_assigned(&self, graph: &FlowGraphNames) -> (&'static [u32], &'static [u32]) {
        let start = graph.start as usize + graph.len as usize;
        let range = start..start + graph.assigned as usize;
        (&self.names[range.clone()], &self.positions[range])
    }

    /// The spans, as (start, end) pairs.
    #[inline]
    pub fn graph_spans(&self, graph: &FlowGraphNames) -> &'static [u32] {
        let start = graph.start as usize + graph.len as usize + graph.assigned as usize;
        &self.names[start..start + 2 * graph.spans as usize]
    }

    #[inline]
    pub fn graph_calls(&self, graph: &FlowGraphNames) -> &'static [FlowCallMention] {
        &self.calls[graph.calls_start as usize..graph.calls_start as usize + graph.calls as usize]
    }
}

/// The position below which a node may be reached by a walk that starts at position `pos` of a graph with outermost
/// loop spans `spans` ((start, end) pairs): `pos`, or the end of the loop or immediately invoked function `pos` is in.
/// A walk goes from a node to the nodes the binder made before it, which start before it in the text except across a
/// loop's back edge or into an immediately invoked function's arguments (bound before its body).
#[inline]
pub fn flow_reach_limit(spans: &[u32], pos: u32) -> u32 {
    let n = spans.len() / 2;
    let mut lo = 0;
    let mut hi = n;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if spans[2 * mid] <= pos { lo = mid + 1 } else { hi = mid }
    }
    if lo > 0 && pos < spans[2 * lo - 1] { spans[2 * lo - 1] } else { pos }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlowSkipMode {
    /// No index is built and every walk runs.
    Off,
    On,
    /// Every walk the index would skip runs anyway, and a result other than the declared type aborts the process.
    Shadow,
}

/// `TSRS_FLOW_SKIP`: `0`/`off`, `1` (default) or `shadow`.
pub fn flow_skip_mode() -> FlowSkipMode {
    static MODE: std::sync::OnceLock<FlowSkipMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("TSRS_FLOW_SKIP").as_deref() {
        Ok("0") | Ok("off") => FlowSkipMode::Off,
        Ok("shadow") => FlowSkipMode::Shadow,
        _ => FlowSkipMode::On,
    })
}

const CAP: usize = 1 << 20;

// Indexed by text index (identifier.rs `register_source_text`). Written by the binder of the file once it is bound;
// checkers read it only after the bind phase, which hands them the bound files, so relaxed accesses suffice (as for
// the source texts).
static INDEXES: [AtomicPtr<FlowNameIndex>; CAP] = [const { AtomicPtr::new(std::ptr::null_mut()) }; CAP];

pub fn register_flow_name_index(text_index: u32, index: P<FlowNameIndex>) {
    if (text_index as usize) < CAP {
        // Relaxed: readers get the bound file through the bind phase's synchronization (see `INDEXES`).
        INDEXES[text_index as usize].store(std::ptr::from_ref(index.get()).cast_mut(), Relaxed);
    }
}

/// With `unregister_source_text`: the file's memory is about to be freed.
pub fn unregister_flow_name_index(text_index: u32) {
    if (text_index as usize) < CAP {
        // Relaxed: as for `unregister_source_text`, no checker of a program that still uses the file reads it.
        INDEXES[text_index as usize].store(std::ptr::null_mut(), Relaxed);
    }
}

/// The index registered for a file's text, if any.
pub fn flow_name_index(text_index: u32) -> Option<&'static FlowNameIndex> {
    if text_index as usize >= CAP {
        return None;
    }
    // Relaxed: see `INDEXES`.
    let p = INDEXES[text_index as usize].load(Relaxed);
    // SAFETY: as in `flow_graph_of`.
    (!p.is_null()).then(|| unsafe { &*p })
}

/// The index of the file that made `flow`, and the graph `flow` is in; None for flow nodes without a graph.
#[inline]
pub fn flow_graph_of(flow: P<FlowNode>) -> Option<(&'static FlowNameIndex, &'static FlowGraphNames)> {
    let graph = flow.graph();
    let text_index = flow.text_index();
    if graph == 0 || text_index == NO_SOURCE_TEXT || text_index as usize >= CAP {
        return None;
    }
    // Relaxed: see `INDEXES`.
    let p = INDEXES[text_index as usize].load(Relaxed);
    if p.is_null() {
        return None;
    }
    // SAFETY: the pointer was stored from an arena `&'static FlowNameIndex` by `register_flow_name_index` and is
    // cleared before the file's memory is freed.
    let index = unsafe { &*p };
    index.graphs.get(graph as usize).map(|g| (index, g))
}
