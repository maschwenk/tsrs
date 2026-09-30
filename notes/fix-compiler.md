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
