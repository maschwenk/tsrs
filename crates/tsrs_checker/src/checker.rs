//! Non-function declarations of `checker.go`, the `Checker` struct, `NewChecker`, and the methods that replace Go's
//! function-valued `Checker` fields.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, OnceLock};

use bitflags::bitflags;

use crate::*;
use tsrs_core::PSliceCell;

// CheckMode

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct CheckMode: u32 {
        const Normal = 0; // Normal type checking
        const Contextual = 1 << 0; // Explicitly assigned contextual type, therefore not cacheable
        const Inferential = 1 << 1; // Inferential typing
        const SkipContextSensitive = 1 << 2; // Skip context sensitive function expressions
        const SkipGenericFunctions = 1 << 3; // Skip single signature generic functions
        const IsForSignatureHelp = 1 << 4; // Call resolution for purposes of signature help
        const RestBindingElement = 1 << 5; // Checking a type that is going to be used to determine the type of a rest binding element
        //   e.g. in `const { a, ...rest } = foo`, when checking the type of `foo` to determine the type of `rest`,
        //   we need to preserve generic types instead of substituting them for constraints
        const TypeOnly = 1 << 6; // Called from getTypeOfExpression, diagnostics may be omitted
        const ForceTuple = 1 << 7;
    }
}

/// Go `type TypeSystemEntity any` (always one of these in practice).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TypeSystemEntity {
    Node(P<Node>),
    Symbol(P<Symbol>),
    Type(P<Type>),
    Signature(P<Signature>),
}

impl From<P<Node>> for TypeSystemEntity {
    fn from(v: P<Node>) -> Self {
        TypeSystemEntity::Node(v)
    }
}
impl From<P<Symbol>> for TypeSystemEntity {
    fn from(v: P<Symbol>) -> Self {
        TypeSystemEntity::Symbol(v)
    }
}
impl From<P<Type>> for TypeSystemEntity {
    fn from(v: P<Type>) -> Self {
        TypeSystemEntity::Type(v)
    }
}
impl From<P<Signature>> for TypeSystemEntity {
    fn from(v: P<Signature>) -> Self {
        TypeSystemEntity::Signature(v)
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum TypeSystemPropertyName {
    #[default]
    Type,
    ResolvedBaseConstructorType,
    DeclaredType,
    ResolvedReturnType,
    ResolvedBaseConstraint,
    ResolvedTypeArguments,
    ResolvedBaseTypes,
    WriteType,
    InitializerIsUndefined,
    AliasTarget,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TypeResolution {
    pub target: TypeSystemEntity,
    pub property_name: TypeSystemPropertyName,
    pub result: bool,
    /// Flow memo taint source of what is computed while this resolution is in progress (flowmemo.rs).
    pub serial: u32,
}

// ContextualInfo

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ContextualInfo {
    pub node: P<Node>,
    pub t: Option<P<Type>>,
    pub is_cache: bool,
}

// InferenceContextInfo

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InferenceContextInfo {
    pub node: P<Node>,
    pub context: Option<P<InferenceContext>>,
}

// WideningKind

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum WideningKind {
    #[default]
    Normal,
    FunctionReturn,
    GeneratorNext,
    GeneratorYield,
}

// EnumLiteralKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EnumLiteralKey {
    pub enum_symbol: P<Symbol>,
    pub value: LiteralValue,
}

// EnumRelationKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EnumRelationKey {
    pub source_id: SymbolId,
    pub target_id: SymbolId,
}

// TypeCacheKind

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum CachedTypeKind {
    #[default]
    LiteralUnionBaseType,
    IndexType,
    StringIndexType,
    EquivalentBaseType,
    ApparentType,
    AwaitedType,
    EvolvingArrayType,
    ArrayLiteralType,
    PermissiveInstantiation,
    RestrictiveInstantiation,
    RestrictiveTypeParameter,
    IndexedAccessForReading,
    IndexedAccessForWriting,
    Widened,
    RegularObjectLiteral,
    PromisedTypeOfPromise,
    DefaultOnlyType,
    SyntheticType,
    DecoratorContext,
    DecoratorContextStatic,
    DecoratorContextPrivate,
    DecoratorContextPrivateStatic,
}

// CachedTypeKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CachedTypeKey {
    pub kind: CachedTypeKind,
    pub type_id: TypeId,
}

// NarrowedTypeKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NarrowedTypeKey {
    pub t: P<Type>,
    pub candidate: P<Type>,
    pub assume_true: bool,
    pub check_derived: bool,
}

// UnionOfUnionKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UnionOfUnionKey {
    pub id1: TypeId,
    pub id2: TypeId,
    pub r: UnionReduction,
    pub a: CacheHashKey,
}

// CachedSignatureKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CachedSignatureKey {
    pub sig: P<Signature>,
    pub key: CacheHashKey, // Type list key or one of the special keys below
}

pub const SignatureKeyErased: CacheHashKey = CacheHashKey::hash_string_128("-");
pub const SignatureKeyCanonical: CacheHashKey = CacheHashKey::hash_string_128("*");
pub const SignatureKeyBase: CacheHashKey = CacheHashKey::hash_string_128("#");
pub const SignatureKeyInner: CacheHashKey = CacheHashKey::hash_string_128("<");
pub const SignatureKeyOuter: CacheHashKey = CacheHashKey::hash_string_128(">");

// StringMappingKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StringMappingKey {
    pub s: P<Symbol>,
    pub t: P<Type>,
}

// AssignmentReducedKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AssignmentReducedKey {
    pub id1: TypeId,
    pub id2: TypeId,
}

// DiscriminatedContextualTypeKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DiscriminatedContextualTypeKey {
    pub node_id: NodeId,
    pub type_id: TypeId,
}

// InstantiationExpressionKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InstantiationExpressionKey {
    pub node_id: NodeId,
    pub type_id: TypeId,
}

// SubstitutionTypeKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SubstitutionTypeKey {
    pub base_id: TypeId,
    pub constraint_id: TypeId,
}

// ReverseMappedTypeKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ReverseMappedTypeKey {
    pub source_id: TypeId,
    pub target_id: TypeId,
    pub constraint_id: TypeId,
}

// IterationTypesKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IterationTypesKey {
    pub type_id: TypeId,
    pub use_: IterationUse,
}

// PropertiesTypesKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PropertiesTypesKey {
    pub type_id: TypeId,
    pub include: TypeFlags,
    pub include_origin: bool,
}

// NonExistentPropertyKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NonExistentPropertyKey {
    pub prop_node: P<Node>,
    pub containing_type: P<Type>,
    pub is_unchecked_js: bool,
}

// FlowLoopKey

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FlowLoopKey {
    pub flow_node: P<FlowNode>,
    pub ref_key: CacheHashKey,
}

#[derive(Clone, Debug)]
pub struct FlowLoopInfo {
    pub key: FlowLoopKey,
    pub types: Vec<P<Type>>,
    /// Flow memo taint source of the in-process types (flowmemo.rs).
    pub serial: u32,
}

// InferenceFlags

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct InferenceFlags: u32 {
        const None = 0; // No special inference behaviors
        const NoDefault = 1 << 0; // Infer silentNeverType for no inferences (otherwise anyType or unknownType)
        const AnyDefault = 1 << 1; // Infer anyType (in JS files) for no inferences (otherwise unknownType)
        const SkippedGenericFunction = 1 << 2; // A generic function was skipped during inference
        const NoConstraintChecks = 1 << 3;
    }
}

// InferenceContext
//
// 1.43M contexts on the private monorepo single, so the four fields that fewer than 4% of them set (return mappers, inferred type
// parameters, intra-expression sites) live in a tail allocated on the first non-default write (`InferenceContextRare`,
// read through accessors that return the zero value when it is absent), and `inferences` packs with `flags`:
// 64 bytes instead of 128 (48 with compressed pointers, where `inferences` is a one-word `ThinSliceCell`).

#[derive(Default)]
pub struct InferenceContext {
    pub inferences: PSliceCell<P<InferenceInfo>>, // Inferences made for each type parameter
    pub flags: Cell<InferenceFlags>, // Inference flags
    pub signature: Cell<Option<P<Signature>>>, // Generic signature for which inferences are made (if any)
    pub compare_types: Cell<Option<TypeComparer>>, // Type comparer function
    // Mapper that fixes inferences / that doesn't: created on first use with `TSRS_LAZY_INFERENCE_MAPPERS` (`mapper()`,
    // `non_fixing_mapper()`), see notes/mem-round3.md.
    mapper: Cell<Option<P<TypeMapper>>>,
    non_fixing_mapper: Cell<Option<P<TypeMapper>>>,
    // The `InferenceContextRare`'s `P::to_bits` with the escaped bit (`RARE_ESCAPED`) in bit 0: set
    // when one of the context's inference mappers escapes (notes/mem-recycle.md), after which it is never recycled.
    rare: Cell<usize>,
}

const RARE_ESCAPED: usize = 1;

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<InferenceContext>() == if tsrs_core::COMPRESSED_PTRS { 48 } else { 64 });
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<InferenceContext>() == 36);

#[derive(Default)]
pub(crate) struct InferenceContextRare {
    return_mapper: Cell<Option<P<TypeMapper>>>, // Type mapper for inferences from return types (if any)
    outer_return_mapper: Cell<Option<P<TypeMapper>>>, // Type mapper for inferences from return types of outer function (if any)
    inferred_type_parameters: Cell<&'static [P<Type>]>, // Inferred type parameters for function result
    intra_expression_inference_sites: RefCell<Vec<IntraExpressionInferenceSite>>,
}

impl InferenceContext {
    pub(crate) fn new(inferences: &'static [P<InferenceInfo>], signature: Option<P<Signature>>, flags: InferenceFlags, compare_types: TypeComparer) -> InferenceContext {
        InferenceContext {
            inferences: PSliceCell::new(inferences),
            signature: Cell::new(signature),
            flags: Cell::new(flags),
            compare_types: Cell::new(Some(compare_types)),
            ..Default::default()
        }
    }

    /// The arena handle of this context (contexts are only created in the arena and never moved).
    fn as_p(&self) -> P<InferenceContext> {
        // SAFETY: see above.
        unsafe { P::from_arena(&*std::ptr::from_ref::<InferenceContext>(self)) }
    }

    fn rare(&self) -> Option<P<InferenceContextRare>> {
        // SAFETY: nonzero bits were stored from a live `P<InferenceContextRare>` (`rare_for_write`).
        unsafe { P::from_bits_opt(self.rare.get() & !RARE_ESCAPED) }
    }

    /// Whether one of the context's inference mappers escaped (it may be used after its creator is done).
    pub(crate) fn escaped(&self) -> bool {
        self.rare.get() & RARE_ESCAPED != 0
    }

    /// Marks the context escaped, with every mapper it holds (an escaped context's mappers may be used through it).
    pub(crate) fn escape(&self) {
        // As for mappers (`escape_mapper`): a frozen seed context is never recycled by a fork.
        if self.escaped() || tsrs_core::sharedgraph::frozen(self) {
            return;
        }
        self.rare.set(self.rare.get() | RARE_ESCAPED);
        for m in [self.mapper.get(), self.non_fixing_mapper.get(), self.return_mapper(), self.outer_return_mapper()].into_iter().flatten() {
            escape_mapper(m);
        }
    }

    /// The context's own inference mappers, when created (for recycling).
    pub(crate) fn own_mappers(&self) -> [Option<P<TypeMapper>>; 2] {
        [self.mapper.get(), self.non_fixing_mapper.get()]
    }

    /// Go `context.mapper`, the mapper that fixes inferences. Go creates it with the context; here it may be created
    /// on the first call (candidate B2), once, so every caller gets the same mapper.
    pub fn mapper(&self) -> Option<P<TypeMapper>> {
        if self.mapper.get().is_none() {
            let m = new_inference_type_mapper(self.as_p(), true /*fixing*/);
            if self.escaped() {
                escape_mapper(m);
            }
            self.mapper.set(Some(m));
        }
        self.mapper.get()
    }

    /// Go `context.nonFixingMapper` (created like `mapper()`).
    pub fn non_fixing_mapper(&self) -> Option<P<TypeMapper>> {
        if self.non_fixing_mapper.get().is_none() {
            let m = new_inference_type_mapper(self.as_p(), false /*fixing*/);
            if self.escaped() {
                escape_mapper(m);
            }
            self.non_fixing_mapper.set(Some(m));
        }
        self.non_fixing_mapper.get()
    }

    pub fn set_non_fixing_mapper(&self, mapper: P<TypeMapper>) {
        if self.escaped() {
            escape_mapper(mapper);
        }
        self.non_fixing_mapper.set(Some(mapper));
    }

    fn rare_for_write(&self) -> P<InferenceContextRare> {
        match self.rare() {
            Some(rare) => rare,
            None => {
                let rare = P::new_recycled(InferenceContextRare::default());
                self.rare.set(rare.to_bits() | (self.rare.get() & RARE_ESCAPED));
                rare
            }
        }
    }
    pub fn return_mapper(&self) -> Option<P<TypeMapper>> {
        self.rare().and_then(|r| r.return_mapper.get())
    }
    // The return mappers live and die with the context: they escape when it does (`escape` walks them), and
    // `recycle` frees them with it (notes/mem-scoped-arenas.md).
    pub fn set_return_mapper(&self, mapper: Option<P<TypeMapper>>) {
        if mapper.is_some() || self.rare().is_some() {
            if let Some(m) = mapper {
                if self.escaped() {
                    escape_mapper(m);
                }
            }
            self.rare_for_write().return_mapper.set(mapper);
        }
    }
    pub fn outer_return_mapper(&self) -> Option<P<TypeMapper>> {
        self.rare().and_then(|r| r.outer_return_mapper.get())
    }
    pub fn set_outer_return_mapper(&self, mapper: Option<P<TypeMapper>>) {
        if mapper.is_some() || self.rare().is_some() {
            if let Some(m) = mapper {
                if self.escaped() {
                    escape_mapper(m);
                }
            }
            self.rare_for_write().outer_return_mapper.set(mapper);
        }
    }
    pub fn inferred_type_parameters(&self) -> &'static [P<Type>] {
        self.rare().map_or(&[], |r| r.inferred_type_parameters.get())
    }
    pub fn set_inferred_type_parameters(&self, type_parameters: &'static [P<Type>]) {
        if !type_parameters.is_empty() || self.rare().is_some() {
            self.rare_for_write().inferred_type_parameters.set(type_parameters);
        }
    }
    pub fn has_intra_expression_inference_sites(&self) -> bool {
        self.rare().is_some_and(|r| !r.intra_expression_inference_sites.borrow().is_empty())
    }
    pub fn intra_expression_inference_sites(&self) -> Vec<IntraExpressionInferenceSite> {
        self.rare().map_or_else(Vec::new, |r| r.intra_expression_inference_sites.borrow().clone())
    }
    pub fn push_intra_expression_inference_site(&self, site: IntraExpressionInferenceSite) {
        self.rare_for_write().intra_expression_inference_sites.borrow_mut().push(site);
    }
    pub fn clear_intra_expression_inference_sites(&self) {
        if let Some(rare) = self.rare() {
            rare.intra_expression_inference_sites.borrow_mut().clear();
        }
    }

    /// Gives a context whose mappers never escaped back to the arena when its creator is done with it: the context,
    /// its own inference mappers, its inference infos and their candidate lists, its info slice and its tail. Its
    /// creator guarantees that nothing else holds it (no live inference-context stack entry, no clone that shares
    /// state: clones copy their infos). Contexts that escaped are kept.
    pub(crate) fn recycle(n: P<InferenceContext>) {
        if n.escaped() {
            return;
        }
        for m in n.own_mappers().into_iter().flatten() {
            if matches!(m.data(), TypeMapperData::Inference { n: owner, .. } if owner == n) {
                debug_assert!(!m.escaped());
                // SAFETY: an inference mapper of `n` that never escaped is referenced only by `n` and by mappers
                // that did not escape either (dead with their creators).
                unsafe { tsrs_core::free!(m) };
            }
        }
        let inferences = n.inferences.get();
        for &info in inferences {
            info.candidates.recycle();
            info.contra_candidates.recycle();
            // SAFETY: infos belong to exactly one context (clones copy them; merged infos come from a local list).
            unsafe { tsrs_core::free!(info) };
        }
        // SAFETY: the context's own list (`alloc_slice_recycled` in `newInferenceContextWorker`, or the merged copy).
        unsafe { tsrs_core::free_slice!(inferences) };
        if let Some(rare) = n.rare() {
            // The return mappers: the mapper of a clone made for this context by `inferTypeArguments`, and the
            // outer return mapper `createOuterReturnMapper` cached here (the mapper of another clone, merged after
            // the return mapper of that time). Only this context refers to them unless they escaped.
            let return_mapper = rare.return_mapper.get();
            if let Some(o) = rare.outer_return_mapper.get() {
                if !o.escaped() {
                    match o.data() {
                        TypeMapperData::Merged { m1, m2 } => {
                            // SAFETY: made by `createOuterReturnMapper` for this context only.
                            unsafe { tsrs_core::free!(o) };
                            InferenceContext::recycle_held_mapper(m2);
                            if Some(m1) != return_mapper {
                                // A replaced return mapper: `o` was its last holder.
                                InferenceContext::recycle_held_mapper(m1);
                            }
                        }
                        _ => InferenceContext::recycle_held_mapper(o),
                    }
                }
            }
            if let Some(r) = return_mapper {
                InferenceContext::recycle_held_mapper(r);
            }
            drop(std::mem::take(&mut *rare.intra_expression_inference_sites.borrow_mut()));
            // SAFETY: only `n` points to its tail (the type parameter list it holds is not freed).
            unsafe { tsrs_core::free!(rare) };
        }
        // SAFETY: see above.
        unsafe { tsrs_core::free!(n) };
    }

    /// Recycles the clone behind `m`, the mapper (`InferenceContext::mapper`) of a context that only `m`'s holder,
    /// which is being recycled, refers to; with `m` itself (one of the clone's own mappers). Kept if `m` escaped.
    fn recycle_held_mapper(m: P<TypeMapper>) {
        if m.escaped() {
            return;
        }
        if let TypeMapperData::Inference { n, .. } = m.data() {
            debug_assert!(n.mapper.get() == Some(m));
            InferenceContext::recycle(n);
        }
    }
}

