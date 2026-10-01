use std::fmt;
use std::hash::Hash;

use bitflags::bitflags;

use crate::*;

// GoMap: a Go `map[K]V` field (a nil-able reference to a shared hash table), pointer-sized.

pub struct GoMap<K: 'static, V: 'static>(Cell<Option<P<RefCell<FxHashMap<K, V>>>>>);

impl<K: 'static, V: 'static> Default for GoMap<K, V> {
    fn default() -> Self {
        GoMap(Cell::new(None))
    }
}

impl<K: Eq + Hash + 'static, V: Clone + 'static> GoMap<K, V> {
    pub fn new_empty() -> Self {
        let m = GoMap::default();
        m.make();
        m
    }
    /// Go `m = make(map[K]V)`.
    pub fn make(&self) {
        self.0.set(Some(P::new(RefCell::new(FxHashMap::default()))));
    }
    pub fn is_nil(&self) -> bool {
        self.0.get().is_none()
    }
    /// Go `v, ok := m[k]` (reading a nil map is allowed).
    pub fn get(&self, key: &K) -> Option<V> {
        self.0.get().and_then(|m| m.borrow().get(key).cloned())
    }
    pub fn has(&self, key: &K) -> bool {
        self.0.get().is_some_and(|m| m.borrow().contains_key(key))
    }
    /// Go `m[k] = v`. Creates the map if it is nil (Go would panic; faithful code always `make`s first).
    pub fn set(&self, key: K, value: V) {
        let m = match self.0.get() {
            Some(m) => m,
            None => {
                self.make();
                self.0.get().unwrap()
            }
        };
        m.borrow_mut().insert(key, value);
    }
    pub fn delete(&self, key: &K) {
        if let Some(m) = self.0.get() {
            m.borrow_mut().remove(key);
        }
    }
    pub fn len(&self) -> usize {
        self.0.get().map_or(0, |m| m.borrow().len())
    }
    pub fn clear(&self) {
        if let Some(m) = self.0.get() {
            m.borrow_mut().clear();
        }
    }
    /// The shared table (Go map value), for aliasing assignments `a.m = b.m`.
    pub fn get_ref(&self) -> Option<P<RefCell<FxHashMap<K, V>>>> {
        self.0.get()
    }
    pub fn set_ref(&self, m: Option<P<RefCell<FxHashMap<K, V>>>>) {
        self.0.set(m)
    }
    /// Go `a.m = someFreshlyBuiltMap`.
    pub fn assign(&self, m: FxHashMap<K, V>) {
        self.0.set(Some(P::new(RefCell::new(m))))
    }
    /// Snapshot of the entries (Go `for k, v := range m`).
    pub fn entries(&self) -> Vec<(K, V)>
    where
        K: Clone,
    {
        self.0.get().map_or_else(Vec::new, |m| m.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }
}

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

// Links for referenced symbols

#[derive(Default)]
pub struct SymbolReferenceLinks {
    pub reference_kinds: Cell<SymbolFlags>, // Flags for the meanings of the symbol that were referenced
}

// Links for value symbols
//
// Go's `ValueSymbolLinks` holds all seven fields inline. Most links only ever get `resolved_type`, `target` and
// `mapper` (on Project 81% of the 13.6M records set none of the other four), so those four live in a tail
// allocated on the first write of a non-default value: 32 bytes per record instead of 56. Reads of an absent tail
// return the zero value, exactly like reading the unset field.

#[derive(Default)]
pub struct ValueSymbolLinks {
    pub resolved_type: Cell<Option<P<Type>>>, // Type of value symbol
    pub target: Cell<Option<P<Symbol>>>,
    pub mapper: Cell<Option<P<TypeMapper>>>,
    rare: Cell<Option<P<ValueSymbolLinksRare>>>,
}

#[derive(Default)]
struct ValueSymbolLinksRare {
    write_type: Cell<Option<P<Type>>>,
    name_type: Cell<Option<P<Type>>>,
    containing_type: Cell<Option<P<Type>>>, // Mapped type for mapped type property, containing union or intersection type for synthetic property
    function_or_constructor_checked: Cell<bool>,
}

const _: () = assert!(std::mem::size_of::<ValueSymbolLinks>() == 32);

impl ValueSymbolLinks {
    #[inline]
    fn rare_for_write(&self) -> P<ValueSymbolLinksRare> {
        match self.rare.get() {
            Some(rare) => rare,
            None => {
                let rare = P::new(ValueSymbolLinksRare::default());
                self.rare.set(Some(rare));
                rare
            }
        }
    }

