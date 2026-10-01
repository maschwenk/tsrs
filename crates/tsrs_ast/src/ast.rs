use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Once, OnceLock, RwLock};

use rustc_hash::FxHashMap;
use tsrs_core::collections::Set;
use tsrs_core::tspath::Path;
use tsrs_core::{
    alloc_slice, alloc_str, alloc_vec, compute_ecma_line_starts, undefined_text_range, LanguageVariant, ResolutionMode,
    FrozenCell, OwnedCell, ScriptKind, TextPos, TextRange, Tristate, P,
};
use tsrs_diagnostics as diagnostics;

use crate::*;

// parseJSDocForNode is the package-level function for lazily parsing JSDoc.
// It is set by the parser crate via set_parse_jsdoc_for_node.
pub type ParseJSDocForNodeFn = fn(&'static SourceFile, P<Node>) -> Vec<P<Node>>;

static PARSE_JSDOC_FOR_NODE: OnceLock<ParseJSDocForNodeFn> = OnceLock::new();

// SetParseJSDocForNode registers the lazy JSDoc parse function. Called once by the parser crate.
pub fn set_parse_jsdoc_for_node(f: ParseJSDocForNodeFn) {
    let _ = PARSE_JSDOC_FOR_NODE.set(f);
}

// Visitor
//
// Go `type Visitor func(*Node) bool`; Rust visitors are `&mut dyn FnMut(P<Node>) -> bool`.

#[inline]
pub(crate) fn visit(v: &mut dyn FnMut(P<Node>) -> bool, node: Option<P<Node>>) -> bool {
    match node {
        Some(node) => v(node),
        None => false,
    }
}

#[inline]
pub(crate) fn visit_nodes(v: &mut dyn FnMut(P<Node>) -> bool, nodes: &[P<Node>]) -> bool {
    for &node in nodes {
        if v(node) {
            return true;
        }
    }
    false
}

#[inline]
pub(crate) fn visit_node_list(v: &mut dyn FnMut(P<Node>) -> bool, node_list: Option<P<NodeList>>) -> bool {
    match node_list {
        Some(list) => visit_nodes(v, list.nodes),
        None => false,
    }
}

#[inline]
pub(crate) fn visit_modifiers(v: &mut dyn FnMut(P<Node>) -> bool, modifiers: Option<P<ModifierList>>) -> bool {
    match modifiers {
        Some(list) => visit_nodes(v, list.list.nodes),
        None => false,
    }
}

/// Go `core.Same` on slices: identity of the backing array and length.
#[inline]
pub fn same_slice<T>(a: &[T], b: &[T]) -> bool {
    a.len() == b.len() && std::ptr::eq(a.as_ptr(), b.as_ptr())
}

// NodeFactory

pub type NodeHook = Rc<dyn Fn(P<Node>)>;
pub type NodeUpdateHook = Rc<dyn Fn(P<Node>, P<Node>)>;

#[derive(Default, Clone)]
pub struct NodeFactoryHooks {
    pub on_create: Option<NodeHook>,       // Hooks the creation of a node.
    pub on_update: Option<NodeUpdateHook>, // Hooks the updating of a node.
    pub on_clone: Option<NodeUpdateHook>,  // Hooks the cloning of a node.
}

/// Go `*ast.NodeFactory`. A handle: every method takes `&self`, and `clone()` yields another handle to the same
/// factory (shared hooks and counters), like copying the Go pointer. So `f.new_x(f.new_y())` works and a factory can
/// be shared by a visitor, the node builder and an emit context.
#[derive(Default, Clone)]
pub struct NodeFactory {
    pub(crate) hooks: NodeFactoryHooks,
    pub(crate) node_count: Rc<Cell<usize>>,
    pub(crate) text_count: Rc<Cell<usize>>,
}

pub fn new_node_factory(hooks: NodeFactoryHooks) -> NodeFactory {
    NodeFactory { hooks, node_count: Rc::default(), text_count: Rc::default() }
}

fn node_header(kind: Kind, data_tag: NodeDataTag) -> Node {
    Node {
        header: OwnedCell::new(NodeHeaderWord::new(kind, data_tag)),
        flags: OwnedCell::new(NodeFlags::None),
        id: AtomicU32::new(0),
        loc: OwnedCell::new(undefined_text_range()),
    }
}

/// Creates a node whose data struct is `data` (Go: the data struct embeds `NodeBase`, one allocation).
pub(crate) fn new_node<T: NodePayload>(kind: Kind, data: T, hooks: &NodeFactoryHooks) -> P<Node> {
    let a: &'static NodeAlloc<T> = P::new(NodeAlloc { node: node_header(kind, T::TAG), data }).get();
    // SAFETY: `NodeAlloc` is `repr(C)` with the header first, so the pointer to the allocation is a pointer to
    // its header; the header is never moved or freed (leak arena).
    let n = P::from_static(unsafe { &*(a as *const NodeAlloc<T>).cast::<Node>() });
    if let Some(on_create) = &hooks.on_create {
        on_create(n);
    }
    n
}

/// Creates a node whose data struct has no fields (`Token`, `KeywordTypeNode`, ...): just the header.
pub(crate) fn new_empty_node(kind: Kind, data_tag: NodeDataTag, hooks: &NodeFactoryHooks) -> P<Node> {
    let n = P::new(node_header(kind, data_tag));
    if let Some(on_create) = &hooks.on_create {
        on_create(n);
    }
    n
}

impl NodeFactory {
    pub fn new(hooks: NodeFactoryHooks) -> NodeFactory {
        new_node_factory(hooks)
    }

    #[inline]
    pub(crate) fn new_node<T: NodePayload>(&self, kind: Kind, data: T) -> P<Node> {
        self.node_count.set(self.node_count.get() + 1);
        new_node(kind, data, &self.hooks)
    }

    #[inline]
    pub(crate) fn new_empty_node(&self, kind: Kind, data_tag: NodeDataTag) -> P<Node> {
        self.node_count.set(self.node_count.get() + 1);
        new_empty_node(kind, data_tag, &self.hooks)
    }

    pub fn node_count(&self) -> usize {
        self.node_count.get()
    }

    pub fn text_count(&self) -> usize {
        self.text_count.get()
    }

    pub fn as_node_factory(&self) -> &NodeFactory {
        self
    }
}

pub(crate) fn update_node(updated: P<Node>, original: P<Node>, hooks: &NodeFactoryHooks) -> P<Node> {
    if updated != original {
        updated.flags.set(original.flags.get());
        updated.loc.set(original.loc.get());
        if let Some(on_update) = &hooks.on_update {
            on_update(updated, original);
        }
    }
    updated
}

pub(crate) fn clone_node(updated: P<Node>, original: P<Node>, hooks: &NodeFactoryHooks) -> P<Node> {
    update_node(updated, original, hooks);
    if updated != original {
        if let Some(on_clone) = &hooks.on_clone {
            on_clone(updated, original);
        }
    }
    updated
}

// NodeList

pub struct NodeList {
    pub loc: OwnedCell<TextRange>,
    pub nodes: &'static [P<Node>],
}

impl NodeFactory {
    pub fn new_node_list(&self, nodes: Vec<P<Node>>) -> P<NodeList> {
        self.new_node_list_from_static(alloc_vec(nodes))
    }

    pub fn new_node_list_from_slice(&self, nodes: &[P<Node>]) -> P<NodeList> {
        self.new_node_list_from_static(alloc_slice(nodes))
    }

    /// Stores `nodes` without copying (keeps slice identity, e.g. for sentinel slices).
    pub fn new_node_list_from_static(&self, nodes: &'static [P<Node>]) -> P<NodeList> {
        P::new(NodeList { loc: OwnedCell::new(undefined_text_range()), nodes })
    }
}

impl NodeList {
    #[inline]
    pub fn nodes(&self) -> &'static [P<Node>] {
        self.nodes
    }
    #[inline]
    pub fn loc(&self) -> TextRange {
        self.loc.get()
    }
    #[inline]
    pub fn pos(&self) -> i32 {
        self.loc.get().pos()
    }
    #[inline]
    pub fn end(&self) -> i32 {
        self.loc.get().end()
    }

    pub fn has_trailing_comma(&self) -> bool {
        let Some(last) = self.nodes.last() else {
            return false;
        };
        last.end() < self.end()
    }

    pub fn clone_list(&self, f: &NodeFactory) -> P<NodeList> {
        let result = f.new_node_list_from_static(self.nodes);
        result.loc.set(self.loc.get());
        result
    }
}

// ModifierList

pub struct ModifierList {
    pub list: NodeList,
    pub modifier_flags: ModifierFlags,
}

impl NodeFactory {
    pub fn new_modifier_list(&self, nodes: Vec<P<Node>>) -> P<ModifierList> {
        let nodes = alloc_vec(nodes);
        P::new(ModifierList {
            list: NodeList { loc: OwnedCell::new(undefined_text_range()), nodes },
            modifier_flags: modifiers_to_flags(nodes),
        })
    }
}

impl ModifierList {
    #[inline]
    pub fn nodes(&self) -> &'static [P<Node>] {
        self.list.nodes
    }
    #[inline]
    pub fn loc(&self) -> TextRange {
        self.list.loc.get()
    }
    #[inline]
    pub fn pos(&self) -> i32 {
        self.list.pos()
    }
    #[inline]
    pub fn end(&self) -> i32 {
        self.list.end()
    }
    #[inline]
    pub fn has_trailing_comma(&self) -> bool {
        self.list.has_trailing_comma()
    }

    pub fn clone_list(&self, f: &NodeFactory) -> P<ModifierList> {
        P::new(ModifierList {
            list: NodeList { loc: OwnedCell::new(self.list.loc.get()), nodes: self.list.nodes },
            modifier_flags: self.modifier_flags,
        })
    }
}

// AST Node

/// Go `ast.Node`. The node's data struct (Go: the struct that embeds `NodeBase`) is allocated together with the
/// header, right after it (`NodeAlloc`); `data_tag()` says which struct it is. `node.data()` returns it as a
/// `NodeData`, `as_*()` and the generated accessors read it in place. Data structs without fields (`Token`,
/// `KeywordTypeNode`, ...) allocate only the header.
///
/// The header is 24 bytes: the kind, the data tag and the parent pointer share one word (`NodeHeaderWord`), and the
/// id is stored in 32 bits (ids still come from a 64-bit counter and are returned as `NodeId`; more than
/// `u32::MAX` node ids panic, like symbol ids). 23M nodes on the private monorepo.
pub struct Node {
    header: OwnedCell<NodeHeaderWord>,
    pub flags: OwnedCell<NodeFlags>,
    pub(crate) id: AtomicU32,
    pub loc: OwnedCell<TextRange>,
}

const _: () = assert!(std::mem::size_of::<Node>() == 24);

/// A node's kind, data tag and parent in one word: the parent's address divided by 8 in the low 45 bits (nodes are
/// 8-aligned and user-space addresses are below 2^48 on every supported platform; checked when the parent is set),
/// the kind in the next 9 bits and the data tag in the 8 above. Only the parent changes after creation. The parent's
/// provenance is exposed when it is stored and recovered with `with_exposed_provenance`.
#[derive(Clone, Copy)]
struct NodeHeaderWord(u64);

