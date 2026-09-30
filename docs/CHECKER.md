# `tsrs_checker` design contract

Read `docs/PORTING.md` and `docs/AST.md` first. This file fixes the checker-specific
decisions so that ~25 agents can port `ts-ref/tsc/internal/checker/` in parallel.
The `checker-foundation` agent implements the data model and keeps this file accurate.

## Files

| Go file | Rust |
| --- | --- |
| `types.go`, `links.go`, `mapper.go`, top of `checker.go` (declarations, `Checker` struct, `NewChecker`, global init) | `types.rs`, `links.rs`, `mapper.rs`, `checker.rs` (foundation) |
| non-function declarations (types/consts/vars) of `relater.go`, `flow.go`, `inference.go`, `jsx.go`, `utilities.go` | `relater_types.rs`, `flow_types.rs`, `inference_types.rs`, `jsx_types.rs`, `utilities_types.rs` (foundation; `grammarchecks.go`/`exports.go` have none) |
| `Program` interface | `program.rs` (foundation) |
| `checker.go` (32.6k lines) | `checker_01.rs` … `checker_15.rs` by Go line range (table below), all `impl Checker` |
| `relater.go` | `relater_1.rs` (1–2579), `relater_2.rs` (2580–end) |
| `flow.go`, `inference.go`, `grammarchecks.go`, `utilities.go`, `jsx.go`, `exports.go`, `jsdoc.go` | same name `.rs` |
| `printer.go`, `symbolaccessibility.go`, `symboltracker.go` | `printer.rs` — type/symbol/signature -> string for messages |
| `nodebuilder.go`, `nodebuilderimpl.go`, `nodebuilderscopes.go` | `nodebuilder.rs`, `nodebuilderimpl_1.rs` (1–1850), `nodebuilderimpl_2.rs` (1851–end), `nodebuilderscopes.rs`; data model in `nodebuilder_types.rs` + `printer_types.rs` (section "Node builder") |
| `emitresolver.go`, `services.go`, `nodecopy.go`, `pseudotypenodebuilder.go`, `nodebuilder_hover.go`, `tracer.go` | not ported (but see "Node builder" for the few callees the node builder needs) |
| `../evaluator/evaluator.go` | `evaluator.rs` (foundation) |

`checker.go` line ranges: 01: 1–2172, 02: 2173–4320, 03: 4321–6526, 04: 6527–8748, 05: 8749–10856,
06: 10857–13068, 07: 13069–15238, 08: 15239–17432, 09: 17433–19607, 10: 19608–21763, 11: 21764–23953,
12: 23954–26122, 13: 26123–28299, 14: 28300–30509, 15: 30510–32667. A declaration belongs to the file whose
range contains its first line.

## Checker

`pub struct Checker` (checker.rs) has every Go field (snake_case: `TypeCount` -> `type_count`,
`ReverseMappedSymbolLinks` -> `reverse_mapped_symbol_links`), plain (non-`Cell`) types; all methods take `&mut self`.
Go `c.foo(x)` -> `self.foo(x)`. `new_checker(program: &'static dyn Program) -> Box<Checker>` is Go `NewChecker`
(tracer and mutex dropped). Before Go assigns them, pointer fields hold placeholder objects (never observable after
`new_checker` returns).