    #[inline]
    pub fn write_type(&self) -> Option<P<Type>> {
        self.rare.get().and_then(|r| r.write_type.get())
    }
    #[inline]
    pub fn set_write_type(&self, t: Option<P<Type>>) {
        if t.is_some() || self.rare.get().is_some() {
            self.rare_for_write().write_type.set(t);
        }
    }
    #[inline]
    pub fn name_type(&self) -> Option<P<Type>> {
        self.rare.get().and_then(|r| r.name_type.get())
    }
    #[inline]
    pub fn set_name_type(&self, t: Option<P<Type>>) {
        if t.is_some() || self.rare.get().is_some() {
            self.rare_for_write().name_type.set(t);
        }
    }
    #[inline]
    pub fn containing_type(&self) -> Option<P<Type>> {
        self.rare.get().and_then(|r| r.containing_type.get())
    }
    #[inline]
    pub fn set_containing_type(&self, t: Option<P<Type>>) {
        if t.is_some() || self.rare.get().is_some() {
            self.rare_for_write().containing_type.set(t);
        }
    }
    #[inline]
    pub fn function_or_constructor_checked(&self) -> bool {
        self.rare.get().is_some_and(|r| r.function_or_constructor_checked.get())
    }
    #[inline]
    pub fn set_function_or_constructor_checked(&self, v: bool) {
        if v || self.rare.get().is_some() {
            self.rare_for_write().function_or_constructor_checked.set(v);
        }
    }
}

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
    pub constituents: Cell<&'static [P<Type>]>, // Calculated list of constituents for a deferred type
    pub write_constituents: Cell<&'static [P<Type>]>, // Constituents of a deferred `writeType`
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
    pub type_only_export_star_map: GoMap<String, P<Node>>, // Set on a module symbol when some of its exports were resolved through a 'export type * from "mod"' declaration
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
    pub type_parameters: Cell<&'static [P<Type>]>, // Type parameters of type alias (undefined if non-generic)
    pub instantiations: GoMap<CacheHashKey, P<Type>>, // Instantiations of generic type alias (undefined if non-generic)
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
    pub switch_types: Cell<&'static [P<Type>]>,
    pub witnesses: Cell<Option<&'static [&'static str]>>, // Go nil (non-literal case) vs empty
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
    pub variances: Cell<Option<&'static [VarianceFlags]>>, // nil = not computed (Go distinguishes nil from empty)
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
    pub extended_containers_by_file: RefCell<FxHashMap<NodeId, &'static [P<Symbol>]>>, // Symbols of nodes which which logically contain this one, cached by file the request is made within
    pub extended_containers: Cell<Option<&'static [P<Symbol>]>>, // Containers (other than the parent) which this symbol is aliased in
    pub accessible_chain_cache: RefCell<FxHashMap<accessibleChainCacheKey, &'static [P<Symbol>]>>,
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
    pub outer_type_parameters: Cell<Option<&'static [P<Type>]>>, // Outer type parameters of anonymous object type (Go distinguishes nil = not computed)
}

#[derive(Default)]
pub struct ComputedNameNodeLinks {
    pub has_name: Cell<Option<bool>>, // If the node has a computable name (Go *bool: nil = not computed)
    pub name: Cell<&'static str>, // Resolved name associated with the type of the node
}

// Links for enum members

#[derive(Default)]
pub struct EnumMemberLinks {
    pub value: Cell<evaluator::Result>, // Constant value of enum member
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
    pub local_jsx_namespace: Cell<&'static str>,
    pub local_jsx_fragment_namespace: Cell<&'static str>,
    pub local_jsx_factory: Cell<Option<P<Node>>>,
    pub local_jsx_fragment_factory: Cell<Option<P<Node>>>,
    pub jsx_fragment_type: Cell<Option<P<Type>>>,
}

// Signature specific links

#[derive(Default)]
pub struct SignatureLinks {
    pub resolved_signature: Cell<Option<P<Signature>>>, // Cached signature of signature node or call expression
    pub effects_signature: Cell<Option<P<Signature>>>, // Signature with possible control flow effects
    pub decorator_signature: Cell<Option<P<Signature>>>, // Signature for decorator as if invoked by the runtime
}

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

#[derive(Default)]
pub struct TypeAlias {
    pub symbol: Cell<Option<P<Symbol>>>,
    pub type_arguments: Cell<&'static [P<Type>]>,
}

