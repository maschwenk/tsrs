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
