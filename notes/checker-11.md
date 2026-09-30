# checker-11 notes (checker_11.rs = checker.go 21764–23953)

## Signature changes

- `has_common_declaration`: `symbols: P<OrderedSet<P<Symbol>>>` -> `symbols: &OrderedSet<P<Symbol>>` (Go passes a pointer to a
  stack-local set; only caller is `create_union_or_intersection_property` in this file).
- `instantiate_mapped_type`: `alias: P<TypeAlias>` -> `alias: Option<P<TypeAlias>>` (Go passes `newAlias`, which is nil when
  the type has no alias).
- `instantiate_type_alias`: `(alias: P<TypeAlias>, m) -> P<TypeAlias>` -> `(alias: Option<P<TypeAlias>>, m) -> Option<P<TypeAlias>>`
  (Go: `if alias == nil { return nil }`; every caller passes a possibly-nil `t.alias`). The caller in checker_12
  (checker.go:24906, `result.alias = c.instantiateTypeAlias(root.alias, mapper)`) must pass/assign the Option.

## Shared-file edits

- `types.rs`: `TypeNodeLinks.outer_type_parameters` `Cell<&'static [P<Type>]>` -> `Cell<Option<&'static [P<Type>]>>`.
  `getObjectTypeInstantiation` distinguishes nil (not computed) from empty (computed, none in scope; Go stores `[]*Type{}`).
  Only checker_11 uses this field.

## Needs from others

- `is_node_descendant_of(node, ancestor: P<Node>)` (utilities.go): Go accepts a nil ancestor (returns false). `get_this_type`
  guards the call with `container.body().is_some_and(..)` instead of passing nil.
- `get_write_type_of_symbol` returns `Option`; `create_union_or_intersection_property` unwraps it where Go appends it to
  `writeTypes` (Go never produces nil there in practice).
- `get_this_container` in tsrs_ast returns `P<Node>` (non-nil); Go checks `container != nil` in `getThisType` — dropped.

## Doubts

- `fill_missing_type_arguments`: Go passes the `result` slice itself to `newTypeMapper(typeParameters, result)` and keeps
  writing `result[j]` for later `j`, so lazily-evaluated instantiations (and `compareTypeMappers` on Array mappers) see the
  final values. The arena mapper takes a snapshot (`alloc_slice(&result)`) at each iteration. Only observable for forward
  references in type parameter defaults (already an error) or ordering ties; faithful emulation needs a mutable-target mapper.
- `get_type_arguments`: Go tests `resolvedTypeArguments == nil`; `resolved_type_arguments` is `Cell<&'static [P<Type>]>`, so
  an empty (non-nil in Go) result is recomputed on every call. Go's instantiateTypes of an empty list also yields nil in the
  common cases, so this should match, but a zero-arity reference via a type node goes through push/popTypeResolution again.
- `instantiate_list`/`instantiate_types` return a `Vec` copy; callers that used `core.Same(new, old)` compare elements instead
  (equivalent: a new slice is only built when some element changed).
- `push_active_mapper`/`pop_active_mapper`: Go keeps cleared maps in the slice capacity for reuse; the port pushes a fresh map
  and pops (drops) it. The cache map at an existing index is never replaced while in use, so indexing after the worker call is
  equivalent to Go holding the map reference.
- `get_resolved_type_parameter_default`: Go's `targetDefault != nil` else-branch is unreachable (the function never returns
  nil) and is not ported as a separate branch.
- `is_mapped_type_with_keyof_constraint_declaration` unwraps the constraint declaration where Go would nil-deref.
