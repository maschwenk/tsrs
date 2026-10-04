# perf-round2: #39 on compressed pointers, and the round's total

Branch `perf/round2`. It started as the integration of the four performance branches of 2026-10-04 (#37, #40, #38,
#39, merged in that order with real merge commits). While it ran, the owner merged #37, #40, #38 and #41
(`perf/dev-loop`) into main, so after merging origin/main 21b95e9 the branch's diff against main is exactly
**#39 (`mem/scoped-arenas`) adapted to compressed pointers** (`git diff origin/main...perf/round2`: #39's 13 files,
plus the compressed-mode half of the poison check below). This note has the conflict resolutions, the gates, #39's
own delta on the new main, and the round's total against the pre-round main 12a8acd.

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

Warm runs: each round makes a fresh seed (a cold `--incremental` run into the binary's own `--tsBuildInfoFile`),
then the warm run (which rewrites the tsbuildinfo, PR #37's "warm, no edit"), then appends
`export const __tsrsRound2Leaf = N;` to `src/apiServer.test.ts` (nothing imports it), runs, and restores the file.
Another agent edited a different corpus file during the evening; three warm-scenario runs whose seed and run saw
different files (a full recheck, Types 13.8M instead of 376 / 1,267) were dropped and two extra warm rounds run, so
the warm rows have 6-7 runs per binary. Cold runs that saw the edited file (`Lines` +2) differ only in `Lines` /
`Symbols` and were kept. Counters (Types, Symbols, Instantiations) and diagnostics (0 errors) are identical between
the binaries in every run that saw the same files.

The language server: `tools/lsp-mem/lsp_mem.py --edits 40 --every 10` on
`src/services/guestPhotoSourcing/guestPhotoReviewQueueService.ts` (diagnostics + hover after each edit), 5
interleaved rounds; medians; ranges 12a8acd start 2,403-2,419 / end 3,491-3,523, r2 2,207-2,231 / 3,015-3,074 MiB;
0 errors in every session.

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

## Conflicts and how they were resolved

The integration merges (kept in the branch history) and what they had to reconcile; main's own merges of #38, #40 and
#41 were textually clean, and so were these, except one file.

- **#37 + #40**: no conflict.
- **#38 into #37 + #40**: no textual conflict and no semantic one. The expected clash, #40's Bloom filter in "the
  spare word of small symbol tables" against #38's handle-packed symbol-table entries, does not exist: they are
  different fields of `SymbolMap`. The filter lives in `SymbolMap::extra` (`ExtraSlot`: either a
  `Box<SymbolMapExtra>`, a heap address, or the 64-bit filter with bit 0 set); heap boxes are not arena objects,
  compression does not touch that word, and `SymbolMap` stays 24 bytes in both builds. #38 rewrote the entries
  (`SymbolMapEntry`, `P::pack` in the low 45 bits), whose odd-key / length / hash bits #40's `position` reads through
  the unchanged `KeyPrint` / `entry_matches`. #40 added no packed word and no `from_static` / `addr()` use.
- **#39 into the rest**: one conflict, `crates/tsrs_core/src/ptr.rs`. #39 adds `check_not_freed` (alloc-profile builds
  with `TSRS_ARENA_POISON=1`: every `P` dereference panics if the target starts with eight poison bytes) to `P::get`
  and `Deref`; #38 split those into a plain-pointer and a compressed impl. Resolution: the check stays in the plain
  `get` / `Deref` and is added to the compressed `get` (the compressed `Deref` calls `get`). Its guard was
  `size_of_val >= 8 && align_of_val >= 8`; a compressed handle always names an 8-aligned block (`p_layout` aligns
  and pads every `P` target to 8), so there the guard is `size >= 8` only, which also covers the 4-aligned types that
  most checker structs became once their fields are handles.
- Auto-merged regions read by hand: `InferenceContext` (#38: `inferences` a one-word `PSliceCell`, `rare` a `to_bits`
  word; #39: return-mapper recycling and escape-gated setters, all through the accessors; the `RARE_ESCAPED` bit
  survives because `to_bits` is 8-aligned), `TypeMapper` (#38 re-encoded both words and kept the escape bit; #39 only
  calls `escaped()`, `data()`, `free!`, `recycle_*`), the census (#39's changes are in code that only runs in
  plain-pointer builds; compressed builds refuse `TSRS_CENSUS=1`, as #38 left it).

## Gates (r2 against a build of origin/main 21b95e9)

- Suite `--baselines types,symbols`, default and `TSRS_LAZY_MEMBERS=0`: whole `TSRS_TEST_RESULTS` trees identical
  (`diff -rq`, `summary.json` excluded), compressed vs compressed main, plain vs plain main, and plain vs compressed
  main. 13,458 error baselines pass (2 codes, 2 fail, as on main), `.types` / `.symbols` 12,779 / 12,779.
- `--baselines js,jsmap,sourcemap`: trees identical, 13,462 / `.js` 13,392 / `.js.map` 149 / `.sourcemap.txt` 156 pass,
  0 fail. (An earlier run at load 50 timed out `compiler/intersectionConstructorReductionCrash` at the 20 s limit; it
  takes 13-16 s with js baselines on every binary, base included, and passes on a rerun.)
- Fourslash: 4,066 pass / 63 fail / 417 skip, result trees identical, both representations.
- `cargo test --release -p tsrs_cli`, both representations: tsctests with a fresh dump (`tools/oracle/tsctests/dump.sh`,
  522 scenarios): 374 pass / 32 fail, the same pass and fail lists as main (tsc 187/216, tsbuild 187/190);
  default_emit 6/6. `api::memory_tests` compiled out locally (glibc `malloc_trim`; does not link on macOS, on main
  either; not committed).
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked`, default and plain-ptrs, and `-p tsrs_cli
  --features alloc-profile` in both: clean.

### #39's census proof on the merged tree (plain-ptrs alloc-profile build)

`TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1 TSRS_CENSUS_ASSERT=1 TSRS_CENSUS_SKIP_FREED=1`:

| run (38k-file codebase) | freed or rewound | strong mark: freed blocks reachable | precise walk: references to freed blocks |
| --- | --- | --- | --- |
| 1 checker, default | 17.25M blocks / 388.1 MB | 0 | 0 of 48.2M |
| 4 checkers, default | 26.14M / 559.5 MB | 0 | 0 |
| 1 checker, `TSRS_LAZY_MEMBERS=0` | 18.33M / 404.7 MB | 0 | 0 |
| 4 checkers, `TSRS_LAZY_MEMBERS=0` | 27.30M / 577.2 MB | 0 | 0 |

The same block counts as #39 measured on its own branch. Conformance corpus (12,758 files, `tsrs --strict --target
esnext`, one process per file): in every file 0 freed blocks strongly reachable and 0 precise-walk references to freed
blocks, no assert exit (4,006 files without and 8,752 with diagnostics, as on #39's branch).

### Poison mode in the compressed build

Supported. A positive control (temporary test, not committed: allocate, `free!`, dereference) panics with "arena:
dereference of a freed block" in both the compressed and the plain alloc-profile build, so the check is live in both
representations. The 38k-file codebase with `TSRS_ARENA_POISON=1` through the compressed alloc-profile build, 1 / 4
checkers, default / `TSRS_LAZY_MEMBERS=0` (4 checkers with `--checkerAssignment go`): no panic, output and counters
identical to main's release binary apart from timing lines and #41's `FS:` call counters, which vary between runs of
main itself (113,800-113,903 stats over these 8 runs). Conformance corpus through the same
build: 12,758 files, no panic, stdout and exit code identical to main's release binary in every file.

## Earlier integration run (before #41 reached main)

The four-branch merge (#37 #40 #38 #39 on ff92b7f) against ff92b7f, 5 interleaved rounds, load 4-8 (24 in one run),
same scenarios; this is the run that showed the pieces add up before main moved:

| run | ff92b7f | merged, compressed | merged, plain-ptrs |
| --- | --- | --- | --- |
| 1 checker peak / instructions | 5.064 GiB / 295.7 G | 4.251 (-16.1%) / 291.3 G (-1.5%) | 4.972 (-1.8%) / 277.6 G (-6.1%) |
| 4 checkers peak / instructions / wall | 6.703 / 402.1 G / 6.99 s | 5.661 (-15.5%) / 393.1 G (-2.2%) / 6.69 s | 6.599 / 374.8 G (-6.8%) / 6.59 s |
| 8 checkers peak / instructions / wall | 8.527 / 515.0 G / 5.70 s | 7.213 (-15.4%) / 502.3 G (-2.5%) / 5.63 s | 8.397 / 478.2 G (-7.1%) / 5.42 s |
| `--noCheck` peak | 1.865 | 1.658 (-11.1%) | 1.867 |
| warm no edit, wall / peak | 6.70 s / 3.518 | 1.67 s / 3.297 | 1.73 s / 3.399 |
| leaf edit, wall / peak | 7.74 s / 3.977 | 2.76 s / 3.748 | 2.78 s / 3.846 |

## Recommended merge order

#37, #40, #38 and #41 are merged. This PR is the remaining piece (#39 rebased onto compressed pointers); merge it
instead of #39, whose own head conflicts with main in `ptr.rs` and would lose the poison check in compressed builds.

## Reproduce

```sh
# binaries: this branch (compressed and --features tsrs_core/plain-ptrs), origin/main, 12a8acd; alloc-profile builds:
CARGO_TARGET_DIR=target/prof-plain cargo build --release -p tsrs_cli --features alloc-profile,tsrs_core/plain-ptrs  # census
CARGO_TARGET_DIR=target/prof cargo build --release -p tsrs_cli --features alloc-profile                             # poison, compressed
cd <38k-file codebase>
/usr/bin/time -l tsrs -p . --noEmit --extendedDiagnostics --pretty false --incremental false --checkers N
tsrs -p . --noEmit --extendedDiagnostics --pretty false --tsBuildInfoFile <own dir>/t.tsbuildinfo   # seed, then warm, then leaf edit
TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1 TSRS_CENSUS_ASSERT=1 TSRS_CENSUS_SKIP_FREED=1 target/prof-plain/release/tsrs -p . --noEmit --incremental false --checkers 1
TSRS_ARENA_POISON=1 target/prof/release/tsrs -p . --noEmit --extendedDiagnostics --pretty false --incremental false
python3 tools/lsp-mem/lsp_mem.py --cmd "<tsrs> --lsp -stdio" --project <38k-file codebase> \
  --file src/services/guestPhotoSourcing/guestPhotoReviewQueueService.ts --edits 40 --every 10
```

The interleaving driver (fresh seed per round, leaf edit restored in a `finally`, runs whose seed and run saw
different corpus files dropped) was a scratch script, not committed.
