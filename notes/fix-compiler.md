# fix-compiler notes

## Emit output-path verification (TS5055 / TS5056)

- Go `verifyCompilerOptions` ends with an output-path check (`outputpaths.ForEachEmittedFile` over
  `getSourceFilesToEmit`): any emit path equal to an input file -> TS5055 (+ tsconfig hint chain when there is no
  config file), duplicate emit paths -> TS5056. It was skipped in the port as "emit"; it is a program diagnostic, so
  it is now ported (`OutputPaths`, `get_output_paths_for`, `for_each_emitted_file` in `tsrs_tsoptions::outputpaths`,
  `Program: OutputPathsHost`, `emitter::get_source_files_to_emit`).
- The Go harness does *not* suppress other diagnostics when these exist: baselines contain both TS5055 and the JS
  `TS8xxx`/semantic errors. The previous failures were only the missing TS5055/TS5056 lines; this also fixed the
  `TS6054`/`TS5096`/`nodeNextPackageSelfName...` entries, whose first diff line was just the missing TS5055/5056.

## intersectionConstructorReductionCrash timeout

- Not an infinite loop: Go (`tsgo-ref`) itself needs ~10 s on this test. The time is the exponential walk in
  `hasBaseType` (called from `resolveBaseTypesOfClass` circularity check) over the diamond-shaped mixin hierarchy.
- The Rust `get_base_types` returned `Vec` (`resolved_base_types.get().to_vec()`), allocating on each of the millions of
  calls: 25 s. Returning the stored `&'static [P<Type>]` (as PORTING.md prescribes for cached slices) -> 8 s, and the
  full suite went from ~22 s to ~11 s wall.
- Grep pattern: functions returning `Vec` built from a stored `Cell<&'static [T]>` via `.to_vec()`.

## Remaining non-declaration fails: none are checker bugs

Verified against Go with an instrumented reference built via `go build -overlay` (a patched copy of `checker.go` that
prints `runtime/debug.Stack()` at the error site, built from `cmd/tsrs-oracle-testrunner`; no edits to `ts-ref`).
Recipe: copy `internal/checker/checker.go` to scratch, patch, write an overlay JSON mapping *both* the symlinked and the
real path of the file to the copy, then `GOTOOLCHAIN=auto go build -overlay ... ./cmd/tsrs-oracle-testrunner` and run
`<bin> diags <test-name>`.

- Harness emit side effect (`mutuallyRecursiveInference` TS5114 position, `recursiveMappedTypes` TS2615 position): the Go
  harness calls `postProgram.Emit()` *before* collecting the post-emit diagnostics it baselines. The const-enum inliner
  queries the checker on `this.a` / `x.type` first, so the depth-limit/circularity error lands on that node. The
  pre-emit program (and `tsgo` CLI) report exactly what tsrs reports. Not fixable without emulating JS emit's
  checker queries.
- Placeholder printer: `noTypeToStringRecursion`/`noTypeToStringStackOverflow` (Go's TS7023 is triggered by
  `typeToString('() => any')` resolving the return type while it is being resolved), `incompatibleAssignmentOf...`
  (TS2719 vs TS2322 picked by comparing printed names), `objectTypeWithStringAndNumberIndexSignatureToAny`
  (`chainArgsMatch` compares printed type strings), `types.asyncGenerators.es2018.2` (duplicate TS2504 not deduped
  because the two types print differently), `privateNameInTypeQuery` and the `'"each"'` quoting in
  `recursiveMappedTypes` (`symbolToString`).
- Declaration diagnostics: `declarationEmitExpandoPropertyPrivateName` (TS4032), `declarationFiles` (TS2527).

## Declaration diagnostics (not ported; estimate)

59 failing tests (TS2883, TS4xxx, TS9xxx, TS5088, TS7056, TS7080, TS6424, TS2527). Go entry points:
- `compiler.Program.GetDeclarationDiagnostics` -> `getDeclarationDiagnosticsForFile` (program.go:1626) ->
  `emitter.go getDeclarationDiagnostics` -> `declarations.NewDeclarationTransformer(host, nil, options, "", "")
  .TransformSourceFile(file)` -> `GetDiagnostics()`. The harness only calls it when `GetEmitDeclarations()`.
- `transformers/declarations`: transform.go 3.0k, diagnostics.go 0.7k (TS4xxx message selection), tracker.go 0.26k
  (SymbolTracker: TS2883 `reportInferenceFallback`/cannot-be-named, TS2527 inaccessible this/unique symbol, TS7056
  truncation, TS5088 cyclic), util.go 0.2k, supplementalreferences.go; plus `transformers/transformer.go`/`utilities.go`
  (~0.4k).
- `checker/emitresolver.go` (1.3k; symbol accessibility -> TS4xxx, `IsDeclarationVisible`, expando/late-bound checks) and
  `pseudochecker/` (1.1k; isolatedDeclarations inference -> TS9xxx).
- Depends on the real node builder (`nodebuilderimpl.go` + tracker callbacks) now in progress elsewhere, plus the
  already-ported `tsrs_printer` factory/emit context and `tsrs_ast` visitor.
- Size: ~7k Go lines beyond the node builder (-> ~8-9k Rust), roughly 2-3 agent-days, best started after the node
  builder lands.
