# checker-08 notes (checker_08.rs = checker.go:15239–17432)

## Signature changes

- `get_resolved_members_or_exports_of_symbol`: `-> P<SymbolTable>` -> `-> Option<P<SymbolTable>>`.
  Go returns `combineSymbolTables(early, late)`, which is nil when both are nil (e.g. a class with no members/exports);
  nil in the links also means "not yet resolved", so an empty table cannot be substituted without changing caching.
  Callers in other files (checker.go:1477 in checker_01) must handle `None` (Go indexes the nil map -> no entry).

## Shared-file edits

- `crates/tsrs_checker/src/program.rs`: added `fn get_project_reference_from_source(&self, path: &Path) -> Option<&'static SourceOutputAndProjectReference>`
  to the `Program` trait (Go `modulespecifiers.ModuleSpecifierGenerationHost.GetProjectReferenceFromSource`, used by
  `resolve_external_module` at checker.go:15655).
- `crates/tsrs_compiler/src/checker_program.rs`: implemented it like the other project-reference methods (always `None`).
  Note: `cargo check -p tsrs_compiler` fails at body-base already (`program.rs:1233` private `get_diagnostics`), unrelated.

## Needs from others

- `resolve_external_module_symbol` (mine) keeps the generated `P<Symbol> -> P<Symbol>` signature, although Go accepts and
  returns nil (`if moduleSymbol != nil`). Callers with a nil-able symbol must `.map(|s| c.resolve_external_module_symbol(s, ..))`
  (Go: nil in, nil out). Changing it to Option now would break callers already written against P.
- `get_type_from_import_attributes` (checker_03, checker.go:5569) is `P<Node> -> P<Type>` but Go takes nil and returns nil.
  I call it as `ast::get_import_attributes(n).map(|a| c.get_type_from_import_attributes(a))` (`// SIG:` comment at the call).
  Should be `Option<P<Node>> -> Option<P<Type>>`.

## Doubts

- `resolve_qualified_name`: Go `c.globalObjectType != nil` is ported as `self.global_object_type.id != TypeId(0)`
  (before init the field holds the id-0 placeholder from `new_checker`; real types get ids >= 1 as `newType` increments
  `TypeCount` first). Relies on the new_type port keeping that.
- `get_write_type_of_instantiated_symbol`: unwraps `get_write_type_of_symbol(target)`; Go would pass nil to `instantiateType`
  (nil out) and store nil. Only reachable for a synthetic target with neither write nor resolved type.
- `mark_symbol_of_alias_declaration_if_type_only`: unwraps `get_symbol_of_declaration`; Go would use a nil key in the links store.
- Go map iteration order (lookup table of export collisions, globals in `get_ambient_modules`, `range source` in
  `extend_export_symbols`) is random; the port iterates FxHashMap / SymbolTable insertion order.
