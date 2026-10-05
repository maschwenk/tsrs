# linux-x86-round: huge pages for the compressed arena, and an x86-64 look at the check phase

Linux x86-64 is where CI and the coding sandboxes run, and all three checker CPU rounds were tuned on Apple Silicon.
Part 1: the arena lost its transparent huge pages when `P<T>` became a 32-bit handle, and what fixing it shows about
the x86 cost of the handles. Part 2, an x86 profile of the check phase on top of it, is added with the changes it
leads to.

## Machine and method

A cloud sandbox microVM (KVM guest), 18 vCPUs, 47 GiB, no swap, kernel 7.2.9, 4 KiB pages, THP `enabled` =
`madvise`, `defrag` = `madvise`, mTHP sizes off. The VM restarts on whatever host is free, so two hosts appear here:
**host A**, Intel Xeon Platinum 8259CL @ 2.50 GHz (Cascade Lake), and **host B**, Intel Xeon Platinum 8375C @ 2.90 GHz
(Ice Lake), which was noisier (one binary's wall varied by 15% within a session). Only runs interleaved within one
session are compared. The private corpus is the 38k-file codebase with its workspace packages unbuilt: 40,543 errors,
the same output for every binary (md5 of the error lines and the Types / Symbols / Instantiations counters checked in
every run).

Every run: `tsrs -p . --noEmit --incremental false --pretty false --extendedDiagnostics --checkers N` under
`perf stat -e instructions,cycles,dTLB-load-misses,dTLB-store-misses` (user + kernel) and `/usr/bin/time` (wall,
user, sys, minor faults, max RSS); `AnonHugePages` sampled from `/proc/<pid>/smaps_rollup` every second (maximum),
and the system's `thp_fault_alloc` / `thp_fault_fallback` deltas from `/proc/vmstat`. Binaries are the `dist` profile
(fat LTO, one codegen unit) without PGO, built from one commit in one session. 5 interleaved rounds, binary order
rotated every round; tables give medians, and deltas are medians of the per-round paired ratios (with their range).

## Part 1: transparent huge pages for the arena

### What went wrong

Before pointer compression, thread-arena chunks came from mimalloc, which calls `madvise(MADV_HUGEPAGE)` on the OS
memory it maps. notes/linux-perf.md measured that this is worth 11-12% of wall time on this kind of host and warned
that any change of chunk allocation on Linux must keep the advice. Pointer compression then moved every chunk into
the 32 GiB reservation (`tsrs_core::reserve`: one `mmap(PROT_NONE, MAP_NORESERVE)`, `mprotect` per chunk), which never
asked for huge pages. On a THP `madvise` host (the default of Ubuntu and of most distributions) the arena has run on
4 KiB pages since: in a 4-checker run of main, the arena's 2.6 GiB had no huge page and only the mimalloc heap had
them (2.4 GiB `AnonHugePages`, 0.9 M minor faults, against 31 K for a `plain-ptrs` build).

### The change

`reserve::alloc_chunk(size, huge)`: thread-arena chunks of 2 MiB and more (`huge`; every chunk after a thread's 1 MiB
first one) start and end on 2 MiB boundaries in the reservation and are advised with `MADV_HUGEPAGE` right before the
`mprotect` that commits them; the arena rounds their sizes to 2 MiB so that a tail it would not use is not part of a
resident huge page. A fault gets a huge page only if the aligned 2 MiB around it lies inside one advised read-write
mapping, which a chunk that shares a 2 MiB block with an uncommitted neighbour does not: unaligned chunks would lose
their first and last block. The chunk allocator keeps first fit and puts the range an aligned request skips back on
the free list, where region slabs (64 KiB granularity) reuse it. The first chunk of each thread stays at 1 MiB on
4 KiB pages, so threads that allocate little do not hold a 2 MiB page each (see "Region slabs").

- The advice survives `mprotect`: the commit splits the reservation's mapping, and the split keeps the flag. Checked
  in `/proc/<pid>/smaps` during a four-checker run of the first version (below), 14 s in: every read-write mapping of
  the arena carried `hg` in `VmFlags`, and its `AnonHugePages` equalled its `Rss` (2.72 of 2.72 GiB; main: 0 of
  2.64 GiB).
- Thread-arena chunks are never released, so the decommit path (a fresh `mmap(MAP_FIXED, PROT_NONE)`, which drops
  the advice) only sees region slabs. A range that a slab gave back and a thread chunk later takes gets the advice
  again at its commit.
- Region slabs keep 4 KiB pages (no advice): see "Region slabs" below.
- macOS and other unixes: unchanged. No chunk is `huge` there (the arena's `HUGE_THREAD_CHUNK` is `usize::MAX`), so
  chunk sizes, offsets and system calls are what they were.

### Result

Main = origin/main 8cba935; fix = that plus the change; plain = the fix commit with `--features tsrs_core/plain-ptrs`
(references; its arena chunks come from mimalloc and have huge pages). Host A ran the first version of the change,
which also put each thread's first chunk (then 2 MiB) on a huge page; host B ran the final version. On check-only runs
the two differ by a few hundred page faults per thread.

Fix against main, paired per round:

| host, checkers | wall | cycles | sys | instructions | minor faults | dTLB load misses | max RSS |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A, 1 | -4.4% (-6.6..-1.7) | -5.0% (-7.2..-2.7) | -33% | -0.8% | 740 K -> 31 K | 35 -> 14 M | +0.1% |
| A, 4 | -4.4% (-6.8..+1.6) | -2.9% (-8.1..-1.8) | -33% | -0.8% | 900 K -> 29 K | 42 -> 14 M | +0.5% |
| A, 8 | -5.5% (-13.4..-5.1) | -6.1% (-6.8..-3.4) | -36% | -0.9% | 1,068 K -> 22 K | 50 -> 15 M | +0.5% |
| B, 1 | -1.7% (-6.2..+22.3) | -2.3% (-5.8..+5.0) | -26% | -0.8% | 743 K -> 35 K | 34 -> 20 M | +0.7% |
| B, 4 | -5.9% (-8.9..+7.1) | -3.4% (-6.6..+5.8) | -29% | -0.8% | 901 K -> 36 K | 45 -> 23 M | +0.7% |
| B, 8 | -4.7% (-8.5..+2.7) | -3.9% (-6.7..-1.9) | -31% | -0.7% | 1,070 K -> 30 K | 56 -> 25 M | +0.7% |

(Instructions fall by the kernel's page-fault work. On host B the first run of the session, fix with one checker,
read the corpus from a cold page cache: +10 s of wall, the +22% outlier.) Every huge-page fault succeeded
(`thp_fault_fallback` 0 in all runs; the VM had ~40 GiB free). The RSS cost is small because the arena is dense: a
partly used 2 MiB block exists only at each thread's bump finger.

Host A, all three builds (medians of 5):

| checkers | build | wall s (range) | user s | sys s | instructions G | cycles G | dTLB load / store misses M | minor faults | max RSS GiB | max AnonHugePages GiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | main | 55.53 (54.98-57.03) | 59.1 | 4.13 | 256.8 | 190.6 | 35 / 7 | 740,106 | 4.46 | 1.62 |
| 1 | fix | 53.15 (52.56-54.44) | 57.8 | 2.75 | 254.7 | 181.0 | 14 / 3 | 30,830 | 4.47 | 4.32 |
| 1 | plain | 52.77 (50.38-54.38) | 57.4 | 3.03 | 239.2 | 180.1 | 15 / 3 | 37,103 | 5.30 | 5.11 |
| 4 | main | 22.87 (22.70-23.60) | 78.9 | 4.71 | 349.0 | 252.5 | 42 / 9 | 899,923 | 5.90 | 2.40 |
| 4 | fix | 21.70 (21.32-23.22) | 77.3 | 3.16 | 346.3 | 240.9 | 14 / 3 | 28,618 | 5.91 | 5.73 |
| 4 | plain | 21.37 (20.97-21.42) | 74.7 | 3.59 | 325.0 | 233.9 | 16 / 3 | 30,433 | 6.96 | 6.74 |
| 8 | main | 18.53 (18.13-19.77) | 101.3 | 5.57 | 448.8 | 322.9 | 50 / 9 | 1,068,205 | 7.44 | 3.25 |
| 8 | fix | 17.32 (17.12-17.59) | 98.0 | 3.69 | 445.0 | 305.0 | 15 / 3 | 22,169 | 7.48 | 7.29 |
| 8 | plain | 17.30 (16.91-17.76) | 94.8 | 3.99 | 417.3 | 296.5 | 17 / 4 | 27,512 | 8.76 | 8.47 |

Host B, all three builds (medians of 5):

| checkers | build | wall s (range) | user s | sys s | instructions G | cycles G | dTLB load / store misses M | minor faults | max RSS GiB | max AnonHugePages GiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | main | 46.96 (45.39-47.73) | 49.9 | 3.32 | 256.6 | 179.7 | 34 / 12 | 742,702 | 4.45 | 1.58 |
| 1 | fix | 46.17 (44.06-55.50) | 49.5 | 2.46 | 254.5 | 174.4 | 20 / 6 | 34,569 | 4.48 | 4.31 |
| 1 | plain | 44.96 (43.06-47.38) | 48.1 | 2.74 | 238.8 | 170.2 | 23 / 8 | 35,985 | 5.29 | 5.10 |
| 4 | main | 19.46 (18.97-21.30) | 66.9 | 4.08 | 348.7 | 239.7 | 45 / 16 | 901,384 | 5.89 | 2.34 |
| 4 | fix | 18.71 (18.05-20.32) | 66.8 | 2.94 | 346.0 | 234.4 | 23 / 8 | 36,233 | 5.92 | 5.68 |
| 4 | plain | 18.05 (17.78-20.29) | 63.0 | 3.23 | 324.7 | 222.4 | 21 / 8 | 30,106 | 6.94 | 6.69 |
| 8 | main | 15.40 (15.02-16.96) | 84.7 | 4.72 | 448.2 | 302.1 | 56 / 20 | 1,069,666 | 7.43 | 3.23 |
| 8 | fix | 15.42 (14.38-15.93) | 82.3 | 3.28 | 445.1 | 288.7 | 25 / 10 | 30,182 | 7.49 | 7.27 |
| 8 | plain | 15.09 (13.97-17.07) | 82.8 | 3.75 | 416.8 | 291.7 | 29 / 12 | 24,156 | 8.76 | 8.44 |

### THP `always` and `never`

Host B with the system's THP mode switched, two interleaved 4-checker rounds each, and the arena split taken 14 s
into a run:

| THP | build | arena huge / resident GiB | wall s | cycles G | minor faults | max RSS GiB |
| --- | --- | --- | --- | --- | --- | --- |
| `always` | main | 2.56 / 2.84 | 18.86, 20.74 | 233.7, 248.2 | 108 K | 6.19 |
| `always` | fix | 2.88 / 2.89 | 18.43, 19.07 | 232.8, 238.3 | 28-37 K | 6.19 |
| `never` | main | 0 / 2.70 | 22.90, 22.03 | 274.6, 265.4 | 1.49 M | 5.78 |
| `never` | fix | 0 / 2.68 | 23.90, 22.41 | 275.4, 271.9 | 1.49 M | 5.79 |

- `always`: the reservation already got huge pages without advice wherever a 2 MiB block happened to lie inside
  committed neighbours (90% of the arena); the aligned chunks get the rest. Advising is harmless there (same RSS).
- `never`: no huge pages for anyone, the same page faults; nothing to gain and nothing lost. Four more interleaved
  rounds: fix against main wall -0.6% (-4.6..+2.0), cycles +0.4%, sys -1.8%, max RSS +0.1%.

### Region slabs, emit and the language server

Regions (one per file during emit, per file version, checker and program in the language server) carve their chunks
from per-thread 1 MiB slabs, or get a slab of their own above 256 KiB, and release slabs as their regions die; up to
64 released 1 MiB slabs stay committed for reuse. A slab on a huge page that shares it with a neighbour cannot be
returned without splitting the page. Measured on vscode `src` (`--noEmit false --declaration --sourceMap --outDir
/tmp/... --rootDir <vscode>`, 4 checkers, 5 interleaved rounds, medians; then a language-server session of 60 edits
in `vs/editor/common/model/textModel.ts` with `tools/lsp-mem`), on both hosts:

| | main | all thread chunks huge (first version) | final: first chunk on 4 KiB pages | also slabs huge (2 MiB, advised) |
| --- | --- | --- | --- | --- |
| emit run wall s, host A / B | 6.56 / 5.48 | 6.16 / 5.28 | - / 5.09 | 6.17 / - |
| emit phase s, host A / B | 1.649 / 1.371 | 1.576 / 1.365 | - / 1.336 | 1.622 / - |
| emit run max RSS GiB, host A / B | 3.361 / 3.349 | 3.410 / 3.409 | - / 3.387 | 3.445 / - |
| server RSS start -> end MiB, host A | 1722 -> 1732 | 1758 -> 1771 | - | 1749 -> 1762 |
| server RSS start -> end MiB, host B | 1692 -> 1706 | 1754 -> 1771 | 1705 -> 1732 | - |

Output trees (28,213 files) and the 371 diagnostics were identical in every run.

- Slabs on huge pages made emit no faster (the emit phase is bound by the checker's escaped work, which lands in the
  thread arenas, and by writing) and cost 35 MB more, so slabs keep 4 KiB pages and the decommit path is unchanged:
  the language server's region memory behaves as before (same growth per edit).
- The first version also gave each thread's first chunk (then 2 MiB) a huge page, so every thread that allocates at
  all held a resident 2 MiB block: +60 MiB for a language server on vscode, +50-60 MB for emit. The final version
  keeps the first chunk at 1 MiB on 4 KiB pages; only threads that fill it move to huge chunks: +12-27 MiB for the
  server, +38 MB (+1.1%) for emit. On the check-only runs the two versions behave the same (they differ by a few
  hundred page faults per thread).

### Gates

Against 8cba935 built in a second worktree. macOS (code path unchanged): conformance `--baselines types,symbols`
13,458 (+2 codes, 2 fail) / 12,779 / 12,779, default and `TSRS_LAZY_MEMBERS=0`, `test-results` trees identical;
`--baselines js,jsmap,sourcemap` 13,392 / 149 / 156, identical; fourslash 4,066 pass / 63 fail, identical; tsctests
374 / 32 / 1 with identical lists; the 38k-file codebase: diagnostics and checker counters identical. Linux sandbox:
conformance `--baselines types,symbols` with release builds of main and the change, trees identical; `tsrs_core`
tests pass (including the aligned-take test); `RUSTFLAGS="-D warnings" cargo check --workspace --locked`,
`tools/lint/ratchet.py` and `tools/lint/source.py` clean on both systems.

## What pointer compression costs on x86-64, both sides with huge pages

The same runs, fix against plain, paired per round:

| host, checkers | wall | cycles | instructions | user | max RSS |
| --- | --- | --- | --- | --- | --- |
| A, 1 | +0.6% (+0.1..+4.3) | +0.9% (-0.1..+3.4) | +6.5% | +0.6% | -15.7% |
| A, 4 | +1.4% (-0.5..+9.2) | +2.6% (+1.3..+6.1) | +6.6% | +3.2% | -15.0% |
| A, 8 | -0.9% (-1.2..+3.3) | +1.4% (-0.8..+3.5) | +6.6% | +2.4% | -14.6% |
| B, 1 | +7.2% (-7.0..+28.0) | +6.0% (-5.1..+10.6) | +6.6% | +6.6% | -15.4% |
| B, 4 | +1.3% (-2.0..+12.6) | +0.8% (-1.9..+9.1) | +6.6% | +1.3% | -14.8% |
| B, 8 | +2.2% (-6.7..+4.4) | +0.3% (-2.9..+1.9) | +6.8% | +0.8% | -14.5% |

Host A, the quieter one, puts the handles at about +1% wall and +1-2.6% cycles; host B agrees within its noise
(its one-checker rounds spread over 15%). The "+4.4% wall with four checkers, +2.3% with one" `dist` comparison in
notes/perf-round2-followups.md, and the +3-8% of the zero-based-handles branch, compared a compressed build without
huge pages to a `plain-ptrs` build with them. The handles still retire 6.5% more instructions, but those are register
moves and shifts that retire cheaply, and the smaller working set pays for part of them; for 15% less memory.

## Reproducing

```sh
cargo build --profile dist -p tsrs_cli                                   # compressed (default)
cargo build --profile dist -p tsrs_cli --features tsrs_core/plain-ptrs   # references
cd <38k-file codebase>
perf stat -x, -e instructions,cycles,dTLB-load-misses,dTLB-store-misses -- \
  /usr/bin/time -f "%e %U %S %R %F %M" tsrs -p . --noEmit --incremental false --pretty false --extendedDiagnostics --checkers 4
# while it runs: awk '/^AnonHugePages/{print $2}' /proc/<tsrs pid>/smaps_rollup; per mapping: /proc/<pid>/smaps
# (the arena is 0x4001_0000_0000 + 32 GiB; `hg` in VmFlags = advised)
```
