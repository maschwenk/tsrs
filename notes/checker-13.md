# checker-13 notes (checker_13.rs = checker.go 26123–28299)

## Signature changes

- `contains_type(types, t) -> bool` -> `contains_type(c: &mut Checker, types, t) -> bool` and
  `insert_type(types, t)` -> `insert_type(c: &mut Checker, types, t)`: both binary-search with `CompareTypes`, whose
  Rust port (`compare_types(c, ..)`) needs the checker (Type has no checker back pointer). External Go callers:
  flow.go:2439, relater.go:2870/2879/2953/3007/3034, checker.go:31328 — they must pass `c`.
- `remove_subtypes(..) -> Vec<P<Type>>` -> `Option<Vec<P<Type>>>`: Go returns nil ("too complex") as a distinct result.
  Only caller is in this file.
- `add_types_to_intersection` / `add_type_to_intersection`: `type_set: P<orderedSet<P<Type>>>` ->
  `&mut orderedSet<P<Type>>` (Go passes `&orderedTypes`, a stack value that `add` mutates; `orderedSet::add` takes
  `&mut self`). Only callers are in this file.
- `get_next_base_constraint(t: P<Type>, ..)` -> `t: Option<P<Type>>`: Go checks `t == nil` and is called with nil
  (missing type-parameter constraint, nil indexed-access result). Only callers are in this file.
- `expand_signature_parameters_with_tuple_members(.., rest_type: &'static TypeReference, ..)` -> `rest_type: P<Type>`:
  Go calls `restType.AsType()`; Rust payload structs have no back pointer to their `Type`. Other Go caller:
  nodebuilderimpl.go (not ported here).

## Shared-file edits

None.

## Needs from others

- `orderedSet::contains` / `add` (utilities.rs) are still stubs; `get_intersection_type_ex` depends on them.
- `docs/sigs/checker.txt` still lists the old signatures above (not edited: shared file).

## Doubts

- `get_cross_product_union_size` returns `i32` (generated sig) where Go uses 64-bit `int`; the overflow cap is
  `i32::MAX` instead of `math.MaxInt`. Observable behavior is the same (anything >= 100000 errors).
- `remove_subtypes`: the 100000-check estimate is computed in `i64` (Go 64-bit int) to avoid i32 overflow.
- `compare_type_ids` computes in i64 then truncates to i32 (sig is i32).
- Diagnostic args that are Go `any` literal values (`indexType.AsLiteralType().value`) are formatted by a local
  `literal_value_to_string` (`%v`: string as-is, jsnum Display, bool; nil -> "<nil>"; BigInt uses PseudoBigInt
  Display, which differs from Go's `%v` of the struct, but Go only formats string/number literals there).
- `get_index_node_for_access_expression` keeps the generated `Option` result although Go never returns nil
  (the ComputedPropertyName arm returns `node.expression()`); callers unwrap where Go dereferences.
- `filter_type` implements `core.Same(types, filtered)` as "no element removed" (equivalent for `core.Filter`).
- `get_write_type_of_symbol(..)` is unwrapped in `get_property_type_for_index_type` (Go dereferences it downstream).