impl TypeAlias {
    pub fn symbol(&self) -> Option<P<Symbol>> {
        self.symbol.get()
    }
    pub fn type_arguments(&self) -> &'static [P<Type>] {
        self.type_arguments.get()
    }
}

/// Go's nil-receiver `(*TypeAlias).Symbol()` / `TypeArguments()`: `t.alias().symbol()` works on `Option<P<TypeAlias>>`.
pub trait TypeAliasOptExt {
    fn symbol(self) -> Option<P<Symbol>>;
    fn type_arguments(self) -> &'static [P<Type>];
}

impl TypeAliasOptExt for Option<P<TypeAlias>> {
    fn symbol(self) -> Option<P<Symbol>> {
        self.and_then(|a| a.symbol.get())
    }
    fn type_arguments(self) -> &'static [P<Type>] {
        self.map_or(&[], |a| a.type_arguments.get())
    }
}

// Type
//
// Go's `Type` points to its type-specific data (`data TypeData`, an interface). Here the data struct is allocated
// together with the header, right after it (`TypeAlloc`), and `data_tag` says which struct it is: one allocation
// per type and a 32-byte header. `t.data()` returns the `TypeData` view; the `as_*` casts read the data in place.

pub struct Type {
    pub flags: Cell<TypeFlags>,
    pub object_flags: Cell<ObjectFlags>,
    pub id: TypeId,
    data_tag: TypeDataTag,
    pub symbol: Cell<Option<P<Symbol>>>,
    pub alias: Cell<Option<P<TypeAlias>>>,
}

const _: () = assert!(std::mem::size_of::<Type>() == 32);

/// One arena allocation per type: the header, then the data struct (`repr(C)`: header at offset 0).
#[repr(C)]
struct TypeAlloc<T> {
    header: Type,
    data: T,
}

/// A type data struct, stored after the header of types tagged `TAG`.
pub trait TypePayload: Sized + 'static {
    const TAG: TypeDataTag;
}

/// Which data struct follows a type's header (one variant per `TypeData` variant).
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
    const TAG: TypeDataTag = TypeDataTag::Intrinsic;
}
impl TypePayload for LiteralType {
    const TAG: TypeDataTag = TypeDataTag::Literal;
}
impl TypePayload for UniqueESSymbolType {
    const TAG: TypeDataTag = TypeDataTag::UniqueESSymbol;
}
impl TypePayload for ObjectType {
    const TAG: TypeDataTag = TypeDataTag::Object;
}
impl TypePayload for TypeReference {
    const TAG: TypeDataTag = TypeDataTag::TypeReference;
}
impl TypePayload for InterfaceType {
    const TAG: TypeDataTag = TypeDataTag::Interface;
}
impl TypePayload for TupleType {
    const TAG: TypeDataTag = TypeDataTag::Tuple;
}
impl TypePayload for InstantiationExpressionType {
    const TAG: TypeDataTag = TypeDataTag::InstantiationExpression;
}
impl TypePayload for MappedType {
    const TAG: TypeDataTag = TypeDataTag::Mapped;
}
impl TypePayload for ReverseMappedType {
    const TAG: TypeDataTag = TypeDataTag::ReverseMapped;
}
impl TypePayload for EvolvingArrayType {
    const TAG: TypeDataTag = TypeDataTag::EvolvingArray;
}
impl TypePayload for UnionType {
    const TAG: TypeDataTag = TypeDataTag::Union;
}
impl TypePayload for IntersectionType {
    const TAG: TypeDataTag = TypeDataTag::Intersection;
}
impl TypePayload for TypeParameter {
    const TAG: TypeDataTag = TypeDataTag::TypeParameter;
}
impl TypePayload for IndexType {
    const TAG: TypeDataTag = TypeDataTag::Index;
}
impl TypePayload for IndexedAccessType {
    const TAG: TypeDataTag = TypeDataTag::IndexedAccess;
}
impl TypePayload for TemplateLiteralType {
    const TAG: TypeDataTag = TypeDataTag::TemplateLiteral;
}
impl TypePayload for StringMappingType {
    const TAG: TypeDataTag = TypeDataTag::StringMapping;
}
impl TypePayload for SubstitutionType {
    const TAG: TypeDataTag = TypeDataTag::Substitution;
}
impl TypePayload for ConditionalType {
    const TAG: TypeDataTag = TypeDataTag::Conditional;
}

