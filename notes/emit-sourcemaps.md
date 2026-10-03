# emit/sourcemaps (wave E7: source maps + declaration maps)

Branch `emit/sourcemaps`. Plan: docs/EMIT.md section 7 (E7).

## Done

- **`tsrs_sourcemap` crate** (Go `internal/sourcemap`): generator, decoder, lineinfo, util, source, source_mapper.
  The `lsp` work had already ported the whole package as `tsrs_ls::sourcemap` (EMIT.md section 11: "adopt theirs");
  it was moved verbatim into the new crate and `tsrs_ls` re-exports it (`pub use tsrs_sourcemap as sourcemap;`), so
  every `tsrs_ls::sourcemap::…`/`crate::sourcemap::…` path keeps compiling. Tests: all 32 Go tests of
  generator_test.go plus 7 source_mapper tests (`cargo test -p tsrs_sourcemap`).
  - Dependencies follow Go's imports: `tsrs_core`, `tsrs_scanner` (source_mapper uses
    `ComputePositionOfLineAndUTF16Character`) and `tsrs_ast` (for the `Source` impl below).
  - Go's `*ast.SourceFile` satisfies `sourcemap.Source` implicitly; Rust needs an impl, and the orphan rule puts it in
    `tsrs_sourcemap/src/source.rs` (`impl Source for tsrs_ast::SourceFile`). `tsrs_ast` does not depend on the new crate.
- **Printer source-map paths** (`tsrs_printer`): `SourceMapGenerator` is now `tsrs_sourcemap::Generator`;
  `setSourceMapSource`, `emitPos`, `emitSourcePos` ported (printer_3.rs), `lineCharacterCache` (utilities.rs),
  `PrintHandlers.map_source_position` (Go `MapSourcePosition`, `(Source, pos, ok)` -> `Option<(Source, pos)>`), and
  all Printer fields of Go (`sourceMapGenerator`, `sourceMapSourceIndex`, `sourceMapSourceIsJson`,
  `sourceMapLineCharCache`, `mostRecentSourceMapSource[Index]`) with Go's save/restore in `Write`.
  - Sources are `SourceMapSource = &'static dyn tsrs_sourcemap::Source` (arena `SourceFile` or a leaked
    `declarationMapSource`), compared by address like Go interface values holding pointers.
  - The generator is borrowed for the duration of `write` through a raw pointer (same pattern as the existing
    `borrowedWriter`; one `unsafe` with a SAFETY comment).
  - The line/character cache is held by value; Go shares it through a pointer, but every save/restore in Go restores
    the same object that was saved, so moving it out and back is equivalent (and the cache only affects speed).
  - Go quirk kept: `emitSourcePos` saves/restores source, index and cache but not `sourceMapSourceIsJson`.
- **Emitter glue** (`tsrs_compiler/src/emitter.rs`): `declarationMapSource`/`newDeclarationMapSource`,
  `shouldEmitSourceMaps`, `getSourceRoot`, `getSourceMapDirectory`, `getSourceMappingURL` (the last two take the
  `OutputPathsHost` view of the emit host: Go reads only the common source directory, cwd and case sensitivity).
  `outputpaths` was already ported completely in `tsrs_tsoptions::outputpaths` (checked against Go).
- **Harness**: `tsrs_testrunner/src/sourcemap_recorder.rs` (harnessutil/sourcemap_recorder.go +
  `CompilationResult.GetSourceMapRecord`), `sourcemap_baseline.rs` (`DoSourcemapBaseline`,
  `createSourceMapPreviewLink` with the literal `url.QueryEscape`/`QueryUnescape` round trip, `DoSourcemapRecordBaseline`).
  They take the parts of `CompilationResult` they read as a struct of slices/closures until the emit harness exists.
- **Oracles**:
  - `crates/tsrs_printer/examples/sourcemap_oracle.rs`: printer-level check (print an untransformed `.js` with a
    generator). On 60 `.mjs` files from node_modules vs `tsgo --allowJs --module preserve --sourceMap`: every file whose
    JS is identical (38; the others differ only because no transformer runs here) has an identical `.js.map`.
  - `tools/oracle/emit/monorepo.sh`: the monorepo emit oracle (all packages whose `build` runs `tsc`; outputs redirected
    to /tmp; repo status checked before/after).

- **Wired on top of E1/E2** (main d3a2598): `printSourceFile`'s source-map branch (generator, `SourceMaps` in the emit
  result, `//# sourceMappingURL=`, the `.map` write), `getSourceMappingURL`'s inline `Base64DataURL`,
  `SourceMapEmitResult.source_map`; `emitDeclarationFile` already passes the declaration-map options. The duplicate
  free functions this branch had in emitter.rs were dropped in favor of E1's methods.
- **Harness**: `tsrs-test run --baselines jsmap,sourcemap` (kinds `js.map`, `sourcemap.txt`; result lists
  `js.map-<class>.txt`, `sourcemap.txt-<class>.txt`; `show <name> --jsmap --sourcemap`). They use E2's emitting
  compilation. A test whose generated baseline is `NoContent` and that has no reference file passes trivially in Go;
  here it is reported as `skip` so the pass counts are meaningful. `EmitOutputs` gained Go's `inputs`/`outputs`
  lists (needed by `createSourceMapPreviewLink`).

## Numbers (2026-10-03, on main d3a2598)

- `--baselines js`: 1363 pass, same pass list as main (the 2 timeouts are the usual flaky ones).
- `--baselines jsmap,sourcemap`: 0 pass, 12030 crash, 3165 skip: every test with source maps panics in the E3 stubs
  (typeeraser) before printing. Needs emit/transforms.
- Monorepo oracle (`tools/oracle/emit/monorepo.sh /root/Owner -- --emitDeclarationOnly`, reference = tsgo built from
  ts-ref): 103/103 packages fully identical, 4623/4623 files, including 2298 `.d.ts.map`.