impl NodeHeaderWord {
    const PARENT_BITS: u32 = 45;
    const PARENT_MASK: u64 = (1 << Self::PARENT_BITS) - 1;
    const KIND_SHIFT: u32 = Self::PARENT_BITS;
    const KIND_BITS: u32 = 9;
    const TAG_SHIFT: u32 = Self::KIND_SHIFT + Self::KIND_BITS;

    #[inline]
    fn new(kind: Kind, data_tag: NodeDataTag) -> NodeHeaderWord {
        NodeHeaderWord((kind as u16 as u64) << Self::KIND_SHIFT | (data_tag as u8 as u64) << Self::TAG_SHIFT)
    }

    #[inline]
    fn kind(self) -> Kind {
        let v = (self.0 >> Self::KIND_SHIFT) as u16 & ((1 << Self::KIND_BITS) - 1);
        // SAFETY: the bits were stored from a `Kind` (repr(i16), contiguous discriminants 0..=Count < 2^9).
        unsafe { std::mem::transmute::<i16, Kind>(v as i16) }
    }

    #[inline]
    fn data_tag(self) -> NodeDataTag {
        // SAFETY: the bits were stored from a `NodeDataTag` (repr(u8)).
        unsafe { std::mem::transmute::<u8, NodeDataTag>((self.0 >> Self::TAG_SHIFT) as u8) }
    }

    #[inline]
    fn parent(self) -> Option<P<Node>> {
        let addr = ((self.0 & Self::PARENT_MASK) << 3) as usize;
        // SAFETY: a nonzero address was stored from a live `P<Node>` (arena nodes are never freed or moved), whose
        // provenance `with_parent` exposed.
        (addr != 0).then(|| P::from_static(unsafe { &*std::ptr::with_exposed_provenance::<Node>(addr) }))
    }

    #[inline]
    fn with_parent(self, parent: Option<P<Node>>) -> NodeHeaderWord {
        let addr = parent.map_or(0, |p| (p.get() as *const Node).expose_provenance()) as u64;
        assert!(addr & 7 == 0 && addr >> (Self::PARENT_BITS + 3) == 0, "node address {addr:#x} does not fit the node header");
        NodeHeaderWord(self.0 & !Self::PARENT_MASK | addr >> 3)
    }
}

const _: () = assert!((Kind::Count as u64) < 1 << NodeHeaderWord::KIND_BITS);
const _: () = assert!(NodeHeaderWord::TAG_SHIFT + 8 <= 64);

/// One arena allocation per node: the header, then the data struct. `repr(C)` puts the header at offset 0 and
/// the data at `offset_of!(NodeAlloc<T>, data)` (24 for every data struct: none is aligned to more than 8).
#[repr(C)]
pub(crate) struct NodeAlloc<T> {
    node: Node,
    data: T,
}

/// A node data struct with fields, stored after the header of nodes tagged `TAG` (impls are generated).
pub(crate) trait NodePayload: Sized + 'static {
    const TAG: NodeDataTag;
}

// Node accessors. Accessors that dispatch over the node data (name(), modifiers(), *_data(), as_*(),
// for_each_child(), ...) are generated in generated.rs.

impl Node {
    #[inline(always)]
    pub fn kind(&self) -> Kind {
        self.header.get().kind()
    }

    /// Which data struct follows this node's header.
    #[inline(always)]
    pub(crate) fn data_tag(&self) -> NodeDataTag {
        self.header.get().data_tag()
    }

