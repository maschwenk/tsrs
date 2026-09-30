//! Non-function declarations of `relater.go`.

use std::fmt::Display;

use bitflags::bitflags;

use crate::*;

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct SignatureCheckMode: u32 {
        const None = 0;
        const BivariantCallback = 1 << 0;
        const StrictCallback = 1 << 1;
        const IgnoreReturnTypes = 1 << 2;
        const StrictArity = 1 << 3;
        const StrictTopSignature = 1 << 4;
        const Callback = Self::BivariantCallback.bits() | Self::StrictCallback.bits();
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct MinArgumentCountFlags: u32 {
        const None = 0;
        const StrongArityForUntypedJS = 1 << 0;
        const VoidIsNonOptional = 1 << 1;
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct IntersectionState: u32 {
        const None = 0;
        const Source = 1 << 0; // Source type is a constituent of an outer intersection
        const Target = 1 << 1; // Target type is a constituent of an outer intersection
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct RecursionFlags: u32 {
        const None = 0;
        const Source = 1 << 0;
        const Target = 1 << 1;
        const Both = Self::Source.bits() | Self::Target.bits();
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct ExpandingFlags: u8 {
        const None = 0;
        const Source = 1 << 0;
        const Target = 1 << 1;
        const Both = Self::Source.bits() | Self::Target.bits();
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct RelationComparisonResult: u32 {
        const None = 0;
        const Succeeded = 1 << 0;
        const Failed = 1 << 1;
        const ReportsUnmeasurable = 1 << 3;
        const ReportsUnreliable = 1 << 4;
        const ComplexityOverflow = 1 << 5;
        const ReportsMask = Self::ReportsUnmeasurable.bits() | Self::ReportsUnreliable.bits();
        const Overflow = Self::ComplexityOverflow.bits();
    }
}

/// Go `args []any` of a stored diagnostic are kept pre-formatted (`diagnostics::stringify_args`).
#[derive(Clone, Debug)]
pub struct DiagnosticAndArguments {
    pub message: &'static Message,
    pub arguments: Vec<String>,
}

#[derive(Default)]
pub struct ErrorOutputContainer {
    pub errors: RefCell<Vec<P<Diagnostic>>>,
    pub skip_logging: Cell<bool>,
}

/// Go `type ErrorReporter func(message *diagnostics.Message, args ...any)`, with the checker passed back.
pub type ErrorReporter<'a> = &'a mut dyn FnMut(&mut Checker, &'static Message, &[&dyn Display]);

/// Go `RecursionId{value any}` restricted to `*ast.Node | *ast.Symbol | *Type` (see `asRecursionId`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RecursionId {
    Node(P<Node>),
    Symbol(P<Symbol>),
    Type(P<Type>),
}

impl From<P<Node>> for RecursionId {
    fn from(v: P<Node>) -> Self {
        RecursionId::Node(v)
    }
}
impl From<P<Symbol>> for RecursionId {
    fn from(v: P<Symbol>) -> Self {
        RecursionId::Symbol(v)
    }
}
impl From<P<Type>> for RecursionId {
    fn from(v: P<Type>) -> Self {
        RecursionId::Type(v)
    }
}

#[derive(Default)]
pub struct Relation {
    pub results: RefCell<FxHashMap<CacheHashKey, RelationComparisonResult>>,
}

/// Go `TypeDiscriminator`; the `c` field is dropped (methods take `c: &mut Checker`).
pub struct TypeDiscriminator<'a> {
    pub props: Vec<P<Symbol>>,
    pub is_related_to: &'a mut dyn FnMut(&mut Checker, P<Type>, P<Type>) -> Ternary,
}

/// Go `Discriminator` interface. Implementations delegate to the inherent (generated) methods.
pub trait Discriminator {
    fn len(&mut self, c: &mut Checker) -> i32; // Number of discriminant properties
    fn name(&mut self, c: &mut Checker, index: i32) -> String; // Property name of index-th discriminator
    fn matches(&mut self, c: &mut Checker, index: i32, t: P<Type>) -> bool; // True if index-th discriminator matches the given type
}

#[derive(Default)]
pub struct errorState {
    pub error_chain: Option<P<ErrorChain>>,
    pub related_info: Vec<P<Diagnostic>>,
}

pub struct ErrorChain {
    pub next: Option<P<ErrorChain>>,
    pub message: &'static Message,
    pub args: Vec<String>,
}

/// Go `Relater`. An arena handle (`P<Relater>`, pooled through `Checker::free_relater` like Go); the `c` field is
/// dropped and every method takes `c: &mut Checker`.
#[derive(Default)]
pub struct Relater {
    pub relation: Cell<Option<P<Relation>>>,
    pub error_node: Cell<Option<P<Node>>>,
    pub error_chain: Cell<Option<P<ErrorChain>>>,
    pub related_info: RefCell<Vec<P<Diagnostic>>>,
    pub maybe_keys: RefCell<Vec<CacheHashKey>>,
    pub maybe_keys_set: RefCell<Set<CacheHashKey>>,
    pub source_stack: RefCell<Vec<P<Type>>>,
    pub target_stack: RefCell<Vec<P<Type>>>,
    pub maybe_count: Cell<i32>,
    pub source_depth: Cell<i32>,
    pub target_depth: Cell<i32>,
    pub expanding_flags: Cell<ExpandingFlags>,
    pub overflow: Cell<bool>,
    pub relation_count: Cell<i32>,
    pub next: Cell<Option<P<Relater>>>,
}
