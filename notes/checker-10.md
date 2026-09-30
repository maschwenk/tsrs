# checker-10 notes (`checker_10.rs` = checker.go 19608–21763)

## Signature changes

None. Two generated signatures are narrower than Go's semantics; handled at the call site instead (marked `// SIG:`):

- `get_annotated_accessor_type(accessor: P<Node>)`: Go accepts nil (returns nil). The only nil-passing caller in this
  file (`get_return_type_from_annotation`) matches on the `Option` and returns `None`. Should be `Option<P<Node>>`.
- `instantiate_symbol(symbol: P<Symbol>, ..) -> P<Symbol>`: Go accepts/returns nil. `instantiate_signature_ex` maps the
  optional this-parameter with `.map(..)`. Should be `Option<P<Symbol>> -> Option<P<Symbol>>`.

## Shared-file edits

None.

## Needs from others

- `instantiate_signature(sig, m: Option<P<TypeMapper>>)` forwards to `instantiate_signature_ex(sig, m: P<TypeMapper>, ..)`
  with `m.unwrap()`. Go would pass nil through (then `instantiateSymbols(params, nil)` etc.). If a caller really passes
  nil, `instantiate_signature_ex`/`instantiate_symbols` need `Option` mappers.
- `resolve_declared_members` returns `Option<&InterfaceType>` (generated) but is never nil; always `Some`.

## Doubts

- `get_union_signatures`: Go's `core.Same(signatures, masterList)` compares slice identity; the Rust lists are separate
  `Vec`s, so the port compares by index (`i != index_with_length_over_one`). Differs only if two constituents returned
  the same backing signature slice in Go.
- `instantiate_symbol`: when `symbol` is itself instantiated, `m` is `unwrap()`ed before `combine_type_mappers`
  (Go would build a composite mapper with a nil `m2` if `m` were nil).
- `check_and_aggregate_yield_operand_types`: `get_yielded_type_of_yield_expression` can return nil in Go (awaited type
  failure) and Go appends the nil into `yieldTypes`; the port `unwrap()`s (Go would crash later in `getUnionTypeEx`).
- `get_signature_from_declaration`: a nil `resolveName` result for a parameter-property symbol is `unwrap()`ed when
  pushed into `parameters` (Go would append nil).
- `get_type_predicate_from_body`: `fn.Body()` is unwrapped before `ForEachReturnStatement` (Go panics on nil there too).
- `combine_union_or_intersection_member_signatures`: Go's `append(left.composite.signatures, right)` may alias the
  left composite's backing array; the port always allocates a new slice.
- Several `c.foo(..)` → `for_each_type`/`some_type`/`same_map`/`for_each_return_statement` closures capture `self`
  (these helpers are free functions without a checker parameter); evaluation order is unchanged.
