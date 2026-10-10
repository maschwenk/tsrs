# perf-checker-scaling: what more checkers buy, and what they lose

With Go's default of 4 checkers the check phase is most of a cold run on the 38k-file codebase. This note measures
the curve from 1 to 16 checkers on the 38k-file codebase, vscode and webpack, splits the gap to linear scaling into
its causes, tries to reduce them without changing results, and proposes a default checker count that depends on the
machine. Diagnostics are identical for every checker count and assignment (gated below); the `--extendedDiagnostics`
Types / Symbols / Instantiations counters depend on the count and the assignment (each checker counts what it
creates) but not on the run.

Machine: Apple M5 Max, 18 cores (6 "Super" + 12 "Performance", `hw.perflevel0/1`), 128 GiB. Shared all day with
other agents' builds and corpus runs: 1-minute load 19-55 during the measurements (stated per table). Wall times are
therefore medians of interleaved rounds and the per-checker numbers are thread CPU seconds
(`TSRS_ASSIGNMENT_STATS=times`, `CLOCK_THREAD_CPUTIME_ID`), which do not grow when another process takes the core
but do grow when a core runs slower or waits on memory. Instructions retired (`/usr/bin/time -l`) are the
load-independent work measure.

Setup: base = origin/main 89b53b5 (9850161 + a bench-results commit), `--noEmit --incremental false
--extendedDiagnostics --pretty false`, `--checkers N`, default locality assignment. The driver scripts are
not committed; the commands at the end reproduce every measurement.

## 1. The curve

The 38k-file codebase, medians of 3 interleaved rounds, load 25-32 (k = checkers; "instr" is the whole process,
the parse/bind front end is 50.5 G of it, measured with `--listFilesOnly`):

| k | wall s | check s | checker CPU sum s | slowest checker CPU s | slowest / mean | instr G | peak GiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 22.16 | 20.16 | 19.10 | 19.10 | - | 296.5 | 4.32 |
| 2 | 16.40 | 14.66 | 25.04 | 13.08 | +4.5% | 337.9 | 4.92 |
| 4 | 11.56 | 10.51 | 32.29 | 8.89 | +10.1% | 397.5 | 5.79 |
| 6 | 12.32 | 9.60 | 40.45 | 7.71 | +15.8% | 462.7 | 6.70 |
| 8 | 8.92 | 7.83 | 45.11 | 6.71 | +18.9% | 503.8 | 7.37 |
| 10 | 8.58 | 6.60 | 48.53 | 6.08 | +24.4% | 540.9 | 7.96 |
| 12 | 7.96 | 6.75 | 52.66 | 5.63 | +28.1% | 571.0 | 8.52 |
| 16 | 7.87 | 6.80 | 58.60 | 5.00 | +34.8% | 635.0 | 9.43 |

vscode (`src`, 371 errors) and webpack (840 errors), same session, load 34-38:

| k | vscode check s | CPU sum | slowest | slowest / mean | instr G | peak GiB | webpack check s | CPU sum | slowest / mean | instr G | peak GiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 6.86 | 6.74 | 6.74 | - | 120.1 | 2.04 | 0.67 | 0.67 | - | 15.2 | 0.32 |
| 2 | 4.12 | 7.75 | 4.01 | +3.5% | 123.4 | 2.16 | 0.47 | 0.71 | +32% | 15.8 | 0.35 |
| 4 | 3.02 | 9.52 | 2.98 | +25% | 126.1 | 2.29 | 0.30 | 0.78 | +54% | 16.7 | 0.38 |
| 6 | 2.47 | 9.19 | 2.23 | +30% | 128.6 | 2.40 | 0.26 | 0.98 | +59% | 17.9 | 0.42 |
| 8 | 1.49 | 10.44 | 1.47 | +14% | 131.3 | 2.54 | 0.20 | 1.00 | +60% | 18.5 | 0.45 |
| 10 | 1.77 | 11.80 | 1.58 | +32% | 133.4 | 2.64 | 0.19 | 1.18 | +67% | 19.3 | 0.49 |
| 12 | 2.33 | 12.54 | 1.34 | +29% | 135.4 | 2.69 | 0.17 | 1.27 | +51% | 20.1 | 0.52 |
| 16 | 1.36 | 12.43 | 1.08 | +39% | 138.2 | 2.88 | 0.14 | 1.46 | +43% | 20.8 | 0.56 |