- **Function-valued fields** are methods with the snake_case name and the same arguments, all implemented in checker.rs
  (none are generated): `compare_symbols`, `compare_symbol_chains`, `resolve_name`, `resolve_name_for_symbol_suggestion`
  (backed by the fields `name_resolver` / `name_resolver_for_suggestion: P<NameResolver<Checker>>`), `evaluate`,
  `is_primitive_or_object_or_empty_type`, `contains_missing_type`, `could_contain_type_variables`,
  `is_string_index_signature_only_type`, `mark_node_assignments`, `compare_types_assignable`, and every lazily resolved
  global: `get_global_es_symbol_type`, `get_global_big_int_type`, `get_global_import_meta_type`,
  `get_global_import_attributes_type[_checked]`, `get_global_non_nullable_type_alias_or_nil`, `get_global_extract_symbol`,
  `get_global_disposable_type`, `get_global_async_disposable_type`, `get_global_awaited_symbol[_or_nil]`,
  `get_global_nan_symbol_or_nil`, `get_global_record_symbol`, `get_global_template_strings_array_type`,
  `get_global_es_symbol_constructor_symbol_or_nil`, `get_global_es_symbol_constructor_type_symbol_or_nil`,
  `get_global_import_call_options_type[_checked]`, `get_global_promise_type[_checked]`, `get_global_promise_like_type`,
  `get_global_promise_constructor_symbol[_or_nil]`, `get_global_omit_symbol`, `get_global_no_infer_symbol_or_nil`,
  `get_global_{iterator,iterable,iterable_iterator,iterator_object,generator}_type` (+ `_checked` variants as in Go),
  the `get_global_async_*` counterparts, `get_global_iterator_{yield,return}_result_type`,
  `get_global_typed_property_descriptor_type`, `get_global_class_{,method_,getter_,setter_,accessor_,field_}decorator_context_type`,
  `get_global_class_accessor_decorator_{target,result}_type`. Type getters return `P<Type>`, symbol getters
  `Option<P<Symbol>>`; each caches in a `<name>_cache` field (Go `core.Memoize`, nil results cached too).
  `get_global_types(names, arity, report_errors)` is Go's `getGlobalTypesResolver` closure body.
  When Go passes one as a value, pass a closure `|c, t| c.is_primitive_or_object_or_empty_type(t)`;
  `c.compareTypesAssignable` as a `TypeComparer` value is `self.compare_types_assignable_comparer()`.
- `IterationTypesResolver` (`P<…>`, fields `sync_iteration_types_resolver` / `async_iteration_types_resolver`): its Go
  function fields are methods taking the checker: `resolver.getGlobalIterableType()` -> `resolver.get_global_iterable_type(self)`,
  `resolver.resolveIterationType(t, n)` -> `resolver.resolve_iteration_type(self, t, n)`, `get_global_builtin_iterator_types(self)`.
- **Callbacks** passed to checker methods take the checker as first parameter (PORTING.md rule):
  `impl FnMut(&mut Checker, P<Type>) -> bool`. Closures never capture `self`.
- **Deferred work**: `[]func()` fields are `Vec<Box<dyn FnOnce(&mut Checker)>>`.
- **Program**: `pub trait Program: Send + Sync` (program.rs) with the Go `checker.Program` methods the checker uses plus
  the four host methods it calls (`use_case_sensitive_file_names`, `get_current_directory`,
  `get_default_resolution_mode_for_file`, `get_mode_for_usage_location`). The checker holds `program: &'static dyn Program`.
  `ast.HasFileName` parameters are `P<SourceFile>`, `tspath.Path` parameters `&Path`. Project-reference results use a
  placeholder: `SourceOutputAndProjectReference { source, output_dts, resolved: &'static dyn ProjectReferenceCommandLine }`
  (`compiler_options()`, `common_source_directory()`) instead of `*tsoptions.ParsedCommandLine`.
- Arena objects never point back to the checker (Go's `Type.checker` field is dropped; Go `t.checker.foo()` becomes a
  call on the checker you already have, e.g. `keyBuilder::write_generic_type_references` needs a `c` argument).
- **Helper structs that hold `c *Checker`** (`Relater`, `TypeDiscriminator`, `ObjectLiteralDiscriminator`,
  `TupleNormalizer`) drop the `c` field; every method takes `c: &mut Checker` as the first parameter after the receiver,
  and Go `r.c.foo()` -> `c.foo()`. `Relater` is an arena handle: `P<Relater>` with `Cell`/`RefCell` fields, `&self`
  methods, pooled via `Checker::free_relater` exactly like Go's `getRelater`/`putRelater`. This is required because Go
  stores relater methods as values (`newInferenceContext(..., r.isRelatedToWorker)`, `findMatchingDiscriminantType(s, t,
  r.isRelatedToSimple)`): in Rust that is `|c, s, t| r.is_related_to_simple(c, s, t)` with `r: P<Relater>` copied into
  the closure. `trait Discriminator { len, name, matches }` — all `(&mut self, c: &mut Checker, …)`; a Go
  `Discriminator` parameter is `&mut dyn Discriminator`. `TypeDiscriminator<'a>` holds
  `is_related_to: &'a mut dyn FnMut(&mut Checker, P<Type>, P<Type>) -> Ternary`.
