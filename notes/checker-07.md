# checker-07 notes (checker_07.rs = checker.go:13069-15238)

## Signature changes

- `get_diagnostics(ctx, source_file, collection: P<DiagnosticsCollection>)` -> `(ctx, source_file, suggestions: bool)`.
  Go passes `&c.diagnostics` / `&c.suggestionDiagnostics`; both are plain `Checker` fields, so a flag selects the
  collection (only callers are `get_diagnostics_exported` / `get_suggestion_diagnostics` in this file).
- `add_deferred_diagnostic(callback: impl FnMut(&mut Checker))` -> `impl FnOnce(&mut Checker) + 'static`, because the
  field is `Vec<Box<dyn FnOnce(&mut Checker)>>`.
- `get_export_symbol_of_value_symbol_if_exported(Option<P<Symbol>>) -> P<Symbol>` -> `-> Option<P<Symbol>>`
  (Go returns nil for nil input; every Go caller nil-checks the result).
- `check_and_report_error_for_resolving_import_alias_to_type_only_symbol(node, resolved: P<Symbol>)` ->
  `resolved: Option<P<Symbol>>` (Go passes the possibly-nil result of getSymbolOfPartOfRightHandSideOfImportEquals;
  the parameter is unused in Go too).
- `get_target_of_module_default(..) -> P<Symbol>` -> `Option<P<Symbol>>` (returns possibly-nil exportDefaultSymbol).
- `resolve_export_by_name(..) -> P<Symbol>` -> `Option<P<Symbol>>` (nil when no such export; callers compare with nil).
- `get_target_of_namespace_import(..) -> P<Symbol>` -> `Option<P<Symbol>>` (nil when the module does not resolve).

## Shared-file edits

None.

## Needs from others

Adapted at call sites with `// SIG:` comments; these callees should accept/return Option like Go:
- `instantiate_contextual_type` (checker_15?): Go takes and returns a nil-able contextual type. Called here via
  `contextual_type.map(..)`.
- `resolve_external_module_symbol` (checker_08): Go takes and returns a nil-able module symbol. Called via `.map(..)`
  / wrapped in `Some(..)` where the input is non-nil.
- `resolve_es_module_symbol` (checker_08): Go takes and returns a nil-able symbol. Called via `.map(..)` (equivalent:
  for nil input Go returns nil).
- `get_type_from_import_attributes` (checker_03): Go returns nil for a nil node. Called via
  `ast::get_import_attributes(..).map(..)`.
- `resolve_symbol` / `resolve_symbol_ex` / `get_merged_symbol` take `P<Symbol>` although Go routinely passes nil
  (`getMergedSymbol` even nil-checks). Kept as generated; all nil-able call sites here use `.map(..)`. Other agents
  must do the same.

## Doubts

- `report_non_exported_member`: Go's `findInMap(exports, ...)` iterates a Go map (random order); the port scans the
  SymbolTable in insertion order. Only matters when several exports alias the same local (message arg differs).
- `check_object_literal`: Go's nested `createObjectLiteralType` closure is a free fn `create_object_literal_type` over
  an `ObjectLiteralState` struct holding the mutable locals it reads; the final `mapType` callback borrows that state.
- `produce_deferred_diagnostics`: callbacks appended while running are dropped (as in Go, which ranges over the
  original slice and then sets the field to nil).
- `check_contextual_deprecations` still calls `is_canceled()` (a stub in utilities.rs; cancellation is out of scope).