Output (diagnostics text and order) was byte-identical in every run of a project, for every k. Counters were
identical across runs of the same k and binary, with one exception that is not the checker pool: the first seven
runs of the day on the 38k-file codebase counted 2 more symbols at k <= 10 (k = 1 included); an agent was editing a
corpus file at the time (`src/apiServer.test.ts` has a modification time from this morning) and every later run
agrees.

## 2. Where the gap to linear goes

Slowest checker CPU = (one-checker CPU / k) x (a) x (b) x (c), with (b) = checker instructions at k / at 1
(duplicated work), (c) = CPU seconds per checker instruction at k / at 1 (the same instruction costs more), (a) =
slowest / mean (imbalance). The check wall time is the slowest checker plus scheduling (the last column; > 1 when
the machine has no free core for every checker).

| project | k | ideal s | (b) duplicated instructions | (c) CPU per instruction | (a) slowest / mean | slowest CPU s | check wall / slowest |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 38k-file | 2 | 9.55 | +17% | +12% | +4% | 13.08 | 1.12 |
| 38k-file | 4 | 4.78 | +41% | +20% | +10% | 8.89 | 1.18 |
| 38k-file | 8 | 2.39 | +84% | +28% | +19% | 6.71 | 1.17 |
| 38k-file | 12 | 1.59 | +112% | +30% | +28% | 5.63 | 1.20 |
| 38k-file | 16 | 1.19 | +138% | +29% | +35% | 5.00 | 1.36 |
| vscode | 4 | 1.69 | +7% | +33% | +25% | 2.98 | 1.01 |
| vscode | 8 | 0.84 | +12% | +38% | +14% | 1.47 | 1.01 |
| vscode | 16 | 0.42 | +20% | +54% | +39% | 1.08 | 1.26 |
| webpack | 4 | 0.17 | +13% | +3% | +54% | 0.30 | 1.02 |
| webpack | 8 | 0.08 | +30% | +15% | +60% | 0.20 | 0.99 |
| webpack | 16 | 0.04 | +50% | +45% | +43% | 0.13 | 1.08 |

(checker instructions = process instructions minus the front end: 50.5 G / 28.0 G / 4.19 G.)

### (b) duplicated work: a few heavy type graphs that every checker builds again

On the 38k-file codebase duplication is the largest loss; on vscode it is small. Per-file thread CPU
(`TSRS_FILE_TIMES`, which now has a CPU column) at 1, 8 and 16 checkers:

- The excess is concentrated in a few files. At 8 checkers 58 files have > 50 ms more CPU than in the one-checker
  run and account for 11.2 s of the 13.4 s summed excess; at 16 checkers 103 files account for 19.8 of 26.1 s. In the
  one-checker run these files cost 0-5 ms; in a checker that has not resolved what they reach they cost 0.2-2 s.
  Examples: the router aggregator `src/router/index.ts` (128 imports): 3 ms with one checker, 1.35 s at 8, 2.06 s at
  16; `src/router/resolveRouter/resolveRouter.ts` 1 ms -> 0.62 s; a scout service 4 ms -> 0.61 s.
- A checker's fixed cost is small: 16 random project files, each alone in a fresh checker (checkers run one after
  the other on one thread, so no contention), cost 0-60 ms each. Lib types are not where the duplication is.
  Creating the checkers takes 3-7 ms (in parallel).
- The heavy files share one graph. The 16 files with the largest excess at 16 checkers cost 7.2 s when each runs
  alone in a fresh checker, but 1.22 s when the 15 non-router ones run together in one fresh checker (the first one,
  0.27 s, builds what the others reuse; the router aggregator alone is 1.8 s). Their common imports are the
  activity-reporting service, the job / workflow definitions and the ORM: a project-wide service graph (~0.3-0.5 s
  of checker CPU) that every checker builds as soon as it checks its first file that reaches it, plus the router
  type (~1.5-2 s) that whichever checker holds the aggregator builds.

Since the graph is reached from files in every directory, every checker builds it, whatever the partition. That is
the per-checker fixed cost that grows the total with k; it cannot be removed by assignment, and sharing types across
checkers cannot be exact (notes/mem-shared-base.md).

