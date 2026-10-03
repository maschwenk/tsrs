# emit/incremental: E13 incremental (tsbuildinfo) + E14 `--build` + E15 tsctests harness

Branch `emit/incremental`. Base: main `d3a2598` (emit/core E1+E2; main's later `a6dd429` only updates bench
results). Dependency: `emit/core-2` at `d317313` (merged; provides `compiler.ProgramLike` in tsrs_compiler, which
replaced this wave's local copy). Design decisions: docs/EMIT.md section 10b; numbers: section 13. Everything runs only
under `TSRS_EMIT=1`; without it tsrs still forces `--noEmit`, checks incremental projects from scratch, writes no
tsbuildinfo and rejects `-b` (`crates/tsrs_cli/tests/emit_gate.rs`).

## What is ported

- `crates/tsrs_incremental` (Go `execute/incremental`, every file): snapshot, buildInfo (Go's JSON shape, field order,
  `omitzero`, tuple encodings; read and write), buildinfotosnapshot, snapshottobuildinfo, programtosnapshot,
  referencemap, affectedfileshandler (shape signatures by printing the `.d.ts` with `EmitOnlyBuilderSignature`),
  emitfileshandler, program, incremental, host.
- `performIncrementalCompilation` and the `-b` entry (tsc.go); `tsrs_cli::build` (Go `execute/build`: orchestrator,
  buildtask, uptodatestatus, host, parseCache, compilerHost). Watch mode is not ported. On this checkpoint projects build one at a
  time in build order (Go's `--builders 1` path; same output order); `--builders` concurrency is the follow-up below. `tsc` module: `CommandLineTesting` hooks, writer /
  `WriteFile` / mtime cache in `EmitInput`, aggregate statistics, builder status reporter.
- E15: `crates/tsrs_cli/src/tsctests` (runner.go, sys.go, fs.go, readablebuildinfo.go, the fsbaselineutil differ,
  TracerForBaselining) replaying the scenarios that `tools/oracle/tsctests/dump.sh` records from the Go tests.
- Small shared edits: `ast::new_diagnostic_from_serialized`, `tsoptions::for_each_compiler_option_value`,
  `ParsedOptions.project_references_is_nil`, `WriteFileData.build_info`, `vfstest::from_map_with_clock`,
  `core::version()` overridable by `TSRS_TS_VERSION` at build time (Go's ldflags), `TSRS_LIB_PATH` (noembed-style
  library directory; honoured only under `TSRS_EMIT=1`).

## Reproducing

Reference compiler: `go build -o /root/bin/tsgo-ref ./cmd/tsc` in `ts-ref/tsc` at the pinned commit `b85298b6`
(version `7.1.0-dev`, embedded libraries, so the default tsrs build compares byte for byte).

```sh
cargo build --release -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash
TSRS=target/release/tsrs tools/oracle/incremental/fixtures/run-all.sh      # edit sequences vs tsgo-ref
TSGO=/root/bin/tsgo-ref tools/oracle/emit/monorepo.sh <monorepo> -j 6 --buildinfo -- --noEmit
TSGO=/root/bin/tsgo-ref tools/oracle/emit/monorepo.sh <monorepo> -j 6 --buildinfo -- --emitDeclarationOnly --declarationMap false
PATH=<go>:$PATH tools/oracle/tsctests/dump.sh && cargo test --release -p tsrs_cli tsctests -- --nocapture
```

## Results (head `5b33672`, 2026-10-03)

- Edit sequences (`fixtures/run-all.sh`; output, emitted files and tsbuildinfo compared after every step): `inc1`
  noEmit incremental, 6 steps (cold build, no-op, body edit, shape edit, error fixed, global file deleted); `inc2`
  composite + emitDeclarationOnly, 6 steps (incl. const enum change and a new error); `b1` three-project `-b --verbose`,
  6 steps (cold build, no-op rebuild, touched input without change, body edit, shape edit, no-op) and 9 steps of
  output deletion (a deleted upstream `.d.ts`: like tsgo, the upstream project stays "up to date" and the dependent
  reports TS6305; a deleted `.tsbuildinfo`: rebuilt; a deleted input). All identical to tsgo-ref.
- Monorepo oracle (103 packages whose build runs tsc; outputs under /tmp; the monorepo's `git status` is checked
  empty before and after): `--buildinfo -- --noEmit`: 103/103 packages identical (exit codes, diagnostics), 100/100
  tsbuildinfo byte-identical; `--buildinfo -- --emitDeclarationOnly --declarationMap false`: 103/103 packages,
  2,425/2,425 files (declarations + tsbuildinfo with emit signatures).
- Measured with emit/transforms `3dde949` merged on top (not part of this branch; one ambiguous helper from
  `d317313` reverted locally): monorepo `--buildinfo -- --sourceMap false --declarationMap false`: 83/103 packages
  fully identical (JS, declarations, tsbuildinfo), 4,341 files identical, 0 different; the other 20 stop at gate stubs
  (metadata 8, classfields 4, legacydecorators 3, jsx 3, commonjsmodule 2). tsctests: tsc 146/216, tsbuild 105/190.
- On this branch alone: tsctests tsc 64/216, tsbuild 17/190 (406 non-watch scenarios without content mappers);
  294 stop at other waves' stubs (typeeraser 215, importelision 64, commonjsmodule 12, sourcemap 2, esmodule 1);
  31 fail on output tsrs does not port (`--help`, `--init`, `--showConfig`, `--locale`, `--generateTrace`, the tsrs
  `--version` line). No scenario that gets past the stubs fails.
- xstate-main (1,536 files, `TSRS_EMIT=1 tsrs -p . --incremental`, best of 3; npm tsgo in brackets): no incremental
  0.43 s (1.02); cold incremental 0.51 s (1.08); no change 0.11 s (0.20); body edit of a leaf file 0.14 s (0.25),
  except the first edit after a cold build (0.53 s; tsgo 1.09 s), which rechecks importers once because a cold build
  stores versions as signatures, as in Go; shape edit of `packages/core/src/types.ts` 0.53 s (1.09). Its tsbuildinfo is
  byte-identical to tsgo's, cold and after an edit.
- Gates (base: main `d3a2598` built in a worktree): conformance errors + `--baselines types,symbols` byte-identical in
  the default mode and with `TSRS_LAZY_MEMBERS=0` (13,458 / 12,779 pass); fourslash 4,066 / 63, same pass list;
  `RUSTFLAGS="-D warnings" cargo check --workspace --locked --all-targets` exit 0 (rustc 1.99); `--baselines js` 1,364
  pass, the same pass list as main on this machine (main's reported 1,365 includes
  `intersectionConstructorReductionCrash`, which times out here on both); `tests/emit_gate.rs` 5/5.

## Gaps

- JS emit, source maps and declaration maps come from the other waves; until they merge, projects that emit JS stop
  at their gate stubs (see above for what changes with transforms merged).
- Watch mode (`-w`, `-b -w`) is not ported. `--builders` concurrency: done on `mfs-cx/emit-builders` (below).
- `@incremental` js-baseline variants: done on branch `mfs-cx/emit-incremental-harness` (see below).
- A draft PR could not be opened from the sandbox (`gh pr create` and the REST API answer 403 for the token); this
  note is the PR description.

## Follow-up: `mfs-cx/emit-incremental-harness` (based on `da23dd6`)

- `b1b1ba1`: `--baselines js` runs the emit harness through `compiler.ProgramLike` and wraps `@incremental` variants in
  `incremental.NewProgram` with Go's `testBuildInfoReader` (harnessutil.go `createProgram`); the default mode keeps the
  plain program. All 5 variants (`incrementalConcurrentSafeAliasFollowing`, `incrementalConfig`,
  `incrementalInvalid`, `incrementalTsBuildInfoFile`, `jsEmitIntersectionProperty`) take the incremental path for the
  pre-emit, post-emit, DtsFileErrors and noCheck programs (checked with a temporary trace, not committed).
  `jsEmitIntersectionProperty` passes its `.js` and error baselines that way; the other 4 stop at the importelision
  gate stub (dependency: emit/transforms; before this change they stopped at typeeraser). In the default mode all 5
  pass their error baselines (and types/symbols where they have them).
- Gates vs main `d3a2598`: conformance errors + `--baselines types,symbols` identical (default and
  `TSRS_LAZY_MEMBERS=0`); fourslash 4,066 / 63 same pass list; `--baselines js` 1,364 same pass list (only the
  crash message of `incrementalConcurrentSafeAliasFollowing` changes); `RUSTFLAGS="-D warnings" cargo check
  --workspace --locked --all-targets` exit 0; `tests/emit_gate.rs` 5/5.
- Build concurrency assessment (docs/EMIT.md section 10b, "Build concurrency"): `--builders` is parsed and validated
  like Go (TS5002 for 0 and -1, TS5073 for a non-number; same bytes and exit codes as tsgo built from ts-ref) and then
  ignored; output is identical to tsgo for `--builders 1` and `--builders 8` on a five-project graph because both
  report in `Order()`. Smallest faithful next step: `rangeTasks` with N threads over `ScheduleOrder()`, `done`/`built`
  signals per task, `Mutex`-guarded task state, reporter in `Order()`; measure peak memory first. (Superseded: at the
  pinned commit `rangeTasks` iterates `order`, not `ScheduleOrder()`; see the builders follow-up.)
- The `--baselines js` harness still builds plain programs for the 5 `@incremental` test variants.
- A draft PR could not be opened from the sandbox (`gh pr create` and the REST API answer 403 for the token); this
  note is the PR description.

## Follow-up: `mfs-cx/emit-builders` (based on `da23dd6`)

Review layer `mfs-cx/emit-builders-review` carries the same three commits on top of `mfs-cx/emit-incremental-harness`
(`7eb3e04`): `b5dfe98` -> `7d8a5e4`, `8a790a0` -> `51a44f7`, `535adce` -> this docs commit (EMIT.md and these notes
merged with the harness follow-up).

- `b5dfe98`: build-mode state made genuinely thread-safe (Mutex/atomic task state, `Send + Sync` reporters, the host
  holds the orchestrator by `&'static` so `Sync` is compiler-checked); behaviour unchanged.
- `8a790a0`: Go's `rangeTasks` on `--builders` threads (default 4, 1 with `--singleThreaded`) over `Order()`, task
  `done`/`built` signals (Mutex + Condvar for Go's closed channels), `waitOnUpstream`, reporting in `Order()` on the
  calling thread. Design and evidence: docs/EMIT.md section 10b, "Build concurrency".
- Validation against tsgo built from ts-ref (`go build ./cmd/tsc` at `b85298b6`): `graph` and `cycle` fixtures at
  `--builders 1/4/8`, with and without `--stopBuildOnErrors`, through `tools/oracle/incremental/steps.sh` (cold build,
  no-op, fixing the failing project, no-op): identical output, trees and tsbuildinfo; `--dry`, `--clean --dry`,
  `--clean`, `--force` at 4/8 builders identical; the existing fixtures (`run-all.sh`) unchanged; 20 cold runs per
  builder count, one output each. Concurrency verified with temporary per-task tracing (not committed): at
  `--builders 4` four of the five independent projects of a wide graph ran simultaneously, at 8 all five.
  Peak RSS (`graph`, best of 3): tsrs 291 / 313 / 309 MB, tsgo 122 / 175 / 199 MB at 1 / 4 / 8 builders.
- `TSRS_CHECK_SHARED=1` debug runs at `--builders 1/4/8` report no write to a shared object. tsctests unchanged
  (tsc 64/216, tsbuild 17/190, same pass list). Default mode unchanged: `-b` without `TSRS_EMIT=1` still writes nothing
  (`tests/emit_gate.rs` 5/5).

## Clean scheduler layer `mfs-cx/emit-builders-stack` (on `mfs-cx/emit-incremental-harness` `7eb3e04`)

Head `e92f7b5`: `7d8a5e4` (thread-safe build state), `51a44f7` (`--builders` scheduler), `e92f7b5` (docs). The code
delta over `7eb3e04` is byte-identical to `535adce` over `da23dd6` (`git diff` of `crates/` and `tools/`); `535adce`
itself is unchanged.

Gates on `e92f7b5` (base: main `d3a2598`):
- conformance errors + `--baselines types,symbols` identical in all four modes (lazy members on / `TSRS_LAZY_MEMBERS=0`
  x single- / multi-threaded test programs): 13,458 pass, 12,779 types/symbols pass;
- fourslash 4,066 / 63, same pass list; `--baselines js` 1,364, same pass list as `d3a2598`;
- `RUSTFLAGS="-D warnings" cargo check --workspace --locked --all-targets` exit 0; `tests/emit_gate.rs` 5/5;
- `fixtures/run-all.sh` identical to tsgo-ref; `graph` and `cycle` at `--builders 1/4/8`, with and without
  `--stopBuildOnErrors`: identical output, trees and tsbuildinfo.

Trial merge with main `0251527` (source maps; local commit, not pushed): the only conflict is the EMIT.md section 13
table (both rows kept). `tools/oracle/emit/monorepo.sh` and `run.py` merge automatically: main's fail-closed result
checks (`summarize.py` verdicts, missing rows) and this branch's `--buildinfo` coexist (`run.py --buildinfo` adds the
tsbuildinfo to the same JSON row the summarizer judges). On the merged tree: four-mode errors/types/symbols identical
to main `0251527`; `--baselines js` 1,364, same pass list as `0251527`; fourslash 4,066 / 63; emit_gate 5/5;
`run-all.sh` identical.

Bounded larger public graph (`xsgraph`: a solution with one project per xstate-main package, 14 projects plus the
root, `include` pointing at the read-only checkout, `incremental` + `noEmit`, `types: []`; built under /tmp), measured
on `e92f7b5` before the reviewer's checker-thread fix (F1, `52be3a5`):
- tsgo-ref and tsrs at `--builders 1/4/8`: identical `-b --verbose` output (568 lines, same errors) and identical
  tsbuildinfo files in all six runs.
- Temporary start/end tracing (not committed): at most 1 / 4 / 8 projects building simultaneously at 1 / 4 / 8
  builders; span 1.32 / 0.92 / 0.91 s.
- Wall (best of 3) and peak RSS (max of 3): tsrs 1.14 s / 1,119 MB, 0.96 s / 1,161 MB, 0.95 s / 1,175 MB; tsgo 2.10 s /
  531 MB, 1.10 s / 1,129 MB, 1.16 s / 1,857 MB. `-b core` alone (the largest project, 137 files): tsrs 216 MB, tsgo
  183 MB.

Program retention: Go's `buildOrCleanProject` sets `task.result.program = nil` when not testing and the GC reclaims
the program (the host's cached `.d.ts`/JSON files stay). tsrs drops its `P<incremental::Program>` handle at the same
point, but the memory stays: `free_program` (tsrs_compiler) only frees the `Program` struct and its resolution host,
not the arena memory (AST, checker types) that makes up almost all of a program; that needs the LSP's region
machinery (tsrs_project memregions). So tsrs's peak at `--builders 1` grows with the sum of all projects (1,119 MB
for the graph vs 216 MB for its largest project) where Go's tracks the projects in flight (531 MB). No change is
made here: calling `free_program` would not reclaim the arenas, and the reviewer did not reproduce a retention
regression after F1 at the sizes they tested.
