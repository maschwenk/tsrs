// Identifier node data (written by hand; tools/gen-ast/gen-ast.ts skips it).
//
// Source identifiers can keep their length and file index instead of their text: the text is
// `source_text(index)[end - len..end]`. The index shares storage with the flow node; after binding it is read from
// `FlowNode::text_index`. With compressed pointers, the flow/index slot takes four bytes and the node's six spare
// marker bits hold a four-bit length and two-bit mode. Source names of at most 15 bytes therefore occupy 24 bytes
// including the header. Other names keep a PackedStr after the slot and occupy 32 bytes; no text interning is needed.
// Plain pointers retain the previous full-width word, with a 17-bit source-name length.
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

struct SourceTextSlot {
    ptr: AtomicPtr<u8>,
    len: AtomicUsize,
}

// Zero-initialized, so untouched pages cost nothing. A text is registered by the thread that parses it, before any
// identifier of it exists; other threads reach those identifiers only through the synchronization that hands them
// the AST (end of the parse phase, `jsdoc_mu`), so relaxed accesses suffice.
static SOURCE_TEXTS: [SourceTextSlot; SOURCE_TEXTS_CAP] =
    [const { SourceTextSlot { ptr: AtomicPtr::new(std::ptr::null_mut()), len: AtomicUsize::new(0) } }; SOURCE_TEXTS_CAP];
static SOURCE_TEXT_COUNT: AtomicU32 = AtomicU32::new(0);

/// Registers a file's text for compact identifiers; `NO_SOURCE_TEXT` once the table is full (identifiers of later
/// files then store their text).
pub fn register_source_text(text: &'static str) -> u32 {
    let Ok(index) = SOURCE_TEXT_COUNT.try_update(Relaxed, Relaxed, |n| {
        ((n as usize) < SOURCE_TEXTS_CAP).then(|| n + 1)
    }) else {
        return NO_SOURCE_TEXT;
    };
    let slot = &SOURCE_TEXTS[index as usize];
    slot.ptr.store(text.as_ptr().cast_mut(), Relaxed);
    slot.len.store(text.len(), Relaxed);
    index
}

// Forgets the text of a file whose memory is about to be freed (language server regions): the table must not keep a
// pointer into freed memory. The index is not reused.
pub fn unregister_source_text(index: u32) {
    if (index as usize) < SOURCE_TEXTS_CAP {
        let slot = &SOURCE_TEXTS[index as usize];
        slot.len.store(0, Relaxed);
        slot.ptr.store(std::ptr::null_mut(), Relaxed);
    }
}

#[inline]
fn source_text_ptr(index: u32) -> *const u8 {
    SOURCE_TEXTS[index as usize].ptr.load(Relaxed)
}

#[inline]
#[expect(
    clippy::disallowed_methods,
    reason = "from_utf8 re-validates the whole file text on every call: 291 G -> 1,247 G instructions, one checker (notes/lint-paydown-compiler.md)"
)]
fn source_text(index: u32) -> &'static str {
    let slot = &SOURCE_TEXTS[index as usize];
    // SAFETY: the slot was stored from a `&'static str` by `register_source_text` (see the static's comment).
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(slot.ptr.load(Relaxed), slot.len.load(Relaxed))) }
}

// Logical word used by the accessors: bits 0..45 are the flow node (P::pack) or text index, bits 45..62 the length,
// and bits 62..64 the mode. Compressed storage splits it between the payload and the node's marker byte.
const SLOT_MASK: u64 = (1 << tsrs_core::PACK_BITS) - 1;
const LEN_SHIFT: u32 = 45;
const LEN_MAX: u64 = if tsrs_core::COMPRESSED_PTRS { 15 } else { (1 << 17) - 1 };
const MODE_SHIFT: u32 = 62;
/// Text from the source, no flow node; the slot is the text index.
const MODE_SOURCE: u64 = 0;
/// Text from the source; the slot is the flow node, which holds the text index.
const MODE_SOURCE_FLOW: u64 = 1;
/// Text stored after the word (`IdentifierWithText`); the slot is the flow node (0 = none).
const MODE_TEXT: u64 = 2;
/// Text from the source; the slot is the text index and the flow node (made for another text) is in `SIDE_FLOW`.
const MODE_SOURCE_SIDE: u64 = 3;

// Flow nodes of compact identifiers whose text index differs from the identifier's, keyed by the identifier's
// address. The binder only sets flow nodes it made for the file it binds, so this stays empty in practice.
static SIDE_FLOW: LazyLock<Mutex<FxHashMap<usize, P<FlowNode>>>> = LazyLock::new(Default::default);

