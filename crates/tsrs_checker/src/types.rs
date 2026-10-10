use std::cell::OnceCell;
use std::fmt;
use std::hash::Hash;

use bitflags::bitflags;
use tsrs_core::owned_array::{ArrayCell, ArrayView, OptionArrayCell};

use crate::*;

// ParseFlags

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct ParseFlags: u32 {
        const None = 0;
        const Yield = 1 << 0;
        const Await = 1 << 1;
        const Type = 1 << 2;
        const IgnoreMissingOpenBrace = 1 << 4;
        const JSDoc = 1 << 5;
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum SignatureKind {
    #[default]
    Call,
    Construct,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum IndexKind {
    #[default]
    String,
    Number,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum MemberOverrideStatus {
    #[default]
    None,
    NeedsOverride,
    HasInvalidOverride,
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct ContextFlags: u32 {
        const None = 0;
        const Signature = 1 << 0; // Obtaining contextual signature
        const NoConstraints = 1 << 1; // Don't obtain type variable constraints
        const IgnoreNodeInferences = 1 << 2; // Ignore inference to current node and parent nodes out to the containing call for, for example, completions
        const SkipBindingPatterns = 1 << 3; // Ignore contextual types applied by binding patterns
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct TypeFormatFlags: u32 {
        const None = 0;
        const NoTruncation = 1 << 0;
        const WriteArrayAsGenericType = 1 << 1;
        const GenerateNamesForShadowedTypeParams = 1 << 2;
        const UseStructuralFallback = 1 << 3;
        const WriteTypeArgumentsOfSignature = 1 << 5;
        const UseFullyQualifiedType = 1 << 6;
        const SuppressAnyReturnType = 1 << 8;
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
        const AllowUniqueESSymbolType = 1 << 20;
        const AddUndefined = 1 << 17;
        const WriteArrowStyleSignature = 1 << 18;
        const InArrayType = 1 << 19;
        const InElementType = 1 << 21;
        const InFirstTypeArgument = 1 << 22;
        const InTypeAlias = 1 << 23;

        const NodeBuilderFlagsMask = Self::NoTruncation.bits() | Self::WriteArrayAsGenericType.bits() | Self::GenerateNamesForShadowedTypeParams.bits() | Self::UseStructuralFallback.bits() | Self::WriteTypeArgumentsOfSignature.bits() |
            Self::UseFullyQualifiedType.bits() | Self::SuppressAnyReturnType.bits() | Self::MultilineObjectLiterals.bits() | Self::WriteClassExpressionAsTypeLiteral.bits() |
            Self::UseTypeOfFunction.bits() | Self::OmitParameterModifiers.bits() | Self::UseAliasDefinedOutsideCurrentScope.bits() | Self::AllowUniqueESSymbolType.bits() | Self::InTypeAlias.bits() |
            Self::UseInstantiationExpressions.bits() | Self::UseSingleQuotesForStringLiteralType.bits() | Self::NoTypeReduction.bits() | Self::OmitThisParameter.bits();
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct SymbolFormatFlags: u32 {
        const None = 0;
        const WriteTypeParametersOrArguments = 1 << 0;
        const UseOnlyExternalAliasing = 1 << 1;
        const AllowAnyNodeKind = 1 << 2;
        const UseAliasDefinedOutsideCurrentScope = 1 << 3;
        const WriteComputedProps = 1 << 4;
        const DoNotIncludeSymbolChain = 1 << 5;
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct ExternalEmitHelpers: u32 {
        const None = 0;
        const Rest = 1 << 0; // __rest (used by ESNext object rest transformation)
        const Decorate = 1 << 1; // __decorate (used by TypeScript decorators transformation)
        const Metadata = 1 << 2; // __metadata (used by TypeScript decorators transformation)
        const Param = 1 << 3; // __param (used by TypeScript decorators transformation)
        const Awaiter = 1 << 4; // __awaiter (used by ES2017 async functions transformation)
        const Await = 1 << 5; // __await (used by ES2017 async generator transformation)
        const AsyncGenerator = 1 << 6; // __asyncGenerator (used by ES2017 async generator transformation)
        const AsyncDelegator = 1 << 7; // __asyncDelegator (used by ES2017 async generator yield* transformation)
        const AsyncValues = 1 << 8; // __asyncValues (used by ES2017 for..await..of transformation)
        const ExportStar = 1 << 9; // __exportStar (used by CommonJS/AMD/UMD module transformation)
        const ImportStar = 1 << 10; // __importStar (used by CommonJS/AMD/UMD module transformation)
        const ImportDefault = 1 << 11; // __importDefault (used by CommonJS/AMD/UMD module transformation)
        const MakeTemplateObject = 1 << 12; // __makeTemplateObject (used for constructing template string array objects)
        const ClassPrivateFieldGet = 1 << 13; // __classPrivateFieldGet (used by the class private field transformation)
        const ClassPrivateFieldSet = 1 << 14; // __classPrivateFieldSet (used by the class private field transformation)
        const ClassPrivateFieldIn = 1 << 15; // __classPrivateFieldIn (used by the class private field transformation)
        const SetFunctionName = 1 << 16; // __setFunctionName (used by class fields and ECMAScript decorators)
        const PropKey = 1 << 17; // __propKey (used by class fields and ECMAScript decorators)
        const AddDisposableResourceAndDisposeResources = 1 << 18; // __addDisposableResource and __disposeResources (used by ESNext transformations)
        const RewriteRelativeImportExtension = 1 << 19; // __rewriteRelativeImportExtension (used by --rewriteRelativeImportExtensions)
        const ESDecorateAndRunInitializers = Self::Decorate.bits(); // __esDecorate and __runInitializers (used by ECMAScript decorators transformation)

        const FirstEmitHelper = Self::Rest.bits();
        const LastEmitHelper = Self::RewriteRelativeImportExtension.bits();

        // Helpers included by ES2017 for..await..of
        const ForAwaitOfIncludes = Self::AsyncValues.bits();

        // Helpers included by ES2017 async generators
        const AsyncGeneratorIncludes = Self::Await.bits() | Self::AsyncGenerator.bits();

        // Helpers included by yield* in ES2017 async generators
        const AsyncDelegatorIncludes = Self::Await.bits() | Self::AsyncDelegator.bits() | Self::AsyncValues.bits();
    }
}

pub const externalHelpersModuleNameText: &str = "tslib";

// Ids

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct TypeId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct SignatureId(pub u32);

// Links for referenced symbols: Go's `SymbolReferenceLinks` has one field, `referenceKinds` (the meanings the symbol
// was referenced with), which `SymbolReferenceLinkStore` (links.rs) keeps in its slots.

pub use crate::owned_value_links::ValueSymbolLinks;

// Additional links for mapped symbols

#[derive(Default)]
pub struct MappedSymbolLinks {
    pub key_type: Cell<Option<P<Type>>>, // Key type for mapped type member
    pub synthetic_origin: Cell<Option<P<Symbol>>>, // For a property on a mapped or spread type, points back to the original property
}

// Additional links for deferred type symbols

#[derive(Default)]
pub struct DeferredSymbolLinks {
    pub parent: Cell<Option<P<Type>>>, // Source union/intersection of a deferred type
    pub constituents: ArrayCell<P<Type>>, // Calculated list of constituents for a deferred type
    pub write_constituents: ArrayCell<P<Type>>, // Constituents of a deferred `writeType`
}

// Links for alias symbols

#[derive(Default)]
pub struct AliasSymbolLinks {
    pub immediate_target: Cell<Option<P<Symbol>>>, // Immediate target of an alias. May be another alias. Do not access directly, use `checker.getImmediateAliasedSymbol` instead.
    pub alias_target: Cell<Option<P<Symbol>>>, // Resolved (non-alias) target of an alias
    pub referenced: Cell<bool>, // True if alias symbol has been referenced as a value that can be emitted
    pub type_only_declaration: Cell<Option<P<Node>>>, // First resolved alias declaration that makes the symbol only usable in type constructs
}

// Links for module symbols

#[derive(Default)]
pub struct ModuleSymbolLinks {
    pub resolved_exports: Cell<Option<P<SymbolTable>>>, // Resolved exports of module or combined early- and late-bound static members of a class.
    pub type_only_export_star_map: OwnedMap<String, P<Node>>, // Set on a module symbol when some of its exports were resolved through a 'export type * from "mod"' declaration
    pub exports_checked: Cell<bool>,
}

#[derive(Default)]
pub struct ReverseMappedSymbolLinks {
    pub property_type: Cell<Option<P<Type>>>,
    pub mapped_type: Cell<Option<P<Type>>>, // References a mapped type
    pub constraint_type: Cell<Option<P<Type>>>, // References an index type
}

// Links for late-bound symbols

#[derive(Default)]
pub struct LateBoundLinks {
    pub late_symbol: Cell<Option<P<Symbol>>>,
}

// Links for export type symbols

#[derive(Default)]
pub struct ExportTypeLinks {
    pub target: Cell<Option<P<Symbol>>>, // Target symbol
    pub originating_import: Cell<Option<P<Node>>>, // Import declaration which produced the symbol, present if the symbol is marked as uncallable but had call signatures in `resolveESModuleSymbol`
}

// Links for type aliases

#[derive(Default)]
pub struct TypeAliasLinks {
    pub declared_type: Cell<Option<P<Type>>>,
    pub type_parameters: ArrayCell<P<Type>>, // Type parameters of type alias (undefined if non-generic)
    pub instantiations: OwnedPackedMap<CacheHashKey, P<Type>>, // Instantiations of generic type alias (undefined if non-generic)
    pub is_constructor_declared_property: Cell<bool>,
}

// Links for declared types (type parameters, class types, interface types, enums)

#[derive(Default)]
pub struct DeclaredTypeLinks {
    pub declared_type: Cell<Option<P<Type>>>,
    pub interface_checked: Cell<bool>,
    pub index_signatures_checked: Cell<bool>,
    pub type_parameters_checked: Cell<bool>,
    pub enum_checked: Cell<bool>,
}

// Links for switch clauses

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum ExhaustiveState {
    #[default]
    Unknown, // Exhaustive state not computed
    Computing, // Exhaustive state computation in progress
    False, // Switch statement is not exhaustive
    True, // Switch statement is exhaustive
}

#[derive(Default)]
pub struct SwitchStatementLinks {
    pub exhaustive_state: Cell<ExhaustiveState>, // Switch statement exhaustiveness
    pub switch_types_computed: Cell<bool>,
    pub witnesses_computed: Cell<bool>,
    pub switch_types: ArrayCell<P<Type>>,
    pub witnesses: OptionArrayCell<TextView>, // Go nil (non-literal case) vs empty
}

#[derive(Default)]
pub struct ArrayLiteralLinks {
    pub indices_computed: Cell<bool>,
    pub first_spread_index: Cell<i32>, // Index of first spread expression (or -1 if none)
    pub last_spread_index: Cell<i32>, // Index of last spread expression (or -1 if none)
}

// Links for late-binding containers

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum MembersOrExportsResolutionKind {
    #[default]
    ResolvedExports = 0,
    ResolvedMembers = 1,
}

/// Go `[2]ast.SymbolTable`, indexed by `MembersOrExportsResolutionKind as usize`.
#[derive(Default)]
pub struct MembersAndExportsLinks(pub [Cell<Option<P<SymbolTable>>>; 2]);

impl std::ops::Deref for MembersAndExportsLinks {
    type Target = [Cell<Option<P<SymbolTable>>>; 2];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// Links for synthetic spread properties

#[derive(Default)]
pub struct SpreadLinks {
    pub left_spread: Cell<Option<P<Symbol>>>, // Left source for synthetic spread property
    pub right_spread: Cell<Option<P<Symbol>>>, // Right source for synthetic spread property
}

// Links for variances of type aliases and interface types

#[derive(Default)]
pub struct VarianceLinks {
    pub variances: OptionArrayCell<VarianceFlags>, // nil = not computed (Go distinguishes nil from empty)
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct VarianceFlags: u32 {
        const Invariant = 0; // Neither covariant nor contravariant
        const Covariant = 1 << 0; // Covariant
        const Contravariant = 1 << 1; // Contravariant
        const Bivariant = Self::Covariant.bits() | Self::Contravariant.bits(); // Both covariant and contravariant
        const Independent = 1 << 2; // Unwitnessed type parameter
        const VarianceMask = Self::Invariant.bits() | Self::Covariant.bits() | Self::Contravariant.bits() | Self::Independent.bits(); // Mask containing all measured variances without the unmeasurable flag
        const Unmeasurable = 1 << 3; // Variance result is unusable - relationship relies on structural comparisons which are not reflected in generic relationships
        const Unreliable = 1 << 4; // Variance result is unreliable - checking may produce false negatives, but not false positives
        const AllowsStructuralFallback = Self::Unmeasurable.bits() | Self::Unreliable.bits();
    }
}

#[derive(Default)]
pub struct MarkedAssignmentSymbolLinks {
    pub last_assignment_pos: Cell<i32>,
    pub has_definite_assignment: Cell<bool>, // Symbol is definitely assigned somewhere
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct accessibleChainCacheKey {
    pub use_only_external_aliasing: bool,
    pub location: Option<P<Node>>,
    pub meaning: SymbolFlags,
}

#[derive(Default)]
pub struct ContainingSymbolLinks {
    pub extended_containers_by_file: RefCell<FxHashMap<NodeId, Box<[P<Symbol>]>>>, // Symbols of nodes which which logically contain this one, cached by file the request is made within
    pub extended_containers: OptionArrayCell<P<Symbol>>, // Containers (other than the parent) which this symbol is aliased in
    pub accessible_chain_cache: RefCell<FxHashMap<accessibleChainCacheKey, Box<[P<Symbol>]>>>,
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct AccessFlags: u32 {
        const None = 0;
        const IncludeUndefined = 1 << 0;
        const NoIndexSignatures = 1 << 1;
        const Writing = 1 << 2;
        const CacheSymbol = 1 << 3;
        const AllowMissing = 1 << 4;
        const ExpressionPosition = 1 << 5;
        const ReportDeprecated = 1 << 6;
        const SuppressNoImplicitAnyError = 1 << 7;
        const Contextual = 1 << 8;
        const Persistent = Self::IncludeUndefined.bits();
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct NodeCheckFlags: u32 {
        const None = 0;
        const TypeChecked = 1 << 0; // Node has been type checked
        const ContextChecked = 1 << 6; // Contextual types have been assigned
        const EnumValuesComputed = 1 << 10; // Values for enum members have been computed, and any errors have been reported for them.
        const AssignmentsMarked = 1 << 17; // Parameter assignments have been marked
        const ContainsClassWithPrivateIdentifiers = 1 << 20; // Marked on all block-scoped containers containing a class with private identifiers.
        const ContainsSuperPropertyInStaticInitializer = 1 << 21; // Marked on all block-scoped containers containing a static initializer with 'super.x' or 'super[x]'.
        const InCheckIdentifier = 1 << 22;
        const InitializerIsUndefined = 1 << 24;
        const InitializerIsUndefinedComputed = 1 << 25;
    }
}

// Common links

#[derive(Default)]
pub struct NodeLinks {
    pub flags: Cell<NodeCheckFlags>, // Set of flags specific to Node
    pub declaration_requires_scope_change: Cell<Tristate>, // Set by `useOuterVariableScopeInParameter` in checker when downlevel emit would change the name resolution scope inside of a parameter.
    pub has_reported_statement_in_ambient_context: Cell<bool>, // Cache boolean if we report statements in ambient context
}

#[derive(Default)]
pub struct SymbolNodeLinks {
    pub resolved_symbol: Cell<Option<P<Symbol>>>, // Resolved symbol associated with node
}

#[derive(Default)]
pub struct TypeNodeLinks {
    pub resolved_type: Cell<Option<P<Type>>>, // Resolved type associated with node
    pub outer_type_parameters: OptionArrayCell<P<Type>>, // Outer type parameters of anonymous object type (Go distinguishes nil = not computed)
    link_key: Cell<tsrs_core::PKey>, // tsrs: the node this record is filed under (`KeyedLinkStore`), in what was padding
}

impl crate::links::KeyedLinks for TypeNodeLinks {
    fn link_key(&self) -> &Cell<tsrs_core::PKey> {
        &self.link_key
    }
}

// Three owner/edge words: the resolved type, array owner and native-pointer lookup key.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<TypeNodeLinks>() == 24);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<TypeNodeLinks>() == 12);

#[derive(Default)]
pub struct ComputedNameNodeLinks {
    pub has_name: Cell<Option<bool>>, // If the node has a computable name (Go *bool: nil = not computed)
    pub name: TextCell, // Resolved name associated with the type of the node
}

// Links for enum members

#[derive(Default)]
pub struct EnumMemberLinks {
    pub value: SnapshotCell<evaluator::Result>, // Constant value of enum member
}

// Links for assertion expressions

#[derive(Default)]
pub struct AssertionLinks {
    pub expr_type: Cell<Option<P<Type>>>, // Assertion expression type
}

// SourceFile links

#[derive(Default)]
pub struct SourceFileLinks {
    pub type_checked: Cell<bool>,
    pub unused_checked: Cell<bool>,
    pub external_helpers_module: Cell<Option<P<Symbol>>>,
    pub requested_external_emit_helpers: Cell<ExternalEmitHelpers>,
    pub deferred_nodes: RefCell<OrderedSet<P<Node>>>,
    pub identifier_check_nodes: RefCell<Vec<P<Node>>>,
    pub local_jsx_namespace: TextCell,
    pub local_jsx_fragment_namespace: TextCell,
    pub local_jsx_factory: Cell<Option<P<Node>>>,
    pub local_jsx_fragment_factory: Cell<Option<P<Node>>>,
    pub jsx_fragment_type: Cell<Option<P<Type>>>,
}

// Signature specific links

#[derive(Default)]
pub struct SignatureLinks {
    pub resolved_signature: Cell<Option<SignatureKey>>, // Cached signature of signature node or call expression
    pub effects_signature: Cell<Option<SignatureKey>>, // Signature with possible control flow effects
    pub decorator_signature: Cell<Option<SignatureKey>>, // Signature for decorator as if invoked by the runtime
    link_key: Cell<tsrs_core::PKey>, // tsrs: the node this record is filed under (`KeyedLinkStore`), in what was padding
}

impl crate::links::KeyedLinks for SignatureLinks {
    fn link_key(&self) -> &Cell<tsrs_core::PKey> {
        &self.link_key
    }
}

// Qualified signature keys make this 32 bytes on native and 28 on Wasm.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<SignatureLinks>() == 32);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<SignatureLinks>() == 28);

// Note that for types of different kinds, the numeric values of TypeFlags determine the order
// computed by the CompareTypes function and therefore the order of constituent types in union types.
// Since union type processing often bails out early when a result is known, it is important to order
// TypeFlags in increasing order of potential type complexity. In particular, indexed access and
// conditional types should sort last as those types are potentially recursive and possibly infinite.

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct TypeFlags: u32 {
        const None = 0;
        const Any = 1 << 0;
        const Unknown = 1 << 1;
        const Undefined = 1 << 2;
        const Null = 1 << 3;
        const Void = 1 << 4;
        const String = 1 << 5;
        const Number = 1 << 6;
        const BigInt = 1 << 7;
        const Boolean = 1 << 8;
        const ESSymbol = 1 << 9; // Type of symbol primitive introduced in ES6
        const StringLiteral = 1 << 10;
        const NumberLiteral = 1 << 11;
        const BigIntLiteral = 1 << 12;
        const BooleanLiteral = 1 << 13;
        const UniqueESSymbol = 1 << 14; // unique symbol
        const EnumLiteral = 1 << 15; // Always combined with StringLiteral, NumberLiteral, or Union
        const Enum = 1 << 16; // Numeric computed enum member value (must be right after EnumLiteral, see getSortOrderFlags)
        const NonPrimitive = 1 << 17; // intrinsic object type
        const Never = 1 << 18; // Never type
        const TypeParameter = 1 << 19; // Type parameter
        const Object = 1 << 20; // Object type
        const Index = 1 << 21; // keyof T
        const TemplateLiteral = 1 << 22; // Template literal type
        const StringMapping = 1 << 23; // Uppercase/Lowercase type
        const Substitution = 1 << 24; // Type parameter substitution
        const IndexedAccess = 1 << 25; // T[K]
        const Conditional = 1 << 26; // T extends U ? X : Y
        const Union = 1 << 27; // Union (T | U)
        const Intersection = 1 << 28; // Intersection (T & U)
        const Reserved1 = 1 << 29; // Used by union/intersection type construction
        const Reserved2 = 1 << 30; // Used by union/intersection type construction
        const Reserved3 = 1 << 31;

        const AnyOrUnknown = Self::Any.bits() | Self::Unknown.bits();
        const Nullable = Self::Undefined.bits() | Self::Null.bits();
        const Literal = Self::StringLiteral.bits() | Self::NumberLiteral.bits() | Self::BigIntLiteral.bits() | Self::BooleanLiteral.bits();
        const Unit = Self::Enum.bits() | Self::Literal.bits() | Self::UniqueESSymbol.bits() | Self::Nullable.bits();
        const Freshable = Self::Enum.bits() | Self::Literal.bits();
        const StringOrNumberLiteral = Self::StringLiteral.bits() | Self::NumberLiteral.bits();
        const StringOrNumberLiteralOrUnique = Self::StringLiteral.bits() | Self::NumberLiteral.bits() | Self::UniqueESSymbol.bits();
        const DefinitelyFalsy = Self::StringLiteral.bits() | Self::NumberLiteral.bits() | Self::BigIntLiteral.bits() | Self::BooleanLiteral.bits() | Self::Void.bits() | Self::Undefined.bits() | Self::Null.bits();
        const PossiblyFalsy = Self::DefinitelyFalsy.bits() | Self::String.bits() | Self::Number.bits() | Self::BigInt.bits() | Self::Boolean.bits();
        const Intrinsic = Self::Any.bits() | Self::Unknown.bits() | Self::String.bits() | Self::Number.bits() | Self::BigInt.bits() | Self::ESSymbol.bits() | Self::Void.bits() | Self::Undefined.bits() | Self::Null.bits() | Self::Never.bits() | Self::NonPrimitive.bits();
        const StringLike = Self::String.bits() | Self::StringLiteral.bits() | Self::TemplateLiteral.bits() | Self::StringMapping.bits();
        const NumberLike = Self::Number.bits() | Self::NumberLiteral.bits() | Self::Enum.bits();
        const BigIntLike = Self::BigInt.bits() | Self::BigIntLiteral.bits();
        const BooleanLike = Self::Boolean.bits() | Self::BooleanLiteral.bits();
        const EnumLike = Self::Enum.bits() | Self::EnumLiteral.bits();
        const ESSymbolLike = Self::ESSymbol.bits() | Self::UniqueESSymbol.bits();
        const VoidLike = Self::Void.bits() | Self::Undefined.bits();
        const Primitive = Self::StringLike.bits() | Self::NumberLike.bits() | Self::BigIntLike.bits() | Self::BooleanLike.bits() | Self::EnumLike.bits() | Self::ESSymbolLike.bits() | Self::VoidLike.bits() | Self::Null.bits();
        const DefinitelyNonNullable = Self::StringLike.bits() | Self::NumberLike.bits() | Self::BigIntLike.bits() | Self::BooleanLike.bits() | Self::EnumLike.bits() | Self::ESSymbolLike.bits() | Self::Object.bits() | Self::NonPrimitive.bits();
        const DisjointDomains = Self::NonPrimitive.bits() | Self::StringLike.bits() | Self::NumberLike.bits() | Self::BigIntLike.bits() | Self::BooleanLike.bits() | Self::ESSymbolLike.bits() | Self::VoidLike.bits() | Self::Null.bits();
        const UnionOrIntersection = Self::Union.bits() | Self::Intersection.bits();
        const StructuredType = Self::Object.bits() | Self::Union.bits() | Self::Intersection.bits();
        const TypeVariable = Self::TypeParameter.bits() | Self::IndexedAccess.bits();
        const InstantiableNonPrimitive = Self::TypeVariable.bits() | Self::Conditional.bits() | Self::Substitution.bits();
        const InstantiablePrimitive = Self::Index.bits() | Self::TemplateLiteral.bits() | Self::StringMapping.bits();
        const Instantiable = Self::InstantiableNonPrimitive.bits() | Self::InstantiablePrimitive.bits();
        const StructuredOrInstantiable = Self::StructuredType.bits() | Self::Instantiable.bits();
        const ObjectFlagsType = Self::Any.bits() | Self::Nullable.bits() | Self::Never.bits() | Self::Object.bits() | Self::Union.bits() | Self::Intersection.bits();
        const Simplifiable = Self::IndexedAccess.bits() | Self::Conditional.bits() | Self::Index.bits();
        const Singleton = Self::Any.bits() | Self::Unknown.bits() | Self::String.bits() | Self::Number.bits() | Self::Boolean.bits() | Self::BigInt.bits() | Self::ESSymbol.bits() | Self::Void.bits() | Self::Undefined.bits() | Self::Null.bits() | Self::Never.bits() | Self::NonPrimitive.bits();
        // 'TypeFlagsNarrowable' types are types where narrowing actually narrows.
        // This *should* be every type other than null, undefined, void, and never
        const Narrowable = Self::Any.bits() | Self::Unknown.bits() | Self::StructuredOrInstantiable.bits() | Self::StringLike.bits() | Self::NumberLike.bits() | Self::BigIntLike.bits() | Self::BooleanLike.bits() | Self::ESSymbol.bits() | Self::UniqueESSymbol.bits() | Self::NonPrimitive.bits();
        // The following flags are aggregated during union and intersection type construction
        const IncludesMask = Self::Any.bits() | Self::Unknown.bits() | Self::Primitive.bits() | Self::Never.bits() | Self::Object.bits() | Self::Union.bits() | Self::Intersection.bits() | Self::NonPrimitive.bits() | Self::TemplateLiteral.bits() | Self::StringMapping.bits();
        // The following flags are used for different purposes during union and intersection type construction
        const IncludesMissingType = Self::TypeParameter.bits();
        const IncludesNonWideningType = Self::Index.bits();
        const IncludesWildcard = Self::IndexedAccess.bits();
        const IncludesEmptyObject = Self::Conditional.bits();
        const IncludesInstantiable = Self::Substitution.bits();
        const IncludesConstrainedTypeVariable = Self::Reserved1.bits();
        const IncludesError = Self::Reserved2.bits();
        const NotPrimitiveUnion = Self::Any.bits() | Self::Unknown.bits() | Self::Void.bits() | Self::Never.bits() | Self::Object.bits() | Self::Intersection.bits() | Self::IncludesInstantiable.bits();
    }
}

static typeFlagNames: [(TypeFlags, &str); 29] = [
    (TypeFlags::Any, "Any"),
    (TypeFlags::Unknown, "Unknown"),
    (TypeFlags::Undefined, "Undefined"),
    (TypeFlags::Null, "Null"),
    (TypeFlags::Void, "Void"),
    (TypeFlags::String, "String"),
    (TypeFlags::Number, "Number"),
    (TypeFlags::BigInt, "BigInt"),
    (TypeFlags::Boolean, "Boolean"),
    (TypeFlags::ESSymbol, "ESSymbol"),
    (TypeFlags::StringLiteral, "StringLiteral"),
    (TypeFlags::NumberLiteral, "NumberLiteral"),
    (TypeFlags::BigIntLiteral, "BigIntLiteral"),
    (TypeFlags::BooleanLiteral, "BooleanLiteral"),
    (TypeFlags::UniqueESSymbol, "UniqueESSymbol"),
    (TypeFlags::EnumLiteral, "EnumLiteral"),
    (TypeFlags::Enum, "Enum"),
    (TypeFlags::NonPrimitive, "NonPrimitive"),
    (TypeFlags::Never, "Never"),
    (TypeFlags::TypeParameter, "TypeParameter"),
    (TypeFlags::Object, "Object"),
    (TypeFlags::Index, "Index"),
    (TypeFlags::TemplateLiteral, "TemplateLiteral"),
    (TypeFlags::StringMapping, "StringMapping"),
    (TypeFlags::Substitution, "Substitution"),
    (TypeFlags::IndexedAccess, "IndexedAccess"),
    (TypeFlags::Conditional, "Conditional"),
    (TypeFlags::Union, "Union"),
    (TypeFlags::Intersection, "Intersection"),
];

/// FormatTypeFlags returns the individual flag names as a slice of strings.
pub fn format_type_flags(flags: TypeFlags) -> Vec<&'static str> {
    let mut result = Vec::with_capacity(flags.bits().count_ones() as usize);
    for (flag, name) in typeFlagNames.iter() {
        if flags.intersects(*flag) {
            result.push(*name);
        }
    }
    if result.is_empty() {
        result.push("None");
    }
    result
}

/// Go `TypeFlags.String()`: a pipe-separated string of flag names.
impl fmt::Display for TypeFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format_type_flags(*self).join("|"))
    }
}

/// Go `VarianceFlags.String()`.
impl fmt::Display for VarianceFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variance = *self & VarianceFlags::VarianceMask;
        let mut result = String::from(if variance == VarianceFlags::Invariant {
            "in out"
        } else if variance == VarianceFlags::Bivariant {
            "[bivariant]"
        } else if variance == VarianceFlags::Contravariant {
            "in"
        } else if variance == VarianceFlags::Covariant {
            "out"
        } else if variance == VarianceFlags::Independent {
            "[independent]"
        } else {
            ""
        });
        if self.intersects(VarianceFlags::Unmeasurable) {
            result.push_str(" (unmeasurable)");
        } else if self.intersects(VarianceFlags::Unreliable) {
            result.push_str(" (unreliable)");
        }
        f.write_str(&result)
    }
}

// Types included in TypeFlags.ObjectFlagsType have an objectFlags property. Some ObjectFlags
// are specific to certain types and reuse the same bit position. Those ObjectFlags require a check
// for a certain TypeFlags value to determine their meaning.
bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct ObjectFlags: u32 {
        const None = 0;
        const Class = 1 << 0; // Class
        const Interface = 1 << 1; // Interface
        const Reference = 1 << 2; // Generic type reference
        const Tuple = 1 << 3; // Synthesized generic tuple type
        const Anonymous = 1 << 4; // Anonymous
        const Mapped = 1 << 5; // Mapped
        const Instantiated = 1 << 6; // Instantiated anonymous or mapped type
        const ObjectLiteral = 1 << 7; // Originates in an object literal
        const EvolvingArray = 1 << 8; // Evolving array type
        const ObjectLiteralPatternWithComputedProperties = 1 << 9; // Object literal pattern with computed properties
        const ReverseMapped = 1 << 10; // Object contains a property from a reverse-mapped type
        const JsxAttributes = 1 << 11; // Jsx attributes type
        const JSLiteral = 1 << 12; // Object type declared in JS - disables errors on read/write of nonexisting members
        const FreshLiteral = 1 << 13; // Fresh object literal
        const ArrayLiteral = 1 << 14; // Originates in an array literal
        const PrimitiveUnion = 1 << 15; // Union of only primitive types
        const ContainsWideningType = 1 << 16; // Type is or contains undefined or null widening type
        const ContainsObjectOrArrayLiteral = 1 << 17; // Type is or contains object literal type
        const NonInferrableType = 1 << 18; // Type is or contains anyFunctionType or silentNeverType
        const CouldContainTypeVariablesComputed = 1 << 19; // CouldContainTypeVariables flag has been computed
        const CouldContainTypeVariables = 1 << 20; // Type could contain a type variable
        const MembersResolved = 1 << 21; // Members have been resolved

        const ClassOrInterface = Self::Class.bits() | Self::Interface.bits();
        const RequiresWidening = Self::ContainsWideningType.bits() | Self::ContainsObjectOrArrayLiteral.bits();
        const PropagatingFlags = Self::ContainsWideningType.bits() | Self::ContainsObjectOrArrayLiteral.bits() | Self::NonInferrableType.bits();
        const InstantiatedMapped = Self::Mapped.bits() | Self::Instantiated.bits();
        // Object flags that uniquely identify the kind of ObjectType
        const ObjectTypeKindMask = Self::ClassOrInterface.bits() | Self::Reference.bits() | Self::Tuple.bits() | Self::Anonymous.bits() | Self::Mapped.bits() | Self::ReverseMapped.bits() | Self::EvolvingArray.bits() | Self::InstantiationExpressionType.bits() | Self::SingleSignatureType.bits();
        // Flags that require TypeFlags.Object
        const ContainsSpread = 1 << 22; // Object literal contains spread operation
        const ObjectRestType = 1 << 23; // Originates in object rest declaration
        const InstantiationExpressionType = 1 << 24; // Originates in instantiation expression
        const SingleSignatureType = 1 << 25; // A single signature type extracted from a potentially broader type
        const IsClassInstanceClone = 1 << 26; // Type is a clone of a class instance type
        // Flags that require TypeFlags.Object and ObjectFlags.Reference
        const IdenticalBaseTypeCalculated = 1 << 27; // has had `getSingleBaseForNonAugmentingSubtype` invoked on it already
        const IdenticalBaseTypeExists = 1 << 28; // has a defined cachedEquivalentBaseType member
        const FromTypeNode = 1 << 29; // Originates in resolution of AST type node
        // Flags that require TypeFlags.UnionOrIntersection or TypeFlags.Substitution
        const IsGenericTypeComputed = 1 << 22; // IsGenericObjectType flag has been computed
        const IsGenericObjectType = 1 << 23; // Union or intersection contains generic object type
        const IsGenericIndexType = 1 << 24; // Union or intersection contains generic index type
        const IsGenericType = Self::IsGenericObjectType.bits() | Self::IsGenericIndexType.bits();
        // Flags that require TypeFlags.Union
        const ContainsIntersections = 1 << 25; // Union contains intersections
        const IsUnknownLikeUnionComputed = 1 << 26; // IsUnknownLikeUnion flag has been computed
        const IsUnknownLikeUnion = 1 << 27; // Union of null, undefined, and empty object type
        const IsUniformEnumComputed = 1 << 28; // IsUniformEnum flag has been computed
        const IsUniformEnum = 1 << 29; // Union contains uniform literal types
        // Flags that require TypeFlags.Intersection
        const IsNeverIntersectionComputed = 1 << 25; // IsNeverLike flag has been computed
        const IsNeverIntersection = 1 << 26; // Intersection reduces to never
        const IsConstrainedTypeVariable = 1 << 27; // T & C, where T's constraint and C are primitives, object, or {}
    }
}

// TypeAlias

pub type TypeAliasKey = tsrs_core::arena_owner::ArenaKey<TypeAlias>;

#[derive(Default)]
pub struct TypeAlias {
    symbol: Option<P<Symbol>>,
    type_arguments: ArrayCell<P<Type>>,
}

const _: () = assert!(std::mem::size_of::<TypeAlias>() == if cfg!(target_pointer_width = "64") { 16 } else { 8 });

impl TypeAlias {
    pub(crate) fn new(symbol: Option<P<Symbol>>, type_arguments: &[P<Type>]) -> Self {
        Self { symbol, type_arguments: ArrayCell::new(type_arguments) }
    }

    pub fn symbol(&self) -> Option<P<Symbol>> {
        self.symbol
    }
    pub fn type_arguments(&self) -> ArrayView<P<Type>> {
        self.type_arguments.get()
    }

    pub(crate) fn set_type_arguments(&self, type_arguments: Vec<P<Type>>) {
        self.type_arguments.set_owned(type_arguments);
    }
}

/// The alias argument of the type constructors that look up a cache before they create a type (union,
/// intersection, indexed access, object type instantiation; Go passes a `*TypeAlias`). `Pending` is an instantiated
/// alias (Go's `instantiateTypeAlias` result) that is allocated only when a type is created with it: the cache key
/// needs only its symbol and type arguments, and most calls return a cached type (on the 38k-file codebase 1.79M of the 2.34M
/// instantiated aliases were dropped that way). A pending alias is allocated at most once, so every type created
/// with it shares one `TypeAlias`, as before.
#[derive(Clone, Copy, Default)]
pub enum AliasArg<'a> {
    #[default]
    None,
    Some(TypeAliasKey),
    Pending(&'a PendingTypeAlias),
}

pub struct PendingTypeAlias {
    pub(crate) symbol: Option<P<Symbol>>,
    pub(crate) type_arguments: Vec<P<Type>>,
    alias: Cell<Option<TypeAliasKey>>,
}

impl PendingTypeAlias {
    pub fn new(symbol: Option<P<Symbol>>, type_arguments: Vec<P<Type>>) -> PendingTypeAlias {
        PendingTypeAlias { symbol, type_arguments, alias: Cell::new(None) }
    }

    fn materialize(&self, aliases: &mut tsrs_core::arena_owner::ArenaBuilder<TypeAlias>) -> TypeAliasKey {
        if let Some(key) = self.alias.get() {
            aliases.get(key).expect("pending type alias belongs to another checker");
            return key;
        }
        let key = aliases.alloc(TypeAlias::new(self.symbol, &self.type_arguments));
        self.alias.set(Some(key));
        key
    }
}

impl From<Option<TypeAliasKey>> for AliasArg<'_> {
    fn from(alias: Option<TypeAliasKey>) -> Self {
        alias.map_or(AliasArg::None, AliasArg::Some)
    }
}