- `FlowState` and `InferenceState` are arena handles (`P<…>`, `Cell` fields) pooled via `free_flow_state` /
  `freeinference_state` like Go. `CallState` is a plain struct passed as `&mut CallState`.
- **Named func types**: `TypeComparer` = `&'static dyn Fn(&mut Checker, P<Type>, P<Type>, bool) -> Ternary` (Copy; build
  with `type_comparer(|c, s, t, report_errors| …)`); `ErrorReporter<'a>` = `&'a mut dyn FnMut(&mut Checker, &'static Message,
  &[&dyn Display])` (usually `Option<ErrorReporter>`).
- Dropped Go fields: `symbolArena`/`signatureArena`/`indexInfoArena` (use `P::new`), `tracer` (all tracing blocks are
  dropped), `ctx` (`isCanceled` is `false`; `was_canceled` kept), `mu`, `emitResolver`/`emitResolverOnce`,
  and the never-assigned `getGlobalClassAccessorDecoratorContxtType`. (`typeToStringNodebuilder` is kept:
  `type_to_string_nodebuilder: Option<P<NodeBuilder>>`.)
  `ambientModulesOnce` is a `bool`; nil-able maps that Go resets or tests for nil are `Option<FxHashMap>`
  (`flow_type_cache`, `packages_map`). Go `map[string]V` fields use `String` keys (the generator maps `map[string]V`
  the same way); `collections.Set[T]` is `tsrs_core::collections::Set<T>`.
- Package-level vars: `typeofNEFacts` / `intrinsicTypeKinds` are `LazyLock<FxHashMap<&str, _>>` (`typeofNEFacts.get(text)`),
  `JsxNames.intrinsic_elements` / `ReactNames.fragment` (snake_case fields), `LanguageFeatureMinimumTarget.class_fields`,
  `primitive_type_alias_suggestions()` and `get_feature_map()` are functions, `SignatureKeyErased` & co. and
  `nonDottedNameCacheKey` are `const CacheHashKey`. Go-named lowercase types keep their Go names (`keyBuilder`,
  `errorState`, `orderedSet`, `thisAssignmentDeclarationKind` with variants `None/Typed/Constructor/Method`).
  `symbolTableID` (a `u64` alias) is declared in checker.rs; the `stKind*` constants belong to printer.rs.
- printer_types.rs holds what printer.rs signatures need from other packages: `nodebuilder.Flags` / `InternalFlags`,
  `trait SymbolTracker`, `SymbolAccessibility(Result)` (from `printer`), `VerbosityContext`,
  `accessibleSymbolChainContext`, `SymbolTrackerImpl`, the `tsrs_printer` seam (section "Node builder") and the empty
  placeholder `EmitResolver`. Go `context.Context` parameters are the unit struct `Context`; `iter.Seq[T]` is
  `Seq<T> = Vec<T>`.
- `evaluator.go` is ported as `evaluator.rs`: `evaluator::Result { value: LiteralValue, … }` (always written with the
  module path; it would shadow `std::result::Result`), `evaluator::evaluate(host, evaluate_entity, outer_kinds, expr,
  location)`, `evaluator::any_to_string`, `evaluator::is_truthy`. The checker calls `self.evaluate(expr, location)`.

## Node builder

Error messages render types, symbols and signatures through Go's pipeline: `c.typeToString` & co. (printer.rs) ->
`NodeBuilder` entry point (nodebuilder.rs) -> `NodeBuilderImpl` builds synthetic type nodes (nodebuilderimpl_*.rs,
nodebuilderscopes.rs; symbol chains via symbolaccessibility.go in printer.rs) -> `tsrs_printer::Printer` prints them.
Baselines compare the text byte for byte, so all of it is ported faithfully.

