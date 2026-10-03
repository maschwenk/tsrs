# emit/async (wave E9): async.go, forawait.go

## Ported

- `estransforms/async.go` -> `crates/tsrs_transformers/src/estransforms/async_.rs` (complete, every function, Go order,
  `// async.go:LINE` markers). `assignmentTargetContainsSuperProperty`, `isUpdateExpression`, `isSimpleParameterList`
  are `pub(crate)` there because `utilities.rs` / `forawait.rs` use them (same package in Go).
- `estransforms/forawait.go` -> `forawait.rs` (complete).
- `estransforms/utilities.go`: only `superAccessState` and its methods (utilities.go:52-277) -> new
  `estransforms/utilities.rs`. `convertClassDeclarationToClassExpression`, `createNotNullCondition`,
  `createAccessorPropertyBackingField` are not ported (not used here; the classfields wave may add them to the same file).
- Cherry-picked origin/emit/core 0c2fbba ("the rest of transformers/utilities.go", same content as on emit/core and
  emit/jsx-decorators) for `ConvertBindingPatternToAssignmentPattern`, so later merges are trivial.

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

None outside `crates/tsrs_transformers/src/estransforms/` (mod.rs: `mod utilities`), besides the cherry-picked
`transformers/utilities.rs` + `lib.rs` from emit/core. No printer / emit-context / ast / checker changes were needed
(the awaiter / async generator / await / async values / async delegator helpers and `ASYNC_SUPER_HELPER` /
`ADVANCED_ASYNC_SUPER_HELPER` already existed).

## Tests (tools/oracle/emit/cases.py, tsgo vs `TSRS_EMIT=1 tsrs`)

On emit/async alone every async/es2017/es2018/emitter case still crashes in the typeeraser/importelision stubs
(before = after: async 248 crash + 4 empty, es2017 19 crash, es2018 7 crash, emitter 17 crash).

With int/local + emit/async (scratch merge, not pushed), before = int/local, after = merge:

| set | before | after |
| --- | --- | --- |
| 282 files: conformance async/es2017/es2018/emitter + every test with `for await` / async generators | crash 279, empty 30, pass 104 (413) | crash 143, empty 30, pass 240 |
| 261 more files containing `async`/`await` | crash 229, empty 46, pass 107 (382) | crash 145, empty 47, pass 190 |
| the 137 of those that crash in the CJS module stub, rewritten to `@module: es2015` | crash 191, empty 7, pass 57 (255) | crash 6, empty 7, pass 242 |
| all of conformance | crash 3718, empty 779, pass 3052 (7549) | crash 3020, empty 806, pass 3723 |
| all of compiler | crash 3580, empty 884, fail 1, pass 3075 (7540) | crash 2685, empty 903, fail 1, pass 3951 |

0 fails and 0 crashes in async/forawait; every remaining crash is another wave's stub (commonjsmodule, classfields,
using, objectrestspread, …). The one compiler fail (`regexInvalidUtf8WithUnicodeFlag`) predates this branch. No
case regressed (only crash -> pass/empty transitions). An ad-hoc case (super property/element access and assignment
in async methods, async generators with `yield*`, labeled `for await`, `arguments` capture in arrows, colliding
`var`s, top-level await) passes for es2015-es2018.

## Not done

Nothing in async.go / forawait.go. Classes with fields still crash in the classfields stub, so async code inside
such classes is untested here.
