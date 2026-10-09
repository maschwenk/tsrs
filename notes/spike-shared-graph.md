# spike-shared-graph: one type graph shared by the checker threads (prototype, 2026-10-08)

The owner asked: "let's start with the graph shared between threads, at least implement it and see what the gains
are. don't worry about exactness." This note covers the prototype on branch `spike/shared-graph` (not for landing),
built with `--features shared-graph` and switched on with `TSRS_SHARED_GRAPH=1`. Sections 1-8 are from the Mac
(Apple M5 Max, 18 cores, 16 KiB pages, no THP), which other agents were also using (1-minute load 15-40); bun's
numbers there are from Linux. Section 9 has the one Linux run.

**Verdict: not pursued.** On Linux (16-vCPU runner, the README scoreboard's machine) the prototype lowers peak memory
by 5-10% at the default 8 checkers and 4-16% at 16, and makes every project 6-18% slower in wall time, because the
seed is built serially before the checkers start. That breaks the rule for this work (beat bun check on memory
without losing speed): at 8 checkers it would tie bun on cal-diy and beat it on formbricks-web, and still trail on
t3code-server, mikro-orm, supabase-studio and vscode. The code stays on the unmerged branch `spike/shared-graph`
(PR 213, closed). Diagnostics were identical in every run, on the Mac and on Linux.

## 1. Question and answer

- **Peak memory at 32 checkers** (Mac, medians of 3 interleaved runs), on vs main: t3code-server -14% (2729 -> 2354
  MiB), formbricks-web -27%, cal-diy -27%, supabase-studio -22%, xstate-main -29%, vscode -6%, drizzle-orm +2%,
  webpack -2%. With a 20 permille seed: t3code -19%, formbricks -31%, webpack -24%.
- **Per extra checker** (16 -> 32 slope, Mac): t3code 35.0 -> 24.7 MiB, formbricks 40.5 -> 17.4, cal-diy 36.2 ->
  18.9, supabase 30.9 -> 16.4, vscode 31.6 -> 26.2. bun check adds 6-15 MiB per thread on Linux (t3code about 14).
  The plan predicted 5-10 MiB saved per extra thread on t3code and formbricks. Measured: 10.3 and 23.1.
- **CPU**: at 32 checkers the forks do not redo the seed's work, so the process retires 2-30% fewer instructions than
  main on six of the eight projects (drizzle and webpack +2-3%). With one checker it retires 6-8% more (the overlay's read paths).
- **Wall** (7 interleaved runs per cell, section 4.3): +2..+12% at 4 and 16 checkers, -9..+1% at 32, with overlapping
  interquartile ranges. On this shared Mac, runs of the same binary moved up to 1.7x. The serial seed (T_w =
  0.01-0.21 s at 10 permille) is the structural wall cost, and only a Linux run can say what it costs at 0.5-1.2 s
  walls.
- **Exactness**: diagnostics are byte-identical to main in every run with the switch on, and every cell produced one
  distinct output. The runs: the matrix (eight projects x 4/16/32 checkers x 10 and 20 permille seeds, 3 runs
  each), one checker on three projects, 12 seven-run cells, and the budget curve (2.5-100 permille).

## 2. The design: frozen seed + forks

This is plan.md's A+C design ("frozen prefix" plus "seeded base"):

1. **Seed.** One checker checks a program-derived sample of files on its own thread, inside a dedicated arena
   region. The sample (`TSRS_SHARED_GRAPH_SEED=spread:<permille>`, default 10) is the lighter half of the checked
   non-declaration, non-leaf files, evenly spaced in program order up to that share of the checked weight. The pool
   creates its plain checkers meanwhile, as on main.
2. **Freeze.** The region's chunks are marked in a page bitmap and `mprotect`ed read-only
   (`TSRS_SHARED_GRAPH_PROTECT`, on by default). The seed checker is leaked as `&'static`. A missed write by a fork
   is a fault with a backtrace, never a silent race.
3. **Forks.** In the type-check pass each pool thread waits for the frozen seed and replaces its still-unused plain
   checker with `Checker::fork(seed)`. A fork:
   - copies scalars and handles;
   - reads the seed's link stores and interning maps through (its own map first, then the seed's);
   - starts its relation caches and memos empty;
   - clones the seed's diagnostics;
   - keeps whatever it would write into a frozen object in its own **overlay**.
   A fork behaves exactly like one sequential checker whose history is "the seed files, then my files". Main
   already runs arbitrary histories (stealing, `random:` assignment) and its output does not depend on them
   (notes/perf-order-independence.md). That is why the output stayed identical.

Why not the other designs: plan.md section 1. Live CAS publication (B) gives no number before about day 8, is the
only design that can hang, and gives up determinism. Bun's barrier steps need a merge with id renumbering. The frozen
seed had a measured number on day 1 through M1's emulation, and that number held up (section 4.4).

## 3. What is shared and what stays per thread

| part | shared (frozen seed, read by every fork) | per fork |
| --- | --- | --- |
| types, signatures, symbols, index infos, mappers, nodes the seed created | the objects (K_t types, B_W MiB of arena, section 4.4) | new objects (ids from K_t up) |
| lazy fields of frozen objects (47 fields: resolved members, declared members and base types, constraints and defaults, mapped-type parts, conditional branches, union/intersection tails, signature return types and predicates) | the seed's value | `OvCell`: a fork's write goes to its overlay, keyed by the cell's address. A process-wide dirty-line bitmap (one bit per 64 bytes of the frozen range) sends a read to the overlay only when some fork wrote that line. |
| lazily computed object-flag families (MembersResolved, CouldContainTypeVariables, IsGeneric*, IsUnknownLikeUnion, IsUniformEnum, IsNeverIntersection, IsConstrainedTypeVariable, IdenticalBaseType*) | the seed's bits | overlay word by type id. Only the readers of these bits use `object_flags_lazy()`; every other flag read is a plain load. |
| the other 79 lazy-looking fields (`Type.symbol_or_alias`, `TypeReference.node`, union `types`, `StructuredMembers`, signature construction fields, `IndexInfo`, `TypePredicate`, ...) | the seed's value | nothing: a discovery build found no fork writes them on a frozen object; `mprotect` would catch one |
| instantiation tables inside frozen objects (`ReferenceInstantiations`, `GoMap`, `GoPackedMap`, union/intersection property caches, lazy member and mapped tables) | the seed's table | the fork's own copy of that one table, made on its first insert or first use |
| 27 link stores | the seed's records | read-through; a record is copied into the fork's store on its first `get` |
| interning maps (union, intersection, tuple, indexed-access, union-of-union, string literal, object-type instantiations, lazy member and mapped tables) | the seed's map | the fork's own entries (`basedmap.rs`; removed seed keys are remembered) |
| relation caches, flow and inference memos, scratch pools, stacks | none | start empty |
| `OwnedCell` fields of symbols and nodes | the seed's value | reads plain. The only fork writes, `is_discriminant_property`'s cached bits, compute without caching on a frozen symbol. |

## 4. Numbers (Mac, all of them)

Binaries: main = origin/main ae64b5c. The matrix (4.1) used the prototype at 69c26fd, where it was always compiled
in; the feature build at 6c6f3b0 is the same code with the gate constant true. 4.2 and 4.3 used builds of 6c6f3b0
(default and `--features shared-graph`). Mac runs use `--noEmit --incremental
false --pretty false --extendedDiagnostics` under `/usr/bin/time -l`, interleaved. The scripts are in section 9.

### 4.1 Eight projects x 4/16/32 checkers (3 interleaved reps, medians)

Peak MiB, with the change vs main; instructions retired (the load-independent measure) vs main; wall medians are
noisy (see 4.3). "off" is the prototype compiled in with the switch off. Errors are the same in every variant, and the
diagnostics are byte-identical to main in every cell.

| project | N | peak main | on | on 20 permille | off | instr main | on | on 20 permille | wall main | on | errors (all variants) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| t3code-server | 4 | 1197 | 1154 (-4%) | 1126 (-6%) | 1199 (+0%) | 79.6 G | +4% | +1% | 1.75 | 2.47 | 6 |
| t3code-server | 16 | 2169 | 1959 (-10%) | 1855 (-14%) | 2181 (+1%) | 151.8 G | -1% | -4% | 1.99 | 1.87 | 6 |
| t3code-server | 32 | 2729 | 2354 (-14%) | 2204 (-19%) | 2722 (-0%) | 178.0 G | -4% | -10% | 1.93 | 1.78 | 6 |
| formbricks-web | 4 | 1440 | 1378 (-4%) | 1356 (-6%) | 1446 (+0%) | 68.0 G | -4% | -5% | 1.73 | 1.54 | 0 |
| formbricks-web | 16 | 1992 | 1661 (-17%) | 1577 (-21%) | 1997 (+0%) | 90.5 G | -16% | -20% | 1.11 | 0.99 | 0 |
| formbricks-web | 32 | 2640 | 1939 (-27%) | 1817 (-31%) | 2633 (-0%) | 120.3 G | -30% | -34% | 1.02 | 0.92 | 0 |
| cal-diy | 4 | 1151 | 1081 (-6%) | 1051 (-9%) | 1170 (+2%) | 61.3 G | +2% | -2% | 1.17 | 1.55 | 136 |
| cal-diy | 16 | 1889 | 1492 (-21%) | 1513 (-20%) | 1890 (+0%) | 97.8 G | -12% | -11% | 0.88 | 0.85 | 136 |
| cal-diy | 32 | 2468 | 1794 (-27%) | 1804 (-27%) | 2484 (+1%) | 120.8 G | -17% | -17% | 1.00 | 0.93 | 136 |
| supabase-studio | 4 | 1193 | 1133 (-5%) | 1129 (-5%) | 1193 (+0%) | 82.7 G | -2% | -4% | 2.00 | 1.80 | 9 |
| supabase-studio | 16 | 1699 | 1446 (-15%) | 1412 (-17%) | 1699 (+0%) | 108.8 G | -11% | -14% | 0.93 | 1.47 | 9 |
| supabase-studio | 32 | 2193 | 1709 (-22%) | 1696 (-23%) | 2192 (-0%) | 139.2 G | -20% | -24% | 1.10 | 0.97 | 9 |
| drizzle-orm | 4 | 488 | 490 (+0%) | 491 (+1%) | 488 (+0%) | 18.6 G | -1% | -0% | 0.37 | 0.50 | 10846 |
| drizzle-orm | 16 | 790 | 769 (-3%) | 777 (-2%) | 790 (+0%) | 24.1 G | +1% | +1% | 0.31 | 0.27 | 10846 |
| drizzle-orm | 32 | 1071 | 1088 (+2%) | 1060 (-1%) | 1055 (-1%) | 28.7 G | +3% | -0% | 0.29 | 0.28 | 10846 |
| vscode | 4 | 1826 | 1814 (-1%) | 1809 (-1%) | 1824 (-0%) | 108.0 G | +3% | +3% | 2.27 | 2.29 | 371 |
| vscode | 16 | 2114 | 2031 (-4%) | 2014 (-5%) | 2118 (+0%) | 115.8 G | +1% | +1% | 1.37 | 1.16 | 371 |
| vscode | 32 | 2620 | 2450 (-6%) | 2438 (-7%) | 2618 (-0%) | 123.2 G | -2% | -3% | 1.00 | 1.24 | 371 |
| webpack | 4 | 381 | 380 (-0%) | 360 (-6%) | 381 (+0%) | 15.7 G | +0% | -5% | 0.32 | 0.31 | 840 |
| webpack | 16 | 513 | 492 (-4%) | 427 (-17%) | 512 (-0%) | 19.2 G | +0% | -13% | 0.19 | 0.21 | 840 |
| webpack | 32 | 643 | 627 (-2%) | 486 (-24%) | 659 (+2%) | 23.0 G | +2% | -23% | 0.22 | 0.21 | 840 |
| xstate-main | 4 | 233 | 222 (-5%) | 222 (-5%) | 232 (-0%) | 9.7 G | -3% | -3% | 0.26 | 0.28 | 0 |
| xstate-main | 16 | 321 | 264 (-18%) | 261 (-19%) | 321 (+0%) | 12.0 G | -15% | -16% | 0.19 | 0.14 | 0 |
| xstate-main | 32 | 429 | 303 (-29%) | 295 (-31%) | 430 (+0%) | 15.4 G | -29% | -31% | 0.17 | 0.15 | 0 |

### 4.2 One checker: the design's serial overhead (instructions retired, 3 reps, medians)

| project | main | default build (prototype compiled out) | feature build, switch off | feature build, switch on |
| --- | ---: | ---: | ---: | ---: |
| t3code-server | 48.21 G | 48.16 G (-0.1%) | 50.28 G (+4.3%) | 51.91 G (+7.7%) |
| formbricks-web | 51.75 G | 51.35 G (-0.8%) | 53.36 G (+3.1%) | 54.93 G (+6.1%) |
| xstate-main | 7.63 G | 7.45 G (-2.4%) | 7.74 G (+1.5%) | 8.18 G (+7.2%) |

With the switch on and one checker, the run still seeds, freezes and forks once. The fork's cost over the switch-off
path is +2-3%: dirty-line tests, two-level lookups, overlay hits. Compiling the read paths in at all costs +1.5-4.3%,
and that is why the prototype is a cargo feature.

### 4.3 Wall, 7 interleaved reps (median, interquartile range)

Seven interleaved runs per cell, main against the feature build (6c6f3b0). "Check" is the pass, including the
wait for the seed.

| project | N | main wall s, median (IQR) | on wall s, median (IQR) | on vs main | check, on vs main | peak main | peak on | outputs |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| t3code-server | 4 | 1.43 (1.36-1.76) | 1.54 (1.39-1.68) | +8% | +9% | 1196 | 1155 | 1, = main |
| t3code-server | 16 | 1.35 (1.26-1.39) | 1.50 (1.37-1.57) | +11% | +12% | 2187 | 1973 | 1, = main |
| t3code-server | 32 | 1.64 (1.40-1.77) | 1.55 (1.37-1.89) | -5% | -6% | 2730 | 2345 | 1, = main |
| formbricks-web | 4 | 1.32 (1.16-1.48) | 1.44 (1.26-1.52) | +9% | +12% | 1443 | 1380 | 1, = main |
| formbricks-web | 16 | 0.81 (0.72-0.96) | 0.83 (0.72-1.05) | +2% | +8% | 1990 | 1651 | 1, = main |
| formbricks-web | 32 | 1.02 (0.85-1.10) | 0.93 (0.78-1.00) | -9% | -11% | 2638 | 1932 | 1, = main |
| cal-diy | 4 | 1.14 (1.01-1.17) | 1.21 (1.06-1.39) | +6% | +9% | 1151 | 1083 | 1, = main |
| cal-diy | 16 | 0.75 (0.70-0.92) | 0.79 (0.75-0.96) | +5% | +4% | 1902 | 1514 | 1, = main |
| cal-diy | 32 | 0.84 (0.79-1.01) | 0.84 (0.78-0.95) | +0% | +5% | 2497 | 1794 | 1, = main |
| vscode | 4 | 2.11 (1.90-2.44) | 2.21 (2.03-2.60) | +5% | +9% | 1823 | 1816 | 1, = main |
| vscode | 16 | 0.93 (0.84-1.14) | 0.96 (0.90-1.22) | +3% | +8% | 2116 | 2033 | 1, = main |
| vscode | 32 | 0.92 (0.84-1.00) | 0.93 (0.90-1.00) | +1% | -3% | 2618 | 2452 | 1, = main |

At 4 and 16 checkers the switch costs +2..+12% of wall and check time on the Mac: the seed's serial T_w plus the
fork's per-read overhead, with less duplicated work to win back. At 32 checkers, where duplication is largest, it is
-9..+1%. The IQRs overlap in most cells.

### 4.4 Seed size: memory, instructions and the serial seed time

M1 (emulation at 16 checkers, commit 8f139c0; handoff numbers) predicted, at 10 permille, 11.9 MiB saved per extra
thread on t3code and 11.8 on formbricks. The real forks measured at 16 -> 32: 10.3 and 23.1.

Seed per project at 10 permille (16 checkers, feature build):

| project | seed files | K_t types | B_W (seed arena) | T_w (serial seed) | dirty 64-byte lines |
| --- | ---: | ---: | ---: | ---: | ---: |
| t3code-server | 55 | 69,669 | 15.5 MiB | 0.10 s | 6,418 |
| formbricks-web | 109 | 344,577 | 35.7 MiB | 0.18 s | 9,550 |
| cal-diy | 150 | 115,669 | 22.2 MiB | 0.15 s | 11,459 |
| supabase-studio | 151 | 114,185 | 23.1 MiB | 0.21 s | 12,513 |
| drizzle-orm | 29 | 19,548 | 8.2 MiB | 0.03 s | 1,562 |
| vscode | 367 | 56,537 | 11.3 MiB | 0.10 s | 15,882 |
| webpack | 51 | 9,264 | 2.4 MiB | 0.01 s | 1,689 |
| xstate-main | 7 | 19,785 | 3.9 MiB | 0.02 s | 1,641 |

The budget curve (commit 62bfdd0, before the two-level maps; 2 reps, medians; walls omitted as noise):

| project | N | seed permille | seed files | K_t | B_W MiB | T_w s | peak MiB | instr G |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| t3code-server | 32 | 2.5 | 15 | 47,921 | 10.6 | 0.07 | 2467 | 175.6 |
| t3code-server | 32 | 10 | 55 | 69,669 | 15.5 | 0.09 | 2373 | 168.1 |
| t3code-server | 32 | 20 | 113 | 109,891 | 24.2 | 0.15 | 2281 | 159.5 |
| t3code-server | 32 | 50 | 277 | 225,637 | 51.2 | 0.44 | 2210 | 147.5 |
| t3code-server | 32 | 100 | 591 | 306,511 | 69.3 | 0.55 | 2074 | 136.4 |
| formbricks-web | 32 | 2.5 | 26 | 27,344 | 6.1 | 0.04 | 2365 | 115.1 |
| formbricks-web | 32 | 10 | 109 | 344,577 | 35.7 | 0.18 | 2027 | 84.3 |
| formbricks-web | 32 | 20 | 218 | 386,821 | 43.7 | 0.28 | 1924 | 78.3 |
| formbricks-web | 32 | 50 | 539 | 461,718 | 57.4 | 0.47 | 1974 | 75.1 |
| formbricks-web | 32 | 100 | 1127 | 567,126 | 83.7 | 0.66 | 1952 | 68.0 |
| cal-diy | 32 | 2.5 | 35 | 49,243 | 9.4 | 0.07 | 2116 | 117.7 |
| cal-diy | 32 | 10 | 150 | 115,669 | 22.2 | 0.21 | 1850 | 98.9 |
| cal-diy | 32 | 20 | 289 | 127,514 | 24.8 | 0.18 | 1884 | 100.9 |
| cal-diy | 32 | 50 | 725 | 291,958 | 50.6 | 0.42 | 1856 | 90.5 |
| cal-diy | 32 | 100 | 1447 | 352,056 | 64.3 | 0.59 | 1658 | 80.1 |

(At 16 checkers the same shape: t3code 2066 / 1991 / 1896 / 1888 / 1710 MiB, formbricks 1858 / 1695 / 1626 / 1638 /
1607, cal-diy 1695 / 1538 / 1546 / 1483 / 1424 at 2.5 / 10 / 20 / 50 / 100 permille. Main at 32: t3code 2729,
formbricks 2640, cal-diy 2468.)

A larger seed keeps saving memory and instructions. The saving flattens past 20 permille on formbricks, and T_w
grows linearly. The default stays at 10 permille, and 20 permille is the measured alternative in 4.1.
`TSRS_SHARED_GRAPH_OVERLAP=k` lets k checkers start at once as share-nothing checkers while the seed runs. At k = 2
it showed no wall gain at 32 checkers on the Mac and cost about 50 MiB. Default 0.

### 4.5 Against bun check (bun: Linux, 64 threads; tsrs main: Linux, 32 checkers; prototype: Mac)

| project | MiB per extra checker, Mac 16 -> 32: main | on | on 20 permille | bun check, Linux | Linux peak: tsrs main 32 checkers | bun 64 threads | Mac on / main at 32 | Linux main x Mac ratio (extrapolation) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| t3code-server | 35.0 | 24.7 | 21.8 | 14 | 2.73 GiB | 1.59 GiB | 0.86 | 2.35 GiB |
| formbricks-web | 40.5 | 17.4 | 15.0 | 6-15 | 2.82 | 2.22 | 0.73 | 2.06 |
| cal-diy | 36.2 | 18.9 | 18.2 | 6-15 | 2.50 | 2.01 | 0.73 | 1.82 |
| supabase-studio | 30.9 | 16.4 | 17.8 | 6-15 | 2.24 | 2.14 | 0.78 | 1.75 |
| drizzle-orm | 17.6 | 19.9 | 17.7 | 6-15 | 1.08 | 1.01 | 1.02 | 1.10 |
| vscode | 31.6 | 26.2 | 26.5 | 6-15 | 2.62 | 2.88 | 0.94 | 2.46 |
| webpack | 8.1 | 8.4 | 3.7 | | 701 MiB | 775 MiB | 0.98 | 687 MiB |
| xstate-main | 6.8 | 2.4 | 2.1 | | 312 MiB | 612 MiB | 0.71 | 222 MiB |

drizzle-orm and webpack gain nothing at 10 permille. Their seeds are small (8.2 and 2.4 MiB), and drizzle's checkers
each instantiate their own schema types. webpack at 20 permille saves 24%.

Linux peaks are from bench/results/2026-10-08-e80e7764f8d0.md (Bun 1.4.3-canary.1+5749c3129, tsrs e80e776, 10
reps). The per-thread slopes for bun are notes/mem-round4.md section 1 (bench-compare 4gv48c0n96). If the Mac ratio
on/main at 32 holds on Linux, tsrs at 32 checkers would reach about t3code 2.35 GiB (bun 1.59), formbricks 2.06
(bun 2.22), cal-diy 1.82 (bun 2.01), supabase 1.75 (bun 2.14), drizzle 1.10 (bun 1.01). That is an extrapolation,
not a measurement. Linux has 4 KiB pages and THP, and the slack and residency behave differently there
(notes/mem-linux-residency-32.md).

## 5. Exactness and determinism

- **Diagnostics vs main**: 0 differing lines in every cell measured. That covers the matrix (eight projects x
  4/16/32, 10 and 20 permille seeds), the single-checker runs, the 7-rep wall runs and the budget curve (2.5-100
  permille). No union-order, recursion-cutoff or circularity-location difference appeared. The history argument held
  on these projects. The conformance suite was not run with the switch on.
- **Determinism**: every cell produced one distinct output: 96 matrix cells x 3 reps and the 7-rep cells at 16 and
  32 checkers on t3code, formbricks, cal-diy and vscode. The seed is a function of the program, so the only
  run-to-run variation is the stealing order, as on main.
- **Waits and deadlocks**: none by construction. A fork thread waits once, on a `OnceLock`, for the frozen seed.
  There are no locks or condition variables on the check path, and no thread ever waits for a value another fork
  computes.
- **`mprotect` faults fixed during development** (each named its field by backtrace):
  - symbol ids written into frozen symbols (round 1: the seed assigns ids eagerly);
  - `LazyMappedTable` borrow flags (cloned unguarded);
  - reads without an overlay on the main thread after the pass;
  - `is_discriminant_property`'s cached check flags (compute, don't cache);
  - `EvolvingArrayType.final_array_type`, reached only with a 20 permille seed (back to `OvCell`).
  No fault in the final runs: the eight projects at 4/16/32 checkers with 10 and 20 permille seeds, at 8 checkers
  with 50 and 100 permille, and t3code, formbricks and cal-diy at 16/32 with 2.5-100 permille.
