# perf-frontend-64: program construction on 64 threads

Goal: tsrs's "Parse time" (all of program construction: config roots, file graph, binding, collection, option
verification) on vscode (10,427 files) was a flat 0.26 s at 4, 8, 16, 32 and 64 checkers on the 64-vCPU Depot runner
(`depot-ubuntu-24.04-64`, EPYC 9R45, 2 threads per core), 30% of the 0.86 s total at 64 checkers, while `bun check`
loads the same files in 0.27 s at 64 threads and 0.35 s at 16. The checker count does not size the front end's pool
(`RAYON_NUM_THREADS` does; the pool has one thread per vCPU by default), so the flat line meant the front end did not
scale with cores. Base: origin/main 1368df3 (bench/results/compare/2026-10-06-2420b7ed410b-64t.md). Branch
`perf/frontend`. Gates: diagnostics byte-identical (vscode, webpack, xstate-main at 1, 4, 16 checkers locally; vscode
at 4, 8, 16, 64 on the runner), `--listFiles` and `--explainFiles` (142k lines) identical, crate tests, lint ratchet.

## Measuring

- The front end alone: `tsrs -p src --noEmit --incremental false --extendedDiagnostics --pretty false --listFilesOnly`
  with `RAYON_NUM_THREADS=N` (notes/perf-checker-scaling.md measured it the same way). `--extendedDiagnostics` prints
  the tsrs sub-phases (`tsrs_core::phases`).
- `TSRS_FRONTEND_STATS=1` (new, `tsrs_core::festats`, off by default at the cost of one atomic load per probe): per
  thread accumulators around each prefetch job (metadata, read, parse, bind, resolve), around every lock acquisition
  of the resolver / package.json / cached-vfs maps and of the speculative walk's claimed set, summed over the pool
  after each round and printed as `Program:     stats: ...` rows: job wall, thread CPU, wall x threads, lock waits and
  acquisition counts, the longest job.
- 64 threads: `depot ci run --workflow .depot/workflows/perf-probe.yml` with tools/perf/probe.sh, which builds
  origin/main from a stash of the uploaded diff as the "base" binary and interleaves base and new; `perf record -e
  cpu-clock` (flat, user and kernel symbols) and `perf stat` for instructions. Four runs.

## Where the 0.26 s went (base, 64 threads)

| sub-phase | 64 threads | 16 | 1 |
| --- | --- | --- | --- |
| root file lookups | 0.017 | 0.015 | 0.021 |
| file graph: parallel parse + resolve (round 1 incl. the speculative walk; 9 rounds) | 0.136-0.146 | 0.139-0.142 | 1.62-1.66 |
| file graph: sequential load (per task: sub tasks from the prefetched resolutions) | 0.060-0.064 | 0.065-0.067 | 0.063-0.065 |
| collect files | 0.013-0.014 | 0.014-0.016 | 0.014 |
| verify options | 0.012 | 0.012 | 0.012 |
| Parse time | 0.255-0.265 | 0.267-0.269 | 1.755-1.792 |
| process: user / sys s | 2.6-3.4 / 4.0-5.0 | 2.1-2.2 / 0.3 | 1.8 / 0.1 |

The parallel phase did not scale past 16 threads and the kernel time exploded: at 64 threads `perf` put 52% of all
samples in `__pv_queued_spin_lock_slowpath` (the futex queues behind `std::sync::RwLock`), plus `RwLock::write_contended`
and `read_contended` in user space. The stats say why: the phase takes 112k acquisitions of the speculative walk's one
`Mutex<FxHashSet<Path>>` (one per import edge, nearly all for files that already have a task), 389k of the resolver's
and package.json `SyncMap`s (each one `RwLock<FxHashMap>`) and 250k of the cached vfs's five `RwLock<FxHashMap>`s
(file / directory existence, realpath, stat, listings), 650k contended acquisitions of a dozen locks in a 0.14 s
phase. On this Mac at 16 threads the waits were already 0.28 thread-seconds of 2.8 (claimed 0.02-0.03, syncmap
0.17-0.18, vfs 0.09-0.10) and tripled per doubling of threads (syncmap: 0.020 at 4, 0.064 at 8, 0.177 at 16).

The lib chain (`lib.es2024` -> ... -> `lib.es5`, one loader round per level, 9 rounds) was a suspect and is not it:
rounds 2-9 total ~1 ms; round 1 is 98% of the file graph.

The sequential load (0.064 s, the same at every thread count) spent, per import edge (110k), ~400 ns: the
`resolved_import_sub_task` decision (with a `to_path` allocation), `get_mode_for_usage_location`, the per-file
`ModeAwareCache` insert (hash + growth), the include reason, `resolved_file_name.to_string()` for a name the callee
did not use, one hash of the ~100-byte path to find the existing task and a second one in `filesParser::start`;
then collect files hashed every path twice more per edge (`task_data_by_path` and the include-reason map), and verify
options computed a canonical absolute path per file that only its error path reads.

