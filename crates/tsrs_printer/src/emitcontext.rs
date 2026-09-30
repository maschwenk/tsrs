use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering};

use rustc_hash::FxHashMap;
use tsrs_ast::*;
use tsrs_core::collections::OrderedSet;
use tsrs_core::*;

use crate::*;
use crate::factory::{new_node_factory, NodeFactory};

// Stores side-table information used during transformation that can be read by the printer to customize emit
//
// NOTE: EmitContext is not guaranteed to be thread-safe.
//
// Handled as `P<EmitContext>` (shared by the node builder and the printers it feeds, like Go's `*EmitContext`), so
// all state is interior-mutable. The transform-only parts (variable/lexical environments, visitor hooks) are not
// ported.
pub struct EmitContext {
    pub factory: RefCell<NodeFactory>, // Required. The NodeFactory to use to create new nodes
    auto_generate: RefCell<FxHashMap<P<Node>, AutoGenerateInfo>>,
    text_source: RefCell<FxHashMap<P<Node>, P<Node>>>,
    original: RefCell<FxHashMap<P<Node>, P<Node>>>,
    emit_nodes: RefCell<FxHashMap<P<Node>, emitNode>>,
    assigned_name: RefCell<FxHashMap<P<Node>, P<Node>>>,
    class_this: RefCell<FxHashMap<P<Node>, P<Node>>>,
    emit_helpers: RefCell<OrderedSet<P<EmitHelper>>>,
}

pub fn new_emit_context() -> P<EmitContext> {
    let c = P::new(EmitContext {
        factory: RefCell::new(NodeFactory::default()),
        auto_generate: RefCell::default(),
        text_source: RefCell::default(),
        original: RefCell::default(),
        emit_nodes: RefCell::default(),
        assigned_name: RefCell::default(),
        class_this: RefCell::default(),
        emit_helpers: RefCell::default(),
    });
    *c.factory.borrow_mut() = new_node_factory(c);
    c
}

// Go pools emit contexts; the release function resets the context like Go's does before returning it to the pool.
pub fn get_emit_context() -> (P<EmitContext>, impl FnOnce()) {
    let c = new_emit_context();
    (c, move || c.reset())
}

impl EmitContext {
    pub fn new() -> P<EmitContext> {
        new_emit_context()
    }

    pub fn reset(&self) {
        self.auto_generate.borrow_mut().clear();
        self.text_source.borrow_mut().clear();
        self.original.borrow_mut().clear();
        self.emit_nodes.borrow_mut().clear();
        self.assigned_name.borrow_mut().clear();
        self.class_this.borrow_mut().clear();
        self.emit_helpers.borrow_mut().clear();
    }

    pub(crate) fn on_create(&self, node: P<Node>) {
        node.flags.set(node.flags.get() | NodeFlags::Synthesized);
    }

    pub(crate) fn on_update(&self, updated: P<Node>, original: P<Node>) {
        self.set_original(updated, original);
    }

    pub(crate) fn on_clone(&self, updated: P<Node>, original: P<Node>) {
        self.set_original(updated, original);
        if is_identifier(updated) || is_private_identifier(updated) {
            let auto_generate = self.auto_generate.borrow().get(&original).copied();
            if let Some(auto_generate) = auto_generate {
                let auto_generate_copy = auto_generate;
                self.auto_generate.borrow_mut().insert(updated, auto_generate_copy);
            }
        }
    }

    //
    // Name Generation
    //

    // Gets whether a given name has an associated AutoGenerateInfo entry.
    pub fn has_auto_generate_info(&self, node: Option<P<Node>>) -> bool {
        if let Some(node) = node {
            return self.auto_generate.borrow().contains_key(&node);
        }
        false
    }

    // Gets the associated AutoGenerateInfo entry for a given name.
    pub fn get_auto_generate_info(&self, name: Option<P<Node>>) -> Option<AutoGenerateInfo> {
        let name = name?;
        self.auto_generate.borrow().get(&name).copied()
    }

    pub(crate) fn set_auto_generate_info(&self, name: P<Node>, info: AutoGenerateInfo) {
        self.auto_generate.borrow_mut().insert(name, info);
    }

