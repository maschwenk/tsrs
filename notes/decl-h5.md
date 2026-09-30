# decl-h5: rest of checker/emitresolver.go

Ported every remaining `todo!()` in `crates/tsrs_checker/src/emitresolver.rs` (50 functions; the 14 ported earlier are
unchanged). Go's `checkerMu` locking is dropped (the caller serializes, tsrs_declarations `Resolver`); Go's
`isValueAliasDeclaration` / `aliasMarkingVisitor` method values are direct calls of the `..._worker` methods;
`MarkLinkedReferencesRecursively`'s recursive `visit` closure is the private fn `mark_linked_references_recursively_visit`.

## Signature changes

- `NodeBuilder::serialize_type_for_declaration` (nodebuilder.rs): `symbol: P<Symbol>` -> `symbol: Option<P<Symbol>>`.
  Go's `CreateTypeOfDeclaration` passes `getSymbolOfDeclaration(declaration)`, which may be nil, and the impl's
  `serializeTypeForDeclaration` handles a nil symbol. It had no other callers.
- None in emitresolver.rs: all signatures used by tsrs_declarations `resolver.rs` kept.

## Shared-file edits

- `crates/tsrs_checker/src/nodebuilder.rs`: the signature change above (2 lines).
- `crates/tsrs_checker/src/nodebuilder_types.rs`: one comment (`emitresolver_subset.rs` -> `emitresolver.rs`).
- `docs/CHECKER.md`: file table row split (`emitresolver.go` is now fully ported) and the "Emit resolver subset"
  paragraph rewritten as "Emit resolver" describing the whole file.

## Needs from others

- None.

## Doubts

- `create_type_parameters_of_signature_declaration` returns `None` when the node builder's `Vec` is empty. Go's slice is
  nil exactly when empty (`typeParametersToTypeParameterDeclarations` only appends to a nil slice or returns nil;
  `exitContextSlice` returns nil on error), so this should be faithful, but it relies on that reasoning.
- `CreateLiteralConstValue`, `CreateLateBoundIndexSignatures`, `TryJSTypeNodeToTypeNode` `unwrap()` the result of
  `emit_context.parse_node(..)` (Go dereferences / passes it on without a nil check). `CreateLiteralConstValue`'s
  `t == nil` and `IsThisPropertyAssignmentDeclarationRedundant`'s `parentType == nil` checks are dropped because the
  Rust `get_type_of_symbol` / `get_declared_type_of_symbol` never return nil.
- `is_common_js_module_exports` returns false for a binary expression without a parent / grandparent (Go would
  panic on the nil `Parent.Kind`); this cannot happen for parse-tree nodes.
- `IsImportRequiredByAugmentation` and `CreateLateBoundIndexSignatures` iterate a `SymbolTable` in insertion order
  where Go iterates a map (random order). The first only computes a boolean; the second feeds sibling symbols to
  `get_index_infos_of_index_symbol` exactly like the existing checker port (`get_index_infos_of_symbol`).
- `CreateLateBoundIndexSignatures`: Go's `core.IfElse` evaluates both arms, so the `static` modifier and the modifier
  list are created even when unused; the port creates them too (only affects node allocation, not output).
