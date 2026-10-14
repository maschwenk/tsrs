// Identifier node data (written by hand; tools/gen-ast/gen-ast.ts skips it).
//
// Go's `Identifier` holds `Text` and `FlowNodeBase.FlowNode`. Almost every identifier is made by the parser from a
// token whose text is the slice of its file's text that ends where the identifier ends (on the private monorepo
// 7.86M of 8.15M; the rest are missing identifiers, clones and synthesized names). Such an identifier keeps only the
// text's length and the file's text index (`register_source_text`): the text is `source_text(index)[end - len..end]`
// with `end` from the node's range. With compressed pointers (the default on unix) the flow node is a 32-bit handle,
// and the word holds the handle, the index and the length side by side. With plain pointers the flow node takes 45
// bits, so the index sits in its bits until the binder sets one; from then on it is read from the flow node
// (`FlowNode::text_index`: the binder made that flow node for the same file). Either way flow node and text fit one
// word, and the node is 32 bytes instead of 40. Every other identifier stores its text after the word
// (`IdentifierWithText`, 40 bytes as before).
//
// `text()` returns exactly the slice the parser had (same pointer and length), so nothing observable changes. The
// range end of a compact identifier is fixed when it is created: `Node::set_loc` panics if it would change.

use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::{LazyLock, Mutex};

use rustc_hash::FxHashMap;
use tsrs_core::{OwnedCell, PackedStr, TextRange, P};

use crate::ast::{clone_node, Node, NodeAlloc, NodeFactory, NodePayload};
use crate::generated::NodeDataTag;
use crate::flow::FlowNode;
use crate::kind::Kind;

/// No source text: the text index of flow nodes the checker makes, and of parses whose text is not registered.
pub const NO_SOURCE_TEXT: u32 = u32::MAX;

const SOURCE_TEXTS_CAP: usize = 1 << 20;

// Zero-initialized, so untouched pages cost nothing. Addresses and lengths are separate arrays, so `text()` reads an
// address with one scaled load. A text is registered by the thread that parses it, before any identifier of it exists;
// other threads reach those identifiers only through the synchronization that hands them the AST (end of the parse
// phase, `jsdoc_mu`), so relaxed accesses suffice.
static SOURCE_TEXT_PTRS: [AtomicPtr<u8>; SOURCE_TEXTS_CAP] = [const { AtomicPtr::new(std::ptr::null_mut()) }; SOURCE_TEXTS_CAP];
static SOURCE_TEXT_LENS: [AtomicUsize; SOURCE_TEXTS_CAP] = [const { AtomicUsize::new(0) }; SOURCE_TEXTS_CAP];
static SOURCE_TEXT_COUNT: AtomicU32 = AtomicU32::new(0);

/// Registers a file's text for compact identifiers; `NO_SOURCE_TEXT` once the table is full (identifiers of later
/// files then store their text).
pub fn register_source_text(text: &'static str) -> u32 {
    let Ok(index) = SOURCE_TEXT_COUNT.try_update(Relaxed, Relaxed, |n| {
        ((n as usize) < SOURCE_TEXTS_CAP).then(|| n + 1)
    }) else {
        return NO_SOURCE_TEXT;
    };
    // Relaxed: readers reach the slot through the AST hand-off (the statics' comment).
    SOURCE_TEXT_PTRS[index as usize].store(text.as_ptr().cast_mut(), Relaxed);
    SOURCE_TEXT_LENS[index as usize].store(text.len(), Relaxed);
    index
}

// Forgets the text of a file whose memory is about to be freed (language server regions): the table must not keep a
// pointer into freed memory. The index is not reused.
pub fn unregister_source_text(index: u32) {
    if (index as usize) < SOURCE_TEXTS_CAP {
        // Relaxed: no identifier of the freed file is read again (the statics' comment).
        SOURCE_TEXT_LENS[index as usize].store(0, Relaxed);
        SOURCE_TEXT_PTRS[index as usize].store(std::ptr::null_mut(), Relaxed);
    }
}

