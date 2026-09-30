# nb-2 notes

Scope: `nodebuilderimpl_2.rs` (nodebuilderimpl.go 1851–end, 41 functions) and `nodebuilderscopes.rs`
(nodebuilderscopes.go, 4 functions). All ported; no `todo!()` left.

## Signature changes

- `create_type_nodes_from_resolved_type(resolved_type: &'static StructuredType)` -> `(resolved_type: P<Type>)`.
  Go reads `resolvedType.objectFlags`, which lives on the Rust `Type` header; the only caller passes the type it gave
  to `resolve_structured_type_members` (which returns `t.as_structured_type()`, i.e. the same object).
- `get_parent_symbol_of_type_parameter(type_parameter: &'static TypeParameter)` -> `(type_parameter: P<Type>)`
  (Go reads `typeParameter.symbol`, a header field). Only caller: `type_reference_to_type_node`.
- `track_computed_name(.., enclosing_declaration: P<Node>)` -> `Option<P<Node>>` (Go passes the possibly-nil saved
  enclosing declaration; `resolve_name`/`track_symbol` take `Option`). nb-1's caller (nodebuilderimpl.go:1787) must pass
  `b.ctx().enclosing_declaration.get()` (agreed with nb-1 via the coordinator).
- `get_property_name_node_for_symbol` / `get_property_name_node_for_symbol_from_name_type`:
  `enclosing_declaration: P<Node>` -> `Option<P<Node>>` (Go passes nil and tests `enumEnclosingDeclaration == nil`).
  Only called from this file.
- Kept as generated on purpose: `create_access_expression -> Option<P<Node>>` (Go never returns nil; nb-1 unwraps).

## Shared-file edits

- `nodebuilder_types.rs`: `SerializedTypeEntry.node: P<Node>` -> `Option<P<Node>>`. Go caches the (possibly nil) result
  of the transform in `visitAndTransformType` and returns `DeepCloneNode(nil)` = nil on a hit (e.g. an empty union under
  `AllowEmptyUnionOrIntersection`). Only `visit_and_transform_type` uses the struct.

## Needs from others

- printer.rs (nb-3): `is_value_symbol_accessible`, `is_type_symbol_accessible`, `is_symbol_accessible` (and probably
  `is_symbol_accessible_by_flags`) take `enclosing_declaration: P<Node>`, but the node builder passes
  `b.ctx.enclosingDeclaration`, which may be nil (`isSymbolAccessibleWorker` then answers Accessible). They should take
  `Option<P<Node>>`. Until then nodebuilderimpl_2.rs uses private adapters `is_*_accessible_opt` (marked `// SIG:`)
  that return Accessible/true for `None`; after the signature change replace them with direct calls.
- emitresolver_subset.rs (nb-4): `EmitResolver::requires_adding_implicit_undefined(.., enclosing_declaration: P<Node>)`
  should take `Option<P<Node>>` (Go passes `b.ctx.enclosingDeclaration`; `isRequiredInitializedParameter` tests it with
  `!= nil && IsFunctionLikeDeclaration`). `serialize_type_for_declaration` currently passes the declaration itself (a
  parameter/property, never function-like) for `None`, which is exact; marked `// SIG:`.
- nb-1: `get_resolved_type_without_abstract_construct_signatures` now takes `t: P<Type>` on nb-1's branch;
  `create_type_node_from_object_type` still calls the old `&'static StructuredType` stub with `resolved` (marked
  `// SIG:`); after the merge pass `t`.
- pseudochecker (nb-5): `PseudoType::as_pseudo_type_inferred()` does not exist on this branch; the port matches
  `pt.data` against `pseudochecker::PseudoTypeData::Inferred` directly (same semantics).

## Doubts

- Go slices of nodes may hold nil elements; Rust `Vec<P<Node>>` cannot. `signature_to_signature_declaration_helper`
  skips a nil `typeToTypeNode` result under `WriteTypeArgumentsOfSignature` (Go would append nil). Elsewhere a nil node
  passed to a factory parameter that is non-`Option` in Rust is `unwrap()`ed (Go would build a node with a nil child):
  conditional/tuple/indexed-access/template/keyof/string-mapping children, `appendReferenceToType` arguments. These are
  nil only in declaration-emit error paths (`typeToString` sets `IgnoreErrors`, which includes the `AllowEmpty*` flags).
- `KindArrowFunction` in `signature_to_signature_declaration_helper`: Go passes a nil `EqualsGreaterThanToken` (the Go
  printer then prints no `=>`); the Rust factory requires a token, so a real `=>` token is passed. Not reached by
  type-to-string.
- Enum member access through an import type: Go flips `IsTypeOf = true` on the freshly built `ImportTypeNode`; the Rust
  field is not a `Cell`, so a new `ImportTypeNode` with the same children and `is_type_of = true` is built instead
  (identical printed text; node identity differs, which nothing observes).
- `create_type_node_from_object_type`: a nil member list (`createTypeNodesFromResolvedType` returned nil) becomes an
  empty `NodeList` (Go's printer emits nil and empty type-literal member lists identically).
- `visit_and_transform_type`: Go also stores cache entries under a nil enclosing declaration; they are never read back,
  so the port skips them.
- `enter_new_scope`: Go distinguishes nil and empty `originalParameters`/`expandedParams`; the port treats empty as nil
  (callers passing a non-nil `originalParameters` pass the signature's parameters, non-empty whenever the expanded
  parameters are). Go's `bindPatternWorker` returns after the first element of a binding pattern; kept as is.
- `clone_node_builder_context`'s restore closure restores once (a second call is a no-op); Go's restore funcs can be
  re-run. All callers run it once.
- `type_reference_to_type_node`: Go's `TypeParameters() != nil` is reproduced as `all_type_parameters` non-empty, so the
  `getGlobalIterable*Type` calls (which resolve and cache globals) happen exactly when Go makes them.
- `get_expanded_parameters` keeps Go's quirk of keying `counters` by the *new* (suffixed) name.
