# mem-64: peak memory of tsrs on a wide machine (baseline and per-checker cost)

Task of the 64-thread hill-climb (bench/results/compare/2026-10-06-2420b7ed410b-64t.md): on vscode tsrs peaks at
3.0 GiB with 4 checkers against `bun check`'s 1.45 GiB at 4 threads, and grows ~33 MiB per extra checker to 5.0 GiB
at 64 (bun: 2.9 GiB). This note splits the peak into front end, per-checker state and allocator behaviour, lands two
exact checker-side reductions, and measures the one allocator setting that would halve the gap. Base: origin/main
1368df3 (the 64-thread probe workflow, fd2ffc1, on top). All diagnostics byte-identical (vscode, webpack,
xstate-main at 1, 4, 16 checkers; 371 / 840 / 0 errors).

## 1. Where the memory is

### Linux, 64 vCPUs (Depot `depot-ubuntu-24.04-64`, EPYC 9R45, THP `madvise`), vscode, release build

Peak RSS (`ru_maxrss`) and, sampled from `/proc/<pid>/smaps` every 50 ms at the moment of the largest total, the
resident bytes inside the compressed-pointer arena reservation ("arena": AST, binder, every checker's types,
symbols, links) against everything else ("other": the mimalloc heap, i.e. hash tables, vectors, source texts; the
binary; stacks). `huge` is `AnonHugePages` from `smaps_rollup`. GiB; tools/perf/probe.sh, probe runs 1ksdm1zzx9 and
rkbqgw4n6w.

| run | peak RSS | arena | other | huge | THP off (`MIMALLOC_ALLOW_THP=0`): peak / arena / other |
| --- | --- | --- | --- | --- | --- |
| `--noCheck` (front end, 64 parse threads) | 2.02 | 0.91 | 1.08 | 1.93 | 1.18 / 0.85 / 0.35 |
| 1 checker | 2.78 | 1.47 | 1.31 | 2.64 | |
| 4 checkers | 3.00-3.02 | 1.55 | 1.46 | 2.87 | 2.14-2.16 / 1.49 / 0.67 |
| 16 checkers | 3.56-3.62 | 1.73 | 1.86 | 3.49 | 2.45-2.52 / 1.65 / 0.87 |
| 64 checkers | 4.97-4.99 | 2.07 | 2.90 | 4.80 | 3.12-3.21 / 1.94 / 1.26 |
| bun check, 16 / 64 threads | 1.90-1.94 / 2.96-3.00 | | | | |

Read:

- **The front end is 2.0 GiB of the 3.0 GiB at 4 checkers**, 0.9 GiB of it the arena (AST + binder; 877 MB
  requested on macOS, see below) and 1.1 GiB "other", of which only 0.35 GiB is live heap: 0.73 GiB is huge-page
  amplification of the mimalloc heap (64 parse threads, each with its own partly filled 64 KiB / 512 KiB mimalloc
  pages in every size class it touched, every 2 MiB block of them resident because mimalloc advises
  `MADV_HUGEPAGE` on its arenas). The same run on an 18-core macOS laptop peaks at 1.21-1.29 GiB.
