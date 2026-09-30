# decl-h4: diagnostics_.rs + tracker.rs

All functions in `crates/tsrs_declarations/src/diagnostics_.rs` (diagnostics.go) and `src/tracker.rs` (tracker.go)
ported; no `todo!()` left.

## Signature changes

- `wrap_simple_diagnostic_selector`, `wrap_named_diagnostic_selector`, `wrap_fallback_error_diagnostic_selector`:
  `selector: impl FnMut(..)` -> `impl Fn(..) + 'static`. The returned `GetSymbolAccessibilityDiagnostic`
  (`Rc<dyn Fn>`) captures the selector. Selectors that return a non-optional `&'static Message` are adapted with a
  closure `|n, r| Some(f(n, r))` when passed to `wrap_simple_diagnostic_selector` (its selector returns `Option`,
  because `getVariableDeclarationTypeVisibilityDiagnosticMessage` can return nil).
- `create_diagnostic_for_node(node: P<Node>, ..)` -> `node: impl Into<Option<P<Node>>>`. Go passes a possibly-nil
  node (`handleSymbolAccessibilityError`'s `diagNode` can be nil when a named selector's declaration has no name)
  and `checker.NewDiagnosticForNode` accepts nil. Callers that pass a `P<Node>` compile unchanged.

## Shared-file edits

None.

## Needs from others

None. (Uses `Resolver::{is_symbol_accessible, get_referenced_value_declaration_unsafe,
is_expando_function_declaration_unsafe, requires_adding_implicit_undefined_unsafe}` with `c` as documented.)

## Doubts

- Go's `createDiagnosticForNode(n, getErrorByDeclarationKind(k))` / `getRelatedSuggestionByDeclarationKind(k)` with a
  nil message panics in `ast.NewDiagnostic` (nil deref); the port `.unwrap()`s the `Option<&Message>` at each such
  call site, which panics at the same point (before the diagnostic is created; `NewDiagnosticForNode` only computes
  the error range first, which has no side effects).
- `handleSymbolAccessibilityError`: `lateMarkedStatements` append-if-unique uses pointer equality on `P<Node>`, as
  `core.AppendIfUnique` compares `*ast.Node` with `==`.
- The `panic(... + node.Kind.String())` messages use `kind_string()` (Rust `Debug`, e.g. `ClassDeclaration`), not
  Go's `KindClassDeclaration`; only affects panic text.
