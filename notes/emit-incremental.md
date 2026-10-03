# emit/incremental: E13 incremental (tsbuildinfo) + E14 `--build` + E15 tsctests harness

Branch `emit/incremental` (merged with main at d3a2598, emit/core E1+E2). A draft PR could not be opened with the
available token (`gh pr create` and the REST endpoint both answer 403 "Resource not accessible by integration"), so this
note is the PR description. Design decisions: docs/EMIT.md section 10b; numbers: section 13.

## What is ported

- `crates/tsrs_incremental` (Go `execute/incremental`, every file): snapshot, buildInfo (Go's JSON shape, field order,
  `omitzero`, tuple encodings; read and write), buildinfotosnapshot, snapshottobuildinfo, programtosnapshot,
  referencemap, affectedfileshandler (shape signatures by printing the `.d.ts` with `EmitOnlyBuilderSignature`),
  emitfileshandler, program, incremental, host. `ProgramLike` + `HandleNoEmitOptions`/`GetDiagnosticsOfAnyProgram`
  over it are in `tsrs_incremental::emit` (TODO(emit/core): move to tsrs_compiler).
- `performIncrementalCompilation` and the `-b` entry (tsc.go), `tsrs_cli::build` (Go `execute/build`: orchestrator,
  buildtask, uptodatestatus, host, parseCache, compilerHost; watch mode not ported; tasks run one at a time in build
  order). `tsc` module: `CommandLineTesting` hooks, writer / `WriteFile` / mtime cache in `EmitInput`, aggregate
  statistics, builder status reporter, `diagnosticwriter` status formatters.
- E15 harness: `crates/tsrs_cli/src/tsctests` (runner.go, sys.go, fs.go, readablebuildinfo.go, fsbaselineutil
  differ, TracerForBaselining) replaying scenarios dumped from the Go tests by `tools/oracle/tsctests/dump.sh`.
- Shared edits (small): `ast::new_diagnostic_from_serialized`; `tsoptions::for_each_compiler_option_value`;
  `ParsedOptions.project_references_is_nil` (Go's nil `ProjectReferences`); `WriteFileData.build_info`;
  `vfstest::from_map_with_clock`; `core::version()` overridable by `TSRS_TS_VERSION` at build time (Go's ldflags);
  `TSRS_LIB_PATH` (noembed-style lib directory, honoured only under `TSRS_EMIT=1`).
- Oracles: `tools/oracle/emit/run.py --buildinfo` and `monorepo.sh --buildinfo` (also compare the tsbuildinfo),
  `tools/oracle/incremental/steps.sh` (scripted edit sequences against tsgo, plain or `-b`).

## Numbers (2026-10-03)

- Monorepo oracle (tsgo 7.1.0-dev.20260929.1; tsrs built with `TSRS_TS_VERSION=7.1.0-dev.20260929.1`; outputs under
  /tmp only; monorepo `git status` empty before and after):
  - `monorepo.sh /root/Owner --buildinfo -- --noEmit` (the Olympus-style incremental check): 103/103 packages fully
    identical (exit code, diagnostics), 100/100 tsbuildinfo byte-identical.
  - `monorepo.sh /root/Owner --buildinfo -- --emitDeclarationOnly --declarationMap false`: 103/103 packages, 2,425/2,425
    files identical (declarations + tsbuildinfo with emit signatures).
  - Default (JS) mode is blocked on the transformer waves (typeeraser/importelision stubs), as on main.
- Scripted edit sequences vs tsgo (output, emitted files and tsbuildinfo after every step identical): noEmit
  incremental (6 steps incl. shape change and file delete), composite + emitDeclarationOnly (6 steps incl. const enum
  change and new error), 3-project `-b --verbose` graph (build, no-op, touch, body edit, shape edit, no-op).
- xstate-main (1,536 files, `TSRS_EMIT=1 tsrs -p . --incremental`, best of 3; tsgo in brackets): full check without
  incremental 0.43 s (1.02); cold incremental 0.51 s (1.08); no change 0.11 s (0.20); edit of a leaf file's body
  0.14 s (0.25) after the first edit, which rechecks importers once because a cold build stores versions as
  signatures (0.53 s; tsgo 1.09 s, same behaviour); edit of `packages/core/src/types.ts` changing its shape 0.53 s
  (1.09). xstate's tsbuildinfo is byte-identical to tsgo's, cold and after an edit.
- tsctests (`cargo test --release -p tsrs_cli tsctests -- --nocapture`, 406 non-watch, non-content-mapper scenarios):
  tsc 64/216, tsbuild 17/190 pass; 294 stop at other waves' gate stubs (typeeraser 215, importelision 64,
  commonjsmodule 12, sourcemap 2, esmodule 1); 31 fail on output tsrs does not port (`--help`, `--init`,
  `--showConfig`, `--locale`, `--generateTrace`, the tsrs `--version` line).
- Gates: conformance errors + `--baselines types,symbols` byte-identical to main (default and `TSRS_LAZY_MEMBERS=0`);
  fourslash 4,066 / 63 with main's pass list; `RUSTFLAGS="-D warnings" cargo check --workspace --locked --all-targets`
  clean (rustc 1.99); `--baselines js` 1,364 pass, same pass list as main built on this box (main's 1,365 includes
  `intersectionConstructorReductionCrash`, which times out here on both); `tests/emit_gate.rs` 5/5 (two new checks:
  `-b` writes nothing without the gate; tsbuildinfo written and reused with it).

## Left

- JS emit, source maps and declaration maps come from the other waves; the tsctests crashes and the default
  monorepo oracle mode move with them. Re-run `cargo test --release -p tsrs_cli tsctests` after each merge.
- Wire `incremental.NewProgram` into the `--baselines js` harness for the 5 `@incremental` variants.
- Build mode runs one project at a time (Go: `--builders`, default 4); watch mode (`-w`, `-b -w`) is not ported.
- `ProgramLike` should move into tsrs_compiler (coordinate with emit/core).