**Handles and receivers.** Everything is re-entrant (a diagnostic reported during lazy resolution formats a type with
the *same* cached builder, which pushes a new context), so the node builder objects are arena handles like `Relater`:

| Go | Rust | methods |
| --- | --- | --- |
| `*NodeBuilder` | `P<NodeBuilder>` { `ctx_stack: RefCell<Vec<Option<P<NodeBuilderContext>>>>`, `host`, `impl_: P<NodeBuilderImpl>`, `verbosity: Cell<Option<P<VerbosityContext>>>` } | `&self`; entry points that reach the checker take `c: &mut Checker` (`b.impl_.type_to_type_node(c, t)`) |
| `*NodeBuilderImpl` | `P<NodeBuilderImpl>` { `f`, `e`, `pc`, `links`, `symbol_links`, `ctx: Cell<Option<P<NodeBuilderContext>>>`, `clone_binding_name_visitor`, `id_to_symbol: RefCell<FxHashMap<P<Node>, P<Symbol>>>` } | `&self, c: &mut Checker` (Go `b.ch.foo()` -> `c.foo()`) |
| `*NodeBuilderContext` | `P<NodeBuilderContext>`, every field `Cell`/`RefCell` | — |
| `*SymbolTrackerImpl` | `P<SymbolTrackerImpl>`, used as `&'static dyn SymbolTracker` (`p.get()`) | `&self` |
| `*VerbosityContext` | `Option<P<VerbosityContext>>`, `Cell` fields (`vc.truncated.set(true)`) | — |

- `b.ctx` is `b.ctx()` (unwraps; Go never reads it while nil); save/restore/swap with `b.ctx.get()`/`b.ctx.set(..)`.
  Fields: `b.ctx().flags.get()`, `ctx.approximate_length.set(ctx.approximate_length.get() + 3)`,
  `ctx.type_stack.borrow_mut().push(Some(t))`. Never hold a `borrow()` of a context collection across a call back
  into the builder or the checker (copy out or clone the small Vec first).
- `NodeBuilderContext::new(host)` is the Go zero value; `enterContext` builds
  `P::new(NodeBuilderContext { flags: Cell::new(flags), …, ..NodeBuilderContext::new(b.host) })`.
