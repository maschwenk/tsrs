# checker-03 notes (checker_03.rs = checker.go 4321–6526)

All 75 functions in the range are ported; no `todo!()` left.

## Signature changes

- `get_type_from_import_attributes`: `(node: P<Node>) -> P<Type>` -> `(node: Option<P<Node>>) -> Option<P<Type>>`.
  Go takes nil attributes and returns nil for them; every caller passes `ast.GetImportAttributes(...)` (nil-able)
  straight into `resolveExternalModuleName(..., importAttributesType *Type)` which is already `Option<P<Type>>`.
  Callers in other files (checker.go:14758, 14802, 14843, 14868, 14877, 14890, 14916, 15199, 15383, 16497, 25057)
  should pass `get_import_attributes(n)` directly and forward the `Option`.

## Shared-file edits

None.

## Needs from others

- `get_export_symbol_of_value_symbol_if_exported` (checker_07, checker.go:14612) returns `P<Symbol>` but Go returns nil
  for a nil input (`getMergedSymbol(nil)`); should be `-> Option<P<Symbol>>`. Adapted in `check_export_assignment`
  with a `// SIG:` comment (only called when the resolved symbol is non-nil).
- `include_undefined_in_index_signature` (flow.rs, flow.go:2339) takes/returns `P<Type>` but Go handles nil
  (`if t == nil { return nil }`); should be `Option<P<Type>> -> Option<P<Type>>`. Adapted in
  `get_iterated_type_or_element_type` with `.map(...)` and a `// SIG:` comment.
- `add_deferred_diagnostic(callback: impl FnMut(&mut Checker))` has no `'static` bound, but it must store the closure
  (`Vec<Box<dyn FnOnce(&mut Checker)>>`); it will need `+ 'static`. My closures are `move` and `'static`.

## Doubts

- Go map iteration order: `checkKindsOfPropertyMemberOverrides` (notImplementedInfo), `checkTypeForDuplicateIndexSignatures`
  (indexSignatureMap) use `OrderedMap` (insertion order); `checkExternalModuleExports` and `hasExportedMembersOfKind`
  iterate the symbol table in insertion order. Go's order is random; diagnostics are sorted later, but side effects of
  `getSymbolFlags` (alias resolution) in `hasExportedMembersOfKind` could happen in a different order than a given Go run.
- `checkModuleAugmentationElement`: the `fallthrough` from `KindImportEqualsDeclaration` was folded into one match arm
  with an early `return` for internal-module import-equals.
- `checkDecorator`: the `PropertyDeclaration` fallthrough is expressed as a guarded arm (`if !legacy_decorators`) followed
  by `PropertyDeclaration | Parameter`.
- `checkClassForStaticPropertyNameConflicts`: the class symbol lookup/`symbolToString` happens inside the matching arm,
  same as Go's argument evaluation (only when an error is reported).
