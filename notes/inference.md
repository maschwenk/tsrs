# inference agent notes

## Signature changes

- `clone_inference_context(n: P<InferenceContext>, ..) -> P<InferenceContext>` ->
  `(n: Option<P<InferenceContext>>, ..) -> Option<P<InferenceContext>>`. Go checks `n == nil` and returns nil;
  checker.go:9612 passes `outerContext` (from `getInferenceContext`, nil-able).
- `get_mapper_from_context(n: P<InferenceContext>) -> P<TypeMapper>` ->
  `(n: Option<P<InferenceContext>>) -> Option<P<TypeMapper>>`. Go returns nil for nil; callers pass
  `cloneInferredPartOfContext(..)` / `getInferenceContext(..)` results (both nil-able) and feed the result to
  `instantiateType` (whose mapper param is already `Option`).

## Shared-file edits

- `inference_types.rs`: `InferenceState.inferences` `Cell<&'static [P<InferenceInfo>]>` -> `RefCell<Vec<P<InferenceInfo>>>`.
  `infer_types` receives a borrowed `&[P<InferenceInfo>]` (not `'static`); storing it would need an arena
  allocation per `inferTypes` call. The pooled Vec is cleared in `put_inference_state` (Go: `n.inferences[:0]`)
  and refilled by copy. Only inference.rs uses InferenceState.
- `checker.rs`: `reverse_mapped_cache: FxHashMap<ReverseMappedTypeKey, P<Type>>` -> `FxHashMap<.., Option<P<Type>>>`.
  Go stores nil results and `inferReverseMappedType` distinguishes a cached nil (comma-ok) -> returns unknownType.

## Needs from others

- `some_type(t, f: impl FnMut(P<Type>) -> bool)` (checker.go:27007, free fn) is called with a closure capturing
  `self` (`|t| self.is_mutable_array_like_type(t)`); fine as long as it stays a non-checker free fn.
- `compare_types_assignable_comparer()` allocates a closure per call; used for `c.compareTypesAssignable` values in
  `new_inference_context` and the template-literal paths. Harmless, but a cached `TypeComparer` field would be cheaper.

## Doubts

- `invoke_once` holds a shared `RefCell` borrow of `n.source_stack`/`n.target_stack` across `is_deeply_nested_type`
  (assumed non-reentrant into inference on the same state). `infer_reverse_mapped_type` moves the checker stacks out
  with `mem::take` for the same call and restores them.
- `infer_from_matching_types` sorts with Rust's stable `sort_by` where Go uses `slices.SortFunc` (pdqsort,
  unstable). `matchedTargets` has no duplicates and `CompareTypes` is a total order on distinct types, so the
  result should be identical; only matters if `compare_types` can return 0 for distinct types.
- `infer_type_for_homomorphic_mapped_type`: Go stores nil results in `reverseHomomorphicMappedCache`; a stored nil
  reads back as a miss (`cached != nil`), so the port simply does not insert None.
- `get_common_supertype`: Go's `core.Same(primaryTypes, types)` (slice identity after `SameMap`) is ported as
  element-wise equality of the mapped vec, which is what SameMap identity means.
- InferencePriority ordering (`min`, `<`) compares `.bits()` (i32; Circularity = -1) rather than the derived Ord.
- Several places clone small candidate/declaration vecs before iterating with checker calls (getInferredType,
  getCovariantInference, isFromInferenceBlockedSource, inferFromProperties, inferFromIntraExpressionSites) to avoid
  holding RefCell borrows across re-entrant calls. inferFromIntraExpressionSites clears the Vec at the end (Go sets
  nil), matching Go's range-over-snapshot semantics.