- `TSRS_ARENA_POISON=1` (alloc-profile build): t3code-server and formbricks-web at 16 checkers with the switch on: diagnostics identical to main and
  to a `TSRS_FREE_LEAVES=0` run, no poisoned read (a fork never reaches a freed leaf through the seed: the seed never
  checks a leaf file).

## 6. Costs inside the design

- **Single-checker instructions** +6-8% over main (4.2).
- **Serial seed** T_w 0.01-0.21 s at 10 permille, during which every pool thread waits.
- **Overlay**: 2-5% of the seed's 64-byte lines become dirty. Reads of clean lines cost a range check and a bit
  test. At 32 checkers a fork holds 1.9-2.8 K overlay cells on average (max 5.1 K) and about 140 pages of
  object-flag words (2 KiB each). That is about 0.3 MiB per fork, 8.5-8.8 MiB over 32 forks on t3code, formbricks,
  cal-diy and vscode.
- **Copies**: a link record is copied on a fork's first `get`. A frozen instantiation table is copied on the fork's
  first insert into it, and a frozen lazy mapped table on first use.
- **What a fork still duplicates**: everything its own files create. That includes library generics instantiated
  with project types (54% of t3code's duplicated types, notes/mem-per-checker-duplication.md), which no seed can
  predict. This is why t3code keeps 24.7 MiB per extra checker against bun's ~14.

## 7. What a landing would still need (estimates; not pursued, see the verdict)

- **The landing gates with the switch on** (common5.md): conformance in canonical mode and with
  `TSRS_LAZY_MEMBERS=0`, fourslash, the determinism CI with random assignments plus the seed. 1-2 days if they pass,
  open-ended if they find history-dependent output the bench projects do not exercise.
- **Linux numbers** at the default count on pr-verify, against the 5%-of-peak gain bar and the 2% wall bar. Half a
  day plus the runner time in section 9. The wall answer is the open question: T_w is serial, and Linux walls at 32
  checkers are 0.5-1.2 s.
- **Shrinking T_w**: seed the files the pool would check first, or seed while the parse finishes. 2-4 days.
  Otherwise the default budget must be set per machine.
- **Removing the +6-8% single-checker cost**: a front cache or per-fork dirty bits in a register-friendly place.
  2-3 days, or keep the feature off below some checker count.
- **The LSP/API**: today the switch is ignored outside the CLI's type-check pass. The language server would need the
  frozen region to outlive edits. Weeks.
- **Code**: about 2,400 added lines over 32 files, including a hand-written 345-field `Checker::fork`, which every new
  checker field must keep up with. A derive or a generated list. 1 day.
- **Safety**: `mprotect` as a debug and CI mode, plus the discovery build's field list as a test. 1 day.
- **Total**: about two weeks to a landable PR, if the Linux wall answer is acceptable.

## 8. Rejected or not built, and why

- **Live CAS publication (design B) first**: no memory number until about day 8, the only design that can hang, and
  nondeterministic. It remains the growth path for instantiations created during checking (plan.md M5(b)).
- **Republish by rebase (stage 2, plan M5(a))**: not built. The residual is not where a later seed would help most
  (t3code's Effect generics, section 6).
- **Cloning interning maps into every fork** (round 1): 1.5-3.5 MB per fork. Replaced by two-level maps
  (formbricks -6% at 32 checkers).
- **`OwnedCell` through the overlay** (round 1): +22% instructions with the switch off, because it holds the node
  header, flags and loc. Replaced by plain reads plus compute-without-caching at the one writing site.
- **Object flags through the overlay on every read**: +7% instructions. Replaced by `object_flags_lazy()` at the
  ~70 sites that read the lazily computed families.
- **The seed overlap with share-nothing checkers** (`TSRS_SHARED_GRAPH_OVERLAP`): kept as a knob, default 0, with no
  measured gain.
- **A front cache** (plan M3): not built. The switch-on fork costs +2-3% over switch-off, below the plan's +5%
  threshold for it.

## 9. Reproduce

```sh
git checkout spike/shared-graph
cargo build --release --locked -p tsrs_cli --features shared-graph   # the prototype
cargo build --release --locked -p tsrs_cli                           # default build: prototype compiled out
TSRS_SHARED_GRAPH=1 [TSRS_SHARED_GRAPH_SEED=spread:20] [TSRS_SHARED_GRAPH_STATS=1] [TSRS_HEAP_CENSUS=1] \
  target/release/tsrs -p <project> --noEmit --checkers 32 --extendedDiagnostics
```

Other switches: `TSRS_SHARED_GRAPH=emulate` (M1's prefix emulation, any build); `TSRS_SHARED_GRAPH_PROTECT=0|log`;
`TSRS_SHARED_GRAPH_OVERLAP=<k>`; `TSRS_SHARED_GRAPH_LOG_OWNED=1` (stacks of fork writes to frozen `OwnedCell`s and
object flags).

The Mac scripts (sgrun.py: one run with a timeout and a load wait; measure.py: interleaved matrix; analyze.py:
tables) were in the session's scratch directory. The commands, interleaved, used `/usr/bin/time -l`.

**Linux probe.** The branch was rebased onto main 6df09dbe on 2026-10-08: the wasm build (#203), #212 (a fork now
carries the seed's deferred type-argument checks, a5d2a57) and #209 (right-sized runners). The commit ids in
sections 4-5 are from before the rebase; the measured code is the same. After the rebase:
- the switch-off identity holds (xstate-main and webpack at 1 and 4 checkers, byte-identical to a main build of
  6df09dbe);
- with the switch on at 16 checkers, xstate-main and t3code-server match main;
- mikro-orm matches main at 8 and 16 checkers (Mac peak -10% and -15%, one run each).

Result (Depot run 413806b8q7, `depot-ubuntu-24.04-16`, spike head 0fa5258, off = the default build, on = the
prototype with `TSRS_SHARED_GRAPH=1`, 5 interleaved reps, medians; error counts identical in every run). The whole
dispatch took about 6 runner-minutes, not the 25-30 estimated below.

| project | checkers | peak off | peak on | peak | wall off | wall on | wall | user CPU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| t3code-server | 8 | 1802 MiB | 1710 MiB | -5.1% | 2.10 s | 2.26 s | +7.5% | +1.1% |
| t3code-server | 16 | 2181 | 1987 | -8.9% | 1.79 | 1.90 | +6.0% | -7.3% |
| formbricks-web | 8 | 1710 | 1546 | -9.6% | 1.01 | 1.14 | +12.8% | -8.0% |
| formbricks-web | 16 | 2007 | 1679 | -16.3% | 0.81 | 0.95 | +18.3% | -15.8% |
| cal-diy | 8 | 1454 | 1311 | -9.9% | 1.02 | 1.11 | +8.4% | -3.4% |
| cal-diy | 16 | 1925 | 1628 | -15.4% | 0.83 | 0.93 | +11.8% | -11.2% |
| supabase-studio | 8 | 1413 | 1280 | -9.4% | 1.22 | 1.32 | +8.6% | -6.4% |
| supabase-studio | 16 | 1720 | 1464 | -14.9% | 0.86 | 0.99 | +14.5% | -15.9% |
| mikro-orm | 8 | 1649 | 1495 | -9.3% | 1.51 | 1.69 | +11.7% | -2.8% |
| mikro-orm | 16 | 1987 | 1679 | -15.5% | 1.15 | 1.30 | +13.0% | -12.7% |
| vscode | 8 | 1987 | 1946 | -2.1% | 1.81 | 1.95 | +7.2% | +1.9% |
| vscode | 16 | 2140 | 2058 | -3.8% | 1.12 | 1.25 | +11.4% | +0.8% |

User CPU falls at 16 checkers (the forks do not redo the seed's work) while wall rises at both counts: the seed is a
serial phase on the critical path. Shrinking it (section 7, "Shrinking T_w") is the only lever left, and the wall
cost would have to fall below the 2% bar while the memory gain at 8 checkers is 5-10%. Against bun on this machine
(bench of 4a3c1877, peak tsrs / bun at the defaults): formbricks-web 1.68 / 1.61 GiB, cal-diy 1.44 / 1.29,
supabase-studio 1.38 / 1.21, mikro-orm 1.60 / 1.41, vscode 1.94 / 1.86, t3code-server 1.77 / 1.11. Applying the
8-checker savings, only formbricks-web and cal-diy reach bun.

The README scoreboard is now the 16-vCPU machine's default mode (bench.yml `measure-wide` on
`depot-ubuntu-24.04-16`). There tsrs runs 8 checkers (half the cores, at least 8), and the bench adds a
`--checkers 16` table. bun check uses less memory there on t3code-server (0.63x), mikro-orm (0.87x),
supabase-studio (0.88x), cal-diy (0.91x), formbricks-web (0.96x) and vscode (0.97x)
(bench/results/2026-10-08-4a3c1877fa21.md).

`tools/perf/sharedprobe.sh` keeps a copy of the workflow's default build (the prototype compiled out) and builds
the prototype in the same target directory. It then runs `tools/perf/leafprobe.py` with off and on interleaved, at
8 checkers (the default on 16 cores) and 16. One dispatch covers the six projects:

```sh
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref spike/shared-graph \
  --input runner=depot-ubuntu-24.04-16 \
  --input projects=t3code-server,formbricks-web,cal-diy,supabase-studio,mikro-orm,vscode \
  --input script=tools/perf/sharedprobe.sh --input probe_args='--checkers 8,16 --reps 5 --no-strace --no-perf-stat'
```

Estimated cost on `depot-ubuntu-24.04-16`:
- runner setup and cache restores, about 5 minutes;
- cal-diy and mikro-orm cloned and installed, about 5 minutes;
- the default build, about 8 minutes (rust-cache is cold for this runner's key);
- the prototype build, about 6 minutes (the dependencies are reused);
- 132 timed runs of 0.4-2 s, about 3 minutes.

That is about 25-30 runner-minutes on the 16-vCPU machine. Two dispatches, split by project, would build twice and
cost about 40 minutes in total. The 32-vCPU runner is not needed: neither the scoreboard nor the bench runs more
than 16 checkers now, and 8 -> 16 gives the per-checker slope.