impl<'a> AliasArg<'a> {
    /// `alias` if given, else the pending one (Go: `if alias == nil { alias = c.instantiateTypeAlias(...) }`).
    pub fn given_or_pending(alias: Option<TypeAliasKey>, pending: &'a Option<PendingTypeAlias>) -> AliasArg<'a> {
        match (alias, pending) {
            (Some(alias), _) => AliasArg::Some(alias),
            (None, Some(pending)) => AliasArg::Pending(pending),
            (None, None) => AliasArg::None,
        }
    }

    pub fn is_none(self) -> bool {
        matches!(self, AliasArg::None)
    }

    /// The alias to store in a created type: a pending alias is allocated on the first call.
    pub fn alias(self, c: &mut Checker) -> Option<TypeAliasKey> {
        match self {
            AliasArg::None => None,
            AliasArg::Some(a) => {
                c.type_alias(a);
                Some(a)
            }
            AliasArg::Pending(p) => Some(p.materialize(&mut c.type_aliases)),
        }
    }
}

/// Go's nil-receiver `(*TypeAlias).Symbol()` / `TypeArguments()`, resolved through the owning checker.
pub trait TypeAliasOptExt {
    fn symbol(self, c: &Checker) -> Option<P<Symbol>>;
    fn type_arguments(self, c: &Checker) -> ArrayView<P<Type>>;
}

impl TypeAliasOptExt for Option<TypeAliasKey> {
    fn symbol(self, c: &Checker) -> Option<P<Symbol>> {
        self.and_then(|a| c.type_alias(a).symbol())
    }
    fn type_arguments(self, c: &Checker) -> ArrayView<P<Type>> {
        self.map_or_else(ArrayView::default, |a| c.type_alias(a).type_arguments())
    }
}

impl TypeAliasOptExt for TypeAliasKey {
    fn symbol(self, c: &Checker) -> Option<P<Symbol>> {
        c.type_alias(self).symbol()
    }
    fn type_arguments(self, c: &Checker) -> ArrayView<P<Type>> {
        c.type_alias(self).type_arguments()
    }
}

// Type
//
// A type owns its payload variant. Views borrow this record; graph edges inside payloads still use legacy P.

pub struct Type {
    pub flags: Cell<TypeFlags>,
    pub object_flags: Cell<ObjectFlags>,
    pub id: TypeId,
    symbol: Cell<Option<P<Symbol>>>,
    alias: Cell<Option<TypeAliasKey>>,
    data: OwnedTypeData,
}

const _: () = assert!(std::mem::size_of::<Type>() == if cfg!(target_pointer_width = "64") { 48 } else { 32 });

#[repr(C, u8)]
pub(crate) enum OwnedTypeData {
    Intrinsic(Box<IntrinsicType>),
    Literal(Box<LiteralType>),
    UniqueESSymbol(Box<UniqueESSymbolType>),
    Object(Box<ObjectType>),
    TypeReference(Box<TypeReference>),
    Interface(Box<InterfaceType>),
    Tuple(Box<TupleType>),
    InstantiationExpression(Box<InstantiationExpressionType>),
    Mapped(Box<MappedType>),
    ReverseMapped(Box<ReverseMappedType>),
    EvolvingArray(Box<EvolvingArrayType>),
    Union(Box<UnionType>),
    Intersection(Box<IntersectionType>),
    TypeParameter(Box<TypeParameter>),
    Index(Box<IndexType>),
    IndexedAccess(Box<IndexedAccessType>),
    TemplateLiteral(Box<TemplateLiteralType>),
    StringMapping(Box<StringMappingType>),
    Substitution(Box<SubstitutionType>),
    Conditional(Box<ConditionalType>),
}

/// Census builds (`TSRS_CENSUS=1`): registers the fields of checker arena types that the census's strong mark must
/// not read as plain pointers (`tsrs_core::census_layout`), from the current layouts: type headers (flags, ids and
/// the owned payload discriminant), literal values, type parameter and mapped type flags,
/// empty type-argument slices, conditional roots, inference infos. Once per process (with the AST's); nothing in
/// other builds.
pub(crate) fn census_layouts() {
    use std::any::type_name;
    use std::mem::{offset_of, size_of};
    use tsrs_core::CensusField;
    if !tsrs_core::census_recording() {
        return;
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tsrs_ast::census_layouts();
        crate::mapper::census_layouts();
        // OwnedTypeData is repr(C, u8): its discriminant precedes an aligned one-pointer variant payload.
        let pointer = offset_of!(Type, data) + std::mem::align_of::<Box<IntrinsicType>>();
        let header = CensusField::all_but(0, size_of::<Type>(), &[offset_of!(Type, symbol), pointer]);
        tsrs_core::census_layout(type_name::<Type>(), &header);
        let name = type_name::<LiteralType>();
        // Literal snapshots own their text; discriminants and numeric payload bytes are not graph edges.
        let d = 0;
        let value = CensusField::NoPointer { off: d + offset_of!(LiteralType, value), len: size_of::<SnapshotCell<Option<LiteralValue>>>() };
        tsrs_core::census_layout(name, &[value]);
        let d = 0;
        let offsets = [
            offset_of!(TypeParameter, constrained_type),
            offset_of!(TypeParameter, constraint),
            offset_of!(TypeParameter, target),
            offset_of!(TypeParameter, mapper),
            offset_of!(TypeParameter, is_this_type),
            offset_of!(TypeParameter, is_distributed),
            offset_of!(TypeParameter, resolved_default_type),
            offset_of!(TypeParameter, distributed_type),
        ];
        let size = size_of::<TypeParameter>();
        tsrs_core::census_layout(
            type_name::<TypeParameter>(),
            &[
                CensusField::scalar(d, offset_of!(TypeParameter, is_this_type), &offsets, size),
                CensusField::scalar(d, offset_of!(TypeParameter, is_distributed), &offsets, size),
                CensusField::NoPointer { off: offset_of!(TypeParameter, mapper), len: size_of::<MapperCell>() },
            ],
        );
        let d = 0;
        let offsets = [
            offset_of!(MappedType, object_type),
            offset_of!(MappedType, declaration),
            offset_of!(MappedType, type_parameter),
            offset_of!(MappedType, constraint_type),
            offset_of!(MappedType, name_type),
            offset_of!(MappedType, template_type),
            offset_of!(MappedType, modifiers_type),
            offset_of!(MappedType, resolved_apparent_type),
            offset_of!(MappedType, contains_error),
        ];
        let contains_error = CensusField::scalar(d, offset_of!(MappedType, contains_error), &offsets, size_of::<MappedType>());
        let mapper = CensusField::NoPointer { off: offset_of!(MappedType, object_type) + offset_of!(ObjectType, mapper), len: size_of::<MapperCell>() };
        tsrs_core::census_layout(type_name::<MappedType>(), &[contains_error, mapper]);
        let object_mapper = offset_of!(ObjectType, mapper);
        let reference_mapper = offset_of!(TypeReference, object_type) + object_mapper;
        let interface_mapper = offset_of!(InterfaceType, type_reference) + reference_mapper;
        for (name, off) in [
            (type_name::<ObjectType>(), object_mapper),
            (type_name::<TypeReference>(), reference_mapper),
            (type_name::<InterfaceType>(), interface_mapper),
            (type_name::<TupleType>(), offset_of!(TupleType, interface_type) + interface_mapper),
            (type_name::<InstantiationExpressionType>(), offset_of!(InstantiationExpressionType, object_type) + object_mapper),
            (type_name::<ReverseMappedType>(), offset_of!(ReverseMappedType, object_type) + object_mapper),
            (type_name::<EvolvingArrayType>(), offset_of!(EvolvingArrayType, object_type) + object_mapper),
        ] {
            tsrs_core::census_layout(name, &[CensusField::NoPointer { off, len: size_of::<MapperCell>() }]);
        }
        tsrs_core::census_layout(type_name::<ConditionalType>(), &[
            CensusField::NoPointer { off: offset_of!(ConditionalType, mapper), len: size_of::<MapperCell>() },
            CensusField::NoPointer { off: offset_of!(ConditionalType, combined_mapper), len: size_of::<MapperCell>() },
        ]);
        // Owned array cells contain a nullable, thin Arc<Vec<T>> pointer. The mark follows the ordinary heap
        // allocations, rather than interpreting the field as the former arena slice header.
        let u = |d: usize| {
            let base = d + offset_of!(UnionType, union_or_intersection_type);
            [
                CensusField::NoPointer { off: base + offset_of!(UnionOrIntersectionType, rare), len: std::mem::align_of::<Box<UnionRare>>() },
            ]
        };
        tsrs_core::census_layout(type_name::<UnionType>(), &u(0));
        const _: () = assert!(offset_of!(UnionType, union_or_intersection_type) == offset_of!(IntersectionType, union_or_intersection_type));
        tsrs_core::census_layout(type_name::<IntersectionType>(), &u(0));
        let pointers = [offset_of!(StructuredMembers, members), offset_of!(StructuredMembers, properties), offset_of!(StructuredMembers, signatures), offset_of!(StructuredMembers, count_or_index_infos) + offset_of!(CountOrIndexInfos, index_infos)];
        tsrs_core::census_layout(type_name::<StructuredMembers>(), &CensusField::all_but(0, size_of::<StructuredMembers>(), &pointers));
        let pointers = [
            offset_of!(Signature, declaration), offset_of!(Signature, type_parameters), offset_of!(Signature, parameters),
            offset_of!(Signature, resolved_return_type),
            offset_of!(Signature, rare),
        ];
        tsrs_core::census_layout(type_name::<Signature>(), &CensusField::all_but(0, size_of::<Signature>(), &pointers));
        let offsets = [
            offset_of!(ConditionalRoot, node),
            offset_of!(ConditionalRoot, check_type),
            offset_of!(ConditionalRoot, extends_type),
            offset_of!(ConditionalRoot, is_distributive),
            offset_of!(ConditionalRoot, infer_type_parameters),
            offset_of!(ConditionalRoot, outer_type_parameters),
            offset_of!(ConditionalRoot, instantiations),
            offset_of!(ConditionalRoot, alias),
        ];
        let is_distributive = CensusField::scalar(0, offset_of!(ConditionalRoot, is_distributive), &offsets, size_of::<ConditionalRoot>());
        let alias = CensusField::NoPointer { off: offset_of!(ConditionalRoot, alias), len: size_of::<Option<TypeAliasKey>>() };
        tsrs_core::census_layout(type_name::<ConditionalRoot>(), &[is_distributive, alias]);
        let offsets = [
            offset_of!(InferenceInfo, type_parameter),
            offset_of!(InferenceInfo, candidates),
            offset_of!(InferenceInfo, contra_candidates),
            offset_of!(InferenceInfo, inferred_type),
            offset_of!(InferenceInfo, priority),
            offset_of!(InferenceInfo, top_level),
            offset_of!(InferenceInfo, is_fixed),
            offset_of!(InferenceInfo, implied_arity),
        ];
        let size = size_of::<InferenceInfo>();
        let scalars: Vec<CensusField> = [
            offset_of!(InferenceInfo, priority),
            offset_of!(InferenceInfo, top_level),
            offset_of!(InferenceInfo, is_fixed),
            offset_of!(InferenceInfo, implied_arity),
        ]
        .iter()
        .map(|&o| CensusField::scalar(0, o, &offsets, size))
        .collect();
        tsrs_core::census_layout(type_name::<InferenceInfo>(), &scalars);
    });
}