/// The address of a registered text. The index is masked into the table, so the read needs no bounds check: an index
/// a compact identifier holds always came from `register_source_text`.
#[inline]
fn source_text_ptr(index: u32) -> *const u8 {
    // Relaxed: the slot was written before the AST that holds the index was handed to this thread (the statics' comment).
    debug_assert!(index < SOURCE_TEXT_COUNT.load(Relaxed));
    SOURCE_TEXT_PTRS[index as usize & (SOURCE_TEXTS_CAP - 1)].load(Relaxed)
}

#[inline]
#[expect(
    clippy::disallowed_methods,
    reason = "from_utf8 re-validates the whole file text on every call: 291 G -> 1,247 G instructions, one checker (notes/lint-paydown-compiler.md)"
)]
fn source_text(index: u32) -> &'static str {
    // Relaxed: as in `source_text_ptr`.
    let (ptr, len) = (SOURCE_TEXT_PTRS[index as usize].load(Relaxed), SOURCE_TEXT_LENS[index as usize].load(Relaxed));
    // SAFETY: the slot was stored from a `&'static str` by `register_source_text` (see the statics' comment).
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }
}

// The word. Compressed pointers: bits 0..32 the flow node's handle (`P::pack`; 0 = none), bits 32..52 the text index,
// bits 52..62 the text length, bits 62..64 the mode (`MODE_SOURCE` or `MODE_TEXT`). A text is found with no load
// but the word, the node's end and the text's address, and setting a flow node writes the low half. Plain pointers:
// bits 0..45 the flow node (`P::pack`, like the node header's parent) or the text index, bits 45..62 the text length,
// bits 62..64 the mode.
const INDEX_IN_WORD: bool = tsrs_core::COMPRESSED_PTRS;
const SLOT_MASK: u64 = if INDEX_IN_WORD { u32::MAX as u64 } else { (1 << tsrs_core::PACK_BITS) - 1 };
const INDEX_SHIFT: u32 = 32;
const LEN_SHIFT: u32 = if INDEX_IN_WORD { 52 } else { tsrs_core::PACK_BITS };
const MODE_SHIFT: u32 = 62;
/// Longer identifiers store their text (1,023 bytes with compressed pointers).
const LEN_MAX: u64 = (1 << (MODE_SHIFT - LEN_SHIFT)) - 1;
const _: () = assert!(!INDEX_IN_WORD || (INDEX_SHIFT as usize + SOURCE_TEXTS_CAP.trailing_zeros() as usize) <= LEN_SHIFT as usize);
/// Text from the source. Compressed pointers: the slot is the flow node. Plain pointers: the slot is the text index,
/// there is no flow node.
const MODE_SOURCE: u64 = 0;
/// Plain pointers only: text from the source; the slot is the flow node, which holds the text index.
const MODE_SOURCE_FLOW: u64 = 1;
/// Text stored after the word (`IdentifierWithText`); the slot is the flow node (0 = none).
const MODE_TEXT: u64 = 2;
/// Plain pointers only: text from the source; the slot is the text index and the flow node (made for another text) is
/// in `SIDE_FLOW`.
const MODE_SOURCE_SIDE: u64 = 3;
// With plain pointers `text()` sends the two rare modes to its cold path with one test of the top bit.
const _: () = assert!(MODE_SOURCE >> 1 == 0 && MODE_SOURCE_FLOW >> 1 == 0 && MODE_TEXT >> 1 == 1 && MODE_SOURCE_SIDE >> 1 == 1);

// Flow nodes of compact identifiers whose text index differs from the identifier's, keyed by the identifier's
// address. The binder only sets flow nodes it made for the file it binds, so this stays empty in practice.
static SIDE_FLOW: LazyLock<Mutex<FxHashMap<usize, P<FlowNode>>>> = LazyLock::new(Default::default);

pub struct Identifier {
    word: OwnedCell<u64>,
}

#[repr(C)]
struct IdentifierWithText {
    identifier: Identifier,
    text: PackedStr,
}

impl NodePayload for IdentifierWithText {
    const TAG: NodeDataTag = NodeDataTag::Identifier;
}

const _: () = assert!(std::mem::size_of::<NodeAlloc<Identifier>>() == 32);

/// TOOL (`--features ast-sizing` only): arena bytes of an identifier node with its text stored.
#[cfg(feature = "ast-sizing")]
pub(crate) const IDENTIFIER_WITH_TEXT_SIZE: usize = std::mem::size_of::<NodeAlloc<IdentifierWithText>>();

