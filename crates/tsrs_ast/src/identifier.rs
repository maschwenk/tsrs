// Identifier node data (written by hand; tools/gen-ast/gen-ast.ts skips it).
//
// Go's `Identifier` holds `Text` and `FlowNodeBase.FlowNode`. Almost every identifier is made by the parser from a
// token whose text is the slice of its file's text that ends where the identifier ends (on the private monorepo
// 7.86M of 8.15M; the rest are missing identifiers, clones and synthesized names). Such an identifier keeps only the
// text's length and the file's text index (`register_source_text`): the text is `source_text(index)[end - len..end]`
// with `end` from the node's range. The index sits in the bits of the flow node until the binder sets one; from then
// on it is read from the flow node (`FlowNode::text_index`: the binder made that flow node for the same file). So
// flow node and text fit one word, and the node is 32 bytes instead of 40. Every other identifier stores its text
// after the word as owned Rust text (`IdentifierWithText`, 40 bytes as before).
//
// `text()` returns exactly the slice the parser had (same pointer and length), so nothing observable changes. The
// range end of a compact identifier is fixed when it is created: `Node::set_loc` panics if it would change.

use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::{LazyLock, Mutex};

use rustc_hash::FxHashMap;
use tsrs_core::{OwnedCell, TextView, TextRange, P};

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
    // A compare-exchange loop rather than `fetch_update`: that method is deprecated in favor of `try_update` on
    // newer toolchains, and the replacement does not exist on older ones.
    let mut n = SOURCE_TEXT_COUNT.load(Relaxed);
    let index = loop {
        if n as usize >= SOURCE_TEXTS_CAP {
            return NO_SOURCE_TEXT;
        }
        match SOURCE_TEXT_COUNT.compare_exchange_weak(n, n + 1, Relaxed, Relaxed) {
            Ok(_) => break n,
            Err(current) => n = current,
        }
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

// The word: bits 0..45 the flow node (`P::pack`, like the node header's parent) or the text index, bits 45..62 the text length, bits 62..64 the mode.
const SLOT_MASK: u64 = (1 << tsrs_core::PACK_BITS) - 1;
const LEN_SHIFT: u32 = 45;
const LEN_MAX: u64 = (1 << 17) - 1;
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
    word: OwnedCell<u64>,
}

#[repr(C)]
struct IdentifierWithText {
    identifier: Identifier,
    text: TextView,
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
    let field = |data: usize| tsrs_core::CensusField::X8 { off: data + word, modes: 1 << MODE_SOURCE_FLOW | 1 << MODE_TEXT };
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
    #[expect(clippy::cast_ptr_alignment, reason = "the header starts the `NodeAlloc`, so it has the allocation's alignment")]
    fn node(&self) -> &Node {
        // SAFETY: identifiers are created only by `new_node` inside a `NodeAlloc` (`repr(C)`, header first), so the
        // header lies at this offset before the data struct.
        unsafe { &*std::ptr::from_ref::<Identifier>(self).cast::<u8>().sub(std::mem::offset_of!(NodeAlloc<Identifier>, data)).cast::<Node>() }
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
            (word & SLOT_MASK) as u32
        }
    }

    #[inline]
    #[expect(clippy::disallowed_methods, reason = "from_utf8 here: +1.6% instructions, one checker (notes/lint-paydown-compiler.md)")]
    pub fn text(&self) -> &str {
        let word = self.word.get();
        if Self::mode(word) == MODE_TEXT {
            return self.stored_text();
        }
        let len = ((word >> LEN_SHIFT) & LEN_MAX) as usize;
        let end = self.node().end() as usize;
        debug_assert!(end <= source_text(Self::text_index(word)).len() && len <= end);
        // SAFETY: `new_source_identifier` checked that `end - len..end` is the token's text inside the registered
        // text (so in bounds and on char boundaries), and `Node::set_loc` keeps the end from changing (the range of a
        // node is written only there, or by the factory when it creates a node).
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(source_text_ptr(Self::text_index(word)).add(end - len), len)) }
    }

    #[cold]
    fn stored_text(&self) -> &str {
        // SAFETY: identifiers in `MODE_TEXT` were allocated as `IdentifierWithText` (`repr(C)`, identifier first).
        unsafe { &*std::ptr::from_ref::<Identifier>(self).cast::<IdentifierWithText>() }.text.as_ref()
    }

    #[inline]
    pub fn flow_node(&self) -> Option<P<FlowNode>> {
        let word = self.word.get();
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
        if mode == MODE_TEXT {
            self.word.set(word & !SLOT_MASK | flow_slot(flow));
            return;
        }
        let index = Self::text_index(word);
        let len = word & (LEN_MAX << LEN_SHIFT);
        if mode == MODE_SOURCE_SIDE {
            SIDE_FLOW.lock().unwrap().remove(&(std::ptr::from_ref::<Identifier>(self) as usize));
        }
        let word = match flow {
            None => MODE_SOURCE << MODE_SHIFT | len | index as u64,
            Some(f) if f.text_index() == index => MODE_SOURCE_FLOW << MODE_SHIFT | len | flow_slot(flow),
            Some(f) => {
                match SIDE_FLOW.lock().unwrap().entry(std::ptr::from_ref::<Identifier>(self) as usize) {
                    Entry::Occupied(mut e) => *e.get_mut() = f,
                    Entry::Vacant(e) => {
                        e.insert(f);
                    }
                }
                MODE_SOURCE_SIDE << MODE_SHIFT | len | index as u64
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
    pub fn new_identifier(&self, text: &str) -> P<Node> {
        self.text_count.set(self.text_count.get() + 1);
        self.new_node(
            Kind::Identifier,
            IdentifierWithText { identifier: Identifier { word: OwnedCell::new(MODE_TEXT << MODE_SHIFT) }, text: TextView::from(text) },
        )
    }

    /// `new_identifier` for a token of the text registered as `text_index`, spanning `loc`: when `text` is the
    /// slice of that text that ends at `loc.end()`, the identifier keeps only its length. Sets the range, which can
    /// then not change its end.
    pub fn new_source_identifier(&self, text: &'static str, text_index: u32, loc: TextRange) -> P<Node> {
        let end = loc.end() as usize;
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
            Identifier { word: OwnedCell::new(MODE_SOURCE << MODE_SHIFT | (text.len() as u64) << LEN_SHIFT | text_index as u64) },
        );
        node.loc.set(loc);
        node
    }
}

/// `Node::set_loc` of an identifier to a range with another end.
#[cold]
pub(crate) fn check_source_identifier_loc(node: &Node, loc: TextRange) {
    if node.as_identifier().is_source_text() {
        panic!("the range end of identifier {:?} cannot change ({} -> {})", node.as_identifier().text(), node.end(), loc.end());
    }
}
