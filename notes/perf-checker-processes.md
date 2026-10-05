# perf-checker-processes: forked checker processes that share one warm checker (spike, parked)

The idea: let one checker (the base) check or resolve a warm-up set, then `fork()` N workers. Each child inherits
the whole process copy-on-write (program, AST, binder symbols, and the base checker with every type it built) and
continues the same checker on its own slice of the files. It sends its files' diagnostics back over a pipe. Go cannot do
this: its runtime is multi-threaded and garbage collected. tsrs can, because all checker state lives in arenas at a fixed
address (`tsrs_core::reserve`), so a child sees the same handles.

Verdict: the spike works and its output matches threads at the same N, but it is **not a wall-time win and was not
landed**. It saves CPU (17-47% fewer instructions) and some memory (0.2-1.9 GiB at 8-16 workers). Wall time is at parity
on macOS and 5-10% worse on Linux. Code: branch `perf/checker-processes` (commits up to 112dbb3 have the measurement
tools: write tracer, VM accounting, pause hook).

## Why wall time cannot improve (the model)

With N checkers each slice costs S + U/N: S is the shared type graph every checker needs (the router aggregator, the
service / ORM graph), U the per-file work. Threads pay S once per checker, but in parallel, so the critical path is
S + U/N and the total CPU is N·S + U. Processes pay S once, serially, before the fork. The critical path is again
S + U/N, but the total is S + U. So processes can save CPU and memory, not time. The second-order effects push both
ways. Less contention for cores and memory bandwidth helps. Fork cost and copy-on-write faults (85-185k minor faults
per child) hurt. Both modes keep the same static imbalance: the slowest worker is 30-60% above the others.

On the 38k-file codebase (Mac) S is large: one checker has 17.0 s of checker CPU, eight have 33.9 s
(`TSRS_FILE_TIMES`). 96 files carry 17.7 s of the excess at 16 checkers: each costs 0-5 ms in a checker that has the
graph and 0.2-1.2 s in one that does not.

## What was built (spike)

- `tsrs_core::procs::fork_children` / `ForkedChildren::join`: N children from the calling thread. Each runs a
  closure, writes a length-prefixed result to a pipe and `_exit`s. Its stdout/stderr go to a second pipe. The parent
  polls all pipes at once (a child blocked on a full pipe never waits for another), reaps every child, and turns a
  killed child, a non-zero exit or a truncated stream into an error carrying the child's output.
- `tsrs_ast::{encode,decode}_diagnostic`: every field (file by index, span, code, category, source, message by key or
  ad-hoc text, args, chain, related information, the three flags, repopulate info).