/// TOOL (`--features ast-sizing` only): arena bytes of the identifier node `n`.
#[cfg(feature = "ast-sizing")]
pub(crate) fn identifier_alloc_size(n: &Node) -> usize {
    if n.payload::<Identifier>().is_source_text() {
        std::mem::size_of::<NodeAlloc<Identifier>>()
    } else {
        IDENTIFIER_WITH_TEXT_SIZE
    }
}

/// Census builds: the identifier word holds a flow node's address / 8 in the modes that keep one
/// (`crate::census_layouts`).
pub(crate) fn census_layout() {
    use std::mem::offset_of;
    let word = offset_of!(Identifier, word);
    let field = |data: usize| tsrs_core::CensusField::X8 { off: data + word, modes: (1 << MODE_SOURCE_FLOW) | (1 << MODE_TEXT) };
    tsrs_core::census_layout(std::any::type_name::<NodeAlloc<Identifier>>(), &[field(offset_of!(NodeAlloc<Identifier>, data))]);
    let data = offset_of!(NodeAlloc<IdentifierWithText>, data) + offset_of!(IdentifierWithText, identifier);
    tsrs_core::census_layout(std::any::type_name::<NodeAlloc<IdentifierWithText>>(), &[field(data)]);
}

#[inline]
fn flow_slot(flow: Option<P<FlowNode>>) -> u64 {
    P::pack_opt(flow)
}

#[inline]
fn slot_flow(word: u64) -> Option<P<FlowNode>> {
    // SAFETY: a nonzero flow slot was stored by `flow_slot` from a live `P<FlowNode>` (arena objects are never moved;
    // a flow node is recycled only when nothing references it).
    unsafe { P::unpack_opt(word) }
}

impl Identifier {
    #[inline]
    fn node(&self) -> &Node {
        // SAFETY: identifiers are created only by `new_node` inside a `NodeAlloc` (`repr(C)`, header first), so the
        // header lies at this offset before the data struct.
        unsafe { tsrs_core::ptr::arena_neighbor(self, -(std::mem::offset_of!(NodeAlloc<Identifier>, data) as isize)) }
    }

    #[inline]
    fn mode(word: u64) -> u64 {
        word >> MODE_SHIFT
    }

    /// Whether the text is stored after the word (`MODE_TEXT`). With compressed pointers the only other mode is
    /// `MODE_SOURCE`, so this is one bit test.
    #[inline]
    fn is_stored(word: u64) -> bool {
        if INDEX_IN_WORD {
            debug_assert!(matches!(Self::mode(word), MODE_SOURCE | MODE_TEXT));
            return word >> 63 != 0;
        }
        Self::mode(word) == MODE_TEXT
    }

    /// The text index of a compact identifier (not `MODE_TEXT`).
    #[inline]
    fn text_index(word: u64) -> u32 {
        if INDEX_IN_WORD {
            return (word >> INDEX_SHIFT) as u32 & (SOURCE_TEXTS_CAP - 1) as u32;
        }
        if Self::mode(word) == MODE_SOURCE_FLOW {
            slot_flow(word).unwrap().text_index()
        } else {
            (word & SLOT_MASK) as u32
        }
    }

