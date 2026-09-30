//! Non-function declarations of nodebuilder.go, nodebuilderimpl.go and nodebuilderscopes.go (the node builder data
//! model). See docs/CHECKER.md, "Node builder".

use tsrs_ast::{NodeFactory, NodeVisitor};
use tsrs_core::collections::{CopyOnWriteMap, CopyOnWriteSet};
use tsrs_module::ModeAwareCacheKey;

use crate::*;

// nodebuilder.go

/// Go `NodeBuilder`: the public entry points (`TypeToTypeNode`, `SymbolToEntityName`, …) that push/pop a
/// `NodeBuilderContext` around a `NodeBuilderImpl` call. An arena handle (`P<NodeBuilder>`, `&self` methods), because
/// the checker caches one (`Checker::type_to_string_nodebuilder`) and serialization re-enters it (a diagnostic
/// reported during lazy resolution formats a type with the same builder, pushing a new context).
pub struct NodeBuilder {
    /// Go pushes `b.impl.ctx`, which is nil for the outermost call.
    pub ctx_stack: RefCell<Vec<Option<P<NodeBuilderContext>>>>,
    pub host: &'static dyn Program,
    pub impl_: P<NodeBuilderImpl>,
    pub verbosity: Cell<Option<P<VerbosityContext>>>, // nil for non-hover callers
}

// nodebuilderimpl.go

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CompositeSymbolIdentity {
    pub is_constructor_node: bool,
    pub symbol_id: SymbolId,
    pub node_id: NodeId,
}

/// Go `TrackedSymbolArgs`, handled as `P<TrackedSymbolArgs>` (built once by a composite literal).
pub struct TrackedSymbolArgs {
    pub symbol: P<Symbol>,
    pub enclosing_declaration: Option<P<Node>>,
    pub meaning: SymbolFlags,
}

/// Go `SerializedTypeEntry`, handled as `P<SerializedTypeEntry>` (built once by a composite literal).
pub struct SerializedTypeEntry {
    pub node: P<Node>,
    pub truncating: bool,
    pub added_length: i32,
    pub tracked_symbols: Vec<P<TrackedSymbolArgs>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CompositeTypeCacheIdentity {
    pub type_id: TypeId,
    pub flags: Flags,
    pub internal_flags: InternalFlags,
}

#[derive(Default)]
pub struct NodeBuilderLinks {
    pub serialized_types: GoMap<CompositeTypeCacheIdentity, P<SerializedTypeEntry>>, // Collection of types serialized at this location
    pub fake_scope_for_signature_declaration: Cell<Option<&'static str>>, // If present, this is a fake scope injected into an enclosing declaration chain.
}

/// Go `module.ModeAwareCache[moduleSpecifierResult]` is `map[ModeAwareCacheKey]moduleSpecifierResult`.
#[derive(Default)]
pub struct NodeBuilderSymbolLinks {
    pub specifier_cache: GoMap<ModeAwareCacheKey, moduleSpecifierResult>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct moduleSpecifierResult {
    pub specifier: &'static str,
    pub import_attributes_type: Option<P<Type>>,
}

/// Go `NodeBuilderContext`: the per-call state of a node builder invocation. Handled as `P<NodeBuilderContext>`
/// (Go swaps `b.impl.ctx` pointers, `SymbolTrackerImpl` and the `cloneNodeBuilderContext` restore closure hold it);
/// every field Go assigns is a `Cell`/`RefCell`. Build with `NodeBuilderContext { …, ..NodeBuilderContext::new(host) }`.
pub struct NodeBuilderContext {
    pub host: &'static dyn Program,
    pub tracker: Cell<Option<&'static dyn SymbolTracker>>, // never None after NodeBuilder::enter_context
    pub approximate_length: Cell<i32>,
    pub max_truncation_length: Cell<i32>,
    pub encountered_error: Cell<bool>,
    pub truncating: Cell<bool>,
    pub reported_diagnostic: Cell<bool>,
    pub flags: Cell<Flags>,
    pub internal_flags: Cell<InternalFlags>,
    pub depth: Cell<i32>,
    pub max_expansion_depth: Cell<i32>, // -1 means no expansion, 0+ = verbosity levels
    /// `ExpandSymbolForHover` pushes a nil sentinel, hence `Option`.
    pub type_stack: RefCell<Vec<Option<P<Type>>>>,
    pub can_increase_expansion_depth: Cell<bool>,
    pub expansion_truncated: Cell<bool>,
    pub enclosing_declaration: Cell<Option<P<Node>>>,
    pub enclosing_file: Cell<Option<P<SourceFile>>>,
    /// Go assigns whole slices (`t.root.inferTypeParameters`) and restores the saved one.
    pub infer_type_parameters: Cell<&'static [P<Type>]>,
    pub visited_types: RefCell<Set<TypeId>>,
    pub symbol_depth: RefCell<FxHashMap<CompositeSymbolIdentity, i32>>,
    /// Go saves the slice, sets it to nil, and stores/restores it around `visitAndTransformType`.
    pub tracked_symbols: RefCell<Vec<P<TrackedSymbolArgs>>>,
    pub mapper: Cell<Option<P<TypeMapper>>>,
    pub reverse_mapped_stack: RefCell<Vec<P<Symbol>>>,
    pub enclosing_symbol_types: RefCell<FxHashMap<SymbolId, P<Type>>>,
    pub suppress_report_inference_fallback: Cell<bool>,
    pub remapped_symbol_references: RefCell<FxHashMap<SymbolId, P<Symbol>>>,

