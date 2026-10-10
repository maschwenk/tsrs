# spike-shared-graph: one type graph shared by the checker threads (2026-10-08, updated 2026-10-10)

Status (2026-10-10): sections 1-9 are the first round, without a memory target and with the pool waiting for the
seed. Section 10 is the version proposed for landing (PR #281): built with `--features shared-graph`, on with
`TSRS_SHARED_GRAPH=1`, and meant for `--maxMemory`, where a retired checker is replaced by a fork of the seed. That
version keeps two knobs, `TSRS_SHARED_GRAPH_SEED_PERCENT` (section 10.2; the first round's
`TSRS_SHARED_GRAPH_SEED` was in permille, so its 10 is 1%) and `TSRS_SHARED_GRAPH_PROTECT=0`, plus
`TSRS_DEBUG_REGIONS=1` for debugging. It removed the switches the first round used for measurement and discovery,
which sections 1-9 still name: `TSRS_SHARED_GRAPH=emulate`, the `spread:`/`files:` seed rules,
`TSRS_SHARED_GRAPH_OVERLAP`, `_STATS`, `_LOG_OWNED`, `_LOG_OVERRIDES`, `_WAIT`, protect mode `log`, and
`tools/perf/sharedprobe.sh`. They are in the git history of PR #281 (b0a4a6e0).

The owner asked: "let's start with the graph shared between threads, at least implement it and see what the gains
are. don't worry about exactness." This note covers the prototype on branch `spike/shared-graph` (not for landing),
built with `--features shared-graph` and switched on with `TSRS_SHARED_GRAPH=1`. **Every number is from the Mac**
(Apple M5 Max, 18 cores, 16 KiB pages, no THP), which other agents were also using (1-minute load 15-40). Depot was
not used (the owner's spend). Section 9 gives the Linux probe to run later, with its cost. Bun's numbers are from
Linux.

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
   region. The sample (`TSRS_SHARED_GRAPH_SEED=<permille>`, default 10) is the lighter half of the checked
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
it showed no wall gain at 32 checkers on the Mac and cost about 50 MiB. Default 0 (removed; section 10.1's pool, which
does not wait for the seed, replaces it under `--maxMemory`).

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

## 7. What a landing would still need (estimates)

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
- **The seed overlap with share-nothing checkers** (`TSRS_SHARED_GRAPH_OVERLAP`): no
  measured gain; removed in section 10.
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

**Linux probe, not run (the owner decides on the Depot spend).** The branch was rebased onto main 6df09dbe on
2026-10-08: the wasm build (#203), #212 (a fork now carries the seed's deferred type-argument checks, a5d2a57) and
#209 (right-sized runners). The commit ids in sections 4-5 are from before the rebase; the measured code is the
same. After the rebase:
- the switch-off identity holds (xstate-main and webpack at 1 and 4 checkers, byte-identical to a main build of
  6df09dbe);
- with the switch on at 16 checkers, xstate-main and t3code-server match main;
- mikro-orm matches main at 8 and 16 checkers (Mac peak -10% and -15%, one run each).

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

## 10. Update (2026-10-09): on top of `--maxMemory`, the 38k-file codebase

This round put the first round's code on top of `--maxMemory` (PR 265; 55 commits of main since the first round). It is
PR #281. With both on, a checker that `--maxMemory` retires is replaced by a fork of the frozen seed instead
of a fresh checker, so the replacement starts with the seed's graph instead of rebuilding it.

What the merge needed:

- Main's new `Checker` fields (`too_complex_*`, `tuple_elements`): a fork continues the seed's values.
- `escape_mapper` / `recycle_mapper` / `InferenceContext::escape` (main's mapper recycling): a frozen mapper or context
  counts as escaped, so a fork never writes its escape bit or frees it (a `mprotect` fault on the 38k-file codebase).
- Seed files exclude every file with a region of its own (`fileregions::has_region`): the seed used to start before
  the leaves were classified (`is_check_leaf` was false for all), so it could check a leaf and freeze objects that point
  into its tree, which is freed once checked. The seed now starts with the type-check pass, after `classify`, so the
  leaf guard (`is_unreadable_check_leaf`) also keeps it from reading other leaves.
- A retired fork's overlay is entered again while the retirement collects its global diagnostics, and a new fork
  leaves its own overlay current.
- Tried and removed: starting the pool without waiting for the seed (plain checkers, switched to forks once the seed
  is frozen, or replaced by forks when retired). It hid the serial seed (29.1 s against 33.6 s at 8G) but plain
  checkers running beside forks after the freeze crashed in 1-3 of 10 runs (reads of retired regions) that this round
  did not explain; entering the seed region as a scratch region (so that escaping data leaves it) made forks share
  unfrozen seed data and crashed every run. The pool waited for the seed, as in the first round (section 10.1 found
  both causes of the crashes and starts the pool at once).

Measured (Mac, 14 cores, 8 checkers, release build with `--features shared-graph`; diagnostics byte-identical in every
run; 20 stress runs at 10 and 50 permille with `--maxMemory 8G`, 12 more over seeds 10-100 and targets 8-10G, and every
testdata/regressions case with a 30% seed and a retirement after nearly every file: no failure):

| | peak | instructions | user CPU | wall | serial seed |
| --- | ---: | ---: | ---: | ---: | ---: |
| main (no feature), no target | 18.0 GB | 2.277 T | 187 s | 26.0 s | |
| feature compiled in, switch off | 18.0 GB | 2.358 T (+3.6%) | 196 s | 27.8 s | |
| seed 50 permille, no target | 16.5 GB | 2.290 T | 193 s | 32.5 s | 5.9 s |
| `--maxMemory 8G`, no seed | 8.72 GB | 3.195 T | 242 s | 32.8 s | |
| `--maxMemory 8G`, seed 10 permille | 8.84 GB | 2.687 T | 205 s | 30.8 s | 2.6 s |
| `--maxMemory 8G`, seed 20 permille | 8.95 GB | 2.664 T | 206 s | 31.8 s | 3.9 s |
| `--maxMemory 8G`, seed 50 permille | 8.87 GB | 2.524 T | 202 s | 33.0 s | 5.9 s |
| `--maxMemory 9G`, seed 20 permille | 9.69 GB | 2.586 T | 200 s | 30.9 s | |
| `--maxMemory 10G`, seed 20 permille | 10.83 GB | 2.520 T | 196 s | 30.4 s | |

Under a memory target the seed removes most of the rebuild cost of retirements (-16 to -21% instructions, -15% user
CPU at 8G) and costs its own size in memory (238-476 MiB, shared). Wall improves little (-6% at best) because the pool
waits for the serial seed, 2.6-5.9 s on this program. The design is sound and was exact everywhere; a rewrite would
need the same overlay, frozen-region and fork machinery. What is left: making the seed cost no wall time (seed while
the front end finishes, or let the pool start safely before the freeze), and the +3.6% the feature costs compiled in.

### 10.1 The wall time: two races fixed, the pool no longer waits for the seed

The seed is serial and every pool thread waited for it (2.6-5.9 s on the 38k-file codebase), which ate most of the
seed's gain. Starting the pool at once (plain checkers, each retired for a fork at its first file boundary after the
freeze) crashed in 1-3 of 10 runs. To find out why, `TSRS_DEBUG_REGIONS=1` retires checker regions for good (pages given
back, addresses never reused, chunks logged) so that a stale pointer faults at an address that names its region, and
the fault handler prints the address and the thread; registers mapped to regions under lldb showed the two causes:

- **`freeze` published its state with Relaxed stores** ("published to the forks by spawning their threads after the
  freeze"). A checker running during the freeze could see the frozen span before the dirty bitmap and dereference a
  null bitmap. The span is now stored with Release after the start and both bitmaps, and `dirty` and `is_frozen_addr`
  load it with Acquire.
- **The last, partial 4 KiB page of each seed chunk was not marked frozen** (the page bitmap covered only whole pages
  inside a chunk, and seed chunk sizes are not page multiples). Seed objects there read as unfrozen, so a fork's lazily
  filled fields went inline into memory every fork shares: one fork's pointer became every fork's, and dangled once that
  fork was retired (the successor fork on the same thread faulted on its predecessor's region, through a seed object at
  offset 0x1e84568 of a 0x1e84800-byte chunk). The bitmap now covers every page a chunk touches; each seed chunk has a
  slab of its own rounded to whole pages, so no other object shares those pages. This one also affects the waiting
  pool, more rarely.

After both: no failure in 10 runs with regions retired for good, 16 runs over seeds 10-100 and targets 8-10G in both
modes, and the regression cases with a 30% seed and a retirement after nearly every file. Under `--maxMemory` the pool
now starts at once. Without a target it still waits for the seed and forks each unused checker.

3 interleaved runs (medians):

| | wall | peak | instructions |
| --- | ---: | ---: | ---: |
| main, no target | 25.6 s | 18.07 GB | 2.273 T |
| `--maxMemory 8G`, no seed | 32.9 s | 8.73 GB | 3.168 T |
| `--maxMemory 8G`, seed 50 permille, pool does not wait | 29.1 s | 8.68 GB | 2.785 T |
| `--maxMemory 9G`, seed 20 permille, pool does not wait | 27.9 s | 10.10 GB | 2.688 T |

(One no-seed 8G run peaked at 9.46 GB: the target is approached from above, one retirement at a time.) Staying under 9
GB now costs +14% wall against +29% without the seed. What remains of the cost: plain checkers retired at the freeze
(their state is rebuilt by the forks), the forks' own rebuilds after each retirement, and the +3.6% the feature costs
compiled in.

### 10.2 Seed size under `--maxMemory` (2026-10-10)

The first round chose 10 permille (1%) while every checker waited for the seed: a larger seed saved more, but its
serial time grew with it. With the pool no longer waiting under `--maxMemory`, that cost is gone and the trade is
different: a larger seed means fewer rebuilds after retirements, but a later freeze, and the plain checkers' work up
to the freeze is discarded when they become forks. Sweep at commit 47cac77d (38k-file codebase, 8 checkers,
`--maxMemory 8G`, 3 interleaved runs, medians, diagnostics identical in all 15):

| seed | wall | user CPU | instructions | peak |
| --- | ---: | ---: | ---: | ---: |
| 1% | 29.56 s | 217.7 s | 2.804 T | 8.63 GB |
| 2% | 29.13 s | 216.1 s | 2.785 T | 8.67 GB |
| 5% | 28.59 s | 214.1 s | 2.764 T | 8.83 GB |
| 10% | 28.90 s | 220.0 s | 2.803 T | 8.87 GB |
| 20% | 28.86 s | 221.3 s | 2.819 T | 8.95 GB |

5% is best on wall (-3.3%), CPU and instructions (-1.4%) against 1%, for +0.2 GB peak (the seed is frozen, so it
counts toward the target but is never retired). Past 5% the discarded plain-checker work outweighs the saved
rebuilds. The default is now 5% with `--maxMemory` and stays 1% without, and the knob is a percent
(`TSRS_SHARED_GRAPH_SEED_PERCENT`, decimals allowed) instead of permille. One program only: the bench projects were
not swept in this mode.