- Reference binary: the npm nightly 7.1.0-dev.20260929.1 predates the pinned commit and lacks #64460 (declaration
  maps of `export default <identifier>`); with it 187 `.d.ts.map` differ. Use a tsgo built from ts-ref.

## Local integration (unpushed `int-local` = this branch + origin/emit/transforms 3dde949), 2026-10-03

- `--baselines js`: 7322 pass, 1 fail, 6071 crash (other waves' stubs).
- `.js.map`: 86 pass, **0 fail**, 63 crash; `.sourcemap.txt`: 93 pass, **0 fail**, 63 crash. Of the 151 / 158 reference
  baselines, every non-passing one crashes in another wave's stub (forawait 36, classfields 15, commonjsmodule 6,
  jsx 3, esdecorator 2, legacydecorators 1) or is a content-mapper test (2, unsupported).
- Monorepo oracle, full mode (configs' own sourceMap/declarationMap): 83/103 packages fully identical, 8996 files
  identical, **0 different**, 1252 not emitted (the 20 other packages panic in metadata 8, classfields 4,
  legacydecorators 3, commonjsmodule 3, jsx 2). Every emitted map is identical: 2317 `.js.map`, 2166 `.d.ts.map`.

## Left

- `.js.map` / `.sourcemap.txt` baseline counts and the full monorepo oracle with maps, once the E3/E4 transformers
  land (nothing in this wave is known to be missing for them).

## Checkpoint for review (2026-10-03) — draft PR body

PR creation is blocked: `gh pr create` and `POST /repos/maschwenk/tsrs/pulls` with `PAT_FOR_OSS` both return
`403 Resource not accessible by integration` (the token has repo push/admin but not pull-request write). Intended
draft PR: head `emit/sourcemaps`, base `main`, title "emit/sourcemaps: E7 source maps + declaration maps", label
`coder-task-generated`; body = this section.

### Scope (this branch only)
`tsrs_sourcemap` crate, printer source-map paths, emitter `printSourceFile` source-map branch / inline URL /
`SourceMapEmitResult.source_map`, harness `--baselines jsmap,sourcemap`. No transformer code.

### Gates (base = main d3a2598, head = emit/sourcemaps bb2a6f1; same box, run sequentially by /tmp/gate2.sh)
- Conformance errors + `--baselines types,symbols`, default and `TSRS_LAZY_MEMBERS=0`: `diff -rq` of the whole
  `target/test-results` trees: 0 differing files (summary.json excluded: timings). Both: 13457 pass / 2 codes / 2 fail
  / 1 timeout; `.types`/`.symbols` 12778.
- `--baselines js`: base 1363 pass, head 1363 pass, identical `js-pass.txt`; 2 timeouts on both sides.
- The 2 timeouts (`crossProductUnionIntersectionRepeatedPropDeclarationsNoOOM1`, `intersectionConstructorReductionCrash`)
  rechecked one at a time with `--jobs 1 --timeout 900 --baselines js,types,symbols`: pass errors/js/types/symbols on
  both base and head (22 s / 41 s base, 21 s / 44 s head). So the js count is 1365 on both = main's 1365; the
  timeouts are load-only, not regressions.
- Fourslash: 4066 pass / 63 fail on both, identical pass list (exit 1 on both because of the 63 known failures).
- `RUSTFLAGS="-D warnings" cargo check --workspace --locked --all-targets`: exit 0 (rustc 1.99.0).
- `cargo test -p tsrs_cli --test emit_gate`: 3/3 pass. `cargo test -p tsrs_sourcemap`: 39 pass.

### Monorepo oracle (standalone, this branch)
Reference: tsgo built from pinned ts-ref (`cd ts-ref/tsc && go build -o /root/bin/tsgo-ref ./cmd/tsc`, Go 1.27.1).
The npm nightly 7.1.0-dev.20260929.1 predates b85298b6 (lacks #64460); with it 187 `.d.ts.map` differ.
```
TSGO=/root/bin/tsgo-ref tools/oracle/emit/monorepo.sh /root/Owner -j 6 -- --emitDeclarationOnly
```
103/103 packages fully identical; 4623/4623 files, of which 2298 `.d.ts.map`. `/root/Owner` git status empty
before and after.

### Integration evidence (temporary local branch `int-local`, not pushed)
`int-local` f315c3a = merge of `emit/sourcemaps` 9d8bf1f (same code as bb2a6f1; later commits are docs only) and
`emit/transforms` 3dde949.
- `tsrs-test run --suite all --baselines js,jsmap,sourcemap`: js 7322 pass / 1 fail / 6071 crash; `.js.map` 86 pass,
  0 fail, 63 crash; `.sourcemap.txt` 93 pass, 0 fail, 63 crash. All 63 crashes are other waves' stubs (forawait 36,
  classfields 15, commonjsmodule 6, jsx 3, esdecorator 2, legacydecorators 1); 2 more reference baselines are
  content-mapper tests (unsupported).
- `TSGO=/root/bin/tsgo-ref tools/oracle/emit/monorepo.sh /root/Owner -j 6` (configs' own sourceMap/declarationMap):
  83/103 packages fully identical, 8996 files identical, 0 different, 1252 not emitted (20 packages panic in metadata
  8, classfields 4, legacydecorators 3, commonjsmodule 3, jsx 2). Every emitted map identical: 2317 `.js.map`,
  2166 `.d.ts.map`.

### Dependencies / left
- JS source maps for the remaining baselines and packages need E3/E4 (commonjsmodule, metadata, legacydecorators) and
  E5/E6/E9/E10 (classfields, esdecorator, forawait, jsx). Nothing source-map-specific is known to be missing.
