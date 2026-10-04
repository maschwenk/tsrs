# perf-round2-followups: what is left after the 2026-10-04 performance round

One list of the open ideas, decisions and loose ends from the round that landed #37, #40, #38, #41, #42, #49, #50,
#51 and #55. Each stream's own note has the detail and the measurements; this file is the index, ordered by expected
value. Numbers are from the 38k-file codebase on an 18-core Apple Silicon machine unless noted.

Where the round ended: peak memory -15% (4.25 GiB with one checker, 5.68 with four), warm no-edit run ~0.9 s, leaf
edit ~1.1 s, hub edit 12.4 s, cold incremental peak 5.94 GiB, emit 1.4-2.3x faster. The cold check itself is flat:
pointer compression costs about +4.9% instructions and took back most of the checker-CPU round's -6.9%
(notes/perf-round2.md).

## Decisions waiting on the owner

- **Default checker count** (#48, draft; notes/perf-checker-scaling.md). `clamp(cores/2, 4, 8)` gives -16% wall for
  +1.6 GiB on the 38k-file codebase (-34% on vscode) and makes `--extendedDiagnostics` counters machine-dependent.
  Recommendation: close it and pass `--checkers` where the hardware is known (8 on laptops, 4 or 1 on small boxes).
- **#36** (persisted front-end design, docs only): close, or merge as the record of why not to build it.
- **A release.** main is well ahead of 0.3.0; nothing from this round is published.

## Not verified yet

- **The 32 GiB reservation outside macOS and GitHub runners.** Compressed pointers reserve a fixed virtual range
  (notes/mem-pointer-compression.md, "Not done"). It needs a 47-bit user address space and no `ulimit -v` below
  32 GiB, else tsrs aborts with a message. Not yet run inside the sandboxes and dev containers where the monorepo
  uses it. Do this before the next release.
- **x86-64 cost of compression.** The +4-5% instructions were measured on arm64 (one extra `add` per dereference);
  the x86 addressing mode should make it nearly free. Unmeasured.
- **Linux emit.** The 38k-file codebase is bound by file creation on macOS (4 writer permits). On Linux file creation
  runs in parallel, so the writer cap and the remaining transform CPU may both matter (notes/perf-emit.md).
- **CI on main.** The `CI` workflow did not run for the #55 merge commit (654342d) or 48665c7; only the macOS Node
  API workflow did. `ci.yml` no longer has a manual trigger. Find out whether that is a path filter or a gap.
- **`cargo test -p tsrs_cli` does not link on macOS**: `api::memory_tests` calls glibc `malloc_trim`. Every agent
  this round excluded it locally. Needs a `cfg(target_os = "linux")` from the Node API side.

## Ideas, by expected value

1. **The heavy type graphs every checker rebuilds** (notes/perf-checker-scaling.md). At 8 checkers the checkers do
   +84% total work, and 58 files account for 11.2 of the 13.4 s of excess: one project-wide service graph
   (0.3-0.5 s per checker) and the router aggregator (`src/router/index.ts`, 128 imports: 3 ms with one checker,
   1.4-2 s cold). No partition avoids it and sharing types across checkers cannot be exact
   (notes/mem-shared-base.md). The fix is in the checked codebase, not in tsrs: if the router type is one large
   inferred type, an explicit annotation would remove most of it for tsc and tsrs alike. `TSRS_FILE_TIMES` (per-file
   thread CPU) is the tool to find such files in any project.
2. **Emit memory over check-only: 0.53 GiB left** (notes/mem-emit-regions.md, "What remains"): raw source maps kept in
   `EmitResult` as Go does (0.15), the print backlog of up to 2,048 files (0.15; a bound of 256 saved 0.14 GiB but
   cost 8% emit time on vscode), declaration-diagnostics leftovers (0.1).
3. **Emit CPU on the checker thread** (notes/perf-emit.md): `get_local_module_specifier`,
   `get_nearest_ancestor_directory_with_package_json` (a memo, ~0.1 s), `get_accessible_symbol_chain_from_symbol_table`,
   each about 3% of emit. Only visible in wall time once writes are not the bound (Linux).
4. **Signature emit parallelism on hub edits** (notes/perf-dev-loop2.md): about 55% parallel efficiency because each
   dependency level waits for the previous one; module-specifier computation is 4.3 CPU-s of it. The two shortcuts
   around it were rejected (below), so this is what is left for hub edits.
5. **Checker CPU** (notes/perf-checker-cpu2.md, "What remains"): interned names so `SymbolMap::position` hits compare
   pointers instead of name bytes (~5% of check samples; a parser/binder change);
   `instantiate_type_with_alias_worker` cache probes (~3%). Everything else is below 0.3%. What is left at the top of
   the profile is memory latency.
6. **Smaller layouts that handles now allow** (notes/mem-pointer-compression.md, "Not done"): `Type` header 24 -> 20
   bytes (at most ~38 MB), `TypeMapper` below 16 bytes (needs a home for its kind and escape bits).
7. **The "nothing changed" fast path** (notes/perf-dev-loop.md): exact conditions written up; it still has to
   re-resolve every import and would change `--extendedDiagnostics` counters, so the 0.9 s no-edit run is close to
   its floor (program construction 0.6 s, of which `open()` is ~0.3 s).
8. **Parallel `affectedfileshandler` / `emitfileshandler`** (notes/perf-incremental-parallel.md): still sequential in
   the port; costs nothing under `--noEmit`. Worth doing with an emit-on incremental benchmark.

## Measured and rejected (do not redo)

- Hub-edit shortcut, storing file versions as signatures when an edit re-checks most of the program (#64): hub edit
  10.7 -> 7.9 s, global `.d.ts` 18.0 -> 7.2 s, but the first later body-only edit of each skipped file re-checks the
  whole program (1.0 -> 7.6 s), and the stored signatures differ from tsgo's (notes/perf-hub-edit-shortcut.md).
- Deferred signatures, checking first and computing Go's signatures after the check: ~1 s on the hub edit
  (11.1 -> 10.0 s), and 7-11 of ~22k signatures differ from base because printed declarations depend on type
  creation order (tsgo itself is nondeterministic on those files). The global-scope part of it, batching the per-file
  signature emits, is exact and landed (global `.d.ts` edit 18.0 -> 10.5 s).
- Persisted, mmap-able front end: 26% of nodes cacheable, 0.02-0.03 s at 18 threads, at most 0.15 GiB
  (docs/PERSISTED_FRONTEND.md on #36).
- Scope regions for inference contexts: 25-50% of scopes keep a live block (notes/mem-scoped-arenas.md).
- Work stealing between checkers: counters vary between runs. Partition changes: +-4% with no consistent winner
  (notes/perf-checker-scaling.md).
- Sharing types across checkers: cannot be exact (notes/mem-shared-base.md).
- Directory listings instead of existence probes, `openat`, a typed tsbuildinfo decode, skip-if-identical
  tsbuildinfo writes (notes/perf-dev-loop.md, perf-dev-loop2.md).
- Pointer compression below +2% instructions on arm64; `PSlice`/`PStr` for memory (slices are already one word)
  (notes/mem-pointer-compression.md).
- A per-file `type_to_string` builder, lazy `ErrorSymbolName`, dropping source maps from `EmitResult`
  (notes/mem-emit-regions.md).

## Housekeeping

- `bench-cache/solutions/mui-docs` (a local bench checkout) holds ~13k stray emit outputs from a benchmark that used
  `--outDir` without `--rootDir`; mui-docs numbers are off until it is restored. The file list is in the perf-emit
  worktree's `scratch/mui-polluted-files.txt`. Emitting benchmarks must pass `--rootDir <repo root>` and run under a
  write-deny `sandbox-exec` profile (notes/perf-emit.md, "Reproducing").
- Finished agent worktrees under `wt/` hold ~80 GiB of build output and scratch; all their branches are merged.