    /// The data struct after this node's header. Callers check `data_tag() == T::TAG` first.
    #[inline(always)]
    pub(crate) fn payload<T: NodePayload>(&self) -> &'static T {
        debug_assert!(self.data_tag() == T::TAG);
        // SAFETY: a node tagged `T::TAG` was allocated by `new_node::<T>` as a `NodeAlloc<T>` whose header is
        // `self`, so its data struct lives at this offset from the header, for the rest of the process.
        unsafe { &*(self as *const Node).cast::<u8>().add(std::mem::offset_of!(NodeAlloc<T>, data)).cast::<T>() }
    }

    /// The arena pointer for this node. Every `Node` is created by `NodeFactory`/`new_node` in the leak
    /// arena (Node has a crate-private field, so it cannot be constructed elsewhere), so `&self` is
    /// always a reference to a `'static` arena value.
    #[inline]
    pub fn as_p(&self) -> P<Node> {
        // SAFETY: see above; nodes are never freed or moved.
        P::from_static(unsafe { &*(self as *const Node) })
    }

    #[inline]
    pub fn as_node(&self) -> P<Node> {
        self.as_p()
    }
    #[inline]
    pub fn pos(&self) -> i32 {
        self.loc.get().pos()
    }
    #[inline]
    pub fn end(&self) -> i32 {
        self.loc.get().end()
    }
    #[inline]
    pub fn loc(&self) -> TextRange {
        self.loc.get()
    }
    #[inline]
    pub fn set_loc(&self, loc: TextRange) {
        self.loc.set(loc)
    }
    #[inline]
    pub fn flags(&self) -> NodeFlags {
        self.flags.get()
    }
    #[inline]
    pub fn set_flags(&self, flags: NodeFlags) {
        self.flags.set(flags)
    }
    #[inline]
    pub fn parent(&self) -> Option<P<Node>> {
        self.header.get().parent()
    }
    #[inline]
    pub fn set_parent(&self, parent: Option<P<Node>>) {
        self.header.set(self.header.get().with_parent(parent))
    }

    /// Go `IterChildren`: the children in `for_each_child` order.
    pub fn iter_children(&self) -> Vec<P<Node>> {
        let mut children = Vec::new();
        self.for_each_child(&mut |child| {
            children.push(child);
            false
        });
        children
    }

    pub fn parameter_list(&self) -> Option<P<NodeList>> {
        self.function_like_data().unwrap().parameters.get()
    }

    pub fn parameters(&self) -> &'static [P<Node>] {
        match self.parameter_list() {
            Some(list) => list.nodes,
            None => &[],
        }
    }

    pub fn kind_string(&self) -> String {
        format!("{:?}", self.kind())
    }

    pub fn kind_value(&self) -> i16 {
        self.kind() as i16
    }

    pub fn decorators(&self) -> Vec<P<Node>> {
        match self.modifiers() {
            None => Vec::new(),
            Some(modifiers) => modifiers.nodes().iter().copied().filter(|&m| is_decorator(m)).collect(),
        }
    }

    /// Go `AsMutable()`: in Rust the mutators live on `Node` directly.
    #[inline]
    pub fn as_mutable(&self) -> &Node {
        self
    }

    pub fn set_modifiers(&self, modifiers: Option<P<ModifierList>>) {
        self.set_modifiers_data(modifiers)
    }

    pub fn symbol(&self) -> Option<P<Symbol>> {
        match self.declaration_data() {
            Some(data) => data.symbol.get(),
            None => None,
        }
    }

    pub fn local_symbol(&self) -> Option<P<Symbol>> {
        match self.exportable_data() {
            Some(data) => data.local_symbol.get(),
            None => None,
        }
    }

    pub fn locals(&self) -> Option<P<SymbolTable>> {
        match self.locals_container_data() {
            Some(data) => data.locals.get(),
            None => None,
        }
    }

    pub fn flow_node(&self) -> Option<P<FlowNode>> {
        match self.flow_node_data() {
            Some(data) => data.flow_node.get(),
            None => None,
        }
    }

    pub fn body(&self) -> Option<P<Node>> {
        match self.body_data() {
            Some(data) => data.body,
            None => None,
        }
    }

    /// Go `Text()`. Joined texts (JsxNamespacedName, JSDoc text) are allocated in the arena.
    pub fn text(&self) -> &'static str {
        match self.kind() {
            Kind::Identifier => self.as_identifier().text(),
            Kind::PrivateIdentifier => self.as_private_identifier().text(),
            Kind::StringLiteral => self.as_string_literal().text(),
            Kind::NumericLiteral => self.as_numeric_literal().text(),
            Kind::BigIntLiteral => self.as_big_int_literal().text(),
            Kind::MetaProperty => self.as_meta_property().name.text(),
            Kind::NoSubstitutionTemplateLiteral => self.as_no_substitution_template_literal().text(),
            Kind::TemplateHead => self.as_template_head().text(),
            Kind::TemplateMiddle => self.as_template_middle().text(),
            Kind::TemplateTail => self.as_template_tail().text(),
            Kind::JsxNamespacedName => {
                let n = self.as_jsx_namespaced_name();
                alloc_str(&format!("{}:{}", n.namespace.text(), n.name.text()))
            }
            Kind::RegularExpressionLiteral => self.as_regular_expression_literal().text(),
            Kind::JSDocText => join_texts(self.as_jsdoc_text().text()),
            Kind::JSDocLink => join_texts(self.as_jsdoc_link().text()),
            Kind::JSDocLinkCode => join_texts(self.as_jsdoc_link_code().text()),
            Kind::JSDocLinkPlain => join_texts(self.as_jsdoc_link_plain().text()),
            _ => panic!("Unhandled case in Node.Text: {:?}", self.kind()),
        }
    }

    pub fn expression(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::PropertyAccessExpression => Some(self.as_property_access_expression().expression),
            Kind::ElementAccessExpression => Some(self.as_element_access_expression().expression),
            Kind::ParenthesizedExpression => Some(self.as_parenthesized_expression().expression.get()),
            Kind::CallExpression => Some(self.as_call_expression().expression),
            Kind::NewExpression => Some(self.as_new_expression().expression),
            Kind::ExpressionWithTypeArguments => Some(self.as_expression_with_type_arguments().expression),
            Kind::ComputedPropertyName => Some(self.as_computed_property_name().expression),
            Kind::NonNullExpression => Some(self.as_non_null_expression().expression),
            Kind::TypeAssertionExpression => Some(self.as_type_assertion().expression),
            Kind::AsExpression => Some(self.as_as_expression().expression),
            Kind::SatisfiesExpression => Some(self.as_satisfies_expression().expression),
            Kind::TypeOfExpression => Some(self.as_type_of_expression().expression),
            Kind::SpreadAssignment => Some(self.as_spread_assignment().expression),
            Kind::SpreadElement => Some(self.as_spread_element().expression),
            Kind::TemplateSpan => Some(self.as_template_span().expression),
            Kind::DeleteExpression => Some(self.as_delete_expression().expression),
            Kind::VoidExpression => Some(self.as_void_expression().expression),
            Kind::AwaitExpression => Some(self.as_await_expression().expression),
            Kind::YieldExpression => self.as_yield_expression().expression,
            Kind::PartiallyEmittedExpression => Some(self.as_partially_emitted_expression().expression),
            Kind::IfStatement => Some(self.as_if_statement().expression),
            Kind::DoStatement => Some(self.as_do_statement().expression),
            Kind::WhileStatement => Some(self.as_while_statement().expression),
            Kind::WithStatement => Some(self.as_with_statement().expression),
            Kind::ForInStatement | Kind::ForOfStatement => Some(self.as_for_in_or_of_statement().expression),
            Kind::SwitchStatement => Some(self.as_switch_statement().expression),
            Kind::CaseClause => self.as_case_or_default_clause().expression,
            Kind::ExpressionStatement => Some(self.as_expression_statement().expression),
            Kind::ReturnStatement => self.as_return_statement().expression.get(),
            Kind::ThrowStatement => Some(self.as_throw_statement().expression),
            Kind::ExternalModuleReference => Some(self.as_external_module_reference().expression),
            Kind::ExportAssignment => Some(self.as_export_assignment().expression.get()),
            Kind::Decorator => Some(self.as_decorator().expression),
            Kind::JsxExpression => self.as_jsx_expression().expression,
            Kind::JsxSpreadAttribute => Some(self.as_jsx_spread_attribute().expression),
            _ => panic!("Unhandled case in Node.Expression: {:?}", self.kind()),
        }
    }

    pub fn raw_text(&self) -> &'static str {
        match self.kind() {
            Kind::TemplateHead => self.as_template_head().raw_text(),
            Kind::TemplateMiddle => self.as_template_middle().raw_text(),
            Kind::TemplateTail => self.as_template_tail().raw_text(),
            _ => panic!("Unhandled case in Node.RawText: {:?}", self.kind()),
        }
    }

    /// Go `MutableNode.SetExpression`. Only the kinds the reparser mutates have a mutable `expression`
    /// field in this port; the other Go cases panic.
    pub fn set_expression(&self, expr: P<Node>) {
        match self.kind() {
            Kind::ParenthesizedExpression => self.as_parenthesized_expression().expression.set(expr),
            Kind::ReturnStatement => self.as_return_statement().expression.set(Some(expr)),
            Kind::ExportAssignment => self.as_export_assignment().expression.set(expr),
            _ => panic!("Unhandled case in mutableNode.SetExpression: {:?}", self.kind()),
        }
    }

    pub fn argument_list(&self) -> Option<P<NodeList>> {
        match self.kind() {
            Kind::CallExpression => Some(self.as_call_expression().arguments),
            Kind::NewExpression => self.as_new_expression().arguments,
            _ => panic!("Unhandled case in Node.Arguments: {:?}", self.kind()),
        }
    }

    pub fn arguments(&self) -> &'static [P<Node>] {
        match self.argument_list() {
            Some(list) => list.nodes,
            None => &[],
        }
    }

    pub fn type_argument_list(&self) -> Option<P<NodeList>> {
        match self.kind() {
            Kind::CallExpression => self.as_call_expression().type_arguments,
            Kind::NewExpression => self.as_new_expression().type_arguments,
            Kind::TaggedTemplateExpression => self.as_tagged_template_expression().type_arguments,
            Kind::TypeReference => self.as_type_reference_node().type_arguments(),
            Kind::ExpressionWithTypeArguments => self.as_expression_with_type_arguments().type_arguments.get(),
            Kind::ImportType => self.as_import_type_node().type_arguments(),
            Kind::TypeQuery => self.as_type_query_node().type_arguments(),
            Kind::JsxOpeningElement => self.as_jsx_opening_element().type_arguments,
            Kind::JsxSelfClosingElement => self.as_jsx_self_closing_element().type_arguments,
            _ => panic!("Unhandled case in Node.TypeArguments"),
        }
    }

    pub fn type_arguments(&self) -> &'static [P<Node>] {
        match self.type_argument_list() {
            Some(list) => list.nodes,
            None => &[],
        }
    }

    pub fn type_parameter_list(&self) -> Option<P<NodeList>> {
        match self.kind() {
            Kind::ClassDeclaration => self.as_class_declaration().type_parameters(),
            Kind::ClassExpression => self.as_class_expression().type_parameters(),
            Kind::InterfaceDeclaration => self.as_interface_declaration().type_parameters,
            Kind::TypeAliasDeclaration | Kind::JSTypeAliasDeclaration => self.as_type_alias_declaration().type_parameters.get(),
            Kind::JSDocTemplateTag => Some(self.as_jsdoc_template_tag().type_parameters),
            _ => {
                if let Some(func_like) = self.function_like_data() {
                    return func_like.type_parameters.get();
                }
                panic!("Unhandled case in Node.TypeParameterList")
            }
        }
    }

    pub fn type_parameters(&self) -> &'static [P<Node>] {
        match self.type_parameter_list() {
            Some(list) => list.nodes,
            None => &[],
        }
    }

    pub fn member_list(&self) -> Option<P<NodeList>> {
        match self.kind() {
            Kind::ClassDeclaration => Some(self.as_class_declaration().members()),
            Kind::ClassExpression => Some(self.as_class_expression().members()),
            Kind::InterfaceDeclaration => Some(self.as_interface_declaration().members),
            Kind::EnumDeclaration => Some(self.as_enum_declaration().members),
            Kind::TypeLiteral => Some(self.as_type_literal_node().members),
            Kind::MappedType => self.as_mapped_type_node().members,
            _ => panic!("Unhandled case in Node.MemberList: {:?}", self.kind()),
        }
    }

    pub fn members(&self) -> &'static [P<Node>] {
        match self.member_list() {
            Some(list) => list.nodes,
            None => &[],
        }
    }

    pub fn statement_list(&self) -> Option<P<NodeList>> {
        match self.kind() {
            Kind::SourceFile => Some(self.as_source_file().statements),
            Kind::Block => Some(self.as_block().statements),
            Kind::ModuleBlock => Some(self.as_module_block().statements),
            Kind::CaseClause | Kind::DefaultClause => Some(self.as_case_or_default_clause().statements),
            _ => panic!("Unhandled case in Node.StatementList: {:?}", self.kind()),
        }
    }

    pub fn statements(&self) -> &'static [P<Node>] {
        match self.statement_list() {
            Some(list) => list.nodes,
            None => &[],
        }
    }

    pub fn can_have_statements(&self) -> bool {
        matches!(self.kind(), Kind::SourceFile | Kind::Block | Kind::ModuleBlock | Kind::CaseClause | Kind::DefaultClause)
    }

    pub fn modifier_flags(&self) -> ModifierFlags {
        match self.modifiers() {
            Some(modifiers) => modifiers.modifier_flags,
            None => ModifierFlags::None,
        }
    }

    pub fn modifier_nodes(&self) -> &'static [P<Node>] {
        match self.modifiers() {
            Some(modifiers) => modifiers.nodes(),
            None => &[],
        }
    }

    /// Go `Type()`.
    pub fn type_node(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::VariableDeclaration => self.as_variable_declaration().type_.get(),
            Kind::Parameter => self.as_parameter_declaration().type_.get(),
            Kind::PropertySignature => self.as_property_signature_declaration().type_.get(),
            Kind::PropertyDeclaration => self.as_property_declaration().type_.get(),
            Kind::PropertyAssignment => self.as_property_assignment().type_.get(),
            Kind::ShorthandPropertyAssignment => self.as_shorthand_property_assignment().type_.get(),
            Kind::TypePredicate => self.as_type_predicate_node().type_,
            Kind::ParenthesizedType => Some(self.as_parenthesized_type_node().type_),
            Kind::TypeOperator => Some(self.as_type_operator_node().type_),
            Kind::MappedType => self.as_mapped_type_node().type_,
            Kind::TypeAssertionExpression => Some(self.as_type_assertion().type_),
            Kind::AsExpression => Some(self.as_as_expression().type_),
            Kind::SatisfiesExpression => Some(self.as_satisfies_expression().type_),
            Kind::TypeAliasDeclaration | Kind::JSTypeAliasDeclaration => self.as_type_alias_declaration().type_.get(),
            Kind::NamedTupleMember => Some(self.as_named_tuple_member().type_),
            Kind::OptionalType => Some(self.as_optional_type_node().type_),
            Kind::RestType => Some(self.as_rest_type_node().type_),
            Kind::TemplateLiteralTypeSpan => Some(self.as_template_literal_type_span().type_),
            Kind::JSDocTypeExpression => Some(self.as_jsdoc_type_expression().type_),
            Kind::JSDocParameterTag | Kind::JSDocPropertyTag => self.as_jsdoc_parameter_or_property_tag().type_expression,
            Kind::JSDocNullableType => Some(self.as_jsdoc_nullable_type().type_),
            Kind::JSDocNonNullableType => Some(self.as_jsdoc_non_nullable_type().type_),
            Kind::JSDocOptionalType => Some(self.as_jsdoc_optional_type().type_),
            Kind::ExportAssignment => self.as_export_assignment().type_.get(),
            Kind::BinaryExpression => self.as_binary_expression().type_.get(),
            _ => {
                if let Some(func_like) = self.function_like_data() {
                    return func_like.type_.get();
                }
                None
            }
        }
    }

    /// Go `MutableNode.SetType`. Only the kinds the reparser mutates have a mutable type field in this
    /// port; the other Go cases panic.
    pub fn set_type(&self, t: Option<P<Node>>) {
        match self.kind() {
            Kind::VariableDeclaration => self.as_variable_declaration().type_.set(t),
            Kind::Parameter => self.as_parameter_declaration().type_.set(t),
            Kind::PropertySignature => self.as_property_signature_declaration().type_.set(t),
            Kind::PropertyDeclaration => self.as_property_declaration().type_.set(t),
            Kind::PropertyAssignment => self.as_property_assignment().type_.set(t),
            Kind::ShorthandPropertyAssignment => self.as_shorthand_property_assignment().type_.set(t),
            Kind::TypeAliasDeclaration | Kind::JSTypeAliasDeclaration => self.as_type_alias_declaration().type_.set(t),
            Kind::ExportAssignment => self.as_export_assignment().type_.set(t),
            Kind::BinaryExpression => self.as_binary_expression().type_.set(t),
            _ => {
                if let Some(func_like) = self.function_like_data() {
                    func_like.type_.set(t);
                } else {
                    panic!("Unhandled case in mutableNode.SetType: {:?}", self.kind());
                }
            }
        }
    }

    pub fn initializer(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::VariableDeclaration => self.as_variable_declaration().initializer.get(),
            Kind::Parameter => self.as_parameter_declaration().initializer,
            Kind::BindingElement => self.as_binding_element().initializer,
            Kind::PropertyDeclaration => self.as_property_declaration().initializer.get(),
            Kind::PropertySignature => self.as_property_signature_declaration().initializer,
            Kind::PropertyAssignment => Some(self.as_property_assignment().initializer.get()),
            Kind::EnumMember => self.as_enum_member().initializer,
            Kind::ForStatement => self.as_for_statement().initializer,
            Kind::ForInStatement | Kind::ForOfStatement => Some(self.as_for_in_or_of_statement().initializer),
            Kind::JsxAttribute => self.as_jsx_attribute().initializer,
            _ => panic!("Unhandled case in Node.Initializer"),
        }
    }

    /// Go `MutableNode.SetInitializer`. Only the kinds the reparser mutates are supported.
    pub fn set_initializer(&self, initializer: P<Node>) {
        match self.kind() {
            Kind::VariableDeclaration => self.as_variable_declaration().initializer.set(Some(initializer)),
            Kind::PropertyDeclaration => self.as_property_declaration().initializer.set(Some(initializer)),
            Kind::PropertyAssignment => self.as_property_assignment().initializer.set(initializer),
            _ => panic!("Unhandled case in mutableNode.SetInitializer"),
        }
    }

    pub fn tag_name(&self) -> P<Node> {
        match self.kind() {
            Kind::JsxOpeningElement => self.as_jsx_opening_element().tag_name,
            Kind::JsxClosingElement => self.as_jsx_closing_element().tag_name,
            Kind::JsxSelfClosingElement => self.as_jsx_self_closing_element().tag_name,
            Kind::JSDocUnknownTag
            | Kind::JSDocAugmentsTag
            | Kind::JSDocImplementsTag
            | Kind::JSDocDeprecatedTag
            | Kind::JSDocPublicTag
            | Kind::JSDocPrivateTag
            | Kind::JSDocProtectedTag
            | Kind::JSDocReadonlyTag
            | Kind::JSDocOverrideTag
            | Kind::JSDocCallbackTag
            | Kind::JSDocOverloadTag
            | Kind::JSDocParameterTag
            | Kind::JSDocPropertyTag
            | Kind::JSDocReturnTag
            | Kind::JSDocThisTag
            | Kind::JSDocTypeTag
            | Kind::JSDocTemplateTag
            | Kind::JSDocTypedefTag
            | Kind::JSDocSeeTag
            | Kind::JSDocSatisfiesTag
            | Kind::JSDocThrowsTag
            | Kind::JSDocImportTag => self.jsdoc_tag_base().tag_name,
            _ => panic!("Unhandled case in Node.TagName: {:?}", self.kind()),
        }
    }

    fn jsdoc_tag_base(&self) -> &'static JSDocTagBase {
        match self.data() {
            NodeData::JSDocUnknownTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocAugmentsTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocImplementsTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocDeprecatedTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocPublicTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocPrivateTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocProtectedTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocReadonlyTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocOverrideTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocCallbackTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocOverloadTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocParameterOrPropertyTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocReturnTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocThisTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocTypeTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocTemplateTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocTypedefTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocSeeTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocSatisfiesTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocThrowsTag(d) => &d.jsdoc_tag_base,
            NodeData::JSDocImportTag(d) => &d.jsdoc_tag_base,
            _ => panic!("not a JSDoc tag: {:?}", self.kind()),
        }
    }

    pub fn property_name(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::ImportSpecifier => self.as_import_specifier().property_name,
            Kind::ExportSpecifier => self.as_export_specifier().property_name,
            Kind::BindingElement => self.as_binding_element().property_name,
            _ => None,
        }
    }

    pub fn property_name_or_name(&self) -> Option<P<Node>> {
        let mut name = self.property_name();
        if name.is_none() {
            name = self.name();
        }
        name
    }

    pub fn is_type_only(&self) -> bool {
        match self.kind() {
            Kind::ImportEqualsDeclaration => self.as_import_equals_declaration().is_type_only,
            Kind::ImportSpecifier => self.as_import_specifier().is_type_only,
            Kind::ImportClause => self.as_import_clause().phase_modifier.get() == Kind::TypeKeyword,
            Kind::ExportDeclaration => self.as_export_declaration().is_type_only,
            Kind::ExportSpecifier => self.as_export_specifier().is_type_only,
            _ => false,
        }
    }

    // If updating this function, also update `hasComment`.
    pub fn comment_list(&self) -> Option<P<NodeList>> {
        match self.kind() {
            Kind::JSDoc => Some(self.as_jsdoc().comment),
            Kind::JSDocUnknownTag
            | Kind::JSDocAugmentsTag
            | Kind::JSDocImplementsTag
            | Kind::JSDocDeprecatedTag
            | Kind::JSDocPublicTag
            | Kind::JSDocPrivateTag
            | Kind::JSDocProtectedTag
            | Kind::JSDocReadonlyTag
            | Kind::JSDocOverrideTag
            | Kind::JSDocCallbackTag
            | Kind::JSDocOverloadTag
            | Kind::JSDocParameterTag
            | Kind::JSDocPropertyTag
            | Kind::JSDocReturnTag
            | Kind::JSDocThisTag
            | Kind::JSDocTypeTag
            | Kind::JSDocTemplateTag
            | Kind::JSDocTypedefTag
            | Kind::JSDocSeeTag
            | Kind::JSDocSatisfiesTag
            | Kind::JSDocThrowsTag
            | Kind::JSDocImportTag => self.jsdoc_tag_base().comment,
            _ => panic!("Unhandled case in Node.CommentList: {:?}", self.kind()),
        }
    }

    pub fn comments(&self) -> &'static [P<Node>] {
        match self.comment_list() {
            Some(list) => list.nodes,
            None => &[],
        }
    }

    pub fn label(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::LabeledStatement => Some(self.as_labeled_statement().label),
            Kind::BreakStatement => self.as_break_statement().label,
            Kind::ContinueStatement => self.as_continue_statement().label,
            _ => panic!("Unhandled case in Node.Label: {:?}", self.kind()),
        }
    }

    pub fn attributes(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::JsxOpeningElement => Some(self.as_jsx_opening_element().attributes),
            Kind::JsxSelfClosingElement => Some(self.as_jsx_self_closing_element().attributes),
            Kind::ModuleDeclaration => self.as_module_declaration().attributes,
            _ => panic!("Unhandled case in Node.Attributes: {:?}", self.kind()),
        }
    }

    pub fn children(&self) -> P<NodeList> {
        match self.kind() {
            Kind::JsxElement => self.as_jsx_element().children,
            Kind::JsxFragment => self.as_jsx_fragment().children,
            _ => panic!("Unhandled case in Node.Children: {:?}", self.kind()),
        }
    }

    pub fn module_specifier(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::ImportDeclaration | Kind::JSImportDeclaration => Some(self.as_import_declaration().module_specifier),
            Kind::ExportDeclaration => self.as_export_declaration().module_specifier,
            Kind::JSDocImportTag => Some(self.as_jsdoc_import_tag().module_specifier),
            _ => panic!("Unhandled case in Node.ModuleSpecifier: {:?}", self.kind()),
        }
    }

    pub fn import_clause(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::ImportDeclaration | Kind::JSImportDeclaration => self.as_import_declaration().import_clause,
            Kind::JSDocImportTag => self.as_jsdoc_import_tag().import_clause,
            _ => panic!("Unhandled case in Node.ImportClause: {:?}", self.kind()),
        }
    }

    pub fn statement(&self) -> P<Node> {
        match self.kind() {
            Kind::DoStatement => self.as_do_statement().statement(),
            Kind::WhileStatement => self.as_while_statement().statement(),
            Kind::ForStatement => self.as_for_statement().statement(),
            Kind::ForInStatement | Kind::ForOfStatement => self.as_for_in_or_of_statement().statement,
            Kind::WithStatement => self.as_with_statement().statement,
            Kind::LabeledStatement => self.as_labeled_statement().statement,
            _ => panic!("Unhandled case in Node.Statement: {:?}", self.kind()),
        }
    }

    pub fn property_list(&self) -> P<NodeList> {
        match self.kind() {
            Kind::ObjectLiteralExpression => self.as_object_literal_expression().properties,
            Kind::JsxAttributes => self.as_jsx_attributes().properties,
            _ => panic!("Unhandled case in Node.PropertyList: {:?}", self.kind()),
        }
    }

    pub fn properties(&self) -> &'static [P<Node>] {
        self.property_list().nodes
    }

    pub fn element_list(&self) -> P<NodeList> {
        match self.kind() {
            Kind::NamedImports => self.as_named_imports().elements,
            Kind::NamedExports => self.as_named_exports().elements,
            Kind::ObjectBindingPattern | Kind::ArrayBindingPattern => self.as_binding_pattern().elements,
            Kind::ArrayLiteralExpression => self.as_array_literal_expression().elements,
            Kind::TupleType => self.as_tuple_type_node().elements,
            _ => panic!("Unhandled case in Node.ElementList: {:?}", self.kind()),
        }
    }

    pub fn elements(&self) -> &'static [P<Node>] {
        self.element_list().nodes
    }

    pub fn postfix_token(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::MethodDeclaration => self.as_method_declaration().postfix_token(),
            Kind::ShorthandPropertyAssignment => self.as_shorthand_property_assignment().postfix_token(),
            Kind::MethodSignature => self.as_method_signature_declaration().postfix_token(),
            Kind::PropertySignature => self.as_property_signature_declaration().postfix_token(),
            Kind::PropertyAssignment => self.as_property_assignment().postfix_token(),
            Kind::PropertyDeclaration => self.as_property_declaration().postfix_token(),
            Kind::EnumMember => self.as_enum_member().postfix_token(),
            Kind::GetAccessor => self.as_get_accessor_declaration().postfix_token(),
            Kind::SetAccessor => self.as_set_accessor_declaration().postfix_token(),
            _ => None,
        }
    }

    pub fn question_token(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::Parameter => return self.as_parameter_declaration().question_token.get(),
            Kind::ConditionalExpression => return Some(self.as_conditional_expression().question_token),
            Kind::MappedType => return self.as_mapped_type_node().question_token,
            Kind::NamedTupleMember => return self.as_named_tuple_member().question_token,
            _ => {}
        }
        let postfix = self.postfix_token();
        if let Some(postfix) = postfix {
            if postfix.kind() == Kind::QuestionToken {
                return Some(postfix);
            }
        }
        None
    }

    pub fn question_dot_token(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::ElementAccessExpression => self.as_element_access_expression().question_dot_token,
            Kind::PropertyAccessExpression => self.as_property_access_expression().question_dot_token,
            Kind::CallExpression => self.as_call_expression().question_dot_token,
            Kind::TaggedTemplateExpression => self.as_tagged_template_expression().question_dot_token,
            _ => panic!("Unhandled case in Node.QuestionDotToken: {:?}", self.kind()),
        }
    }

    pub fn type_expression(&self) -> Option<P<Node>> {
        match self.kind() {
            Kind::JSDocParameterTag | Kind::JSDocPropertyTag => self.as_jsdoc_parameter_or_property_tag().type_expression,
            Kind::JSDocReturnTag => self.as_jsdoc_return_tag().type_expression,
            Kind::JSDocTypeTag => Some(self.as_jsdoc_type_tag().type_expression),
            Kind::JSDocTypedefTag => self.as_jsdoc_typedef_tag().type_expression,
            Kind::JSDocCallbackTag => Some(self.as_jsdoc_callback_tag().type_expression),
            Kind::JSDocSatisfiesTag => Some(self.as_jsdoc_satisfies_tag().type_expression),
            Kind::JSDocThrowsTag => self.as_jsdoc_throws_tag().type_expression,
            _ => panic!("Unhandled case in Node.TypeExpression: {:?}", self.kind()),
        }
    }

    pub fn class_name(&self) -> P<Node> {
        match self.kind() {
            Kind::JSDocAugmentsTag => self.as_jsdoc_augments_tag().class_name,
            Kind::JSDocImplementsTag => self.as_jsdoc_implements_tag().class_name,
            _ => panic!("Unhandled case in Node.ClassName: {:?}", self.kind()),
        }
    }

    // Determines if `n` contains `descendant` by walking up the `Parent` pointers from `descendant`. This method panics if
    // `descendant` or one of its ancestors is not parented except when that node is a `SourceFile`.
    pub fn contains(&self, descendant: Option<P<Node>>) -> bool {
        let mut descendant = descendant;
        while let Some(d) = descendant {
            if std::ptr::eq(d.get(), self) {
                return true;
            }
            let parent = d.parent();
            if parent.is_none() && !is_source_file(d) {
                panic!("descendant is not parented");
            }
            descendant = parent;
        }
        false
    }

    pub fn is_jsdoc(&self) -> bool {
        self.kind() == Kind::JSDoc
    }

    // if you provide nil for file, this code will walk to the root of the tree to find the file
    pub fn jsdoc(&self, file: Option<&'static SourceFile>) -> &'static [P<Node>] {
        if !self.flags.get().intersects(NodeFlags::HasJSDoc) {
            return &[];
        }
        let file = match file {
            Some(file) => file,
            None => match get_source_file_of_node(self.as_p()) {
                Some(file) => file.get(),
                None => return &[],
            },
        };
        if file.has_lazy_jsdoc.get() {
            return file.resolve_jsdoc(self.as_p());
        }
        file.jsdoc_cache.borrow().get(&self.as_p()).copied().unwrap_or(&[])
    }

    // EagerJSDoc returns JSDoc nodes that have already been parsed and cached,
    // without triggering lazy JSDoc parsing.
    pub fn eager_jsdoc(&self, file: Option<&'static SourceFile>) -> &'static [P<Node>] {
        if !self.flags.get().intersects(NodeFlags::HasJSDoc) {
            return &[];
        }
        let file = match file {
            Some(file) => file,
            None => match get_source_file_of_node(self.as_p()) {
                Some(file) => file.get(),
                None => return &[],
            },
        };
        if file.has_lazy_jsdoc.get() {
            let _guard = file.jsdoc_mu.read().unwrap();
            return file.jsdoc_cache.borrow().get(&self.as_p()).copied().unwrap_or(&[]);
        }
        file.jsdoc_cache.borrow().get(&self.as_p()).copied().unwrap_or(&[])
    }
}

fn join_texts(texts: &[&'static str]) -> &'static str {
    if texts.len() == 1 {
        return texts[0];
    }
    alloc_str(&texts.concat())
}

pub fn is_write_only_access(node: P<Node>) -> bool {
    access_kind(node) == AccessKind::Write
}

pub fn is_write_access(node: P<Node>) -> bool {
    access_kind(node) != AccessKind::Read
}

pub fn is_write_access_for_reference(node: P<Node>) -> bool {
    let decl = get_declaration_from_name(Some(node));
    (decl.is_some() && declaration_is_write_access(decl)) || node.kind() == Kind::DefaultKeyword || is_write_access(node)
}

pub fn get_declaration_from_name(name: Option<P<Node>>) -> Option<P<Node>> {
    let name = name?;
    let parent = name.parent()?;
    let mut fallthrough = false;
    match name.kind() {
        Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral | Kind::NumericLiteral => {
            if is_computed_property_name(parent) {
                return parent.parent();
            }
            fallthrough = true;
        }
        Kind::Identifier => fallthrough = true,
        Kind::PrivateIdentifier => {
            if is_declaration(parent) && parent.name() == Some(name) {
                return Some(parent);
            }
        }
        _ => {}
    }
    if fallthrough {
        if is_declaration(parent) {
            if parent.name() == Some(name) {
                return Some(parent);
            }
            return None;
        }
        if is_qualified_name(parent) {
            let tag = parent.parent();
            if let Some(tag) = tag {
                if is_jsdoc_parameter_tag(tag) && tag.name() == Some(parent) {
                    return Some(tag);
                }
            }
            return None;
        }
        let bin_exp = parent.parent();
        if let Some(bin_exp) = bin_exp {
            if is_binary_expression(bin_exp) && get_assignment_declaration_kind(bin_exp) != JSDeclarationKind::None {
                // (binExp.left as BindableStaticNameExpression).symbol || binExp.symbol
                let left_has_symbol = bin_exp.as_binary_expression().left.symbol().is_some();
                if left_has_symbol || bin_exp.symbol().is_some() {
                    if get_name_of_declaration(Some(bin_exp)) == Some(name) {
                        return Some(bin_exp);
                    }
                }
            }
        }
    }
    None
}

fn declaration_is_write_access(decl: Option<P<Node>>) -> bool {
    let Some(decl) = decl else {
        return false;
    };
    // Consider anything in an ambient declaration to be a write access since it may be coming from JS.
    if decl.flags.get().intersects(NodeFlags::Ambient) {
        return true;
    }

    match decl.kind() {
        Kind::BinaryExpression
        | Kind::BindingElement
        | Kind::ClassDeclaration
        | Kind::ClassExpression
        | Kind::DefaultKeyword
        | Kind::EnumDeclaration
        | Kind::EnumMember
        | Kind::ExportSpecifier
        | Kind::ImportClause // default import
        | Kind::ImportEqualsDeclaration
        | Kind::ImportSpecifier
        | Kind::InterfaceDeclaration
        | Kind::JSDocCallbackTag
        | Kind::JSDocTypedefTag
        | Kind::JsxAttribute
        | Kind::ModuleDeclaration
        | Kind::NamespaceExportDeclaration
        | Kind::NamespaceImport
        | Kind::NamespaceExport
        | Kind::Parameter
        | Kind::ShorthandPropertyAssignment
        | Kind::TypeAliasDeclaration
        | Kind::JSTypeAliasDeclaration
        | Kind::TypeParameter => true,

        // In `({ x: y } = 0);`, `x` is not a write access.
        Kind::PropertyAssignment => !is_array_literal_or_object_literal_destructuring_pattern(decl.parent()),

        // functions considered write if they provide a value (have a body)
        Kind::FunctionDeclaration
        | Kind::FunctionExpression
        | Kind::Constructor
        | Kind::MethodDeclaration
        | Kind::GetAccessor
        | Kind::SetAccessor => decl.body().is_some(),

        // variable/property write if initializer present or is in catch clause
        Kind::VariableDeclaration | Kind::PropertyDeclaration => {
            let has_init = match decl.kind() {
                Kind::VariableDeclaration => decl.as_variable_declaration().initializer.get().is_some(),
                _ => decl.as_property_declaration().initializer.get().is_some(),
            };
            has_init || decl.parent().is_some_and(is_catch_clause)
        }

        Kind::MethodSignature | Kind::PropertySignature | Kind::JSDocPropertyTag | Kind::JSDocParameterTag => false,

        // preserve TS behavior: crash on unexpected kinds
        _ => panic!("Unhandled case in declarationIsWriteAccess"),
    }
}

pub fn is_array_literal_or_object_literal_destructuring_pattern(node: Option<P<Node>>) -> bool {
    let Some(node) = node else {
        return false;
    };
    if !(is_array_literal_expression(node) || is_object_literal_expression(node)) {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    // [a,b,c] from:
    // [a, b, c] = someExpression;
    if is_binary_expression(parent)
        && parent.as_binary_expression().left == node
        && parent.as_binary_expression().operator_token.kind() == Kind::EqualsToken
    {
        return true;
    }
    // [a, b, c] from:
    // for([a, b, c] of expression)
    if is_for_of_statement(parent) && parent.initializer() == Some(node) {
        return true;
    }
    // {x, a: {a, b, c} } = someExpression
    if is_property_assignment(parent) {
        return is_array_literal_or_object_literal_destructuring_pattern(parent.parent());
    }
    // [a, b, c] of
    // [x, [a, b, c] ] = someExpression
    is_array_literal_or_object_literal_destructuring_pattern(Some(parent))
}

fn access_kind(node: P<Node>) -> AccessKind {
    let Some(parent) = node.parent() else {
        return AccessKind::Read;
    };
    match parent.kind() {
        Kind::ParenthesizedExpression => access_kind(parent),
        Kind::PrefixUnaryExpression => {
            let operator = parent.as_prefix_unary_expression().operator;
            if operator == Kind::PlusPlusToken || operator == Kind::MinusMinusToken {
                return AccessKind::ReadWrite;
            }
            AccessKind::Read
        }
        Kind::PostfixUnaryExpression => {
            let operator = parent.as_postfix_unary_expression().operator;
            if operator == Kind::PlusPlusToken || operator == Kind::MinusMinusToken {
                return AccessKind::ReadWrite;
            }
            AccessKind::Read
        }
        Kind::BinaryExpression => {
            if parent.as_binary_expression().left == node {
                let operator = parent.as_binary_expression().operator_token;
                if is_assignment_operator(operator.kind()) {
                    if operator.kind() == Kind::EqualsToken {
                        return AccessKind::Write;
                    }
                    return AccessKind::ReadWrite;
                }
            }
            AccessKind::Read
        }
        Kind::PropertyAccessExpression => {
            if parent.as_property_access_expression().name != node {
                return AccessKind::Read;
            }
            access_kind(parent)
        }
        Kind::PropertyAssignment => {
            let parent_access = access_kind(parent.parent().unwrap());
            // In `({ x: varname }) = { x: 1 }`, the left `x` is a read, the right `x` is a write.
            if node == parent.as_property_assignment().name() {
                return reverse_access_kind(parent_access);
            }
            parent_access
        }
        Kind::ShorthandPropertyAssignment => {
            // Assume it's the local variable being accessed, since we don't check public properties for --noUnusedLocals.
            if Some(node) == parent.as_shorthand_property_assignment().object_assignment_initializer.get() {
                return AccessKind::Read;
            }
            access_kind(parent.parent().unwrap())
        }
        Kind::ArrayLiteralExpression => access_kind(parent),
        Kind::ForInStatement | Kind::ForOfStatement => {
            if node == parent.as_for_in_or_of_statement().initializer {
                return AccessKind::Write;
            }
            AccessKind::Read
        }
        _ => AccessKind::Read,
    }
}

fn reverse_access_kind(a: AccessKind) -> AccessKind {
    match a {
        AccessKind::Read => AccessKind::Write,
        AccessKind::Write => AccessKind::Read,
        AccessKind::ReadWrite => AccessKind::ReadWrite,
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum AccessKind {
    Read,      // Only reads from a variable
    Write,     // Only writes to a variable without ever reading it. E.g.: `x=1;`.
    ReadWrite, // Reads from and writes to a variable. E.g.: `f(x++);`, `x/=1`.
}

pub fn is_declaration_node(node: P<Node>) -> bool {
    node.declaration_data().is_some()
}

pub fn is_locals_container(node: P<Node>) -> bool {
    node.locals_container_data().is_some()
}

pub fn is_type_or_js_type_alias_declaration(node: P<Node>) -> bool {
    node.kind() == Kind::TypeAliasDeclaration || node.kind() == Kind::JSTypeAliasDeclaration
}

pub fn is_import_declaration_or_js_import_declaration(node: P<Node>) -> bool {
    node.kind() == Kind::ImportDeclaration || node.kind() == Kind::JSImportDeclaration
}

pub fn is_any_export_assignment(node: P<Node>) -> bool {
    node.kind() == Kind::ExportAssignment
}

impl NodeFactory {
    pub fn new_modifier(&self, kind: Kind) -> P<Node> {
        self.new_token(kind)
    }
}

impl ImportAttributes {
    /// Go `(*ImportAttributesNode).GetResolutionModeOverride`; `node` is the ImportAttributes node or None.
    pub fn get_resolution_mode_override(
        node: Option<P<Node>>,
        mut grammar_error_on_node: Option<&mut dyn FnMut(P<Node>, &'static diagnostics::Message, &[&dyn std::fmt::Display]) -> bool>,
    ) -> Option<ResolutionMode> {
        let node = node?;
        let attributes = node.as_import_attributes().attributes;
        let attribute = attributes.nodes.iter().copied().find(|attribute| attribute.name().unwrap().text() == "resolution-mode")?;
        let elem = attribute.as_import_attribute();
        if !is_string_literal_like(elem.value) {
            return None;
        }
        let text = elem.value.text();
        if text != "import" && text != "require" {
            if let Some(f) = grammar_error_on_node.as_mut() {
                f(elem.value, &diagnostics::X_resolution_mode_should_be_either_require_or_import, &[]);
            }
            return None;
        }
        if text == "import" {
            Some(tsrs_core::RESOLUTION_MODE_ESM)
        } else {
            Some(tsrs_core::ModuleKind::CommonJS)
        }
    }
}

/// Free-function spelling of `ImportAttributes::get_resolution_mode_override` (Go's nil-safe method on
/// `*ImportAttributesNode`).
pub fn get_resolution_mode_override(
    node: Option<P<Node>>,
    grammar_error_on_node: Option<&mut dyn FnMut(P<Node>, &'static diagnostics::Message, &[&dyn std::fmt::Display]) -> bool>,
) -> Option<ResolutionMode> {
    ImportAttributes::get_resolution_mode_override(node, grammar_error_on_node)
}

// PatternAmbientModule

pub struct PatternAmbientModule {
    pub pattern: tsrs_core::Pattern,
    pub symbol: P<Symbol>,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum CommentDirectiveKind {
    #[default]
    Unknown,
    ExpectError,
    Ignore,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CommentDirective {
    pub loc: TextRange,
    pub kind: CommentDirectiveKind,
}

// SourceFile

#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct SourceFileMetaData {
    pub package_json_type: String,
    pub package_json_directory: String,
    pub implied_node_format: ResolutionMode,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CheckJsDirective {
    pub enabled: bool,
    pub range: CommentRange,
}

pub trait HasFileName {
    fn file_name(&self) -> &str;
    fn path(&self) -> &Path;
}

pub struct SourceFile {
    node: OwnedCell<Option<P<Node>>>, // back pointer to the SourceFile node, set by the factory
    pub declaration_base: DeclarationBase,
    pub locals_container_base: LocalsContainerBase,

    // Fields set by NewSourceFile
    parse_options: SourceFileParseOptions,
    text: &'static str,
    pub statements: P<NodeList>,       // NodeList[*Statement]
    pub end_of_file_token: P<Node>, // TokenNode[*EndOfFileToken]

    // Fields set by parser
    pub diagnostics: OwnedCell<&'static [P<Diagnostic>]>,
    pub js_diagnostics: OwnedCell<&'static [P<Diagnostic>]>,
    pub jsdoc_diagnostics: OwnedCell<&'static [P<Diagnostic>]>,
    pub language_variant: OwnedCell<LanguageVariant>,
    pub script_kind: OwnedCell<ScriptKind>,
    pub is_declaration_file: OwnedCell<bool>,
    pub uses_uri_style_node_core_modules: OwnedCell<Tristate>,
    pub identifier_count: OwnedCell<usize>,
    pub imports: OwnedCell<&'static [P<Node>]>,              // []LiteralLikeNode
    pub module_augmentations: OwnedCell<&'static [P<Node>]>, // []ModuleName
    pub ambient_module_names: OwnedCell<&'static [&'static str]>,
    pub comment_directives: OwnedCell<&'static [CommentDirective]>,
    // Written by the parser; with lazy JSDoc, also by any checker thread under `jsdoc_mu` (Go `jsdocMu`).
    pub(crate) jsdoc_cache: FrozenCell<FxHashMap<P<Node>, &'static [P<Node>]>>,
    jsdoc_mu: RwLock<()>,
    pub(crate) has_lazy_jsdoc: OwnedCell<bool>,
    identifiers: OnceLock<Set<&'static str>>,
    pub reparsed_clones: OwnedCell<&'static [P<Node>]>,
    pub pragmas: OwnedCell<&'static [Pragma]>,
    pub referenced_files: OwnedCell<&'static [P<FileReference>]>,
    pub type_reference_directives: OwnedCell<&'static [P<FileReference>]>,
    pub lib_reference_directives: OwnedCell<&'static [P<FileReference>]>,
    pub check_js_directive: OwnedCell<Option<P<CheckJsDirective>>>,
    pub node_count: OwnedCell<usize>,
    pub text_count: OwnedCell<usize>,
    pub common_js_module_indicator: OwnedCell<Option<P<Node>>>,
    // If this is the SourceFile itself, then this module was "forced"
    // to be an external module (previously "true").
    pub external_module_indicator: OwnedCell<Option<P<Node>>>,

    // Fields set by binder
    is_bound: AtomicBool,
    bind_once: Once,
    pub bind_diagnostics: OwnedCell<&'static [P<Diagnostic>]>,
    pub symbol_count: OwnedCell<usize>,
    pub pattern_ambient_modules: OwnedCell<&'static [P<PatternAmbientModule>]>,
    pub global_exports: OwnedCell<Option<P<SymbolTable>>>,

    // Fields set by ECMALineMap
    ecma_line_map: OnceLock<&'static [TextPos]>,

    // Fields for UTF-8 to UTF-16 position mapping
    position_map: OnceLock<P<PositionMap>>,
}

impl NodeFactory {
    pub fn new_source_file(
        &self,
        opts: SourceFileParseOptions,
        text: &'static str,
        statements: P<NodeList>,
        end_of_file_token: P<Node>,
    ) -> P<Node> {
        if tsrs_core::tspath::get_encoded_root_length(&opts.file_name) == 0
            || opts.file_name != tsrs_core::tspath::normalize_path(&opts.file_name)
        {
            panic!("fileName should be normalized and absolute: {:?}", opts.file_name);
        }
        let node = self.new_node(Kind::SourceFile, SourceFile {
            node: OwnedCell::new(None),
            declaration_base: DeclarationBase { symbol: OwnedCell::new(None) },
            locals_container_base: LocalsContainerBase { locals: OwnedCell::new(None), next_container: OwnedCell::new(None) },
            parse_options: opts,
            text,
            statements,
            end_of_file_token,
            diagnostics: OwnedCell::new(&[]),
            js_diagnostics: OwnedCell::new(&[]),
            jsdoc_diagnostics: OwnedCell::new(&[]),
            language_variant: OwnedCell::new(LanguageVariant::default()),
            script_kind: OwnedCell::new(ScriptKind::default()),
            is_declaration_file: OwnedCell::new(false),
            uses_uri_style_node_core_modules: OwnedCell::new(Tristate::Unknown),
            identifier_count: OwnedCell::new(0),
            imports: OwnedCell::new(&[]),
            module_augmentations: OwnedCell::new(&[]),
            ambient_module_names: OwnedCell::new(&[]),
            comment_directives: OwnedCell::new(&[]),
            jsdoc_cache: FrozenCell::new(FxHashMap::default()),
            jsdoc_mu: RwLock::new(()),
            has_lazy_jsdoc: OwnedCell::new(false),
            identifiers: OnceLock::new(),
            reparsed_clones: OwnedCell::new(&[]),
            pragmas: OwnedCell::new(&[]),
            referenced_files: OwnedCell::new(&[]),
            type_reference_directives: OwnedCell::new(&[]),
            lib_reference_directives: OwnedCell::new(&[]),
            check_js_directive: OwnedCell::new(None),
            node_count: OwnedCell::new(0),
            text_count: OwnedCell::new(0),
            common_js_module_indicator: OwnedCell::new(None),
            external_module_indicator: OwnedCell::new(None),
            is_bound: AtomicBool::new(false),
            bind_once: Once::new(),
            bind_diagnostics: OwnedCell::new(&[]),
            symbol_count: OwnedCell::new(0),
            pattern_ambient_modules: OwnedCell::new(&[]),
            global_exports: OwnedCell::new(None),
            ecma_line_map: OnceLock::new(),
            position_map: OnceLock::new(),
        });
        node.as_source_file().node.set(Some(node));
        node
    }
}

impl SourceFile {
    /// The `SourceFile` node (Go `file.AsNode()`).
    #[inline]
    pub fn as_node(&self) -> P<Node> {
        self.node.get().unwrap()
    }

    /// `P<SourceFile>` for this file.
    #[inline]
    pub fn as_p(&self) -> P<SourceFile> {
        self.as_node().as_source_file_p()
    }

    pub fn parse_options(&self) -> &SourceFileParseOptions {
        &self.parse_options
    }

    pub fn text(&self) -> &'static str {
        self.text
    }

    // Content mappers are not ported: every file is its own original.

    // OriginalText returns the untransformed source text for content-mapped files, or Text() otherwise.
    pub fn original_text(&self) -> &'static str {
        self.text
    }

    // OriginalFileName returns the canonical filename associated with a supplemental source file, or FileName() otherwise.
    pub fn original_file_name(&self) -> &str {
        self.file_name()
    }

    pub fn is_content_mapped(&self) -> bool {
        false
    }

    pub fn content_mapper(&self) -> &'static str {
        ""
    }

    pub fn is_content_mapper_failure_stub(&self) -> bool {
        false
    }

    pub fn canonical_source_file(&self) -> Option<P<SourceFile>> {
        None
    }

    pub fn is_content_mapper_supplemental(&self) -> bool {
        false
    }

    pub fn has_identifier(&self, name: &str) -> bool {
        self.identifiers.get_or_init(|| collect_identifiers_for_source_file(self)).keys().contains(name)
    }

    pub fn file_name(&self) -> &str {
        &self.parse_options.file_name
    }

    pub fn path(&self) -> &Path {
        &self.parse_options.path
    }

    pub fn imports(&self) -> &'static [P<Node>] {
        self.imports.get()
    }

    pub fn diagnostics(&self) -> &'static [P<Diagnostic>] {
        self.diagnostics.get()
    }

    pub fn set_diagnostics(&self, diags: &[P<Diagnostic>]) {
        self.diagnostics.set(alloc_slice(diags))
    }

    pub fn js_diagnostics(&self) -> &'static [P<Diagnostic>] {
        self.js_diagnostics.get()
    }

    pub fn set_js_diagnostics(&self, diags: &[P<Diagnostic>]) {
        self.js_diagnostics.set(alloc_slice(diags))
    }

    pub fn jsdoc_diagnostics(&self) -> &'static [P<Diagnostic>] {
        self.jsdoc_diagnostics.get()
    }

    pub fn set_jsdoc_diagnostics(&self, diags: &[P<Diagnostic>]) {
        self.jsdoc_diagnostics.set(alloc_slice(diags))
    }

    pub fn set_jsdoc_cache(&self, cache: FxHashMap<P<Node>, &'static [P<Node>]>) {
        *self.jsdoc_cache.borrow_mut() = cache;
    }

    pub fn set_has_lazy_jsdoc(&self, lazy: bool) {
        self.has_lazy_jsdoc.set(lazy)
    }

    pub(crate) fn resolve_jsdoc(&'static self, n: P<Node>) -> &'static [P<Node>] {
        let Some(parse) = PARSE_JSDOC_FOR_NODE.get() else {
            panic!("resolveJSDoc called but parseJSDocForNode is not registered; ensure the parser package is imported");
        };
        // Fast path: check cache under read lock
        {
            let _guard = self.jsdoc_mu.read().unwrap();
            if let Some(&jsdocs) = self.jsdoc_cache.borrow().get(&n) {
                return jsdocs;
            }
        }
        // Slow path: parse and cache under write lock
        let _guard = self.jsdoc_mu.write().unwrap();
        // Double-check after acquiring write lock
        if let Some(&jsdocs) = self.jsdoc_cache.borrow().get(&n) {
            return jsdocs;
        }
        let jsdocs = alloc_vec(parse(self, n));
        self.jsdoc_cache.borrow_mut_locked().insert(n, jsdocs);
        jsdocs
    }

    pub fn bind_diagnostics(&self) -> &'static [P<Diagnostic>] {
        self.bind_diagnostics.get()
    }

    pub fn set_bind_diagnostics(&self, diags: &[P<Diagnostic>]) {
        self.bind_diagnostics.set(alloc_slice(diags))
    }

    pub fn for_each_child(&self, v: &mut dyn FnMut(P<Node>) -> bool) -> bool {
        visit_node_list(v, Some(self.statements)) || visit(v, Some(self.end_of_file_token))
    }

    pub fn visit_each_child(&self, node: P<Node>, v: &mut NodeVisitor) -> P<Node> {
        let statements = v.visit_top_level_statements_hooked(Some(self.statements)).unwrap();
        let end_of_file_token = v.visit_token_hooked(Some(self.end_of_file_token)).unwrap();
        v.factory.update_source_file(node, statements, end_of_file_token)
    }

    pub fn is_js(&self) -> bool {
        is_source_file_js(self.as_p())
    }

    fn copy_from(&self, other: &SourceFile) {
        // Do not copy fields set by NewSourceFile (Text, FileName, Path, or Statements)
        self.language_variant.set(other.language_variant.get());
        self.script_kind.set(other.script_kind.get());
        self.is_declaration_file.set(other.is_declaration_file.get());
        self.uses_uri_style_node_core_modules.set(other.uses_uri_style_node_core_modules.get());
        self.imports.set(other.imports.get());
        self.module_augmentations.set(other.module_augmentations.get());
        self.ambient_module_names.set(other.ambient_module_names.get());
        self.comment_directives.set(other.comment_directives.get());
        self.pragmas.set(other.pragmas.get());
        self.referenced_files.set(other.referenced_files.get());
        self.type_reference_directives.set(other.type_reference_directives.get());
        self.lib_reference_directives.set(other.lib_reference_directives.get());
        self.common_js_module_indicator.set(other.common_js_module_indicator.get());
        self.external_module_indicator.set(other.external_module_indicator.get());
        let node = self.as_node();
        node.flags.set(node.flags.get() | other.as_node().flags.get());
    }

    pub fn clone_node(&self, node: P<Node>, f: &NodeFactory) -> P<Node> {
        let updated = f.new_source_file(self.parse_options.clone(), self.text, self.statements, self.end_of_file_token);
        let new_file = updated.as_source_file();
        new_file.copy_from(self);
        clone_node(updated, node, &f.hooks)
    }

    pub fn ecma_line_map(&self) -> &'static [TextPos] {
        self.ecma_line_map.get_or_init(|| alloc_vec(compute_ecma_line_starts(self.text)))
    }

    pub fn is_bound(&self) -> bool {
        self.is_bound.load(Ordering::Acquire)
    }

    // GetPositionMap returns the PositionMap for this source file, computing it lazily.
    pub fn get_position_map(&self) -> P<PositionMap> {
        *self.position_map.get_or_init(|| P::new(compute_position_map(self.text)))
    }

    pub fn bind_once(&self, bind: impl FnOnce()) {
        self.bind_once.call_once(|| {
            bind();
            self.is_bound.store(true, Ordering::Release);
        });
    }

    // Getters mirroring Go's exported fields.

    pub fn language_variant(&self) -> LanguageVariant {
        self.language_variant.get()
    }
    pub fn script_kind(&self) -> ScriptKind {
        self.script_kind.get()
    }
    pub fn is_declaration_file(&self) -> bool {
        self.is_declaration_file.get()
    }
    pub fn uses_uri_style_node_core_modules(&self) -> Tristate {
        self.uses_uri_style_node_core_modules.get()
    }
    pub fn identifier_count(&self) -> usize {
        self.identifier_count.get()
    }
    pub fn module_augmentations(&self) -> &'static [P<Node>] {
        self.module_augmentations.get()
    }
    pub fn ambient_module_names(&self) -> &'static [&'static str] {
        self.ambient_module_names.get()
    }
    pub fn comment_directives(&self) -> &'static [CommentDirective] {
        self.comment_directives.get()
    }
    pub fn reparsed_clones(&self) -> &'static [P<Node>] {
        self.reparsed_clones.get()
    }
    pub fn pragmas(&self) -> &'static [Pragma] {
        self.pragmas.get()
    }
    pub fn referenced_files(&self) -> &'static [P<FileReference>] {
        self.referenced_files.get()
    }
    pub fn type_reference_directives(&self) -> &'static [P<FileReference>] {
        self.type_reference_directives.get()
    }
    pub fn lib_reference_directives(&self) -> &'static [P<FileReference>] {
        self.lib_reference_directives.get()
    }
    pub fn check_js_directive(&self) -> Option<P<CheckJsDirective>> {
        self.check_js_directive.get()
    }
    pub fn node_count(&self) -> usize {
        self.node_count.get()
    }
    pub fn text_count(&self) -> usize {
        self.text_count.get()
    }
    pub fn common_js_module_indicator(&self) -> Option<P<Node>> {
        self.common_js_module_indicator.get()
    }
    pub fn external_module_indicator(&self) -> Option<P<Node>> {
        self.external_module_indicator.get()
    }
    pub fn symbol_count(&self) -> usize {
        self.symbol_count.get()
    }
    pub fn pattern_ambient_modules(&self) -> &'static [P<PatternAmbientModule>] {
        self.pattern_ambient_modules.get()
    }
    pub fn global_exports(&self) -> Option<P<SymbolTable>> {
        self.global_exports.get()
    }
    pub fn symbol(&self) -> Option<P<Symbol>> {
        self.declaration_base.symbol.get()
    }
    pub fn locals(&self) -> Option<P<SymbolTable>> {
        self.locals_container_base.locals.get()
    }
    pub fn statements(&self) -> P<NodeList> {
        self.statements
    }
    pub fn end_of_file_token(&self) -> P<Node> {
        self.end_of_file_token
    }
}

