# linux-perf: first Linux profile (page faults, transparent huge pages, pre-faulting)

Update (2026-10-05): pointer compression later moved the arena chunks out of mimalloc into a reservation that did not
ask for huge pages, which cost 2-6% wall time on Linux until the reservation advised its chunks itself
(notes/linux-x86-round.md). The measurements below are from before that, with mimalloc-backed chunks.

All earlier performance work was measured on an 18-core Apple Silicon Mac, where kernel time is a large share
(the private monorepo: ~15 s user + ~8 s sys with one checker). The users run Linux (x86_64 cloud sandbox VMs,
arm64 dev VMs, x86_64 CI). This is the first measurement there, and an A/B of transparent huge pages (THP) and
pre-faulting for the arena chunks (`tsrs_core::arena`, notes/mem-recycle.md).

## Machine

A cloud sandbox microVM (the environment coding agents get for the private monorepo), x86_64, 18 vCPUs, 47 GiB,
no swap, no cgroup limits inside the VM, Ubuntu 24.04, kernel 7.2.6, 4 KiB pages, `perf` PMU events available.
THP `enabled` = `madvise`, `defrag` = `madvise`; all mTHP sizes `never`; `AnonHugePages` 0 kB system-wide when
idle. The VM was stopped and resumed halfway and landed on a different host CPU:

- host 1: Intel Xeon Platinum 8259CL @ 2.50 GHz (Cascade Lake);
- host 2: AMD EPYC 9554 (Zen 4); no `dTLB-store-misses` event.

Neighbours are noisy: on host 1 the same binary and mode got 25% slower over half an hour (cycles up at equal
instructions), so only interleaved runs are compared and the absolute numbers carry ~±5%.

Project: the private monorepo at a newer commit than the Mac numbers (37,510 files, 5.91 M lines), with its
workspace packages unbuilt, so it reports ~39,700 error lines (same output for every binary and mode).
`--noEmit --incremental false --extendedDiagnostics`, `/usr/bin/time -v` inside `perf stat`;
`AnonHugePages` sampled from `/proc/<pid>/smaps_rollup` every second (maximum reported).

## Baseline: release 0.1.5 (npm `@maschwenk/tsrs-linux-x64`, PGO + fat LTO), medians of 3 interleaved runs

| host | mode | wall s | user s | sys s | peak RSS GiB | page faults | AnonHugePages GiB | instructions | cycles |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `--checkers 1` | 47.9 | 51.8 | 3.5 | 5.98 | 29 K | 5.78 | 219 G | 165 G |
| 1 | 4 checkers | 19.6 | 68.1 | 4.1 | 7.66 | 23 K | 7.42 | 299 G | 219 G |
| 2 | `--checkers 1` | 37.1 | 36.9 | 5.3 | 5.99 | 27 K | 5.76 | 219 G | 139 G |
| 2 | 4 checkers | 16.2 | 50.6 | 7.7 | 7.66 | 19 K | 7.46 | 298 G | 188 G |
| 2 | `--singleThreaded` (1 run) | 41.5 | 35.7 | 5.8 | 5.57 | 24 K | 5.43 | 218 G | 135 G |

For scale, TypeScript 7.1.0-dev.20260930.4 `tsc` (native, same commit tsrs ports) on host 2, one run each:
`--checkers 1` 127.1 s wall, 177.9 s user, 41.9 s sys, 14.6 GiB, 4.35 M page faults, no huge pages;
4 checkers 75.3 s wall, 285.1 s user, 61.1 s sys, 22.6 GiB, 6.7 M page faults. tsrs is 3.4x / 4.6x faster
in wall time there.

Single-thread speed: one checker takes 2.5-3.5x the Mac's user time (IPC 1.33 on Cascade Lake, 1.58 on Zen 4;
the Mac numbers are from an older commit of the project).

## Page faults are already cheap: mimalloc gives the arena huge pages

The arena takes its chunks from the global allocator (mimalloc v2), and mimalloc calls `madvise(MADV_HUGEPAGE)`
on the OS memory it maps for its arenas and huge objects (`allow_thp`, default on). So with THP in `madvise` mode
almost the whole process is on 2 MiB pages without any change: 5.8 of 6.0 GiB (one checker) and 7.4 of 7.7 GiB
(4 checkers) are `AnonHugePages`, and the run takes ~25 K page faults instead of the ~1.5 M a 4 KiB-page run
needs. On macOS (16 KiB pages, no THP for anonymous memory) the same memory needs ~400 K faults (estimate: 6 GiB /
16 KiB), the likely source of the Mac's large sys time; on Linux sys is 3.5-7.7 s of which ~2 s is the parse phase's file I/O.

## A/B: arena chunks via `mmap`, THP, pre-faulting (`TSRS_ARENA`, branch `fix/linux-thp`, not landed)

Experiment build (commit e28a472, plain `cargo build --release` from 0.1.5, hence slower than the release):
`TSRS_ARENA` = `mmap` (chunks from `mmap`, no advice), `thp` (2 MiB-aligned `mmap` + `MADV_HUGEPAGE`),
`populate` (`mmap` + `MADV_POPULATE_WRITE` of the whole chunk), `thp,populate`; `MIMALLOC_ALLOW_THP=0` turns THP
off for the whole process (mimalloc sets `PR_SET_THP_DISABLE`), i.e. what a `never` host looks like.

Host 1, medians of 3 interleaved rounds:

