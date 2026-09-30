# jsx agent notes (`crates/tsrs_checker/src/jsx.rs` <- `jsx.go`)

All 60 functions ported. No `todo!()` left.

## Signature changes

All in jsx.rs; none of these functions is called from outside jsx.go in Go except where noted.

- `get_jsx_element_properties_name(P<Symbol>)` -> `(Option<P<Symbol>>)`: Go passes `getJsxNamespaceAt`'s nil-able result
  straight through and `getNameFromJsxElementAttributesContainer` nil-checks it.
- `get_jsx_element_children_property_name(P<Symbol>)` -> `(Option<P<Symbol>>)`: same reason.
- `get_jsx_managed_attributes_from_located_attributes(.., ns: P<Symbol>, ..)` -> `ns: Option<P<Symbol>>`: same reason.
- `parse_isolated_entity_name(&str) -> P<Node>` -> `Option<P<Node>>`: `parser.ParseIsolatedEntityName` returns nil on invalid input.
- `get_jsx_runtime_import_specifier(..) -> (String, P<Node>)` -> `(String, Option<P<Node>>)`: specifier is nil-able
  (matches `Program::get_jsx_runtime_import_specifier`).
- `generate_jsx_children(node, impl FnMut..) -> Vec<JsxElaborationElement>` ->
  `(node, InvalidTextDiagnosticFn) -> impl FnMut(&mut Checker) -> Option<JsxElaborationElement>`: Go returns a lazy
  `iter.Seq`; eager collection would reorder number-literal type creation (type ids) relative to the elaboration work.
  The pull closure reproduces Go's interleaving exactly (the consumer never breaks early).
- `elaborate_iterable_or_array_like_target_elementwise(iterator: &[JsxElaborationElement], ..)` ->
  `(iterator: impl FnMut(&mut Checker) -> Option<JsxElaborationElement>, ..)`: consumer side of the above.
- `get_elaboration_element_for_jsx_child(.., impl FnMut(&mut Checker) -> (..))` -> `(.., InvalidTextDiagnosticFn)`:
  the returned element's `create_diagnostic` (`Rc<dyn Fn>`, from jsx_types.rs) must capture the callback, so it is
  shared: `pub(crate) type InvalidTextDiagnosticFn = Rc<dyn Fn(&mut Checker) -> (&'static Message, Vec<String>)>`
  (declared in jsx.rs). The memoization of Go's `getInvalidTextualChildDiagnostic` is a `RefCell` inside that closure.

Kept as generated although Go never actually returns nil (callers `unwrap()`):
`get_intrinsic_attributes_type_from_jsx_opening_like_element`, `get_effective_first_argument_for_jsx_signature`,
`get_jsx_props_type_from_call_signature`, `get_jsx_props_type_from_class_type`, `get_jsx_stateless_element_type_at`.

## Shared-file edits

- `crates/tsrs_checker/Cargo.toml`: added `tsrs_parser.workspace = true` (for `parser.ParseIsolatedEntityName`;
  no dependency cycle: tsrs_parser depends only on core/diagnostics/ast/scanner). `Cargo.lock` updated accordingly.

## Needs from others

- `resolve_symbol(P<Symbol>)` (checker_07?): Go's `resolveSymbol(nil)` returns nil and jsx.go relies on that twice
  (`getJsxNamespaceAt`). I adapt at the call site with `.map(|s| self.resolve_symbol(s))` (`// SIG:` comments); no change
  strictly needed.

## Doubts

- `checkJsxPreconditions` / `getJsxStatelessElementTypeAt` compare `getJsxElementTypeAt(..)` with nil, but it can never be
  nil (`getJsxType` falls back to `errorType`). Ported faithfully: the call is kept for its side effects, the nil branch
  is dead (so "JSX element implicitly has type 'any' because the global type 'JSX.Element' does not exist" is never
  reported from there, same as Go).
- `createJsxAttributesTypeFromAttributesProperty`: `getSpreadType(spread, createJsxAttributesType(), attributesSymbol,
  objectFlags, false)` — the closure mutates `objectFlags` (adds `FreshLiteral`). Go's spec leaves the evaluation order of
  the plain `objectFlags` operand vs the call unspecified; gc reads such variables after the calls (verified with a tiny
  program: `g(0, create(), flags)` sees the mutated `flags`), so the port reads `object_flags` after
  `create_jsx_attributes_type` (i.e. including `FreshLiteral`).
- `TypeToString` (exported Go method) is called as `type_to_string_exported`.