impl HasFileName for SourceFile {
    fn file_name(&self) -> &str {
        SourceFile::file_name(self)
    }
    fn path(&self) -> &Path {
        SourceFile::path(self)
    }
}

impl Node {
    /// `P<SourceFile>` for a SourceFile node (Go code that holds `*ast.SourceFile`).
    #[inline]
    pub fn as_source_file_p(&self) -> P<SourceFile> {
        P::from_static(self.as_source_file())
    }
}

fn collect_identifiers_for_source_file(source_file: &SourceFile) -> Set<&'static str> {
    let mut identifiers: Set<&'static str> = Set::default();
    fn collect(node: P<Node>, identifiers: &mut Set<&'static str>) -> bool {
        match node.kind() {
            Kind::Identifier
            | Kind::PrivateIdentifier
            | Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::NoSubstitutionTemplateLiteral => {
                identifiers.add(node.text());
            }
            _ => {}
        }
        node.for_each_child(&mut |child| collect(child, identifiers));
        false
    }
    collect(source_file.as_node(), &mut identifiers);
    identifiers
}

impl NodeFactory {
    pub fn update_source_file(&self, node: P<Node>, statements: P<NodeList>, end_of_file_token: P<Node>) -> P<Node> {
        let file = node.as_source_file();
        if statements != file.statements || end_of_file_token != file.end_of_file_token {
            let updated = self.new_source_file(file.parse_options.clone(), file.text, statements, end_of_file_token);
            updated.as_source_file().copy_from(file);
            return update_node(updated, node, &self.hooks);
        }
        node
    }
}