pub struct Identifier {
    word: OwnedCell<[u32; if tsrs_core::COMPRESSED_PTRS { 1 } else { 2 }]>,
}

// Both identifier forms must have the same alignment: Node::payload<Identifier> also addresses this prefix.
#[repr(C, packed(4))]
struct IdentifierWithText {
    identifier: Identifier,
    text: PackedStr,
}

impl NodePayload for Identifier {
    const TAG: NodeDataTag = NodeDataTag::Identifier;
    #[inline]
    fn initial_markers(&self) -> u8 {
        // new() temporarily puts the length above the 20-bit source index so it reaches the header before hooks run.
        if tsrs_core::COMPRESSED_PTRS { ((self.word.get()[0] >> 20) as u8 & 15) << 2 } else { 0 }
    }
}

impl NodePayload for IdentifierWithText {
    const TAG: NodeDataTag = NodeDataTag::Identifier;
    #[inline]
    fn initial_markers(&self) -> u8 {
        if tsrs_core::COMPRESSED_PTRS { (MODE_TEXT as u8) << 6 } else { 0 }
    }
}

const _: () = assert!(std::mem::size_of::<NodeAlloc<Identifier>>() == std::mem::size_of::<Node>() + if tsrs_core::COMPRESSED_PTRS { 4 } else { 8 });
const _: () = assert!(std::mem::offset_of!(NodeAlloc<Identifier>, data) == std::mem::offset_of!(NodeAlloc<IdentifierWithText>, data));
const _: () = assert!(SOURCE_TEXTS_CAP <= 1 << 20);

/// TOOL (`--features tsrs_ast/ast-sizing` only): arena bytes of an identifier node with its text stored.
#[cfg(feature = "ast-sizing")]
pub(crate) const IDENTIFIER_WITH_TEXT_SIZE: usize = std::mem::size_of::<NodeAlloc<IdentifierWithText>>();