    /// The text. Inlined into every caller (`Node::text` has about 750 call sites): with compressed pointers a
    /// compact identifier's text is the word, the node's end and one load of the text's address. Out of line, with
    /// a frame and the checks, it cost 36 instructions a call (vscode, one checker: 33.9 M calls, 1.6% of all
    /// instructions). Stored text (and, with plain pointers, side-table flow nodes) takes the cold path.
    #[expect(clippy::inline_always, reason = "measured: with #[inline] LLVM kept 14 out-of-line copies of `Node::text`")]
    #[inline(always)]
    pub fn text(&self) -> &'static str {
        let word = self.word.get();
        if INDEX_IN_WORD {
            if Self::is_stored(word) {
                return self.stored_text();
            }
            return self.source_slice(word, Self::text_index(word));
        }
        if Self::mode(word) >> 1 != 0 {
            return self.text_cold(word);
        }
        let index = if Self::mode(word) == MODE_SOURCE_FLOW {
            // SAFETY: `set_flow_node` stores `MODE_SOURCE_FLOW` only with a flow node in the slot.
            unsafe { P::<FlowNode>::unpack(word) }.text_index()
        } else {
            (word & SLOT_MASK) as u32
        };
        self.source_slice(word, index)
    }

    /// `text()` in `MODE_TEXT` and `MODE_SOURCE_SIDE`.
    #[cold]
    #[inline(never)]
    fn text_cold(&self, word: u64) -> &'static str {
        if Self::mode(word) == MODE_TEXT {
            return self.stored_text();
        }
        self.source_slice(word, Self::text_index(word))
    }

    /// The text of a compact identifier whose text index is `index`.
    #[expect(clippy::inline_always, reason = "measured: with #[inline] LLVM kept 14 out-of-line copies of `Node::text`")]
    #[inline(always)]
    #[expect(clippy::disallowed_methods, reason = "from_utf8 here: +1.6% instructions, one checker (notes/lint-paydown-compiler.md)")]
    fn source_slice(&self, word: u64, index: u32) -> &'static str {
        let len = ((word >> LEN_SHIFT) & LEN_MAX) as usize;
        let end = self
            .node()
            .end()
            .expect_source("source identifiers have source positions") as usize;
        debug_assert!(end <= source_text(index).len() && len <= end);
        // SAFETY: `new_source_identifier` checked that `end - len..end` is the token's text inside the registered
        // text (so in bounds and on char boundaries), and `Node::set_loc` keeps the end from changing (the range of a
        // node is written only there, or by the factory when it creates a node).
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(source_text_ptr(index).add(end - len), len)) }
    }

    /// `text().len()`, without forming the text (a compact identifier keeps its length in the word).
    #[inline]
    pub fn text_len(&self) -> usize {
        let word = self.word.get();
        if Self::is_stored(word) {
            return self.stored_text().len();
        }
        ((word >> LEN_SHIFT) & LEN_MAX) as usize
    }

    #[cold]
    fn stored_text(&self) -> &'static str {
        // SAFETY: identifiers in `MODE_TEXT` were allocated as `IdentifierWithText` (`repr(C)`, identifier first).
        unsafe { tsrs_core::ptr::arena_neighbor::<_, IdentifierWithText>(self, 0) }.text.as_str()
    }

    #[inline]
    pub fn flow_node(&self) -> Option<P<FlowNode>> {
        let word = self.word.get();
        if INDEX_IN_WORD {
            return slot_flow(word);
        }
        match Self::mode(word) {
            MODE_SOURCE => None,
            MODE_SOURCE_SIDE => self.side_flow(),
            _ => slot_flow(word),
        }
    }

    #[cold]
    fn side_flow(&self) -> Option<P<FlowNode>> {
        SIDE_FLOW.lock().unwrap().get(&(std::ptr::from_ref::<Identifier>(self) as usize)).copied()
    }

    pub fn set_flow_node(&self, flow: Option<P<FlowNode>>) {
        let word = self.word.get();
        let mode = Self::mode(word);
        if INDEX_IN_WORD || mode == MODE_TEXT {
            self.word.set(word & !SLOT_MASK | flow_slot(flow));
            return;
        }
        let index = Self::text_index(word);
        let len = word & (LEN_MAX << LEN_SHIFT);
        if mode == MODE_SOURCE_SIDE {
            SIDE_FLOW.lock().unwrap().remove(&(std::ptr::from_ref::<Identifier>(self) as usize));
        }
        let word = match flow {
            None => (MODE_SOURCE << MODE_SHIFT) | len | index as u64,
            Some(f) if f.text_index() == index => (MODE_SOURCE_FLOW << MODE_SHIFT) | len | flow_slot(flow),
            Some(f) => {
                match SIDE_FLOW.lock().unwrap().entry(std::ptr::from_ref::<Identifier>(self) as usize) {
                    Entry::Occupied(mut e) => *e.get_mut() = f,
                    Entry::Vacant(e) => {
                        e.insert(f);
                    }
                }
                (MODE_SOURCE_SIDE << MODE_SHIFT) | len | index as u64
            }
        };
        self.word.set(word);
    }

    /// Whether the text is derived from the node's range (see `Node::set_loc`).
    #[inline]
    pub(crate) fn is_source_text(&self) -> bool {
        Self::mode(self.word.get()) != MODE_TEXT
    }

    pub fn clone_node(&self, node: P<Node>, f: &NodeFactory) -> P<Node> {
        clone_node(f.new_identifier(self.text()), node, &f.hooks)
    }
}