- **The checkers add 1.0 GiB at 4 checkers and 3.0 GiB at 64.** Per extra checker (4 -> 64, linear): 33.4 MiB, of
  which arena 8.7 MiB (checker objects 7.7 + the resident tail of each thread arena's last huge page), heap 24.7 MiB;
  with THP off the heap part is 10 MiB, so **~15 MiB of every checker's 33 MiB is huge-page amplification of its
  thread-local mimalloc pages** (two OS threads per checker: creation and checking), and the arena's own THP cost is
  0.07 / 0.08 / 0.13 GiB at 4 / 16 / 64 checkers (the finger's partly used 2 MiB block in each of the ~2k+20
  thread arenas; notes/linux-x86-round.md measured why the arena wants huge pages anyway).
- Instructions are the same in every variant (123.4-123.6 G at 4 checkers, 136.2-136.7 G at 16); THP changes only
  page-fault and TLB work.

### What a checker actually creates (TSRS_ASSIGNMENT_STATS, probe 1ksdm1zzx9 / probe0)

| checkers | symbols | types | instantiations | per extra checker (4 -> 64) |
| --- | --- | --- | --- | --- |
| 1 | 4,823,773 | 2,667,411 | 3,501,192 | |
| 4 | 5,198,226 | 3,132,083 | 4,376,083 | |
| 16 | 5,918,145 | 4,031,628 | 6,264,602 | |
| 64 | 7,056,535 | 5,519,354 | 9,825,661 | +31K symbols, +40K types, +91K instantiations |

At 64 checkers each checker makes 55-99K symbols, 67-124K types and 59-315K instantiations for 59-1,004 own files:
the lib and shared-module working set resolved again per checker (notes/mem-shared-base.md: cannot be shared
exactly). An idle checker (a one-file project, 1 -> 64 checkers) costs 0.6 MiB on macOS and 1.4 MiB on Linux including
its threads, so the per-checker cost is all work-related.

### macOS (M5 Max, 18 cores, 16 KiB pages, no THP), vscode: arena / heap / rest per checker

`--features alloc-profile` (`TSRS_ALLOC_PROFILE_TOP`, per-arena table added here), peak = `/usr/bin/time -l`
peak memory footprint. MB = 2^20 bytes.

| checkers | arena requested | heap (non-arena) | peak | peak - (arena + heap) |
| --- | --- | --- | --- | --- |
| noCheck | 877 | 190 (260 at its peak) | 1,233 | 166 |
| 1 | 1,495 | 437 | 2,015 | 83 |
| 4 | 1,590 | 521 | 2,234 | 123 |
| 16 | 1,756 | 666 | 2,580 | 158 |
| 64 | 2,065 | 989 | 3,304 | 250 |

Per extra checker, 16 -> 64: arena 6.4 MB, heap 6.7 MB, allocator / stacks / arena tails 2.7 MB = 15.8 MB (the
Linux 33 MiB is this plus the huge-page amplification; 4 -> 16 is 29 MB per checker on both because small checker
counts still duplicate more of the project).

Arena rows that scale with the checker count (MB per extra checker, 16 -> 64; count at 1 / 64 checkers): `Symbol`
0.75 (4.83M / 7.08M, 32 B), `[ValueSymbolLinks]` chunks 0.59, `[P<Symbol>]` 0.55, `LiteralType` 0.49 (0.47M / 1.22M:
each checker re-creates the lib's string literal types), `ObjectType` 0.34, `TypeMapper` 0.33, `Signature` 0.29,
`[TypeNodeLinks]` chunks 0.27, `InterfaceType` 0.23 (declared lib types), `[P<Type>]` 0.23, `StructuredMembers` 0.21,
`TypeReference` 0.17, `UnionType` 0.15, `TypeParameter` 0.12, `LazyMemberTable` 0.11. Every AST row is flat (shared).
The arenas themselves: 147 at 64 checkers (1 main, 18 parse workers, 64 creation threads with < 1 MB used each on
a 4 KiB-page first chunk, 64 checker threads with 16-64 MB); capacity 3,093 MB against 2,065 MB used, but the
untouched tails are not resident (only their first huge page on Linux, above).

Heap containers each checker owns (`TSRS_HEAP_CENSUS=1`, MB summed over checkers; "id pages" = the two id-keyed
link stores' page tables):

| checkers | total | per checker | id pages (per checker) | largest other rows |
| --- | --- | --- | --- | --- |
| 1 | 166 | 166 | 16.5 | signature / symbol-reference / type-node link slot tables 18 each, object-type instantiations 14, relation assignable 9 |
| 4 | 217 | 54 | 52 (13.1) | object-type instantiations 18, relation assignable 18, signature / type-node links 18, symbol-reference links 16 |
| 16 | 272 | 17 | 110 (6.9) | object-type instantiations 25, relation assignable / signature / symbol-reference / type-node links 17 each |
| 64 | 477 | 7.5 | 210 (3.3) | type-node links 36, object-type instantiations 36, signature links 26, relation assignable 21, symbol-reference links 19 |

The id page tables were the one row that grows faster than the work: a 1,024-id page (2 KiB narrow / 4 KiB dense)
per page of the id space a checker touches, at 14% occupancy at 64 checkers (ids come from per-thread blocks of
1,024, so a checker's links are dense in its own blocks and thin in the 63 others', and every checker touches most
lib symbols whichever checker numbered them): 7.2M value-symbol links in 47K pages of 1,024 slots. The rest of the
per-checker heap is hash tables at their natural load (0.5-0.9) whose entries follow the duplicated objects.

Nothing is retained after its last use that is worth a change: the heap live at the end of the front end is the
source texts (`read_file`, needed by diagnostics and identifier text), the module-resolution cache (7.5 MB) and the
file metadata; parser speculation and binder scratch are already rewound or freed (notes/mem-recycle.md).

## 2. Changes

### 2a. Id-keyed link stores: 128-id groups in the arena (crates/tsrs_checker/src/links.rs)

`IdLinkStore` (`value_symbol_links` by symbol id, `symbol_node_links` by node id) now maps an id through
`index[id / 128]` (`Option<P<IdGroup>>`, 4 bytes per group of the id space seen) to an arena `IdGroup`: 264 bytes
(`before` + 128 x `u16` slot offsets + the pointer of a dense `[u32; 128]` form that a group gets when an offset
outgrows 16 bits, as the narrow pages did). Lookup: two dependent loads (index entry, group), as before (page entry,
page); the sparse page form (`TSRS_SPARSE_ID_PAGES`, notes/mem-shared-base.md: bitmap + rank + shifting inserts,
+0.45-1.2% instructions) is removed, `set_sparse_id_pages` / `set_multiple_checkers` are inert for tsrslint and the
pool. Slots, their first-access order and the values are unchanged, so nothing the checker computes can change. The
groups live in the arena: dense, huge-page backed on Linux, no mimalloc size class per thread. The group index grows
by a quarter instead of doubling (every checker's index ends near the top of the id space).

vscode, id-to-slot tables (census): 211 MB -> 107 MB at 64 checkers, 113 -> 59 MB at 16, 54 -> 26 MB at 4.

### 2b. Reads of reference kinds make no links (checker_01.rs, checker_04.rs)

The unused-locals checks (`isReferenced`, `checkUnusedLocalsAndParameters`, `isUnreferencedVariableDeclaration`,
`isUnreferencedTypeParameter`, `checkUnusedRenamedBindingElements`) read `symbolReferenceLinks.Get(symbol)`, which
creates empty links for every never-referenced local. The links hold nothing but the flags, so `reference_kinds()`
reads through `try_get` and answers `None` for a missing record: vscode single 940K -> 810K entries, the slot table
18 -> 9 MB (one doubling less); 1.22M -> 1.09M at 64 checkers.

### Results, macOS (interleaved, 2 rounds each; `/usr/bin/time -l` peak footprint, instructions retired)

| project, checkers | base peak MiB | new peak MiB | change | base instructions | new |
| --- | --- | --- | --- | --- | --- |
| vscode, 1 | 2,010-2,026 | 2,001-2,004 | -15 (-0.7%) | 113.6-114.6 G | 113.5-114.4 G |
| vscode, 4 | 2,235-2,238 | 2,206-2,208 | -30 (-1.3%) | 119.7 G | 119.5-120.5 G |
| vscode, 16 | 2,584-2,589 | 2,523-2,531 | -60 (-2.3%) | 131.6-132.4 G | 131.8-132.3 G |
| vscode, 64 | 3,317-3,321 | 3,195-3,205 | -119 (-3.6%) | 155.0-156.1 G | 154.6-154.8 G |
| webpack, 1 / 4 / 16 | 327 / 388 / 550 | 326-330 / 384-386 / 543-544 | 0 / -3 / -7 | 14.1-14.4 / 15.8 / 20.6 G | 14.1 / 15.8-15.9 / 20.5-20.6 G |
| xstate-main, 1 / 4 / 16 | 200 / 238 / 336 | 200 / 236 / 335 | 0 | 8.1-8.3 / 9.0 / 11.9-12.0 G | 7.9 / 9.0 / 12.0 G |

(The 4-64 checker instruction counts move +-0.5% from run to run with stealing; single-checker vscode is -0.1%.)
Diagnostics identical in every run (full `--pretty false` output compared).

### Results, Linux 64 vCPUs (probe runs rkbqgw4n6w, nh2qw0m6kr, 5kwpqvqln2; GiB)

Probe nh2qw0m6kr (3 reps base / new, 2 reps the `no_thp` builds, 1 rep the allocator knobs; medians, range of the
peak; `arena` / `other` sampled at the peak; instructions from a perf counter over all threads):

| run | peak RSS GiB (median, range) | arena | other | instructions G | check s | total s |
| --- | --- | --- | --- | --- | --- | --- |
| base, noCheck | 2.034 | 0.91 | 1.13 | 23.6 | | 0.323 |
| new, noCheck | 2.013 | 0.91 | 1.11 | 23.5 | | 0.331 |
| base, 1 checker | 2.745 | 1.46 | 1.28 | 117.2 | 10.470 | 10.778 |
| new, 1 checker | 2.745 | 1.49 | 1.26 | 117.0 | 10.458 | 10.762 |
| base, 4 checkers | 3.002 (3.000-3.005) | 1.55 | 1.45 | 123.6 | 2.699 | 3.022 |
| new, 4 checkers | 2.978 (2.962-3.000) | 1.57 | 1.41 | 123.3 | 2.752 | 3.069 |
| base, 16 checkers | 3.593 (3.589-3.621) | 1.72 | 1.86 | 136.9 | 0.802 | 1.123 |
| new, 16 checkers | 3.529 (3.503-3.564) | 1.76 | 1.74 | 136.2 | 0.797 | 1.115 |
| base, 64 checkers | 4.991 (4.978-4.995) | 2.08 | 2.91 | 161.6 | 0.679 | 1.008 |
| new, 64 checkers | 4.960 (4.957-4.990) | 2.15 | 2.81 | 161.2 | 0.675 | 1.014 |
| base + no_thp, noCheck / 1 / 4 / 16 / 64 | 1.329 / 2.103 / 2.306 (2.295-2.318) / 2.689 (2.686-2.692) / 3.455 (3.451-3.458) | 0.92 / 1.47 / 1.55 / 1.73 / 2.07 | 0.43 / 0.64 / 0.76 / 0.97 / 1.40 | 23.5 / 117.3 / 123.5 / 136.6 / 161.4 | - / 10.854 / 2.788 / 0.841 / 0.695 | |
| new + no_thp, noCheck / 1 / 4 / 16 / 64 | 1.328 / 2.080 / 2.272 (2.261-2.282) / 2.631 (2.627-2.635) / 3.355 (3.350-3.359) | 0.92 / 1.48 / 1.57 / 1.78 / 2.14 | 0.43 / 0.61 / 0.71 / 0.86 / 1.22 | 23.5 / 117.0 / 123.4 / 136.5 / 160.5 | - / 10.562 / 2.848 / 0.829 / 0.678 | |
| new, `MIMALLOC_ARENA_EAGER_COMMIT=0`, 4 / 64 | 2.268 / 3.331 | 1.58 / 2.13 | 0.70 / 1.21 | 123.3 / 160.7 | 2.943 / 0.697 | |
| new, `MIMALLOC_PURGE_DELAY=100`, 4 / 64 | 2.970 / 4.925 | 1.58 / 2.13 | 1.39 / 2.79 | 123.4 / 161.0 | 2.764 / 0.686 | |
| new, `MIMALLOC_ALLOW_THP=0`, 4 / 64 | 2.121 / 3.097 | 1.51 / 2.01 | 0.63 / 1.10 | 123.3 / 161.8 | 2.963 / 0.711 | |

Probe 5kwpqvqln2 (the runner's default of 8 checkers, 3 reps; 4 and 64 again, 2 reps):

| run | peak RSS GiB (range) | arena | other | instructions G | check s |
| --- | --- | --- | --- | --- | --- |
| base, 8 checkers | 3.232-3.246 | 1.63 | 1.59-1.61 | 129.0-129.1 | 1.432-1.447 |
| new, 8 checkers | 3.186-3.207 | 1.66-1.67 | 1.52-1.54 | 128.7-128.9 | 1.405-1.437 |
| base + no_thp, 8 checkers | 2.466-2.485 | 1.63-1.64 | 0.84-0.86 | 129.0-129.1 | 1.456-1.467 |
| new + no_thp, 8 checkers | 2.440-2.445 | 1.66-1.67 | 0.78-0.79 | 128.8-128.9 | 1.456-1.475 |
| base / new, 4 checkers | 3.015-3.027 / 2.970-2.994 | | | 123.6 / 123.3-123.4 | 2.66-2.78 / 2.69-2.71 |
| base / new, 64 checkers | 5.017-5.019 / 4.921-4.952 | | | 161.1-161.5 / 161.2 | 0.67-0.78 / 0.67 |
| one-file project, base, 1 / 8 / 64 checkers | 0.115 / 0.143 / 0.201 (no_thp: 0.066 / 0.079 / 0.111) | | | | |

An idle checker (the one-file project: it initializes, owns two OS threads, checks nothing) costs 1.4 MiB on Linux
with the huge-page heap and 0.7 MiB without (0.6 MiB on macOS), so the 33 MiB of a working checker is its work: the
heap pages a checking thread touches in every size class.

With the huge-page heap (the shipped configuration) the two checker changes are worth -0.03 / -0.05 / -0.06 /
-0.08 GiB at 4 / 8 / 16 / 64 checkers (-1.0% / -1.4% / -1.7% / -1.6%): the id groups move ~100 MB from the heap
into the arena, and under THP the heap they leave behind stays largely resident (partly filled pages). With a
4 KiB-page heap they are worth -0.03 / -0.03 / -0.06 / -0.10 GiB (-1.5% / -1.0% / -2.2% / -2.9%), on macOS
-0.03 / - / -0.06 / -0.12 GiB. Instructions: -0.2% / -0.5% / -0.2% (within the run-to-run spread of stealing).
Diagnostics identical in every row (371 errors; `error TS` lines compared across the variants).

## 3. The allocator: huge pages on the mimalloc heap

Applied later, with more variants measured: notes/mem-thp.md (`mimalloc/no_thp`).

mimalloc v3 (libmimalloc-sys 0.1.49) advises `MADV_HUGEPAGE` on the 1 GiB arenas it reserves (prim/unix/prim.c,
`allow_thp`), so on a THP `madvise` host every 2 MiB block of the heap that any thread touched is resident in full.
Partly filled thread-local pages (one per size class per thread: 64 KiB small, 512 KiB medium, 4 MiB large) then
cost their whole size, and with 64 parse threads plus two threads per checker that is most of the "other" column
above (~10 MiB per allocating thread). The arena of tsrs (`tsrs_core::reserve`) advises its chunks too, but a bump
allocator wastes at most the finger's block per thread.

The crate feature `mimalloc/no_thp` (`MI_NO_THP`) compiles mimalloc's advice out while the tsrs arena keeps its huge
pages. Measured on the probe with that one-line change in crates/tsrs_cli/Cargo.toml (`mimalloc = { version =
"0.1.52", features = ["no_thp"] }`), base and new (2a + 2b) binaries, 2-3 reps:

| run | base | base + no_thp | new | new + no_thp | check time, base -> new + no_thp |
| --- | --- | --- | --- | --- | --- |
| noCheck | 2.02-2.03 | 1.32-1.33 | 1.99-2.01 | 1.33 | 0.31-0.32 -> 0.33-0.34 s total |
| 1 checker | 2.75-2.77 | 2.10 | 2.75 | 2.08-2.10 | 10.4-10.5 -> 10.6 s (one 13.1 s outlier) |
| 4 checkers | 3.00-3.03 | 2.30-2.32 | 2.96-3.01 | 2.26-2.30 | 2.67-2.70 -> 2.72-2.85 s (+2-5%) |
| 8 checkers (the runner's default) | 3.23-3.25 | 2.47-2.49 | 3.19-3.21 | 2.44-2.45 | 1.43-1.45 -> 1.46-1.48 s (+2%) |
| 16 checkers | 3.56-3.62 | 2.67-2.69 | 3.50-3.56 | 2.63-2.65 | 0.79-0.80 -> 0.80-0.84 s (+2-5%) |
| 64 checkers | 4.97-5.02 | 3.45-3.46 | 4.92-4.99 | 3.35-3.36 | 0.67-0.69 -> 0.67-0.68 s |

Instructions identical (123.3-123.6 G at 4, 128.7-129.1 at 8, 136.2-136.9 G at 16, 160.3-161.8 G at 64). Per extra
checker, 4 -> 64: 33.4 MiB base, 19.6 base + no_thp, 33.0 new, **18.0 new + no_thp**. Against bun: 2.27 vs 1.45 GiB
at 4 threads, 3.35 vs 2.97 GiB at 64.

This is an allocator-policy decision outside this task's write set (tsrs_cli), with a wall-time cost of ~2-5% of
check time at 4-16 checkers (page faults and TLB on the heap; 0 at 64) against -23% / -24% / -26% / -32% peak RSS
at 4 / 8 / 16 / 64 checkers, and it would show on the 8-vCPU README benchmark as a small wall regression. Not
applied here; the numbers are for the owner. Knobs that keep the advice: `MIMALLOC_PURGE_DELAY=0` gives back
0.1-0.2 GiB at 64 checkers for +25% check time; `MIMALLOC_ARENA_EAGER_COMMIT=0` gives the same memory as `no_thp`
(2.27 / 3.33 GiB at 4 / 64: piecemeal commits split the mappings so that no huge page forms) for +7% check time at
4 checkers (the commits are system calls); `MIMALLOC_PURGE_DELAY=100` nothing (2.97 / 4.93). Disabling THP for the
whole process (`MIMALLOC_ALLOW_THP=0`) saves another 0.1-0.2 GiB (the arena's finger blocks) for +4-10% check time
at 4 checkers. Fewer allocating threads would cut the same amplification without touching the allocator (each
parse worker holds ~11 MiB of it at the peak; the 64 creation threads hold little, see the one-file project).

## 4. Tried, rejected, not done

- **Sparse id pages by default** (the existing `TSRS_SPARSE_ID_PAGES=1`): -0.15 GiB at 64 checkers on Linux and
  macOS, -0.06 at 16, 0 at 4, for +2% instructions (126.1 vs 123.6 G at 4 checkers, 165 vs 161 G at 64). The
  128-id groups save the same memory at 64 and more at 4-16 for no instructions.
- **Trimming thread arenas at thread exit** (`MADV_DONTNEED` on the unused part of the finger's huge page): the
  per-arena table shows the 64 creation threads use < 1 MB each, on the 4 KiB-page first chunk, so there is nothing
  to trim; the parse workers (rayon keeps them) and the checker threads are alive at the peak.
- **Right-sizing fixed reservations**: there are none in the checker beyond `globals` (sized from the global symbol
  count) and per-call vectors; every cache map allocates on first insert.
- **Inline values for the three big pointer-keyed link stores** (signature / type-node / symbol-reference links:
  8-byte slot + 8-16-byte arena value per entry): callers keep the `P<V>` across other inserts, so the value cannot
  move with the table. Not done.
- **Arena-backed hash tables** (the per-checker tables through an `allocator-api2` allocator on the thread arena,
  so that they stop costing a mimalloc page per size class per thread): what the id groups did for one row,
  generalized; large and risky (every `FxHashMap` type in the checker, buffers freed on growth that only a size-class
  free list could reuse), and `no_thp` removes the amplification it would target more completely. Not started.
- Front-end memory (0.9 GiB arena + 0.35 GiB live heap + 0.73 GiB huge-page amplification on 64 parse threads) is
  the front-end worker's; nothing there is retained past its last use that a checker-side change could free.

## 5. Gates

- Diagnostics: full `--pretty false` output of vscode, webpack and xstate-main identical to the base binary at 1, 4,
  16 (and 64 for vscode) checkers, macOS; vscode identical between the four Linux variants at 4 / 16 / 64.
- `cargo test -p tsrs_checker` (links tests rewritten for the groups: offsets past 16 bits turn a group dense without
  losing a slot; scrambled ids over 300 groups map to their own slots), `cargo test --release -p tsrs_cli` targets
  `default_emit`, `derived_variance`, `flow_memo` (the `memory_tests` in api.rs do not link on macOS, glibc
  `malloc_trim`, as on main).
- `tools/lint/ratchet.py`: ok, none new; `tools/lint/source.py --update` (two Relaxed atomics of the sparse-page
  switches removed from `atomics.tsv`).
- Tooling kept: `tools/perf/probe.sh` (variant builds, `/proc` sampling, perf counter), the alloc profile's per-arena
  table (`crates/tsrs_core/src/alloc_profile.rs`).