pub trait SourceFileLike {
    fn text(&self) -> &str;
    fn ecma_line_map(&self) -> &[TextPos];
}

impl SourceFileLike for SourceFile {
    fn text(&self) -> &str {
        self.text
    }
    fn ecma_line_map(&self) -> &[TextPos] {
        SourceFile::ecma_line_map(self)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CommentRange {
    pub text_range: TextRange,
    pub kind: Kind,
    pub has_trailing_new_line: bool,
}

impl CommentRange {
    #[inline]
    pub fn pos(&self) -> i32 {
        self.text_range.pos()
    }
    #[inline]
    pub fn end(&self) -> i32 {
        self.text_range.end()
    }
}

impl NodeFactory {
    pub fn new_comment_range(&self, kind: Kind, pos: i32, end: i32, has_trailing_new_line: bool) -> CommentRange {
        CommentRange { text_range: TextRange::new(pos, end), kind, has_trailing_new_line }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileReference {
    pub text_range: TextRange,
    pub file_name: String,
    pub resolution_mode: ResolutionMode,
    pub preserve: bool,
}

impl FileReference {
    #[inline]
    pub fn pos(&self) -> i32 {
        self.text_range.pos()
    }
    #[inline]
    pub fn end(&self) -> i32 {
        self.text_range.end()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PragmaArgument {
    pub text_range: TextRange,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub struct Pragma {
    pub comment_range: CommentRange,
    pub name: String,
    pub args: FxHashMap<String, PragmaArgument>,
}

pub type PragmaKindFlags = u8;

pub const PragmaKindTripleSlashXML: PragmaKindFlags = 1 << 0;
pub const PragmaKindSingleLine: PragmaKindFlags = 1 << 1;
pub const PragmaKindMultiLine: PragmaKindFlags = 1 << 2;
pub const PragmaKindFlagsNone: PragmaKindFlags = 0;
pub const PragmaKindAll: PragmaKindFlags = PragmaKindTripleSlashXML | PragmaKindSingleLine | PragmaKindMultiLine;
pub const PragmaKindDefault: PragmaKindFlags = PragmaKindAll;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PragmaArgumentSpecification {
    pub name: &'static str,
    pub optional: bool,
    pub capture_span: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PragmaSpecification {
    pub args: &'static [PragmaArgumentSpecification],
    pub kind: PragmaKindFlags,
}

impl PragmaSpecification {
    pub fn is_triple_slash(&self) -> bool {
        (self.kind & PragmaKindTripleSlashXML) > 0
    }
}

// Hand-written visitor implementations for nodes with runtime-dependent
// child ordering. Generated code in generated.rs delegates to these.

pub(crate) fn for_each_child_jsdoc_parameter_or_property_tag(node: &JSDocParameterOrPropertyTag, v: &mut dyn FnMut(P<Node>) -> bool) -> bool {
    v(node.tag_name())
        || (node.is_name_first && (v(node.name) || visit(v, node.type_expression)))
        || (!node.is_name_first && (visit(v, node.type_expression) || v(node.name)))
        || visit_node_list(v, node.comment())
}

pub(crate) fn visit_each_child_jsdoc_parameter_or_property_tag(node: &JSDocParameterOrPropertyTag, n: P<Node>, v: &mut NodeVisitor) -> P<Node> {
    let tag_name = v.visit_node_hooked(Some(node.tag_name())).unwrap();
    let name = v.visit_node_hooked(Some(node.name)).unwrap();
    let type_expression = v.visit_node_hooked(node.type_expression);
    let comment = v.visit_nodes_hooked(node.comment());
    v.factory.update_jsdoc_parameter_or_property_tag(n, tag_name, name, node.is_bracketed, type_expression, node.is_name_first, comment)
}

impl NodeFactory {
    pub fn release_arenas(&self) {}
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use tsrs_core::{TextRange, P};

    use crate::*;

    fn set_parents(node: P<Node>) {
        node.for_each_child(&mut |child| {
            child.set_parent(Some(node));
            set_parents(child);
            false
        });
    }

    fn children(node: P<Node>) -> Vec<P<Node>> {
        node.iter_children()
    }

    #[test]
    fn build_walk_and_cast() {
        let mut f = NodeFactory::default();
        let a = f.new_identifier("a");
        let plus = f.new_token(Kind::PlusToken);
        let b = f.new_identifier("b");
        let bin = f.new_binary_expression(None, a, None, plus, b);
        let stmt = f.new_expression_statement(bin);
        let list = f.new_node_list(vec![stmt]);
        let block = f.new_block(list, false);
        set_parents(block);

        assert_eq!(f.node_count(), 6);
        assert!(is_binary_expression(bin));
        assert!(!is_identifier(bin));
        assert_eq!(children(bin), vec![a, plus, b]);
        assert_eq!(children(block), vec![stmt]);
        assert_eq!(bin.as_binary_expression().left(), a);
        assert_eq!(bin.as_binary_expression().operator_token.kind(), Kind::PlusToken);
        assert_eq!(stmt.expression(), Some(bin));
        assert_eq!(block.statements(), &[stmt]);
        assert_eq!(a.text(), "a");
        assert_eq!(a.parent(), Some(bin));
        assert_eq!(bin.parent(), Some(stmt));
        assert_eq!(stmt.parent(), Some(block));
        assert!(block.contains(Some(a)));
        assert!(block.locals_container_data().is_some());
        assert!(bin.declaration_data().is_some());
        assert!(a.flow_node_data().is_some());
        assert!(plus.declaration_data().is_none());
        assert_eq!(a.loc(), TextRange::new(-1, -1));

        // Stopping early propagates true.
        let mut seen = 0;
        assert!(bin.for_each_child(&mut |_| {
            seen += 1;
            seen == 2
        }));
        assert_eq!(seen, 2);
    }

    #[test]
    #[should_panic]
    fn cast_mismatch_panics() {
        let mut f = NodeFactory::default();
        let a = f.new_identifier("a");
        a.as_binary_expression();
    }

    #[test]
    fn source_file_back_pointer() {
        let mut f = NodeFactory::default();
        let stmt = f.new_empty_statement();
        let statements = f.new_node_list(vec![stmt]);
        let eof = f.new_token(Kind::EndOfFile);
        let opts = SourceFileParseOptions { file_name: "/a.ts".to_string(), ..Default::default() };
        let node = f.new_source_file(opts, ";", statements, eof);
        set_parents(node);
        let file = node.as_source_file();
        assert_eq!(file.as_node(), node);
        assert_eq!(file.file_name(), "/a.ts");
        assert_eq!(file.text(), ";");
        assert_eq!(node.statements(), &[stmt]);
        assert_eq!(children(node), vec![stmt, eof]);
        assert_eq!(get_source_file_of_node(stmt).map(|f| f.as_node()), Some(node));
        file.symbol_count.set(3);
        assert_eq!(file.symbol_count(), 3);
        assert!(node.declaration_data().is_some());
        assert!(is_locals_container(node));
    }

    #[test]
    fn reparser_mutations() {
        let mut f = NodeFactory::default();
        let name = f.new_identifier("x");
        let decl = f.new_variable_declaration(name, None, None, None);
        assert_eq!(decl.type_node(), None);
        let t = f.new_keyword_type_node(Kind::NumberKeyword);
        decl.as_mutable().set_type(Some(t));
        assert_eq!(decl.type_node(), Some(t));
        assert_eq!(decl.as_variable_declaration().type_(), Some(t));
        assert_eq!(children(decl), vec![name, t]);

        let m = f.new_modifier(Kind::ExportKeyword);
        decl.set_modifiers(None);
        let stmt_list = f.new_node_list(vec![decl]);
        let list = f.new_variable_declaration_list(stmt_list, NodeFlags::Const);
        assert!(list.flags().contains(NodeFlags::Const));
        let mods = f.new_modifier_list(vec![m]);
        let var_stmt = f.new_variable_statement(None, list);
        var_stmt.set_modifiers(Some(mods));
        assert_eq!(var_stmt.modifier_flags(), ModifierFlags::Export);
        assert_eq!(children(var_stmt), vec![m, list]);
    }

    #[test]
    fn clone_and_visit_each_child() {
        let mut f = NodeFactory::default();
        let a = f.new_identifier("a");
        let plus = f.new_token(Kind::PlusToken);
        let b = f.new_identifier("b");
        let bin = f.new_binary_expression(None, a, None, plus, b);
        bin.set_loc(TextRange::new(3, 8));

        let copy = bin.clone_node(&mut f);
        assert_ne!(copy, bin);
        assert_eq!(copy.loc(), bin.loc());
        assert_eq!(copy.as_binary_expression().left(), a);

        // Identity visitor: nothing changes.
        let mut v = new_node_visitor(Some(Rc::new(|_: &mut NodeVisitor, n: P<Node>| Some(n))), None, NodeVisitorHooks::default());
        assert_eq!(v.visit_each_child(Some(bin)), Some(bin));

        // Replacing a child produces an updated node with the original's location.
        let c: P<Node> = v.factory.new_identifier("c");
        let mut v = new_node_visitor(
            Some(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| if n == a { Some(c) } else { Some(n) })),
            Some(std::mem::take(&mut f)),
            NodeVisitorHooks::default(),
        );
        let updated = v.visit_each_child(Some(bin)).unwrap();
        assert_ne!(updated, bin);
        assert_eq!(updated.loc(), TextRange::new(3, 8));
        assert_eq!(updated.as_binary_expression().left(), c);
        assert_eq!(updated.as_binary_expression().right(), b);
    }

    #[test]
    fn symbol_table_order_and_snapshot() {
        let table = SymbolTable::new();
        let s1 = Symbol::new(SymbolFlags::Variable, "b");
        let s2 = Symbol::new(SymbolFlags::Function, "a");
        table.set("b", s1);
        table.set("a", s2);
        assert_eq!(table.keys(), vec!["b", "a"]);
        table.for_each(|name, _| {
            if name == "b" {
                table.delete("a");
            }
        });
        assert_eq!(table.len(), 1);
        assert_eq!(table.lookup("b"), Some(s1));
        assert!(!table.has("a"));
        assert!(InternalSymbolNameCall.starts_with(InternalSymbolNamePrefix));
        assert_eq!(escape_symbol_name(InternalSymbolNameCall), "__call");
    }

    // Regression: a table past the linear-search size keeps insertion order, keeps the original position on
    // re-set and finds every remaining name after a delete shifts the hash index (wrong lookups would make the
    // binder/checker miss or duplicate members).
    #[test]
    fn symbol_table_indexed_order_and_delete() {
        let names: Vec<&'static str> = (0..20).map(|i| &*Box::leak(format!("n{i}").into_boxed_str())).collect();
        let syms: Vec<P<Symbol>> = names.iter().map(|n| Symbol::new(SymbolFlags::Property, n)).collect();
        let table = SymbolTable::with_capacity(2);
        for (n, s) in names.iter().zip(&syms) {
            table.set(n, *s);
        }
        table.set(names[3], syms[0]);
        table.delete(names[5]);
        table.delete("missing");
        let mut expected: Vec<&str> = names.clone();
        expected.remove(5);
        assert_eq!(table.keys(), expected);
        assert_eq!(table.lookup(names[3]), Some(syms[0]));
        for (i, n) in names.iter().enumerate() {
            match i {
                3 => {}
                5 => assert_eq!(table.lookup(n), None),
                _ => assert_eq!(table.lookup(n), Some(syms[i])),
            }
        }
        let copy = table.clone_table();
        copy.set("new", syms[1]);
        assert_eq!(copy.len(), 20);
        assert_eq!(table.len(), 19);
        assert_eq!(copy.lookup(names[19]), Some(syms[19]));
        // Keys that are not their symbol's name ("n3" -> n0, "new" -> n1) survive deletes before them.
        assert_eq!(copy.lookup("new"), Some(syms[1]));
        copy.delete(names[1]);
        copy.delete(names[0]);
        assert_eq!(copy.lookup_entry(names[3]), Some((names[3], syms[0])));
        assert_eq!(copy.keys().last(), Some(&"new"));
        assert_eq!(copy.lookup("new"), Some(syms[1]));
        let small = SymbolTable::new();
        small.set("alias", syms[2]);
        small.set(names[4], syms[4]);
        small.delete("alias");
        assert_eq!(small.entries(), vec![(names[4], syms[4])]);
    }

    #[test]
    fn symbol_table_long_names_share_the_capped_length() {
        // Entries store the key length capped at 63: keys longer than that are told apart by hash and text.
        let long = |c: char, n: usize| -> &'static str { Box::leak(format!("{}{c}", "x".repeat(n)).into_boxed_str()) };
        let names: Vec<&'static str> = (0..40).map(|i| long(char::from(b'a' + (i % 26) as u8), 63 + i / 26)).collect();
        let syms: Vec<P<Symbol>> = names.iter().map(|n| Symbol::new(SymbolFlags::Property, n)).collect();
        for count in [5, 40] {
            let table = SymbolTable::new();
            for i in 0..count {
                table.set(names[i], syms[i]);
            }
            for i in 0..count {
                assert_eq!(table.lookup(names[i]), Some(syms[i]));
            }
            assert_eq!(table.lookup(long('z', 70)), None);
            table.delete(names[1]);
            assert_eq!(table.lookup(names[1]), None);
            assert_eq!(table.lookup(names[count - 1]), Some(syms[count - 1]));
        }
    }

    #[test]
    fn kinds_and_flags() {
        assert_eq!(Kind::FirstKeyword, Kind::BreakKeyword);
        assert_eq!(Kind::LastToken, Kind::DeferKeyword);
        assert_eq!(Kind::from_i16(Kind::Identifier as i16), Kind::Identifier);
        assert!(is_keyword_kind(Kind::ClassKeyword));
        assert!(is_assignment_operator(Kind::PlusEqualsToken));
        assert_eq!(NodeFlags::BlockScoped, NodeFlags::Let | NodeFlags::Const | NodeFlags::Using);
        assert!(SymbolFlags::Value.contains(SymbolFlags::Function));
        assert!(!SymbolFlags::FunctionScopedVariableExcludes.intersects(SymbolFlags::FunctionScopedVariable));
        assert_eq!(std::mem::size_of::<Node>(), 24);
    }

    #[test]
    fn node_header_word_keeps_kind_tag_and_parent() {
        let f = NodeFactory::default();
        let parent = f.new_keyword_expression(Kind::ThisKeyword);
        let child = f.new_token(Kind::Count);
        assert_eq!((child.kind(), child.data_tag(), child.parent()), (Kind::Count, NodeDataTag::Token, None));
        child.set_parent(Some(parent));
        assert_eq!((child.kind(), child.data_tag(), child.parent()), (Kind::Count, NodeDataTag::Token, Some(parent)));
        assert_eq!((parent.kind(), parent.data_tag()), (Kind::ThisKeyword, NodeDataTag::KeywordExpression));
        child.set_parent(None);
        assert_eq!((child.kind(), child.parent()), (Kind::Count, None));
    }
}
