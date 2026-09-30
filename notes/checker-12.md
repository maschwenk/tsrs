# checker-12 notes (`checker_12.rs` = checker.go 23954–26122)

All functions of the range are ported; no `todo!()` left (the only match is the generated header comment).

## Signature changes

None in `checker_12.rs`.

## Shared-file edits

None.

## Needs from others

- `instantiate_type_alias` (checker_11.rs, checker.go:23185): Go takes and returns `*TypeAlias` and returns nil for a nil
  alias. The generated signature is `(alias: P<TypeAlias>, ..) -> P<TypeAlias>`; it should be
  `(alias: Option<P<TypeAlias>>, ..) -> Option<P<TypeAlias>>`. `get_conditional_type` adapts at the call site with
  `root.alias.get().map(|a| self.instantiate_type_alias(a, mapper))` (`// SIG:` comment).
- `get_type_from_import_attributes` (checker_03.rs, checker.go:5569): Go returns nil for a nil node. The signature should be
  `(node: Option<P<Node>>) -> Option<P<Type>>`. `get_type_from_import_type_node` adapts with
  `ast::get_import_attributes(node).map(|a| self.get_type_from_import_attributes(a))` (`// SIG:` comment).
- `some_type` / `every_type` (checker_13.rs) take `impl FnMut(P<Type>) -> bool` with no checker argument, so callbacks that
  need the checker cannot be passed. `get_conditional_type` (someType with isTypeAssignableTo) and
  `get_conditional_flow_type_of_type` (everyType with isArrayOrTupleType) inline the Go `someType`/`everyType` body
  (union -> any/all over `types()`, else call on `t`). If checker-13 adds checker-taking variants, those two sites could
  switch to them.
- `get_actual_type_variable` (checker_15.rs) returns `Option`; Go never returns nil in practice. Callers here `unwrap()`
  where Go passes the result on to `instantiateType`, and compare the `Option`s where Go compares pointers.

## Doubts

- `get_infer_type_parameters`: Go ranges over `node.Locals()` (a Go map, random order); the port uses the ordered
  SymbolTable's `values()` (insertion order). Go's result order is nondeterministic, so this cannot be matched exactly.
- `get_type_from_conditional_type_node`: Go creates `root.instantiations` when `outerTypeParameters != nil` (a filter that
  removes every element yields a non-nil empty slice). The port uses `!is_empty()`. Unobservable, because the map is only
  read when `len(outerTypeParameters) != 0`.
- `get_type_alias_instantiation`: Go would panic when it writes into a nil `links.instantiations` (a non-generic alias);
  `GoMap::set` creates the map instead. Only reachable for inputs where Go crashes.
- `get_conditional_flow_type_of_type`: `node.Parent` is unwrapped in the loop, as Go dereferences it (`ast.IsParameterDeclaration(parent)`).
- `map_type_ex`: the recursion goes through a private `map_type_ex_worker` that takes `&mut dyn FnMut` (a generic
  recursive `impl FnMut` would not monomorphize). Behavior is unchanged.
- `new_intrinsic_type_ex` / `new_unique_es_symbol_type` / `new_template_literal_type` / `get_string_literal_type` allocate
  their `&str` arguments into the arena (`alloc_str`) on every call, because the signatures take `&str`.