    // Walks the associated AutoGenerateInfo entries of a name to find the root Nopde from which the name should be generated.
    pub fn get_node_for_generated_name(&self, name: P<Node>) -> P<Node> {
        if let Some(auto_generate) = self.get_auto_generate_info(Some(name)) {
            if auto_generate.flags.is_node() {
                return self.get_node_for_generated_name_worker(auto_generate.node.unwrap(), auto_generate.id);
            }
        }
        name
    }

    pub(crate) fn get_node_for_generated_name_worker(&self, node: P<Node>, auto_generate_id: AutoGenerateId) -> P<Node> {
        let mut node = node;
        let mut original = self.original(node);
        while let Some(o) = original {
            node = o;
            if is_member_name(node) {
                // if "node" is a different generated name (having a different "autoGenerateId"), use it and stop traversing.
                let auto_generate = self.get_auto_generate_info(Some(node));
                match auto_generate {
                    None => break,
                    Some(auto_generate) => {
                        if auto_generate.flags.is_node() && auto_generate.id != auto_generate_id {
                            break;
                        }
                        if auto_generate.flags.is_node() {
                            original = auto_generate.node;
                            continue;
                        }
                    }
                }
            }
            original = self.original(node);
        }
        node
    }

    //
    // Original Node Tracking
    //

    // Sets the original node for a given node.
    //
    // NOTE: This is the equivalent to `setOriginalNode` in Strada.
    pub fn set_original(&self, node: P<Node>, original: P<Node>) {
        self.set_original_ex(node, original, false);
    }

    pub fn unset_original(&self, node: P<Node>) {
        self.original.borrow_mut().remove(&node);
    }

    pub fn set_original_ex(&self, node: P<Node>, original: P<Node>, allow_overwrite: bool) {
        let existing = self.original.borrow().get(&node).copied();
        match existing {
            None => {
                self.original.borrow_mut().insert(node, original);
                let source = self.emit_nodes.borrow().get(&original).cloned();
                if let Some(emit_node) = source {
                    self.emit_nodes.borrow_mut().entry(node).or_default().copy_from(&emit_node);
                }
            }
            Some(existing) => {
                if !allow_overwrite && existing != original {
                    panic!("Original node already set.");
                } else if allow_overwrite {
                    self.original.borrow_mut().insert(node, original);
                }
            }
        }
    }

    // Gets the original node for a given node.
    //
    // NOTE: This is the equivalent to reading `node.original` in Strada.
    pub fn original(&self, node: P<Node>) -> Option<P<Node>> {
        self.original.borrow().get(&node).copied()
    }

    // Gets the most original node associated with this node by walking Original pointers.
    //
    // NOTE: This method is analogous to `getOriginalNode` in the old compiler, but the name has changed to avoid accidental
    // conflation with `SetOriginal`/`Original`
    pub fn most_original(&self, node: P<Node>) -> P<Node> {
        let mut node = node;
        let mut original = self.original(node);
        while let Some(o) = original {
            node = o;
            original = self.original(node);
        }
        node
    }

    // Gets the original parse tree node for a given node.
    //
    // NOTE: This is the equivalent to `getParseTreeNode` in Strada.
    pub fn parse_node(&self, node: Option<P<Node>>) -> Option<P<Node>> {
        let node = self.most_original(node?);
        if is_parse_tree_node(node) {
            return Some(node);
        }
        None
    }

    pub fn is_file_level_unique_name(&self, source_file: P<SourceFile>, name: &str, has_global_name: Option<&dyn Fn(&str) -> bool>) -> bool {
        if let Some(has_global_name) = has_global_name {
            if has_global_name(name) {
                return false;
            }
        }
        let source_file = self.most_original(source_file.as_node()).as_source_file();
        !source_file.has_identifier(name)
    }

    //
    // Emit-related Data
    //

    pub fn emit_flags(&self, node: P<Node>) -> EmitFlags {
        if let Some(emit_node) = self.emit_nodes.borrow().get(&node) {
            return emit_node.emit_flags;
        }
        EmitFlags::None
    }

    pub fn set_emit_flags(&self, node: P<Node>, flags: EmitFlags) {
        self.emit_nodes.borrow_mut().entry(node).or_default().emit_flags = flags;
    }

    pub fn add_emit_flags(&self, node: P<Node>, flags: EmitFlags) {
        self.emit_nodes.borrow_mut().entry(node).or_default().emit_flags |= flags;
    }