- Field types (nodebuilder_types.rs): `tracker: Cell<Option<&'static dyn SymbolTracker>>` (Some after `enter_context`),
  `type_stack: RefCell<Vec<Option<P<Type>>>>` (hover pushes a nil sentinel), `infer_type_parameters: Cell<&'static [P<Type>]>`
  (Go assigns `t.root.inferTypeParameters`), `visited_types: RefCell<Set<TypeId>>`, `symbol_depth`,
  `enclosing_symbol_types`, `remapped_symbol_references` (`RefCell<FxHashMap<…>>`), `tracked_symbols:
  RefCell<Vec<P<TrackedSymbolArgs>>>` (Go's `= nil` + restore is `mem::take` + put back), the four per-scope
  `CopyOnWriteMap`/`CopyOnWriteSet` (`tsrs_core::collections`, `enter_scope()` returns a scope token for
  `exit_scope(token)`; `cloneNodeBuilderContext` returns a `Box<dyn FnMut()>` that restores all four).
- Caches: `b.links: LinkStore<Node, NodeBuilderLinks>` (`serialized_types: GoMap<CompositeTypeCacheIdentity,
  P<SerializedTypeEntry>>`, `fake_scope_for_signature_declaration: Cell<Option<&'static str>>`), `b.symbol_links:
  LinkStore<Symbol, NodeBuilderSymbolLinks>` (`specifier_cache: GoMap<ModeAwareCacheKey, moduleSpecifierResult>`,
  `moduleSpecifierResult { specifier: &'static str, import_attributes_type }` is `Copy`). In stub files write
  `crate::LinkStore` if you need the type name (`tsrs_core::LinkStore` is also glob-imported).
- Returned cleanup funcs (`saveRestoreFlags`, `enterNewScope`, `addSymbolTypeToContext`) are
  `Box<dyn FnMut(&mut Checker)>`; call them as `cleanup(c)`. `b.ctx()` captured by value (`P`) inside them is fine.
- Constructors are ported: `new_node_builder[_ex](c, e[, id_to_symbol])`, `new_node_builder_impl`,
  `new_symbol_tracker_impl` (Go's `tracker.(*SymbolTrackerImpl)` is `tracker.as_symbol_tracker_impl()`), and
  `NodeBuilder::emit_context()`. `Checker::get_node_builder` caches in `self.type_to_string_nodebuilder`.

**Factory and emit context.** `b.f` is a `tsrs_ast::NodeFactory` handle sharing the emit context's factory (its
`on_create` hook marks nodes `Synthesized`, which `ast.NodeIsSynthesized`-style checks rely on). All factory methods
take `&self`, so Go's nested calls port as written: `b.f.new_type_reference_node(b.f.new_identifier(n), None)`,
`b.f.new_node_list(vec![…])`, `node.clone_node(&b.f)`, `b.f.deep_clone_node(Some(n))`. `ast.ReplaceModifiers(f, …)`
is `replace_modifiers(&b.f, node, modifiers)`, `ast.CreateModifiersFromModifierFlags(flags, f.NewModifier)` is
`create_modifiers_from_modifier_flags(flags, |k| b.f.new_modifier(k))`. `b.e` is `P<EmitContext>`; its methods take
`&self`: `b.e.add_emit_flags(node, EmitFlags::NoAsciiEscaping)`, `set_emit_flags`, `original(node) -> Option`,
`most_original(Option<P<Node>>) -> Option<P<Node>>` (Go nil in, nil out), `set_original_ex(node, original, allow_overwrite)`,
`assign_comment_range(to, from)`, `add_synthetic_leading_comment(node, kind, text, has_trailing_new_line) -> P<Node>`
(and `_trailing_`). `printer.EFSingleLine` -> `EmitFlags::SingleLine`.

**The `tsrs_printer` seam.** The checker names the printer crate only through the re-exports at the top of
printer_types.rs: `EmitContext`, `EmitFlags`, `EmitTextWriter`, `Printer`, `PrinterOptions`, `PrintHandlers`,
`new_emit_context()`, `new_printer(options, handlers, Some(emit_context)) -> Printer`, `new_text_writer(new_line,
indent) -> Box<dyn EmitTextWriter>`, `get_single_line_string_writer() -> Box<dyn EmitTextWriter>` (Go's pool release
func dropped), `Printer::write(&mut self, node, source_file: Option<P<SourceFile>>, writer: &mut dyn EmitTextWriter)`
(source-map argument dropped), `Printer::emit(&mut self, node, source_file) -> String`, `writer.string()` (Go
`String()`). Until `tsrs_printer` merges they come from printer_standin.rs (`// STAND-IN`, placeholder bodies).
`printer::NodeFactory` is reached as `e.factory` (it derefs to `ast::NodeFactory`) and is not re-exported.

**Known gaps (callees outside the ported files).** The node builder calls into files and packages that are not
ported; body agents must keep the call structure and list these in their notes rather than invent behavior:
`GetEmitResolver().hasVisibleDeclarations` (symbolaccessibility `isAnySymbolAccessible`), `.isEntityNameVisible`,
`.requiresAddingImplicitUndefined` (emitresolver.go); `tryReuseExistingNodeHelper`, `reuseNode`,
`tryJSTypeNodeToTypeNode` (nodecopy.go); `pseudoTypeEquivalentToType`, `pseudoTypeToType`,
`pseudoTypeToNodeWithCheckerFallback`, `pseudoReturnTypeMatchesPredicate` (pseudotypenodebuilder.go) and the
`internal/pseudochecker` package (`b.pc` is a placeholder `P<PseudoChecker>`); `modulespecifiers.GetModuleSpecifiers` /
`CountPathComponents` (not ported); hover-only `expandSymbolForHover`, `IsLib{Symbol,Type}ForHoverVerbosity`.
`clone_binding_name_visitor`: Go's visitor calls `b.cloneBindingName`, which needs `c`; a `'static` `VisitFn` cannot
capture `&mut Checker`, so `new_node_builder_impl` leaves it `None` and the `clone_binding_name` port must choose a
visiting strategy (flag it in your notes).

## Types

```rust
pub struct Type {
    pub flags: Cell<TypeFlags>,
    pub object_flags: Cell<ObjectFlags>,
    pub id: TypeId,                         // u32 newtype
    pub symbol: Cell<Option<P<Symbol>>>,
    pub alias: Cell<Option<P<TypeAlias>>>,
    pub data: TypeData,
}
#[derive(Clone, Copy)]
pub enum TypeData { Intrinsic(&'static IntrinsicType), Literal(&'static LiteralType), UniqueESSymbol(..),
                    Object(&'static ObjectType) /* anonymous */, TypeReference(..), Interface(..), Tuple(..),
                    InstantiationExpression(..), Mapped(..), ReverseMapped(..), EvolvingArray(..), Union(..),
                    Intersection(..), TypeParameter(..), Index(..), IndexedAccess(..), TemplateLiteral(..),
                    StringMapping(..), Substitution(..), Conditional(..) }
```

Types are always handled as `P<Type>`; the payload is a separate arena allocation (`TypeData::Tuple(alloc(TupleType::default()))`)
so `Type` stays small. `t.flags()`/`t.object_flags()`/`t.symbol()`/`t.alias()` getters return the value. Go's `TypeBase`
(which embeds the header) has no Rust counterpart; header fields are only on `Type`.

Go models the type hierarchy by struct embedding (`TupleType` ⊃ `InterfaceType` ⊃ `TypeReference` ⊃ `ObjectType` ⊃
`StructuredType` ⊃ `ConstrainedType`; `UnionType`/`IntersectionType` ⊃ `UnionOrIntersectionType` ⊃ `StructuredType`;
the other instantiable kinds ⊃ `ConstrainedType`). Rust keeps the same structs, each holding its Go-embedded parent as its
first field, named after the parent in snake_case (`constrained_type`, `structured_type`, `object_type`, `type_reference`,
`interface_type`, `union_or_intersection_type`), and implementing `Deref` to it, so promoted fields and methods resolve
exactly like Go: `t.as_interface_type().resolved_type_arguments.get()`, `t.as_object_type().target.get()`,
`t.as_structured_type().properties.get()`.

- `as_<struct>()` accessors return `&'static <Struct>` and panic on a shape mismatch. The six Go returns-nil casts
  (`AsConstrainedType`, `AsStructuredType`, `AsObjectType`, `AsTypeReference`, `AsInterfaceType`, `AsUnionOrIntersectionType`)
  also exist as `try_as_…() -> Option<&'static …>` (on `Type` and on `TypeData` as `as_…`).
- **Uniform mutability rule**: every field of every arena struct from types.go/checker.go (type payloads, `Signature`,
  `IndexInfo`, `TypePredicate`, `TypeAlias`, `ConditionalRoot`, `CompositeSignature`, `InferenceContext`, `InferenceInfo`,
  `WideningContext`, `Relater`, `FlowState`, `InferenceState`) and of every links struct is a `Cell` (Copy data, slices
  `Cell<&'static [T]>`, strings `Cell<&'static str>`), a `RefCell<Vec<…>>` (slices Go appends to), or a `GoMap` —
  even fields Go only sets at construction, because Go creates them empty and assigns afterwards. All derive `Default`,
  so construct with `Signature { flags: Cell::new(f), ..Default::default() }` or `default()` + `.set()`.
  Exceptions: `Type.id`/`Type.data` (plain), and value structs (`TupleElementInfo`, `IterationTypes`, `FlowType`, keys…)
  which are plain `Copy` structs. Go nil-vs-empty slices that matter are `Option<&'static [T]>`
  (`VarianceLinks.variances`, `WideningContext.siblings`, `ContainingSymbolLinks.extended_containers`).
- **`GoMap<K, V>`** (types.rs): a Go map field — pointer-sized, nil until `make()`/`set()`. `get(&k) -> Option<V>`,
  `has`, `set(k, v)` (creates the map if nil), `delete`, `len`, `is_nil`, `make`, `clear`, `entries()` snapshot,
  `assign(map)` for `x.m = freshMap`, `get_ref`/`set_ref` for aliasing. Used for `instantiations`, `constituent_map`,
  `TypeAliasLinks.instantiations`, `ConditionalRoot.instantiations`, `InferenceState.visited`, … `ast.SymbolTable` fields
  are `Cell<Option<P<SymbolTable>>>`.
- Go `any` literal values are `LiteralValue { String(&'static str), Number(Number), Boolean(bool), BigInt(PseudoBigInt) }`
  (Copy, Eq, Hash); Go's nil is `Option::None`, so a nil-able `any` is `Option<LiteralValue>` (the generator's usual
  pointer rule): `LiteralType.value: Cell<Option<LiteralValue>>`, `evaluator::Result.value: Option<LiteralValue>`,
  `EnumLiteralKey.value: LiteralValue`. Stored diagnostic arguments (Go `[]any` in `ErrorChain.args`,
  `DiagnosticAndArguments`, `DiagnosticDetails`) are pre-formatted `Vec<String>`.
- `Ternary` is a Copy newtype over `i8` with `Ternary::{False, Unknown, Maybe, True}` and `&`, `|`, `&=`, `|=`.
- Go methods on types.go structs exist with snake_case names (`call_signatures()`, `type_parameters()`,
  `outer_type_parameters()`, `element_flags()`, `type_()` for `TypePredicate.Type()`, …). `t.Distributed()` is
  `t.distributed()` via `trait TypeExt` on `P<Type>`; Go's nil-receiver `alias.Symbol()` works on
  `Option<P<TypeAlias>>` via `TypeAliasOptExt`. `TypeFlags`/`VarianceFlags` implement `Display` (Go `String()`),
  `format_type_flags(flags)`.
- `TypeMapper { data: TypeMapperData }` with `enum TypeMapperData { Simple, Array, ArrayToSingle, Deferred { targets:
  Vec<Box<dyn Fn(&mut Checker) -> P<Type>>> }, Function { f: fn(&mut Checker, P<Type>) -> P<Type> }, Merged, Composite,
  Inference { n, fixing } }`. Go `m.Map(t)` -> `m.map(c, t)`, `m.Kind()`, `m.MapsThisOnly()`. mapper.go free functions
  keep their names (`new_simple_type_mapper`, `new_array_type_mapper`, `new_array_to_single_type_mapper`,
  `new_deferred_type_mapper`, `new_function_type_mapper(|c, t| c.x(t))`, `new_merged_type_mapper`,
  `new_composite_type_mapper`, `new_type_mapper`, `merge_type_mappers`, `prepend_type_mapping`, `append_type_mapping`);
  `getMappedType(t, m)` needs the checker and is the method `self.get_mapped_type(t, m)`; `combine_type_mappers`,
  `map_type_with_composite_mapper`, `new_backreference_mapper`, `new_inference_type_mapper` are checker methods as in Go.
- Hash keys: `CacheHashKey { hi: u64, lo: u64 }` (Go `xxh3.Uint128`, xxh3-128 via `xxhash-rust`):
  `CacheHashKey::hash_128(bytes)` = Go `xxh3.Hash128`, `CacheHashKey::hash_string_128(s)` = `xxh3.HashString128` (const fn),
  `is_zero()`. `keyBuilder` (struct in checker.rs, methods ported in checker_09.rs) keeps Go's byte layout
  (little-endian widths, 192-byte inline buffer, `overflow_buffer: Option<Vec<u8>>`).

## Links

Go `c.valueSymbolLinks.Get(symbol)` returns a pointer that is mutated in place. Rust: every links struct has `Cell`
fields and lives in the arena; `self.value_symbol_links.get(symbol)` returns `P<ValueSymbolLinks>` (created on first use),
`try_get(symbol) -> Option<P<…>>`, `has(symbol) -> bool`. Because `P` is `Copy`, no borrow of `self` is held:

```rust
let links = self.value_symbol_links.get(symbol);
if links.resolved_type.get().is_none() {
    let t = self.get_type_of_variable_or_parameter_or_property_worker(symbol);
    links.resolved_type.set(Some(t));
}
```

All link stores use one generic `LinkStore<K, V>` keyed by `P<K>` (`FxHashMap<P<K>, P<V>>`), whatever store flavor Go
uses (`LinkStore<Node, NodeLinks>`, `LinkStore<Symbol, ValueSymbolLinks>`, `LinkStore<SourceFile, SourceFileLinks>`).
`MembersAndExportsLinks` derefs to `[Cell<Option<P<SymbolTable>>>; 2]`: `links[kind as usize].get()`.

## Name resolution in module files

Every stub file starts with `use crate::*; use tsrs_ast::*; use tsrs_core::*;` plus explicit `use tsrs_ast as ast;` and
`use tsrs_diagnostics as diagnostics;`. lib.rs re-exports the data model and every stub module (`pub(crate) use
checker_01::*` …), so free functions of other checker files resolve unqualified. A few checker free functions have the
same name as a tsrs_ast function (`is_binary_operator`, `is_assignment_operator_or_higher`, `is_type_assertion`,
`entity_name_to_string`, `is_node_descendant_of`, `is_instantiated_module`, …); an unqualified call from another file is
ambiguous (E0659): write `crate::name(..)` for the checker's (what Go's unqualified call means) or `ast::name(..)`.

