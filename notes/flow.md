# flow (crates/tsrs_checker/src/flow.rs <- checker/flow.go)

All 131 functions ported; no `todo!()` left. Tracing block in getTypeAtFlowNode dropped.

## Signature changes

- `get_branch_label_antecedents(flow, reduce_labels: &[P<Node>])` -> `reduce_labels: &[P<ast::FlowReduceLabelData>]`:
  Go takes `[]*ast.FlowReduceLabelData`, and `FlowState.reduce_labels` (flow_types.rs) already stores
  `P<FlowReduceLabelData>`.
- `get_switch_clause_type_of_witnesses(node) -> Vec<String>` -> `Option<&'static [&'static str]>`: Go returns nil
  (a case expression is not a string literal) vs a stored slice, and callers test `witnesses == nil`. Only flow.go uses it.

## Shared-file edits

- types.rs `SwitchStatementLinks.witnesses`: `Cell<&'static [&'static str]>` -> `Cell<Option<&'static [&'static str]>>`
  (Go nil vs empty is observable: a switch with no clauses has non-nil empty witnesses). Only flow.rs touches it.

## Needs from others

- `include_undefined_in_index_signature(t: P<Type>) -> P<Type>` keeps the generated non-Option signature. Go's body
  returns nil for nil input; flow.go never passes nil, but checker.go:6311/6378 (checker_03) pass
  `iterationTypes.yieldType` / `arrayElementType`. The 6311 call is guarded non-nil; if `arrayElementType` at 6378 can
  be nil, that caller must return None itself instead of calling (Go would return nil).
- `every_type` / `some_type` (checker free functions) take `impl FnMut(P<Type>) -> bool` without the checker, so
  closures such as `every_type(t, |t| self.is_nullable_type(t))` capture `self` (the only option with that signature).

## Doubts

- `get_explicit_type_of_symbol`: Go's `defer c.resolvingExplicitTypeOfSymbol.Delete(symbol)` is modelled by a private
  helper `get_explicit_type_of_symbol_worker` (body after the guard) followed by the delete. Equivalent unless a callee
  panics.
- `FlowLoopInfo.types` is pushed as a clone of the local `antecedent_types` (Go shares the slice header; the pushed
  length is fixed at push time and the stack entry is popped before the local grows, so a snapshot is equivalent).
- `get_switch_clause_types` returns a fresh `Vec` copy of the cached slice each call (generated signature); semantics
  unchanged.
- `get_type_at_flow_node` unwraps `get_branch_label_antecedents(...)` / `flow.antecedents()` for labels (Go derefs
  `antecedents.Next`); a reduce label with nil antecedents would panic in both.
- Small private accessors `FlowState::{ref_node, declared, initial}` and `flow_type_of(t)` (= Go `FlowType{t: t}`)
  added in flow.rs for readability.