/// TOOL (`--features tsrs_ast/ast-sizing` only): arena bytes of the identifier node `n`.
#[cfg(feature = "ast-sizing")]
pub(crate) fn identifier_alloc_size(n: &Node) -> usize {
    if n.payload::<Identifier>().is_source_text() {
        std::mem::size_of::<NodeAlloc<Identifier>>()
    } else {
        IDENTIFIER_WITH_TEXT_SIZE
    }
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
    fn new(word: u64) -> Self {
        debug_assert!(matches!(Self::mode(word), MODE_SOURCE | MODE_TEXT));
        let mut data = [0; if tsrs_core::COMPRESSED_PTRS { 1 } else { 2 }];
        data[0] = word as u32;
        if tsrs_core::COMPRESSED_PTRS {
            data[0] |= (((word >> LEN_SHIFT) & LEN_MAX) as u32) << 20;
        } else {
            *data.last_mut().unwrap() = (word >> 32) as u32;
        }
        Identifier { word: OwnedCell::new(data) }
    }

    #[inline]
    fn word(&self) -> u64 {
        let data = self.word.get();
        if tsrs_core::COMPRESSED_PTRS {
            let info = self.node().identifier_info();
            u64::from(data[0]) | (u64::from(info & 15) << LEN_SHIFT) | (u64::from(info >> 4) << MODE_SHIFT)
        } else {
            u64::from(data[0]) | (u64::from(*data.last().unwrap()) << 32)
        }
    }

    #[inline]
    fn set_word(&self, word: u64) {
        let mut data = [0; if tsrs_core::COMPRESSED_PTRS { 1 } else { 2 }];
        data[0] = word as u32;
        if tsrs_core::COMPRESSED_PTRS {
            let info = (((word >> LEN_SHIFT) & LEN_MAX) as u8) | (((word >> MODE_SHIFT) as u8) << 4);
            self.node().set_identifier_info(info);
        } else {
            *data.last_mut().unwrap() = (word >> 32) as u32;
        }
        self.word.set(data);
    }

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

    /// The text index of a compact identifier (not `MODE_TEXT`).
    #[inline]
    fn text_index(word: u64) -> u32 {
        if Self::mode(word) == MODE_SOURCE_FLOW {
            slot_flow(word).unwrap().text_index()
        } else {
            // Before the first flow update, the slot can still contain the initial length above the source index.
            (word & if tsrs_core::COMPRESSED_PTRS { (1 << 20) - 1 } else { SLOT_MASK }) as u32
        }
    }

    #[inline]
    #[expect(clippy::disallowed_methods, reason = "from_utf8 here: +1.6% instructions, one checker (notes/lint-paydown-compiler.md)")]
    pub fn text(&self) -> &'static str {
        let word = self.word();
        if Self::mode(word) == MODE_TEXT {
            return self.stored_text();
        }
        let len = ((word >> LEN_SHIFT) & LEN_MAX) as usize;
        let end = self
            .node()
            .end()
            .expect_source("source identifiers have source positions") as usize;
        debug_assert!(end <= source_text(Self::text_index(word)).len() && len <= end);
        // SAFETY: `new_source_identifier` checked that `end - len..end` is the token's text inside the registered
        // text (so in bounds and on char boundaries), and `Node::set_loc` keeps the end from changing (the range of a
        // node is written only there, or by the factory when it creates a node).
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(source_text_ptr(Self::text_index(word)).add(end - len), len)) }
    }

    /// `text().len()`, without forming the text (a compact identifier keeps its length in the word).
    #[inline]
    pub fn text_len(&self) -> usize {
        let word = self.word();
        if Self::mode(word) == MODE_TEXT {
            return self.stored_text().len();
        }
        ((word >> LEN_SHIFT) & LEN_MAX) as usize
    }

    #[cold]
    fn stored_text(&self) -> &'static str {
        // SAFETY: identifiers in `MODE_TEXT` were allocated as `IdentifierWithText` (`repr(C)`, identifier first).
        let text = unsafe { tsrs_core::ptr::arena_neighbor::<_, IdentifierWithText>(self, 0) }.text;
        text.as_str()
    }

    #[inline]
    pub fn flow_node(&self) -> Option<P<FlowNode>> {
        let word = self.word();
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
        let word = self.word();
        let mode = Self::mode(word);
        if mode == MODE_TEXT {
            self.set_word(word & !SLOT_MASK | flow_slot(flow));
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
        self.set_word(word);
    }

    /// Whether the text is derived from the node's range (see `Node::set_loc`).
    #[inline]
    pub(crate) fn is_source_text(&self) -> bool {
        Self::mode(self.word()) != MODE_TEXT
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
            IdentifierWithText { identifier: Identifier::new(MODE_TEXT << MODE_SHIFT), text: PackedStr::new(text) },
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
        let node = self.new_node(
            Kind::Identifier,
            Identifier::new((MODE_SOURCE << MODE_SHIFT) | ((text.len() as u64) << LEN_SHIFT) | text_index as u64),
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
    use crate::{FlowFlags, NodeFlags};

    #[test]
    fn identifier_storage_boundary_preserves_text_flow_and_node_metadata() {
        let factory = NodeFactory::default();
        let parent = factory.new_identifier("parent");
        let other_index = register_source_text("other");
        for len in [0, 1, 15, 16, 128, 2048] {
            let factory = crate::ast::new_node_factory(crate::ast::NodeFactoryHooks {
                on_create: Some(std::rc::Rc::new(move |node| assert_eq!(node.as_identifier().text_len(), len))),
                ..Default::default()
            });
            let text = tsrs_core::alloc_str(&"x".repeat(len));
            let index = register_source_text(text);
            let loc = TextRange::new(0, len as u32);
            let node = factory.new_source_identifier(text, index, loc);
            node.set_parent(Some(parent));
            node.set_flags(NodeFlags::from_bits_retain(u32::MAX));
            node.mark_trailing_comma();
            let same_file = P::new(FlowNode::new(FlowFlags::Start, None, None, index));
            let other_file = P::new(FlowNode::new(FlowFlags::Start, None, None, other_index));
            for flow in [None, Some(same_file), Some(other_file), Some(same_file), None] {
                node.set_flow_node(flow);
                assert_eq!(node.text(), text);
                assert_eq!(node.text().as_ptr(), text.as_ptr());
                assert_eq!(node.as_identifier().text_len(), len);
                assert_eq!(node.flow_node(), flow);
                assert_eq!(node.loc(), loc);
                assert_eq!(node.parent(), Some(parent));
                assert_eq!(node.flags().bits(), u32::MAX);
                assert!(node.has_trailing_comma_marker());
            }
            let cloned = node.clone_node(&factory);
            assert_eq!(cloned.text(), text);
            assert_eq!(cloned.loc(), loc);
        }
    }
}