## Diagnostics

`self.error(node, &diagnostics::X, &[&a, &b]) -> P<Diagnostic>`; same pattern for `error_or_suggestion`,
`error_at`, `new_diagnostic_for_node(node, &diagnostics::X, &[…])`, `add_related_info`, error chains
(`P<ErrorChain>`/`chain_diagnostic_messages`) — names and argument order as Go.
Types/symbols/signatures in messages go through `self.type_to_string(t)`, `self.symbol_to_string(s)`,
`self.signature_to_string(sig)` etc. (Go names, snake_case, returning `String`).

## Conventions for nil

`Option<P<Type>>` only where Go can really hold nil. Signatures are generated mechanically from the Go source
(see `tools/gosig`): a pointer parameter is `Option` iff some call site passes `nil` or the body compares it with `nil`;
a pointer result is `Option` iff the body can return `nil`. **Do not change a generated signature** unless it is
wrong; if you must, change it in your file, fix what you can see, and list it in your report.

## Generated signatures (`tools/gosig`)

Function signatures for the whole checker package are generated mechanically from the Go source into stub files
(`todo!()` bodies) before bodies are ported, so every callee signature can be looked up (`docs/sigs/checker.txt` or grep).
Mapping used by the generator (deviations from PORTING.md are deliberate, for determinism):

- Go `int` -> `i32` always (cast at use sites: `x as usize`, `v.len() as i32`).
- Go `(T, bool)` / multiple results -> the same tuple `(T, bool)`; no `Option` conversion.
- `[]T` parameter -> `&[T]`; `[]T` result -> `Vec<T>` (callers needing a stored slice call `alloc_slice`); a result that is
  `nil`-distinguishable is `Option<Vec<T>>` only if the generator marks it.
- `string` parameter -> `&str`; `string` result -> `String`.
- `func(...)` parameter of a `Checker` method -> `impl FnMut(&mut Checker, ...) -> R` (checker first); elsewhere `impl FnMut(...) -> R`.
- `args ...any` -> `args: &[&dyn std::fmt::Display]`.
- Each stub carries a terse origin marker comment (`// checker.go:1234`); keep it when you fill in the body.