## Changes

1. `tsrs_core::collections::SyncMap` is sharded: 64 `RwLock<hashbrown::HashTable<(K, V)>>` chosen by the top bits
   of the key's Fx hash, and the same hash probes the shard's table (one hash per operation; the first version with
   `FxHashMap` shards hashed twice and cost +0.8% instructions at one thread). `load` takes a borrowed key
   (`str` for `String`). `range` / `keys` / `to_map` visit the shards in index order (Go's `sync.Map.Range` has
   no order). The cached vfs's maps are now this type, keyed by `String` and probed by `&str`. hashbrown becomes a
   dependency of tsrs_core (it was a workspace dependency already; Cargo.lock gains one line).
2. The speculative walk's claimed set: a path that already has a task is answered from the round's read-only task map
   without a lock (2.8k lock acquisitions instead of 112k); the rest is a sharded `SyncSet`.
3. The parallel prefetch builds each import's sub task: `prefetchedImport` carries the resolution mode and a
   `preparedSubTask` (normalized name shared with the existing task's `Arc<str>`, path, data index when the path is
   known before the round, include reason, depth flags, package id), the file's `resolutions_in_file` map in import
   order and the task's `reason_path` string. The sequential load appends the prepared tasks in order
   (`add_prepared_sub_task`; a path first seen in the round is looked up again, so string sharing is as before), takes
   the prebuilt map when the file has no synthetic import (then its insertion sequence is the loop's), and still
   records the first resolution error in order. `resolvedRef::file_name` is a `Cow` (no copy for resolved modules).
4. `parseTask::data_id`: the data index set by `add_sub_task_normalized` / `prepare_sub_task` when known and by
   `start` always, so `start` and `get_processed_files` do not hash the path again; collect files accumulates include
   reasons by data index and moves them into the path-keyed map in first-seen order (the same map state); `seen` is a
   vector by data. `load` skips the extension checks a prefetched file passed in `task_needs_parse`.
5. `check_source_files_belong_to_path` (verify options, rootDir / outDir projects) computes the canonical absolute
   path only for a file outside the root directory, and the `contains_path` comparisons run on the pool (diagnostics
   stay in file order).

## Result (64-thread runner, final build vs origin/main, interleaved, medians of 3; front end = `--listFilesOnly`)

| threads | Parse time base -> new | parallel phase | sequential load | roots | collect | verify | user / sys s base | user / sys s new | peak RSS MB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1.764 -> 1.770 (1.765-1.806) | 1.630 -> 1.684 | 0.064 -> 0.016 | 0.021 -> 0.022 | 0.014 -> 0.010 | 0.012 -> 0.013 | 1.84 / 0.12 | 1.86 / 0.12 | 1259 -> 1251 |
| 4 | 0.570 -> 0.520 | 0.443 -> 0.458 | 0.065 -> 0.016 | 0.011 -> 0.009 | 0.014 -> 0.010 | 0.012 -> 0.005 | 1.97 / 0.11 | 1.95 / 0.16 | 1340 -> 1341 |
| 16 | 0.266 -> 0.171 | 0.141 -> 0.120 | 0.065 -> 0.016 | 0.014 -> 0.005 | 0.015 -> 0.010 | 0.012 -> 0.003 | 2.23 / 0.28 | 2.04 / 0.18 | 1525 -> 1517 |
| 32 | 0.255 -> 0.122 | 0.127 -> 0.074 | 0.067 -> 0.016 | 0.016 -> 0.005 | 0.015 -> 0.010 | 0.012 -> 0.002 | 2.71 / 1.40 | 2.23 / 0.31 | 1712 -> 1749 |
| 64 | 0.260 -> 0.109 | 0.139 -> 0.060 | 0.060 -> 0.016 | 0.017 -> 0.006 | 0.014 -> 0.010 | 0.012 -> 0.003 | 2.91 / 4.59 | 2.53 / 0.46 | 2059 -> 2122 |

At one thread the work the prefetch now does (sub tasks, include reasons, per-file maps) moves from the sequential
load into the "parallel" phase, which is the same thread: Parse time is unchanged, instructions are 23.72 -> 23.63 G
(`perf stat`, -0.4%). At 64 threads: instructions 25.60 -> 24.23 G, task-clock 7.78 -> 3.08 s, `perf`:
`__pv_queued_spin_lock_slowpath` 52% of samples -> 3.8%. On this Mac, one thread, `/usr/bin/time -l`: 27.29-27.35 G
-> 27.11-27.17 G instructions, peak 1275.6 -> 1267.2 MB.

Full check, vscode, 371 errors, same run (wall from `/usr/bin/time`, medians of 3):

| checkers | Parse base -> new | Check base -> new | Total base -> new | wall base -> new | user / sys base -> new | peak RSS GB |
| --- | --- | --- | --- | --- | --- | --- |
| 4 | 0.262 -> 0.113 | 2.685 -> 2.705 | 2.990 -> 2.865 | 3.01 -> 2.88 | 14.2 / 4.7 -> 13.1 / 0.6 | 3.18 -> 3.20 |
| 8 | 0.259 -> 0.112 | 1.420 -> 1.384 | 1.719 -> 1.542 | 1.74 -> 1.56 | 14.3 / 5.2 -> 13.4 / 0.6 | 3.41 -> 3.45 |
| 16 | 0.260 -> 0.113 | 0.793 -> 0.765 | 1.096 -> 0.922 | 1.12 -> 0.94 | 15.8 / 4.4 -> 14.7 / 0.5 | 3.76 -> 3.83 |
| 64 | 0.257 -> 0.106 | 0.728 -> 0.627 | 1.036 -> 0.787 | 1.06 -> 0.82 | 23.5 / 5.0 -> 20.6 / 0.8 | 5.26 -> 5.24 |

The check phase also gets faster at many checkers (0.73 -> 0.63 s at 64): the checker threads go through the same
cached-vfs and package.json maps (`realpath`, `file_exists`, `get_packages_map`), and the kernel time of the whole
process drops from 4-5 s to under 1 s at every checker count. bun check in the same run: "loaded in" 253-262 ms at
64 threads, 333-345 ms at 16; tsrs's Config + Parse is 0.135 s at 64 and 0.195 s at 16.

- Output: the diagnostics of vscode at 16 and 64 checkers are byte-identical base vs new on the runner (the Symbols /
  Types / Instantiations counters of `--extendedDiagnostics` differ from run to run at many checkers for the base
  binary too); vscode / webpack / xstate-main at 1, 4, 16 checkers identical on this Mac; `--listFiles` and
  `--explainFiles` (142k lines) identical.
- Peak RSS: within the run-to-run spread at 1-16 threads for the front end; +40-60 MB (+2-3%) at 32-64 threads, and
  +20-70 MB (about +1%) for the full check at 4-16 checkers (within the spread at 64), with the work moved from the
  main thread to the workers (per-thread allocator retention; not chased, see risks).

## What the parallel phase is now (64 threads, `TSRS_FRONTEND_STATS`)

0.060 s of wall = 3.8 thread-seconds: thread CPU 2.5, job wall 3.3 (parse 1.39, bind 0.75, resolve 0.79, metadata
0.23, read 0.12, lock waits 0.43), idle 0.5-0.8 (the end of the phase). Against 1.9 s of CPU at one thread, the CPU
grows 30% at 64 threads (2 threads per core), and the 0.43 s of lock waits are the remaining cost of 650k
acquisitions of shared cache lines across the CCDs; both are ~0.01 s of wall. The floor for this structure on this
machine is about 2.5 / 64 = 0.04 s.

## Tried and rejected

- Largest files first (`stat` every job, sort by size, so the 1 MB `.d.ts` files do not start last): the parallel
  phase went from 0.06 to 0.24 s at 64 threads and 0.12 to 0.43 at 16. Rayon splits an indexed `par_iter` into about
  one leaf per thread (150 of vscode's 9.6k files) that one thread then runs sequentially, so the sorted order put
  the 150 largest files into one leaf. With `with_max_len(8)` or `(2)` the order was neutral (0.057-0.065 at 64
  threads, 5 reps, within noise of the unsorted order), as was the leaf cap alone (32, 8, 2): the phase is not
  tail-bound. On this Mac the `stat` pass alone costs 0.2 s at 16 threads (kernel contention). Not landed.
- Speculating through the lib chain: rounds 2-9 are ~1 ms in total.

## Risks

- The sharded maps change the iteration order of `SyncMap::range` / `keys` / `to_map` relative to the old single
  `FxHashMap` (Go's `sync.Map` has no order either; the two CLI consumers that iterate resolutions are order-insensitive
  by their own comment). The language server's uses (auto-import registry, symlink sets, dirty maps) were not
  exercised beyond the crate tests.
- The `preparedSubTask` is built from the task map as it stood before the round; a path first seen in the round is
  re-checked sequentially so that string sharing (one `Arc<str>` per file name) stays as before. `--explainFiles`
  (every include reason) and `--listFiles` were compared on vscode only.
- Peak RSS at 32-64 threads (+2-3%, above).
