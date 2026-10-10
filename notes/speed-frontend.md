# speed-frontend: wall time outside checking, checker imbalance

Goal: cut the private monorepo's wall time outside the checkers (program construction, config, diagnostics plumbing,
process overhead) and look at checker imbalance, without changing checker semantics. Every change kept the suite,
the private monorepo's counters and its `--listFiles`, `--explainFiles` and `--traceResolution` output identical.
Condensed on 2026-10-10 (instrumentation and profiling how-to removed).

## Where the time went (the private monorepo, 4 checkers, medians of 5, load average 8-20)

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

1. Sub-phase rows in `--extendedDiagnostics` after Go's table (`tsrs_core::phases`). "Parse time" is Go's: all of
   program construction.
2. File loader: the parallel prefetch of each round also computes metadata and resolves imports, string module
   augmentations, type reference directives and triple-slash references (Go does all of a task's load in parallel);
   the sequential load consumes the results in the same order (the first module-resolution error is still the first
   in load order). Resolution results do not depend on order (resolver caches keyed by name, directory, mode,
   redirect); with `--traceResolution` resolutions stay sequential because traces report cache hits. Root file
   lookups run in parallel. Parse time 2.65 -> 0.98 s on a loaded machine.
3. The include glob reads the directory listings of every directory it will enter in parallel first; the walk, its
   order and its symlink-cycle check are unchanged. Config 0.155 -> 0.117 s.
4. Statistics' line count on the worker pool: 0.14 -> 0.015 s.
5. Checker assignment: per-file import targets on the worker pool, adjacency built in file order as before: assign
   0.070 -> 0.027 s (go assignment 0.052 -> 0.008); assignment unchanged.
6. `TSRS_FILE_TIMES=<path>` (experiments): per-file check seconds, checker, node count and more.
7. Prefetched resolutions carry the normalized resolved file name and its path: sequential load 0.13 -> 0.083 s.

## What remains in the frontend

- The parallel parse + resolve phase (~0.54 s, 14 rounds, round 1 = 27.5k root files ~0.33 s) is bound by
  kernel file-system calls: 114k `stat`s (module resolution probes, root lookups), 40k file reads, 5.3k
  `readdir`s; it stops scaling past ~8 threads. The worker profile shows mutex waits on the resolver's
  module-resolution cache (`SyncMap` is one `Mutex`), but a 64-shard map changed nothing measurable
  (0.51 s either way), so it was not landed. Fewer syscalls would need answering `file_exists` from cached
  directory listings, which is not exactly equivalent on a case-insensitive file system.
- Config glob walk/matching after the listing prefetch: ~0.045 s. Verify options: 0.04 s (sequential common
  source directory check). Bind: 0.04 s (parallel). Process exit (unmapping ~11 GB): ~0.1 s.

## Checker imbalance (4 checkers, locality assignment)

The slowest checker was consistently 10.5-10.7% above the mean over 5 runs; a perfect balance would have saved
~0.7 s of wall.

- Per-file check time is hardly predictable from syntax (correlation 0.17 with the weight, best single node kind
  0.21): it is dominated by the first touch of shared declarations in that checker (a 267-node endpoint costs
  0.47 s, `router/index.ts` 0.59 s, a 32-file directory 0.95 s). Over the 294 directory groups the weight explains
  more (correlation 0.81, R² 0.65).
- A fitted model (named import specifiers + 6x dynamic imports + nodes) reproduced the measured per-checker shares
  in-sample within 0.5 points but made the assignment worse when used (slowest checker 21.5% above the mean, check
  8.2 -> 9.5 s): changing the assignment moves the first-touch costs. Not landed.
- 3 / 4 / 5 checkers (locality): wall 11.8 / 10.3 / 8.8 s, peak 10.70 / 11.32 / 12.01 GB, slowest checker 4% / 10% /
  16% above the mean.

Status (2026-10-10): this note's "the default stays 4" and "tsrs writes nothing today" no longer hold. The default
checker count now scales with the cores (`default_checker_count`, checkerpool.rs), work stealing balances the
checkers (notes/perf-checker-stealing.md), and the opt-in `--checkerCostCache <file>` balances on per-file check
times measured by a previous run (execute.rs).

## Process level

- Startup: `tsrs --version` 5 ms; main starts within a few ms of exec.
- mimalloc options (env, 4-6 interleaved runs each): `PURGE_DELAY=-1`, `ARENA_EAGER_COMMIT=1`, `ARENA_RESERVE=4GiB`,
  `ALLOW_LARGE_OS_PAGES=1`: all within noise (run-to-run spread 8.2-10.8 s), peak within 0.05 GB. No change.
- Thread stacks (512 MB main/checkers, 256 MB rayon workers) were reservations only; nothing to gain.
- Fat LTO + codegen-units=1: -2.4% instructions, wall within noise. Judged then not worth the build time for every
  agent; `[profile.release]` has since moved to fat LTO and one codegen unit (notes/perf-pgo.md, status).