Partitions tried at 8 and 12 checkers (instructions are deterministic per assignment to about +-1%; CPU at load
33-39, medians of 2):

| variant | k = 8 instr G | slowest CPU s | k = 12 instr G | slowest CPU s |
| --- | --- | --- | --- | --- |
| locality (default: groups <= 1/4 checker load, FENNEL penalty 1) | 504.4 | 5.53 | 569.9 | 4.45 |
| groups <= 1/2 checker load | 493.8 | 5.46 | 546.4 | 4.55 |
| groups <= 1/8 | 497.0 | 5.37 | 546.2 | 5.54 |
| groups <= 1/16 | 504.9 | 5.88 | 573.7 | 4.62 |
| FENNEL penalty 0.25 (more affinity) | 484.7 | 5.35 | 577.8 | 4.17 |
| FENNEL penalty 4 (more balance) | 495.3 | 5.71 | 558.2 | 4.62 |

Each variant moves instructions by up to 4% in one direction at one count and the other direction at the other, and
no variant lowers the slowest checker at both counts: which checker the heavy graph's first users land on decides
more than the partition parameters do. Not landed. Clustering by hub imports was not built: the hubs that matter
are imported from every directory, so any partition gives each checker some of their users. In-checker order stays
program order (it is load-bearing for printed property order, notes/perf-balance.md, and a reordering cannot avoid
the graph either).

### (c) the same work gets slower with more checkers

A fixed file set S (checker 0 of the 4-checker locality assignment, 6,070 files, 25% of the weight) on checker 0,
with the rest of the program split into contiguous path runs over the other k - 1 checkers
(`TSRS_CHECKER_ASSIGNMENT=file:`). Checker 0 executes the same instructions in every configuration. Thread CPU of
checker 0, medians of 3, load 27-39:

| configuration | S CPU s | vs alone |
| --- | --- | --- |
| alone (both checkers run one after the other on one thread) | 5.15 | - |
| 2 checkers concurrently | 5.25 | +2% |
| 4 | 5.84 | +13% |
| 6 | 6.02 | +17% |
| 8 | 6.70 | +30% |
| 12 | 6.62 | +29% |
| 16 | 7.00 | +36% |

So 30% of CPU per instruction is lost to the hardware at 8 checkers on this loaded machine: the checker threads
share the clusters' L2 and the memory system with each other and with the other agents, the 12 "Performance" cores
are slower than the 6 "Super" ones, and clocks drop with more active cores. The process-wide cycles counter moves
with CPU time, so the split between frequency and stalls is not visible from `rusage`. Nothing in tsrs can change
this; it is why the summed CPU (19.1 -> 45.1 s at 8) grows faster than the instructions (246 -> 453 G).

### (a) imbalance grows with k

Slowest / mean on the 38k-file codebase: +4% (2), +10% (4), +19% (8), +28% (12), +35% (16). The static weights
cannot see the heavy graph: the router aggregator carries 0.9% of a 16-checker load by weight and costs 55% of the
mean checker CPU. Finer groups average more files per checker but do not help (table above), because one or two
files decide the slowest checker.

- The opt-in cost cache (`--checkerCostCache`, notes/perf-balance.md) at 8 checkers: 20% -> 5.7 / 6.3 / 4.5% on runs
  3-5 (slowest checker 6.14 -> 5.2-5.9 s), CPU sum unchanged within noise. At 12 checkers 28% -> 17 / 16 / 14%: the
  router group alone is ~half a checker's load, and refinement moves only groups off the slowest checker. It stays
  opt-in (it writes a file).
- Landed, exact: **checked declaration files get the source weight multiplier.** The locality weights kept Go's 4x
  multiplier for source files only. Go uses it to tell checked sources from declaration files that skipLibCheck
  mostly leaves unchecked; the locality weights already give unchecked files weight 0, so a checked declaration file
  (no skipLibCheck) was weighted at a quarter of its work. webpack (642 checked declaration files: 393 ns of check
  CPU per base weight unit, sources 536): the checker holding the lib files was the slowest. Base vs this change,
  5 interleaved rounds, load 33:

  | webpack | base check s | slowest / mean | this change check s | slowest / mean |
  | --- | --- | --- | --- | --- |
  | 2 checkers | 0.52 | +32.5% | 0.47 | +19.5% |
  | 4 checkers | 0.36 | +52.6% | 0.28 | +12.4% |
  | 8 checkers | 0.23 | +60.0% | 0.21 | +38.8% |

  CPU sum unchanged (0.95 / 0.96 s at 4), output identical. The 38k-file codebase and vscode use skipLibCheck, so
  their assignment and counters are unchanged (verified: 16,549,988 symbols / 13,788,912 types at 4 checkers;
  vscode 5,177,105 / 3,099,826).