    pub fn snippet_element(&self, node: P<Node>) -> Option<SnippetElement> {
        if let Some(emit_node) = self.emit_nodes.borrow().get(&node) {
            return emit_node.snippet_element;
        }
        None
    }

    pub fn set_snippet_element(&self, node: P<Node>, snippet_element: SnippetElement) {
        self.emit_nodes.borrow_mut().entry(node).or_default().snippet_element = Some(snippet_element);
    }

    // Gets the range to use for a node when emitting comments.
    pub fn comment_range(&self, node: P<Node>) -> TextRange {
        if let Some(emit_node) = self.emit_nodes.borrow().get(&node) {
            if emit_node.flags.intersects(emitNodeFlags::hasCommentRange) {
                return emit_node.comment_range;
            }
        }
        node.loc()
    }

    // Sets the range to use for a node when emitting comments.
    pub fn set_comment_range(&self, node: P<Node>, loc: TextRange) {
        let mut emit_nodes = self.emit_nodes.borrow_mut();
        let emit_node = emit_nodes.entry(node).or_default();
        emit_node.comment_range = loc;
        emit_node.flags |= emitNodeFlags::hasCommentRange;
    }

    // Sets the range to use for a node when emitting comments.
    pub fn assign_comment_range(&self, to: P<Node>, from: P<Node>) {
        self.set_comment_range(to, self.comment_range(from));
    }

    // Gets the range to use for a node when emitting source maps.
    pub fn source_map_range(&self, node: P<Node>) -> TextRange {
        if let Some(emit_node) = self.emit_nodes.borrow().get(&node) {
            if emit_node.flags.intersects(emitNodeFlags::hasSourceMapRange) {
                return emit_node.source_map_range;
            }
        }
        node.loc()
    }

    // Sets the range to use for a node when emitting source maps.
    pub fn set_source_map_range(&self, node: P<Node>, loc: TextRange) {
        let mut emit_nodes = self.emit_nodes.borrow_mut();
        let emit_node = emit_nodes.entry(node).or_default();
        emit_node.source_map_range = loc;
        emit_node.flags |= emitNodeFlags::hasSourceMapRange;
    }

    // Sets the range to use for a node when emitting source maps.
    pub fn assign_source_map_range(&self, to: P<Node>, from: P<Node>) {
        self.set_source_map_range(to, self.source_map_range(from));
    }

    // Sets the range to use for a node when emitting comments and source maps.
    pub fn assign_comment_and_source_map_ranges(&self, to: P<Node>, from: P<Node>) {
        let comment_range = self.comment_range(from);
        let source_map_range = self.source_map_range(from);
        let mut emit_nodes = self.emit_nodes.borrow_mut();
        let emit_node = emit_nodes.entry(to).or_default();
        emit_node.comment_range = comment_range;
        emit_node.source_map_range = source_map_range;
        emit_node.flags |= emitNodeFlags::hasCommentRange | emitNodeFlags::hasSourceMapRange;
    }

    // Gets the range for a token of a node when emitting source maps.
    pub fn token_source_map_range(&self, node: P<Node>, kind: Kind) -> Option<TextRange> {
        if let Some(emit_node) = self.emit_nodes.borrow().get(&node) {
            if let Some(ranges) = &emit_node.token_source_map_ranges {
                if let Some(loc) = ranges.get(&kind) {
                    return Some(*loc);
                }
            }
        }
        None
    }

    // Sets the range for a token of a node when emitting source maps.
    pub fn set_token_source_map_range(&self, node: P<Node>, kind: Kind, loc: TextRange) {
        let mut emit_nodes = self.emit_nodes.borrow_mut();
        let emit_node = emit_nodes.entry(node).or_default();
        emit_node.token_source_map_ranges.get_or_insert_with(FxHashMap::default).insert(kind, loc);
    }

    pub fn assigned_name(&self, node: P<Node>) -> Option<P<Node>> {
        self.assigned_name.borrow().get(&node).copied()
    }

    pub fn text_source(&self, node: P<Node>) -> Option<P<Node>> {
        self.text_source.borrow().get(&node).copied()
    }

    pub(crate) fn set_text_source(&self, node: P<Node>, text_source_node: P<Node>) {
        self.text_source.borrow_mut().insert(node, text_source_node);
    }

    pub fn set_assigned_name(&self, node: P<Node>, name: P<Node>) {
        self.assigned_name.borrow_mut().insert(node, name);
    }

