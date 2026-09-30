# relater-2 notes (`relater_2.rs` = relater.go 2580–5046)

All 60 functions ported; no `todo!()` left. Tracing-only code dropped: `traceUnionsOrIntersectionsTooLarge` is an
empty body (it only reports to the tracer), and the tracer calls in `recursiveTypeRelatedTo` /
`typeRelatedToDiscriminatedType` are gone.

## Signature changes

- `should_check_as_excess_property(prop, container: P<Symbol>)` -> `container: Option<P<Symbol>>`. Go passes
  `source.symbol` (can be nil) and only dereferences it after `prop.ValueDeclaration != nil`. Only caller is in this file.
- `Relater::chain_args_match(&self, c, args: &[&dyn Display])` -> `args: &[Option<&str>]`. Go passes `nil` as a
  wildcard; stored chain args are pre-formatted strings. Only callers are in this file.

## Shared-file edits

- `relater_types.rs`: `errorState` derives `Clone` (Go copies the struct by value; `structuredTypeRelatedToWorker`
  restores the same saved state several times).
- `relater_types.rs`: added `Relater::as_p(&self) -> P<Relater>` (same unsafe pattern as `Node::as_p`). Needed to build
  the `'static` `TypeComparer` closures where Go passes `r.isRelatedToWorker` etc. as values
  (`isTypeMatchedByTemplateLiteralType`, `newInferenceContext`, `compareSignaturesRelated`). relater-1 may want it too.

## Needs from others

- `Relation::get(&self, key)` (relater_1.rs) is shadowed by `P::get`; called as `Relation::get(&rel, key)` here.
  Works, but a rename to `lookup` would be friendlier (PORTING.md "P::get shadows T::get").
- `type_comparer` leaks one small closure per call (`alloc`). `signature_related_to` builds one per call, which is a
  hot path — memory grows with the number of signature comparisons. Consider a cache or a non-leaking comparer type
  later.
- Callers of `put_relater` must copy `r.related_info` / `r.error_chain` out before putting the relater back (Go does
  the same implicitly by holding the old slice header; here `related_info` is replaced with a fresh `Vec`).

## Doubts

- `get_error_state` clones `related_info`; Go shares the slice backing array. Go's aliasing can only become visible if a
  state saved in an inner frame is restored after an outer frame restored a shorter state and appended — LIFO usage in
  this file never does that, so a clone should be equivalent.
- `is_deeply_nested_type(t, &self.source_stack.borrow(), n)` holds a `RefCell` borrow across the call. Safe because
  that function never reaches this relater (no closure/handle to it is passed), but it breaks the "never hold a borrow
  across a checker call" rule literally.
- Error-chain args are pre-formatted with `Display` (`stringify_args`); `chain_args_match` compares strings. Go
  compares `any` values, so an int arg would never equal a string arg in Go, while "3" == "3" here. Only matters if a
  chain entry for the matched messages carried a non-string arg, which none do.
- `get_default_constraint_of_conditional_type` returns non-optional `P<Type>`; Go's `!= nil` check is therefore dropped.
- `hasExcessProperties`: where Go would panic on a nil `Name()` (`ast.IsIdentifier(nil)`), the port unwraps (panics too).
