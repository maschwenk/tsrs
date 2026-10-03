# emit-using (E6 part: using.go)

Branch `mfs-cx/emit-using`, based on `emit/transforms` f708ab1 (E3/E4 checkpoint, itself merged with main d3a2598).

## Explicit dependencies (copied verbatim, not modified)
From `origin/emit/classfields` at d949ed9 (E5):
- `crates/tsrs_transformers/src/estransforms/namedevaluation.rs` (`is_named_evaluation`, `transform_named_evaluation`)
- `crates/tsrs_transformers/src/estransforms/utilities.rs` (`convert_class_declaration_to_class_expression`)
- `crates/tsrs_transformers/src/estransforms/classthis.rs` (`is_class_this_assignment_block`, used by namedevaluation)
When emit/classfields lands, these files are identical add/add and merge cleanly; the `mod.rs` lines are the only overlap.

Printer helpers `new_add_disposable_resource_helper` / `new_dispose_resources_helper` come from main (E1, factory_2.rs).

## Status
- using.go ported completely (crates/tsrs_transformers/src/estransforms/using.rs).
- Deviation in shape only: Go passes a nil element list to `NewArrayLiteralExpression` for `stack: []`; the Rust factory
  requires a list, so an empty one is passed (prints `[]`).