    pub fn class_this(&self, node: P<Node>) -> Option<P<Node>> {
        self.class_this.borrow().get(&node).copied()
    }

    pub fn set_class_this(&self, node: P<Node>, class_this: P<Node>) {
        self.class_this.borrow_mut().insert(node, class_this);
    }

    pub fn request_emit_helper(&self, helper: P<EmitHelper>) {
        if helper.scoped {
            panic!("Cannot request a scoped emit helper");
        }
        for h in helper.dependencies {
            self.request_emit_helper(*h);
        }
        self.emit_helpers.borrow_mut().insert(helper);
    }

    pub fn read_emit_helpers(&self) -> Vec<P<EmitHelper>> {
        let helpers: Vec<P<EmitHelper>> = self.emit_helpers.borrow().iter().copied().collect();
        self.emit_helpers.borrow_mut().clear();
        helpers
    }

    pub fn add_emit_helper(&self, node: P<Node>, helper: &[P<EmitHelper>]) {
        let mut emit_nodes = self.emit_nodes.borrow_mut();
        let emit_node = emit_nodes.entry(node).or_default();
        for h in helper {
            if !emit_node.helpers.contains(h) {
                emit_node.helpers.push(*h);
            }
        }
    }

    pub fn get_emit_helpers(&self, node: P<Node>) -> Vec<P<EmitHelper>> {
        if let Some(emit_node) = self.emit_nodes.borrow().get(&node) {
            return emit_node.helpers.clone();
        }
        Vec::new()
    }

    pub fn get_external_helpers_module_name(&self, node: P<SourceFile>) -> Option<P<Node>> {
        if let Some(parse_node) = self.parse_node(Some(node.as_node())) {
            if let Some(emit_node) = self.emit_nodes.borrow().get(&parse_node) {
                return emit_node.external_helpers_module_name;
            }
        }
        None
    }

    pub fn set_external_helpers_module_name(&self, node: P<SourceFile>, name: P<Node>) {
        let parse_node = self.parse_node(Some(node.as_node()));
        let Some(parse_node) = parse_node else {
            panic!("Node must be a parse tree node or have an Original pointer to a parse tree node.");
        };

        self.emit_nodes.borrow_mut().entry(parse_node).or_default().external_helpers_module_name = Some(name);
    }

    pub fn has_recorded_external_helpers(&self, node: P<SourceFile>) -> bool {
        if let Some(parse_node) = self.parse_node(Some(node.as_node())) {
            let emit_nodes = self.emit_nodes.borrow();
            let emit_node = emit_nodes.get(&parse_node);
            return emit_node.is_some_and(|emit_node| emit_node.external_helpers_module_name.is_some() || emit_node.emit_flags.intersects(EmitFlags::ExternalHelpers));
        }
        false
    }

    pub fn is_call_to_helper(&self, first_segment: P<Node>, helper_name: &str) -> bool {
        is_call_expression(first_segment)
            && is_identifier(first_segment.expression().unwrap())
            && self.emit_flags(first_segment.expression().unwrap()).intersects(EmitFlags::HelperName)
            && first_segment.expression().unwrap().text() == helper_name
    }

    pub fn set_synthetic_leading_comments(&self, node: P<Node>, comments: Vec<SynthesizedComment>) -> P<Node> {
        self.emit_nodes.borrow_mut().entry(node).or_default().leading_comments = comments;
        node
    }

    pub fn add_synthetic_leading_comment(&self, node: P<Node>, kind: Kind, text: &str, has_trailing_new_line: bool) -> P<Node> {
        self.emit_nodes.borrow_mut().entry(node).or_default().leading_comments.push(SynthesizedComment {
            kind,
            loc: TextRange::new(-1, -1),
            has_leading_new_line: false,
            has_trailing_new_line,
            text: text.to_string(),
        });
        node
    }

    pub fn get_synthetic_leading_comments(&self, node: P<Node>) -> Vec<SynthesizedComment> {
        if let Some(emit_node) = self.emit_nodes.borrow().get(&node) {
            return emit_node.leading_comments.clone();
        }
        Vec::new()
    }

    pub fn set_synthetic_trailing_comments(&self, node: P<Node>, comments: Vec<SynthesizedComment>) -> P<Node> {
        self.emit_nodes.borrow_mut().entry(node).or_default().trailing_comments = comments;
        node
    }

