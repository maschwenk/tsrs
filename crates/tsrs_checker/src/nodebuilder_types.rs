//! Non-function declarations of nodebuilder.go, nodebuilderimpl.go and nodebuilderscopes.go (the node builder data
//! model). See docs/CHECKER.md, "Node builder".

use std::cell::OnceCell;

use tsrs_ast::{NodeFactory, VisitFn};
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
    pub host: &'static dyn ModuleSpecifierGenerationHost, // Go `Host`
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
    pub node: Option<P<Node>>, // Go stores the (possibly nil) result of the transform
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
    pub host: &'static dyn ModuleSpecifierGenerationHost, // Go `Host`
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
    pub mapper: MapperCell,
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
    pub fn new(host: &'static dyn ModuleSpecifierGenerationHost) -> NodeBuilderContext {
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
            mapper: MapperCell::new(None),
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
    pub links: tsrs_core::LinkStore<Node, NodeBuilderLinks>,
    pub symbol_links: tsrs_core::LinkStore<Symbol, NodeBuilderSymbolLinks>,

    // state
    pub ctx: Cell<Option<P<NodeBuilderContext>>>,

    // reusable visitor
    /// Go's `cloneBindingNameVisitor` (visit callback `b.cloneBindingName`), kept as its callback: a `NodeVisitor`
    /// holds nothing but the callback, the factory and hooks, and a visit re-enters it, so `clone_binding_name` builds
    /// `new_node_visitor(Some(visit.clone()), Some(b.f.clone()), NodeVisitorHooks::default())` per call and runs it
    /// under `b.checker_slot.lend(c, ..)`. Set by `new_node_builder_impl`.
    pub clone_binding_name_visitor: OnceCell<VisitFn>,

    // symbols for synthesized identifiers, needed for e.g. inlay hints
    pub id_to_symbol: P<RefCell<FxHashMap<P<Node>, P<Symbol>>>>,

    /// Rust only: lends the checker to visitor callbacks (`cloneBindingName`, `getExistingNodeTreeVisitor`), which
    /// Go writes as closures over `b.ch`. See `CheckerSlot`.
    pub checker_slot: P<CheckerSlot>,
    /// Rust only: this builder's own handle, for closures that capture `b` (`b.as_p()`). Set by `new_node_builder_impl`.
    pub this: OnceCell<P<NodeBuilderImpl>>,
}

impl NodeBuilderImpl {
    /// Go `b.ctx` (always set while a `NodeBuilder` entry point runs).
    #[inline]
    pub fn ctx(&self) -> P<NodeBuilderContext> {
        self.ctx.get().unwrap()
    }

    /// `b` as a copyable handle, for closures (Go captures the `*NodeBuilderImpl`).
    #[inline]
    pub fn as_p(&self) -> P<NodeBuilderImpl> {
        *self.this.get().unwrap()
    }
}

/// Rust only: gives `ast::NodeVisitor` callbacks access to the checker. Go's visitor callbacks are closures over
/// `b.ch`; a Rust `VisitFn` is a `'static` `Rc<dyn Fn(&mut NodeVisitor, P<Node>)>` and cannot capture `&mut Checker`.
/// The code that starts a visit lends its checker for the duration (`slot.lend(c, || v.visit_node(n))`); a callback
/// borrows it back with `slot.with(|c| ..)`, and must `lend` its own `c` again before re-entering the visitor
/// (`slot.with(|c| { ..; slot.lend(c, || v.visit_each_child(Some(n))) })`), so every checker reference is derived from
/// the one live above it. Like a `RefCell`, a second `with` without an intervening `lend` panics, and so does `with`
/// outside any `lend`.
#[derive(Default)]
pub struct CheckerSlot {
    ptr: Cell<Option<std::ptr::NonNull<Checker>>>,
    in_use: Cell<bool>,
}

impl CheckerSlot {
    /// Makes `c` available to `with` while `f` runs (nests; the previous state is restored afterwards).
    pub fn lend<R>(&self, c: &mut Checker, f: impl FnOnce() -> R) -> R {
        let saved = (self.ptr.replace(Some(std::ptr::NonNull::from(c))), self.in_use.replace(false));
        let result = f();
        self.ptr.set(saved.0);
        self.in_use.set(saved.1);
        result
    }

