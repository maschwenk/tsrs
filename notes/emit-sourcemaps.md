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

## Not done yet / waiting on emit/core

- Wiring: `printSourceFile` (generator creation, `SourceMaps` in the emit result, `//# sourceMappingURL=`, writing
  the `.map`), `emitDeclarationFile`'s declaration-map printer options and `MapSourcePosition` (content mappers are
  not ported, so the span map is always nil and the handler is never installed, as in Go without a mapper).
- `--baselines jsmap,sourcemap` in the test runner on top of the E2 `CompilationResult`.
