# checker-15 notes (`checker_15.rs` = checker.go 30510–32667)

All 112 functions ported; no `todo!()` left.

## Signature changes

- `instantiate_contextual_type(contextual_type: P<Type>, ..) -> P<Type>` ->
  `(contextual_type: Option<P<Type>>, ..) -> Option<P<Type>>`. Go callers pass nil-able contextual types
  (`getContextualType` results) and the function returns its argument unchanged. Callers to adapt:
  checker.go:7669 (checker_04), 14121 (checker_07), 20555 (checker_10), pseudotypenodebuilder (not ported).
- `get_this_type_of_object_literal_from_contextual_type(.., contextual_type: P<Type>)` ->
  `(.., contextual_type: Option<P<Type>>)`. Go loops `for t != nil` and both callers pass the nil-able result of
  `getApparentTypeOfContextualType`. Caller to adapt: checker.go:12245 (checker_06).

## Shared-file edits

None.

## Needs from others

- `SyntheticExpression.type_` (`&'static dyn Any`) convention: `create_synthetic_expression` stores
  `alloc(t)`, i.e. a `&'static P<Type>`. Readers (Go `node.AsSyntheticExpression().Type.(*Type)`, checker.go:11235
  in checker_06 `check_synthetic_expression`, checker.go:29998/30020 in checker_14) should use
  `*node.as_synthetic_expression().type_.downcast_ref::<P<Type>>().unwrap()`.
- `get_promised_type_of_promise_ex(.., this_type_for_error_out: Option<&mut P<Type>>)` (checker_14) should be
  `Option<&mut Option<P<Type>>>` (Go `**Type` whose target starts nil). `get_awaited_type_no_alias_ex` adapts with a
  `// SIG:` comment: it passes `&mut error_type` as a "not written" sentinel (the callee only writes a non-nil
  this-type).
- `push_contextual_type(node, t: P<Type>, ..)`: Go's `t` is nil-able (`pushCachedContextualType` passes
  `getContextualType`'s result). Kept the generated signature (other files call it with non-nil types) and put the
  body in a private `push_contextual_type_worker(node, Option<P<Type>>, ..)` that both call.
- `IsArgumentsSymbol` lives in services.go (not ported); `contains_arguments_reference` inlines it as
  `symbol == c.arguments_symbol`.
- `get_emit_resolver` returns a fresh placeholder `P<EmitResolver>` (emit resolver not ported; Go memoizes one).

## Doubts

- `get_awaited_type_no_alias_ex` union branch: Go stores a possibly-nil `mapped` in `cachedTypes`; the Rust map
  cannot hold nil, so a nil result removes the key (reads back as a miss, same as Go).
- The error-type sentinel above: if a `then` signature's this-type were literally the error type, Go would add the
  "this context" chained diagnostic head and the port would not.
- `has_context_sensitive_return_expression`: Go `node.TypeParameters() != nil` is ported as
  `node.type_parameter_list().is_some()` (differs only if a list exists with a nil `Nodes` slice).
- `get_symbol_at_location` string-literal case: Go dereferences `grandParent` unconditionally; the port treats a nil
  grandparent as "predicate false" (Go would panic; unreachable for real trees).
