# nb-1 notes: nodebuilderimpl_1.rs (nodebuilderimpl.go 1–1850)

All 71 stubs ported (the two `try_get_module_specifier_from_declaration[_worker]` bodies were already real and are
unchanged). Added one private helper `NodeBuilderImpl::tracker()` (= Go `b.ctx.tracker`) and split Go's deferred pop in
`checkTypeExpandability` into a private `check_type_expandability_worker` (push; worker; pop on every path).

## Signature changes

- `get_resolved_type_without_abstract_construct_signatures(t: &'static StructuredType)` -> `(t: P<Type>)`. A
  `StructuredType` payload has no back pointer to its `Type` header, and Go uses `t.AsType()` / `t.symbol`. Callers
  (nb-2, nodebuilderimpl.go:2827 `b.getResolvedTypeWithoutAbstractConstructSignatures(resolved)`) must pass the type
  they passed to `resolve_structured_type_members`.
- `is_homomorphic_mapped_type_with_non_homomorphic_instantiation(mapped: &'static MappedType)` -> `(mapped: P<Type>)`,
  same reason (`mapped.AsType()`). Only called from this file.
- `array_is_homogeneous<T>` -> `array_is_homogeneous<T: Clone>` (the comparer takes elements by value).

## Shared-file edits

- none.

## Needs from others

Call sites adapted with `// SIG:` comments (they `.unwrap()` the enclosing declaration today, which panics where Go
passes nil; fix the callee signature and drop the unwrap):

- nb-3 (symbolaccessibility.go in printer.rs): `is_symbol_accessible`, `get_accessible_symbol_chain`,
  `needs_qualification`, `get_containers_of_symbol` take `enclosing_declaration: P<Node>`, but Go passes a nil-able
  `b.ctx.enclosingDeclaration` and the callees nil-check it (`isSymbolAccessibleWorker`, `someSymbolTableInScope`,
  `getWithAlternativeContainers`). `getSymbolChain` reaches them with a nil enclosing declaration whenever
  `UseFullyQualifiedType` is set (e.g. `mapToTypeNodes` regenerating colliding names for a plain `typeToString`).
  They should be `Option<P<Node>>`.
- nb-2: `track_computed_name(.., enclosing_declaration: P<Node>)` should be `Option<P<Node>>` (Go passes the nil-able
  `b.ctx.enclosingDeclaration` to `resolveName`/`TrackSymbol`).
- nb-2: `create_access_expression` returns `Option<P<Node>>` although Go never returns nil (it panics); my callers
  `.unwrap()` it. If nb-2 changes it to `P<Node>`, drop the three `.unwrap()`s.

## Doubts

- `map_to_type_nodes`: Go may store a nil `typeToTypeNode` result into the node slice (truncated bare-list
  first/last element and the `UseFullyQualifiedType` regeneration); a Rust `NodeList` cannot hold nil, so those
  three spots `.unwrap()`. Also the regeneration loop iterates the `MultiMap` values in FxHashMap order (Go: random
  map order), which can only matter when regeneration changes `approximateLength` enough to affect truncation.
- `get_symbol_chain`: Go's `append(parentChain, nextSyms...)` may write into the backing array of a slice cached by
  `getAccessibleSymbolChain` (Go aliasing); the port always builds a fresh Vec.
- `get_name_of_symbol_from_name_type`: `jsnum.Number.String()` is `Number::string()`; `valueToString` is the
  Checker method (printer.go), not the free function in utilities.go.
- `getSpecifierForModuleSymbol`'s cache key uses `&'static` text of `context_file.path()` (no copy).
