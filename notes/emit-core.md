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

## emit/core-2 checkpoint (PR #10)

Base: main d3a2598 (#9 merge; main a6dd429 only adds bench results). Head: d317313 (the gates below ran on it).

| check | command | result |
| --- | --- | --- |
| baselines from main | build d3a2598 in a worktree, `tsrs-test run --suite all --baselines types,symbols` (default and `TSRS_LAZY_MEMBERS=0`), `tsrs-fourslash run` | 13458 pass / 2 codes / 2 fail; .types/.symbols 12779; fourslash 4066 / 63 |
| conformance gate | same on head, `diff -r -x summary.json` of the whole `target/test-results` trees against main's | identical in both modes |
| fourslash gate | `tsrs-fourslash run`, pass list compared | 4066 / 63, identical pass list |
| warnings | `RUSTFLAGS="-D warnings" cargo check --workspace --locked --all-targets` (rustc 1.99) | exit 0 |
| release guard | `cargo test --release -p tsrs_cli --test emit_gate` | 3 passed |
| js baselines | `tsrs-test run --suite all --baselines js` | 1364 pass / 0 fail / 12032 crash (gate stubs) / 1800 skip, unchanged from main |
| declaration metric | `TSRS_TEST_DTS_ONLY=1 TSRS_TEST_RESULTS=/tmp/dts-res tsrs-test run --suite all --baselines js` | 1754 pass / 0 fail / 13 crash (declarationMap, emit/sourcemaps) |
| monorepo oracle | `TSGO=<tsgo built from ts-ref b85298b6: go build ./cmd/tsc> tools/oracle/emit/monorepo.sh <root> -j 4 -- --sourceMap false --declarationMap false --emitDeclarationOnly` | 103/103 packages, 2325/2325 .d.ts identical; root `git status --short` empty before and after |
| E12 CLI sweep | `tools/oracle/emit/run.py <small project> -- --emitDeclarationOnly <flag>` for removeComments, stripInternal, newLine crlf, emitBOM, preserveConstEnums, rootDir, noEmitOnError (with and without an error), noEmit, declaration false, isolatedDeclarations, composite+incremental, listEmittedFiles | identical outputs, diagnostics and exit codes |

Remaining / dependencies:
- The JS side of E12 (removeComments/newLine/emitBOM/preserveConstEnums in `.js`) needs the transformers: E3/E4
  (`emit/transforms`) first, then the ES transforms. Every `.js` failure on this branch is a gate-stub crash.
- 13 declaration-metric crashes are `declarationMap` (E7, `emit/sourcemaps`).
- `monorepo.sh` reports differences but exits 0 when the comparison ran; read its totals line.
- The `TODO(emit/core)` needs found on sibling branches are answered here: `ProgramLike` (emit/incremental),
  `file_output` shared (emit/sourcemaps), and the ast utilities behind emit/transforms' `todo_core.rs`. Its helper
  accessors (`rest_helper()` …) map to the statics in `tsrs_printer::helpers_defs` (`REST_HELPER` …).