impl NodeFactory {
    pub fn new_identifier(&self, text: &'static str) -> P<Node> {
        self.text_count.set(self.text_count.get() + 1);
        self.new_node(
            Kind::Identifier,
            IdentifierWithText { identifier: Identifier { word: OwnedCell::new(MODE_TEXT << MODE_SHIFT) }, text: PackedStr::new(text) },
        )
    }

    /// `new_identifier` for a token of the text registered as `text_index`, spanning `loc`: when `text` is the
    /// slice of that text that ends at `loc.end()`, the identifier keeps only its length. Sets the range, which can
    /// then not change its end.
    pub fn new_source_identifier(
        &self,
        text: &'static str,
        text_index: u32,
        loc: TextRange,
    ) -> P<Node> {
        let end = loc
            .end()
            .expect_source("source identifiers have source positions") as usize;
        let from_source = text_index != NO_SOURCE_TEXT && text.len() as u64 <= LEN_MAX && {
            let file = source_text(text_index);
            end <= file.len() && text.len() <= end && std::ptr::eq(file.as_ptr().wrapping_add(end - text.len()), text.as_ptr())
        };
        if !from_source {
            let node = self.new_identifier(text);
            node.loc.set(loc);
            return node;
        }
        self.text_count.set(self.text_count.get() + 1);
        let index = if INDEX_IN_WORD { (text_index as u64) << INDEX_SHIFT } else { text_index as u64 };
        let node = self.new_node(
            Kind::Identifier,
            Identifier { word: OwnedCell::new((MODE_SOURCE << MODE_SHIFT) | ((text.len() as u64) << LEN_SHIFT) | index) },
        );
        node.loc.set(loc);
        node
    }
}

/// `Node::set_loc` of an identifier to a range with another end.
#[cold]
pub(crate) fn check_source_identifier_loc(node: &Node, loc: TextRange) {
    assert!(!node.as_identifier().is_source_text(),
        "the range end of identifier {:?} cannot change ({:?} -> {:?})",
        node.as_identifier().text(),
        node.end(),
        loc.end()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::FlowFlags;

    // A compact identifier keeps its text, length and flow node through every flow-node change (with plain pointers,
    // a flow node of another text goes to `SIDE_FLOW`); one too long for the word's length field stores its text.
    #[test]
    fn source_identifiers_keep_text_and_flow_node() {
        let long = "x".repeat(LEN_MAX as usize + 1);
        let text: &'static str = Box::leak(format!("let ab = {long};").into_boxed_str());
        let index = register_source_text(text);
        let f = NodeFactory::default();
        let span = |start: usize, len: usize| (&text[start..start + len], TextRange::new(start as u32, (start + len) as u32));
        let (short_text, short_loc) = span(4, 2);
        let (long_text, long_loc) = span(9, long.len());
        let short = f.new_source_identifier(short_text, index, short_loc);
        let long_node = f.new_source_identifier(long_text, index, long_loc);
        assert!(short.as_identifier().is_source_text());
        assert!(!long_node.as_identifier().is_source_text());
        let same_file = P::new(FlowNode::new(FlowFlags::Start, None, None, index));
        let other_file = P::new(FlowNode::new(FlowFlags::Start, None, None, NO_SOURCE_TEXT));
        for (node, expected) in [(short, short_text), (long_node, long_text)] {
            for flow in [None, Some(same_file), Some(other_file), Some(same_file), None] {
                node.set_flow_node(flow);
                assert_eq!(node.flow_node(), flow);
                assert!(std::ptr::eq(node.text(), expected) || !node.as_identifier().is_source_text());
                assert_eq!(node.text(), expected);
                assert_eq!(node.as_identifier().text_len(), expected.len());
            }
        }
    }
}
