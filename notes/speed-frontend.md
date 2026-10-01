# speed-frontend: wall time outside checking, checker imbalance

Goal: cut Project wall time outside the checkers (program construction, config, diagnostics plumbing, process
overhead) and look at checker imbalance, without changing checker semantics. Every change: suite identical to
main in default, opt-out (`TSRS_LAZY_MEMBERS=0 --baselines types,symbols`) and parallel-program
(`TS_TEST_PROGRAM_SINGLE_THREADED=false`) modes; Project counters, `--listFiles`, `--explainFiles` and
`--traceResolution` output identical; diagnostics on a synthetic 60-file error project identical.

## Instrumentation

`--extendedDiagnostics` prints tsrs-only sub-phases after Go's table (`tsrs_core::phases`, always recorded, one
mutex push per coarse phase): config include glob / listing prefetch, root file lookups, loader rounds,
parallel parse + resolve, sequential load, file graph, collect files, verify options, checker creation, file
assignment, syntactic / program / global diagnostics, sort, report, list files, error summary, statistics.
"Parse time" is Go's: all of program construction.

`TSRS_FILE_TIMES=<path>` (off by default) writes per-file check seconds, checker, node count, text length,
imports and the node-kind histogram, for cost-model fitting. `TSRS_ASSIGNMENT_STATS=times` (existing) prints the
per-checker group times.

Profiling: `sample` works, `samply`/`cargo flamegraph` are not installed. Attaching takes 0.3-2 s on a loaded
machine, so profiling the start of a run needs a delay before `main` (a local `TSRS_PAUSE_MS` sleep, not
committed) and a uniquely named copy of the binary in `target/release/` (`pgrep -x`; other agents run `tsrs`
too; the bundled libs are found relative to the executable).

## Where the time went (Project, 4 checkers, medians of 5, load average 8-20)

| phase | before (e8d4196) | after |
| --- | --- | --- |
| config (include glob) | 0.170 (0.157) | 0.114 (0.101) |
| root file lookups | 0.109 | 0.030 |
| file graph: parse + metadata + resolution | 1.298 (parallel parse 0.448, metadata 0.124, sequential load 0.682 of which module resolution 0.606) | 0.660 (parallel 0.544, sequential 0.083) |
| collect files | 0.105 | 0.053 |
| verify options (common source dir) | 0.039 | 0.039 |
| "Parse time" total | 1.559 | 0.795 |
| bind | 0.042 | 0.039 |
| checker creation + assignment | 0.076 | 0.031 |
| check | 7.56 | 7.51 |
| statistics (`--extendedDiagnostics` only) | ~0.14 | 0.015 |
| process start / exit | ~0.005 / ~0.1 | same |
| wall | 9.72 | 8.63 |
| peak | 11.72 GB | 10.55 GB (other agents' layout work; these changes leave peak unchanged) |

Outside checking: 1.90 s -> 1.00 s. Diagnostics collection, sorting and printing are ~0 on a clean project.

## Changes (each landed separately, numbers in the commit messages)

1. Sub-phase instrumentation (above).
2. File loader: the parallel prefetch of each round now also computes metadata and resolves imports, string
   module augmentations, type reference directives and triple-slash references (Go does all of a task's load in
   parallel); the sequential load consumes the results in the same order (first module-resolution error is
   still the first in load order). Resolution results do not depend on order (resolver caches keyed by name,
   directory, mode, redirect); with `--traceResolution` resolutions stay sequential because traces report cache
   hits. Root file lookups run in parallel. Also: the casing check in getProcessedFiles skips identical names,
   `normalize_path` copies once, `to_path` lowercases in place. Parse time 2.65 -> 0.98 s on a loaded machine.
3. vfs: the include glob reads the directory listings of every directory it will enter in parallel first
   (rayon scope); the walk, its order and its symlink-cycle check are unchanged. Config 0.155 -> 0.117 s.
4. Statistics: line count (builds every line map) on the worker pool: 0.14 -> 0.015 s.
5. Checker assignment: per-file import targets computed on the worker pool, adjacency built in file order as
   before: assign 0.070 -> 0.027 s (go assignment 0.052 -> 0.008); assignment unchanged.
6. `TSRS_FILE_TIMES` dump (experiments).
7. Prefetched resolutions carry the normalized resolved file name and its path, so the sequential load does not
   normalize twice per import edge: sequential load 0.13 -> 0.083 s.

## What remains in the frontend

- The parallel parse + resolve phase (~0.54 s, 14 rounds, round 1 = 27.5k root files ~0.33 s) is bound by
  kernel file-system calls: 114k `stat`s (module resolution probes, root lookups), 40k file reads, 5.3k
  `readdir`s; it stops scaling past ~8 threads. The worker profile shows mutex waits on the resolver's
  module-resolution cache (`SyncMap` is one `Mutex`), but a 64-shard map changed nothing measurable
  (0.51 s either way), so it was not landed. Fewer syscalls would need answering `file_exists` from cached
  directory listings, which is not exactly equivalent on a case-insensitive file system.
- Config glob walk/matching after the listing prefetch: ~0.045 s. Verify options: 0.04 s (sequential common
  source directory check). Bind: 0.04 s (parallel). Process exit (unmapping ~11 GB): ~0.1 s.

## Checker imbalance

Per-checker check times with the locality assignment are consistently uneven: over 5 runs the slowest checker
(always checker 2) is 10.5-10.7% above the mean (e.g. 6.00 / 6.85 / 7.39 / 6.47 s), the fastest 10% below; a
perfect balance would save ~0.7 s of wall.

- Per-file check time is hardly predictable from syntax (correlation 0.17 with the weight, best single node kind
  0.21): it is dominated by the first touch of shared declarations in that checker (a 267-node endpoint costs
  0.47 s, `router/index.ts` 0.59 s, a 32-file directory 0.95 s). Over the 294 directory groups the weight
  explains more (correlation 0.81, R² 0.65).
- A fitted model (named import specifiers + 6x dynamic imports + nodes) reproduced the measured per-checker
  shares in-sample within 0.5 points but made the assignment worse when used (slowest checker 21.5% above the
  mean, check 8.2 -> 9.5 s): changing the assignment moves the first-touch costs. Not landed.
- 3 / 4 / 5 checkers (locality): wall 11.8 / 10.3 / 8.8 s, peak 10.70 / 11.32 / 12.01 GB, slowest checker
  4% / 10% / 16% above the mean. Neither alternative is no-worse on both axes; the default stays 4.
- Ideas: a cost model from a previous run's per-group times (tsrs writes nothing today), or sharing library
  instantiations between checkers so that hub costs are paid once.

## Process level

- Startup: `tsrs --version` 5 ms; main starts within a few ms of exec.
- mimalloc options (env, 4-6 interleaved runs each): `PURGE_DELAY=-1`, `ARENA_EAGER_COMMIT=1`,
  `ARENA_RESERVE=4GiB`, `ALLOW_LARGE_OS_PAGES=1`: all within noise (run-to-run spread 8.2-10.8 s), peak within
  0.05 GB. No change.
- Thread stacks (512 MB main/checkers, 256 MB rayon workers) are reservations only; nothing to gain.
- `--extendedDiagnostics` overhead: 0.14 s before (statistics' line maps), 0.015 s after.
- Fat LTO + codegen-units=1: -2.4% instructions, wall within noise; not worth the build time for every agent.