- `checkerpool_procs.rs`: one checker in the parent. Slices are the locality assignment for N; the parent checks slice 0
  itself; global diagnostics are merged and de-duplicated; Types/Symbols/Instantiations are summed (base plus each
  child's delta, so they differ from thread mode). Only the command line's single `--noEmit` program may fork: not
  `--build`, the test runners, the language server or the API, and not incremental runs.
- Before the fork, every node and binder symbol gets its id (parallel, deterministic: a counting pass, prefix sums,
  then each file assigns its range in tree order; a symbol belongs to the file of its first declaration; 0.05 s for
  20.4M nodes and 3.7M symbols on the Mac).

Fork safety, as implemented: fork from the checker thread (512 MB stack) while the rayon pools are between jobs and the
thread that started the command line waits in `join`. Nothing in a child uses a thread pool, spawns a thread or takes a
lock another thread could hold. The child ends with `_exit`, and stdout/stderr are flushed before forking. The test in
`crates/tsrs_core/tests/procs.rs` (one test function, so the harness runs no other thread meanwhile) forks 16
children 50 times, plus failing and panicking children. The checker pool also ran 16 processes on xstate and the
38k-file codebase without a hang.

## Results

Times are wall seconds of the whole run; "instr" covers the parent plus all children (macOS: `/usr/bin/time` for the
parent, `proc_pid_rusage` in each child; Linux: `perf stat -e instructions:u`, which follows children). Memory is
explained under "Measuring memory".

### Warm-up strategies (Mac, N = 8, instructions are load-independent; threads 473 G, one checker 278 G)

| warm-up | serial time s | instr G | share of the 8-thread excess removed |
| --- | --- | --- | --- |
| none | 0 | 479 | 0% |
| program-order prefix 1% / 2% / 5% of checked weight | 0.42 / 1.21 / 2.13 | 464 / 422 / 354 | 8% / 26% / 62% |
| every k-th file of each slice, hub-first, 0.5% / 1% / 2% | 0.98 / 1.93 / 2.64 | 400 / 341 / 326 | 40% / 69% / 77% |
| the same 1%, program order | 2.39 | 335 | 72% |
| 50 / 200 most-imported files | 0.48 / 1.23 | 432 / 394 | 24% / 42% |
| resolve the exports of the 100 / 300 / 1,000 / 3,000 most-imported modules (no file checked) | 0.83 / 0.98 / 1.32 / 1.71 | 395 / 389 / 361 / 360 | 45% / 48% / 62% / 62% |
| oracle: the 53 measured first-toucher files of an 8-thread run | 2.41 | 319 | 79% |

Removing excess costs about as much serial time as it saves per worker. Resolving exports gets the most per second.
Checking files in a warm-up drags their own per-file work onto the serial path. Hub-first order beats program order for
the same files (1.93 vs 2.39 s).

### Threads vs processes, Mac (38k-file codebase, 18 cores, load 5-40, interleaved)

Processes use the exports-of-100-hubs warm-up plus id pre-assignment.

| N | threads s | processes s | threads instr G | processes instr G | threads GiB | processes GiB (VM accounting / system anon) |
| --- | --- | --- | --- | --- | --- | --- |
| 8 | 5.48 | 5.42 | 473 | 395 | 7.26 | 7.02 / - |
| 12 | 5.62 | 5.60 | 535 | 419 | 8.48 | 7.76 / - |
| 16 | 4.58 (8 rounds) | 4.58 (8 rounds) | 595 | 431 | 9.48 (anon 9.47) | 8.19 / 6.62 (5 rounds) |
| 16, oracle warm-up | 4.58 | 6.88 | 595 | 328 | 9.48 | 8.84 / 7.82 |

Without a warm-up (ids only) processes use more memory than threads (7.80 / 8.79 / 9.69 GiB at 8 / 12 / 16): each child
rebuilds S and also copies pages.

### Threads vs processes, Linux (x86-64, 18 vCPU, the 40k-error corpus, interleaved)

| build | N | threads s | processes s | threads instr G | processes instr G | threads GiB | processes GiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| main (no huge pages) | 8 | 18.62 | 19.45 (+4.5%) | 451 | 372 | 7.45 | 6.51 |
| main | 12 | 14.02 | 15.13 (+7.9%) | 490 | 377 | 8.44 | 7.06 |
| main | 16 | 13.53 | 14.70 (+8.6%) | 539 | 388 | 9.57 | 7.70 |
| + `perf/linux-thp-arena` | 8 | 17.50 | 18.55 (+6%) | 451 | 372 | 7.47 | 6.54 |
| + `perf/linux-thp-arena` | 16 | 12.57 | 13.50 (+7%) | 539 | 388 | 9.60 | 7.71 |
| + THP, oracle warm-up (182 files at 16) | 8 / 16 | 17.50 / 12.57 | 21.57 / 18.27 | 451 / 539 | 285 / 284 | 7.47 / 9.60 | 6.29 / 7.82 |

Huge pages make threads 8% faster. Huge pages also cut the fork from 20-70 ms to 2.5-4 ms per child. The Linux warm-up
is slow (2.9 s for the 100 hubs vs 0.83 s on the Mac), which is most of the gap.

### Fork cost and faults

- macOS (2.2 GB parent, 16 KiB pages): the first fork takes 14-19 ms, later ones 0.3-0.8 ms.
- Linux (2.6 GB parent, 4 KiB pages): 20-70 ms per fork without huge pages, 2.5-4 ms with them.
- Each child takes 85-185k minor faults.

## Measuring memory

- Linux: summed PSS of all processes from `/proc/<pid>/smaps_rollup`. Sampling during the run under-counts, because
  reading 16 rollups takes longer than the children take to exit. The table therefore uses a snapshot with every
  process paused at its peak (`TSRS_CHECKER_PROCESSES_PAUSE`). That is an upper bound: in a real run children exit one
  by one. The drop in `MemAvailable` agrees within ~0.5 GiB for processes. For threads it reads ~1 GiB below RSS.
- macOS: `phys_footprint` counts shared copy-on-write pages that a child only read (a 1 GiB test: a child that read
  everything showed 1 GiB). The `footprint` tool de-duplicates a simple test program correctly, but not these
  children: it reported 12 GB for an 8-process run that really used ~7 GiB. Two methods that agree with each other:
  - VM-region accounting in each process (`mach_vm_region_recurse`): the parent's resident pages plus each child's
    private pages (private regions, and in copy-on-write regions the pages of the top object). This is the snapshot
    upper bound, measured with an idle parent and N children.
  - The system-wide `vm.page_pageable_internal_count` peak over the run, medians of 5. It is noisy because other
    processes run, but it reproduces `/usr/bin/time` for threads (9.47 vs 9.48 GiB).

## What children write into shared pages

Traced on macOS with every inherited page read-only and a fault handler recording the first writer (pc plus 7
frame-pointer callers). 7 children, oracle warm-up, per child:
- before the id pre-assignment: 156 MiB of front-end (AST/binder) pages, 69% of them from `assign_node_id` (40%) and
  `assign_symbol_id` (30%), plus lazy JSDoc's read lock (`eager_jsdoc`);
- base-checker objects: 60 MiB (lazily filled type fields);
- mimalloc heap: 325 MiB: inserts into the checker's hash tables (instantiation and union caches,
  `Relation::set` and its rehash, lazy member tables, pointer-keyed link stores) and mimalloc's page metadata.