Work stealing would balance at run time but makes the assignment, and with it the counters, vary from run to run.
Not pursued. (Status 2026-10-10: stealing landed for the type-check pass, made safe by output that does not depend on
the assignment; notes/perf-checker-stealing.md, notes/perf-order-independence.md.)

### (d) serial sections around the check

Nothing to take. On the 38k-file codebase: config 0.16 s and program construction ~1 s (already parallel) are
independent of k; checker creation 3-7 ms (in parallel), file assignment 27-38 ms (one thread; the import adjacency
is built on the worker pool), global diagnostics < 15 ms, diagnostics sort and report < 1 ms (0
errors), process exit after `Total time` 0.08 s (1 checker) to 0.14 s (16: more memory to unmap). The check
phase ends within a millisecond of the slowest checker. At 16 checkers the serial parts are < 0.1 s of a 7 s run.

## 3. Small programs

The 38k-file default must not cost small programs anything. xstate (248 checked files) and a 5-file project (68
files with the libs), base binary, medians of 5 / 3, load 19-25:

| | k | check s | instr G | peak MiB |
| --- | --- | --- | --- | --- |
| 5 files | 1 / 4 / 8 / 16 | 0.00 | 0.33 / 0.37 / 0.38 / 0.41 | 22 / 26 / 29 / 33 |
| xstate | 4 / 6 / 8 / 10 / 12 | 0.20 / 0.14 / 0.14 / 0.11 / 0.11 | 9.7 / 10.3 / 10.7 / 11.2 / 11.9 | 245 / 266 / 287 / 307 / 328 |

Idle checkers cost ~5 M instructions and ~1 MiB each (creation and global diagnostics); the 5-file program has work
for only 5 checkers.

## 4. The default

Measured for the decision (medians of 5, interleaved, load 19-23, base binary):

| | 4 checkers (Go) | 6 | 8 | 10 | 12 |
| --- | --- | --- | --- | --- | --- |
| 38k-file wall s | 7.43 | 6.91 | 5.93 (-20%) | 5.37 | 5.22 (-30%) |
| 38k-file peak GiB | 5.78 | 6.69 | 7.38 (+1.60) | 7.98 | 8.59 (+2.81) |
| vscode wall s | 3.38 | 2.48 | 1.84 (-46%) | 2.12 | 1.53 |
| vscode peak GiB | 2.29 | 2.40 | 2.54 (+0.25) | 2.64 | 2.75 |
| webpack wall s | 0.41 | 0.33 | 0.28 (-32%) | 0.27 | 0.29 |
| webpack peak GiB | 0.38 | 0.42 | 0.45 | 0.49 | 0.52 |
| xstate wall s | 0.25 | 0.20 | 0.20 | 0.17 | 0.17 |

Beyond 8 the 38k-file codebase gains another 12% for 1.2 GiB, vscode and webpack nothing reliable; per-checker
memory is ~0.4 GiB on the 38k-file codebase. The proposal (the last commit, separate so it can be dropped):

- without `--checkers` and `--singleThreaded`: `clamp(min(available_parallelism / 2, checked files / 32), 4, 8)`.
  On this machine (18 cores) that is 8 for the three projects, 7 for xstate, 4 for the 5-file project; on an
  8-core machine it is Go's 4. Small programs never get more than Go's 4 unless they have 160+ checked files.
- `--checkers N` and `--singleThreaded` (CLI or tsconfig) win as before; build mode (`-b`, up to 4 projects at
  once) keeps Go's 4 per project.
- Cost: peak memory +1.6 GiB (+28%) on the 38k-file codebase, +0.25 GiB on vscode; counters printed by
  `--extendedDiagnostics` now depend on the machine unless `--checkers` is given. The `--checkers` help text still
  says Go's "4, unless --singleThreaded is passed" (a generated diagnostic message; not changed).

