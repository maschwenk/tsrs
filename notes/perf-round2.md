# perf-round2: #39 on compressed pointers, and the round's total

Branch `perf/round2` (2026-10-04): the integration of #37, #40, #38 and #39; by the end its diff against main was #39
(`mem/scoped-arenas`) adapted to compressed pointers, and it landed in its place. This note keeps the round's total
against the pre-round main 12a8acd, #39's own delta on the new main, and what compression changes for #39. The
conflict log, merge order, gate runs and reproduce steps were removed on 2026-10-10 (all merged; the gates passed
with output identical to main in both pointer representations).

## Round total: origin/main 21b95e9 + #39 against 12a8acd

The 38k-file codebase, `tsrs -p . --noEmit --extendedDiagnostics --pretty false`. 12a8acd = main before the round
(same code as ff92b7f, the base every round-2 branch measured against); r2 = this branch. 5 interleaved rounds (order
reversed every other round), medians with ranges; peak = maximum resident set size; instructions retired from
`/usr/bin/time -l`. 1-minute load 5-42 (other agents and sessions; the script waited up to 3 minutes for load < 8
before each run, then ran anyway, never above 40 for the warm rounds), so wall and check times carry that noise;
peaks and instructions do not.

| run | metric | 12a8acd (pre-round) | r2 (main 21b95e9 + #39) | change |
| --- | --- | --- | --- | --- |
| cold, 1 checker | wall s | 17.44 (16.68-24.95) | 17.51 (16.20-26.38) | noise |
| | check s | 16.57 (15.67-23.68) | 16.66 (15.41-24.78) | noise |
| | peak GiB | 5.060 (4.937-5.071) | **4.250** (4.246-4.253) | **-16.0%** |
| | instructions | 296.7 G (295.6-300.1) | 292.4 G (291.0-296.1) | -1.4% |
| cold, 4 checkers | wall s | 7.80 (7.14-9.12) | 7.29 (6.70-15.23) | -7% (ranges overlap) |
| | check s | 6.36 (6.26-8.01) | 6.38 (5.89-10.50) | noise |
| | peak GiB | 6.706 (6.699-6.712) | **5.679** (5.669-5.691) | **-15.3%** |
| | instructions | 402.9 G (401.1-409.5) | 395.0 G (393.1-414.4) | -2.0% |
| cold, 8 checkers | wall s | 6.86 (6.39-9.74) | 6.32 (6.10-6.44) | -8% (ranges barely overlap) |
| | check s | 5.78 (5.50-6.39) | 5.36 (4.93-5.52) | -7% (ranges barely overlap) |
| | peak GiB | 8.539 (8.523-8.539) | **7.214** (7.204-7.220) | **-15.5%** |
| | instructions | 515.7 G (514.8-522.5) | 503.3 G (502.4-510.0) | -2.4% |
| `--noCheck` | peak GiB | 1.859 (1.858-1.868) | 1.723 (1.720-1.729) | -7.3% |
| warm incremental, no edit (writes tsbuildinfo) | wall s | 7.18 (6.86-9.69) | **1.45** (1.21-1.78) | **-80%** |
| | peak GiB | 3.524 (3.500-3.544) | 2.908 (2.845-2.940) | -17.5% |
| warm incremental, one leaf edit | wall s | 9.32 (7.76-19.38) | **1.47** (1.20-2.01) | **-84%** |
| | peak GiB | 3.978 (3.947-4.015) | 2.881 (2.859-2.961) | -27.6% |
| language server, 40 edits (start / end RSS) | MiB | 2,411 / 3,515 | 2,210 / 3,057 | -8.3% / -13.0% |

Warm runs: a fresh seed per round (a cold `--incremental` run), then the warm run (which rewrites the tsbuildinfo),
then a leaf edit that nothing imports. Counters (Types, Symbols, Instantiations) and diagnostics (0 errors) are
identical between the binaries in every run that saw the same files. Language server: `tools/lsp-mem/lsp_mem.py
--edits 40 --every 10`, 5 interleaved rounds, medians.

What each piece contributed (each PR's own notes, against ff92b7f, which is 12a8acd's code):

| PR | what | 1 checker | 4 checkers | warm / leaf |
| --- | --- | --- | --- | --- |
| #37 incremental-parallel | parallel changes compute, faster tsbuildinfo write | cold unaffected | cold unaffected | 8.60 -> 1.75 s / 8.99 -> 3.34 s |
| #41 dev-loop | loader parse-ahead, `SyncMap` behind an `RwLock`, referenced-by without inverting the map | | | leaf edit check 1.44 -> 0.02 s here (12a8acd vs main) |
| #40 checker-cpu2 | checker instructions | -6.9% instr, peak 0 | -6.5% instr, peak 0 | |
| #38 pointer compression | `P<T>` a 32-bit handle | peak -14.9%, instr +3.9% | peak -14.3%, instr +5.3% | |
| #39 scoped frees (this PR) | proven-dead inference / mapper frees | peak -1.5% | peak -1.6% | |
| sum of the rows | | peak -16.4%, instr -3.0% | peak -15.9%, instr -1.2% | |
| **measured, r2 vs 12a8acd** | | **peak -16.0%, instr -1.4%** | **peak -15.3%, instr -2.0%** | 7.18 -> 1.45 s / 9.32 -> 1.47 s |

The peaks add up (within 0.6 points). Instructions: #40's saving and #38's cost combine to -1.4% / -2.0% rather than
the -3.0% / -1.2% the separate rows suggest; on plain pointers the same tree is -6.1% / -6.8% against 12a8acd
(earlier integration run below), so compression costs +4.9% instructions on top of #40 in the combined tree on both
checker counts. Wall time on 1 and 4 checkers is within the noise of this machine tonight; on 8 checkers the
median is 8% lower and the ranges barely overlap (6.39-9.74 vs 6.10-6.44 s), which is suggestive, not proven.

## #39 on the new main

Same runs, main 21b95e9 against r2 (= main + #39), both representations (plain = `--features tsrs_core/plain-ptrs`,
cold runs only):

| run | main peak GiB | r2 peak GiB | change | main instructions | r2 instructions |
| --- | --- | --- | --- | --- | --- |
| 1 checker, compressed | 4.316 (4.309-4.342) | 4.250 (4.246-4.253) | -1.5% | 291.4 G (290.6-292.2) | 292.4 G (291.0-296.1), +0.3% (noise) |
| 4 checkers, compressed | 5.762 (5.747-5.765) | 5.679 (5.669-5.691) | -1.4% | 394.5 G (393.0-399.1) | 395.0 G (393.1-414.4), +0.1% |
| 8 checkers, compressed | 7.321 (7.309-7.329) | 7.214 (7.204-7.220) | -1.5% | 501.8 G (501.5-509.7) | 503.3 G (502.4-510.0), +0.3% |
| 1 checker, plain | 5.063 (5.053-5.081) | 4.982 (4.971-4.987) | -1.6% | 278.5 G (277.3-279.8) | 278.0 G (277.4-285.0) |
| 4 checkers, plain | 6.725 (6.712-6.736) | 6.609 (6.593-6.618) | -1.7% | 375.1 G (373.8-380.0) | 378.4 G (374.3-381.1), ranges overlap |
| language server end RSS | 3,042 MiB (2,990-3,057) | 3,057 MiB (3,015-3,074) | none (ranges overlap) | | |

#39's frees are worth what they were on plain pointers (-1.5% there): -1.4 to -1.5% peak with compressed pointers,
-1.6 to -1.7% plain, instructions within noise. Above the 0.5% bar, so the frees stay (no split into tooling only).
The language server does not gain: its checker allocates in per-update regions that are dropped whole anyway.

One thing compression does to #39: `free_slice!` of a `[P<Type>]` with an odd number of elements (4n bytes, not a
multiple of 8) is a no-op (`free_class` returns 0), so `getTailRecursionRoot`'s one-element type-argument lists are
neither reused nor poisoned in the compressed build. Safe (nothing live is freed), only less reuse; main's existing
recycled slices have the same property since #38. Padding slices to 8 would reclaim them at 4 bytes per odd list;
not done here.

## The poison check under compression

`check_not_freed` (alloc-profile builds with `TSRS_ARENA_POISON=1`: a `P` dereference panics if the target starts with
eight poison bytes) is in both the plain and the compressed `P::get` / `Deref` (crates/tsrs_core/src/ptr.rs). In the
plain build its guard is `size_of_val >= 8 && align_of_val >= 8`; a compressed handle always names an 8-aligned block
(`p_layout` aligns and pads every `P` target to 8), so there the guard is `size >= 8` only, which also covers the
4-aligned types that most checker structs became once their fields are handles. A positive control (allocate,
`free!`, dereference) panicked in both builds, and the 38k-file codebase and the conformance corpus ran clean under
poison in the compressed build.

The earlier four-branch integration run on ff92b7f (before #41 reached main) agreed: compressed -16.1% / -15.5% /
-15.4% peak and -1.5% / -2.2% / -2.5% instructions at 1 / 4 / 8 checkers; plain-ptrs -1.8% peak at 1 checker and -6.1% /
-6.8% / -7.1% instructions.