Pre-assigning ids removes 90% of the front-end writes (156 to 16 MiB per child) and 1.06 GiB at N = 8 on the Mac.
Linux pagemap diffs (same frame = shared, different = copied, child-only = new) agree: arena copies per child fell from
~400 to ~180 MB, and heap copies stay ~240 MB. After that, a child's private state (0.4-0.75 GiB) is mostly its own
caches plus the copied hash-table pages, the same order as one more checker thread. That is why the memory saving is
small.

Pre-assigning ids in thread mode was measured too (Mac, 3 rounds): +1.6% instructions (the walk), wall within noise,
peak +0.04 GiB at 8 checkers. The less dense id order did not hurt the id-keyed link stores, and it did not help either.
Not landed.

## Exactness

Diagnostics were byte-identical to threads at the same N in every run:
- xstate, webpack (1,027 lines), the 38k-file codebase (0 errors);
- an error-rich clone of the 38k-file codebase with `describe` renamed to `description` in
  `OwnerEndpointDeclaration`: 10,781 errors in 2,202 files;
- the Linux corpus (40,543 errors), at N = 4, 8, 12 and 16, for every warm-up.

Finding: on the Linux corpus **thread mode itself** prints different text at N = 8 than at N = 12 / 16 (and 4). Two
TS2339 / TS2322 messages print an object-literal union with its filled-in optional property first or last
(`{ subject?: undefined; type: string; }` vs `{ type: string; subject?: undefined; }`). This is the order dependence
notes/perf-balance.md saw when reversing the file order. It is the subject of the next stream (output independent of
the checker assignment).

## What would have to be true for processes to pay

- The children's private state would have to be well below a thread checker's. That needs the checker's big hash tables
  split into a frozen base and a per-child overlay, which is invasive on the hottest paths, and lazily filled type
  fields kept out of shared pages.
- A warm-up cheaper than the S it removes would have to exist. The data says it does not: building S costs what it
  costs, serial or not.
- Or dynamic load balancing, where processes have an edge: a stolen file costs a process less because it has the warm
  base. That needs output independent of the assignment first (see above).

## Reproduce

```sh
# spike branch, Mac; corpora as in common4.md
TSRS_CHECKER_PROCESSES=16 TSRS_CHECKER_PREASSIGN_IDS=1 TSRS_CHECKER_WARMUP=none TSRS_CHECKER_WARMUP_EXPORTS=100 \
  TSRS_CHECKER_PROCESSES_STATS=1 tsrs -p . --noEmit --incremental false --extendedDiagnostics --pretty false
# warm-up variants: TSRS_CHECKER_WARMUP=prefix:<permille>|stride:<permille>|stride-po:<permille>|indeg:<n>|files:<list>
# oracle list: TSRS_FILE_TIMES at --checkers 1 and N, files with > 50 ms excess
# memory snapshot: TSRS_CHECKER_PROCESSES_PAUSE=<dir> (processes create <dir>/child-<k>; touch <dir>/go to release)
# write tracing (macOS arm64): TSRS_CHECKER_PROCESSES_WRITETRACE=<dir> [TSRS_CHECKER_PROCESSES_WRITETRACE_HEAP=1]
```