| mode | 1 checker wall / sys s | RSS GiB | faults | AHP GiB | 4 checkers wall / sys s | RSS GiB | faults | AHP GiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| base (mimalloc chunks) | 58.0 / 3.6 | 5.98 | 32 K | 5.79 | 24.7 / 4.0 | 7.69 | 29 K | 7.45 |
| `mmap` | 59.7 / 5.4 | 5.81 | 1,055 K | 1.78 | 25.3 / 6.4 | 7.48 | 1,294 K | 2.48 |
| `thp` | 56.7 / 3.6 | 5.84 | 38 K | 5.65 | 23.7 / 4.0 | 7.50 | 38 K | 7.27 |
| `populate` | 61.2 / 4.9 | 8.13 | 33 K | 1.75 | 27.3 / 5.3 | 8.92 | 31 K | 2.41 |
| `thp,populate` | 57.7 / 3.8 | 8.15 | 30 K | 7.99 | 25.7 / 4.9 | 8.94 | 29 K | 8.70 |
| `MIMALLOC_ALLOW_LARGE_OS_PAGES=1` | 58.5 / 3.4 | 5.97 | 34 K | 5.76 | 24.6 / 4.4 | 7.66 | 26 K | 7.46 |

Host 1 later (slower period), and host 2, THP off:

| host, mode | 1 checker wall / sys s | RSS GiB | faults | 4 checkers wall / sys s | RSS GiB | faults |
| --- | --- | --- | --- | --- | --- | --- |
| 1, base | 73.9 / 5.0 | 5.98 | 38 K | 31.5 / 6.2 | 7.67 | 32 K |
| 1, THP off | 82.7 / 8.5 (+12%) | 5.54 | 1,535 K | 35.4 / 11.0 (+12%) | 7.18 | 1,917 K |
| 1, THP off + `populate` | 87.2 / 9.1 | 7.84 | 521 K | 36.6 / 12.3 | 8.59 | 649 K |
| 2, base | 46.2 / 5.7 | 5.96 | 32 K | 19.4 / 8.4 | 7.67 | 24 K |
| 2, THP off | 51.2 / 9.2 (+11%) | 5.57 | 1,527 K | 21.8 / 11.8 (+12%) | 7.19 | 1,911 K |
| 2, `populate` | 49.8 / 9.5 | 8.14 | 26 K | 21.4 / 9.7 | 8.93 | 26 K |

Host 2, base vs `thp` only, interleaved (10 rounds with 4 checkers, 6 with one):

| mode | wall s | user s | sys s | RSS GiB | cycles | AHP GiB |
| --- | --- | --- | --- | --- | --- | --- |
| base, 4 checkers | 18.51 (17.29-20.97) | 60.6 | 7.2 | 7.67 | 224.5 G | 7.45 |
| `thp`, 4 checkers | 18.48 (17.73-20.21) | 60.3 | 7.2 | 7.51 | 222.9 G | 7.27 |
| base, 1 checker | 46.09 (43.79-46.12) | 46.2 | 5.3 | 5.97 | 173.3 G | 5.72 |
| `thp`, 1 checker | 45.68 (44.08-48.54) | 46.0 | 5.4 | 5.84 | 172.2 G | 5.63 |

Conclusions:

- THP is worth ~11-12% wall and 3.4-4.7 s sys here, at ~0.45 GiB of RSS (huge-page fragmentation in the mimalloc
  heap), and the default build already gets it on `madvise` and `always` hosts. A host with THP `never` (or a
  container that disables it) pays it in full; nothing in tsrs can change that short of `MAP_HUGETLB` pools.
- Explicit `MADV_HUGEPAGE` on the arena chunks changes nothing in wall time (-0.2% / -0.9%, inside the noise;
  the 3-round medians that looked like -4% did not survive 10 rounds). It does lower peak RSS by 2.1-2.5%
  (0.13-0.19 GiB): the chunks no longer go through mimalloc's huge-object path. Not landed: the bar was a wall-time
  win; a memory-only follow-up would be cheap (the `thp` mode as the Linux default) if 2% of RSS matters.
- Pre-faulting whole chunks is strictly worse: the doubling chunks are on average far from full, so peak RSS
  grows by 1.2-2.2 GiB (+16% to +36%), and wall does not improve (the faults it removes were 2 MiB faults, ~25 K of
  them). With THP off it removes two thirds of the faults but sys time still goes up.
- `mmap` without advice loses THP (`enabled=madvise`), which shows that mimalloc's advice is what currently
  provides it; any future change of chunk allocation on Linux must keep `MADV_HUGEPAGE`.
- `MIMALLOC_ALLOW_LARGE_OS_PAGES=1`: no change (as on macOS).

## Parse phase: `--checkers 1` (parallel parse) vs `--singleThreaded`

`--listFilesOnly` (program construction only), medians of 3:

| host | mode | wall s | user s | sys s |
| --- | --- | --- | --- | --- |
| 1 | parallel | 1.36 | 6.63 | 2.33 |
| 1 | `--singleThreaded` | 5.98 | 5.09 | 1.04 |
| 2 | parallel | 1.01 | 4.22 | 1.96 |
| 2 | `--singleThreaded` | 4.58 | 3.08 | 1.47 |

Parallel parse costs 0.5-1.3 s more sys than the sequential one (Mac: ~4 s more over the whole run), and is
4.4-4.5x faster in wall time.