    // per signature scope state
    pub type_parameter_names: RefCell<CopyOnWriteMap<TypeId, P<Node>>>, // values are Identifier nodes
    pub type_parameter_names_by_text: RefCell<CopyOnWriteSet<String>>,
    pub type_parameter_names_by_text_next_name_count: RefCell<CopyOnWriteMap<String, i32>>,
    pub type_parameter_symbol_list: RefCell<CopyOnWriteSet<SymbolId>>,
}

impl NodeBuilderContext {
    /// The Go zero value (with the one field that has no zero value).
    pub fn new(host: &'static dyn Program) -> NodeBuilderContext {
        NodeBuilderContext {
            host,
            tracker: Cell::new(None),
            approximate_length: Cell::new(0),
            max_truncation_length: Cell::new(0),
            encountered_error: Cell::new(false),
            truncating: Cell::new(false),
            reported_diagnostic: Cell::new(false),
            flags: Cell::new(Flags::None),
            internal_flags: Cell::new(InternalFlags::None),
            depth: Cell::new(0),
            max_expansion_depth: Cell::new(0),
            type_stack: RefCell::default(),
            can_increase_expansion_depth: Cell::new(false),
            expansion_truncated: Cell::new(false),
            enclosing_declaration: Cell::new(None),
            enclosing_file: Cell::new(None),
            infer_type_parameters: Cell::new(&[]),
            visited_types: RefCell::default(),
            symbol_depth: RefCell::default(),
            tracked_symbols: RefCell::default(),
            mapper: Cell::new(None),
            reverse_mapped_stack: RefCell::default(),
            enclosing_symbol_types: RefCell::default(),
            suppress_report_inference_fallback: Cell::new(false),
            remapped_symbol_references: RefCell::default(),
            type_parameter_names: RefCell::default(),
            type_parameter_names_by_text: RefCell::default(),
            type_parameter_names_by_text_next_name_count: RefCell::default(),
            type_parameter_symbol_list: RefCell::default(),
        }
    }
}

/// Go `NodeBuilderImpl`. An arena handle (`P<NodeBuilderImpl>`) whose methods take `&self` plus `c: &mut Checker`
/// (Go's `ch` field is dropped: `b.ch.foo()` -> `c.foo()`).
pub struct NodeBuilderImpl {
    // host members
    /// Go `f = e.Factory.AsNodeFactory()`: a handle to the emit context's factory (same hooks, so created nodes are
    /// marked synthesized). All factory methods take `&self`, so calls nest: `b.f.new_x(b.f.new_y())`.
    pub f: NodeFactory,
    pub e: P<EmitContext>,
    pub pc: P<PseudoChecker>,

    // cache
    pub links: LinkStore<Node, NodeBuilderLinks>,
    pub symbol_links: LinkStore<Symbol, NodeBuilderSymbolLinks>,

    // state
    pub ctx: Cell<Option<P<NodeBuilderContext>>>,

    // reusable visitor
    /// Go builds it once with `b.cloneBindingName` as the visit callback. A Rust `VisitFn` is a `'static` closure and
    /// cannot capture the `&mut Checker` that `clone_binding_name` needs; `new_node_builder_impl` leaves it `None`
    /// and `clone_binding_name` decides how to visit (see docs/CHECKER.md, "Node builder").
    pub clone_binding_name_visitor: RefCell<Option<NodeVisitor>>,

    // symbols for synthesized identifiers, needed for e.g. inlay hints
    pub id_to_symbol: RefCell<FxHashMap<P<Node>, P<Symbol>>>,
}

impl NodeBuilderImpl {
    /// Go `b.ctx` (always set while a `NodeBuilder` entry point runs).
    #[inline]
    pub fn ctx(&self) -> P<NodeBuilderContext> {
        self.ctx.get().unwrap()
    }
}

pub const defaultMaximumTruncationLength: i32 = 160;
pub const noTruncationMaximumTruncationLength: i32 = 1_000_000;

#[derive(Clone, Debug)]
pub struct sortedSymbolNamePair {
    pub sym: P<Symbol>,
    pub name: String,
}

/// Go `SignatureToSignatureDeclarationOptions`, passed as `Option<P<…>>` (built once by a composite literal).
#[derive(Default)]
pub struct SignatureToSignatureDeclarationOptions {
    pub modifiers: &'static [P<Node>],
    pub name: Option<P<Node>>, // a PropertyName node
    pub question_token: Option<P<Node>>,
}

pub const MAX_REVERSE_MAPPED_NESTING_INSPECTION_DEPTH: i32 = 3;

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum propertyNameNodeKind {
    Identifier,
    NumericLiteral,
    StringLiteral,
}

// nodebuilderscopes.go

#[derive(Clone, Debug)]
pub struct localsRecord {
    pub name: String,
    pub old_symbol: P<Symbol>,
}

// pseudochecker/checker.go

/// Placeholder for Go `pseudochecker.PseudoChecker` (package `internal/pseudochecker` is not ported yet; its
/// `GetTypeOfDeclaration`/`GetTypeOfAccessor`/`GetReturnTypeOfSignature` and `PseudoType`, and the checker's
/// pseudotypenodebuilder.go that consumes them, are needed by `serialize_type_for_declaration` /
/// `serialize_return_type_for_signature` when they reuse existing annotations).
pub struct PseudoChecker {
    pub strict_null_checks: bool,
    pub exact_optional_property_types: bool,
}

/// Go `pseudochecker.NewPseudoChecker`.
pub fn new_pseudo_checker(strict_null_checks: bool, exact_optional_property_types: bool) -> P<PseudoChecker> {
    P::new(PseudoChecker { strict_null_checks, exact_optional_property_types })
}
