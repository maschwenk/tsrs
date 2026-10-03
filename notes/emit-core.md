# emit-core: waves E1 (pipeline) and E2 (harness and oracle)

Branch `emit/core`, PR maschwenk/tsrs#9. Design decisions: docs/EMIT.md section 10; numbers: section 13.

## Done
- E1: `tsrs_transformers` crate (base, gate stubs for every script transformer), printer factory.go rest and
  helpers.go definitions, compiler emitter/emitHost/`Program::emit`, CLI `TSRS_EMIT=1` gate + `--listEmittedFiles`,
  release guard test `crates/tsrs_cli/tests/emit_gate.rs`.
- E2: `tsrs-test --baselines js`, `tools/oracle/emit/{run.py,monorepo.sh}`.

## Shared-file edits other waves will see
- `tsrs_core/src/ptr.rs`: `P::from_static` is a `const fn` (statics of helper definitions).
- `tsrs_ast`: `range_is_synthesized` (utilities_1.rs), `try_get_property_name_of_binding_or_assignment_element`
  (utilities_3.rs).
- `tsrs_declarations`: `Transformer`/`Resolver` moved out (re-exported); `transform_1.rs` calls
  `tx.get().base.new_transformer` (it now returns `P<Transformer>` and needs `&'static self`).

## For the other waves
- Port into the stub files; keep the stub's struct/constructor and extend them. Add `use super::*;` where a module
  file needs sibling functions.
- `TODO(emit/sourcemaps)`: `print_source_file` in tsrs_compiler/src/emitter.rs (generator branch), `SourceMapEmitResult.source_map`.
- `TODO(emit/incremental)`: `WriteFileData.build_info`, `handle_no_emit_options(emit_build_info)`, harness `create_program`.
- `destructuring.go` and the remaining transformers/utilities.go functions are not ported (E4 / whoever needs them).

## Doubts
- None in ported code; the stubs are deliberately incomplete.