    /// Calls `f` with the checker lent by the innermost `lend`.
    pub fn with<R>(&self, f: impl FnOnce(&mut Checker) -> R) -> R {
        let mut ptr = self.ptr.get().expect("CheckerSlot::with outside CheckerSlot::lend");
        assert!(!self.in_use.replace(true), "CheckerSlot::with re-entered without CheckerSlot::lend");
        // SAFETY: `ptr` comes from the `&mut Checker` passed to the innermost active `lend`, which is borrowed for
        // the whole `lend` call and not otherwise usable while `f` runs; `in_use` guarantees that at most one
        // reference derived from it is live (a nested `with` needs a nested `lend`, which derives from this one).
        let result = f(unsafe { ptr.as_mut() });
        self.in_use.set(false);
        result
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

/// `modulespecifiers.CheckerShape` is implemented by `*Checker` in Go (exported `GetSymbolAtLocation` /
/// `GetAliasedSymbol`).
impl modulespecifiers::CheckerShape for Checker {
    fn get_symbol_at_location(&mut self, node: P<Node>) -> Option<P<Symbol>> {
        self.get_symbol_at_location_exported(node)
    }
    fn get_aliased_symbol(&mut self, symbol: P<Symbol>) -> P<Symbol> {
        Checker::get_aliased_symbol(self, symbol)
    }
}

// nodecopy.go

/// Go `recoveryBoundary`, handled as `P<recoveryBoundary>` (the wrapping tracker holds it). Fields Go sets at
/// construction and only reads are plain; the rest are `Cell`/`RefCell`.
pub struct recoveryBoundary {
    pub ctx: P<NodeBuilderContext>,
    pub had_error: Cell<bool>,
    pub deferred_reports: RefCell<Vec<Box<dyn FnOnce()>>>,
    pub old_tracker: Option<&'static dyn SymbolTracker>,
    /// `finalizeBoundary` moves it back into the context (`take()`).
    pub old_tracked_symbols: RefCell<Vec<P<TrackedSymbolArgs>>>,
    pub tracked_symbols: RefCell<Vec<P<TrackedSymbolArgs>>>,
    pub old_encountered_error: bool,
    pub old_approximate_length: i32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct originalRecoveryScopeState {
    pub tracked_symbols_top: i32,
    pub unreported_errors_top: i32,
    pub had_error: bool,
}

/// Go `wrappingTracker`, handled as `P<wrappingTracker>` and used as `&'static dyn SymbolTracker`; the inherent
/// methods are in nodecopy.rs. `markError(w.wrapped.ReportX)` passes `Some(Box::new(move || wrapped.report_x()))`
/// with `let wrapped = self.wrapped;`.
pub struct wrappingTracker {
    pub wrapped: &'static dyn SymbolTracker,
    pub bound: P<recoveryBoundary>,
}

impl SymbolTracker for wrappingTracker {
    fn track_symbol(&self, c: &mut Checker, symbol: P<Symbol>, enclosing_declaration: Option<P<Node>>, meaning: SymbolFlags) -> bool {
        wrappingTracker::track_symbol(self, c, symbol, enclosing_declaration, meaning)
    }
    fn report_inaccessible_this_error(&self) {
        wrappingTracker::report_inaccessible_this_error(self)
    }
    fn report_private_in_base_of_class_expression(&self, property_name: &str) {
        wrappingTracker::report_private_in_base_of_class_expression(self, property_name)
    }
    fn report_inaccessible_unique_symbol_error(&self) {
        wrappingTracker::report_inaccessible_unique_symbol_error(self)
    }
    fn report_cyclic_structure_error(&self) {
        wrappingTracker::report_cyclic_structure_error(self)
    }
    fn report_likely_unsafe_import_required_error(&self, specifier: &str, symbol_name: &str) {
        wrappingTracker::report_likely_unsafe_import_required_error(self, specifier, symbol_name)
    }
    fn report_truncation_error(&self) {
        wrappingTracker::report_truncation_error(self)
    }
    fn report_nonlocal_augmentation(&self, containing_file: P<SourceFile>, parent_symbol: P<Symbol>, augmenting_symbol: P<Symbol>) {
        wrappingTracker::report_nonlocal_augmentation(self, containing_file, parent_symbol, augmenting_symbol)
    }
    fn report_non_serializable_property(&self, property_name: &str) {
        wrappingTracker::report_non_serializable_property(self, property_name)
    }
    fn report_inference_fallback(&self, c: &mut Checker, node: P<Node>) {
        wrappingTracker::report_inference_fallback(self, c, node)
    }
    fn push_error_fallback_node(&self, node: Option<P<Node>>) {
        wrappingTracker::push_error_fallback_node(self, node)
    }
    fn pop_error_fallback_node(&self) {
        wrappingTracker::pop_error_fallback_node(self)
    }
}

// emitresolver.go (the data; the functions are in emitresolver.rs)

// Links for jsx
#[derive(Default)]
pub struct JSXLinks {
    pub import_ref: Cell<Option<P<Node>>>,
}

// Links for declarations

#[derive(Default)]
pub struct DeclarationLinks {
    pub is_visible: Cell<Tristate>, // if declaration is depended upon by exported declarations
}

#[derive(Default)]
pub struct DeclarationFileLinks {
    pub aliases_marked: Cell<bool>, // if file has had alias visibility marked
}

/// Go `EmitResolver`, handled as `P<EmitResolver>`, a checker holder (`&self, c: &mut Checker`). Go's `checker`
/// field is dropped and so is `checkerMu`: the exported methods that lock it in Go (`IsDeclarationVisible`, ...) are
/// the `_exported`/unsuffixed `pub` methods, and the declaration transformer serializes access to the checker itself
/// (tsrs_declarations `Resolver::lock`). `isValueAliasDeclaration` / `aliasMarkingVisitor` are Go method values of
/// `isValueAliasDeclarationWorker` / `aliasMarkingVisitorWorker`; Rust calls those methods directly.
#[derive(Default)]
pub struct EmitResolver {
    pub reference_resolver: std::cell::OnceCell<P<tsrs_binder::ReferenceResolver<Checker>>>,
    pub jsx_links: tsrs_core::LinkStore<Node, JSXLinks>,
    pub declaration_links: tsrs_core::LinkStore<Node, DeclarationLinks>,
    pub declaration_file_links: tsrs_core::LinkStore<Node, DeclarationFileLinks>,
}
