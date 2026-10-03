# emit/async (wave E9): async.go, forawait.go

## Ported

- `estransforms/async.go` -> `crates/tsrs_transformers/src/estransforms/async_.rs` (complete, every function, Go order,
  `// async.go:LINE` markers). `assignmentTargetContainsSuperProperty`, `isUpdateExpression`, `isSimpleParameterList`
  are `pub(crate)` there because `utilities.rs` / `forawait.rs` use them (same package in Go).
- `estransforms/forawait.go` -> `forawait.rs` (complete).
- `estransforms/utilities.go`: `superAccessState` and its methods (utilities.go:52-277) in `estransforms/utilities.rs`,
  next to the classfields layer's `convertClassDeclarationToClassExpression`, `createNotNullCondition` and
  `createAccessorPropertyBackingField`. The classfields layer had carried its own copy of `superAccessState` and of
  async.go's `assignmentTargetContainsSuperProperty` / `isUpdateExpression`; this layer replaces that copy with the
  async port (whose `async_.rs` owns those two functions, as async.go does in Go).

## Decisions

- Go embeds `superAccessState` in both transformers; Rust keeps it as a field `super_access_state` (Cell/RefCell
  fields, `captured_super_properties: RefCell<Option<OrderedSet<String>>>` = Go's nil-able pointer). Its visitor
  closure captures `P::from_static(&tx.super_access_state)`.
- Go `defer` restores (`visit`'s `EFNoLexicalThis`, `visitArrowFunction`'s `EFNoLexicalArguments`) are a split into
  `visit`/`visit_worker` and `visit_arrow_function`/`visit_arrow_function_worker` with the restore after the call.
- `enclosingFunctionParameterNames` (`*collections.Set[string]`) is `RefCell<Option<FxHashSet<String>>>`; the set is
  built locally and then installed (equivalent: `recordDeclarationName` does not read the field).
- Argument evaluation order of every Go `Update*` call is kept by computing the arguments into locals in Go order
  (e.g. modifiers are visited after the async body in `visitMethodDeclaration`).
- `convertForOfStatementHead` keeps Go's zero `core.TextRange{}` (0, 0) loc for a non-block body.

## Shared-file edits

`estransforms/utilities.rs` (see above). `transformers/utilities.rs` and `lib.rs` need nothing from this layer: main
already has the rest of transformers/utilities.go. No printer / emit-context / ast / checker changes.

## Review layer

Clean review branch `mfs-cx/emit-async` on top of `mfs-cx/emit-classfields`. `async_.rs` and `forawait.rs` are
byte-identical to the helper branch `emit/async` 4e99392. Results: see the layer's commit message. On this base the
TypeScript transformers are still gate stubs (typeeraser), so async tests stop there; the combined local integration
build (every helper layer) exercised async/forawait with 0 js baseline fails.

## Not done

Nothing in async.go / forawait.go.
