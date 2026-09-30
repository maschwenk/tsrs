# relater-1 notes (relater_1.rs = relater.go 1–2579)

## Signature changes

- `as_recursion_id<T /*? ... */>(value: T)` -> `as_recursion_id<T: Into<RecursionId>>(value: T)` (the generator left the
  Go type-set constraint as a comment; `RecursionId` has `From` impls for `P<Node>`/`P<Symbol>`/`P<Type>`).
- `get_variances_worker(symbol, type_parameters: &[P<Type>])` -> `type_parameters: &'static [P<Type>]`: the slice is
  stored in `VarianceStackEntry.type_parameters` (`&'static`). All Go callers pass stored slices
  (`InterfaceType::type_parameters()`, `TypeAliasLinks.type_parameters`), so callers only need to pass those directly.

## Shared-file edits

None.

## Needs from others

- Nil-vs-empty conventions (Go nil results mapped to an empty collection, safe because a non-nil result is never empty):
  - `infer_types_from_template_literal_type` / `infer_from_literal_parts_to_template_literal`: empty `Vec` = Go nil.
    inference.go:567 (`matches := c.inferTypesFromTemplateLiteralType(...)`; `if matches != nil`) must test `!is_empty()`.
  - `map_types_by_key_property` / `compute_key_property_name_and_map`: empty map = Go nil map; `get_key_property_name`
    stores a nil `UnionType.constituent_map` in that case.
- `instantiate_type_predicate(predicate, mapper: P<TypeMapper>)`: `get_type_predicate_of_signature` passes
  `sig.mapper.get().unwrap()` for signatures with a `target` (Go always sets `mapper` alongside `target`).
- `compare_types_assignable_worker` ignores `report_errors` exactly like Go.

## Doubts

- `elaborate_element`: `diagnostic_factory` is `Option<&mut dyn FnMut(&mut Checker, P<Node>) -> P<Diagnostic>>`; callers in
  jsx.rs must build that closure (Go passes `e.createDiagnostic`).
- `is_enum_type_related_to`: Go's `core.IfElse` evaluates `getParentOfSymbol` for both source and target eagerly; ported
  that way (parent is unwrapped only in the EnumMember case).
- `infer_from_literal_parts_to_template_literal`: Go's `addMatch` closure is a local closure taking the checker plus
  `&mut seg/pos/matches`; string slicing is by byte offset as in Go (offsets always come from `find`/`decode_js_string_rune`
  so they lie on char boundaries, except that Go can slice mid-rune where Rust would panic — only possible if a
  delimiter search lands mid-rune, which `str::find` cannot).
- `ErrorReporter` reborrowing: Go passes the same reporter to nested calls; Rust reborrows (`reborrow_reporter`).
  A Go call on a nil reporter would panic; ported as `unwrap()` at each call (only reached when `report_errors`).