/// A Go slice field that most of its owners leave empty (inference candidate lists, 1.78M of them on the private monorepo, 64%
/// never get a covariant and 98% never a contravariant candidate): 8 bytes, and the list lives in an arena
/// `RefCell<Vec>` allocated by the first `push` or a non-empty `from_vec`. Reads of an absent list see it empty.
pub struct LazyVec<T: 'static>(Cell<Option<P<RefCell<Vec<T>>>>>);

impl<T> Default for LazyVec<T> {
    fn default() -> Self {
        LazyVec(Cell::new(None))
    }
}

impl<T: Copy + PartialEq> LazyVec<T> {
    pub fn from_vec(items: Vec<T>) -> Self {
        LazyVec(Cell::new((!items.is_empty()).then(|| P::new_recycled(RefCell::new(items)))))
    }
    pub fn is_empty(&self) -> bool {
        self.0.get().is_none_or(|v| v.borrow().is_empty())
    }
    pub fn contains(&self, item: &T) -> bool {
        self.0.get().is_some_and(|v| v.borrow().contains(item))
    }
    pub fn push(&self, item: T) {
        match self.0.get() {
            Some(v) => v.borrow_mut().push(item),
            None => self.0.set(Some(P::new_recycled(RefCell::new(vec![item])))),
        }
    }
    pub fn clear(&self) {
        if let Some(v) = self.0.get() {
            v.borrow_mut().clear();
        }
    }
    pub fn to_vec(&self) -> Vec<T> {
        self.0.get().map_or_else(Vec::new, |v| v.borrow().clone())
    }
    /// Calls `f` with the items, without copying them.
    pub fn with_slice<R>(&self, f: impl FnOnce(&[T]) -> R) -> R {
        match self.0.get() {
            Some(v) => f(&v.borrow()),
            None => f(&[]),
        }
    }
    /// Frees the list (heap buffer and arena cell) of a dead owner (`InferenceContext::recycle`).
    pub(crate) fn recycle(&self) {
        if let Some(v) = self.0.take() {
            drop(std::mem::take(&mut *v.borrow_mut()));
            // SAFETY: a `LazyVec` cell is owned by its one `LazyVec` (`push` / `from_vec` allocate a new one).
            unsafe { tsrs_core::free!(v) };
        }
    }
}