#[cfg_attr(feature = "alloc-profile", track_caller)]
pub(crate) fn owned_type_payload<T>(value: T) -> Box<T> {
    #[cfg(feature = "alloc-profile")]
    { tsrs_core::alloc_profile::owned_box(value) }
    #[cfg(not(feature = "alloc-profile"))]
    { Box::new(value) }
}

/// The closed set of payload records that can be placed in a type.
pub(crate) trait TypePayload: Sized + 'static {
    fn into_owned(self) -> OwnedTypeData;
    fn borrow(data: &OwnedTypeData) -> Option<&Self>;
}

/// Which payload a type owns.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TypeDataTag {
    Intrinsic,
    Literal,
    UniqueESSymbol,
    Object,
    TypeReference,
    Interface,
    Tuple,
    InstantiationExpression,
    Mapped,
    ReverseMapped,
    EvolvingArray,
    Union,
    Intersection,
    TypeParameter,
    Index,
    IndexedAccess,
    TemplateLiteral,
    StringMapping,
    Substitution,
    Conditional,
}

impl TypePayload for IntrinsicType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Intrinsic(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Intrinsic(value) => Some(value), _ => None }
    }
}
impl TypePayload for LiteralType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Literal(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Literal(value) => Some(value), _ => None }
    }
}
impl TypePayload for UniqueESSymbolType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::UniqueESSymbol(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::UniqueESSymbol(value) => Some(value), _ => None }
    }
}
impl TypePayload for ObjectType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Object(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Object(value) => Some(value), _ => None }
    }
}
impl TypePayload for TypeReference {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::TypeReference(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::TypeReference(value) => Some(value), _ => None }
    }
}
impl TypePayload for InterfaceType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Interface(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Interface(value) => Some(value), _ => None }
    }
}
impl TypePayload for TupleType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Tuple(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Tuple(value) => Some(value), _ => None }
    }
}
impl TypePayload for InstantiationExpressionType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::InstantiationExpression(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::InstantiationExpression(value) => Some(value), _ => None }
    }
}
impl TypePayload for MappedType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Mapped(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Mapped(value) => Some(value), _ => None }
    }
}
impl TypePayload for ReverseMappedType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::ReverseMapped(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::ReverseMapped(value) => Some(value), _ => None }
    }
}
impl TypePayload for EvolvingArrayType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::EvolvingArray(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::EvolvingArray(value) => Some(value), _ => None }
    }
}
impl TypePayload for UnionType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Union(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Union(value) => Some(value), _ => None }
    }
}
impl TypePayload for IntersectionType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Intersection(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Intersection(value) => Some(value), _ => None }
    }
}
impl TypePayload for TypeParameter {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::TypeParameter(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::TypeParameter(value) => Some(value), _ => None }
    }
}
impl TypePayload for IndexType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Index(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Index(value) => Some(value), _ => None }
    }
}
impl TypePayload for IndexedAccessType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::IndexedAccess(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::IndexedAccess(value) => Some(value), _ => None }
    }
}
impl TypePayload for TemplateLiteralType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::TemplateLiteral(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::TemplateLiteral(value) => Some(value), _ => None }
    }
}
impl TypePayload for StringMappingType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::StringMapping(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::StringMapping(value) => Some(value), _ => None }
    }
}
impl TypePayload for SubstitutionType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Substitution(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Substitution(value) => Some(value), _ => None }
    }
}
impl TypePayload for ConditionalType {
    fn into_owned(self) -> OwnedTypeData { OwnedTypeData::Conditional(owned_type_payload(self)) }
    fn borrow(data: &OwnedTypeData) -> Option<&Self> {
        match data { OwnedTypeData::Conditional(value) => Some(value), _ => None }
    }
}
impl Type {
    /// Construct the owning record. Its payload is released by ordinary Rust destruction.
    pub(crate) fn alloc<T: TypePayload>(flags: TypeFlags, object_flags: ObjectFlags, id: TypeId, data: T) -> P<Type> {
        P::new(Self::from_payload(flags, object_flags, id, data))
    }

