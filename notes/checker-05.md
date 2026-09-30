# checker-05 notes (checker.go 8749–10856, 86 functions)

## Signature changes

- `get_candidate_for_overload_failure`: `candidates: &[P<Signature>]` -> `&mut [P<Signature>]`. Go passes
  `s.candidates` and `pickLongestCandidateSignature` writes `candidates[bestIndex]` into that shared backing array
  (visible through `s.candidates` / `*candidatesOutArray`). Only caller is `resolve_call` (this file).
- `infer_from_annotated_parameters_and_return`: `inference_context: P<InferenceContext>` -> `Option<P<InferenceContext>>`.
  Go passes `c.getInferenceContext(node)` which can be nil; it is only dereferenced when a parameter/return type
  annotation exists (unwrapped exactly there). Only callers are in this file.

## Shared-file edits

None.

## Needs from others

- `clone_inference_context` / `get_mapper_from_context` (inference.rs): Go accepts nil and returns nil
  (`getMapperFromContext(cloneInferenceContext(outerContext, ...))` with nil `outerContext`, and
  `getMapperFromContext(cloneInferredPartOfContext(...))` where the clone may be nil). They should take/return
  `Option`. Adapted at the call sites in `infer_type_arguments` (`// SIG:`).
- `get_mapper_from_context` returns `P<TypeMapper>` but `InferenceContext.mapper` is `Option` — should return `Option`.
- `check_grammar_type_arguments` (grammarchecks.rs): takes `P<NodeList>` but Go passes `node.TypeArgumentList()`
  which may be nil (both sub-checks then return false). Adapted in `check_tagged_template_expression` by skipping the
  call when the list is nil (`// SIG:`). Should be `Option<P<NodeList>>`.
- `candidates_out_array` aliasing: Go stores the `s.candidates` slice header into `*candidatesOutArray`, so later
  in-place updates (`chooseOverload`, `pickLongestCandidateSignature`) are visible to the caller. `resolve_call`
  re-copies `s.candidates` into the out vec before each return after those updates.

## Doubts

- `check_type_arguments` returns an empty `Vec` for Go's nil; `choose_overload` tests `is_empty()`. Equivalent
  because a successful result always has `len(typeParameters) > 0` entries there.
- `contextually_check_function_expression_or_object_literal_method`: Go `node.TypeParameters() == nil` ported as
  `node.type_parameter_list().is_none()`. Differs only if an empty-but-present `<>` list yields a nil slice in Go.
- `get_argument_arity_error` / `get_type_argument_arity_error`: Go `math.MaxInt/MinInt` ints ported as `i64`
  sentinels; formatted diagnostic arguments print identically.
- `get_constructor_accessibility_error`: `get_class_like_declaration_of_symbol(...)` unwrapped before
  `is_node_within_class` (signature takes `P<Node>`); Go would pass nil through if the symbol had no class decl.
- `type_to_string_exported` used for Go `c.TypeToString(t)` (vs `type_to_string(t, None)`); same behavior per printer.go.
