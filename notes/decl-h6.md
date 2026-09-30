# decl-h6: EmitContext transform parts (tsrs_printer)

Ported from `ts-ref/tsc/internal/printer/emitcontext.go` into `crates/tsrs_printer/src/emitcontext.rs`:
`NewNodeVisitor` (the `todo!()` stub), environment tracking (`varScopeStack`/`letScopeStack` fields,
`environmentFlags`, `varScope`, Start/End(AndMerge)VariableEnvironment[List], AddVariableDeclaration,
AddHoistedFunctionDeclaration, Start/End(AndMerge)LexicalEnvironment[List], AddLexicalDeclaration,
MergeEnvironment[List], mergeEnvironment, isCustomPrologue, isHoistedFunction, isHoistedVariable,
isHoistedVariableStatement), and the visitor hooks (VisitVariableEnvironment, VisitParameters,
addDefaultValueAssignment(s)IfNeeded, addDefaultValueAssignmentForBindingPattern/Initializer,
AddInitializationStatement, ConvertToFunctionBlock, VisitFunctionBody, VisitIterationBody, VisitEmbeddedStatement).
`Reset()` clears both scope stacks (Go resets every field but `Factory`).

`factory.go` helpers added to `src/factory.rs`: `NewAssignmentExpression`, `NewStrictEqualityExpression`,
`NewVoidZeroExpression`, `NewTypeCheck`. `utilities.go` `findSpanEnd`/`findSpanEndWithEmitContext` added to
`src/utilities.rs`. Regression test `test_node_visitor_environment_hooks` in `src/printer_test.rs` (parameter
initializer/binding pattern moved into the body after a temp is hoisted in the parameter list; also proves the hooks
are re-entrant w.r.t. the scope RefCells).

## Signature changes

- Go exported/unexported pairs that collide after snake-casing: unexported `endAndMergeVariableEnvironment`,
  `endAndMergeLexicalEnvironment`, `mergeEnvironment` -> `*_worker` (`pub(crate)`, return `(Vec<P<Node>>, bool)`).
- Statement slices: params `&[P<Node>]`, results `Vec<P<Node>>`. `*List` variants: `Option<P<NodeList>>` for
  `EndAndMerge*EnvironmentList` (Go accepts nil; like Go, a nil list with merged declarations panics on `.Loc`),
  `P<NodeList>` for `MergeEnvironmentList` (Go dereferences it unconditionally).
- Hooks take `Option<P<…>>` + `&mut NodeVisitor` to match `tsrs_ast::NodeVisitorHooks`.
- `varScope` is `Rc<RefCell<varScope>>` on a `tsrs_core::Stack` (Go `*varScope` mutated through `Peek()`); borrows
  are short and never span a visitor call.

## Shared-file edits

None outside `crates/tsrs_printer`.

## Needs from others

None.

## Doubts

- `AddInitializationStatement`: Go's `scope == nil` check is unreachable (Go `Stack.Peek` panics on an empty stack
  first); the port panics with "stack is empty" like Go's Peek.
- `new_node_visitor` recovers `P<EmitContext>` from `&self` with `unsafe` (`P::from_static`); sound because contexts are
  only created by `new_emit_context` (arena `P::new`). An `EmitContext` built any other way would be UB, but the
  struct has private fields so it cannot be constructed outside the crate.
