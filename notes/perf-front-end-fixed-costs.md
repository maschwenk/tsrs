# perf-front-end-fixed-costs: wall time before, around and after type checking on 64 vCPUs

Goal: cut the wall time tsrs spends outside the checkers' type-check pass (config, the parallel parse, the serial
steps of program construction, statistics, process startup and exit) on the 64-vCPU Depot runner
(`depot-ubuntu-24.04-64`, EPYC 9R45), with diagnostics byte-identical. Base: main 249561e (2026-10-07), vscode
`-p src` (10,427 files), the default 32 checkers and 32 parse threads unless noted.

## Measuring

- Phase rows: `--extendedDiagnostics` (`tsrs_core::phases`), as in notes/perf-frontend-64.md.
- A/B probes (`.depot/workflows/perf-probe.yml` with a probe script in the working copy; the scripts are not
  committed): base and new built in **the same source directory** with `--profile dist` (fat LTO, one codegen unit;
  base = the diff's `crates/` stashed), 7 interleaved runs per cell, wall timed with `wait4` to the millisecond,
  `perf stat` for instructions and page faults, `bench/count.py` (user-space instructions, deterministic) for one
  single-threaded run. Why dist: with `cargo build --release` (16 codegen units) the same diff moved Check time by
  +15 to +60 ms with fewer instructions and less CPU (a base built in another directory gets other crate metadata
  hashes, and any tsrs_core change repartitions codegen units); the dist A/B of the same diff showed Check within
  3 ms (runs hgfgxqpk7v vs z9czxzdx1v).
- Timelines: `perf record -F 10000` and the number of distinct threads with samples per millisecond (run
  s2q58gkl3m); call graphs from a `-C force-frame-pointers=yes` build (run t30qhwczr5); exit from `strace -ttt`
  (`exit_group` to "exited") and `perf record -a` with the `sys_enter_exit_group` tracepoint (run c9qzqhtmzk).
- Same-profile A/Bs: a dist build with one profile trained on the base (instrumented base binary on vscode, webpack
  and xstate-main), used for both builds (runs dsjjx4dfpf, srr05334p2, xggbvj0s9v). They took out the Check-time
  layout swings that the dist A/B of #154 and pr-verify's release builds of #150 showed. Between #143 and #153 an
  instrumented tsrs exited with `_exit`, so these trainings may have recorded little of the tsrs runs; both binaries
  of each A/B used the same profile either way.
- pr-verify (`.depot/workflows/pr-verify.yml`, from #138 on) for every PR after it landed: all diagnostics cells
  identical in every run (60/60, then 102/102 with Bun's projects); its 3-rep release-build walls do not resolve
  changes of a few ms.
- Timers that were not committed (`TSRS_TMP_*`): split of the include glob, the loader's drop, round planning.

## Where the time went (main 249561e, 32 checkers)

Wall 0.68 s. Total time 0.65 s: Config 0.025, Parse 0.125, Check 0.48-0.50. Outside "Total time": ~2 ms from exec
to the tsconfig open, Statistics 0.013-0.016 s (the bench passes `--extendedDiagnostics`), ~15 ms kernel exit.

| interval (perf timeline, ms, run slowed ~20% by perf) | busy threads | what |
| --- | ---: | --- |
| 0-12 | up to 64 | start, the include glob's listing prefetch on rayon's global pool (64 threads) |
| 13-33 | 1 | the include glob's visit (file matching 8.1 ms, directory matching 2.0 ms on the runner) and the extension-priority loop after it (6.8 ms) |
| 34-39 | 32 | root file lookups |
| 40-45 | 1 | `filesParser::start` over the 10.4k roots (2.2 ms locally) and round-1 planning (1.5-1.9 ms) |
| 46-126 | 32 | parallel parse + bind + resolve, saturated, ~2 ms tail |
| 127-147 | 1 | sequential load |
| 148-156 | few | rounds 2-9 (lib chain); round-2 planning walks ~110k queued sub tasks (3.4-5.3 ms locally) |
| 157-167 | 1 | collect files |
| 168-175 | 1 | `mi_free` / `drop_glue`: the file loader dropped on the main thread (3-4 ms locally; no phase row) |
| 176-177 | 32 | verify options |
| 185-196 | 1 + spawns | checker creation (32 thread spawns), file assignment (10 ms) |

The parallel parse is CPU-bound, not tail- or wait-bound: 32 threads have samples in every millisecond of it.
Thread CPU 2.12 s against 2.49 thread-seconds of wall (`TSRS_FRONTEND_STATS`): parse 0.89, bind 0.59, resolve 0.44,
read 0.09, metadata 0.08, lock waits 0.10. Kernel 11% of the front end's cycles: page clearing 2%, and a spinlock at
3.7%, of which 2.5 points are `opendir`/`closedir` of the listing prefetch (64 threads on the fd table lock).
Module resolution is 14.6% of the front end's CPU, mostly cache misses (relative imports are keyed per directory).

## Changes

Status (2026-10-10): #143's `_exit` was reverted in #191 (docs/STATUS.md); `crates/tsrs_cli/src/main.rs` ends
with `std::process::exit` again. The #143 row and its exit-time numbers no longer describe the binary; the lesson in
"A regression I caused" still applies to any change to how the process ends.

| PR | change | numbers (runner, dist A/B) |
| --- | --- | --- |
| #140 (merged) | Statistics: "Memory used" from /proc/self/statm (macOS: proc_pidinfo) instead of spawning `ps`; "Lines" from memchr passes instead of the per-byte loop | Statistics 13 -> 5 ms; instructions 143.6 -> 142.2 G at 32 checkers; wall 0.70 -> 0.68 s (10 ms resolution) |
| #141 (merged) | include glob: the matcher calls run on the listing prefetch's pool; extension groups built once; `try_get_extension_from_path_with` without a copy per candidate | Config 25 -> 15 ms (include glob 18 -> 10); Total 0.633 -> 0.631 s |
| #143 (merged) | the `tsrs` thread ends the process with `_exit` after flushing (skips mimalloc's process-done purge, ~300 madvise calls over ~600 MiB, and the thread-exit stack madvise) | wall minus Total 25.3 -> 23.1 ms (32 checkers), 26.8 -> 24.0 (64), 32.5 -> 30.1 (`--listFilesOnly`); pr-verify 60/60 identical |
| #146 (merged) | module: `normalizePathForCJSResolution` tests the last path component without building the component list | single-threaded instructions: front end 20.331 -> 19.980 G (-1.7%), full 111.430 -> 111.076 G; wall within noise |
| #149 (merged) | loader: round planning asks `task_needs_parse` before hashing; `loader.tasks` reserved per round; root task paths computed by the parallel lookups; per-file maps sized once; the loader's heap dropped on a helper thread | Parse time 0.120 -> 0.109 s, wall 0.662 -> 0.652 s (32 checkers); `--listFilesOnly` Parse 0.120 -> 0.112 s; sequential load 18 -> 16 ms; pr-verify 60/60 identical |
| #150 (merged) | checker pool: work groups run on kept threads (`broadcast`) instead of 32 new threads per group; a busy pool falls back to spawning | Statistics 5 -> 1 ms; wall minus Total 23.1 -> 18.8 ms (32 checkers); page faults -1k; same-profile A/B: Check 0.457 -> 0.458 s, wall 0.637 -> 0.633 s |
| #154 (merged) | loader: `run_queued` walks a file's casings by index and lends the sub task list; the collect walk's frames name their task instead of copying its list; redirect queries answer at once without project references | sequential load -2 ms in three A/Bs (16 -> 14, 16 -> 14, 15 -> 13); Parse 0.112 -> 0.108 / 0.109 -> 0.104 / 0.108 -> 0.105 s; pr-verify 102/102 identical |
| #156 | config parsed inside the worker pool, diagnostics collection installed into it: no rayon global pool in a CLI run | threads created 130 -> 66; listing prefetch 6 -> 4 ms; Config 15 -> 13 ms; root lookups 5 -> 4 ms; peak RSS 2.79 -> 2.74 GiB; page faults 36.6k -> 34.7k |

### A regression I caused (#143) and its fixes

`_exit` also skips the exit handlers that instrumented binaries use to write their profiles. Between #143 and the
fixes, the PGO training runs of the `tsrs` binary wrote empty profiles (+1.8-3.5% instructions in the README bench;
fixed in #153 by keeping `exit` when `LLVM_PROFILE_FILE` is set), and the release workflow's BOLT step would have
found no profile from `tsrs` (fixed in #155: the BOLT training sets `MIMALLOC_SHOW_STATS`, which keeps `exit`). The
gates I ran for #143 (output, exit codes, the probe's dist builds) do not run an instrumented binary. Any future
change to how the process ends needs a PGO and a BOLT training run.

## Before and after (vscode, 32 checkers)

Rows from the first probe (txghscqb3d, main 249561e, release build) and from the last A/B (xggbvj0s9v: main fae854b
with #140-#150, plus #156; same-profile dist build). The builds differ, so compare the serial rows rather than the
whole-run wall. The check pass also changed under other agents in between.

| row | before | after |
| --- | ---: | ---: |
| Config time | 25 ms | 13 ms |
| include glob (listing prefetch) | 18 (6) ms | 8 (4) ms |
| root file lookups | 5 ms | 4 ms |
| parallel parse + resolve | 74 ms | 67 ms |
| sequential load | 18 ms | 15 ms (13 with #154) |
| collect files | 10 ms | 10 ms |
| Statistics (`--extendedDiagnostics`) | 13-16 ms | 1 ms |
| wall minus Total time (startup, statistics, report, exit) | ~32 ms (10 ms timer) | 17.8 ms |
| threads created per run | 386 | 66 |
| peak RSS | 2.79 GiB | 2.74 GiB |

## Tried, not landed

- Releasing memory before exit. The kernel teardown is ~15 ms: ~12.5 ms of `unmap_vmas` on the last thread and
  every thread's `do_exit` (perf -a after `exit_group`). `munmap` of the arena reservation before `exit` took 3.0 ms
  and the teardown moved within noise; `madvise(MADV_DONTNEED)` of the arena from 16 threads took 4.2 ms (slower
  than one munmap: TLB shootdowns and page freeing contend). Run c9qzqhtmzk.
- Smaller global rayon pool (64 / 32 / 8 threads, built at start): teardown 15.1 / 14.2 / 13.6 ms, pool build
  1.7 / 1.0 / 0.3 ms, listing prefetch 4 ms with any of them once the pool exists; wall within noise. Run qmxc6p3tlb.
- Spawning both pools from a helper thread when the config is read: root file lookups 5 -> 4 ms, Config 15 -> 14 ms,
  listing prefetch unchanged (reading the tsconfig is shorter than spawning 64 threads). Starting it earlier would
  spawn ~96 threads for `--version` too. Run fn3d5hhlzt.

## What remains (vscode, 32 checkers, after the merged changes)

Serial work on the program thread, from the phase rows and the program-thread profiles:

| step | ms | note |
| --- | ---: | --- |
| config | 13 | listing prefetch 4 (`opendir`/`closedir` contending on the fd table, with #156 also the worker pool's spawn), extension-priority loop ~3, tsconfig ~1-2 |
| root file lookups | 4 | parallel |
| sequential load | 13-15 | per import edge: `add_prepared_sub_task` (a new task per edge), resolutions and include reasons; about 130 ns per edge |
| collect files | 10 | the depth-first walk over ~120k tasks (memory-latency bound: random access into a 20+ MB task vector) |
| checker creation, assignment | 4-5, 10 | assignment is the checker pool's locality algorithm (another agent's area) |
| global diagnostics (first) | 15 | checker initialization (globals merge), inside Check time |
| kernel exit | ~15 | `unmap_vmas` of ~2.9 GB and every thread's `do_exit`; not reducible from user space without less memory or fewer threads (measured above) |

The parallel parse itself is saturated: ~2.1 thread-seconds of CPU on 32 threads (parse 0.89, bind 0.59, resolve
0.44). Cutting it needs less CPU per file. Module resolution is mostly cache misses on relative imports (one cache
key per directory). The scanner was tuned in notes/perf-parse.md. The binder belongs to another agent.

Ideas not tried:

- The collect walk's include-reason and per-file map work could move after the walk and run per map in parallel;
  the walk itself (order of files) must stay sequential.
- Spawning the worker pool from a helper thread at process start would hide its ~1 ms; it needs a rule for commands
  that never compile (`--version`, `--help`).
- `Diagnostics: report` (2 ms for 371 diagnostics) computes the line maps of the files with errors lazily, one by
  one; computing them on the pool first would save about 1 ms.