### Result: base vs this branch, default command line

`tsrs -p <project> --noEmit --incremental false --extendedDiagnostics --pretty false` without `--checkers`, base
(origin/main) vs the branch head (all three commits), interleaved, medians of 5, load 30-39:

| project | checkers base / head | wall s | check s | slowest checker CPU s | instr G | peak GiB |
| --- | --- | --- | --- | --- | --- | --- |
| 38k-file | 4 / 8 | 7.79 -> 6.54 (-16%) | 6.70 -> 5.16 (-23%) | 6.59 -> 5.12 | 392.6 -> 502.0 | 5.78 -> 7.38 (+1.60) |
| vscode | 4 / 8 | 2.56 -> 1.68 (-34%) | 2.21 -> 1.34 (-39%) | 2.21 -> 1.33 | 125.4 -> 131.2 | 2.29 -> 2.55 (+0.26) |
| webpack | 4 / 8 | 0.37 -> 0.24 (-35%) | 0.31 -> 0.18 (-42%) | 0.31 -> 0.18 | 16.7 -> 18.5 | 0.38 -> 0.45 (+0.07) |
| xstate | 4 / 7 | 0.36 -> 0.23 | 0.23 -> 0.15 | 0.20 -> 0.13 | 9.8 -> 10.6 | 0.24 -> 0.27 |
| 5-file project | 4 / 4 | 0.02 -> 0.02 | 0.00 | - | 0.4 -> 0.4 | 0.03 -> 0.03 |

Output identical in every pair. Counters: the 5-file project identical; the others equal base's counters at the same
`--checkers` (the 38k-file codebase at 8: 18,497,338 types / 20,745,093 symbols / 112,240,065 instantiations), the
same in all five runs.

Without the last commit (default stays 4) the branch changes only webpack-like projects (checked declaration files):
webpack at 4 checkers 0.36 -> 0.28 s check (table in (a)).

## Gates

Against base 89b53b5 built in a second worktree (`cargo build --release`), whole result trees compared
(`summary.json` apart from timings), branch head with all three commits:

- suite `--baselines types,symbols`, default, `TSRS_LAZY_MEMBERS=0`, and `TS_TEST_PROGRAM_SINGLE_THREADED=false`
  (4 checkers per test program, exercising the assignment with checked lib files): 13,458 error baselines pass
  (2 codes, 2 fail, as on main), 12,779 / 12,779 `.types` / `.symbols`; trees identical in all three modes.
- suite `--baselines js,jsmap,sourcemap`: 13,392 / 149 / 156 pass, trees identical.
- fourslash: 4,066 pass / 63 fail / 417 skip, result trees identical.
- `cargo test --release -p tsrs_cli` green (`api::memory_tests` cfg'd out locally: it links glibc `malloc_trim`).
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked` clean.
- The 38k-file codebase, vscode, webpack, xstate, the 5-file project: output byte-identical for k = 1, 2, 4, 6, 8,
  10, 12, 16 (base) and for the head default; counters identical across runs of the same configuration.

## Reproduce

```sh
# curve (per-checker CPU in the extendedDiagnostics output)
TSRS_ASSIGNMENT_STATS=times /usr/bin/time -l tsrs -p . --noEmit --incremental false --extendedDiagnostics --pretty false --checkers 8
# per-file wall and CPU seconds
TSRS_FILE_TIMES=ft8.tsv tsrs -p . --noEmit --incremental false --checkers 8
# assignment inputs, then a hand-made assignment (one checker index per program file)
TSRS_ASSIGNMENT_DUMP=dump tsrs -p . --noEmit --incremental false --checkers 4
TSRS_CHECKER_ASSIGNMENT=file:asg.txt tsrs -p . --noEmit --incremental false --checkers 17
# front-end instructions (no checking)
/usr/bin/time -l tsrs -p . --noEmit --incremental false --listFilesOnly > /dev/null
```

The "alone" and cold-checker experiments ran checkers one after another on one thread (a temporary switch in
`run_work_group`, not committed); the partition sweep used temporary environment overrides of
`LOCALITY_GROUP_FRACTION` and the FENNEL penalty (not committed).
