# checker-04 notes (checker_04.rs = checker.go 6527–8748)

## Signature changes

- `IterationTypesResolver::get_resolved_iteration_types(&self, yield, return, next)` ->
  `(&self, c: &mut Checker, yield, return, next)`: Go calls `r.resolveIterationType`, which needs the checker
  (CHECKER.md helper-struct rule: `c` is the first parameter after the receiver). Only called from checker_04.rs.

## Shared-file edits

None.

## Needs from others

- `check_grammar_type_arguments` (grammarchecks.rs) takes `P<NodeList>`, but Go `checkCallExpression` passes
  `node.TypeArgumentList()`, which is nil for most calls. It should be `Option<P<NodeList>>`. At the call site I skip the call
  when the list is nil (the Go callee returns false with no side effects for nil), marked `// SIG:`.
- `try_get_module_specifier_from_declaration` (nodebuilderimpl.go:1193, advisory sig) is not ported yet. checker_04.rs has a
  private copy of it and its worker at the end of the file, marked `// SIG:`. Delete them once the node-builder port lands.
  A glob re-export can't clash with a private item.

## Doubts

- `check_unused_locals_and_parameters`: Go iterates `node.Locals()` (a Go map), a `collections.Set` and a map, all in random
  order. The port uses the SymbolTable insertion order plus `OrderedSet`/`OrderedMap`. Diagnostic emission order can differ
  from any single Go run; the final output depends on diagnostics being sorted.
- `instantiate_type_with_single_generic_call_signature`: Go's `mergeInferences(context.inferences, inferences)` mutates the
  slice in place. Here `context.inferences` is `Cell<&'static [..]>`, so I merge into a copy and store the copy. This differs
  only if some other holder aliases the old slice; I found none (mappers read `context.inferences` when they run).
- `get_iteration_types_of_method`: Go may append a nil `returnType` (the `IteratorYieldResult<T>` fast path). With one element,
  `getUnionType` returns it, so the result has a nil return type; the port handles that. With two elements (the `return`
  method) Go would crash; the port panics.
- `resolve_call_expression`: Go `node.TypeArguments() != nil` is translated as `node.type_argument_list().is_some()`. This only
  matters for an empty `f<>()` list, where Go's nil-vs-empty `Nodes` depends on the parser.
- `check_expression_with_contextual_type`: Go sets `intraExpressionInferenceSites = nil`; the port clears the `RefCell<Vec>`.
