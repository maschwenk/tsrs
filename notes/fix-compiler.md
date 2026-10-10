# fix-compiler notes

Status (2026-10-10): condensed. The sections "Placeholder printer" and "Declaration diagnostics (not ported;
estimate)" were removed: the node builder is real and the declaration transformer and emit resolver are ported
(crates/tsrs_declarations, crates/tsrs_checker/src/emitresolver.rs). Conformance results: docs/STATUS.md.

## Emit output-path verification (TS5055 / TS5056)

Go `verifyCompilerOptions` ends with an output-path check (`outputpaths.ForEachEmittedFile` over
`getSourceFilesToEmit`): an emit path equal to an input file gives TS5055 (+ tsconfig hint chain when there is no
config file), duplicate emit paths give TS5056. It is a program diagnostic, so it is ported (`tsrs_tsoptions::outputpaths`,
`emitter::get_source_files_to_emit`). The Go harness does not suppress other diagnostics when these exist: baselines
contain both TS5055 and the JS `TS8xxx`/semantic errors.

## intersectionConstructorReductionCrash timeout: `.to_vec()` of a cached slice

- Not an infinite loop: Go (`tsgo-ref`) itself needs ~10 s on this test. The time is the exponential walk in
  `hasBaseType` (called from `resolveBaseTypesOfClass` circularity check) over the diamond-shaped mixin hierarchy.
- The Rust `get_base_types` returned `Vec` (`resolved_base_types.get().to_vec()`), allocating on each of the millions of
  calls: 25 s. Returning the stored `&'static [P<Type>]` (as PORTING.md prescribes for cached slices) -> 8 s, and the
  full suite went from ~22 s to ~11 s wall. (`get_base_types` still returns `&'static [P<Type>]`, checker_09.rs.)
- Grep pattern: functions returning `Vec` built from a stored `Cell<&'static [T]>` via `.to_vec()`.

## Instrumented Go reference with `go build -overlay`

The recipe is in docs/DEBUGGING.md, "Where Go reports an error".

This is how the harness-emit side effect was confirmed (`mutuallyRecursiveInference` TS5114 position,
`recursiveMappedTypes` TS2615 position): the Go harness calls `postProgram.Emit()` before collecting the post-emit
diagnostics it baselines, and the const-enum inliner queries the checker first, so the error lands on another node.
The `tsgo` CLI reports what tsrs reports (docs/STATUS.md).