impl Type {
    /// Allocates a type whose data struct is `data` (only `Checker::new_type` and the checker's placeholder type).
    pub(crate) fn alloc<T: TypePayload>(flags: TypeFlags, object_flags: ObjectFlags, id: TypeId, data: T) -> P<Type> {
        let header = Type { flags: Cell::new(flags), object_flags: Cell::new(object_flags), id, data_tag: T::TAG, symbol: Cell::new(None), alias: Cell::new(None) };
        let a: &'static TypeAlloc<T> = P::new(TypeAlloc { header, data }).get();
        // SAFETY: `TypeAlloc` is `repr(C)` with the header first; arena values are never moved or freed.
        P::from_static(unsafe { &*(a as *const TypeAlloc<T>).cast::<Type>() })
    }

    /// The data struct after this type's header. Callers check `data_tag == T::TAG` first.
    #[inline(always)]
    fn payload<T: TypePayload>(&self) -> &'static T {
        debug_assert!(self.data_tag == T::TAG);
        // SAFETY: a type tagged `T::TAG` was allocated by `Type::alloc::<T>` as a `TypeAlloc<T>` whose header is
        // `self`, so its data struct lives at this offset from the header, for the rest of the process.
        unsafe { &*(self as *const Type).cast::<u8>().add(std::mem::offset_of!(TypeAlloc<T>, data)).cast::<T>() }
    }

    /// The type-specific data (Go `t.data`).
    #[inline]
    pub fn data(&self) -> TypeData {
        match self.data_tag {
            TypeDataTag::Intrinsic => TypeData::Intrinsic(self.payload()),
            TypeDataTag::Literal => TypeData::Literal(self.payload()),
            TypeDataTag::UniqueESSymbol => TypeData::UniqueESSymbol(self.payload()),
            TypeDataTag::Object => TypeData::Object(self.payload()),
            TypeDataTag::TypeReference => TypeData::TypeReference(self.payload()),
            TypeDataTag::Interface => TypeData::Interface(self.payload()),
            TypeDataTag::Tuple => TypeData::Tuple(self.payload()),
            TypeDataTag::InstantiationExpression => TypeData::InstantiationExpression(self.payload()),
            TypeDataTag::Mapped => TypeData::Mapped(self.payload()),
            TypeDataTag::ReverseMapped => TypeData::ReverseMapped(self.payload()),
            TypeDataTag::EvolvingArray => TypeData::EvolvingArray(self.payload()),
            TypeDataTag::Union => TypeData::Union(self.payload()),
            TypeDataTag::Intersection => TypeData::Intersection(self.payload()),
            TypeDataTag::TypeParameter => TypeData::TypeParameter(self.payload()),
            TypeDataTag::Index => TypeData::Index(self.payload()),
            TypeDataTag::IndexedAccess => TypeData::IndexedAccess(self.payload()),
            TypeDataTag::TemplateLiteral => TypeData::TemplateLiteral(self.payload()),
            TypeDataTag::StringMapping => TypeData::StringMapping(self.payload()),
            TypeDataTag::Substitution => TypeData::Substitution(self.payload()),
            TypeDataTag::Conditional => TypeData::Conditional(self.payload()),
        }
    }

    #[inline]
    pub fn data_tag(&self) -> TypeDataTag {
        self.data_tag
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
    pub fn as_intrinsic_type(&self) -> &'static IntrinsicType {
        if self.data_tag != TypeDataTag::Intrinsic {
            panic!("as_intrinsic_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_literal_type(&self) -> &'static LiteralType {
        if self.data_tag != TypeDataTag::Literal {
            panic!("as_literal_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_unique_es_symbol_type(&self) -> &'static UniqueESSymbolType {
        if self.data_tag != TypeDataTag::UniqueESSymbol {
            panic!("as_unique_es_symbol_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_tuple_type(&self) -> &'static TupleType {
        if self.data_tag != TypeDataTag::Tuple {
            panic!("as_tuple_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_instantiation_expression_type(&self) -> &'static InstantiationExpressionType {
        if self.data_tag != TypeDataTag::InstantiationExpression {
            panic!("as_instantiation_expression_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_mapped_type(&self) -> &'static MappedType {
        if self.data_tag != TypeDataTag::Mapped {
            panic!("as_mapped_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_reverse_mapped_type(&self) -> &'static ReverseMappedType {
        if self.data_tag != TypeDataTag::ReverseMapped {
            panic!("as_reverse_mapped_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_evolving_array_type(&self) -> &'static EvolvingArrayType {
        if self.data_tag != TypeDataTag::EvolvingArray {
            panic!("as_evolving_array_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_type_parameter(&self) -> &'static TypeParameter {
        if self.data_tag != TypeDataTag::TypeParameter {
            panic!("as_type_parameter: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_union_type(&self) -> &'static UnionType {
        if self.data_tag != TypeDataTag::Union {
            panic!("as_union_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_intersection_type(&self) -> &'static IntersectionType {
        if self.data_tag != TypeDataTag::Intersection {
            panic!("as_intersection_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_index_type(&self) -> &'static IndexType {
        if self.data_tag != TypeDataTag::Index {
            panic!("as_index_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_indexed_access_type(&self) -> &'static IndexedAccessType {
        if self.data_tag != TypeDataTag::IndexedAccess {
            panic!("as_indexed_access_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_template_literal_type(&self) -> &'static TemplateLiteralType {
        if self.data_tag != TypeDataTag::TemplateLiteral {
            panic!("as_template_literal_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_string_mapping_type(&self) -> &'static StringMappingType {
        if self.data_tag != TypeDataTag::StringMapping {
            panic!("as_string_mapping_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_substitution_type(&self) -> &'static SubstitutionType {
        if self.data_tag != TypeDataTag::Substitution {
            panic!("as_substitution_type: wrong type data");
        }
        self.payload()
    }
    #[inline]
    pub fn as_conditional_type(&self) -> &'static ConditionalType {
        if self.data_tag != TypeDataTag::Conditional {
            panic!("as_conditional_type: wrong type data");
        }
        self.payload()
    }

    // Casts for embedded struct types. `as_*` panics where Go would return nil; `try_as_*` mirrors Go's nil result.

    pub fn as_constrained_type(&self) -> &'static ConstrainedType {
        self.data().as_constrained_type().expect("as_constrained_type: wrong type data")
    }
    pub fn as_structured_type(&self) -> &'static StructuredType {
        self.data().as_structured_type().expect("as_structured_type: wrong type data")
    }
    pub fn as_object_type(&self) -> &'static ObjectType {
        self.data().as_object_type().expect("as_object_type: wrong type data")
    }
    pub fn as_type_reference(&self) -> &'static TypeReference {
        self.data().as_type_reference().expect("as_type_reference: wrong type data")
    }
    pub fn as_interface_type(&self) -> &'static InterfaceType {
        self.data().as_interface_type().expect("as_interface_type: wrong type data")
    }
    pub fn as_union_or_intersection_type(&self) -> &'static UnionOrIntersectionType {
        self.data().as_union_or_intersection_type().expect("as_union_or_intersection_type: wrong type data")
    }
    pub fn try_as_constrained_type(&self) -> Option<&'static ConstrainedType> {
        self.data().as_constrained_type()
    }
    pub fn try_as_structured_type(&self) -> Option<&'static StructuredType> {
        self.data().as_structured_type()
    }
    pub fn try_as_object_type(&self) -> Option<&'static ObjectType> {
        self.data().as_object_type()
    }
    pub fn try_as_type_reference(&self) -> Option<&'static TypeReference> {
        self.data().as_type_reference()
    }
    pub fn try_as_interface_type(&self) -> Option<&'static InterfaceType> {
        self.data().as_interface_type()
    }
    pub fn try_as_union_or_intersection_type(&self) -> Option<&'static UnionOrIntersectionType> {
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

    pub fn mapper(&self) -> Option<P<TypeMapper>> {
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

    pub fn types(&self) -> &'static [P<Type>] {
        let flags = self.flags.get();
        if flags.intersects(TypeFlags::UnionOrIntersection) {
            return self.as_union_or_intersection_type().types.get();
        }
        if flags.intersects(TypeFlags::TemplateLiteral) {
            return self.as_template_literal_type().types.get();
        }
        panic!("Unhandled case in Type.Types")
    }

    pub fn target_interface_type(&self) -> &'static InterfaceType {
        self.as_type_reference().target.get().unwrap().as_interface_type()
    }

    pub fn target_tuple_type(&self) -> &'static TupleType {
        self.as_type_reference().target.get().unwrap().as_tuple_type()
    }

    pub fn symbol(&self) -> Option<P<Symbol>> {
        self.symbol.get()
    }

    pub fn alias(&self) -> Option<P<TypeAlias>> {
        self.alias.get()
    }

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

/// Go's `TypeData` interface. Each payload is a separately arena-allocated struct (`alloc(IntrinsicType { .. })`).
/// Go's `TypeBase` (which embeds the `Type` header) has no Rust counterpart: header fields live on `Type`.
#[derive(Clone, Copy)]
pub enum TypeData {
    Intrinsic(&'static IntrinsicType),
    Literal(&'static LiteralType),
    UniqueESSymbol(&'static UniqueESSymbolType),
    Object(&'static ObjectType), // anonymous (and instantiated anonymous) object types
    TypeReference(&'static TypeReference),
    Interface(&'static InterfaceType),
    Tuple(&'static TupleType),
    InstantiationExpression(&'static InstantiationExpressionType),
    Mapped(&'static MappedType),
    ReverseMapped(&'static ReverseMappedType),
    EvolvingArray(&'static EvolvingArrayType),
    Union(&'static UnionType),
    Intersection(&'static IntersectionType),
    TypeParameter(&'static TypeParameter),
    Index(&'static IndexType),
    IndexedAccess(&'static IndexedAccessType),
    TemplateLiteral(&'static TemplateLiteralType),
    StringMapping(&'static StringMappingType),
    Substitution(&'static SubstitutionType),
    Conditional(&'static ConditionalType),
}

impl TypeData {
    pub fn as_constrained_type(&self) -> Option<&'static ConstrainedType> {
        Some(match *self {
            TypeData::Intrinsic(_) | TypeData::Literal(_) | TypeData::UniqueESSymbol(_) => return None,
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
            TypeData::TypeParameter(d) => d,
            TypeData::Index(d) => d,
            TypeData::IndexedAccess(d) => d,
            TypeData::TemplateLiteral(d) => d,
            TypeData::StringMapping(d) => d,
            TypeData::Substitution(d) => d,
            TypeData::Conditional(d) => d,
        })
    }

    pub fn as_structured_type(&self) -> Option<&'static StructuredType> {
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

    pub fn as_object_type(&self) -> Option<&'static ObjectType> {
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

    pub fn as_type_reference(&self) -> Option<&'static TypeReference> {
        Some(match *self {
            TypeData::TypeReference(d) => d,
            TypeData::Interface(d) => d,
            TypeData::Tuple(d) => d,
            _ => return None,
        })
    }

    pub fn as_interface_type(&self) -> Option<&'static InterfaceType> {
        Some(match *self {
            TypeData::Interface(d) => d,
            TypeData::Tuple(d) => d,
            _ => return None,
        })
    }

    pub fn as_union_or_intersection_type(&self) -> Option<&'static UnionOrIntersectionType> {
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
    pub intrinsic_name: Cell<&'static str>,
}

impl IntrinsicType {
    pub fn intrinsic_name(&self) -> &'static str {
        self.intrinsic_name.get()
    }
}

// LiteralTypeData

/// Go `any` value of a literal type: `string | jsnum.Number | bool | PseudoBigInt`. Go's nil is `Option::None`
/// (`Option<LiteralValue>`), matching how the generated signatures map a nil-able `any`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LiteralValue {
    String(&'static str),
    Number(jsnum::Number),
    Boolean(bool),
    BigInt(jsnum::PseudoBigInt),
}

#[derive(Default)]
pub struct LiteralType {
    pub value: Cell<Option<LiteralValue>>, // string | jsnum.Number | bool | PseudoBigInt | nil (computed enum)
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
    pub name: Cell<&'static str>,
}

// ConstrainedType (type with computed base constraint)

#[derive(Default)]
pub struct ConstrainedType {
    pub resolved_base_constraint: Cell<Option<P<Type>>>,
}

// StructuredType (base of all types with members)

#[derive(Default)]
pub struct StructuredType {
    pub constrained_type: ConstrainedType,
    pub members: Cell<Option<P<SymbolTable>>>,
    pub properties: Cell<&'static [P<Symbol>]>,
    pub signatures: Cell<&'static [P<Signature>]>, // Signatures (call + construct)
    pub call_signature_count: Cell<i32>, // Count of call signatures
    pub index_infos: Cell<&'static [P<IndexInfo>]>,

    pub object_type_without_abstract_construct_signatures: Cell<Option<P<Type>>>,
}
embeds!(StructuredType, constrained_type, ConstrainedType);

impl StructuredType {
    pub fn call_signatures(&self) -> &'static [P<Signature>] {
        &self.signatures.get()[..self.call_signature_count.get() as usize]
    }
    pub fn construct_signatures(&self) -> &'static [P<Signature>] {
        &self.signatures.get()[self.call_signature_count.get() as usize..]
    }
    pub fn properties(&self) -> &'static [P<Symbol>] {
        self.properties.get()
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
    pub mapper: Cell<Option<P<TypeMapper>>>, // Type mapper for instantiated type
    pub instantiations: GoMap<CacheHashKey, P<Type>>, // Map of type instantiations
}
embeds!(ObjectType, structured_type, StructuredType);

// TypeReference (instantiation of an InterfaceType)

#[derive(Default)]
pub struct TypeReference {
    pub object_type: ObjectType,
    pub node: Cell<Option<P<Node>>>, // TypeReferenceNode | ArrayTypeNode | TupleTypeNode when deferred, else nil
    pub resolved_type_arguments: Cell<Option<&'static [P<Type>]>>, // nil = not computed (Go tests against nil)
}
embeds!(TypeReference, object_type, ObjectType);

// InterfaceType (when generic, serves as reference to instantiation of itself)

#[derive(Default)]
pub struct InterfaceType {
    pub type_reference: TypeReference,
    pub all_type_parameters: Cell<&'static [P<Type>]>, // Type parameters (outer + local + thisType)
    pub outer_type_parameter_count: Cell<i32>, // Count of outer type parameters
    pub this_type: Cell<Option<P<Type>>>, // The "this" type (nil if none)
    pub base_types_resolved: Cell<bool>,
    pub declared_members_resolved: Cell<bool>,
    pub resolved_base_constructor_type: Cell<Option<P<Type>>>,
    pub resolved_base_types: Cell<&'static [P<Type>]>,
    pub declared_members: Cell<Option<P<SymbolTable>>>, // Declared members
    pub declared_call_signatures: Cell<&'static [P<Signature>]>, // Declared call signatures
    pub declared_construct_signatures: Cell<&'static [P<Signature>]>, // Declared construct signatures
    pub declared_index_infos: Cell<&'static [P<IndexInfo>]>, // Declared index signatures
}
embeds!(InterfaceType, type_reference, TypeReference);

impl InterfaceType {
    pub fn outer_type_parameters(&self) -> &'static [P<Type>] {
        let all = self.all_type_parameters.get();
        if all.is_empty() {
            return &[];
        }
        &all[..self.outer_type_parameter_count.get() as usize]
    }

    pub fn local_type_parameters(&self) -> &'static [P<Type>] {
        let all = self.all_type_parameters.get();
        if all.is_empty() {
            return &[];
        }
        &all[self.outer_type_parameter_count.get() as usize..all.len() - 1]
    }

    pub fn type_parameters(&self) -> &'static [P<Type>] {
        let all = self.all_type_parameters.get();
        if all.is_empty() {
            return &[];
        }
        &all[..all.len() - 1]
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
    pub fn tuple_element_flags(&self) -> ElementFlags {
        self.flags
    }
    pub fn labeled_declaration(&self) -> Option<P<Node>> {
        self.labeled_declaration
    }
}

#[derive(Default)]
pub struct TupleType {
    pub interface_type: InterfaceType,
    pub element_infos: Cell<&'static [TupleElementInfo]>,
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
    pub fn element_infos(&self) -> &'static [TupleElementInfo] {
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

#[derive(Default)]
pub struct UnionOrIntersectionType {
    pub structured_type: StructuredType,
    pub types: Cell<&'static [P<Type>]>,
    pub property_cache: Cell<Option<P<SymbolTable>>>,
    pub property_cache_without_function_property_augment: Cell<Option<P<SymbolTable>>>,
    pub resolved_properties: Cell<Option<&'static [P<Symbol>]>>, // nil = not computed (Go tests against nil)
}
embeds!(UnionOrIntersectionType, structured_type, StructuredType);

impl UnionOrIntersectionType {
    pub fn types(&self) -> &'static [P<Type>] {
        self.types.get()
    }
}

// UnionType

#[derive(Default)]
pub struct UnionType {
    pub union_or_intersection_type: UnionOrIntersectionType,
    pub resolved_reduced_type: Cell<Option<P<Type>>>,
    pub regular_type: Cell<Option<P<Type>>>,
    pub origin: Cell<Option<P<Type>>>, // Denormalized union, intersection, or index type in which union originates
    pub key_property_name: Cell<&'static str>, // Property with unique unit type that exists in every object/intersection in union type
    pub constituent_map: GoMap<P<Type>, P<Type>>, // Constituents keyed by unit type discriminants
}
embeds!(UnionType, union_or_intersection_type, UnionOrIntersectionType);

// IntersectionType

#[derive(Default)]
pub struct IntersectionType {
    pub union_or_intersection_type: UnionOrIntersectionType,
    pub resolved_apparent_type: Cell<Option<P<Type>>>,
    pub unique_literal_filled_instantiation: Cell<Option<P<Type>>>, // Instantiation with type parameters mapped to never type
}
embeds!(IntersectionType, union_or_intersection_type, UnionOrIntersectionType);

// TypeParameter

#[derive(Default)]
pub struct TypeParameter {
    pub constrained_type: ConstrainedType,
    pub constraint: Cell<Option<P<Type>>>,
    pub target: Cell<Option<P<Type>>>,
    pub mapper: Cell<Option<P<TypeMapper>>>,
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
    pub texts: Cell<&'static [&'static str]>, // Always one element longer than types
    pub types: Cell<&'static [P<Type>]>, // Always at least one element
}
embeds!(TemplateLiteralType, constrained_type, ConstrainedType);

impl TemplateLiteralType {
    pub fn texts(&self) -> &'static [&'static str] {
        self.texts.get()
    }
    pub fn types(&self) -> &'static [P<Type>] {
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
    pub infer_type_parameters: Cell<&'static [P<Type>]>,
    pub outer_type_parameters: Cell<&'static [P<Type>]>,
    pub instantiations: GoMap<CacheHashKey, P<Type>>,
    pub alias: Cell<Option<P<TypeAlias>>>,
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
    pub mapper: Cell<Option<P<TypeMapper>>>,
    pub combined_mapper: Cell<Option<P<TypeMapper>>>,
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

#[derive(Default)]
pub struct Signature {
    pub id: Cell<SignatureId>,
    pub flags: Cell<SignatureFlags>,
    pub min_argument_count: Cell<i32>,
    pub resolved_min_argument_count: Cell<i32>,
    pub declaration: Cell<Option<P<Node>>>,
    pub type_parameters: Cell<&'static [P<Type>]>,
    pub parameters: Cell<&'static [P<Symbol>]>,
    pub this_parameter: Cell<Option<P<Symbol>>>,
    pub resolved_return_type: Cell<Option<P<Type>>>,
    pub resolved_type_predicate: Cell<Option<P<TypePredicate>>>,
    pub target: Cell<Option<P<Signature>>>,
    pub mapper: Cell<Option<P<TypeMapper>>>,
    pub isolated_signature_type: Cell<Option<P<Type>>>,
    pub composite: Cell<Option<P<CompositeSignature>>>,
}

impl Signature {
    pub fn id(&self) -> SignatureId {
        self.id.get()
    }
    pub fn flags(&self) -> SignatureFlags {
        self.flags.get()
    }
    pub fn type_parameters(&self) -> &'static [P<Type>] {
        self.type_parameters.get()
    }
    pub fn declaration(&self) -> Option<P<Node>> {
        self.declaration.get()
    }
    pub fn target(&self) -> Option<P<Signature>> {
        self.target.get()
    }
    pub fn this_parameter(&self) -> Option<P<Symbol>> {
        self.this_parameter.get()
    }
    pub fn parameters(&self) -> &'static [P<Symbol>] {
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
    pub signatures: Cell<&'static [P<Signature>]>, // Individual signatures
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

#[derive(Default)]
pub struct TypePredicate {
    pub kind: Cell<TypePredicateKind>,
    pub parameter_index: Cell<i32>,
    pub parameter_name: Cell<&'static str>,
    pub t: Cell<Option<P<Type>>>,
}

impl TypePredicate {
    pub fn type_(&self) -> Option<P<Type>> {
        self.t.get()
    }
    pub fn kind(&self) -> TypePredicateKind {
        self.kind.get()
    }
    pub fn parameter_index(&self) -> i32 {
        self.parameter_index.get()
    }
    pub fn parameter_name(&self) -> &'static str {
        self.parameter_name.get()
    }
}

// IndexInfo

#[derive(Default)]
pub struct IndexInfo {
    pub key_type: Cell<Option<P<Type>>>,
    pub value_type: Cell<Option<P<Type>>>,
    pub is_readonly: Cell<bool>,
    pub declaration: Cell<Option<P<Node>>>, // IndexSignatureDeclaration
    pub index_symbol: Cell<Option<P<Symbol>>>, // Synthetic property symbol for this index signature
    pub components: Cell<&'static [P<Node>]>, // ElementWithComputedPropertyName
}

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
/// checker, so it is a `Copy` handle; build one with `type_comparer(|c, s, t, report_errors| ...)`.
pub type TypeComparer = &'static dyn Fn(&mut Checker, P<Type>, P<Type>, bool) -> Ternary;

pub fn type_comparer(f: impl Fn(&mut Checker, P<Type>, P<Type>, bool) -> Ternary + 'static) -> TypeComparer {
    alloc(f)
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