    pub(crate) fn from_payload<T: TypePayload>(flags: TypeFlags, object_flags: ObjectFlags, id: TypeId, data: T) -> Self {
        Self { flags: Cell::new(flags), object_flags: Cell::new(object_flags), id, symbol: Cell::new(None), alias: Cell::new(None), data: data.into_owned() }
    }

    #[inline]
    fn payload<T: TypePayload>(&self) -> &T {
        T::borrow(&self.data).expect("wrong type payload variant")
    }

    /// The type-specific data, borrowed from this owning record.
    #[inline]
    pub fn data(&self) -> TypeData<'_> {
        match &self.data {
            OwnedTypeData::Intrinsic(value) => TypeData::Intrinsic(value),
            OwnedTypeData::Literal(value) => TypeData::Literal(value),
            OwnedTypeData::UniqueESSymbol(value) => TypeData::UniqueESSymbol(value),
            OwnedTypeData::Object(value) => TypeData::Object(value),
            OwnedTypeData::TypeReference(value) => TypeData::TypeReference(value),
            OwnedTypeData::Interface(value) => TypeData::Interface(value),
            OwnedTypeData::Tuple(value) => TypeData::Tuple(value),
            OwnedTypeData::InstantiationExpression(value) => TypeData::InstantiationExpression(value),
            OwnedTypeData::Mapped(value) => TypeData::Mapped(value),
            OwnedTypeData::ReverseMapped(value) => TypeData::ReverseMapped(value),
            OwnedTypeData::EvolvingArray(value) => TypeData::EvolvingArray(value),
            OwnedTypeData::Union(value) => TypeData::Union(value),
            OwnedTypeData::Intersection(value) => TypeData::Intersection(value),
            OwnedTypeData::TypeParameter(value) => TypeData::TypeParameter(value),
            OwnedTypeData::Index(value) => TypeData::Index(value),
            OwnedTypeData::IndexedAccess(value) => TypeData::IndexedAccess(value),
            OwnedTypeData::TemplateLiteral(value) => TypeData::TemplateLiteral(value),
            OwnedTypeData::StringMapping(value) => TypeData::StringMapping(value),
            OwnedTypeData::Substitution(value) => TypeData::Substitution(value),
            OwnedTypeData::Conditional(value) => TypeData::Conditional(value),
        }
    }

    #[inline]
    pub fn data_tag(&self) -> TypeDataTag {
        match &self.data {
            OwnedTypeData::Intrinsic(_) => TypeDataTag::Intrinsic,
            OwnedTypeData::Literal(_) => TypeDataTag::Literal,
            OwnedTypeData::UniqueESSymbol(_) => TypeDataTag::UniqueESSymbol,
            OwnedTypeData::Object(_) => TypeDataTag::Object,
            OwnedTypeData::TypeReference(_) => TypeDataTag::TypeReference,
            OwnedTypeData::Interface(_) => TypeDataTag::Interface,
            OwnedTypeData::Tuple(_) => TypeDataTag::Tuple,
            OwnedTypeData::InstantiationExpression(_) => TypeDataTag::InstantiationExpression,
            OwnedTypeData::Mapped(_) => TypeDataTag::Mapped,
            OwnedTypeData::ReverseMapped(_) => TypeDataTag::ReverseMapped,
            OwnedTypeData::EvolvingArray(_) => TypeDataTag::EvolvingArray,
            OwnedTypeData::Union(_) => TypeDataTag::Union,
            OwnedTypeData::Intersection(_) => TypeDataTag::Intersection,
            OwnedTypeData::TypeParameter(_) => TypeDataTag::TypeParameter,
            OwnedTypeData::Index(_) => TypeDataTag::Index,
            OwnedTypeData::IndexedAccess(_) => TypeDataTag::IndexedAccess,
            OwnedTypeData::TemplateLiteral(_) => TypeDataTag::TemplateLiteral,
            OwnedTypeData::StringMapping(_) => TypeDataTag::StringMapping,
            OwnedTypeData::Substitution(_) => TypeDataTag::Substitution,
            OwnedTypeData::Conditional(_) => TypeDataTag::Conditional,
        }
    }
}

impl Type {
    pub fn id(&self) -> TypeId {
        self.id
    }

    pub fn flags(&self) -> TypeFlags {
        self.flags.get()
    }

    pub fn object_flags(&self) -> ObjectFlags {
        self.object_flags.get()
    }

    // Casts for concrete struct types

