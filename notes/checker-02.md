# checker-02 notes (checker_02.rs = checker.go 2173–4320, 92 functions)

## Signature changes

None in checker_02.rs.

## Shared-file edits

None.

## Needs from others

- `check_type_assignable_to_and_optionally_elaborate` (relater.go:424): `error_node` and `expr` should be
  `Option<P<Node>>` (Go passes nil `expr`/`errorNode` from `checkReturnExpression` for a bare `return;` under
  strictNullChecks). `check_return_expression` calls `check_type_related_to_and_optionally_elaborate(..,
  self.assignable_relation, ..)` directly (the Go function's one-line body) with a `// SIG:` comment; switch back
  once the signature is fixed.
- `check_grammar_type_arguments` (grammarchecks.go:855): `type_arguments` should be `Option<P<NodeList>>` (Go
  passes `node.TypeArgumentList()`, nil when absent). `check_type_reference_node` skips the call when the list is
  nil, which is equivalent because both inner checks are no-ops for a nil list (`// SIG:` comment at the call).

## Doubts

- `check_function_or_constructor_symbol_worker` / `checkFlagAgreementBetweenOverloads`: Go groups overloads in a
  `map[*ast.SourceFile][]*ast.Node` and iterates it (random order); the port uses an insertion-ordered map
  (first-seen file first), so diagnostic order across files is deterministic but may differ from a given Go run.
- `check_type_parameter_deferred`: Go saves `saveVarianceTypeParameter := typeParameter` (not the previous
  `c.varianceTypeParameter`) and restores that; ported verbatim (so `variance_type_parameter` is left set).
- `check_catch_clause`: Go iterates `node.Locals()` (a Go map, random order); the port uses the symbol table's
  insertion order.
- `check_for_of_statement`: Go's `core.OrElse(iteratedType, c.errorType)` and `if iteratedType != nil` are dropped
  because `check_right_hand_side_of_for_of` returns `P<Type>` and the Go function never returns nil
  (`checkIteratedTypeOrElementType` falls back to `anyType`).
- `check_type_predicate`: Go's `else if parameterName != nil` is unconditional in the port because
  `TypePredicateNode.parameter_name` is a non-nil `P<Node>` in tsrs_ast.
- `check_deferred_nodes`: iterates `deferred_nodes` by index re-borrowing the `RefCell` each step so nodes added
  during iteration are visited, matching Go's `OrderedMap.Keys()` semantics.
- `TypeToString` calls map to `type_to_string_exported` (Go's exported `TypeToString`).