#[derive(Default)]
pub struct InferenceInfo {
    pub type_parameter: Cell<Option<P<Type>>>, // Type parameter for which inferences are being made
    pub candidates: LazyVec<P<Type>>, // Candidates in covariant positions in decreasing depth order
    pub contra_candidates: LazyVec<P<Type>>, // Candidates in contravariant positions
    pub inferred_type: Cell<Option<P<Type>>>, // Cache for resolved inferred type
    pub priority: Cell<InferencePriority>, // Priority of current inference set
    pub top_level: Cell<bool>, // True if all inferences are to top level occurrences
    pub is_fixed: Cell<bool>, // True if inferences are fixed
    pub implied_arity: Cell<i32>, // Implied arity (or -1)
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<InferenceInfo>() == if tsrs_core::COMPRESSED_PTRS { 28 } else { 48 });
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<InferenceInfo>() == 28);

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct InferencePriority: i32 {
        const None = 0;
        const NakedTypeVariable = 1 << 0; // Naked type variable in union or intersection type
        const SpeculativeTuple = 1 << 1; // Speculative tuple inference
        const SubstituteSource = 1 << 2; // Source of inference originated within a substitution type's substitute
        const HomomorphicMappedType = 1 << 3; // Reverse inference for homomorphic mapped type
        const PartialHomomorphicMappedType = 1 << 4; // Partial reverse inference for homomorphic mapped type
        const MappedTypeConstraint = 1 << 5; // Reverse inference for mapped type
        const ContravariantConditional = 1 << 6; // Conditional type in contravariant position
        const ReturnType = 1 << 7; // Inference made from return type of generic function
        const LiteralKeyof = 1 << 8; // Inference made from a string literal to a keyof T
        const NoConstraints = 1 << 9; // Don't infer from constraints of instantiable types
        const AlwaysStrict = 1 << 10; // Always use strict rules for contravariant inferences
        const MaxValue = 1 << 11; // Seed for inference priority tracking
        const Circularity = -1; // Inference circularity (value less than all other priorities)

        const PriorityImpliesCombination = Self::ReturnType.bits() | Self::MappedTypeConstraint.bits() | Self::LiteralKeyof.bits(); // These priorities imply that the resulting type should be a combination of all candidates
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IntraExpressionInferenceSite {
    pub node: P<Node>,
    pub t: P<Type>,
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct DeclarationMeaning: u32 {
        const GetAccessor = 1 << 0;
        const SetAccessor = 1 << 1;
        const PropertyAssignment = 1 << 2;
        const Method = 1 << 3;
        const PrivateStatic = 1 << 4;
        const GetOrSetAccessor = Self::GetAccessor.bits() | Self::SetAccessor.bits();
        const PropertyAssignmentOrMethod = Self::PropertyAssignment.bits() | Self::Method.bits();
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct DeclarationSpaces: i32 {
        const None = 0;
        const ExportValue = 1 << 0;
        const ExportType = 1 << 1;
        const ExportNamespace = 1 << 2;
    }
}

// IntrinsicTypeKind

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum IntrinsicTypeKind {
    #[default]
    Unknown,
    Uppercase,
    Lowercase,
    Capitalize,
    Uncapitalize,
    NoInfer,
}

pub static intrinsicTypeKinds: LazyLock<FxHashMap<&'static str, IntrinsicTypeKind>> = LazyLock::new(|| {
    FxHashMap::from_iter([
        ("Uppercase", IntrinsicTypeKind::Uppercase),
        ("Lowercase", IntrinsicTypeKind::Lowercase),
        ("Capitalize", IntrinsicTypeKind::Capitalize),
        ("Uncapitalize", IntrinsicTypeKind::Uncapitalize),
        ("NoInfer", IntrinsicTypeKind::NoInfer),
    ])
});

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct MappedTypeModifiers: u32 {
        const IncludeReadonly = 1 << 0;
        const ExcludeReadonly = 1 << 1;
        const IncludeOptional = 1 << 2;
        const ExcludeOptional = 1 << 3;
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum MappedTypeNameTypeKind {
    #[default]
    None,
    Filtering,
    Remapping,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum ReferenceHint {
    #[default]
    Unspecified,
    Identifier,
    Property,
    ExportAssignment,
    Jsx,
    ExportImportEquals,
    ExportSpecifier,
    Decorator,
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct TypeFacts: u32 {
        const None = 0;
        const TypeofEQString = 1 << 0;
        const TypeofEQNumber = 1 << 1;
        const TypeofEQBigInt = 1 << 2;
        const TypeofEQBoolean = 1 << 3;
        const TypeofEQSymbol = 1 << 4;
        const TypeofEQObject = 1 << 5;
        const TypeofEQFunction = 1 << 6;
        const TypeofEQHostObject = 1 << 7;
        const TypeofNEString = 1 << 8;
        const TypeofNENumber = 1 << 9;
        const TypeofNEBigInt = 1 << 10;
        const TypeofNEBoolean = 1 << 11;
        const TypeofNESymbol = 1 << 12;
        const TypeofNEObject = 1 << 13;
        const TypeofNEFunction = 1 << 14;
        const TypeofNEHostObject = 1 << 15;
        const EQUndefined = 1 << 16;
        const EQNull = 1 << 17;
        const EQUndefinedOrNull = 1 << 18;
        const NEUndefined = 1 << 19;
        const NENull = 1 << 20;
        const NEUndefinedOrNull = 1 << 21;
        const Truthy = 1 << 22;
        const Falsy = 1 << 23;
        const IsUndefined = 1 << 24;
        const IsNull = 1 << 25;
        const IsUndefinedOrNull = Self::IsUndefined.bits() | Self::IsNull.bits();
        const All = (1 << 27) - 1;
        // The following members encode facts about particular kinds of types for use in the getTypeFacts function.
        // The presence of a particular fact means that the given test is true for some (and possibly all) values
        // of that kind of type.
        const BaseStringStrictFacts = Self::TypeofEQString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEObject.bits() | Self::TypeofNEFunction.bits() | Self::TypeofNEHostObject.bits() | Self::NEUndefined.bits() | Self::NENull.bits() | Self::NEUndefinedOrNull.bits();
        const BaseStringFacts = Self::BaseStringStrictFacts.bits() | Self::EQUndefined.bits() | Self::EQNull.bits() | Self::EQUndefinedOrNull.bits() | Self::Falsy.bits();
        const StringStrictFacts = Self::BaseStringStrictFacts.bits() | Self::Truthy.bits() | Self::Falsy.bits();
        const StringFacts = Self::BaseStringFacts.bits() | Self::Truthy.bits();
        const EmptyStringStrictFacts = Self::BaseStringStrictFacts.bits() | Self::Falsy.bits();
        const EmptyStringFacts = Self::BaseStringFacts.bits();
        const NonEmptyStringStrictFacts = Self::BaseStringStrictFacts.bits() | Self::Truthy.bits();
        const NonEmptyStringFacts = Self::BaseStringFacts.bits() | Self::Truthy.bits();
        const BaseNumberStrictFacts = Self::TypeofEQNumber.bits() | Self::TypeofNEString.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEObject.bits() | Self::TypeofNEFunction.bits() | Self::TypeofNEHostObject.bits() | Self::NEUndefined.bits() | Self::NENull.bits() | Self::NEUndefinedOrNull.bits();
        const BaseNumberFacts = Self::BaseNumberStrictFacts.bits() | Self::EQUndefined.bits() | Self::EQNull.bits() | Self::EQUndefinedOrNull.bits() | Self::Falsy.bits();
        const NumberStrictFacts = Self::BaseNumberStrictFacts.bits() | Self::Truthy.bits() | Self::Falsy.bits();
        const NumberFacts = Self::BaseNumberFacts.bits() | Self::Truthy.bits();
        const ZeroNumberStrictFacts = Self::BaseNumberStrictFacts.bits() | Self::Falsy.bits();
        const ZeroNumberFacts = Self::BaseNumberFacts.bits();
        const NonZeroNumberStrictFacts = Self::BaseNumberStrictFacts.bits() | Self::Truthy.bits();
        const NonZeroNumberFacts = Self::BaseNumberFacts.bits() | Self::Truthy.bits();
        const BaseBigIntStrictFacts = Self::TypeofEQBigInt.bits() | Self::TypeofNEString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEObject.bits() | Self::TypeofNEFunction.bits() | Self::TypeofNEHostObject.bits() | Self::NEUndefined.bits() | Self::NENull.bits() | Self::NEUndefinedOrNull.bits();
        const BaseBigIntFacts = Self::BaseBigIntStrictFacts.bits() | Self::EQUndefined.bits() | Self::EQNull.bits() | Self::EQUndefinedOrNull.bits() | Self::Falsy.bits();
        const BigIntStrictFacts = Self::BaseBigIntStrictFacts.bits() | Self::Truthy.bits() | Self::Falsy.bits();
        const BigIntFacts = Self::BaseBigIntFacts.bits() | Self::Truthy.bits();
        const ZeroBigIntStrictFacts = Self::BaseBigIntStrictFacts.bits() | Self::Falsy.bits();
        const ZeroBigIntFacts = Self::BaseBigIntFacts.bits();
        const NonZeroBigIntStrictFacts = Self::BaseBigIntStrictFacts.bits() | Self::Truthy.bits();
        const NonZeroBigIntFacts = Self::BaseBigIntFacts.bits() | Self::Truthy.bits();
        const BaseBooleanStrictFacts = Self::TypeofEQBoolean.bits() | Self::TypeofNEString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEObject.bits() | Self::TypeofNEFunction.bits() | Self::TypeofNEHostObject.bits() | Self::NEUndefined.bits() | Self::NENull.bits() | Self::NEUndefinedOrNull.bits();
        const BaseBooleanFacts = Self::BaseBooleanStrictFacts.bits() | Self::EQUndefined.bits() | Self::EQNull.bits() | Self::EQUndefinedOrNull.bits() | Self::Falsy.bits();
        const BooleanStrictFacts = Self::BaseBooleanStrictFacts.bits() | Self::Truthy.bits() | Self::Falsy.bits();
        const BooleanFacts = Self::BaseBooleanFacts.bits() | Self::Truthy.bits();
        const FalseStrictFacts = Self::BaseBooleanStrictFacts.bits() | Self::Falsy.bits();
        const FalseFacts = Self::BaseBooleanFacts.bits();
        const TrueStrictFacts = Self::BaseBooleanStrictFacts.bits() | Self::Truthy.bits();
        const TrueFacts = Self::BaseBooleanFacts.bits() | Self::Truthy.bits();
        const SymbolStrictFacts = Self::TypeofEQSymbol.bits() | Self::TypeofNEString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNEObject.bits() | Self::TypeofNEFunction.bits() | Self::TypeofNEHostObject.bits() | Self::NEUndefined.bits() | Self::NENull.bits() | Self::NEUndefinedOrNull.bits() | Self::Truthy.bits();
        const SymbolFacts = Self::SymbolStrictFacts.bits() | Self::EQUndefined.bits() | Self::EQNull.bits() | Self::EQUndefinedOrNull.bits() | Self::Falsy.bits();
        const ObjectStrictFacts = Self::TypeofEQObject.bits() | Self::TypeofEQHostObject.bits() | Self::TypeofNEString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEFunction.bits() | Self::NEUndefined.bits() | Self::NENull.bits() | Self::NEUndefinedOrNull.bits() | Self::Truthy.bits();
        const ObjectFacts = Self::ObjectStrictFacts.bits() | Self::EQUndefined.bits() | Self::EQNull.bits() | Self::EQUndefinedOrNull.bits() | Self::Falsy.bits();
        const FunctionStrictFacts = Self::TypeofEQFunction.bits() | Self::TypeofEQHostObject.bits() | Self::TypeofNEString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEObject.bits() | Self::NEUndefined.bits() | Self::NENull.bits() | Self::NEUndefinedOrNull.bits() | Self::Truthy.bits();
        const FunctionFacts = Self::FunctionStrictFacts.bits() | Self::EQUndefined.bits() | Self::EQNull.bits() | Self::EQUndefinedOrNull.bits() | Self::Falsy.bits();
        const VoidFacts = Self::TypeofNEString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEObject.bits() | Self::TypeofNEFunction.bits() | Self::TypeofNEHostObject.bits() | Self::EQUndefined.bits() | Self::EQUndefinedOrNull.bits() | Self::NENull.bits() | Self::Falsy.bits();
        const UndefinedFacts = Self::TypeofNEString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEObject.bits() | Self::TypeofNEFunction.bits() | Self::TypeofNEHostObject.bits() | Self::EQUndefined.bits() | Self::EQUndefinedOrNull.bits() | Self::NENull.bits() | Self::Falsy.bits() | Self::IsUndefined.bits();
        const NullFacts = Self::TypeofEQObject.bits() | Self::TypeofNEString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEFunction.bits() | Self::TypeofNEHostObject.bits() | Self::EQNull.bits() | Self::EQUndefinedOrNull.bits() | Self::NEUndefined.bits() | Self::Falsy.bits() | Self::IsNull.bits();
        const EmptyObjectStrictFacts = Self::All.bits() & !(Self::EQUndefined.bits() | Self::EQNull.bits() | Self::EQUndefinedOrNull.bits() | Self::IsUndefinedOrNull.bits());
        const EmptyObjectFacts = Self::All.bits() & !Self::IsUndefinedOrNull.bits();
        const UnknownFacts = Self::All.bits() & !Self::IsUndefinedOrNull.bits();
        const AllTypeofNE = Self::TypeofNEString.bits() | Self::TypeofNENumber.bits() | Self::TypeofNEBigInt.bits() | Self::TypeofNEBoolean.bits() | Self::TypeofNESymbol.bits() | Self::TypeofNEObject.bits() | Self::TypeofNEFunction.bits() | Self::NEUndefined.bits();
        // Masks
        const OrFactsMask = Self::TypeofEQFunction.bits() | Self::TypeofNEObject.bits();
        const AndFactsMask = Self::All.bits() & !Self::OrFactsMask.bits();
    }
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct IterationUse: u32 {
        const AllowsSyncIterablesFlag = 1 << 0;
        const AllowsAsyncIterablesFlag = 1 << 1;
        const AllowsStringInputFlag = 1 << 2;
        const ForOfFlag = 1 << 3;
        const YieldStarFlag = 1 << 4;
        const SpreadFlag = 1 << 5;
        const DestructuringFlag = 1 << 6;
        const PossiblyOutOfBounds = 1 << 7;
        // Spread, Destructuring, Array element assignment
        const Element = Self::AllowsSyncIterablesFlag.bits();
        const Spread = Self::AllowsSyncIterablesFlag.bits() | Self::SpreadFlag.bits();
        const Destructuring = Self::AllowsSyncIterablesFlag.bits() | Self::DestructuringFlag.bits();
        const ForOf = Self::AllowsSyncIterablesFlag.bits() | Self::AllowsStringInputFlag.bits() | Self::ForOfFlag.bits();
        const ForAwaitOf = Self::AllowsSyncIterablesFlag.bits() | Self::AllowsAsyncIterablesFlag.bits() | Self::AllowsStringInputFlag.bits() | Self::ForOfFlag.bits();
        const YieldStar = Self::AllowsSyncIterablesFlag.bits() | Self::YieldStarFlag.bits();
        const AsyncYieldStar = Self::AllowsSyncIterablesFlag.bits() | Self::AllowsAsyncIterablesFlag.bits() | Self::YieldStarFlag.bits();
        const GeneratorReturnType = Self::AllowsSyncIterablesFlag.bits();
        const AsyncGeneratorReturnType = Self::AllowsAsyncIterablesFlag.bits();
        const CacheFlags = Self::AllowsSyncIterablesFlag.bits() | Self::AllowsAsyncIterablesFlag.bits() | Self::ForOfFlag.bits();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct IterationTypes {
    pub yield_type: Option<P<Type>>,
    pub return_type: Option<P<Type>>,
    pub next_type: Option<P<Type>>,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum IterationTypeKind {
    #[default]
    Yield,
    Return,
    Next,
}

/// Go `IterationTypesResolver`. Its function-valued fields are methods taking the checker; `is_async` selects
/// between the two resolvers Go builds in `initializeIterationResolvers`.
pub struct IterationTypesResolver {
    pub is_async: bool,
    pub iterator_symbol_name: &'static str,
    pub must_have_a_next_method_diagnostic: &'static Message,
    pub must_be_a_method_diagnostic: &'static Message,
    pub must_have_a_value_diagnostic: &'static Message,
}

impl IterationTypesResolver {
    pub fn get_global_iterator_type(&self, c: &mut Checker) -> P<Type> {
        if self.is_async { c.get_global_async_iterator_type() } else { c.get_global_iterator_type() }
    }
    pub fn get_global_iterable_type(&self, c: &mut Checker) -> P<Type> {
        if self.is_async { c.get_global_async_iterable_type() } else { c.get_global_iterable_type() }
    }
    pub fn get_global_iterable_type_checked(&self, c: &mut Checker) -> P<Type> {
        if self.is_async { c.get_global_async_iterable_type_checked() } else { c.get_global_iterable_type_checked() }
    }
    pub fn get_global_iterable_iterator_type(&self, c: &mut Checker) -> P<Type> {
        if self.is_async { c.get_global_async_iterable_iterator_type() } else { c.get_global_iterable_iterator_type() }
    }
    pub fn get_global_iterable_iterator_type_checked(&self, c: &mut Checker) -> P<Type> {
        if self.is_async { c.get_global_async_iterable_iterator_type_checked() } else { c.get_global_iterable_iterator_type_checked() }
    }
    pub fn get_global_iterator_object_type(&self, c: &mut Checker) -> P<Type> {
        if self.is_async { c.get_global_async_iterator_object_type() } else { c.get_global_iterator_object_type() }
    }
    pub fn get_global_generator_type(&self, c: &mut Checker) -> P<Type> {
        if self.is_async { c.get_global_async_generator_type() } else { c.get_global_generator_type() }
    }
    pub fn get_global_builtin_iterator_types(&self, c: &mut Checker) -> &'static [P<Type>] {
        if self.is_async {
            if let Some(types) = c.global_builtin_async_iterator_types_cache {
                return types;
            }
            let types = c.get_global_types(&["ReadableStreamAsyncIterator"], 1, false /*reportErrors*/);
            c.global_builtin_async_iterator_types_cache = Some(types);
            types
        } else {
            if let Some(types) = c.global_builtin_iterator_types_cache {
                return types;
            }
            let types = c.get_global_types(&["ArrayIterator", "MapIterator", "SetIterator", "StringIterator"], 1, false /*reportErrors*/);
            c.global_builtin_iterator_types_cache = Some(types);
            types
        }
    }
    pub fn resolve_iteration_type(&self, c: &mut Checker, t: P<Type>, error_node: Option<P<Node>>) -> Option<P<Type>> {
        if self.is_async {
            return c.get_awaited_type_ex(
                t,
                error_node,
                Some(&diagnostics::Type_of_await_operand_must_either_be_a_valid_promise_or_must_not_contain_a_callable_then_member),
                &[],
            );
        }
        Some(t)
    }
}

#[derive(Default)]
pub struct WideningContext {
    pub parent: Cell<Option<P<WideningContext>>>, // Parent context
    pub property_name: Cell<&'static str>, // Name of property in parent
    pub siblings: Cell<Option<&'static [P<Type>]>>, // Types of siblings (nil = not computed)
    pub resolved_properties: Cell<Option<&'static [P<Symbol>]>>, // Properties occurring in sibling object literals (nil = not computed)
    pub child_contexts: GoMap<String, P<WideningContext>>,
    pub widened_types: GoMap<P<Type>, P<Type>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VarianceStackEntry {
    pub symbol: P<Symbol>,
    pub type_parameters: &'static [P<Type>],
}

pub const maxSerializationLevel: i32 = 2;

// Checker

static nextCheckerID: AtomicU32 = AtomicU32::new(0);

/// Go `symbolTableID` (symbolaccessibility.go); the `stKind*` constants live with the printer port.
pub type symbolTableID = u64;

pub struct Checker {
    pub id: u32,
    pub program: &'static dyn Program,
    pub compiler_options: P<CompilerOptions>,
    pub files: &'static [P<SourceFile>],
    pub file_index_map: FxHashMap<P<SourceFile>, i32>,
    pub type_count: u32,
    pub symbol_count: u32,
    pub signature_count: u32,
    pub total_instantiation_count: u32,
    pub instantiation_count: u32,
    /// TSRS_WORK_CENSUS (workcensus.rs); None unless the variable is set.
    pub census: Option<Box<crate::workcensus::Census>>,
    /// TSRS_UNION_CACHE (unioncache.rs; on by default, off under Go-compatible history).
    pub(crate) union_front_cache: crate::unioncache::UnionFrontCache,
    /// TSRS_INFER_MEMO (infermemo.rs; on by default, off under Go-compatible history).
    pub(crate) infer_memo: crate::infermemo::InferMemo,
    /// Calls of `add_diagnostic` and `add_suggestion_diagnostic`, duplicates included (wrapping): the inference memo
    /// stores only walks that added none.
    pub(crate) diagnostic_adds: u32,
    /// tsrs-only: TS2590 reports so far (wrapping; `Checker::too_complex_since`).
    pub(crate) too_complex_reports: u32,
    /// tsrs-only: the TS2590 sites recorded since the last flush, reported when the file being checked is done
    /// (`Checker::flush_too_complex_reports`).
    pub(crate) too_complex_nodes: Vec<P<Node>>,
    /// tsrs-only: the (file, too-complex evaluation) pairs this checker has reported (`Checker::too_complex_key`), so
    /// that a file reports each too-complex type once, at its first site, whichever file's check evaluated it.
    pub(crate) too_complex_reported: rustc_hash::FxHashSet<(P<SourceFile>, u64)>,
    /// Calls of `check_expression_ex`, each of which resets `instantiation_count` (wrapping; the inference memo).
    pub(crate) expression_checks: u32,
    pub instantiation_stack: Vec<P<Type>>,
    pub conditional_constraint_depth: u32,
    pub inline_level: i32,
    pub serialization_level: i32,
    pub current_node: Option<P<Node>>,
    /// tsrs-only: the file `check_source_file` is checking. The one check leaf (`SourceFile::is_check_leaf`) whose tree
    /// this checker may read.
    pub(crate) checking_file: Option<P<SourceFile>>,
    /// tsrs-only: a file whose statements this checker and others already checked in pieces
    /// (`check_source_file_piece`); the next `check_source_file` of it runs only the file-level steps.
    pub(crate) statements_checked_in_pieces: Option<P<SourceFile>>,
    /// tsrs-only: the type parameter and property name of element `i` of every tuple target
    /// (`create_tuple_target_type`).
    pub(crate) tuple_elements: Vec<(P<Type>, &'static str)>,
    /// tsrs-only: type references whose type-argument constraint check was reached while this checker computed a type
    /// in a file it had not started checking (`defer_type_argument_constraints`), by file. Run when this checker checks
    /// the file; dropped with the checker otherwise.
    pub(crate) deferred_type_argument_checks: FxHashMap<P<SourceFile>, Vec<P<Node>>>,
    pub variance_type_parameter: Option<P<Type>>,
    pub language_version: ScriptTarget,
    pub module_kind: ModuleKind,
    pub module_resolution_kind: ModuleResolutionKind,
    pub is_inference_partially_blocked: bool,
    pub legacy_decorators: bool,
    pub emit_standard_class_fields: bool,
    pub strict_null_checks: bool,
    pub strict_function_types: bool,
    pub strict_bind_call_apply: bool,
    pub strict_property_initialization: bool,
    pub strict_builtin_iterator_return: bool,
    pub no_implicit_any: bool,
    pub no_implicit_this: bool,
    pub use_unknown_in_catch_variables: bool,
    pub exact_optional_property_types: bool,
    pub can_collect_symbol_alias_accessibility_data: bool,
    pub was_canceled: bool,
    pub array_variances: &'static [VarianceFlags],
    pub globals: P<SymbolTable>,
    pub string_literal_types: StringLiteralTypes,
    pub number_literal_types: FxHashMap<Number, P<Type>>,
    pub nan_type: Option<P<Type>>,
    pub bigint_literal_types: FxHashMap<PseudoBigInt, P<Type>>,
    pub enum_literal_types: FxHashMap<EnumLiteralKey, P<Type>>,
    pub enum_nan_literal_types: FxHashMap<P<Symbol>, P<Type>>,
    pub indexed_access_types: crate::basedmap::Based<PackedMap<CacheHashKey, P<Type>>>,
    pub template_literal_types: PackedMap<CacheHashKey, P<Type>>,
    pub string_mapping_types: FxHashMap<StringMappingKey, P<Type>>,
    pub unique_es_symbol_types: FxHashMap<P<Symbol>, P<Type>>,
    pub this_expando_kinds: FxHashMap<P<Symbol>, thisAssignmentDeclarationKind>,
    pub this_expando_locations: FxHashMap<P<Symbol>, Option<P<Node>>>,
    pub subtype_reduction_cache: FxHashMap<CacheHashKey, &'static [P<Type>]>,
    pub cached_types: FxHashMap<CachedTypeKey, P<Type>>,
    pub cached_signatures: PackedMap<CachedSignatureKey, P<Signature>>,
    pub undefined_properties: FxHashMap<String, P<Symbol>>,
    // tsrs: the canonical (history-independent) form of `undefined_properties` (get_undefined_property).
    pub undefined_properties_by_prop: FxHashMap<P<Symbol>, P<Symbol>>,
    pub narrowed_types: FxHashMap<NarrowedTypeKey, P<Type>>,
    pub assignment_reduced_types: FxHashMap<AssignmentReducedKey, P<Type>>,
    pub discriminated_contextual_types: FxHashMap<DiscriminatedContextualTypeKey, P<Type>>,
    pub instantiation_expression_types: FxHashMap<InstantiationExpressionKey, P<Type>>,
    pub substitution_types: FxHashMap<SubstitutionTypeKey, P<Type>>,
    pub reverse_mapped_cache: FxHashMap<ReverseMappedTypeKey, Option<P<Type>>>, // Go stores nil results (lookup uses comma-ok)
    pub reverse_homomorphic_mapped_cache: FxHashMap<ReverseMappedTypeKey, P<Type>>,
    pub iteration_types_cache: FxHashMap<IterationTypesKey, IterationTypes>,
    pub marker_types: Set<P<Type>>,
    pub resolving_explicit_type_of_symbol: Set<P<Symbol>>,
    pub undefined_symbol: P<Symbol>,
    pub arguments_symbol: P<Symbol>,
    pub require_symbol: P<Symbol>,
    pub unknown_symbol: P<Symbol>,
    pub unresolved_symbols: FxHashMap<String, P<Symbol>>,
    pub error_types: PackedMap<CacheHashKey, P<Type>>,
    pub module_symbols: FxHashMap<P<Node>, P<Symbol>>,
    /// `Program::get_source_file_for_resolved_module` by the address and length of the resolved file name (the
    /// resolver's string, alive and unchanged as long as the program): `resolve_external_module`.
    pub(crate) resolved_module_source_files: FxHashMap<(usize, usize), Option<P<SourceFile>>>,
    pub global_this_symbol: P<Symbol>,
    pub symbol_table_alias_cache: FxHashMap<symbolTableID, &'static [P<Symbol>]>,
    pub class_expression_name_tables: FxHashMap<NodeId, P<SymbolTable>>,
    /// Go `resolveName = c.createNameResolver().Resolve`; call through `resolve_name`.
    pub name_resolver: P<NameResolver<Checker>>,
    /// Go `resolveNameForSymbolSuggestion`; call through `resolve_name_for_symbol_suggestion`.
    pub name_resolver_for_suggestion: P<NameResolver<Checker>>,
    pub tuple_types: crate::basedmap::Based<PackedMap<CacheHashKey, P<Type>>>,
    pub union_types: crate::basedmap::Based<PackedMap<CacheHashKey, P<Type>>>,
    pub union_of_union_types: crate::basedmap::Based<FxHashMap<UnionOfUnionKey, P<Type>>, UnionOfUnionKey>,
    pub intersection_types: crate::basedmap::Based<PackedMap<CacheHashKey, P<Type>>>,
    pub properties_types: FxHashMap<PropertiesTypesKey, P<Type>>,
    pub diagnostics: ast::DiagnosticsCollection,
    pub suggestion_diagnostics: ast::DiagnosticsCollection,
    pub merged_symbols: FxHashMap<P<Symbol>, P<Symbol>>,
    pub factory: ast::NodeFactory,
    pub node_links: LinkStore<Node, NodeLinks>,
    pub signature_links: KeyedLinkStore<Node, SignatureLinks>,
    pub symbol_node_links: NodeLinkStore<SymbolNodeLinks>,
    pub type_node_links: KeyedLinkStore<Node, TypeNodeLinks>,
    pub enum_member_links: LinkStore<Node, EnumMemberLinks>,
    pub assertion_links: LinkStore<Node, AssertionLinks>,
    pub array_literal_links: LinkStore<Node, ArrayLiteralLinks>,
    pub switch_statement_links: LinkStore<Node, SwitchStatementLinks>,
    pub jsx_element_links: LinkStore<Node, JsxElementLinks>,
    pub computed_name_links: LinkStore<Node, ComputedNameNodeLinks>,
    pub symbol_reference_links: SymbolReferenceLinkStore,
    pub value_symbol_links: ValueSymbolLinkStore<ValueSymbolLinks>,
    /// tsrs-only: the dense rows of this checker's transient symbols (notes/dod-semantic-tables.md).
    pub transient: ast::TransientSymbols,
    pub mapped_symbol_links: LinkStore<Symbol, MappedSymbolLinks>,
    pub deferred_symbol_links: LinkStore<Symbol, DeferredSymbolLinks>,
    pub alias_symbol_links: LinkStore<Symbol, AliasSymbolLinks>,
    pub module_symbol_links: LinkStore<Symbol, ModuleSymbolLinks>,
    pub late_bound_links: LinkStore<Symbol, LateBoundLinks>,
    pub export_type_links: LinkStore<Symbol, ExportTypeLinks>,
    pub members_and_exports_links: LinkStore<Symbol, MembersAndExportsLinks>,
    pub type_alias_links: LinkStore<Symbol, TypeAliasLinks>,
    pub declared_type_links: LinkStore<Symbol, DeclaredTypeLinks>,
    pub spread_links: LinkStore<Symbol, SpreadLinks>,
    pub variance_links: LinkStore<Symbol, VarianceLinks>,
    pub reverse_mapped_symbol_links: LinkStore<Symbol, ReverseMappedSymbolLinks>,
    pub marked_assignment_symbol_links: LinkStore<Symbol, MarkedAssignmentSymbolLinks>,
    pub symbol_container_links: LinkStore<Symbol, ContainingSymbolLinks>,
    pub source_file_links: LinkStore<SourceFile, SourceFileLinks>,
    pub reg_exp_scanner: Option<Box<Scanner>>,
    pub pattern_for_type: FxHashMap<P<Type>, P<Node>>,
    pub(crate) lazy_member_tables: crate::basedmap::Based<FxHashMap<P<Type>, P<LazyMemberTable>>, P<Type>>,
    // Go `StructuredType.objectTypeWithoutAbstractConstructSignatures`: set only by the node builder, for few types,
    // so kept here instead of in every object, union and intersection type.
    pub(crate) object_types_without_abstract_construct_signatures: FxHashMap<P<Type>, P<Type>>,
    // Go `StructuredType.resolvedBaseConstraint` (see `resolved_base_constraint_of`).
    pub(crate) structured_type_base_constraints: FxHashMap<P<Type>, P<Type>>,
    // Go `ObjectType.instantiations` of object types that are not interfaces or tuples (see `ObjectType`).
    pub(crate) object_type_instantiations: crate::basedmap::Based<FxHashMap<P<Type>, PackedMap<CacheHashKey, P<Type>>>, P<Type>>,
    pub(crate) lazy_mapped_tables: crate::basedmap::Based<FxHashMap<P<Type>, std::rc::Rc<LazyMappedTable>>, P<Type>>,
    /// tsrs_core::lazymembers::enabled() (default on; `--noLazyMembers` / `TSRS_LAZY_MEMBERS=0`): no lazy table is created when false.
    pub lazy_members: bool,
    /// notes/mem-lazy.md L1: tuple references get lazy member tables too.
    pub lazy_tuples: bool,
    /// site-counts profile: creation site of each instantiated symbol until its type is resolved.
    #[cfg(feature = "site-counts")]
    pub inst_symbol_sites: FxHashMap<P<Symbol>, &'static std::panic::Location<'static>>,
    /// notes/mem-lazy.md L5: conditional type instantiation without the composite mapper.
    pub lazy_cond_mapper: bool,
    /// notes/mem-lazy.md L6: union/intersection property caches without the eager copy.
    pub lazy_prop_cache: bool,
    /// notes/mem-lazy.md L9: existence-only property queries in getUnmatchedProperties.
    pub lazy_has_prop: bool,
    /// notes/mem-lazy.md L10: getUnmatchedProperties walks lazy targets.
    pub lazy_unmatched: bool,
    /// notes/mem-lazy.md L11: empty-object tests on lazy tables.
    pub lazy_empty: bool,
    pub lazy_member_stats: tsrs_core::lazymembers::LazyMemberStats,
    /// The member names of `Function`, `CallableFunction`, `NewableFunction` and `Object`, once all four are
    /// resolved (`may_be_augment_member`).
    pub(crate) augment_filter: Option<tsrs_ast::NameFilter>,
    /// Instrumentation (feature `assignment-stats`): every type / symbol this checker created.
    #[cfg(feature = "assignment-stats")]
    pub stats_created: (Vec<P<Type>>, Vec<P<Symbol>>),
    pub context_free_types: FxHashMap<P<Node>, P<Type>>,
    pub any_type: P<Type>,
    pub auto_type: P<Type>,
    pub wildcard_type: P<Type>,
    pub blocked_string_type: P<Type>,
    pub error_type: P<Type>,
    pub unresolved_type: P<Type>,
    pub non_inferrable_any_type: P<Type>,
    pub intrinsic_marker_type: P<Type>,
    pub unknown_type: P<Type>,
    pub undefined_type: P<Type>,
    pub undefined_widening_type: P<Type>,
    pub missing_type: P<Type>,
    pub undefined_or_missing_type: P<Type>,
    pub optional_type: P<Type>,
    pub null_type: P<Type>,
    pub null_widening_type: P<Type>,
    pub string_type: P<Type>,
    pub number_type: P<Type>,
    pub bigint_type: P<Type>,
    pub regular_false_type: P<Type>,
    pub false_type: P<Type>,
    pub regular_true_type: P<Type>,
    pub true_type: P<Type>,
    pub boolean_type: P<Type>,
    pub es_symbol_type: P<Type>,
    pub void_type: P<Type>,
    pub never_type: P<Type>,
    pub silent_never_type: P<Type>,
    pub implicit_never_type: P<Type>,
    pub unreachable_never_type: P<Type>,
    pub non_primitive_type: P<Type>,
    pub string_or_number_type: P<Type>,
    pub string_number_symbol_type: P<Type>,
    pub number_or_big_int_type: P<Type>,
    pub template_constraint_type: P<Type>,
    pub numeric_string_type: P<Type>,
    pub unique_literal_type: P<Type>,
    pub unique_literal_mapper: P<TypeMapper>,
    pub reliability_flags: RelationComparisonResult,
    pub report_unreliable_mapper: P<TypeMapper>,
    pub report_unmeasurable_mapper: P<TypeMapper>,
    pub restrictive_mapper: P<TypeMapper>,
    pub permissive_mapper: P<TypeMapper>,
    pub empty_object_type: P<Type>,
    pub empty_jsx_object_type: P<Type>,
    pub empty_fresh_jsx_object_type: P<Type>,
    pub empty_type_literal_type: P<Type>,
    pub unknown_empty_object_type: P<Type>,
    pub unknown_union_type: P<Type>,
    pub empty_generic_type: P<Type>,
    pub any_function_type: P<Type>,
    pub no_constraint_type: P<Type>,
    pub circular_constraint_type: P<Type>,
    pub resolving_default_type: P<Type>,
    pub marker_super_type: P<Type>,
    pub marker_sub_type: P<Type>,
    pub marker_other_type: P<Type>,
    pub marker_super_type_for_check: P<Type>,
    pub marker_sub_type_for_check: P<Type>,
    pub no_type_predicate: P<TypePredicate>,
    pub any_signature: P<Signature>,
    pub unknown_signature: P<Signature>,
    pub resolving_signature: P<Signature>,
    pub silent_never_signature: P<Signature>,
    pub cached_arguments_referenced: FxHashMap<P<Node>, bool>,
    pub enum_number_index_info: P<IndexInfo>,
    pub any_base_type_index_info: P<IndexInfo>,
    pub pattern_ambient_modules: Vec<P<ast::PatternAmbientModule>>,
    pub pattern_ambient_module_augmentations: Option<P<SymbolTable>>,
    pub pattern_ambient_module_augmentation_targets: Option<P<SymbolTable>>,
    pub module_import_attributes_types: FxHashMap<P<Symbol>, P<Type>>,
    pub global_object_type: P<Type>,
    pub global_function_type: P<Type>,
    pub global_callable_function_type: P<Type>,
    pub global_newable_function_type: P<Type>,
    pub global_array_type: P<Type>,
    pub global_readonly_array_type: P<Type>,
    pub global_string_type: P<Type>,
    pub global_number_type: P<Type>,
    pub global_boolean_type: P<Type>,
    pub global_reg_exp_type: P<Type>,
    pub global_this_type: P<Type>,
    pub any_array_type: P<Type>,
    pub auto_array_type: P<Type>,
    pub any_readonly_array_type: P<Type>,
    pub deferred_global_import_meta_expression_type: Option<P<Type>>,
    pub contextual_binding_patterns: Vec<P<Node>>,
    pub empty_string_type: P<Type>,
    pub zero_type: P<Type>,
    pub zero_big_int_type: P<Type>,
    pub typeof_type: P<Type>,
    pub type_resolutions: Vec<TypeResolution>,
    // Canonical base-type cycles (`canonicalize_base_type_cycles`): first resolutions of base types in progress, the
    // types they resolved, and the ones found circular.
    pub base_types_depth: u32,
    pub base_types_resolved_log: Vec<P<Type>>,
    pub base_types_circular: Vec<P<Type>>,
    pub resolution_start: i32,
    pub variance_stack: Vec<VarianceStackEntry>,
    pub call_resolution_stack: Vec<P<Node>>,
    pub apparent_argument_count: Option<i32>,
    pub last_get_combined_node_flags_node: Option<P<Node>>,
    pub last_get_combined_node_flags_result: NodeFlags,
    pub last_get_combined_modifier_flags_node: Option<P<Node>>,
    pub last_get_combined_modifier_flags_result: ModifierFlags,
    pub freeinference_state: Option<P<InferenceState>>,
    pub free_flow_state: Option<P<FlowState>>,
    pub flow_loop_cache: FxHashMap<FlowLoopKey, P<Type>>,
    pub flow_loop_stack: Vec<FlowLoopInfo>,
    pub shared_flows: Vec<SharedFlow>,
    pub antecedent_types: Vec<P<Type>>,
    pub flow_analysis_disabled: bool,
    pub flow_invocation_count: i32,
    pub flow_type_cache: Option<FxHashMap<P<Node>, P<Type>>>, // Go nil map, saved/restored as a whole
    /// The flow memo (flowmemo.rs). Boxed: the checker's hot fields keep their offsets.
    pub flow_memo: Box<crate::flowmemo::FlowMemo>,
    pub last_flow_node: Option<P<FlowNode>>,
    pub last_flow_node_reachable: bool,
    pub flow_node_reachable: FxHashMap<P<FlowNode>, bool>,
    pub flow_node_post_super: FxHashMap<P<FlowNode>, bool>,
    pub renamed_binding_elements_in_types: Vec<P<Node>>,
    pub contextual_infos: Vec<ContextualInfo>,
    pub inference_context_infos: Vec<InferenceContextInfo>,
    pub awaited_type_stack: Vec<P<Type>>,
    pub reverse_mapped_source_stack: Vec<P<Type>>,
    pub reverse_mapped_target_stack: Vec<P<Type>>,
    pub reverse_expanding_flags: ExpandingFlags,
    pub free_relater: Option<P<Relater>>,
    pub subtype_relation: P<Relation>,
    pub strict_subtype_relation: P<Relation>,
    pub assignable_relation: P<Relation>,
    pub comparable_relation: P<Relation>,
    pub identity_relation: P<Relation>,
    pub enum_relation: FxHashMap<EnumRelationKey, RelationComparisonResult>,
    pub get_global_es_symbol_type_cache: Option<P<Type>>,
    pub get_global_big_int_type_cache: Option<P<Type>>,
    pub get_global_import_meta_type_cache: Option<P<Type>>,
    pub get_global_import_attributes_type_cache: Option<P<Type>>,
    pub get_global_import_attributes_type_checked_cache: Option<P<Type>>,
    pub get_global_non_nullable_type_alias_or_nil_cache: Option<Option<P<Symbol>>>,
    pub get_global_extract_symbol_cache: Option<Option<P<Symbol>>>,
    pub get_global_disposable_type_cache: Option<P<Type>>,
    pub get_global_async_disposable_type_cache: Option<P<Type>>,
    pub get_global_awaited_symbol_cache: Option<Option<P<Symbol>>>,
    pub get_global_awaited_symbol_or_nil_cache: Option<Option<P<Symbol>>>,
    pub get_global_nan_symbol_or_nil_cache: Option<Option<P<Symbol>>>,
    pub get_global_record_symbol_cache: Option<Option<P<Symbol>>>,
    pub get_global_template_strings_array_type_cache: Option<P<Type>>,
    pub get_global_es_symbol_constructor_symbol_or_nil_cache: Option<Option<P<Symbol>>>,
    pub get_global_es_symbol_constructor_type_symbol_or_nil_cache: Option<Option<P<Symbol>>>,
    pub get_global_import_call_options_type_cache: Option<P<Type>>,
    pub get_global_import_call_options_type_checked_cache: Option<P<Type>>,
    pub get_global_promise_type_cache: Option<P<Type>>,
    pub get_global_promise_type_checked_cache: Option<P<Type>>,
    pub get_global_promise_like_type_cache: Option<P<Type>>,
    pub get_global_promise_constructor_symbol_cache: Option<Option<P<Symbol>>>,
    pub get_global_promise_constructor_symbol_or_nil_cache: Option<Option<P<Symbol>>>,
    pub get_global_omit_symbol_cache: Option<Option<P<Symbol>>>,
    pub get_global_iterator_type_cache: Option<P<Type>>,
    pub get_global_iterable_type_cache: Option<P<Type>>,
    pub get_global_iterable_type_checked_cache: Option<P<Type>>,
    pub get_global_iterable_iterator_type_cache: Option<P<Type>>,
    pub get_global_iterable_iterator_type_checked_cache: Option<P<Type>>,
    pub get_global_iterator_object_type_cache: Option<P<Type>>,
    pub get_global_generator_type_cache: Option<P<Type>>,
    pub get_global_async_iterator_type_cache: Option<P<Type>>,
    pub get_global_async_iterable_type_cache: Option<P<Type>>,
    pub get_global_async_iterable_type_checked_cache: Option<P<Type>>,
    pub get_global_async_iterable_iterator_type_cache: Option<P<Type>>,
    pub get_global_async_iterable_iterator_type_checked_cache: Option<P<Type>>,
    pub get_global_async_iterator_object_type_cache: Option<P<Type>>,
    pub get_global_async_generator_type_cache: Option<P<Type>>,
    pub get_global_iterator_yield_result_type_cache: Option<P<Type>>,
    pub get_global_iterator_return_result_type_cache: Option<P<Type>>,
    pub get_global_typed_property_descriptor_type_cache: Option<P<Type>>,
    pub get_global_class_decorator_context_type_cache: Option<P<Type>>,
    pub get_global_class_method_decorator_context_type_cache: Option<P<Type>>,
    pub get_global_class_getter_decorator_context_type_cache: Option<P<Type>>,
    pub get_global_class_setter_decorator_context_type_cache: Option<P<Type>>,
    pub get_global_class_accessor_decorator_context_type_cache: Option<P<Type>>,
    pub get_global_class_accessor_decorator_target_type_cache: Option<P<Type>>,
    pub get_global_class_accessor_decorator_result_type_cache: Option<P<Type>>,
    pub get_global_class_field_decorator_context_type_cache: Option<P<Type>>,
    pub global_builtin_iterator_types_cache: Option<&'static [P<Type>]>,
    pub global_builtin_async_iterator_types_cache: Option<&'static [P<Type>]>,
    pub sync_iteration_types_resolver: P<IterationTypesResolver>,
    pub async_iteration_types_resolver: P<IterationTypesResolver>,
    pub _jsx_namespace: String,
    pub _jsx_factory_entity: Option<P<Node>>,
    pub skip_direct_inference_nodes: Set<P<Node>>,
    pub ctx: Option<Context>, // Go nil until checkSourceFile
    pub active_mappers: Vec<P<TypeMapper>>,
    pub active_type_mappers_caches: Vec<PackedMap<CacheHashKey, P<Type>>>,
    // Mappers and inference contexts `getConditionalType` made, recycled when it returns (notes/mem-recycle.md).
    pub scratch_mappers: Vec<P<TypeMapper>>,
    pub scratch_contexts: Vec<P<InferenceContext>>,
    /// `getTailRecursionRoot` mappers with the type-argument lists made for them, recycled with `scratch_mappers`.
    pub scratch_mapper_lists: Vec<(P<TypeMapper>, &'static [P<Type>])>,
    pub free_type_mapper_caches: Vec<PackedMap<CacheHashKey, P<Type>>>, // Rust-only: cleared maps for reuse (Go keeps them in the slice capacity)
    pub free_type_lists: Vec<Vec<P<Type>>>, // Rust-only: empty buffers for `instantiate_types_changed`
    pub ambient_modules_once: bool, // Go sync.Once: true once ambient_modules has been computed
    pub ambient_modules: Vec<P<Symbol>>,
    pub within_unreachable_code: bool,
    pub reported_unreachable_nodes: Set<P<Node>>,
    pub non_existent_properties: Set<NonExistentPropertyKey>,
    pub deferred_diagnostic_callbacks: Vec<Box<dyn FnOnce(&mut Checker)>>,
    /// tsrs-only: export tables indexed by resolved target, for `get_alias_for_symbol_in_container` (printer.rs).
    pub exports_by_target_index: FxHashMap<P<SymbolTable>, crate::printer::ExportsByTarget>,
    /// tsrs-only: the external modules by what they export, for `get_alternative_containing_modules` (printer.rs).
    pub module_export_index: crate::printer::ModuleExportIndex,
    /// tsrs-only: the caches above are bypassed while this is nonzero: one per `resolve_alias` and module export
    /// table computation in progress, plus one until `initialize_checker` has merged the global and augmentation
    /// symbol tables.
    pub alias_cache_blockers: u32,
    /// The placeholder that `P<Type>` fields hold until Go would assign them (Go nil). Compare against it where Go
    /// tests such a field against nil (`c.globalObjectType != nil`).
    pub unassigned_type: P<Type>,
    pub type_to_string_nodebuilder: Option<P<NodeBuilder>>,
    /// tsrs-only: accessible-chain cache entries keyed by a node in the current emit scratch region
    /// (`forget_scratch_keyed_caches`, notes/mem-emit-regions.md).
    pub scratch_keyed_chain_cache: Vec<(P<Symbol>, accessibleChainCacheKey)>,
    pub emit_resolver: Option<P<EmitResolver>>, // Go `emitResolver` + `emitResolverOnce`: None until `get_emit_resolver`
    /// Shared graph (`tsrs_core::sharedgraph`): this checker's values for lazy fields of frozen objects.
    pub overlay: Box<tsrs_core::sharedgraph::Overlay>,
    /// Shared graph: this checker is the seed whose graph will be frozen (symbol ids are assigned eagerly).
    pub seed_mode: bool,
    /// Shared graph: made by `fork` from the frozen seed.
    pub is_fork: bool,
}

// checker.go:911
/// Go `NewChecker(program, tracer)`. The tracer and the returned mutex are not ported.
pub fn new_checker(program: &'static dyn Program) -> Box<Checker> {
    program.bind_source_files();
    crate::types::census_layouts();

    // Placeholders for pointer fields Go leaves nil until they are assigned below.
    let compiler_options = program.options();
    let dummy_type = Type::alloc(TypeFlags::None, ObjectFlags::None, TypeId(0), IntrinsicType::default());
    let dummy_symbol = P::new(Symbol::default());
    let dummy_mapper = new_simple_type_mapper(dummy_type, dummy_type);
    let dummy_signature = P::new(Signature::default());
    let dummy_index_info = P::new(IndexInfo::default());
    let dummy_resolver = P::new(NameResolver::<Checker>::new(compiler_options, None));
    let dummy_iteration_resolver = P::new(IterationTypesResolver {
        is_async: false,
        iterator_symbol_name: "",
        must_have_a_next_method_diagnostic: &diagnostics::An_iterator_must_have_a_next_method,
        must_be_a_method_diagnostic: &diagnostics::The_0_property_of_an_iterator_must_be_a_method,
        must_have_a_value_diagnostic: &diagnostics::The_type_returned_by_the_0_method_of_an_iterator_must_have_a_value_property,
    });

    let files = program.source_files();
    // Relaxed: the counter only hands out distinct ids.
    let id = nextCheckerID.fetch_add(1, Ordering::Relaxed) + 1;
    let transient = ast::TransientSymbols::new(id);
    let mut c = Box::new(Checker {
        id,
        program,
        compiler_options,
        files,
        file_index_map: create_file_index_map(files),
        type_count: 0,
        symbol_count: 0,
        signature_count: 0,
        total_instantiation_count: 0,
        instantiation_count: 0,
        census: crate::workcensus::census_path().map(|_| crate::workcensus::Census::new()),
        union_front_cache: crate::unioncache::UnionFrontCache::new(),
        infer_memo: crate::infermemo::InferMemo::new(),
        diagnostic_adds: 0,
        too_complex_reports: 0,
        too_complex_nodes: Vec::new(),
        too_complex_reported: rustc_hash::FxHashSet::default(),
        expression_checks: 0,
        instantiation_stack: Vec::new(),
        conditional_constraint_depth: 0,
        inline_level: 0,
        serialization_level: 0,
        current_node: None,
        checking_file: None,
        statements_checked_in_pieces: None,
        tuple_elements: Vec::new(),
        deferred_type_argument_checks: FxHashMap::default(),
        variance_type_parameter: None,
        language_version: compiler_options.get_emit_script_target(),
        module_kind: compiler_options.get_emit_module_kind(),
        module_resolution_kind: compiler_options.get_module_resolution_kind(),
        is_inference_partially_blocked: false,
        legacy_decorators: compiler_options.experimental_decorators == Tristate::True,
        emit_standard_class_fields: compiler_options.get_emit_standard_class_fields(),
        strict_null_checks: compiler_options.get_strict_option_value(compiler_options.strict_null_checks),
        strict_function_types: compiler_options.get_strict_option_value(compiler_options.strict_function_types),
        strict_bind_call_apply: compiler_options.get_strict_option_value(compiler_options.strict_bind_call_apply),
        strict_property_initialization: compiler_options.get_strict_option_value(compiler_options.strict_property_initialization),
        strict_builtin_iterator_return: compiler_options.get_strict_option_value(compiler_options.strict_builtin_iterator_return),
        no_implicit_any: compiler_options.get_strict_option_value(compiler_options.no_implicit_any),
        no_implicit_this: compiler_options.get_strict_option_value(compiler_options.no_implicit_this),
        use_unknown_in_catch_variables: compiler_options.get_strict_option_value(compiler_options.use_unknown_in_catch_variables),
        exact_optional_property_types: compiler_options.exact_optional_property_types == Tristate::True,
        can_collect_symbol_alias_accessibility_data: compiler_options.verbatim_module_syntax.is_false_or_unknown(),
        was_canceled: false,
        array_variances: alloc_slice(&[VarianceFlags::Covariant]),
        globals: SymbolTable::with_capacity(count_global_symbols(files) as usize),
        string_literal_types: StringLiteralTypes::default(),
        number_literal_types: FxHashMap::default(),
        nan_type: None,
        bigint_literal_types: FxHashMap::default(),
        enum_literal_types: FxHashMap::default(),
        enum_nan_literal_types: FxHashMap::default(),
        indexed_access_types: Default::default(),
        template_literal_types: PackedMap::default(),
        string_mapping_types: FxHashMap::default(),
        unique_es_symbol_types: FxHashMap::default(),
        this_expando_kinds: FxHashMap::default(),
        this_expando_locations: FxHashMap::default(),
        subtype_reduction_cache: FxHashMap::default(),
        cached_types: FxHashMap::default(),
        cached_signatures: PackedMap::default(),
        undefined_properties: FxHashMap::default(),
        undefined_properties_by_prop: FxHashMap::default(),
        narrowed_types: FxHashMap::default(),
        assignment_reduced_types: FxHashMap::default(),
        discriminated_contextual_types: FxHashMap::default(),
        instantiation_expression_types: FxHashMap::default(),
        substitution_types: FxHashMap::default(),
        reverse_mapped_cache: FxHashMap::default(),
        reverse_homomorphic_mapped_cache: FxHashMap::default(),
        iteration_types_cache: FxHashMap::default(),
        marker_types: Set::new(),
        resolving_explicit_type_of_symbol: Set::new(),
        undefined_symbol: dummy_symbol,
        arguments_symbol: dummy_symbol,
        require_symbol: dummy_symbol,
        unknown_symbol: dummy_symbol,
        unresolved_symbols: FxHashMap::default(),
        error_types: PackedMap::default(),
        module_symbols: FxHashMap::default(),
        resolved_module_source_files: FxHashMap::default(),
        global_this_symbol: dummy_symbol,
        symbol_table_alias_cache: FxHashMap::default(),
        class_expression_name_tables: FxHashMap::default(),
        name_resolver: dummy_resolver,
        name_resolver_for_suggestion: dummy_resolver,
        tuple_types: Default::default(),
        union_types: Default::default(),
        union_of_union_types: Default::default(),
        intersection_types: Default::default(),
        properties_types: FxHashMap::default(),
        diagnostics: ast::DiagnosticsCollection::default(),
        suggestion_diagnostics: ast::DiagnosticsCollection::default(),
        merged_symbols: FxHashMap::default(),
        factory: ast::NodeFactory::default(),
        node_links: LinkStore::default(),
        signature_links: KeyedLinkStore::default(),
        symbol_node_links: NodeLinkStore::default(),
        type_node_links: KeyedLinkStore::default(),
        enum_member_links: LinkStore::default(),
        assertion_links: LinkStore::default(),
        array_literal_links: LinkStore::default(),
        switch_statement_links: LinkStore::default(),
        jsx_element_links: LinkStore::default(),
        computed_name_links: LinkStore::default(),
        symbol_reference_links: SymbolReferenceLinkStore::default(),
        value_symbol_links: ValueSymbolLinkStore::new(transient.owner()),
        transient,
        mapped_symbol_links: LinkStore::default(),
        deferred_symbol_links: LinkStore::default(),
        alias_symbol_links: LinkStore::default(),
        module_symbol_links: LinkStore::default(),
        late_bound_links: LinkStore::default(),
        export_type_links: LinkStore::default(),
        members_and_exports_links: LinkStore::default(),
        type_alias_links: LinkStore::default(),
        declared_type_links: LinkStore::default(),
        spread_links: LinkStore::default(),
        variance_links: LinkStore::default(),
        reverse_mapped_symbol_links: LinkStore::default(),
        marked_assignment_symbol_links: LinkStore::default(),
        symbol_container_links: LinkStore::default(),
        source_file_links: LinkStore::default(),
        reg_exp_scanner: None,
        pattern_for_type: FxHashMap::default(),
        lazy_member_tables: Default::default(),
        object_types_without_abstract_construct_signatures: FxHashMap::default(),
        structured_type_base_constraints: FxHashMap::default(),
        object_type_instantiations: Default::default(),
        lazy_mapped_tables: Default::default(),
        lazy_members: tsrs_core::lazymembers::enabled(),
        lazy_tuples: tsrs_core::lazymembers::lazy_tuples(),
        #[cfg(feature = "site-counts")]
        inst_symbol_sites: FxHashMap::default(),
        lazy_cond_mapper: tsrs_core::lazymembers::lazy_cond_mapper(),
        lazy_prop_cache: tsrs_core::lazymembers::lazy_prop_cache(),
        lazy_has_prop: tsrs_core::lazymembers::lazy_has_prop(),
        lazy_unmatched: tsrs_core::lazymembers::lazy_unmatched(),
        lazy_empty: tsrs_core::lazymembers::lazy_empty(),
        lazy_member_stats: Default::default(),
        augment_filter: None,
        #[cfg(feature = "assignment-stats")]
        stats_created: Default::default(),
        context_free_types: FxHashMap::default(),
        any_type: dummy_type,
        auto_type: dummy_type,
        wildcard_type: dummy_type,
        blocked_string_type: dummy_type,
        error_type: dummy_type,
        unresolved_type: dummy_type,
        non_inferrable_any_type: dummy_type,
        intrinsic_marker_type: dummy_type,
        unknown_type: dummy_type,
        undefined_type: dummy_type,
        undefined_widening_type: dummy_type,
        missing_type: dummy_type,
        undefined_or_missing_type: dummy_type,
        optional_type: dummy_type,
        null_type: dummy_type,
        null_widening_type: dummy_type,
        string_type: dummy_type,
        number_type: dummy_type,
        bigint_type: dummy_type,
        regular_false_type: dummy_type,
        false_type: dummy_type,
        regular_true_type: dummy_type,
        true_type: dummy_type,
        boolean_type: dummy_type,
        es_symbol_type: dummy_type,
        void_type: dummy_type,
        never_type: dummy_type,
        silent_never_type: dummy_type,
        implicit_never_type: dummy_type,
        unreachable_never_type: dummy_type,
        non_primitive_type: dummy_type,
        string_or_number_type: dummy_type,
        string_number_symbol_type: dummy_type,
        number_or_big_int_type: dummy_type,
        template_constraint_type: dummy_type,
        numeric_string_type: dummy_type,
        unique_literal_type: dummy_type,
        unique_literal_mapper: dummy_mapper,
        reliability_flags: RelationComparisonResult::None,
        report_unreliable_mapper: dummy_mapper,
        report_unmeasurable_mapper: dummy_mapper,
        restrictive_mapper: dummy_mapper,
        permissive_mapper: dummy_mapper,
        empty_object_type: dummy_type,
        empty_jsx_object_type: dummy_type,
        empty_fresh_jsx_object_type: dummy_type,
        empty_type_literal_type: dummy_type,
        unknown_empty_object_type: dummy_type,
        unknown_union_type: dummy_type,
        empty_generic_type: dummy_type,
        any_function_type: dummy_type,
        no_constraint_type: dummy_type,
        circular_constraint_type: dummy_type,
        resolving_default_type: dummy_type,
        marker_super_type: dummy_type,
        marker_sub_type: dummy_type,
        marker_other_type: dummy_type,
        marker_super_type_for_check: dummy_type,
        marker_sub_type_for_check: dummy_type,
        no_type_predicate: P::new(TypePredicate::default()),
        any_signature: dummy_signature,
        unknown_signature: dummy_signature,
        resolving_signature: dummy_signature,
        silent_never_signature: dummy_signature,
        cached_arguments_referenced: FxHashMap::default(),
        enum_number_index_info: dummy_index_info,
        any_base_type_index_info: dummy_index_info,
        pattern_ambient_modules: Vec::new(),
        pattern_ambient_module_augmentations: None,
        pattern_ambient_module_augmentation_targets: None,
        module_import_attributes_types: FxHashMap::default(),
        global_object_type: dummy_type,
        global_function_type: dummy_type,
        global_callable_function_type: dummy_type,
        global_newable_function_type: dummy_type,
        global_array_type: dummy_type,
        global_readonly_array_type: dummy_type,
        global_string_type: dummy_type,
        global_number_type: dummy_type,
        global_boolean_type: dummy_type,
        global_reg_exp_type: dummy_type,
        global_this_type: dummy_type,
        any_array_type: dummy_type,
        auto_array_type: dummy_type,
        any_readonly_array_type: dummy_type,
        deferred_global_import_meta_expression_type: None,
        contextual_binding_patterns: Vec::new(),
        empty_string_type: dummy_type,
        zero_type: dummy_type,
        zero_big_int_type: dummy_type,
        typeof_type: dummy_type,
        type_resolutions: Vec::new(),
        base_types_depth: 0,
        base_types_resolved_log: Vec::new(),
        base_types_circular: Vec::new(),
        resolution_start: 0,
        variance_stack: Vec::new(),
        call_resolution_stack: Vec::new(),
        apparent_argument_count: None,
        last_get_combined_node_flags_node: None,
        last_get_combined_node_flags_result: NodeFlags::None,
        last_get_combined_modifier_flags_node: None,
        last_get_combined_modifier_flags_result: ModifierFlags::None,
        freeinference_state: None,
        free_flow_state: None,
        flow_loop_cache: FxHashMap::default(),
        flow_loop_stack: Vec::new(),
        shared_flows: Vec::new(),
        antecedent_types: Vec::new(),
        flow_analysis_disabled: false,
        flow_invocation_count: 0,
        flow_type_cache: None,
        flow_memo: Box::new(crate::flowmemo::FlowMemo::new()),
        last_flow_node: None,
        last_flow_node_reachable: false,
        flow_node_reachable: FxHashMap::default(),
        flow_node_post_super: FxHashMap::default(),
        renamed_binding_elements_in_types: Vec::new(),
        contextual_infos: Vec::new(),
        inference_context_infos: Vec::new(),
        awaited_type_stack: Vec::new(),
        reverse_mapped_source_stack: Vec::new(),
        reverse_mapped_target_stack: Vec::new(),
        reverse_expanding_flags: ExpandingFlags::None,
        free_relater: None,
        subtype_relation: P::new(Relation::default()),
        strict_subtype_relation: P::new(Relation::default()),
        assignable_relation: P::new(Relation::default()),
        comparable_relation: P::new(Relation::default()),
        identity_relation: P::new(Relation::default()),
        enum_relation: FxHashMap::default(),
        get_global_es_symbol_type_cache: None,
        get_global_big_int_type_cache: None,
        get_global_import_meta_type_cache: None,
        get_global_import_attributes_type_cache: None,
        get_global_import_attributes_type_checked_cache: None,
        get_global_non_nullable_type_alias_or_nil_cache: None,
        get_global_extract_symbol_cache: None,
        get_global_disposable_type_cache: None,
        get_global_async_disposable_type_cache: None,
        get_global_awaited_symbol_cache: None,
        get_global_awaited_symbol_or_nil_cache: None,
        get_global_nan_symbol_or_nil_cache: None,
        get_global_record_symbol_cache: None,
        get_global_template_strings_array_type_cache: None,
        get_global_es_symbol_constructor_symbol_or_nil_cache: None,
        get_global_es_symbol_constructor_type_symbol_or_nil_cache: None,
        get_global_import_call_options_type_cache: None,
        get_global_import_call_options_type_checked_cache: None,
        get_global_promise_type_cache: None,
        get_global_promise_type_checked_cache: None,
        get_global_promise_like_type_cache: None,
        get_global_promise_constructor_symbol_cache: None,
        get_global_promise_constructor_symbol_or_nil_cache: None,
        get_global_omit_symbol_cache: None,
        get_global_iterator_type_cache: None,
        get_global_iterable_type_cache: None,
        get_global_iterable_type_checked_cache: None,
        get_global_iterable_iterator_type_cache: None,
        get_global_iterable_iterator_type_checked_cache: None,
        get_global_iterator_object_type_cache: None,
        get_global_generator_type_cache: None,
        get_global_async_iterator_type_cache: None,
        get_global_async_iterable_type_cache: None,
        get_global_async_iterable_type_checked_cache: None,
        get_global_async_iterable_iterator_type_cache: None,
        get_global_async_iterable_iterator_type_checked_cache: None,
        get_global_async_iterator_object_type_cache: None,
        get_global_async_generator_type_cache: None,
        get_global_iterator_yield_result_type_cache: None,
        get_global_iterator_return_result_type_cache: None,
        get_global_typed_property_descriptor_type_cache: None,
        get_global_class_decorator_context_type_cache: None,
        get_global_class_method_decorator_context_type_cache: None,
        get_global_class_getter_decorator_context_type_cache: None,
        get_global_class_setter_decorator_context_type_cache: None,
        get_global_class_accessor_decorator_context_type_cache: None,
        get_global_class_accessor_decorator_target_type_cache: None,
        get_global_class_accessor_decorator_result_type_cache: None,
        get_global_class_field_decorator_context_type_cache: None,
        global_builtin_iterator_types_cache: None,
        global_builtin_async_iterator_types_cache: None,
        sync_iteration_types_resolver: dummy_iteration_resolver,
        async_iteration_types_resolver: dummy_iteration_resolver,
        _jsx_namespace: String::new(),
        _jsx_factory_entity: None,
        skip_direct_inference_nodes: Set::new(),
        ctx: None,
        active_mappers: Vec::new(),
        active_type_mappers_caches: Vec::new(),
        scratch_mappers: Vec::new(),
        scratch_contexts: Vec::new(),
        scratch_mapper_lists: Vec::new(),
        free_type_mapper_caches: Vec::new(),
        free_type_lists: Vec::new(),
        ambient_modules_once: false,
        ambient_modules: Vec::new(),
        within_unreachable_code: false,
        reported_unreachable_nodes: Set::new(),
        non_existent_properties: Set::new(),
        deferred_diagnostic_callbacks: Vec::new(),
        exports_by_target_index: FxHashMap::default(),
        module_export_index: Default::default(),
        alias_cache_blockers: 1,
        unassigned_type: dummy_type,
        type_to_string_nodebuilder: None,
        scratch_keyed_chain_cache: Vec::new(),
        emit_resolver: None,
        overlay: Box::default(),
        seed_mode: false,
        is_fork: false,
    });
    c.undefined_symbol = c.new_symbol(SymbolFlags::Property, "undefined");
    c.arguments_symbol = c.new_symbol(SymbolFlags::Property, "arguments");
    c.require_symbol = c.new_symbol(SymbolFlags::Property, "require");
    c.unknown_symbol = c.new_symbol(SymbolFlags::Property, "unknown");
    c.global_this_symbol = c.new_symbol_ex(SymbolFlags::Module, "globalThis", CheckFlags::Readonly);
    c.global_this_symbol.set_exports(Some(c.globals));
    c.globals.set(c.global_this_symbol.name.get(), c.global_this_symbol);
    c.name_resolver = c.create_name_resolver();
    c.name_resolver_for_suggestion = c.create_name_resolver_for_suggestion();
    c.any_type = c.new_intrinsic_type(TypeFlags::Any, "any");
    c.auto_type = c.new_intrinsic_type_ex(TypeFlags::Any, "any", ObjectFlags::NonInferrableType);
    c.wildcard_type = c.new_intrinsic_type(TypeFlags::Any, "any");
    c.blocked_string_type = c.new_intrinsic_type(TypeFlags::Any, "any");
    c.error_type = c.new_intrinsic_type(TypeFlags::Any, "error");
    c.unresolved_type = c.new_intrinsic_type(TypeFlags::Any, "unresolved");
    c.non_inferrable_any_type = c.new_intrinsic_type_ex(TypeFlags::Any, "any", ObjectFlags::ContainsWideningType);
    c.intrinsic_marker_type = c.new_intrinsic_type(TypeFlags::Any, "intrinsic");
    c.unknown_type = c.new_intrinsic_type(TypeFlags::Unknown, "unknown");
    c.undefined_type = c.new_intrinsic_type(TypeFlags::Undefined, "undefined");
    c.undefined_widening_type = c.create_widening_type(c.undefined_type);
    c.missing_type = c.new_intrinsic_type(TypeFlags::Undefined, "undefined");
    c.undefined_or_missing_type = if c.exact_optional_property_types { c.missing_type } else { c.undefined_type };
    c.optional_type = c.new_intrinsic_type(TypeFlags::Undefined, "undefined");
    c.null_type = c.new_intrinsic_type(TypeFlags::Null, "null");
    c.null_widening_type = c.create_widening_type(c.null_type);
    c.string_type = c.new_intrinsic_type(TypeFlags::String, "string");
    c.number_type = c.new_intrinsic_type(TypeFlags::Number, "number");
    c.bigint_type = c.new_intrinsic_type(TypeFlags::BigInt, "bigint");
    c.regular_false_type = c.new_literal_type(TypeFlags::BooleanLiteral, Some(LiteralValue::Boolean(false)), None);
    c.false_type = c.new_literal_type(TypeFlags::BooleanLiteral, Some(LiteralValue::Boolean(false)), Some(c.regular_false_type));
    c.regular_false_type.as_literal_type().fresh_type.set(Some(c.false_type));
    c.false_type.as_literal_type().fresh_type.set(Some(c.false_type));
    c.regular_true_type = c.new_literal_type(TypeFlags::BooleanLiteral, Some(LiteralValue::Boolean(true)), None);
    c.true_type = c.new_literal_type(TypeFlags::BooleanLiteral, Some(LiteralValue::Boolean(true)), Some(c.regular_true_type));
    c.regular_true_type.as_literal_type().fresh_type.set(Some(c.true_type));
    c.true_type.as_literal_type().fresh_type.set(Some(c.true_type));
    c.boolean_type = c.get_union_type(&[c.regular_false_type, c.regular_true_type]);
    c.es_symbol_type = c.new_intrinsic_type(TypeFlags::ESSymbol, "symbol");
    c.void_type = c.new_intrinsic_type(TypeFlags::Void, "void");
    c.never_type = c.new_intrinsic_type(TypeFlags::Never, "never");
    c.silent_never_type = c.new_intrinsic_type_ex(TypeFlags::Never, "never", ObjectFlags::NonInferrableType);
    c.implicit_never_type = c.new_intrinsic_type(TypeFlags::Never, "never");
    c.unreachable_never_type = c.new_intrinsic_type(TypeFlags::Never, "never");
    c.non_primitive_type = c.new_intrinsic_type(TypeFlags::NonPrimitive, "object");
    c.string_or_number_type = c.get_union_type(&[c.string_type, c.number_type]);
    c.string_number_symbol_type = c.get_union_type(&[c.string_type, c.number_type, c.es_symbol_type]);
    c.number_or_big_int_type = c.get_union_type(&[c.number_type, c.bigint_type]);
    c.numeric_string_type = c.get_template_literal_type(&["", ""], &[c.number_type]); // The `${number}` type
    c.template_constraint_type = c.get_union_type(&[c.string_type, c.number_type, c.boolean_type, c.bigint_type, c.null_type, c.undefined_type]);
    c.unique_literal_type = c.new_intrinsic_type(TypeFlags::Never, "never"); // Special `never` flagged by union reduction to behave as a literal
    c.unique_literal_mapper = new_function_type_mapper(|c, t| c.get_unique_literal_type_for_type_parameter(t));
    c.report_unreliable_mapper = new_function_type_mapper(|c, t| c.report_unreliable_worker(t));
    c.report_unmeasurable_mapper = new_function_type_mapper(|c, t| c.report_unmeasurable_worker(t));
    c.restrictive_mapper = new_function_type_mapper(|c, t| c.restrictive_mapper_worker(t));
    c.permissive_mapper = new_function_type_mapper(|c, t| c.permissive_mapper_worker(t));
    c.empty_object_type = c.new_anonymous_type(None /*symbol*/, None, &[], &[], &[]);
    c.empty_jsx_object_type = c.new_anonymous_type(None /*symbol*/, None, &[], &[], &[]);
    c.empty_fresh_jsx_object_type = c.new_anonymous_type(None /*symbol*/, None, &[], &[], &[]);
    let type_literal_symbol = c.new_symbol(SymbolFlags::TypeLiteral, ast::InternalSymbolNameType);
    c.empty_type_literal_type = c.new_anonymous_type(Some(type_literal_symbol), None, &[], &[], &[]);
    c.unknown_empty_object_type = c.new_anonymous_type(None /*symbol*/, None, &[], &[], &[]);
    c.unknown_union_type = c.create_unknown_union_type();
    c.empty_generic_type = c.new_anonymous_type(None /*symbol*/, None, &[], &[], &[]);
    c.object_type_instantiations.insert(c.empty_generic_type, PackedMap::default());
    c.any_function_type = c.new_anonymous_type(None /*symbol*/, None, &[], &[], &[]);
    c.any_function_type.object_flags.set(c.any_function_type.object_flags.get_lazy() | ObjectFlags::NonInferrableType);
    c.no_constraint_type = c.new_anonymous_type(None /*symbol*/, None, &[], &[], &[]);
    c.circular_constraint_type = c.new_anonymous_type(None /*symbol*/, None, &[], &[], &[]);
    c.resolving_default_type = c.new_anonymous_type(None /*symbol*/, None, &[], &[], &[]);
    c.marker_super_type = c.new_type_parameter(None);
    c.marker_sub_type = c.new_type_parameter(None);
    c.marker_sub_type.as_type_parameter().constraint.set(Some(c.marker_super_type));
    c.marker_other_type = c.new_type_parameter(None);
    c.marker_super_type_for_check = c.new_type_parameter(None);
    c.marker_sub_type_for_check = c.new_type_parameter(None);
    c.marker_sub_type_for_check.as_type_parameter().constraint.set(Some(c.marker_super_type_for_check));
    c.no_type_predicate = P::new(TypePredicate {
        kind: Cell::new(TypePredicateKind::Identifier),
        parameter_index: Cell::new(0),
        parameter_name: Cell::new("<<unresolved>>"),
        t: Cell::new(Some(c.any_type)),
    });
    c.any_signature = c.new_signature(SignatureFlags::None, None, &[], None, &[], Some(c.any_type), None, 0);
    c.unknown_signature = c.new_signature(SignatureFlags::None, None, &[], None, &[], Some(c.error_type), None, 0);
    c.resolving_signature = c.new_signature(SignatureFlags::None, None, &[], None, &[], Some(c.any_type), None, 0);
    c.silent_never_signature = c.new_signature(SignatureFlags::None, None, &[], None, &[], Some(c.silent_never_type), None, 0);
    c.enum_number_index_info = P::new(IndexInfo {
        key_type: Cell::new(Some(c.number_type)),
        value_type: Cell::new(Some(c.string_type)),
        is_readonly: Cell::new(true),
        ..Default::default()
    });
    c.any_base_type_index_info = P::new(IndexInfo {
        key_type: Cell::new(Some(c.string_type)),
        value_type: Cell::new(Some(c.any_type)),
        is_readonly: Cell::new(false),
        ..Default::default()
    });
    c.empty_string_type = c.get_string_literal_type("");
    c.zero_type = c.get_number_literal_type(Number(0.0));
    c.zero_big_int_type = c.get_big_int_literal_type(PseudoBigInt::default());
    let mut typeof_names: Vec<&'static str> = typeofNEFacts.keys().copied().collect();
    typeof_names.sort_unstable();
    let typeof_types: Vec<P<Type>> = typeof_names.iter().map(|name| c.get_string_literal_type(name)).collect();
    c.typeof_type = c.get_union_type(&typeof_types);
    c.initialize_closures();
    c.initialize_iteration_resolvers();
    c.initialize_checker();
    c.alias_cache_blockers -= 1;
    c
}

// Methods for Go's function-valued Checker fields.

impl Checker {
    /// Shared graph (`tsrs_core::sharedgraph`): a checker whose history is the frozen seed `base`'s, then
    /// its own. Interning and identity maps are cloned (their keys and values point into the frozen graph), pure memos
    /// start empty, link stores read through to the base's and copy a record on first access, stacks start empty.
    #[expect(clippy::clone_on_copy, reason = "a field-by-field list: `clone` for every field, whatever its type")]
    pub fn fork(base: &'static Checker) -> Box<Checker> {
        base.assert_freezable();
        // Relaxed: the counter only hands out distinct ids (as in new_checker).
        let id = nextCheckerID.fetch_add(1, Ordering::Relaxed) + 1;
        let transient = ast::TransientSymbols::new(id);
        let mut c = Box::new(Checker {
            id,
            program: base.program,
            compiler_options: base.compiler_options.clone(),
            files: base.files,
            file_index_map: base.file_index_map.clone(),
            type_count: base.type_count.clone(),
            symbol_count: base.symbol_count.clone(),
            signature_count: base.signature_count.clone(),
            total_instantiation_count: base.total_instantiation_count.clone(),
            instantiation_count: base.instantiation_count.clone(),
            census: crate::workcensus::census_path().map(|_| crate::workcensus::Census::new()),
            union_front_cache: crate::unioncache::UnionFrontCache::new(),
            infer_memo: crate::infermemo::InferMemo::new(),
            // A fork's history is the seed's, then its own (`fork`): it continues the seed's TS2590 bookkeeping and
            // shares its tuple element type parameters (frozen types).
            too_complex_reports: base.too_complex_reports,
            too_complex_nodes: base.too_complex_nodes.clone(),
            too_complex_reported: base.too_complex_reported.clone(),
            tuple_elements: base.tuple_elements.clone(),
            diagnostic_adds: base.diagnostic_adds.clone(),
            expression_checks: base.expression_checks.clone(),
            instantiation_stack: Vec::new(),
            conditional_constraint_depth: base.conditional_constraint_depth.clone(),
            inline_level: base.inline_level.clone(),
            serialization_level: base.serialization_level.clone(),
            current_node: None,
            checking_file: None,
            statements_checked_in_pieces: None,
            // The seed's history: its checks deferred to files it did not check run in the fork that checks them.
            deferred_type_argument_checks: base.deferred_type_argument_checks.clone(),
            variance_type_parameter: None,
            language_version: base.language_version.clone(),
            module_kind: base.module_kind.clone(),
            module_resolution_kind: base.module_resolution_kind.clone(),
            is_inference_partially_blocked: base.is_inference_partially_blocked.clone(),
            legacy_decorators: base.legacy_decorators.clone(),
            emit_standard_class_fields: base.emit_standard_class_fields.clone(),
            strict_null_checks: base.strict_null_checks.clone(),
            strict_function_types: base.strict_function_types.clone(),
            strict_bind_call_apply: base.strict_bind_call_apply.clone(),
            strict_property_initialization: base.strict_property_initialization.clone(),
            strict_builtin_iterator_return: base.strict_builtin_iterator_return.clone(),
            no_implicit_any: base.no_implicit_any.clone(),
            no_implicit_this: base.no_implicit_this.clone(),
            use_unknown_in_catch_variables: base.use_unknown_in_catch_variables.clone(),
            exact_optional_property_types: base.exact_optional_property_types.clone(),
            can_collect_symbol_alias_accessibility_data: base.can_collect_symbol_alias_accessibility_data.clone(),
            was_canceled: base.was_canceled.clone(),
            array_variances: base.array_variances,
            globals: base.globals.clone(),
            string_literal_types: base.string_literal_types.fork(),
            number_literal_types: base.number_literal_types.clone(),
            nan_type: base.nan_type.clone(),
            bigint_literal_types: base.bigint_literal_types.clone(),
            enum_literal_types: base.enum_literal_types.clone(),
            enum_nan_literal_types: base.enum_nan_literal_types.clone(),
            indexed_access_types: crate::basedmap::Based::over(&base.indexed_access_types),
            template_literal_types: base.template_literal_types.clone(),
            string_mapping_types: base.string_mapping_types.clone(),
            unique_es_symbol_types: base.unique_es_symbol_types.clone(),
            this_expando_kinds: base.this_expando_kinds.clone(),
            this_expando_locations: base.this_expando_locations.clone(),
            subtype_reduction_cache: base.subtype_reduction_cache.clone(),
            cached_types: base.cached_types.clone(),
            cached_signatures: base.cached_signatures.clone(),
            undefined_properties: base.undefined_properties.clone(),
            undefined_properties_by_prop: base.undefined_properties_by_prop.clone(),
            narrowed_types: base.narrowed_types.clone(),
            assignment_reduced_types: base.assignment_reduced_types.clone(),
            discriminated_contextual_types: base.discriminated_contextual_types.clone(),
            instantiation_expression_types: base.instantiation_expression_types.clone(),
            substitution_types: base.substitution_types.clone(),
            reverse_mapped_cache: base.reverse_mapped_cache.clone(),
            reverse_homomorphic_mapped_cache: base.reverse_homomorphic_mapped_cache.clone(),
            iteration_types_cache: base.iteration_types_cache.clone(),
            marker_types: base.marker_types.clone(),
            resolving_explicit_type_of_symbol: base.resolving_explicit_type_of_symbol.clone(),
            undefined_symbol: base.undefined_symbol.clone(),
            arguments_symbol: base.arguments_symbol.clone(),
            require_symbol: base.require_symbol.clone(),
            unknown_symbol: base.unknown_symbol.clone(),
            unresolved_symbols: base.unresolved_symbols.clone(),
            error_types: base.error_types.clone(),
            module_symbols: base.module_symbols.clone(),
            resolved_module_source_files: base.resolved_module_source_files.clone(),
            global_this_symbol: base.global_this_symbol.clone(),
            symbol_table_alias_cache: base.symbol_table_alias_cache.clone(),
            class_expression_name_tables: base.class_expression_name_tables.clone(),
            name_resolver: base.name_resolver.clone(),
            name_resolver_for_suggestion: base.name_resolver_for_suggestion.clone(),
            tuple_types: crate::basedmap::Based::over(&base.tuple_types),
            union_types: crate::basedmap::Based::over(&base.union_types),
            union_of_union_types: crate::basedmap::Based::over(&base.union_of_union_types),
            intersection_types: crate::basedmap::Based::over(&base.intersection_types),
            properties_types: base.properties_types.clone(),
            diagnostics: base.diagnostics.clone(),
            suggestion_diagnostics: base.suggestion_diagnostics.clone(),
            merged_symbols: base.merged_symbols.clone(),
            factory: ast::NodeFactory::default(),
            node_links: base.node_links.fork(),
            signature_links: base.signature_links.fork(),
            symbol_node_links: base.symbol_node_links.fork(),
            type_node_links: base.type_node_links.fork(),
            enum_member_links: base.enum_member_links.fork(),
            assertion_links: base.assertion_links.fork(),
            array_literal_links: base.array_literal_links.fork(),
            switch_statement_links: base.switch_statement_links.fork(),
            jsx_element_links: base.jsx_element_links.fork(),
            computed_name_links: base.computed_name_links.fork(),
            symbol_reference_links: base.symbol_reference_links.fork(),
            value_symbol_links: base.value_symbol_links.fork(transient.owner()),
            transient,
            mapped_symbol_links: base.mapped_symbol_links.fork(),
            deferred_symbol_links: base.deferred_symbol_links.fork(),
            alias_symbol_links: base.alias_symbol_links.fork(),
            module_symbol_links: base.module_symbol_links.fork(),
            late_bound_links: base.late_bound_links.fork(),
            export_type_links: base.export_type_links.fork(),
            members_and_exports_links: base.members_and_exports_links.fork(),
            type_alias_links: base.type_alias_links.fork(),
            declared_type_links: base.declared_type_links.fork(),
            spread_links: base.spread_links.fork(),
            variance_links: base.variance_links.fork(),
            reverse_mapped_symbol_links: base.reverse_mapped_symbol_links.fork(),
            marked_assignment_symbol_links: base.marked_assignment_symbol_links.fork(),
            symbol_container_links: base.symbol_container_links.fork(),
            source_file_links: base.source_file_links.fork(),
            reg_exp_scanner: None,
            pattern_for_type: base.pattern_for_type.clone(),
            lazy_member_tables: crate::basedmap::Based::over(&base.lazy_member_tables),
            object_types_without_abstract_construct_signatures: base.object_types_without_abstract_construct_signatures.clone(),
            structured_type_base_constraints: base.structured_type_base_constraints.clone(),
            object_type_instantiations: crate::basedmap::Based::over(&base.object_type_instantiations),
            lazy_mapped_tables: crate::basedmap::Based::over(&base.lazy_mapped_tables),
            lazy_members: base.lazy_members.clone(),
            lazy_tuples: base.lazy_tuples.clone(),
            #[cfg(feature = "site-counts")]
            inst_symbol_sites: base.inst_symbol_sites.clone(),
            lazy_cond_mapper: base.lazy_cond_mapper.clone(),
            lazy_prop_cache: base.lazy_prop_cache.clone(),
            lazy_has_prop: base.lazy_has_prop.clone(),
            lazy_unmatched: base.lazy_unmatched.clone(),
            lazy_empty: base.lazy_empty.clone(),
            lazy_member_stats: base.lazy_member_stats.clone(),
            augment_filter: base.augment_filter.clone(),
            #[cfg(feature = "assignment-stats")]
            stats_created: base.stats_created.clone(),
            context_free_types: base.context_free_types.clone(),
            any_type: base.any_type.clone(),
            auto_type: base.auto_type.clone(),
            wildcard_type: base.wildcard_type.clone(),
            blocked_string_type: base.blocked_string_type.clone(),
            error_type: base.error_type.clone(),
            unresolved_type: base.unresolved_type.clone(),
            non_inferrable_any_type: base.non_inferrable_any_type.clone(),
            intrinsic_marker_type: base.intrinsic_marker_type.clone(),
            unknown_type: base.unknown_type.clone(),
            undefined_type: base.undefined_type.clone(),
            undefined_widening_type: base.undefined_widening_type.clone(),
            missing_type: base.missing_type.clone(),
            undefined_or_missing_type: base.undefined_or_missing_type.clone(),
            optional_type: base.optional_type.clone(),
            null_type: base.null_type.clone(),
            null_widening_type: base.null_widening_type.clone(),
            string_type: base.string_type.clone(),
            number_type: base.number_type.clone(),
            bigint_type: base.bigint_type.clone(),
            regular_false_type: base.regular_false_type.clone(),
            false_type: base.false_type.clone(),
            regular_true_type: base.regular_true_type.clone(),
            true_type: base.true_type.clone(),
            boolean_type: base.boolean_type.clone(),
            es_symbol_type: base.es_symbol_type.clone(),
            void_type: base.void_type.clone(),
            never_type: base.never_type.clone(),
            silent_never_type: base.silent_never_type.clone(),
            implicit_never_type: base.implicit_never_type.clone(),
            unreachable_never_type: base.unreachable_never_type.clone(),
            non_primitive_type: base.non_primitive_type.clone(),
            string_or_number_type: base.string_or_number_type.clone(),
            string_number_symbol_type: base.string_number_symbol_type.clone(),
            number_or_big_int_type: base.number_or_big_int_type.clone(),
            template_constraint_type: base.template_constraint_type.clone(),
            numeric_string_type: base.numeric_string_type.clone(),
            unique_literal_type: base.unique_literal_type.clone(),
            unique_literal_mapper: base.unique_literal_mapper.clone(),
            reliability_flags: base.reliability_flags.clone(),
            report_unreliable_mapper: base.report_unreliable_mapper.clone(),
            report_unmeasurable_mapper: base.report_unmeasurable_mapper.clone(),
            restrictive_mapper: base.restrictive_mapper.clone(),
            permissive_mapper: base.permissive_mapper.clone(),
            empty_object_type: base.empty_object_type.clone(),
            empty_jsx_object_type: base.empty_jsx_object_type.clone(),
            empty_fresh_jsx_object_type: base.empty_fresh_jsx_object_type.clone(),
            empty_type_literal_type: base.empty_type_literal_type.clone(),
            unknown_empty_object_type: base.unknown_empty_object_type.clone(),
            unknown_union_type: base.unknown_union_type.clone(),
            empty_generic_type: base.empty_generic_type.clone(),
            any_function_type: base.any_function_type.clone(),
            no_constraint_type: base.no_constraint_type.clone(),
            circular_constraint_type: base.circular_constraint_type.clone(),
            resolving_default_type: base.resolving_default_type.clone(),
            marker_super_type: base.marker_super_type.clone(),
            marker_sub_type: base.marker_sub_type.clone(),
            marker_other_type: base.marker_other_type.clone(),
            marker_super_type_for_check: base.marker_super_type_for_check.clone(),
            marker_sub_type_for_check: base.marker_sub_type_for_check.clone(),
            no_type_predicate: base.no_type_predicate.clone(),
            any_signature: base.any_signature.clone(),
            unknown_signature: base.unknown_signature.clone(),
            resolving_signature: base.resolving_signature.clone(),
            silent_never_signature: base.silent_never_signature.clone(),
            cached_arguments_referenced: base.cached_arguments_referenced.clone(),
            enum_number_index_info: base.enum_number_index_info.clone(),
            any_base_type_index_info: base.any_base_type_index_info.clone(),
            pattern_ambient_modules: base.pattern_ambient_modules.clone(),
            pattern_ambient_module_augmentations: base.pattern_ambient_module_augmentations.clone(),
            pattern_ambient_module_augmentation_targets: base.pattern_ambient_module_augmentation_targets.clone(),
            module_import_attributes_types: base.module_import_attributes_types.clone(),
            global_object_type: base.global_object_type.clone(),
            global_function_type: base.global_function_type.clone(),
            global_callable_function_type: base.global_callable_function_type.clone(),
            global_newable_function_type: base.global_newable_function_type.clone(),
            global_array_type: base.global_array_type.clone(),
            global_readonly_array_type: base.global_readonly_array_type.clone(),
            global_string_type: base.global_string_type.clone(),
            global_number_type: base.global_number_type.clone(),
            global_boolean_type: base.global_boolean_type.clone(),
            global_reg_exp_type: base.global_reg_exp_type.clone(),
            global_this_type: base.global_this_type.clone(),
            any_array_type: base.any_array_type.clone(),
            auto_array_type: base.auto_array_type.clone(),
            any_readonly_array_type: base.any_readonly_array_type.clone(),
            deferred_global_import_meta_expression_type: base.deferred_global_import_meta_expression_type.clone(),
            contextual_binding_patterns: Vec::new(),
            empty_string_type: base.empty_string_type.clone(),
            zero_type: base.zero_type.clone(),
            zero_big_int_type: base.zero_big_int_type.clone(),
            typeof_type: base.typeof_type.clone(),
            // Not always empty between files (webpack: 102 left on the stack); a fork continues the seed's history.
            type_resolutions: base.type_resolutions.clone(),
            base_types_depth: base.base_types_depth.clone(),
            base_types_resolved_log: Vec::new(),
            base_types_circular: Vec::new(),
            resolution_start: base.resolution_start.clone(),
            variance_stack: Vec::new(),
            call_resolution_stack: Vec::new(),
            apparent_argument_count: base.apparent_argument_count.clone(),
            last_get_combined_node_flags_node: None,
            last_get_combined_node_flags_result: NodeFlags::None,
            last_get_combined_modifier_flags_node: None,
            last_get_combined_modifier_flags_result: ModifierFlags::None,
            freeinference_state: None,
            free_flow_state: None,
            flow_loop_cache: FxHashMap::default(),
            flow_loop_stack: Vec::new(),
            shared_flows: Vec::new(),
            antecedent_types: Vec::new(),
            flow_analysis_disabled: base.flow_analysis_disabled.clone(),
            flow_invocation_count: base.flow_invocation_count.clone(),
            flow_type_cache: None,
            flow_memo: Box::new(crate::flowmemo::FlowMemo::new()),
            last_flow_node: None,
            last_flow_node_reachable: false,
            flow_node_reachable: FxHashMap::default(),
            flow_node_post_super: FxHashMap::default(),
            renamed_binding_elements_in_types: Vec::new(),
            contextual_infos: Vec::new(),
            inference_context_infos: Vec::new(),
            awaited_type_stack: Vec::new(),
            reverse_mapped_source_stack: Vec::new(),
            reverse_mapped_target_stack: Vec::new(),
            reverse_expanding_flags: base.reverse_expanding_flags.clone(),
            free_relater: None,
            subtype_relation: P::new(Relation::default()),
            strict_subtype_relation: P::new(Relation::default()),
            assignable_relation: P::new(Relation::default()),
            comparable_relation: P::new(Relation::default()),
            identity_relation: P::new(Relation::default()),
            enum_relation: base.enum_relation.clone(),
            get_global_es_symbol_type_cache: base.get_global_es_symbol_type_cache.clone(),
            get_global_big_int_type_cache: base.get_global_big_int_type_cache.clone(),
            get_global_import_meta_type_cache: base.get_global_import_meta_type_cache.clone(),
            get_global_import_attributes_type_cache: base.get_global_import_attributes_type_cache.clone(),
            get_global_import_attributes_type_checked_cache: base.get_global_import_attributes_type_checked_cache.clone(),
            get_global_non_nullable_type_alias_or_nil_cache: base.get_global_non_nullable_type_alias_or_nil_cache.clone(),
            get_global_extract_symbol_cache: base.get_global_extract_symbol_cache.clone(),
            get_global_disposable_type_cache: base.get_global_disposable_type_cache.clone(),
            get_global_async_disposable_type_cache: base.get_global_async_disposable_type_cache.clone(),
            get_global_awaited_symbol_cache: base.get_global_awaited_symbol_cache.clone(),
            get_global_awaited_symbol_or_nil_cache: base.get_global_awaited_symbol_or_nil_cache.clone(),
            get_global_nan_symbol_or_nil_cache: base.get_global_nan_symbol_or_nil_cache.clone(),
            get_global_record_symbol_cache: base.get_global_record_symbol_cache.clone(),
            get_global_template_strings_array_type_cache: base.get_global_template_strings_array_type_cache.clone(),
            get_global_es_symbol_constructor_symbol_or_nil_cache: base.get_global_es_symbol_constructor_symbol_or_nil_cache.clone(),
            get_global_es_symbol_constructor_type_symbol_or_nil_cache: base.get_global_es_symbol_constructor_type_symbol_or_nil_cache.clone(),
            get_global_import_call_options_type_cache: base.get_global_import_call_options_type_cache.clone(),
            get_global_import_call_options_type_checked_cache: base.get_global_import_call_options_type_checked_cache.clone(),
            get_global_promise_type_cache: base.get_global_promise_type_cache.clone(),
            get_global_promise_type_checked_cache: base.get_global_promise_type_checked_cache.clone(),
            get_global_promise_like_type_cache: base.get_global_promise_like_type_cache.clone(),
            get_global_promise_constructor_symbol_cache: base.get_global_promise_constructor_symbol_cache.clone(),
            get_global_promise_constructor_symbol_or_nil_cache: base.get_global_promise_constructor_symbol_or_nil_cache.clone(),
            get_global_omit_symbol_cache: base.get_global_omit_symbol_cache.clone(),
            get_global_iterator_type_cache: base.get_global_iterator_type_cache.clone(),
            get_global_iterable_type_cache: base.get_global_iterable_type_cache.clone(),
            get_global_iterable_type_checked_cache: base.get_global_iterable_type_checked_cache.clone(),
            get_global_iterable_iterator_type_cache: base.get_global_iterable_iterator_type_cache.clone(),
            get_global_iterable_iterator_type_checked_cache: base.get_global_iterable_iterator_type_checked_cache.clone(),
            get_global_iterator_object_type_cache: base.get_global_iterator_object_type_cache.clone(),
            get_global_generator_type_cache: base.get_global_generator_type_cache.clone(),
            get_global_async_iterator_type_cache: base.get_global_async_iterator_type_cache.clone(),
            get_global_async_iterable_type_cache: base.get_global_async_iterable_type_cache.clone(),
            get_global_async_iterable_type_checked_cache: base.get_global_async_iterable_type_checked_cache.clone(),
            get_global_async_iterable_iterator_type_cache: base.get_global_async_iterable_iterator_type_cache.clone(),
            get_global_async_iterable_iterator_type_checked_cache: base.get_global_async_iterable_iterator_type_checked_cache.clone(),
            get_global_async_iterator_object_type_cache: base.get_global_async_iterator_object_type_cache.clone(),
            get_global_async_generator_type_cache: base.get_global_async_generator_type_cache.clone(),
            get_global_iterator_yield_result_type_cache: base.get_global_iterator_yield_result_type_cache.clone(),
            get_global_iterator_return_result_type_cache: base.get_global_iterator_return_result_type_cache.clone(),
            get_global_typed_property_descriptor_type_cache: base.get_global_typed_property_descriptor_type_cache.clone(),
            get_global_class_decorator_context_type_cache: base.get_global_class_decorator_context_type_cache.clone(),
            get_global_class_method_decorator_context_type_cache: base.get_global_class_method_decorator_context_type_cache.clone(),
            get_global_class_getter_decorator_context_type_cache: base.get_global_class_getter_decorator_context_type_cache.clone(),
            get_global_class_setter_decorator_context_type_cache: base.get_global_class_setter_decorator_context_type_cache.clone(),
            get_global_class_accessor_decorator_context_type_cache: base.get_global_class_accessor_decorator_context_type_cache.clone(),
            get_global_class_accessor_decorator_target_type_cache: base.get_global_class_accessor_decorator_target_type_cache.clone(),
            get_global_class_accessor_decorator_result_type_cache: base.get_global_class_accessor_decorator_result_type_cache.clone(),
            get_global_class_field_decorator_context_type_cache: base.get_global_class_field_decorator_context_type_cache.clone(),
            global_builtin_iterator_types_cache: base.global_builtin_iterator_types_cache.clone(),
            global_builtin_async_iterator_types_cache: base.global_builtin_async_iterator_types_cache.clone(),
            sync_iteration_types_resolver: base.sync_iteration_types_resolver.clone(),
            async_iteration_types_resolver: base.async_iteration_types_resolver.clone(),
            _jsx_namespace: base._jsx_namespace.clone(),
            _jsx_factory_entity: base._jsx_factory_entity.clone(),
            skip_direct_inference_nodes: base.skip_direct_inference_nodes.clone(),
            ctx: None,
            active_mappers: Vec::new(),
            active_type_mappers_caches: Vec::new(),
            scratch_mappers: Vec::new(),
            scratch_contexts: Vec::new(),
            scratch_mapper_lists: Vec::new(),
            free_type_mapper_caches: Vec::new(),
            free_type_lists: Vec::new(),
            ambient_modules_once: base.ambient_modules_once.clone(),
            ambient_modules: base.ambient_modules.clone(),
            within_unreachable_code: base.within_unreachable_code.clone(),
            reported_unreachable_nodes: base.reported_unreachable_nodes.clone(),
            non_existent_properties: base.non_existent_properties.clone(),
            deferred_diagnostic_callbacks: Vec::new(),
            exports_by_target_index: FxHashMap::default(),
            module_export_index: Default::default(),
            alias_cache_blockers: base.alias_cache_blockers.clone(),
            unassigned_type: base.unassigned_type.clone(),
            type_to_string_nodebuilder: None,
            scratch_keyed_chain_cache: Vec::new(),
            emit_resolver: None,
            overlay: Box::default(),
            seed_mode: false,
            is_fork: true,
        });
        c.name_resolver = c.create_name_resolver();
        c.name_resolver_for_suggestion = c.create_name_resolver_for_suggestion();
        c
    }

    /// Shared graph: the checker is between files, so its graph can be frozen.
    pub fn assert_freezable(&self) {
        assert!(
            self.instantiation_stack.is_empty()
                && self.variance_stack.is_empty()
                && self.call_resolution_stack.is_empty()
                && self.flow_loop_stack.is_empty()
                && self.current_node.is_none()
                && self.deferred_diagnostic_callbacks.is_empty(),
            "shared graph: the seed checker is not at a file boundary: resolutions {} instantiations {} variances {} calls {} flow loops {} node {} deferred {}",
            self.type_resolutions.len(),
            self.instantiation_stack.len(),
            self.variance_stack.len(),
            self.call_resolution_stack.len(),
            self.flow_loop_stack.len(),
            self.current_node.is_some(),
            self.deferred_diagnostic_callbacks.len()
        );
    }

    pub(crate) fn compare_symbols(&mut self, s1: Option<P<Symbol>>, s2: Option<P<Symbol>>) -> i32 {
        self.compare_symbols_worker(s1, s2)
    }

    pub(crate) fn compare_symbol_chains(&mut self, a: &[P<Symbol>], b: &[P<Symbol>]) -> i32 {
        self.compare_symbol_chains_worker(a, b)
    }

    pub(crate) fn resolve_name(
        &mut self,
        location: Option<P<Node>>,
        name: &str,
        meaning: SymbolFlags,
        name_not_found_message: Option<&'static Message>,
        is_use: bool,
        exclude_globals: bool,
    ) -> Option<P<Symbol>> {
        let r = self.name_resolver;
        r.resolve(self, location, name, meaning, name_not_found_message, is_use, exclude_globals)
    }

    pub(crate) fn resolve_name_for_symbol_suggestion(
        &mut self,
        location: Option<P<Node>>,
        name: &str,
        meaning: SymbolFlags,
        name_not_found_message: Option<&'static Message>,
        is_use: bool,
        exclude_globals: bool,
    ) -> Option<P<Symbol>> {
        let r = self.name_resolver_for_suggestion;
        r.resolve(self, location, name, meaning, name_not_found_message, is_use, exclude_globals)
    }

    /// Go `c.evaluate = evaluator.NewEvaluator(c.evaluateEntity, ast.OEKParentheses)`.
    pub(crate) fn evaluate(&mut self, expr: P<Node>, location: P<Node>) -> evaluator::Result {
        evaluator::evaluate(self, |c, expr, location| c.evaluate_entity(expr, Some(location)), ast::OuterExpressionKinds::Parentheses, expr, location)
    }

    pub(crate) fn is_primitive_or_object_or_empty_type(&mut self, t: P<Type>) -> bool {
        t.flags().intersects(TypeFlags::Primitive | TypeFlags::NonPrimitive) || self.is_empty_anonymous_object_type(t)
    }

    pub(crate) fn contains_missing_type(&mut self, t: P<Type>) -> bool {
        t == self.missing_type || t.flags().intersects(TypeFlags::Union) && t.types()[0] == self.missing_type
    }

    /// The worker's two cached answers inline (most calls); the worker repeats them.
    #[inline]
    pub(crate) fn could_contain_type_variables(&mut self, t: P<Type>) -> bool {
        if !t.flags().intersects(TypeFlags::StructuredOrInstantiable) {
            return false;
        }
        let object_flags = t.object_flags_lazy();
        if object_flags.intersects(ObjectFlags::CouldContainTypeVariablesComputed) {
            return object_flags.intersects(ObjectFlags::CouldContainTypeVariables);
        }
        self.could_contain_type_variables_worker(t)
    }

    pub(crate) fn is_string_index_signature_only_type(&mut self, t: P<Type>) -> bool {
        self.is_string_index_signature_only_type_worker(t)
    }

    pub(crate) fn mark_node_assignments(&mut self, node: P<Node>) -> bool {
        self.mark_node_assignments_worker(node)
    }

    pub(crate) fn compare_types_assignable(&mut self, source: P<Type>, target: P<Type>, report_errors: bool) -> Ternary {
        self.compare_types_assignable_worker(source, target, report_errors)
    }

    /// `c.compareTypesAssignable` used as a value (`TypeComparer`).
    pub(crate) fn compare_types_assignable_comparer(&self) -> TypeComparer {
        type_comparer(|c, s, t, report_errors| c.compare_types_assignable(s, t, report_errors))
    }

    /// The closure built by Go `getGlobalTypesResolver(names, arity, reportErrors)`.
    pub(crate) fn get_global_types(&mut self, names: &[&str], arity: i32, report_errors: bool) -> &'static [P<Type>] {
        let types: Vec<P<Type>> = names.iter().map(|name| self.get_global_type(name, arity, report_errors)).collect();
        alloc_slice(&types)
    }

    // Lazily resolved globals (Go: `c.getGlobalXxx = c.getGlobalTypeResolver(...)` etc. in NewChecker).

    pub(crate) fn get_global_es_symbol_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_es_symbol_type_cache {
            return t;
        }
        let t = self.get_global_type("Symbol", 0 /*arity*/, false /*reportErrors*/);
        self.get_global_es_symbol_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_big_int_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_big_int_type_cache {
            return t;
        }
        let t = self.get_global_type("BigInt", 0 /*arity*/, false /*reportErrors*/);
        self.get_global_big_int_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_import_meta_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_import_meta_type_cache {
            return t;
        }
        let t = self.get_global_type("ImportMeta", 0 /*arity*/, true /*reportErrors*/);
        self.get_global_import_meta_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_import_attributes_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_import_attributes_type_cache {
            return t;
        }
        let t = self.get_global_type("ImportAttributes", 0 /*arity*/, false /*reportErrors*/);
        self.get_global_import_attributes_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_import_attributes_type_checked(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_import_attributes_type_checked_cache {
            return t;
        }
        let t = self.get_global_type("ImportAttributes", 0 /*arity*/, true /*reportErrors*/);
        self.get_global_import_attributes_type_checked_cache = Some(t);
        t
    }

    pub(crate) fn get_global_non_nullable_type_alias_or_nil(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_non_nullable_type_alias_or_nil_cache {
            return s;
        }
        let s = self.get_global_type_alias_symbol("NonNullable", 1 /*arity*/, false /*reportErrors*/);
        self.get_global_non_nullable_type_alias_or_nil_cache = Some(s);
        s
    }

    pub(crate) fn get_global_extract_symbol(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_extract_symbol_cache {
            return s;
        }
        let s = self.get_global_type_alias_symbol("Extract", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_extract_symbol_cache = Some(s);
        s
    }

    pub(crate) fn get_global_disposable_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_disposable_type_cache {
            return t;
        }
        let t = self.get_global_type("Disposable", 0 /*arity*/, true /*reportErrors*/);
        self.get_global_disposable_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_async_disposable_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_async_disposable_type_cache {
            return t;
        }
        let t = self.get_global_type("AsyncDisposable", 0 /*arity*/, true /*reportErrors*/);
        self.get_global_async_disposable_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_awaited_symbol(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_awaited_symbol_cache {
            return s;
        }
        let s = self.get_global_type_alias_symbol("Awaited", 1 /*arity*/, true /*reportErrors*/);
        self.get_global_awaited_symbol_cache = Some(s);
        s
    }

    pub(crate) fn get_global_awaited_symbol_or_nil(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_awaited_symbol_or_nil_cache {
            return s;
        }
        let s = self.get_global_type_alias_symbol("Awaited", 1 /*arity*/, false /*reportErrors*/);
        self.get_global_awaited_symbol_or_nil_cache = Some(s);
        s
    }

    pub(crate) fn get_global_nan_symbol_or_nil(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_nan_symbol_or_nil_cache {
            return s;
        }
        let s = self.get_global_symbol("NaN", SymbolFlags::Value, if false { Some(&diagnostics::Cannot_find_global_value_0) } else { None });
        self.get_global_nan_symbol_or_nil_cache = Some(s);
        s
    }

    pub(crate) fn get_global_record_symbol(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_record_symbol_cache {
            return s;
        }
        let s = self.get_global_type_alias_symbol("Record", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_record_symbol_cache = Some(s);
        s
    }

    pub(crate) fn get_global_template_strings_array_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_template_strings_array_type_cache {
            return t;
        }
        let t = self.get_global_type("TemplateStringsArray", 0 /*arity*/, true /*reportErrors*/);
        self.get_global_template_strings_array_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_es_symbol_constructor_symbol_or_nil(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_es_symbol_constructor_symbol_or_nil_cache {
            return s;
        }
        let s = self.get_global_symbol("Symbol", SymbolFlags::Value, if false { Some(&diagnostics::Cannot_find_global_value_0) } else { None });
        self.get_global_es_symbol_constructor_symbol_or_nil_cache = Some(s);
        s
    }

    pub(crate) fn get_global_es_symbol_constructor_type_symbol_or_nil(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_es_symbol_constructor_type_symbol_or_nil_cache {
            return s;
        }
        let s = self.get_global_symbol("SymbolConstructor", SymbolFlags::Type, if false { Some(&diagnostics::Cannot_find_global_type_0) } else { None });
        self.get_global_es_symbol_constructor_type_symbol_or_nil_cache = Some(s);
        s
    }

    pub(crate) fn get_global_import_call_options_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_import_call_options_type_cache {
            return t;
        }
        let t = self.get_global_type("ImportCallOptions", 0 /*arity*/, false /*reportErrors*/);
        self.get_global_import_call_options_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_import_call_options_type_checked(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_import_call_options_type_checked_cache {
            return t;
        }
        let t = self.get_global_type("ImportCallOptions", 0 /*arity*/, true /*reportErrors*/);
        self.get_global_import_call_options_type_checked_cache = Some(t);
        t
    }

    pub(crate) fn get_global_promise_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_promise_type_cache {
            return t;
        }
        let t = self.get_global_type("Promise", 1 /*arity*/, false /*reportErrors*/);
        self.get_global_promise_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_promise_type_checked(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_promise_type_checked_cache {
            return t;
        }
        let t = self.get_global_type("Promise", 1 /*arity*/, true /*reportErrors*/);
        self.get_global_promise_type_checked_cache = Some(t);
        t
    }

    pub(crate) fn get_global_promise_like_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_promise_like_type_cache {
            return t;
        }
        let t = self.get_global_type("PromiseLike", 1 /*arity*/, true /*reportErrors*/);
        self.get_global_promise_like_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_promise_constructor_symbol(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_promise_constructor_symbol_cache {
            return s;
        }
        let s = self.get_global_symbol("Promise", SymbolFlags::Value, if true { Some(&diagnostics::Cannot_find_global_value_0) } else { None });
        self.get_global_promise_constructor_symbol_cache = Some(s);
        s
    }

    pub(crate) fn get_global_promise_constructor_symbol_or_nil(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_promise_constructor_symbol_or_nil_cache {
            return s;
        }
        let s = self.get_global_symbol("Promise", SymbolFlags::Value, if false { Some(&diagnostics::Cannot_find_global_value_0) } else { None });
        self.get_global_promise_constructor_symbol_or_nil_cache = Some(s);
        s
    }

    pub(crate) fn get_global_omit_symbol(&mut self) -> Option<P<Symbol>> {
        if let Some(s) = self.get_global_omit_symbol_cache {
            return s;
        }
        let s = self.get_global_type_alias_symbol("Omit", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_omit_symbol_cache = Some(s);
        s
    }

    pub(crate) fn get_global_iterator_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_iterator_type_cache {
            return t;
        }
        let t = self.get_global_type("Iterator", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_iterator_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_iterable_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_iterable_type_cache {
            return t;
        }
        let t = self.get_global_type("Iterable", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_iterable_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_iterable_type_checked(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_iterable_type_checked_cache {
            return t;
        }
        let t = self.get_global_type("Iterable", 3 /*arity*/, true /*reportErrors*/);
        self.get_global_iterable_type_checked_cache = Some(t);
        t
    }

    pub(crate) fn get_global_iterable_iterator_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_iterable_iterator_type_cache {
            return t;
        }
        let t = self.get_global_type("IterableIterator", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_iterable_iterator_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_iterable_iterator_type_checked(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_iterable_iterator_type_checked_cache {
            return t;
        }
        let t = self.get_global_type("IterableIterator", 3 /*arity*/, true /*reportErrors*/);
        self.get_global_iterable_iterator_type_checked_cache = Some(t);
        t
    }

    pub(crate) fn get_global_iterator_object_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_iterator_object_type_cache {
            return t;
        }
        let t = self.get_global_type("IteratorObject", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_iterator_object_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_generator_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_generator_type_cache {
            return t;
        }
        let t = self.get_global_type("Generator", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_generator_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_async_iterator_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_async_iterator_type_cache {
            return t;
        }
        let t = self.get_global_type("AsyncIterator", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_async_iterator_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_async_iterable_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_async_iterable_type_cache {
            return t;
        }
        let t = self.get_global_type("AsyncIterable", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_async_iterable_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_async_iterable_type_checked(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_async_iterable_type_checked_cache {
            return t;
        }
        let t = self.get_global_type("AsyncIterable", 3 /*arity*/, true /*reportErrors*/);
        self.get_global_async_iterable_type_checked_cache = Some(t);
        t
    }

    pub(crate) fn get_global_async_iterable_iterator_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_async_iterable_iterator_type_cache {
            return t;
        }
        let t = self.get_global_type("AsyncIterableIterator", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_async_iterable_iterator_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_async_iterable_iterator_type_checked(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_async_iterable_iterator_type_checked_cache {
            return t;
        }
        let t = self.get_global_type("AsyncIterableIterator", 3 /*arity*/, true /*reportErrors*/);
        self.get_global_async_iterable_iterator_type_checked_cache = Some(t);
        t
    }

    pub(crate) fn get_global_async_iterator_object_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_async_iterator_object_type_cache {
            return t;
        }
        let t = self.get_global_type("AsyncIteratorObject", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_async_iterator_object_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_async_generator_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_async_generator_type_cache {
            return t;
        }
        let t = self.get_global_type("AsyncGenerator", 3 /*arity*/, false /*reportErrors*/);
        self.get_global_async_generator_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_iterator_yield_result_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_iterator_yield_result_type_cache {
            return t;
        }
        let t = self.get_global_type("IteratorYieldResult", 1 /*arity*/, false /*reportErrors*/);
        self.get_global_iterator_yield_result_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_iterator_return_result_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_iterator_return_result_type_cache {
            return t;
        }
        let t = self.get_global_type("IteratorReturnResult", 1 /*arity*/, false /*reportErrors*/);
        self.get_global_iterator_return_result_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_typed_property_descriptor_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_typed_property_descriptor_type_cache {
            return t;
        }
        let t = self.get_global_type("TypedPropertyDescriptor", 1 /*arity*/, true /*reportErrors*/);
        self.get_global_typed_property_descriptor_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_class_decorator_context_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_class_decorator_context_type_cache {
            return t;
        }
        let t = self.get_global_type("ClassDecoratorContext", 1 /*arity*/, true /*reportErrors*/);
        self.get_global_class_decorator_context_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_class_method_decorator_context_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_class_method_decorator_context_type_cache {
            return t;
        }
        let t = self.get_global_type("ClassMethodDecoratorContext", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_class_method_decorator_context_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_class_getter_decorator_context_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_class_getter_decorator_context_type_cache {
            return t;
        }
        let t = self.get_global_type("ClassGetterDecoratorContext", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_class_getter_decorator_context_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_class_setter_decorator_context_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_class_setter_decorator_context_type_cache {
            return t;
        }
        let t = self.get_global_type("ClassSetterDecoratorContext", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_class_setter_decorator_context_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_class_accessor_decorator_context_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_class_accessor_decorator_context_type_cache {
            return t;
        }
        let t = self.get_global_type("ClassAccessorDecoratorContext", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_class_accessor_decorator_context_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_class_accessor_decorator_target_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_class_accessor_decorator_target_type_cache {
            return t;
        }
        let t = self.get_global_type("ClassAccessorDecoratorTarget", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_class_accessor_decorator_target_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_class_accessor_decorator_result_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_class_accessor_decorator_result_type_cache {
            return t;
        }
        let t = self.get_global_type("ClassAccessorDecoratorResult", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_class_accessor_decorator_result_type_cache = Some(t);
        t
    }

    pub(crate) fn get_global_class_field_decorator_context_type(&mut self) -> P<Type> {
        if let Some(t) = self.get_global_class_field_decorator_context_type_cache {
            return t;
        }
        let t = self.get_global_type("ClassFieldDecoratorContext", 2 /*arity*/, true /*reportErrors*/);
        self.get_global_class_field_decorator_context_type_cache = Some(t);
        t
    }
}

/// Go `var primitiveTypeAliasSuggestions = sync.OnceValue(...)`. Go builds a map and ranges over it in random order;
/// a list in declaration order gives the spelling suggestion one answer when two of these tie.
pub fn primitive_type_alias_suggestions() -> &'static [(&'static str, P<Symbol>)] {
    static MAP: OnceLock<Vec<(&'static str, P<Symbol>)>> = OnceLock::new();
    MAP.get_or_init(|| {
        // Process-wide: never in a freeable region (language server).
        let _arena = tsrs_core::arena::enter_thread_arena();
        let mut result = Vec::new();
        for (primitive, builtin) in [
            ("string", "String"),
            ("number", "Number"),
            ("boolean", "Boolean"),
            ("object", "Object"),
            ("bigint", "BigInt"),
            ("symbol", "Symbol"),
        ] {
            let sym = Symbol::new(SymbolFlags::TypeAlias | SymbolFlags::Transient, primitive);
            result.push((builtin, sym));
        }
        result
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InheritanceInfo {
    pub prop: P<Symbol>,
    pub containing_type: P<Type>,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum UnusedKind {
    #[default]
    Local,
    Parameter,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct constructorAccessibilityError {
    pub kind: ModifierFlags,
    pub declaring_class: P<Type>,
}

/// Local state of `resolveCall`, passed as `&mut CallState`.
#[derive(Clone, Default)]
pub struct CallState {
    pub node: Option<P<Node>>,
    pub type_arguments: Vec<P<Node>>,
    pub args: Vec<P<Node>>,
    pub candidates: Vec<P<Signature>>,
    pub arg_check_mode: CheckMode,
    pub is_single_non_generic_candidate: bool,
    pub signature_help_trailing_comma: bool,
    pub recursive_resolution: bool,
    pub candidates_for_argument_error: Vec<P<Signature>>,
    pub candidate_for_argument_arity_error: Option<P<Signature>>,
    pub candidate_for_type_argument_error: Option<P<Signature>>,
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct PredicateSemantics: u32 {
        const None = 0;
        const Always = 1 << 0;
        const Never = 1 << 1;
        const Sometimes = Self::Always.bits() | Self::Never.bits();
    }
}

/// Go `*ExportCollision` in `ExportCollisionTable`: owned by the table here. The table is local to one
/// `getExportsOfModuleWorker` visit and nothing keeps a record past it, so an arena record (never freed) would only
/// leak; owned records and their strings are dropped with the table (notes/mem-export-star-scratch.md).
pub struct ExportCollision {
    pub specifier_text: String,
    pub exports_with_duplicate: Vec<P<Node>>,
}

pub type ExportCollisionTable = FxHashMap<String, ExportCollision>;

/// Go `type CacheHashKey xxh3.Uint128`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct CacheHashKey {
    pub hi: u64,
    pub lo: u64,
}

impl CacheHashKey {
    pub const fn from_u128(v: u128) -> CacheHashKey {
        CacheHashKey { hi: (v >> 64) as u64, lo: v as u64 }
    }
    /// Go `xxh3.Hash128(b)`.
    pub fn hash_128(b: &[u8]) -> CacheHashKey {
        CacheHashKey::from_u128(xxhash_rust::xxh3::xxh3_128(b))
    }
    /// Go `xxh3.HashString128(s)` (usable in constants).
    pub const fn hash_string_128(s: &str) -> CacheHashKey {
        CacheHashKey::from_u128(xxhash_rust::const_xxh3::xxh3_128(s.as_bytes()))
    }
}

/// Go `keyBuilder`; its methods are in checker_09.rs.
pub struct keyBuilder {
    pub inline_length: i32,
    pub overflow_buffer: Option<Vec<u8>>, // Go nil slice = no overflow yet
    pub inline_buffer: [u8; 192],
}

impl Default for keyBuilder {
    fn default() -> Self {
        keyBuilder { inline_length: 0, overflow_buffer: None, inline_buffer: [0; 192] }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum thisAssignmentDeclarationKind {
    #[default]
    None, // not (all) this.property assignments
    Typed, // typed; use the type annotation
    Constructor, // at least one in the constructor; use control flow
    Method, // methods only; look in base first, and if not found, union all declaration types plus undefined
}

/// Go `TupleNormalizer`; the `c` field is dropped (methods take `c: &mut Checker`).
#[derive(Default)]
pub struct TupleNormalizer {
    pub types: Vec<P<Type>>,
    pub infos: Vec<TupleElementInfo>,
    pub last_required_index: i32,
    pub first_rest_index: i32,
    pub last_optional_or_rest_index: i32,
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum UnionReduction {
    #[default]
    None,
    Literal,
    Subtype,
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct IntersectionFlags: u32 {
        const None = 0;
        const NoSupertypeReduction = 1 << 0;
        const NoConstraintReduction = 1 << 1;
    }
}

// Limits on the size of a template literal type produced by getTemplateLiteralType. Recursive instantiations
// such as `Recur<any, `${S}_${S}`>` double the text (or the number of placeholders) on every iteration and
// exhaust memory long before the tail recursion limit in getConditionalType is reached (see #63271).
pub const maxTemplateLiteralTypeLength: i32 = 50_000_000;
pub const maxTemplateLiteralTypeSpans: i32 = 100_000;

/// Go `ObjectLiteralDiscriminator`; the `c` field is dropped (methods take `c: &mut Checker`).
#[derive(Default)]
pub struct ObjectLiteralDiscriminator {
    pub props: Vec<P<Node>>,
    pub members: Vec<P<Symbol>>,
}


impl Checker {
    /// tsrs-only (`--maxMemory`, notes/mem-recycle-checkers.md): entries in the relation caches.
    pub fn relation_cache_entries(&self) -> usize {
        [self.subtype_relation, self.strict_subtype_relation, self.assignable_relation, self.comparable_relation, self.identity_relation]
            .iter()
            .map(|r| r.size() as usize)
            .sum()
    }

    /// tsrs-only (`--maxMemory`): empties the relation caches between two files (results are recomputed on
    /// demand).
    pub fn clear_relation_caches(&mut self) {
        for r in [self.subtype_relation, self.strict_subtype_relation, self.assignable_relation, self.comparable_relation, self.identity_relation] {
            r.clear();
        }
    }
}
