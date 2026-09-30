//! Declarations that printer.rs / symbolaccessibility / symboltracker signatures reference: the node-builder and printer
//! types from other Go packages (`nodebuilder`, `printer`) plus the non-function declarations of
//! `symbolaccessibility.go` and `symboltracker.go`. The node builder itself is not ported yet: types that only it
//! uses are placeholders (marked below) that the printer/node-builder port replaces.

use bitflags::bitflags;

use crate::*;

// nodebuilder/types.go

/// Go `nodebuilder.SymbolTracker`.
pub trait SymbolTracker {
    fn track_symbol(&self, symbol: P<Symbol>, enclosing_declaration: Option<P<Node>>, meaning: SymbolFlags) -> bool;
    fn report_inaccessible_this_error(&self);
    fn report_private_in_base_of_class_expression(&self, property_name: &str);
    fn report_inaccessible_unique_symbol_error(&self);
    fn report_cyclic_structure_error(&self);
    fn report_likely_unsafe_import_required_error(&self, specifier: &str, symbol_name: &str);
    fn report_truncation_error(&self);
    fn report_nonlocal_augmentation(&self, containing_file: P<SourceFile>, parent_symbol: P<Symbol>, augmenting_symbol: P<Symbol>);
    fn report_non_serializable_property(&self, property_name: &str);
    fn report_inference_fallback(&self, node: P<Node>);
    fn push_error_fallback_node(&self, node: Option<P<Node>>);
    fn pop_error_fallback_node(&self);
}

bitflags! {
    /// Go `nodebuilder.Flags`. NOTE: If modifying this enum, must modify `TypeFormatFlags` too!
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct Flags: u32 {
        const None = 0;
        // Options
        const NoTruncation = 1 << 0;
        const WriteArrayAsGenericType = 1 << 1;
        const GenerateNamesForShadowedTypeParams = 1 << 2;
        const UseStructuralFallback = 1 << 3;
        const ForbidIndexedAccessSymbolReferences = 1 << 4;
        const WriteTypeArgumentsOfSignature = 1 << 5;
        const UseFullyQualifiedType = 1 << 6;
        const UseOnlyExternalAliasing = 1 << 7;
        const SuppressAnyReturnType = 1 << 8;
        const WriteTypeParametersInQualifiedName = 1 << 9;
        const MultilineObjectLiterals = 1 << 10;
        const WriteClassExpressionAsTypeLiteral = 1 << 11;
        const UseTypeOfFunction = 1 << 12;
        const OmitParameterModifiers = 1 << 13;
        const UseAliasDefinedOutsideCurrentScope = 1 << 14;
        const UseSingleQuotesForStringLiteralType = 1 << 28;
        const NoTypeReduction = 1 << 29;
        const UseInstantiationExpressions = 1 << 30;
        const OmitThisParameter = 1 << 25;
        const WriteCallStyleSignature = 1 << 27;
        // Error handling
        const AllowThisInObjectLiteral = 1 << 15;
        const AllowQualifiedNameInPlaceOfIdentifier = 1 << 16;
        const AllowAnonymousIdentifier = 1 << 17;
        const AllowEmptyUnionOrIntersection = 1 << 18;
        const AllowEmptyTuple = 1 << 19;
        const AllowUniqueESSymbolType = 1 << 20;
        const AllowEmptyIndexInfoType = 1 << 21;
        // Errors (cont.)
        const AllowNodeModulesRelativePaths = 1 << 26;
        const IgnoreErrors = Self::AllowThisInObjectLiteral.bits() | Self::AllowQualifiedNameInPlaceOfIdentifier.bits() | Self::AllowAnonymousIdentifier.bits() | Self::AllowEmptyUnionOrIntersection.bits() | Self::AllowEmptyTuple.bits() | Self::AllowEmptyIndexInfoType.bits() | Self::AllowNodeModulesRelativePaths.bits();
        // State
        const InObjectTypeLiteral = 1 << 22;
        const InTypeAlias = 1 << 23;
        const InInitialEntityName = 1 << 24;
    }
}

bitflags! {
    /// Go `nodebuilder.InternalFlags`.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct InternalFlags: i32 {
        const None = 0;
        const WriteComputedProps = 1 << 0;
        const NoSyntacticPrinter = 1 << 1;
        const DoNotIncludeSymbolChain = 1 << 2;
        const AllowUnresolvedNames = 1 << 3;
    }
}

// printer/emitresolver.go

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum SymbolAccessibility {
    #[default]
    Accessible,
    NotAccessible,
    CannotBeNamed,
    NotResolved,
}

#[derive(Clone, Debug, Default)]
pub struct SymbolAccessibilityResult {
    pub accessibility: SymbolAccessibility,
    pub aliases_to_make_visible: Vec<P<Node>>, // aliases that need to have this symbol visible
    pub error_symbol_name: String, // Optional - symbol name that results in error
    pub error_node: Option<P<Node>>, // Optional - node that results in error
    pub error_module_name: String, // Optional - If the symbol is not visible from module, module's name
}

// checker/nodebuilder.go

#[derive(Clone, Copy, Debug, Default)]
pub struct VerbosityContext {
    pub level: i32, // 0 = default (no expansion), 1+ = expansion depth
    pub max_truncation_length: i32, // 0 = use default
    pub can_increase_verbosity: bool, // output: whether increasing Level would reveal more
    pub truncated: bool, // output: whether output was truncated
}

/// Placeholder for Go `NodeBuilderContext` (nodebuilderimpl.go, not ported yet).
#[derive(Default)]
pub struct NodeBuilderContext {}

/// Placeholder for Go `printer.EmitContext` (emit is not ported).
#[derive(Default)]
pub struct EmitContext {}

/// Placeholder for Go `printer.Printer` (emit is not ported).
#[derive(Default)]
pub struct Printer {}

/// Placeholder for Go `EmitResolver` (emitresolver.go is not ported).
#[derive(Default)]
pub struct EmitResolver {}

// checker/symbolaccessibility.go

pub struct accessibleSymbolChainContext {
    pub symbol: P<Symbol>,
    pub enclosing_declaration: Option<P<Node>>,
    pub meaning: SymbolFlags,
    pub use_only_external_aliasing: bool,
    pub visited_symbol_tables_map: FxHashMap<SymbolId, FxHashSet<symbolTableID>>,
}

// checker/symboltracker.go

pub struct SymbolTrackerImpl {
    pub context: P<NodeBuilderContext>,
    pub inner: Option<&'static dyn SymbolTracker>,
    pub disable_track_symbol: bool,
}

/// Go `context.Context` parameters (cancellation is not ported; pass `Context`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Context;

/// Go `iter.Seq[T]`: materialized as a `Vec`.
pub type Seq<T> = Vec<T>;
