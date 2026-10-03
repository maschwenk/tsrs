# emit/jsx-decorators (waves E10 + E11)

Branch `emit/jsx-decorators`, based on `emit/core` (merged, not rebased).

## Ported

- `crates/tsrs_transformers/src/jsxtransforms/jsx.rs`: all of jsx.go (classic `React.createElement` with
  `jsxFactory`/`jsxFragmentFactory`/`reactNamespace`, `react-jsx`/`react-jsxdev` with the automatic runtime import or
  `require`, the `createElement` fallback for a `key` after a spread, entity decoding, whitespace fixup). `preserve`
  and `react-native` do not run the transformer (`CompilerOptions.GetJSXTransformEnabled`, emitter.go).
- `crates/tsrs_transformers/src/tstransforms/legacydecorators.rs`, `metadata.rs`, `typeserializer.rs`: all functions.
- `crates/tsrs_transformers/src/utilities.rs`: `IsGeneratedIdentifier`, `MoveRangePastModifiers`,
  `MoveRangePastDecorators` (utilities.go; emit/transforms has the same functions in its `todo_core.rs`, keep one).
- `crates/tsrs_transformers/src/emit_test.rs`: isolated smoke tests (program + lent checker + transforms + printer).
- `tools/oracle/emit/list-packages.js`: classifies the monorepo packages (jsx / decorators); the oracle itself is emit/core's `tools/oracle/emit/monorepo.sh`.

## Decisions / deviations in shape

- Go `tx.Visitor().Visit(node)` (the raw visit callback, no SyntaxList lifting) is a direct call of the
  transformer's own `visit` (it is the callback the visitor holds).
- Go `visit` funcs take nil-able nodes only where Go can pass nil (`JSXTransformer.visit`, for `{}` expressions).
- Go appends possibly-nil visitor results into slices in a few places (`transformDecorators`,
  `transformJsxAttributesToExpression`, the constructor decoration statement); the Rust code skips `None`. In all of
  these Go would print a nil node (panic), and the inputs cannot produce nil there.
- `typeserializer.go` exported/private name pairs (`SerializeTypeOfNode`/`serializeTypeOfNode`): the private one
  gets a trailing `_` (printer convention).
- `metadataSerializer.c` (the context) is a `Cell` of a `Copy` struct; `defer` restores are explicit.
- `collections.GroupBy` (metadata decorators last) is `Iterator::partition` (stable, same order).
- jsx `getSortedSpecifiers` sorts by property name then name, case-sensitive (so `Fragment` < `jsx`), as Go.

## Monorepo oracle

`tools/oracle/emit/monorepo.sh [--only jsx|decorators] [--keep] [pkg...]` lists the packages whose `build` script
runs tsc (`list-packages.js`, effective options from `tsgo --showConfig`), emits each with tsgo and with
`TSRS_EMIT=1 tsrs` into `$OUT/emit-go/<pkg>` / `$OUT/emit-rs/<pkg>` (redirecting `outDir`, `declarationDir` when
the config sets one, and `tsBuildInfoFile`) and compares every file byte for byte. It aborts if anything under the
package directory is newer than its start marker, and checks `git status` before/after. (An early version
mis-parsed empty TSV columns and wrote one package's declaration output into the read-only checkout; the files were
removed (checkout verified clean) and the script now stops at the first stray write.)

The private monorepo has 11 packages that compile JSX and 13 that use experimentalDecorators (names not recorded
here: the corpus is private).

## Status / blocked on

- End-to-end emit of any TS file needs the type eraser, import elision, runtime syntax (emit/transforms, E3) and the
  ES module transformer (E4); the `.js` baseline harness (`--baselines js`) is E2 on emit/core.

## Local integration results (2026-10-03)

Local, unpushed branch `int/local` = this branch + emit/transforms' typeeraser/importelision/runtimesyntax adapted to
emit/core + a minimal esmodule.go, built as `tsrs`; outputs compared with tsgo (source maps off on both sides,
`EXTRA="--sourceMap false --declarationMap false"`, because the generator is emit/sourcemaps'):

- Monorepo, JSX packages: 703 files identical, 0 different; 1 package not emitted (commonjs, E4).
- Monorepo, decorator packages: 318 identical, 0 different; 104 not emitted, all in the 7 es2024 packages, which need classfields.go (E5, unassigned) because target < ESNext runs the class fields
  transformer.
- `tools/oracle/emit/cases.py` (tsgo vs tsrs through the CLIs, per test variant) over the 604 compiler/conformance
  tests with `@jsx`, `@experimentalDecorators` or `@emitDecoratorMetadata`: 980 variants, 244 identical, 43 both
  empty, **0 different**, 693 stop in other waves' gate stubs (commonjsmodule 195, forawait 175, classfields 163,
  using 144, objectrestspread 7, esdecorator 3, nullishcoalescing 1, int/local esmodule gaps 4).

## `--baselines js` (E2 harness), local integration build (`int/local`), 2026-10-03

Test lists: every variant of the tests in conformance/jsx (230 variants), and of conformance/decorators +
compiler/decorator* + *Metadata* (213 variants).

| group | total | pass | fail | crash (other waves' stubs) | skip |
| --- | ---: | ---: | ---: | ---: | ---: |
| jsx | 230 | 90 | 0 | 137 (forawait, commonjsmodule, classfields, source maps) | 3 |
| decorators | 213 | 58 | 0 | 86 (forawait, commonjsmodule, classfields, esdecorator) | 69 |

On `emit/jsx-decorators` alone (type eraser still a stub) nearly every variant crashes in typeeraser.

## Full integration (local `int/all`, never pushed), 2026-10-03

`int/all` = this branch (with origin/main) + origin/emit/transforms (4c40ea2, incl. commonjs) + emit/classfields +
emit/async + emit/es2016-2020 + origin/emit/sourcemaps; reference = tsgo built from the pinned ts-ref commit
(`go build ./cmd/tsc`, Go 1.27.1).

- `--baselines js`, jsx group: 226 / 230 pass, 0 fail, 1 crash (`tsxEmit3`: emitter source-map stub path), 3 skip.
- `--baselines js`, decorators group: 131 / 213 pass, 0 fail, 13 crash (ES decorators, out of scope), 69 skip.
- `--baselines js`, whole suite: 13162 / 15197 pass, 15 fail, 214 crash (esdecorator 116, using 98), 1 timeout,
  1805 skip. Of the 15 fails, 13 pass when rerun alone (`--list` with only that variant): they fail only when their
  sibling `moduleDetection=` variants run in the same process (e.g. commentsOnJSXExpressionsArePreserved with
  moduledetection=force loses `Object.defineProperty(exports, "__esModule", ...)`), so that is state shared across
  variants in the harness/program setup, not the transformers. The other 2 (`exportNonInitializedVariablesInIfThenStatementNoCrash1`,
  `labeledStatementExportDeclarationNoCrash1`, module=commonjs) are commonjs-transform diffs (emit/transforms).
- Monorepo oracle, all 103 tsc-built packages, full emit with source and declaration maps: 103/103 packages fully
  identical, 10,248 files identical, 0 different, 0 missing, 0 extra; checkout unchanged.

Standalone (`emit/jsx-decorators` alone): `--baselines js` 1364 / 15197 pass, 0 fail (typeeraser still a stub here, so
everything else crashes there); conformance errors + types/symbols identical to main in both lazy modes; fourslash
4066/63 same pass list; `RUSTFLAGS="-D warnings" cargo check --workspace --locked --all-targets` clean; emit_gate 3/3.