    pub fn add_synthetic_trailing_comment(&self, node: P<Node>, kind: Kind, text: &str, has_trailing_new_line: bool) -> P<Node> {
        self.emit_nodes.borrow_mut().entry(node).or_default().trailing_comments.push(SynthesizedComment {
            kind,
            loc: TextRange::new(-1, -1),
            has_leading_new_line: false,
            has_trailing_new_line,
            text: text.to_string(),
        });
        node
    }

    pub fn get_synthetic_trailing_comments(&self, node: P<Node>) -> Vec<SynthesizedComment> {
        if let Some(emit_node) = self.emit_nodes.borrow().get(&node) {
            return emit_node.trailing_comments.clone();
        }
        Vec::new()
    }

    // SetTypeNode stores the original type node on a name node when the type is erased,
    // so the emitter can use the type's position for comment preservation.
    pub fn set_type_node(&self, node: P<Node>, type_node: P<Node>) {
        self.emit_nodes.borrow_mut().entry(node).or_default().type_node = Some(type_node);
    }

    // GetTypeNode gets the type node stored on a name node by the type eraser.
    pub fn get_type_node(&self, node: P<Node>) -> Option<P<Node>> {
        if let Some(emit_node) = self.emit_nodes.borrow().get(&node) {
            return emit_node.type_node;
        }
        None
    }

    pub fn new_not_emitted_statement(&self, node: P<Node>) -> P<Node> {
        let statement = self.factory.borrow_mut().new_not_emitted_statement();
        statement.set_loc(node.loc());
        self.set_original(statement, node);
        self.assign_comment_range(statement, node);
        statement
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AutoGenerateOptions {
    pub flags: GeneratedIdentifierFlags,
    pub prefix: &'static str,
    pub suffix: &'static str,
}

static nextAutoGenerateId: AtomicU32 = AtomicU32::new(0);

pub(crate) fn next_auto_generate_id() -> AutoGenerateId {
    AutoGenerateId(nextAutoGenerateId.fetch_add(1, Ordering::SeqCst) + 1)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct AutoGenerateId(pub u32);

#[derive(Clone, Copy, Debug)]
pub struct AutoGenerateInfo {
    pub flags: GeneratedIdentifierFlags, // Specifies whether to auto-generate the text for an identifier.
    pub id: AutoGenerateId, // Ensures unique generated identifiers get unique names, but clones get the same name.
    pub prefix: &'static str, // Optional prefix to apply to the start of the generated name
    pub suffix: &'static str, // Optional suffix to apply to the end of the generated name
    pub node: Option<P<Node>>, // For a GeneratedIdentifierFlagsNode, the node from which to generate an identifier
}

bitflags::bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub(crate) struct emitNodeFlags: u32 {
        const hasCommentRange = 1 << 0;
        const hasSourceMapRange = 1 << 1;
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum SnippetKind {
    #[default]
    TabStop,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SnippetElement {
    pub kind: SnippetKind,
    pub order: i32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SynthesizedComment {
    pub kind: Kind,
    pub loc: TextRange,
    pub has_leading_new_line: bool,
    pub has_trailing_new_line: bool,
    pub text: String,
}

#[derive(Clone, Default)]
pub(crate) struct emitNode {
    flags: emitNodeFlags,
    emit_flags: EmitFlags,
    comment_range: TextRange,
    source_map_range: TextRange,
    token_source_map_ranges: Option<FxHashMap<Kind, TextRange>>,
    helpers: Vec<P<EmitHelper>>,
    external_helpers_module_name: Option<P<Node>>,
    leading_comments: Vec<SynthesizedComment>,
    trailing_comments: Vec<SynthesizedComment>,
    type_node: Option<P<Node>>,
    snippet_element: Option<SnippetElement>,
}

impl emitNode {
    // NOTE: This method is not guaranteed to be thread-safe
    fn copy_from(&mut self, source: &emitNode) {
        self.flags = source.flags;
        self.emit_flags = source.emit_flags;
        self.comment_range = source.comment_range;
        self.source_map_range = source.source_map_range;
        self.token_source_map_ranges = source.token_source_map_ranges.clone();
        self.helpers = source.helpers.clone();
        self.external_helpers_module_name = source.external_helpers_module_name;
        if let Some(snippet_element) = source.snippet_element {
            self.snippet_element = Some(snippet_element);
        }
    }
}
