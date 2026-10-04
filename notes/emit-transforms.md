# emit/transforms (E3 + E4) checkpoint

Branch `emit/transforms`, merged with main d3a2598 (emit/core).

Ported (crates/tsrs_transformers): tstransforms/{typeeraser,importelision,runtimesyntax,utilities}.rs,
inliners/constenum.rs, destructuring.rs, moduletransforms/{esmodule,externalmoduleinfo,utilities}.rs, ast_ext.rs
(Go ast helpers missing from tsrs_ast). usestrict.rs and impliedmodule.rs are E1's (already complete on main).

commonjsmodule.go is split: commonjsmodule.rs (go 1–559), commonjsmodule_2.rs (560–985),
commonjsmodule_3.rs (986–1240: variable statements, top-level nested statements), commonjsmodule_4.rs (1241–end).
Write these in chunks of <=150 lines per tool call.

Next: finish commonjsmodule, build, run gates (js baselines, conformance types/symbols both lazy modes,
fourslash, -D warnings check, emit_gate test), monorepo oracle, draft PR (label coder-task-generated).

## Status (2026-10-03)
All E3/E4 files ported. Gates: conformance errors+types/symbols identical to main d3a2598 (default and
TSRS_LAZY_MEMBERS=0), fourslash 4066/63 same pass list, `-D warnings` workspace check clean, emit_gate passes.
js baselines 8579/15197 pass, 0 fail (main: 1365). Oracle (js mode): 85/103 packages fully identical, 0 different files.

Decisions: `single_or_many` keeps Go's nil-vs-empty distinction (visitTopLevelVariableStatement returns nil when no
statements were produced, so an `if`/label body becomes `{ }`/`;` like Go). Harness source-file cache key now includes
`external_module_indicator_options.force` (Go keys on the whole SourceFileParseOptions).
Shared-file edits: tsrs_testrunner/src/compile.rs (cache key), docs/EMIT.md.

## PR description (gh could not create the PR with this token: 'Resource not accessible by integration')

Wave E3 (tstransforms core) and E4 (module transforms), ported from the Go code at ts-ref b85298b6a81f. Merged with main d3a2598 (emit/core).

## What's ported
- `tstransforms`: typeeraser, importelision, runtimesyntax (enums, namespaces, parameter properties), utilities
- `inliners/constenum`
- `destructuring.go` (crate root)
- `moduletransforms`: commonjsmodule (split into `commonjsmodule{,_2,_3,_4}.rs` by line range), esmodule, externalmoduleinfo, utilities
- `ast_ext.rs`: Go `ast` helpers that tsrs_ast doesn't have yet (`IsExportNamespaceAsDefaultDeclaration`, `IsEmpty{Object,Array}Literal`, `GetRestIndicatorOfBindingOrAssignmentElement`)

`usestrict.go` and `impliedmodule.go` were already finished on main (E1).

## Shared-file edits
- `crates/tsrs_testrunner/src/compile.rs`: the source-file cache key now includes `external_module_indicator_options.force`. Go keys the cache on the whole `SourceFileParseOptions`. Without this, `moduleDetection=force` variants reused a file parsed by another variant, which gave flaky `__esModule` diffs. The default-mode baselines are unchanged.
- `docs/EMIT.md`: inventory rows and a progress row; `notes/emit-transforms.md`.

## Numbers
- `tsrs-test run --suite all --baselines js`: **8579 / 15197 pass, 0 fail** (main: 1365). Two runs gave the same pass list.
  - 4811 tests still crash, all at other waves' stubs: classfields 1933, forawait 1847, jsx 196, legacydecorators 165, esdecorator 114, sourcemaps 101, objectrestspread 97, …
- Monorepo oracle, `tools/oracle/emit/monorepo.sh <monorepo> -j 4 -- --sourceMap false --declarationMap false`:
  - **85 / 103 packages fully identical**; 4545 files identical, **0 different**, 951 not emitted.
  - All 18 remaining packages panic in other waves' stubs (metadata 8, classfields 4, legacydecorators 3, jsx 3).
  - Reference binary: tsgo built from the pinned ts-ref commit with go1.27.1, not the npm nightly.
  - The monorepo's `git status` was empty before and after.

## Gates (all pass)
- Conformance errors plus `--baselines types,symbols`: whole `target/test-results` trees byte-identical to main d3a2598, in default mode and with `TSRS_LAZY_MEMBERS=0`.
- fourslash: 4066 pass / 63 fail, same pass list as main.
- `RUSTFLAGS="-D warnings" cargo check --workspace --locked --all-targets` is clean (rustc 1.99).
- `crates/tsrs_cli/tests/emit_gate.rs` passes. `TSRS_EMIT` is still off by default.

## Left
Nothing in E3/E4 itself. What still fails belongs to other waves' stubs (ES downlevel, decorators, jsx, sourcemaps).
