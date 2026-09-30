# checker-09 notes (checker_09.rs = checker.go 17433–19607)

## Signature changes

- `WideningContext::get_child_context(&mut self, property_name: &str) -> P<WideningContext>` ->
  `WideningContext::get_child_context(w: P<WideningContext>, property_name: &str) -> P<WideningContext>` (associated fn).
  The new child stores a pointer to its parent, so the arena handle is needed; `&mut self` is impossible on an arena
  object anyway. Callers: `WideningContext::get_child_context(context, name)` (only caller is in checker_09).
- `Checker::type_resolution_has_property(&mut self, r: P<TypeResolution>)` -> `(&mut self, r: &TypeResolution)`.
  `TypeResolution` is a plain Copy struct stored by value in `type_resolutions`, not an arena object. Only caller is
  `find_resolution_cycle_start_index` (checker_09).

## Shared-file edits

None.

## Needs from others

- `get_annotated_accessor_type(accessor: P<Node>)` (checker_10, checker.go:20439): Go is called with a nil accessor
  (getter/setter/accessor may be nil) and returns nil. Should be `Option<P<Node>>`. Adapted at call sites with
  `x.and_then(|n| self.get_annotated_accessor_type(n))` (`// SIG:` comments in get_type_of_accessors /
  get_write_type_of_accessors).
- `get_target_of_alias_declaration(node: P<Node>)` (checker_08, checker.go:16059): Go explicitly handles `node == nil`.
  Should be `Option<P<Node>>`. Adapted in get_type_of_alias with `and_then` (`// SIG:` comment).
- Data model (types.rs): `TypeReference.resolved_type_arguments` and `UnionOrIntersectionType.resolved_properties` are
  `Cell<&'static [T]>`, but Go distinguishes nil (not computed) from empty (computed, no elements). See Doubts.
- Data model (checker.rs): `this_expando_locations: FxHashMap<P<Symbol>, P<Node>>` cannot hold Go's nil location; see Doubts.

## Doubts

- `type_resolution_has_property` for `ResolvedTypeArguments`: Go tests `resolvedTypeArguments != nil`; ported as
  `!is_empty()`. A type reference whose resolved type arguments are an empty (non-nil) slice (e.g. `[]` tuple) is
  treated as "not yet resolved", which can change circularity detection in rare cases. Needs the field to become
  `Option<&'static [P<Type>]>` (whoever ports getTypeArguments has the same problem).
- `get_properties_of_union_or_intersection_type`: Go caches an empty (non-nil) result; the port recomputes when the
  cached slice is empty. Should be unobservable (the inner calls are themselves cached) but is extra work.
- `is_constructor_declared_this_property`: Go stores `location == nil` in `thisExpandoLocations`; the Rust map cannot
  store None, so a nil location is represented by a missing entry (the Go "location should be cached" panic is thus
  not reproducible). Behavior is otherwise identical.
- `get_inferred_type_parameter_constraint` (mapped-type case): Go's `core.IfElse` eagerly evaluates
  `getTypeFromTypeNode(Constraint)` even when Constraint is nil (would crash); the port only evaluates it when present.
  Mapped type parameters always have a constraint syntactically, so this should not matter.
- `keyBuilder::write_int(i32)`: Go writes `uint64(int)` (64-bit int); ported as sign-extending `value as i64 as u64`.