    #[inline]
    pub fn as_intrinsic_type(&self) -> &IntrinsicType {
        if self.data_tag() != TypeDataTag::Intrinsic {
            panic!("as_intrinsic_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_literal_type(&self) -> &LiteralType {
        if self.data_tag() != TypeDataTag::Literal {
            panic!("as_literal_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_unique_es_symbol_type(&self) -> &UniqueESSymbolType {
        if self.data_tag() != TypeDataTag::UniqueESSymbol {
            panic!("as_unique_es_symbol_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_tuple_type(&self) -> &TupleType {
        if self.data_tag() != TypeDataTag::Tuple {
            panic!("as_tuple_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_instantiation_expression_type(&self) -> &InstantiationExpressionType {
        if self.data_tag() != TypeDataTag::InstantiationExpression {
            panic!("as_instantiation_expression_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_mapped_type(&self) -> &MappedType {
        if self.data_tag() != TypeDataTag::Mapped {
            panic!("as_mapped_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_reverse_mapped_type(&self) -> &ReverseMappedType {
        if self.data_tag() != TypeDataTag::ReverseMapped {
            panic!("as_reverse_mapped_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_evolving_array_type(&self) -> &EvolvingArrayType {
        if self.data_tag() != TypeDataTag::EvolvingArray {
            panic!("as_evolving_array_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_type_parameter(&self) -> &TypeParameter {
        if self.data_tag() != TypeDataTag::TypeParameter {
            panic!("as_type_parameter: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_union_type(&self) -> &UnionType {
        if self.data_tag() != TypeDataTag::Union {
            panic!("as_union_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_intersection_type(&self) -> &IntersectionType {
        if self.data_tag() != TypeDataTag::Intersection {
            panic!("as_intersection_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_index_type(&self) -> &IndexType {
        if self.data_tag() != TypeDataTag::Index {
            panic!("as_index_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_indexed_access_type(&self) -> &IndexedAccessType {
        if self.data_tag() != TypeDataTag::IndexedAccess {
            panic!("as_indexed_access_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_template_literal_type(&self) -> &TemplateLiteralType {
        if self.data_tag() != TypeDataTag::TemplateLiteral {
            panic!("as_template_literal_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_string_mapping_type(&self) -> &StringMappingType {
        if self.data_tag() != TypeDataTag::StringMapping {
            panic!("as_string_mapping_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_substitution_type(&self) -> &SubstitutionType {
        if self.data_tag() != TypeDataTag::Substitution {
            panic!("as_substitution_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_conditional_type(&self) -> &ConditionalType {
        if self.data_tag() != TypeDataTag::Conditional {
            panic!("as_conditional_type: wrong type data");
        }
        self.payload()
    }

    // Casts for embedded struct types. `as_*` panics where Go would return nil; `try_as_*` mirrors Go's nil result.

    pub fn as_constrained_type(&self) -> &ConstrainedType {
        self.data().as_constrained_type().expect("as_constrained_type: wrong type data")
    }
    pub fn as_structured_type(&self) -> &StructuredType {
        self.data().as_structured_type().expect("as_structured_type: wrong type data")
    }
    pub fn as_object_type(&self) -> &ObjectType {
        self.data().as_object_type().expect("as_object_type: wrong type data")
    }
    pub fn as_type_reference(&self) -> &TypeReference {
        self.data().as_type_reference().expect("as_type_reference: wrong type data")
    }
    pub fn as_interface_type(&self) -> &InterfaceType {
        self.data().as_interface_type().expect("as_interface_type: wrong type data")
    }
    pub fn as_union_or_intersection_type(&self) -> &UnionOrIntersectionType {
        self.data().as_union_or_intersection_type().expect("as_union_or_intersection_type: wrong type data")
    }
    pub fn try_as_constrained_type(&self) -> Option<&ConstrainedType> {
        self.data().as_constrained_type()
    }
    pub fn try_as_structured_type(&self) -> Option<&StructuredType> {
        self.data().as_structured_type()
    }
    pub fn try_as_object_type(&self) -> Option<&ObjectType> {
        self.data().as_object_type()
    }
    pub fn try_as_type_reference(&self) -> Option<&TypeReference> {
        self.data().as_type_reference()
    }
    pub fn try_as_interface_type(&self) -> Option<&InterfaceType> {
        self.data().as_interface_type()
    }
    pub fn try_as_union_or_intersection_type(&self) -> Option<&UnionOrIntersectionType> {
        self.data().as_union_or_intersection_type()
    }

    // Common accessors

    pub fn target(&self) -> Option<P<Type>> {
        let flags = self.flags.get();
        if flags.intersects(TypeFlags::Object) {
            return self.as_object_type().target.get();
        }
        if flags.intersects(TypeFlags::TypeParameter) {
            return self.as_type_parameter().target.get();
        }
        if flags.intersects(TypeFlags::Index) {
            return self.as_index_type().target.get();
        }
        if flags.intersects(TypeFlags::StringMapping) {
            return self.as_string_mapping_type().target.get();
        }
        panic!("Unhandled case in Type.Target")
    }

    pub fn mapper(&self) -> Option<TypeMapperKey> {
        let flags = self.flags.get();
        if flags.intersects(TypeFlags::Object) {
            return self.as_object_type().mapper.get();
        }
        if flags.intersects(TypeFlags::TypeParameter) {
            return self.as_type_parameter().mapper.get();
        }
        if flags.intersects(TypeFlags::Conditional) {
            return self.as_conditional_type().mapper.get();
        }
        panic!("Unhandled case in Type.Mapper")
    }

    pub fn types(&self) -> ArrayView<P<Type>> {
        let flags = self.flags.get();
        if flags.intersects(TypeFlags::UnionOrIntersection) {
            return self.as_union_or_intersection_type().types.get();
        }
        if flags.intersects(TypeFlags::TemplateLiteral) {
            return self.as_template_literal_type().types.get();
        }
        panic!("Unhandled case in Type.Types")
    }

    /// Retain the reference target edge before borrowing a payload from it.
    pub fn reference_target(&self) -> P<Type> {
        self.as_type_reference().target.get().unwrap()
    }

    #[inline]
    pub fn symbol(&self) -> Option<P<Symbol>> { self.symbol.get() }

    #[inline]
    pub fn set_symbol(&self, symbol: Option<P<Symbol>>) { self.symbol.set(symbol); }

    #[inline]
    pub fn alias(&self) -> Option<TypeAliasKey> { self.alias.get() }

    pub fn set_alias(&self, alias: Option<TypeAliasKey>) { self.alias.set(alias); }

    pub fn is_union(&self) -> bool {
        self.flags.get().intersects(TypeFlags::Union)
    }

    pub fn is_string(&self) -> bool {
        self.flags.get().intersects(TypeFlags::String)
    }

    pub fn is_intersection(&self) -> bool {
        self.flags.get().intersects(TypeFlags::Intersection)
    }

    pub fn is_string_literal(&self) -> bool {
        self.flags.get().intersects(TypeFlags::StringLiteral)
    }

    pub fn is_number_literal(&self) -> bool {
        self.flags.get().intersects(TypeFlags::NumberLiteral)
    }

    pub fn is_big_int_literal(&self) -> bool {
        self.flags.get().intersects(TypeFlags::BigIntLiteral)
    }

    pub fn is_enum_literal(&self) -> bool {
        self.flags.get().intersects(TypeFlags::EnumLiteral)
    }

    pub fn is_boolean_like(&self) -> bool {
        self.flags.get().intersects(TypeFlags::BooleanLike)
    }

    pub fn is_string_like(&self) -> bool {
        self.flags.get().intersects(TypeFlags::StringLike)
    }

    pub fn is_class(&self) -> bool {
        self.object_flags.get().intersects(ObjectFlags::Class)
    }

    pub fn is_type_parameter(&self) -> bool {
        self.flags.get().intersects(TypeFlags::TypeParameter)
    }

    pub fn is_index(&self) -> bool {
        self.flags.get().intersects(TypeFlags::Index)
    }

    /// Go `t.IsTupleType()` (= `isTupleType(t)`).
    pub fn is_tuple_type(&self) -> bool {
        self.object_flags.get().intersects(ObjectFlags::Reference)
            && self.as_type_reference().target.get().is_some_and(|t| t.object_flags.get().intersects(ObjectFlags::Tuple))
    }
}

/// Methods Go defines on `*Type` that need the pointer itself.
pub trait TypeExt {
    /// Go `t.Distributed()`.
    fn distributed(self) -> Vec<P<Type>>;
}

impl TypeExt for P<Type> {
    fn distributed(self) -> Vec<P<Type>> {
        if self.flags.get().intersects(TypeFlags::Union) {
            return self.as_union_type().types.get().to_vec();
        }
        if self.flags.get().intersects(TypeFlags::Never) {
            return Vec::new();
        }
        vec![self]
    }
}

// TypeData

/// A borrowed view of the type's owned payload. Embedded base views preserve this borrow's lifetime.
///
/// A payload view cannot escape its record:
/// ```compile_fail
/// use tsrs_checker::{InterfaceType, TypeData};
/// let view = {
///     let record = InterfaceType::default();
///     TypeData::Interface(&record)
/// };
/// println!("{}", view.as_interface_type().is_some());
/// ```
#[derive(Clone, Copy)]
pub enum TypeData<'a> {
    Intrinsic(&'a IntrinsicType),
    Literal(&'a LiteralType),
    UniqueESSymbol(&'a UniqueESSymbolType),
    Object(&'a ObjectType), // anonymous (and instantiated anonymous) object types
    TypeReference(&'a TypeReference),
    Interface(&'a InterfaceType),
    Tuple(&'a TupleType),
    InstantiationExpression(&'a InstantiationExpressionType),
    Mapped(&'a MappedType),
    ReverseMapped(&'a ReverseMappedType),
    EvolvingArray(&'a EvolvingArrayType),
    Union(&'a UnionType),
    Intersection(&'a IntersectionType),
    TypeParameter(&'a TypeParameter),
    Index(&'a IndexType),
    IndexedAccess(&'a IndexedAccessType),
    TemplateLiteral(&'a TemplateLiteralType),
    StringMapping(&'a StringMappingType),
    Substitution(&'a SubstitutionType),
    Conditional(&'a ConditionalType),
}

impl<'a> TypeData<'a> {
    pub fn as_constrained_type(&self) -> Option<&'a ConstrainedType> {
        Some(match *self {
            TypeData::Intrinsic(_) | TypeData::Literal(_) | TypeData::UniqueESSymbol(_) => return None,
            // Structured types keep their base constraint in the checker (see `StructuredType`).
            TypeData::Object(_)
            | TypeData::TypeReference(_)
            | TypeData::Interface(_)
            | TypeData::Tuple(_)
            | TypeData::InstantiationExpression(_)
            | TypeData::Mapped(_)
            | TypeData::ReverseMapped(_)
            | TypeData::EvolvingArray(_)
            | TypeData::Union(_)
            | TypeData::Intersection(_) => return None,
            TypeData::TypeParameter(d) => d,
            TypeData::Index(d) => d,
            TypeData::IndexedAccess(d) => d,
            TypeData::TemplateLiteral(d) => d,
            TypeData::StringMapping(d) => d,
            TypeData::Substitution(d) => d,
            TypeData::Conditional(d) => d,
        })
    }

    pub fn as_structured_type(&self) -> Option<&'a StructuredType> {
        Some(match *self {
            TypeData::Object(d) => d,
            TypeData::TypeReference(d) => d,
            TypeData::Interface(d) => d,
            TypeData::Tuple(d) => d,
            TypeData::InstantiationExpression(d) => d,
            TypeData::Mapped(d) => d,
            TypeData::ReverseMapped(d) => d,
            TypeData::EvolvingArray(d) => d,
            TypeData::Union(d) => d,
            TypeData::Intersection(d) => d,
            _ => return None,
        })
    }

    pub fn as_object_type(&self) -> Option<&'a ObjectType> {
        Some(match *self {
            TypeData::Object(d) => d,
            TypeData::TypeReference(d) => d,
            TypeData::Interface(d) => d,
            TypeData::Tuple(d) => d,
            TypeData::InstantiationExpression(d) => d,
            TypeData::Mapped(d) => d,
            TypeData::ReverseMapped(d) => d,
            TypeData::EvolvingArray(d) => d,
            _ => return None,
        })
    }

    pub fn as_type_reference(&self) -> Option<&'a TypeReference> {
        Some(match *self {
            TypeData::TypeReference(d) => d,
            TypeData::Interface(d) => d,
            TypeData::Tuple(d) => d,
            _ => return None,
        })
    }

    pub fn as_interface_type(&self) -> Option<&'a InterfaceType> {
        Some(match *self {
            TypeData::Interface(d) => d,
            TypeData::Tuple(d) => d,
            _ => return None,
        })
    }

    pub fn as_union_or_intersection_type(&self) -> Option<&'a UnionOrIntersectionType> {
        Some(match *self {
            TypeData::Union(d) => d,
            TypeData::Intersection(d) => d,
            _ => return None,
        })
    }
}

macro_rules! embeds {
    ($t:ty, $field:ident, $parent:ty) => {
        impl std::ops::Deref for $t {
            type Target = $parent;
            #[inline]
            fn deref(&self) -> &$parent {
                &self.$field
            }
        }
    };
}

// IntrinsicTypeData

#[derive(Default)]
pub struct IntrinsicType {
    pub intrinsic_name: TextCell,
}

impl IntrinsicType {
    pub fn intrinsic_name(&self) -> TextView {
        self.intrinsic_name.get()
    }
}

// LiteralTypeData

/// Go `any` value of a literal type: `string | jsnum.Number | bool | PseudoBigInt`. Go's nil is `Option::None`
/// (`Option<LiteralValue>`), matching how the generated signatures map a nil-able `any`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum LiteralValue {
    String(TextView),
    Number(jsnum::Number),
    Boolean(bool),
    BigInt(jsnum::PseudoBigInt),
}

#[derive(Default)]
pub struct LiteralType {
    pub value: SnapshotCell<Option<LiteralValue>>, // string | jsnum.Number | bool | PseudoBigInt | nil (computed enum)
    pub fresh_type: Cell<Option<P<Type>>>, // Fresh version of type
    pub regular_type: Cell<Option<P<Type>>>, // Regular version of type
}

impl LiteralType {
    pub fn value(&self) -> Option<LiteralValue> {
        self.value.get()
    }
    pub fn fresh_type(&self) -> Option<P<Type>> {
        self.fresh_type.get()
    }
    pub fn regular_type(&self) -> Option<P<Type>> {
        self.regular_type.get()
    }
}

// UniqueESSymbolTypeData

#[derive(Default)]
pub struct UniqueESSymbolType {
    pub name: TextCell,
}

// ConstrainedType (type with computed base constraint)

#[derive(Default)]
pub struct ConstrainedType {
    pub resolved_base_constraint: Cell<Option<P<Type>>>,
}

// StructuredType (base of all types with members)

// Go's StructuredType embeds ConstrainedType; here the base constraint of a structured type (set for 4% of the 7.6M
// on the private monorepo) is kept in `Checker::structured_type_base_constraints` (`resolved_base_constraint_of`), and
// `try_as_constrained_type()` is None for structured types.
//
// The resolved members (Go's five fields) live in a `StructuredMembers` record allocated on the first write: most
// structured types are never resolved (on the private monorepo 4.9M of 7.6M: type references answered by lazy member tables,
// unions and intersections whose members nobody asks for), and those now carry one pointer instead of 48 bytes.
// Reads of an absent record return the zero values (nil members, empty slices, count 0), exactly like reading the
// unset fields; once allocated, every getter returns exactly what was last set. An empty index-info list reads as `&[]`.
#[derive(Default)]
pub struct StructuredType {
    resolved: OnceCell<Box<StructuredMembers>>,
    // Go's objectTypeWithoutAbstractConstructSignatures is `Checker::object_types_without_abstract_construct_signatures`.
}

#[derive(Default)]
struct StructuredMembers {
    members: Cell<Option<P<SymbolTable>>>,
    // Each owned array field is a nullable Arc<Vec<T>> pointer.
    properties: ArrayCell<P<Symbol>>,
    signatures: ArrayCell<SignatureKey>, // Signatures (call + construct)
    // Count of call signatures, and index infos (2% of the resolved types on the private monorepo have any).
    count_or_index_infos: CountOrIndexInfos,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<StructuredType>() == 8);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<StructuredType>() == 4);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<StructuredMembers>() == 40);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<StructuredMembers>() == 20);

/// Call signature count and optional index-info slice. Both fields are normal Rust values.
#[derive(Default)]
struct CountOrIndexInfos {
    call_signature_count: Cell<i32>,
    index_infos: ArrayCell<IndexInfoKey>,
}

impl CountOrIndexInfos {
    #[inline]
    fn call_signature_count(&self) -> i32 { self.call_signature_count.get() }
    #[inline]
    fn set_call_signature_count(&self, count: i32) { self.call_signature_count.set(count); }
    #[inline]
    fn index_infos(&self) -> ArrayView<IndexInfoKey> { self.index_infos.get() }
    fn set_index_infos(&self, index_infos: &[IndexInfoKey]) { self.index_infos.set(index_infos); }
}

impl StructuredType {
    #[inline]
    fn resolved_for_write(&self) -> &StructuredMembers {
        self.resolved.get_or_init(|| owned_type_payload(StructuredMembers::default()))
    }

    #[inline]
    pub fn members(&self) -> Option<P<SymbolTable>> {
        self.resolved.get().and_then(|r| r.members.get())
    }
    #[inline]
    pub fn set_members(&self, members: Option<P<SymbolTable>>) {
        self.resolved_for_write().members.set(members);
    }
    #[inline]
    pub fn properties(&self) -> ArrayView<P<Symbol>> {
        self.resolved.get().map_or_else(ArrayView::default, |r| r.properties.get())
    }
    #[inline]
    pub fn set_properties(&self, properties: &[P<Symbol>]) {
        self.resolved_for_write().properties.set(properties);
    }
    /// Call signatures followed by construct signatures.
    #[inline]
    pub fn signatures(&self) -> ArrayView<SignatureKey> {
        self.resolved.get().map_or_else(ArrayView::default, |r| r.signatures.get())
    }
    #[inline]
    pub fn set_signatures(&self, signatures: &[SignatureKey]) {
        self.resolved_for_write().signatures.set(signatures);
    }
    #[inline]
    pub fn call_signature_count(&self) -> i32 {
        self.resolved.get().map_or(0, |r| r.count_or_index_infos.call_signature_count())
    }
    #[inline]
    pub fn set_call_signature_count(&self, count: i32) {
        self.resolved_for_write().count_or_index_infos.set_call_signature_count(count);
    }
    #[inline]
    pub fn index_infos(&self) -> ArrayView<IndexInfoKey> {
        self.resolved.get().map_or_else(ArrayView::default, |r| r.count_or_index_infos.index_infos())
    }
    #[inline]
    pub fn set_index_infos(&self, index_infos: &[IndexInfoKey]) {
        self.resolved_for_write().count_or_index_infos.set_index_infos(index_infos);
    }
    pub fn call_signatures(&self) -> ArrayView<SignatureKey> {
        self.signatures().slice(..self.call_signature_count() as usize)
    }
    pub fn construct_signatures(&self) -> ArrayView<SignatureKey> {
        self.signatures().slice(self.call_signature_count() as usize..)
    }
}

// Except for tuple type references and reverse mapped types, all object types have an associated symbol.
// Possible object type instances are listed in the following.

// InterfaceType:
// ObjectFlagsClass: Originating non-generic class type
// ObjectFlagsClass|ObjectFlagsReference: Originating generic class type
// ObjectFlagsInterface: Originating non-generic interface type
// ObjectFlagsInterface|ObjectFlagsReference: Originating generic interface type

// TupleType:
// ObjectFlagsReference|ObjectFlagsTuple: Originating generic tuple type (synthesized)

// TypeReference
// ObjectFlagsReference: Instantiated generic class, interface, or tuple type

// ObjectType:
// ObjectFlagsAnonymous: Originating anonymous object type
// ObjectFlagsAnonymous|ObjectFlagsInstantiated: Instantiated anonymous object type

// MappedType:
// ObjectFlagsMapped: Originating mapped type
// ObjectFlagsMapped|ObjectFlagsInstantiated: Instantiated mapped type

// InstantiationExpressionType:
// ObjectFlagsAnonymous|ObjectFlagsInstantiationExpression: Originating instantiation expression type
// ObjectFlagsAnonymous|ObjectFlagsInstantiated|ObjectFlagsInstantiationExpression: Instantiated instantiation expression type

// ReverseMappedType:
// ObjectFlagsAnonymous|ObjectFlagsReverseMapped: Reverse mapped type

// EvolvingArrayType:
// ObjectFlagsEvolvingArray: Evolving array type

#[derive(Default)]
pub struct ObjectType {
    pub structured_type: StructuredType,
    pub target: Cell<Option<P<Type>>>, // Target of instantiated type
    pub mapper: MapperCell, // Type mapper for instantiated type
    // Go's `instantiations` map is used only by the targets of instantiations: generic interfaces and tuples keep
    // it in `InterfaceType`, other object types (declared anonymous and mapped types, deferred type references) in
    // `Checker::object_type_instantiations`, so the millions of instantiated object types and references do not
    // carry it.
}
embeds!(ObjectType, structured_type, StructuredType);

// TypeReference (instantiation of an InterfaceType)

#[derive(Default)]
pub struct TypeReference {
    pub object_type: ObjectType,
    pub node: Cell<Option<P<Node>>>, // TypeReferenceNode | ArrayTypeNode | TupleTypeNode when deferred, else nil
    pub resolved_type_arguments: OptionArrayCell<P<Type>>, // nil = not computed (Go tests against nil)
}
embeds!(TypeReference, object_type, ObjectType);

// InterfaceType (when generic, serves as reference to instantiation of itself)

/// Go's `instantiations` map of a generic class, interface or tuple target (`map[CacheHashKey]*Type`, keyed by
/// `getTypeListKey(typeArguments)`). Every value is a non-deferred reference whose `resolved_type_arguments` are
/// exactly the list its key was made from (the target itself for its type parameters), and they never change, so
/// the table stores only the references and compares type-argument lists: one word per slot instead of the 128-bit
/// key plus the value, and a lookup hashes the type ids instead of xxh3 over the key bytes. Exact list equality maps
/// lists to references like Go's collision-free 128-bit key does. Nil until `make()`, like the Go map.
#[derive(Default)]
pub struct ReferenceInstantiations(OnceCell<Box<RefCell<hashbrown::HashTable<P<Type>>>>>);

impl ReferenceInstantiations {
    /// Heap census: the table's slots (4 bytes each).
    #[cfg(feature = "assignment-stats")]
    pub(crate) fn heap_stat(&self) -> Option<crate::heapcensus::HeapStat> {
        let cell = self.0.get()?;
        let table = cell.borrow();
        Some(crate::heapcensus::HeapStat::table(table.len(), table.capacity(), std::mem::size_of::<P<Type>>()))
    }

    fn hash(type_arguments: &[P<Type>]) -> u64 {
        use std::hash::Hasher;
        let mut h = rustc_hash::FxHasher::default();
        h.write_usize(type_arguments.len());
        for t in type_arguments {
            h.write_u32(t.id.0);
        }
        h.finish()
    }

    fn arguments_of(reference: P<Type>) -> ArrayView<P<Type>> {
        reference.as_type_reference().resolved_type_arguments.get().unwrap()
    }

    /// Go `m = make(map[CacheHashKey]*Type)`.
    pub fn make(&self) {
        if let Some(table) = self.0.get() {
            *table.borrow_mut() = hashbrown::HashTable::new();
        } else {
            self.0.get_or_init(|| owned_type_payload(RefCell::new(hashbrown::HashTable::new())));
        }
    }

    /// Go `m[getTypeListKey(typeArguments)]`.
    pub fn get(&self, type_arguments: &[P<Type>]) -> Option<P<Type>> {
        let cell = self.0.get()?;
        let table = cell.borrow();
        table.find(Self::hash(type_arguments), |&t| Self::arguments_of(t) == type_arguments).copied()
    }

    /// Go `m[getTypeListKey(reference's type arguments)] = reference` for a key that is not present yet.
    pub fn add(&self, reference: P<Type>) {
        if self.0.get().is_none() {
            self.make();
        }
        let cell = self.0.get().unwrap();
        let mut table = cell.borrow_mut();
        let arguments = Self::arguments_of(reference);
        debug_assert!(table.find(Self::hash(&arguments), |&t| Self::arguments_of(t) == arguments).is_none());
        table.insert_unique(Self::hash(&arguments), reference, |&t| Self::hash(&Self::arguments_of(t)));
    }
}

#[derive(Default)]
pub struct InterfaceType {
    pub type_reference: TypeReference,
    pub instantiations: ReferenceInstantiations, // Map of type instantiations (Go: in ObjectType)
    pub all_type_parameters: ArrayCell<P<Type>>, // Type parameters (outer + local + thisType)
    pub outer_type_parameter_count: Cell<i32>, // Count of outer type parameters
    pub this_type: Cell<Option<P<Type>>>, // The "this" type (nil if none)
    pub base_types_resolved: Cell<bool>,
    pub declared_members_resolved: Cell<bool>,
    pub resolved_base_constructor_type: Cell<Option<P<Type>>>,
    pub resolved_base_types: ArrayCell<P<Type>>,
    pub declared_members: Cell<Option<P<SymbolTable>>>, // Declared members
    pub declared_call_signatures: ArrayCell<SignatureKey>, // Declared call signatures
    pub declared_construct_signatures: ArrayCell<SignatureKey>, // Declared construct signatures
    pub declared_index_infos: ArrayCell<IndexInfoKey>, // Declared index signatures
}
embeds!(InterfaceType, type_reference, TypeReference);

impl InterfaceType {
    pub fn outer_type_parameters(&self) -> ArrayView<P<Type>> {
        let all = self.all_type_parameters.get();
        if all.is_empty() {
            return ArrayView::default();
        }
        all.slice(..self.outer_type_parameter_count.get() as usize)
    }

    pub fn local_type_parameters(&self) -> ArrayView<P<Type>> {
        let all = self.all_type_parameters.get();
        if all.is_empty() {
            return ArrayView::default();
        }
        all.slice(self.outer_type_parameter_count.get() as usize..all.len() - 1)
    }

    pub fn type_parameters(&self) -> ArrayView<P<Type>> {
        let all = self.all_type_parameters.get();
        if all.is_empty() {
            return ArrayView::default();
        }
        all.slice(..all.len() - 1)
    }

    pub fn this_type(&self) -> Option<P<Type>> {
        self.this_type.get()
    }
}

// TupleType

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct ElementFlags: u32 {
        const None = 0;
        const Required = 1 << 0; // T
        const Optional = 1 << 1; // T?
        const Rest = 1 << 2; // ...T[]
        const Variadic = 1 << 3; // ...T
        const Fixed = Self::Required.bits() | Self::Optional.bits();
        const Variable = Self::Rest.bits() | Self::Variadic.bits();
        const NonRequired = Self::Optional.bits() | Self::Rest.bits() | Self::Variadic.bits();
        const NonRest = Self::Required.bits() | Self::Optional.bits() | Self::Variadic.bits();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TupleElementInfo {
    pub flags: ElementFlags,
    pub labeled_declaration: Option<P<Node>>, // NamedTupleMember | ParameterDeclaration | nil
}

impl TupleElementInfo {
    pub fn tuple_element_flags(self) -> ElementFlags {
        self.flags
    }
    pub fn labeled_declaration(self) -> Option<P<Node>> {
        self.labeled_declaration
    }
}

#[derive(Default)]
pub struct TupleType {
    pub interface_type: InterfaceType,
    pub element_infos: ArrayCell<TupleElementInfo>,
    pub min_length: Cell<i32>, // Number of required or variadic elements
    pub fixed_length: Cell<i32>, // Number of initial required or optional elements
    pub combined_flags: Cell<ElementFlags>,
    pub readonly: Cell<bool>,
}
embeds!(TupleType, interface_type, InterfaceType);

impl TupleType {
    pub fn fixed_length(&self) -> i32 {
        self.fixed_length.get()
    }
    pub fn is_readonly(&self) -> bool {
        self.readonly.get()
    }
    pub fn element_flags(&self) -> Vec<ElementFlags> {
        self.element_infos.get().iter().map(|info| info.flags).collect()
    }
    pub fn element_infos(&self) -> ArrayView<TupleElementInfo> {
        self.element_infos.get()
    }
}

// InstantiationExpressionType

#[derive(Default)]
pub struct InstantiationExpressionType {
    pub object_type: ObjectType,
    pub node: Cell<Option<P<Node>>>,
}
embeds!(InstantiationExpressionType, object_type, ObjectType);

// MappedType

#[derive(Default)]
pub struct MappedType {
    pub object_type: ObjectType,
    pub declaration: Cell<Option<P<Node>>>, // MappedTypeNode
    pub type_parameter: Cell<Option<P<Type>>>,
    pub constraint_type: Cell<Option<P<Type>>>,
    pub name_type: Cell<Option<P<Type>>>,
    pub template_type: Cell<Option<P<Type>>>,
    pub modifiers_type: Cell<Option<P<Type>>>,
    pub resolved_apparent_type: Cell<Option<P<Type>>>,
    pub contains_error: Cell<bool>,
}
embeds!(MappedType, object_type, ObjectType);

impl MappedType {
    pub fn type_parameter(&self) -> Option<P<Type>> {
        self.type_parameter.get()
    }
    pub fn constraint_type(&self) -> Option<P<Type>> {
        self.constraint_type.get()
    }
    pub fn name_type(&self) -> Option<P<Type>> {
        self.name_type.get()
    }
    pub fn template_type(&self) -> Option<P<Type>> {
        self.template_type.get()
    }
    pub fn resolve_components(&self, c: &mut Checker, typ: P<Type>) {
        c.get_type_parameter_from_mapped_type(typ);
        c.get_constraint_type_from_mapped_type(typ);
        c.get_name_type_from_mapped_type(typ);
        c.get_template_type_from_mapped_type(typ);
    }
}

// ReverseMappedType

#[derive(Default)]
pub struct ReverseMappedType {
    pub object_type: ObjectType,
    pub source: Cell<Option<P<Type>>>,
    pub mapped_type: Cell<Option<P<Type>>>,
    pub constraint_type: Cell<Option<P<Type>>>,
}
embeds!(ReverseMappedType, object_type, ObjectType);

// EvolvingArrayType

#[derive(Default)]
pub struct EvolvingArrayType {
    pub object_type: ObjectType,
    pub element_type: Cell<Option<P<Type>>>,
    pub final_array_type: Cell<Option<P<Type>>>,
}
embeds!(EvolvingArrayType, object_type, ObjectType);

// UnionOrIntersectionTypeData

/// Go's `UnionOrIntersectionType` / `UnionType` / `IntersectionType` fields other than `types` are set on few types
/// (on the private monorepo single: of 1.07M unions 12% get an origin, 9.5% a key property name, ~6% each a
/// property cache, resolved properties, a reduced or a regular type; of 1.17M intersections 21% a non-augmented
/// property cache, 14% an apparent type, 4.4% resolved properties), so they live in a tail allocated on the first
/// non-nil write: a `UnionRare` or an `IntersectionRare`, both starting with the shared `UnionOrIntersectionRare`.
/// Reads of an absent tail return the zero value. A Rust enum keeps the kind and owns its lazily initialized box.
#[derive(Default)]
pub struct UnionOrIntersectionType {
    pub structured_type: StructuredType,
    pub types: ArrayCell<P<Type>>,
    rare: UnionOrIntersectionRareState,
}
embeds!(UnionOrIntersectionType, structured_type, StructuredType);

#[derive(Default)]
#[repr(C)]
struct UnionOrIntersectionRare {
    resolved_properties: OptionArrayCell<P<Symbol>>, // nil = not computed (Go tests against nil)
    property_cache: Cell<Option<P<SymbolTable>>>,
    property_cache_without_function_property_augment: Cell<Option<P<SymbolTable>>>,
}

#[derive(Default)]
#[repr(C)]
struct UnionRare {
    shared: UnionOrIntersectionRare,
    resolved_reduced_type: Cell<Option<P<Type>>>,
    regular_type: Cell<Option<P<Type>>>,
    origin: Cell<Option<P<Type>>>, // Denormalized union, intersection, or index type in which union originates
    key_property_name: TextCell,    // Property with unique unit type that exists in every object/intersection in union type
    constituent_map: OwnedMap<P<Type>, P<Type>>, // Constituents keyed by unit type discriminants
}

#[derive(Default)]
#[repr(C)]
struct IntersectionRare {
    shared: UnionOrIntersectionRare,
    resolved_apparent_type: Cell<Option<P<Type>>>,
    unique_literal_filled_instantiation: Cell<Option<P<Type>>>, // Instantiation with type parameters mapped to never type
}

#[repr(C, u8)]
enum UnionOrIntersectionRareState {
    Union(OnceCell<Box<UnionRare>>),
    Intersection(OnceCell<Box<IntersectionRare>>),
}

impl Default for UnionOrIntersectionRareState {
    fn default() -> Self { Self::Union(OnceCell::new()) }
}

impl UnionOrIntersectionType {
    pub fn types(&self) -> ArrayView<P<Type>> {
        self.types.get()
    }
    #[inline]
    fn rare(&self) -> Option<&UnionOrIntersectionRare> {
        match &self.rare {
            UnionOrIntersectionRareState::Union(tail) => tail.get().map(|tail| &tail.shared),
            UnionOrIntersectionRareState::Intersection(tail) => tail.get().map(|tail| &tail.shared),
        }
    }
    fn rare_for_write(&self) -> &UnionOrIntersectionRare {
        match &self.rare {
            UnionOrIntersectionRareState::Union(tail) => &tail.get_or_init(|| owned_type_payload(UnionRare::default())).shared,
            UnionOrIntersectionRareState::Intersection(tail) => &tail.get_or_init(|| owned_type_payload(IntersectionRare::default())).shared,
        }
    }
    pub fn resolved_properties(&self) -> Option<ArrayView<P<Symbol>>> {
        self.rare().and_then(|r| r.resolved_properties.get())
    }
    pub fn set_resolved_properties(&self, properties: Option<&[P<Symbol>]>) {
        if properties.is_some() || self.rare().is_some() {
            self.rare_for_write().resolved_properties.set(properties);
        }
    }
    /// Go `t.propertyCacheWithoutObjectFunctionPropertyAugment` when `skip_object_function_property_augment`, else
    /// `t.propertyCache`.
    #[inline]
    pub fn property_cache(&self, skip_object_function_property_augment: bool) -> Option<P<SymbolTable>> {
        let r = self.rare()?;
        if skip_object_function_property_augment { r.property_cache_without_function_property_augment.get() } else { r.property_cache.get() }
    }
    /// `property_cache`, created empty if it is nil (Go `getSymbolTable(&cache)`).
    pub fn property_cache_for_write(&self, skip_object_function_property_augment: bool) -> P<SymbolTable> {
        let r = self.rare_for_write();
        let cell = if skip_object_function_property_augment { &r.property_cache_without_function_property_augment } else { &r.property_cache };
        ast::get_symbol_table(cell)
    }
}

// UnionType

#[derive(Default)]
pub struct UnionType {
    pub union_or_intersection_type: UnionOrIntersectionType,
}
embeds!(UnionType, union_or_intersection_type, UnionOrIntersectionType);

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<UnionType>() == 32);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<UnionType>() == 16);

impl UnionType {
    #[inline]
    fn union_rare(&self) -> Option<&UnionRare> {
        match &self.union_or_intersection_type.rare {
            UnionOrIntersectionRareState::Union(tail) => tail.get().map(|tail| &**tail),
            UnionOrIntersectionRareState::Intersection(_) => panic!("union payload has intersection state"),
        }
    }
    fn union_rare_for_write(&self) -> &UnionRare {
        match &self.union_or_intersection_type.rare {
            UnionOrIntersectionRareState::Union(tail) => tail.get_or_init(|| owned_type_payload(UnionRare::default())),
            UnionOrIntersectionRareState::Intersection(_) => panic!("union payload has intersection state"),
        }
    }
    #[inline]
    pub fn resolved_reduced_type(&self) -> Option<P<Type>> {
        self.union_rare().and_then(|r| r.resolved_reduced_type.get())
    }
    pub fn set_resolved_reduced_type(&self, t: Option<P<Type>>) {
        if t.is_some() || self.union_rare().is_some() {
            self.union_rare_for_write().resolved_reduced_type.set(t);
        }
    }
    #[inline]
    pub fn regular_type(&self) -> Option<P<Type>> {
        self.union_rare().and_then(|r| r.regular_type.get())
    }
    pub fn set_regular_type(&self, t: Option<P<Type>>) {
        if t.is_some() || self.union_rare().is_some() {
            self.union_rare_for_write().regular_type.set(t);
        }
    }
    #[inline]
    pub fn origin(&self) -> Option<P<Type>> {
        self.union_rare().and_then(|r| r.origin.get())
    }
    pub fn set_origin(&self, t: Option<P<Type>>) {
        if t.is_some() || self.union_rare().is_some() {
            self.union_rare_for_write().origin.set(t);
        }
    }
    pub fn key_property_name(&self) -> TextView {
        self.union_rare().map_or_else(TextView::default, |r| r.key_property_name.get())
    }
    pub fn set_key_property_name(&self, name: &str) {
        if !name.is_empty() || self.union_rare().is_some() {
            self.union_rare_for_write().key_property_name.set(name);
        }
    }
    /// Go `t.constituentMap` for reading (nil while there is no tail).
    pub fn constituent_map(&self) -> Option<&OwnedMap<P<Type>, P<Type>>> {
        self.union_rare().map(|r| &r.constituent_map)
    }
    /// Go `t.constituentMap` for writing.
    pub fn constituent_map_for_write(&self) -> &OwnedMap<P<Type>, P<Type>> {
        &self.union_rare_for_write().constituent_map
    }
}

// IntersectionType

pub struct IntersectionType {
    pub union_or_intersection_type: UnionOrIntersectionType,
}
embeds!(IntersectionType, union_or_intersection_type, UnionOrIntersectionType);

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<IntersectionType>() == 32);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<IntersectionType>() == 16);

impl Default for IntersectionType {
    fn default() -> Self {
        IntersectionType { union_or_intersection_type: UnionOrIntersectionType { rare: UnionOrIntersectionRareState::Intersection(OnceCell::new()), ..Default::default() } }
    }
}

impl IntersectionType {
    #[inline]
    fn intersection_rare(&self) -> Option<&IntersectionRare> {
        match &self.union_or_intersection_type.rare {
            UnionOrIntersectionRareState::Intersection(tail) => tail.get().map(|tail| &**tail),
            UnionOrIntersectionRareState::Union(_) => panic!("intersection payload has union state"),
        }
    }
    fn intersection_rare_for_write(&self) -> &IntersectionRare {
        match &self.union_or_intersection_type.rare {
            UnionOrIntersectionRareState::Intersection(tail) => tail.get_or_init(|| owned_type_payload(IntersectionRare::default())),
            UnionOrIntersectionRareState::Union(_) => panic!("intersection payload has union state"),
        }
    }
    #[inline]
    pub fn resolved_apparent_type(&self) -> Option<P<Type>> {
        self.intersection_rare().and_then(|r| r.resolved_apparent_type.get())
    }
    pub fn set_resolved_apparent_type(&self, t: Option<P<Type>>) {
        if t.is_some() || self.intersection_rare().is_some() {
            self.intersection_rare_for_write().resolved_apparent_type.set(t);
        }
    }
    pub fn unique_literal_filled_instantiation(&self) -> Option<P<Type>> {
        self.intersection_rare().and_then(|r| r.unique_literal_filled_instantiation.get())
    }
    pub fn set_unique_literal_filled_instantiation(&self, t: Option<P<Type>>) {
        if t.is_some() || self.intersection_rare().is_some() {
            self.intersection_rare_for_write().unique_literal_filled_instantiation.set(t);
        }
    }
}

// TypeParameter

#[derive(Default)]
pub struct TypeParameter {
    pub constrained_type: ConstrainedType,
    pub constraint: Cell<Option<P<Type>>>,
    pub target: Cell<Option<P<Type>>>,
    pub mapper: MapperCell,
    pub is_this_type: Cell<bool>,
    pub is_distributed: Cell<bool>,
    pub resolved_default_type: Cell<Option<P<Type>>>,
    pub distributed_type: Cell<Option<P<Type>>>,
}
embeds!(TypeParameter, constrained_type, ConstrainedType);

impl TypeParameter {
    pub fn is_this_type(&self) -> bool {
        self.is_this_type.get()
    }
}

// IndexFlags

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct IndexFlags: u32 {
        const None = 0;
        const StringsOnly = 1 << 0;
        const NoIndexSignatures = 1 << 1;
        const NoReducibleCheck = 1 << 2;
    }
}

// IndexType

#[derive(Default)]
pub struct IndexType {
    pub constrained_type: ConstrainedType,
    pub target: Cell<Option<P<Type>>>,
    pub index_flags: Cell<IndexFlags>,
}
embeds!(IndexType, constrained_type, ConstrainedType);

impl IndexType {
    pub fn target(&self) -> Option<P<Type>> {
        self.target.get()
    }
}

// IndexedAccessType

#[derive(Default)]
pub struct IndexedAccessType {
    pub constrained_type: ConstrainedType,
    pub object_type: Cell<Option<P<Type>>>,
    pub index_type: Cell<Option<P<Type>>>,
    pub access_flags: Cell<AccessFlags>, // Only includes AccessFlags.Persistent
}
embeds!(IndexedAccessType, constrained_type, ConstrainedType);

impl IndexedAccessType {
    pub fn object_type(&self) -> Option<P<Type>> {
        self.object_type.get()
    }
    pub fn index_type(&self) -> Option<P<Type>> {
        self.index_type.get()
    }
}

#[derive(Default)]
pub struct TemplateLiteralType {
    pub constrained_type: ConstrainedType,
    pub texts: ArrayCell<TextView>, // Always one element longer than types
    pub types: ArrayCell<P<Type>>, // Always at least one element
}
embeds!(TemplateLiteralType, constrained_type, ConstrainedType);

impl TemplateLiteralType {
    pub fn texts(&self) -> ArrayView<TextView> {
        self.texts.get()
    }
    pub fn types(&self) -> ArrayView<P<Type>> {
        self.types.get()
    }
}

#[derive(Default)]
pub struct StringMappingType {
    pub constrained_type: ConstrainedType,
    pub target: Cell<Option<P<Type>>>,
}
embeds!(StringMappingType, constrained_type, ConstrainedType);

impl StringMappingType {
    pub fn target(&self) -> Option<P<Type>> {
        self.target.get()
    }
}

#[derive(Default)]
pub struct SubstitutionType {
    pub constrained_type: ConstrainedType,
    pub base_type: Cell<Option<P<Type>>>, // Target type
    pub constraint: Cell<Option<P<Type>>>, // Constraint that target type is known to satisfy
}
embeds!(SubstitutionType, constrained_type, ConstrainedType);

impl SubstitutionType {
    pub fn base_type(&self) -> Option<P<Type>> {
        self.base_type.get()
    }
    pub fn subst_constraint(&self) -> Option<P<Type>> {
        self.constraint.get()
    }
}

#[derive(Default)]
pub struct ConditionalRoot {
    pub node: Cell<Option<P<Node>>>, // ConditionalTypeNode
    pub check_type: Cell<Option<P<Type>>>,
    pub extends_type: Cell<Option<P<Type>>>,
    pub is_distributive: Cell<bool>,
    pub infer_type_parameters: ArrayCell<P<Type>>,
    pub outer_type_parameters: ArrayCell<P<Type>>,
    pub instantiations: OwnedPackedMap<CacheHashKey, P<Type>>,
    pub alias: Cell<Option<TypeAliasKey>>,
}

#[derive(Default)]
pub struct ConditionalType {
    pub constrained_type: ConstrainedType,
    pub root: Cell<Option<P<ConditionalRoot>>>,
    pub check_type: Cell<Option<P<Type>>>,
    pub extends_type: Cell<Option<P<Type>>>,
    pub resolved_true_type: Cell<Option<P<Type>>>,
    pub resolved_false_type: Cell<Option<P<Type>>>,
    pub resolved_inferred_true_type: Cell<Option<P<Type>>>, // The `trueType` instantiated with the `combinedMapper`, if present
    pub resolved_default_constraint: Cell<Option<P<Type>>>,
    pub resolved_constraint_of_distributive: Cell<Option<P<Type>>>,
    pub mapper: MapperCell,
    pub combined_mapper: MapperCell,
}
embeds!(ConditionalType, constrained_type, ConstrainedType);

impl ConditionalType {
    pub fn check_type(&self) -> Option<P<Type>> {
        self.check_type.get()
    }
    pub fn extends_type(&self) -> Option<P<Type>> {
        self.extends_type.get()
    }
}

// SignatureFlags

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct SignatureFlags: u32 {
        const None = 0;
        // Propagating flags
        const HasRestParameter = 1 << 0; // Indicates last parameter is rest parameter
        const HasLiteralTypes = 1 << 1; // Indicates signature is specialized
        const Construct = 1 << 2; // Indicates signature is a construct signature
        const Abstract = 1 << 3; // Indicates signature comes from an abstract class, abstract construct signature, or abstract constructor type
        // Non-propagating flags
        const IsInnerCallChain = 1 << 4; // Indicates signature comes from a CallChain nested in an outer OptionalChain
        const IsOuterCallChain = 1 << 5; // Indicates signature comes from a CallChain that is the outermost chain of an optional expression
        const IsUntypedSignatureInJSFile = 1 << 6; // Indicates signature is from a js file and has no types
        const IsNonInferrable = 1 << 7; // Indicates signature comes from a non-inferrable type
        const IsSignatureCandidateForOverloadFailure = 1 << 8;
        // We do not propagate `IsInnerCallChain` or `IsOuterCallChain` to instantiated signatures, as that would result in us
        // attempting to add `| undefined` on each recursive call to `getReturnTypeOfSignature` when
        // instantiating the return type.
        const PropagatingFlags = Self::HasRestParameter.bits() | Self::HasLiteralTypes.bits() | Self::Construct.bits() | Self::Abstract.bits() | Self::IsUntypedSignatureInJSFile.bits() | Self::IsSignatureCandidateForOverloadFailure.bits();
        const CallChainFlags = Self::IsInnerCallChain.bits() | Self::IsOuterCallChain.bits();
    }
}

// Signature

/// Qualified signature edge. Copying it does not retain the checker or access the record.
pub type SignatureKey = tsrs_core::arena_owner::ArenaKey<Signature>;
/// Shared composite signature metadata, resolved through its owning checker.
pub type CompositeSignatureKey = tsrs_core::arena_owner::ArenaKey<CompositeSignature>;

#[derive(Default)]
pub struct Signature {
    pub id: Cell<SignatureId>,
    pub flags: Cell<SignatureFlags>,
    pub min_argument_count: Cell<i32>,
    pub resolved_min_argument_count: Cell<i32>,
    pub declaration: Cell<Option<P<Node>>>,
    pub type_parameters: ArrayCell<P<Type>>,
    pub parameters: ArrayCell<P<Symbol>>,
    pub resolved_return_type: Cell<Option<P<Type>>>,
    pub target: Cell<Option<SignatureKey>>,
    pub mapper: MapperCell,
    // `thisParameter`, `isolatedSignatureType`, `composite` and a resolved type predicate other than the checker's
    // `noTypePredicate` (few signatures have any) live in a tail allocated on the first non-nil write;
    // `this_parameter()` / `set_this_parameter()` & co. read nil when it is absent. `resolvedTypePredicate ==
    // c.noTypePredicate` is recorded separately as a boolean.
    rare: OnceCell<Box<SignatureRare>>,
    no_type_predicate: Cell<bool>,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<Signature>() == 80);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<Signature>() == 56);

#[derive(Default)]
struct SignatureRare {
    this_parameter: Cell<Option<P<Symbol>>>,
    isolated_signature_type: Cell<Option<P<Type>>>,
    composite: Cell<Option<CompositeSignatureKey>>,
    resolved_type_predicate: Cell<Option<TypePredicateKey>>, // never the checker's `noTypePredicate` sentinel
}

impl Signature {
    pub fn id(&self) -> SignatureId {
        self.id.get()
    }
    pub fn flags(&self) -> SignatureFlags {
        self.flags.get()
    }
    pub fn type_parameters(&self) -> ArrayView<P<Type>> {
        self.type_parameters.get()
    }
    pub fn declaration(&self) -> Option<P<Node>> {
        self.declaration.get()
    }
    pub fn target(&self) -> Option<SignatureKey> {
        self.target.get()
    }
    fn rare_for_write(&self) -> &SignatureRare {
        self.rare.get_or_init(|| owned_type_payload(SignatureRare::default()))
    }
    /// Go `sig.resolvedTypePredicate`; `no_type_predicate` is the checker's `noTypePredicate`.
    #[inline]
    pub fn resolved_type_predicate(&self, no_type_predicate: TypePredicateKey) -> Option<TypePredicateKey> {
        if self.no_type_predicate.get() {
            return Some(no_type_predicate);
        }
        self.rare.get().and_then(|r| r.resolved_type_predicate.get())
    }
    /// Go `sig.resolvedTypePredicate = predicate`; `no_type_predicate` is the checker's `noTypePredicate`.
    pub fn set_resolved_type_predicate(&self, predicate: Option<TypePredicateKey>, no_type_predicate: TypePredicateKey) {
        let none = predicate == Some(no_type_predicate);
        self.no_type_predicate.set(none);
        let stored = if none { None } else { predicate };
        if stored.is_some() || self.rare.get().is_some() {
            self.rare_for_write().resolved_type_predicate.set(stored);
        }
    }
    pub fn this_parameter(&self) -> Option<P<Symbol>> {
        self.rare.get().and_then(|r| r.this_parameter.get())
    }
    pub fn set_this_parameter(&self, this_parameter: Option<P<Symbol>>) {
        if this_parameter.is_some() || self.rare.get().is_some() {
            self.rare_for_write().this_parameter.set(this_parameter);
        }
    }
    pub fn isolated_signature_type(&self) -> Option<P<Type>> {
        self.rare.get().and_then(|r| r.isolated_signature_type.get())
    }
    pub fn set_isolated_signature_type(&self, t: Option<P<Type>>) {
        if t.is_some() || self.rare.get().is_some() {
            self.rare_for_write().isolated_signature_type.set(t);
        }
    }
    pub fn composite(&self) -> Option<CompositeSignatureKey> {
        self.rare.get().and_then(|r| r.composite.get())
    }
    pub fn set_composite(&self, composite: Option<CompositeSignatureKey>) {
        if composite.is_some() || self.rare.get().is_some() {
            self.rare_for_write().composite.set(composite);
        }
    }
    pub fn parameters(&self) -> ArrayView<P<Symbol>> {
        self.parameters.get()
    }
    pub fn has_rest_parameter(&self) -> bool {
        self.flags.get().intersects(SignatureFlags::HasRestParameter)
    }
    pub fn min_argument_count(&self) -> i32 {
        self.min_argument_count.get()
    }
}

#[derive(Default)]
pub struct CompositeSignature {
    pub is_union: Cell<bool>, // True for union, false for intersection
    pub signatures: ArrayCell<SignatureKey>, // Individual signatures
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum TypePredicateKind {
    #[default]
    This,
    Identifier,
    AssertsThis,
    AssertsIdentifier,
}

/// A predicate edge is a qualified key. It has no implicit pointer dereference.
pub type TypePredicateKey = tsrs_core::arena_owner::ArenaKey<TypePredicate>;

/// Immutable predicate metadata in the owning checker's typed store. Its type edge remains legacy.
#[derive(Default)]
pub struct TypePredicate {
    kind: TypePredicateKind,
    parameter_index: i32,
    parameter_name: TextView,
    t: Option<P<Type>>,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<TypePredicate>() == 24);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<TypePredicate>() == 16);

impl TypePredicate {
    pub(crate) fn new(kind: TypePredicateKind, parameter_name: &str, parameter_index: i32, t: Option<P<Type>>) -> Self {
        Self { kind, parameter_index, parameter_name: parameter_name.into(), t }
    }
    pub fn type_(&self) -> Option<P<Type>> { self.t }
    pub fn kind(&self) -> TypePredicateKind { self.kind }
    pub fn parameter_index(&self) -> i32 { self.parameter_index }
    pub fn parameter_name(&self) -> TextView { self.parameter_name.clone() }
}

// IndexInfo

/// An index-signature metadata edge, resolved by its owning checker.
pub type IndexInfoKey = tsrs_core::arena_owner::ArenaKey<IndexInfo>;

#[derive(Default)]
pub struct IndexInfo {
    pub key_type: Cell<Option<P<Type>>>,
    pub value_type: Cell<Option<P<Type>>>,
    pub is_readonly: Cell<bool>,
    pub declaration: Cell<Option<P<Node>>>, // IndexSignatureDeclaration
    pub index_symbol: Cell<Option<P<Symbol>>>, // Synthetic property symbol for this index signature
    pub components: ArrayCell<P<Node>>, // ElementWithComputedPropertyName
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<IndexInfo>() == 48);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<IndexInfo>() == 24);

impl IndexInfo {
    pub fn key_type(&self) -> P<Type> {
        self.key_type.get().unwrap()
    }
    pub fn value_type(&self) -> P<Type> {
        self.value_type.get().unwrap()
    }
    pub fn is_readonly(&self) -> bool {
        self.is_readonly.get()
    }
    pub fn declaration(&self) -> Option<P<Node>> {
        self.declaration.get()
    }
}

/// Ternary values are defined such that
/// x & y picks the lesser in the order False < Unknown < Maybe < True, and
/// x | y picks the greater in the order False < Unknown < Maybe < True.
/// Generally, Ternary.Maybe is used as the result of a relation that depends on itself, and
/// Ternary.Unknown is used as the result of a variance check that depends on itself. We make
/// a distinction because we don't want to cache circular variance check results.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct Ternary(pub i8);

impl Ternary {
    pub const False: Ternary = Ternary(0);
    pub const Unknown: Ternary = Ternary(1);
    pub const Maybe: Ternary = Ternary(3);
    pub const True: Ternary = Ternary(-1);
}

impl std::ops::BitAnd for Ternary {
    type Output = Ternary;
    fn bitand(self, rhs: Ternary) -> Ternary {
        Ternary(self.0 & rhs.0)
    }
}
impl std::ops::BitOr for Ternary {
    type Output = Ternary;
    fn bitor(self, rhs: Ternary) -> Ternary {
        Ternary(self.0 | rhs.0)
    }
}
impl std::ops::BitAndAssign for Ternary {
    fn bitand_assign(&mut self, rhs: Ternary) {
        self.0 &= rhs.0
    }
}
impl std::ops::BitOrAssign for Ternary {
    fn bitor_assign(&mut self, rhs: Ternary) {
        self.0 |= rhs.0
    }
}

/// Go `type TypeComparer func(s *Type, t *Type, reportErrors bool) Ternary`. Stored in `InferenceContext` and the
/// checker, with shared ownership of the closure; build one with `type_comparer(|c, s, t, report_errors| ...)`.
pub type TypeComparer = std::sync::Arc<dyn Fn(&mut Checker, P<Type>, P<Type>, bool) -> Ternary>;

pub fn type_comparer(f: impl Fn(&mut Checker, P<Type>, P<Type>, bool) -> Ternary + 'static) -> TypeComparer {
    std::sync::Arc::new(f)
}

#[derive(Clone, Copy, Debug)]
pub struct LanguageFeatureMinimumTargetMap {
    pub exponentiation: ScriptTarget,
    pub async_functions: ScriptTarget,
    pub for_await_of: ScriptTarget,
    pub async_generators: ScriptTarget,
    pub async_iteration: ScriptTarget,
    pub object_spread_rest: ScriptTarget,
    pub regular_expression_flags_dot_all: ScriptTarget,
    pub bindingless_catch: ScriptTarget,
    pub big_int: ScriptTarget,
    pub nullish_coalesce: ScriptTarget,
    pub optional_chaining: ScriptTarget,
    pub logical_assignment: ScriptTarget,
    pub top_level_await: ScriptTarget,
    pub class_fields: ScriptTarget,
    pub private_names_and_class_static_blocks: ScriptTarget,
    pub regular_expression_flags_has_indices: ScriptTarget,
    pub shebang_comments: ScriptTarget,
    pub using_and_await_using: ScriptTarget,
    pub class_and_class_element_decorators: ScriptTarget,
    pub regular_expression_flags_unicode_sets: ScriptTarget,
}

pub static LanguageFeatureMinimumTarget: LanguageFeatureMinimumTargetMap = LanguageFeatureMinimumTargetMap {
    exponentiation: ScriptTarget::ES2016,
    async_functions: ScriptTarget::ES2017,
    for_await_of: ScriptTarget::ES2018,
    async_generators: ScriptTarget::ES2018,
    async_iteration: ScriptTarget::ES2018,
    object_spread_rest: ScriptTarget::ES2018,
    regular_expression_flags_dot_all: ScriptTarget::ES2018,
    bindingless_catch: ScriptTarget::ES2019,
    big_int: ScriptTarget::ES2020,
    nullish_coalesce: ScriptTarget::ES2020,
    optional_chaining: ScriptTarget::ES2020,
    logical_assignment: ScriptTarget::ES2021,
    top_level_await: ScriptTarget::ES2022,
    class_fields: ScriptTarget::ES2022,
    private_names_and_class_static_blocks: ScriptTarget::ES2022,
    regular_expression_flags_has_indices: ScriptTarget::ES2022,
    shebang_comments: ScriptTarget::ESNext,
    using_and_await_using: ScriptTarget::ESNext,
    class_and_class_element_decorators: ScriptTarget::ESNext,
    regular_expression_flags_unicode_sets: ScriptTarget::ESNext,
};

// Aliases for types
pub type StringLiteralType = Type;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_edges_keep_identity_and_semantic_ids_across_growth_and_composite_replacement() {
        let mut signatures = tsrs_core::arena_owner::ArenaBuilder::with_capacity(1);
        let first = signatures.alloc(Signature::default());
        signatures.get(first).unwrap().id.set(SignatureId(41));
        let clone = signatures.alloc(Signature::default());
        signatures.get(clone).unwrap().id.set(SignatureId(42));
        signatures.get(clone).unwrap().target.set(Some(first));
        let mut composites = tsrs_core::arena_owner::ArenaBuilder::with_capacity(1);
        let composite = composites.alloc(CompositeSignature { is_union: Cell::new(true), signatures: ArrayCell::new(&[first, clone]) });
        signatures.get(first).unwrap().set_composite(Some(composite));
        signatures.get(clone).unwrap().set_composite(Some(composite));
        let snapshot = composites.get(composite).unwrap().signatures.get();
        for _ in 0..128 {
            signatures.alloc(Signature::default());
            composites.alloc(CompositeSignature::default());
        }
        composites.get(composite).unwrap().signatures.set(&[clone]);
        assert_eq!(signatures.get(first).unwrap().id(), SignatureId(41));
        assert_eq!(signatures.get(clone).unwrap().id(), SignatureId(42));
        assert_eq!(signatures.get(clone).unwrap().target(), Some(first));
        assert_eq!(signatures.get(first).unwrap().composite(), signatures.get(clone).unwrap().composite());
        assert_eq!(snapshot.as_ref(), &[first, clone]);
        assert_eq!(composites.get(composite).unwrap().signatures.get().as_ref(), &[clone]);
        let mut foreign = tsrs_core::arena_owner::ArenaBuilder::new();
        let foreign_key = foreign.alloc(Signature::default());
        foreign.get(foreign_key).unwrap().id.set(SignatureId(41));
        assert_ne!(first, foreign_key);
        assert!(signatures.get(foreign_key).is_none());
        assert!(foreign.get(first).is_none());
        let mut foreign_composites = tsrs_core::arena_owner::ArenaBuilder::new();
        let foreign_composite = foreign_composites.alloc(CompositeSignature::default());
        assert!(composites.get(foreign_composite).is_none());
        assert!(foreign_composites.get(composite).is_none());
        drop(composites);
        drop(signatures);
        // The array retains its key buffer, not the signature records.
        assert_eq!(snapshot.as_ref(), &[first, clone]);
        assert!(foreign.get(snapshot[0]).is_none());
    }

    #[test]
    fn pending_alias_hashes_without_allocation_and_materializes_once_in_its_owner() {
        let region = tsrs_core::arena::Region::new(4096);
        let _scope = region.enter();
        let symbol = Symbol::new(SymbolFlags::TypeAlias, "Alias");
        let t = Type::alloc(TypeFlags::Any, ObjectFlags::None, TypeId(1), IntrinsicType::default());
        let pending = PendingTypeAlias::new(Some(symbol), vec![t]);
        let mut aliases = tsrs_core::arena_owner::ArenaBuilder::with_capacity(1);
        let mut hash = keyBuilder::default();
        hash.write_alias_arg(&aliases, AliasArg::Pending(&pending));
        let pending_hash = hash.hash();
        assert!(pending.alias.get().is_none());
        assert_eq!(aliases.len(), 0);
        let key = pending.materialize(&mut aliases);
        for _ in 0..128 {
            aliases.alloc(TypeAlias::default());
        }
        assert_eq!(pending.materialize(&mut aliases), key);
        assert_eq!(aliases.len(), 129);
        let mut hash = keyBuilder::default();
        hash.write_alias_arg(&aliases, AliasArg::Some(key));
        assert_eq!(hash.hash(), pending_hash);
        let mut foreign = tsrs_core::arena_owner::ArenaBuilder::new();
        let other = foreign.alloc(TypeAlias::new(Some(symbol), &[t]));
        assert_ne!(key, other);
        assert!(aliases.get(other).is_none());
        assert!(foreign.get(key).is_none());
        let mut hash = keyBuilder::default();
        hash.write_alias_arg(&foreign, AliasArg::Some(other));
        assert_eq!(hash.hash(), pending_hash);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pending.materialize(&mut foreign))).is_err());
    }

    #[test]
    fn alias_argument_snapshots_survive_deferred_replacement_and_store_destruction() {
        let region = tsrs_core::arena::Region::new(4096);
        let _scope = region.enter();
        let t1 = Type::alloc(TypeFlags::Any, ObjectFlags::None, TypeId(1), IntrinsicType::default());
        let t2 = Type::alloc(TypeFlags::Unknown, ObjectFlags::None, TypeId(2), IntrinsicType::default());
        let mut aliases = tsrs_core::arena_owner::ArenaBuilder::new();
        let key = aliases.alloc(TypeAlias::new(None, &[t1]));
        let before = aliases.get(key).unwrap().type_arguments();
        aliases.get(key).unwrap().set_type_arguments(vec![t2]);
        let after = aliases.get(key).unwrap().type_arguments();
        drop(aliases);
        assert_eq!(before.as_ref(), &[t1]);
        assert_eq!(after.as_ref(), &[t2]);
        assert_eq!(before[0].id.0, 1);
        assert_eq!(after[0].id.0, 2);
    }

    #[test]
    fn index_metadata_key_keeps_cache_identity_after_store_growth() {
        let region = tsrs_core::arena::Region::new(4096);
        let _scope = region.enter();
        let symbol = Symbol::new(SymbolFlags::Property, "index");
        let mut records = tsrs_core::arena_owner::ArenaBuilder::with_capacity(1);
        let key = records.alloc(IndexInfo::default());
        for _ in 0..128 {
            records.alloc(IndexInfo::default());
        }
        records.get(key).unwrap().index_symbol.set(Some(symbol));
        assert_eq!(records.get(key).unwrap().index_symbol.get(), Some(symbol));
        let mut other = tsrs_core::arena_owner::ArenaBuilder::new();
        let foreign = other.alloc(IndexInfo::default());
        assert!(records.get(foreign).is_none());
        assert!(other.get(key).is_none());
        assert_eq!(other.get(foreign).unwrap().index_symbol.get(), None);
    }

    #[test]
    fn predicate_keys_preserve_sentinel_identity_and_reject_foreign_storage() {
        let mut predicates = tsrs_core::arena_owner::ArenaBuilder::with_capacity(1);
        let sentinel = predicates.alloc(TypePredicate::new(TypePredicateKind::Identifier, "<<unresolved>>", 0, None));
        let predicate = predicates.alloc(TypePredicate::new(TypePredicateKind::Identifier, "retained λ parameter", 1, None));
        let signature = Signature::default();
        assert_eq!(signature.resolved_type_predicate(sentinel), None);
        signature.set_resolved_type_predicate(Some(sentinel), sentinel);
        assert_eq!(signature.resolved_type_predicate(sentinel), Some(sentinel));
        signature.set_resolved_type_predicate(Some(predicate), sentinel);
        assert_eq!(signature.resolved_type_predicate(sentinel), Some(predicate));
        let mut other = tsrs_core::arena_owner::ArenaBuilder::new();
        let foreign = other.alloc(TypePredicate::default());
        assert!(predicates.get(foreign).is_none());
        assert!(other.get(predicate).is_none());
        for _ in 0..128 {
            predicates.alloc(TypePredicate::default());
        }
        let text = predicates.get(predicate).unwrap().parameter_name();
        assert_eq!(predicates.get(predicate).unwrap().parameter_index(), 1);
        drop(predicates);
        assert_eq!(text, "retained λ parameter");
    }

    #[test]
    fn literal_snapshots_survive_replacement_and_type_owner_destruction() {
        let region = tsrs_core::arena::Region::new(4096);
        let snapshot = {
            let _scope = region.enter();
            let payload = LiteralType::default();
            payload.value.set(Some(LiteralValue::String(String::from("retained λ literal").into())));
            let ty = Type::alloc(TypeFlags::StringLiteral, ObjectFlags::None, TypeId(1), payload);
            let snapshot = ty.as_literal_type().value();
            ty.as_literal_type().value.set(Some(LiteralValue::String("replacement".into())));
            snapshot
        };
        drop(region);
        assert_eq!(snapshot, Some(LiteralValue::String("retained λ literal".into())));
    }

    #[test]
    fn inference_context_keys_and_callback_owners_survive_growth_and_release_with_the_store() {
        let capture = std::rc::Rc::new(());
        let weak = std::rc::Rc::downgrade(&capture);
        let comparer = type_comparer(move |_, _, _, _| {
            let _ = &capture;
            Ternary::True
        });
        let mut infos = tsrs_core::arena_owner::ArenaBuilder::with_capacity(1);
        let info = infos.alloc(InferenceInfo::default());
        let mut contexts = tsrs_core::arena_owner::ArenaBuilder::with_capacity(1);
        let context = contexts.alloc(InferenceContext::new(&[info], None, InferenceFlags::NoDefault, std::sync::Arc::clone(&comparer)));
        let snapshot = contexts.get(context).unwrap().inferences.get();
        drop(comparer);
        for _ in 0..128 {
            contexts.alloc(InferenceContext::default());
            infos.alloc(InferenceInfo::default());
        }
        assert!(weak.upgrade().is_some());
        assert_eq!(contexts.get(context).unwrap().flags.get(), InferenceFlags::NoDefault);
        assert_eq!(snapshot.as_ref(), &[info]);
        contexts.get(context).unwrap().inferences.set(&[]);
        assert_eq!(snapshot.as_ref(), &[info]);
        let mut foreign_contexts = tsrs_core::arena_owner::ArenaBuilder::new();
        let foreign_context = foreign_contexts.alloc(InferenceContext::default());
        let mut foreign_infos = tsrs_core::arena_owner::ArenaBuilder::new();
        let foreign_info = foreign_infos.alloc(InferenceInfo::default());
        assert!(contexts.get(foreign_context).is_none());
        assert!(foreign_contexts.get(context).is_none());
        assert!(infos.get(foreign_info).is_none());
        assert!(foreign_infos.get(info).is_none());
        drop(contexts);
        assert!(weak.upgrade().is_none());
        drop(infos);
        // Retaining the array retains only keys, not the inference records or comparison callback.
        assert_eq!(snapshot.as_ref(), &[info]);
        assert!(foreign_infos.get(snapshot[0]).is_none());
    }

    #[test]
    fn value_symbol_links_synthetic_mode_keeps_every_field() {
        let t1 = Type::alloc(TypeFlags::Any, ObjectFlags::None, TypeId(1), IntrinsicType::default());
        let t2 = Type::alloc(TypeFlags::Any, ObjectFlags::None, TypeId(2), IntrinsicType::default());
        let s = Symbol::new(SymbolFlags::Property, "p");
        let mut mappers = tsrs_core::arena_owner::ArenaBuilder::new();
        let m = allocate_simple_mapper(&mut mappers, t1, t2);
        let fields = |l: &ValueSymbolLinks| (l.target(), l.mapper(), l.containing_type(), l.name_type(), l.write_type(), l.function_or_constructor_checked());

        // A synthetic property: containing and name type in the words of target and mapper, then a target moves them.
        let l = ValueSymbolLinks::default();
        l.set_containing_type(Some(t1));
        l.set_name_type(Some(t2));
        l.set_target(None);
        assert_eq!(fields(&l), (None, None, Some(t1), Some(t2), None, false));
        l.set_write_type(Some(t1));
        l.set_function_or_constructor_checked(true);
        assert_eq!(fields(&l), (None, None, Some(t1), Some(t2), Some(t1), true));
        l.set_mapper(Some(m));
        assert_eq!(fields(&l), (None, Some(m), Some(t1), Some(t2), Some(t1), true));
        l.set_target(Some(s));
        l.set_containing_type(None);
        assert_eq!(fields(&l), (Some(s), Some(m), None, Some(t2), Some(t1), true));

        // An instantiated symbol keeps containing/name types in the tail.
        let l = ValueSymbolLinks::default();
        l.set_target(Some(s));
        l.set_name_type(Some(t2));
        l.set_target(None);
        l.set_containing_type(Some(t1));
        assert_eq!(fields(&l), (None, None, Some(t1), Some(t2), None, false));
        l.set_mapper(Some(m));
        assert_eq!(fields(&l), (None, Some(m), Some(t1), Some(t2), None, false));
    }
}

