# `tsrs_checker` design contract

Read `docs/PORTING.md` and `docs/AST.md` first. This file fixes the checker-specific
decisions so that ~25 agents can port `ts-ref/tsc/internal/checker/` in parallel.
The `checker-foundation` agent implements the data model and keeps this file accurate.

## Files

| Go file | Rust |
| --- | --- |
| `types.go`, `links.go`, `mapper.go`, top of `checker.go` (declarations, `Checker` struct, `NewChecker`, global init) | `types.rs`, `links.rs`, `mapper.rs`, `checker.rs` (foundation) |
| `checker.go` (32.6k lines) | `checker_01.rs` … `checker_15.rs` by Go line range (table below), all `impl Checker` |
| `relater.go` | `relater_1.rs` (1–2579), `relater_2.rs` (2580–end) |
| `flow.go`, `inference.go`, `grammarchecks.go`, `utilities.go`, `jsx.go`, `exports.go`, `jsdoc.go` | same name `.rs` |
| `printer.go`, `nodebuilder*.go`, `symbolaccessibility.go`, `symboltracker.go` | `printer.rs` (+ later a faithful node-builder port) — type/symbol/signature -> string for messages |
| `emitresolver.go`, `services.go`, `nodecopy.go`, `pseudotypenodebuilder.go`, `nodebuilder_hover.go`, `tracer.go` | not ported |
| `../evaluator/evaluator.go` | `evaluator.rs` |

`checker.go` line ranges: 01: 1–2172, 02: 2173–4320, 03: 4321–6526, 04: 6527–8748, 05: 8749–10856,
06: 10857–13068, 07: 13069–15238, 08: 15239–17432, 09: 17433–19607, 10: 19608–21763, 11: 21764–23953,
12: 23954–26122, 13: 26123–28299, 14: 28300–30509, 15: 30510–32667. A declaration belongs to the file whose
range contains its first line.

## Checker

`pub struct Checker` has every Go field (snake_case), plain (non-`Cell`) types; all methods take `&mut self`.
Go `c.foo(x)` -> `self.foo(x)`.

- **Function-valued fields** (`resolveName`, `getGlobalPromiseType`, `isPrimitiveOrObjectOrEmptyType`, `compareSymbols`, …)
  are *methods* in Rust with the snake_case name and the same arguments: `c.getGlobalPromiseType()` ->
  `self.get_global_promise_type()`. Their caches live in ordinary fields. When Go passes one as a value, pass a closure
  `|c, t| c.is_primitive_or_object_or_empty_type(t)`.
- **Callbacks** passed to checker methods take the checker as first parameter (PORTING.md rule):
  `impl FnMut(&mut Checker, P<Type>) -> bool`. Closures never capture `self`.
- **Deferred work**: `[]func()` fields are `Vec<Box<dyn FnOnce(&mut Checker)>>`.
- **Program**: `pub trait Program: Send + Sync` (in `program.rs`) mirrors the Go `checker.Program` interface with
  snake_case method names; the checker holds `program: &'static dyn Program`. `ast.HasFileName` parameters are `P<SourceFile>`.
- Arena objects never point back to the checker (Go's `Type.checker` field is dropped).
- Short-lived Go helper structs that hold `c *Checker` (`Relater`, inference state, `typeFacts` walkers, iteration helpers …)
  become `struct Relater<'c> { c: &'c mut Checker, … }`, constructed for the duration of the operation. This is the one
  place lifetimes appear. Inside them, Go `r.c.foo()` -> `self.c.foo()`. Go's free-list pooling of these is dropped.

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
pub enum TypeData { Intrinsic(IntrinsicType), Literal(LiteralType), Object(ObjectType), TypeReference(TypeReference),
                    Interface(InterfaceType), Tuple(TupleType), Mapped(MappedType), Union(UnionType), … }
```

Types are always handled as `P<Type>`. `t.flags()`/`t.object_flags()` getters return the value.

Go models the type hierarchy by struct embedding (`TupleType` ⊃ `InterfaceType` ⊃ `TypeReference` ⊃ `ObjectType` ⊃
`StructuredType` ⊃ `ConstrainedType`). Rust keeps the same structs, each holding its Go-embedded parent as its first
field and implementing `Deref` to it, so promoted fields and methods resolve exactly like Go:
`t.as_interface_type().resolved_type_arguments.get()`, `t.as_object_type().target.get()`,
`t.as_structured_type().properties.get()`.

- `as_<struct>()` accessors (`as_object_type`, `as_type_reference`, `as_interface_type`, `as_tuple_type`, `as_union_type`,
  `as_structured_type`, `as_constrained_type`, `as_literal_type`, `as_type_parameter`, `as_conditional_type`, …) return
  `&'static <Struct>` and panic when the type is not of that shape — except the ones Go implements on `TypeData` that
  return nil for other shapes (`AsConstrainedType`, `AsStructuredType`, `AsObjectType`, `AsTypeReference`,
  `AsInterfaceType`, `AsUnionOrIntersectionType`), which also exist as `try_as_…() -> Option<&'static …>`.
- Every field that Go assigns after the type is created (nearly all of them: resolved members, targets, caches) is a
  `Cell`/`RefCell` named like the Go field. Read with `.get()`, write with `.set()`.
  Slices: `Cell<&'static [P<Type>]>`. Maps: `RefCell<FxHashMap<…>>`.
- `LiteralType.value` (Go `any`) is `enum LiteralValue { None, String(&'static str), Number(jsnum::Number), Boolean(bool), BigInt(PseudoBigInt) }`.
- `Signature`, `IndexInfo`, `TypePredicate`, `TypeAlias`, `TypeMapper`, `InferenceContext`, `InferenceInfo`, `TupleElementInfo`,
  `ConditionalRoot`, flow/iteration helper records … are arena structs handled as `P<…>` with `Cell` fields where Go mutates.
  Small Go value structs passed by value stay `Copy` structs.
- `TypeMapper`: `enum`-backed struct; Go `m.Map(t)` -> `m.map(c, t)` (`fn map(&self, c: &mut Checker, t: P<Type>) -> P<Type>`):
  mappers that Go implements with captured closures or a stored checker get the checker passed in instead.
  Function mappers are `fn(&mut Checker, P<Type>) -> P<Type>` pointers or enum variants carrying their captured data.
- Hash keys (`CacheHashKey`, xxh3-based in Go) keep Go's construction order and width (`u128`); use the `xxhash-rust` crate
  (`xxh3` feature) and mirror Go's key-building helpers exactly.

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

All link stores use one generic `LinkStore<K, V>` keyed by `P<K>` (`FxHashMap<P<K>, P<V>>`), whatever store flavor Go uses.

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
