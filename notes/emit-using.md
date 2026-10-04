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

## Results (2026-10-03, branch head before this note: 2c25aae)

### This branch alone (main d3a2598 + E3/E4 f708ab1 + using.go)
- `tsrs-test run --suite all --baselines js`: 8607 / 15197 pass, 0 fail (emit/transforms: 8579). +28 newly passing,
  0 lost: usingDeclarations.2/.3, usingDeclarationsInFor, InForOf.1, InForAwaitOf, TopLevelOfModule.1/.2/.3,
  WithImportHelpers, HoistedExportOrder, NestedRest, awaitUsingDeclarations.2/.3, awaitUsingDeclarationsInFor,
  InForAwaitOf, InForOf.1, TopLevelOfModule.1, WithImportHelpers (several target variants each).
- All remaining using/await-using variants that do not pass stop in other waves' stubs: legacydecorators 48,
  esdecorator 25, forawait 12, classfields 9 (plus 138 es5 variants that the harness skips).
- Default mode is unchanged: conformance errors + `--baselines types,symbols` whole trees byte-identical to main d3a2598
  with default settings and with `TSRS_LAZY_MEMBERS=0`; fourslash 4066/63 with main's pass list; `RUSTFLAGS="-D warnings"
  cargo check --workspace --locked --all-targets` clean; `emit_gate.rs` passes (TSRS_EMIT stays opt-in).

### Local composition check (not pushed): this branch + emit/classfields d949ed9 + emit/async 4e99392
### + emit/jsx-decorators 3056a75 + emit/es2016-2020 d63c040
Conflicts resolved only in mod.rs lists, duplicate utilities/destructuring/ast helpers (kept one copy).
- js baselines: 13094 / 15197 pass, 0 fail. Using-related variants (`[uU]singDeclaration` ids): 179 pass, 0 fail,
  25 crash only in the esdecorator.go stub (`usingDeclarationsNamedEvaluationDecoratorsAndClassFields`,
  `usingDeclarationsWithESClassDecorators.*`), 138 skipped (es5).
- CLI oracle with tsgo built from ts-ref b85298b6a81f (`go build ./cmd/tsc`, go1.27.1): the 65 single-file
  using/await-using conformance+compiler tests without ES decorators (27 decorator files excluded), compiled as one
  program, outputs in /tmp: for `--module esnext|commonjs` x `--target es2022|es2017` (`--moduleResolution bundler
  --lib esnext,dom --strict false --skipLibCheck`): 65/65 .js files byte-identical in each of the 4 modes, tsc output
  (diagnostics) byte-identical, both exit 2 (the tests contain intentional errors). 49 outputs contain
  `__addDisposableResource`, 28 the async `await result_1` dispose path, 56 loops.

## Left
- ES-decorator variants (25) need esdecorator.go (assigned elsewhere).
