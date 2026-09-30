# decl-h1: transform_1.rs (transform.go 1–1348)

All functions in `crates/tsrs_declarations/src/transform_1.rs` ported (no `todo!()` left), including
`transformBinaryExpressionToExportDeclaration`, whose body runs to transform.go:1348.

## Signature changes

- `new_declaration_transformer`: `context: P<EmitContext>` -> `Option<P<EmitContext>>` (Go passes nil from
  `getDeclarationDiagnostics`; `Transformer::new_transformer` creates a fresh context for `None`).
- `setup_diagnostic_context`: returns `(bool, Box<dyn FnMut() + '_>)` instead of `Box<dyn FnMut()>` ('static). The
  restore closure must write `self.suppress_new_diagnostic_contexts` (a transformer field, not shared state), so it
  borrows `&self`. This avoids an `unsafe` `as_p()`. Callers in transform_3.rs (`let (_, mut cleanup) = ...; cleanup();`)
  compile the same way.
- `transform_function_like_to_declaration`: `full_signature_type: P<Node>` -> `Option<P<Node>>` (Go passes
  `assignment.Type()`, usually nil).

## Shared-file edits

- `transform_3.rs`: `ensure_type_params(&self, node, params: P<NodeList>)` -> `params: Option<P<NodeList>>` (signature
  only; body still the other helper's). Go passes `input.TypeParameters`, nil when absent, and `VisitNodes(nil)`
  returning nil is what triggers the node-builder fallback, so there is no call-site workaround.
- `tsrs_ast/src/utilities_3.rs` (end): added `is_external_module_indicator` (Go `ast.IsExternalModuleIndicator`,
  utilities.go:1703); it was missing.

## Needs from others

- `rewrite_module_specifier(parent, input: P<Node>) -> P<Node>` (transform_2.rs) should be
  `(parent, input: Option<P<Node>>) -> Option<P<Node>>`: Go returns nil for a nil input before any side effect. The
  ExportDeclaration call site maps over the Option (`// SIG:`); the equivalent form is correct either way.
- `unwrap_parenthesized_expression` (util.rs) returns `Option`, but Go never returns nil for a non-nil input. I
  `unwrap()` it.
- `check_entity_name_visibility(entity_name, enclosing_declaration: P<Node>)`: I pass
  `self.enclosing_declaration.get().unwrap()`. `visitSourceFile` always sets it, so it is never nil there.

## Doubts

- `SymbolTrackerSharedState.get_symbol_accessibility_diagnostic` needs an initial value. Go's zero value is a nil
  func, so I use the `throw_diagnostic` wrapper (both panic if called before `visitSourceFile` sets it).
- `get_this_property_assignment_key` unwraps `name` for `IsPrivateIdentifier`, matching Go's nil dereference; the
  later `name != nil` check is then dead, as in Go.
- Go factory calls that can receive nil from a visitor where the Rust factory needs `P<Node>` are `unwrap()`ed:
  mapped type `TypeParameter`, the four conditional-type children, the variable declaration name through
  `bindingNameVisitor`, and `update_heritage_clause`'s `VisitNodes` result. In Go these would store nil, which does
  not happen in practice.
- `transform_source_file` reaches `node.Symbol.Exports[...]` with `and_then`, so a nil symbol means "no export=".
  Go would panic, but an external or CommonJS module always has a symbol.
