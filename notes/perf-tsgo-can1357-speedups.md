# perf-tsgo-can1357-speedups: can1357's tsgo checker speedups, measured against tsrs, and what transfers (2026-10-09)

Can Bölük published [can1357/TypeScript `perf/checker-speedups-v3`](https://github.com/can1357/TypeScript/tree/perf/checker-speedups-v3)
(30 commits on microsoft/TypeScript main `6ad8c56f`, +3,905 / −298) with the claim that it makes tsgo "beat the
"$400k" slop rewrite" ([tweet](https://x.com/_can1357/status/2108468044959809852), 2026-10-09). This note measures the
branch against the tsgo it starts from, the tsgo tsrs is pinned to and tsrs; checks his comparison; classifies every
commit; and says what transfers to tsrs.

All numbers are from one Mac: Apple M5 Max, 6 performance + 12 efficiency cores, 128 GB, macOS 26.6, load average
6-11 (a browser kept ~1 core busy). Depot was skipped at the owner's request, so **there are no Linux numbers** apart from
the automatic pr-verify run on the port's pull request. The
Linux scoreboard may differ: three of his changes act mainly or only on Linux (huge pages for the Go heap,
prefaulting, and the zero-page faults his GC commit describes).

## Summary

- **His branch, Mac, nine bench projects:** 14-37% less wall time than tsgo main at the default 4 checkers (median
  26%), 13-27% less single-threaded, the same diagnostics. tsrs main is still **2.6-4.4x faster than his tsgo** at
  each tool's default and **1.6-2.2x single-threaded**, with 2.0-3.4x less peak memory.
- **Runtime vs checker:** on the Mac, 69-100% of his 4-checker gain (median ~86%) is in the 27 commits before his three
  runtime commits (GOGC=300 under a soft limit, no collection until the program is bound, THP for the Go heap). Those
  27 include allocation cuts that in Go also save collector work, prefaulting, and the checker partition. With the
  collector off (GOGC=off), his 27 checker and core commits remove 16-17% of tsgo's single-threaded instructions (27% on
  excalidraw; per-commit table below). His own Linux numbers put more of the gain in runtime work: prefaulting "removing it costs 19-36%
  wall", the deferred first collection -10% to -30%, THP -3% to -15%.
- **His comparison** is against Theo's tsc-rs (pingdotgg/ts-rust, npm `tsc-rs` 0.1.0 at the time), not tsrs.
  **Verdict: partly reproduced.** The speedup over stock tsgo is real and output-identical. "Beats tsc-rs" holds on the
  Mac for excalidraw (+30-33%, mostly his type-printing memo) and typeorm (+4-8%) but not for trpc, playwright, sentry
  or vscode, where tsc-rs 0.1.0 is 10-28% faster at 4 and 8 checkers. His Linux table shows wins on all six; the gap is
  plausibly his Linux-only runtime work, which could not be measured here, and his project setups (commits, TypeScript 7
  config fixes) are unpublished. All four compilers print identical diagnostics on all six of his projects. tsrs is
  1.6-2.6x faster than both at 4 and 8 checkers.
- **Output:** his four "baseline changes" are new tests (guards for his memo and lazy members), not edits of existing
  baselines; main and his branch print the same on all four. On the bench projects the only difference is
  history-dependent (drizzle-orm, below).
- **What transfers:** one change clears the bar by far and is ported: building unions by merging their sorted runs,
  which generalizes his sorted-input check and two-union merge (draft https://github.com/maschwenk/tsrs/pull/250,
  -10% single-threaded instructions on mikro-orm). His member-order changes and his type-printing memo were ported and
  measured too and stay under 1% on every bench project (the memo removes 13% on excalidraw, not a bench project), so
  they are not proposed. Everything else is already in tsrs, fixes a Go runtime or allocation cost with no Rust
  counterpart, or measures under 1% here.

## Binaries and method

| name | what | commit | build |
| --- | --- | --- | --- |
| pin | tsgo at the commit tsrs ports (`bin/tsgo-ref`) | microsoft/TypeScript `b85298b6a81f` | go1.27.0, `go build ./cmd/tsc`, CGO on |
| main | microsoft/TypeScript main = his merge base (59 commits, 51 touch `tsc/`) | `6ad8c56f9b5a` | same |
| can | his branch head | can1357 `9288c4aaf082` | same |
| can w/o GC/THP | his branch before its three runtime commits | `728ae2e5f62a` | same |
| tsrs | tsrs main | `cab0f26e` | `cargo build --release -p tsrs_cli` (fat LTO, no PGO) |
| tsc-rs | Theo's Rust port, the "rust" of his table | npm `tsc-rs@0.1.0` (`@tsc-rs/darwin-arm64`), pinned to `673a5f17d713` | prebuilt |

The tsgo builds are plain `GOTOOLCHAIN=auto go build -o <bin> ./cmd/tsc` from `tsc/` (the build info of `tsgo-ref`
shows the same flags); there is no `cmd/tsgo`.

Runs use `bench/run.py`'s invocation and parsing (`-p <project> --noEmit --incremental false --extendedDiagnostics
--pretty false`; `--singleThreaded` in the single mode, plus `RAYON_NUM_THREADS=1` for tsrs). Each project gets one
untimed warm-up per tool, then 5 reps (3 for his projects) in which every tool runs every mode and the tool order
rotates. Wall is the process wall clock, peak is `ru_maxrss`, instructions and CPU come from `proc_pid_rusage` on the
exited process (all threads, so tsgo's include its collector). Tables give medians; min-max spreads are in the
scratch tables and stay within a few percent except for single outliers. Default thread counts: tsgo 4 checkers,
tsrs 9 (half of 18 cores), tsc-rs 4.

## Results on the bench projects (Mac)

Default mode (each tool's own checker count), wall seconds:

| project | pin | main | can | can w/o GC/THP | tsrs | can / main | main / pin | tsrs faster than can |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| xstate-main | 0.46 | 0.47 | 0.34 | 0.36 | 0.12 | 0.73 | 1.00 | 2.71x |
| webpack | 0.79 | 0.74 | 0.58 | 0.63 | 0.20 | 0.78 | 0.94 | 2.94x |
| drizzle-orm | 1.09 | 1.09 | 0.80 | 0.81 | 0.25 | 0.74 | 0.99 | 3.18x |
| mikro-orm | 5.25 | 5.33 | 4.57 | 4.66 | 1.05 | 0.86 | 1.02 | 4.36x |
| cal-diy | 3.06 | 2.92 | 2.13 | 2.26 | 0.83 | 0.73 | 0.95 | 2.56x |
| formbricks-web | 3.47 | 3.88 | 2.53 | 2.75 | 0.79 | 0.65 | 1.12 | 3.21x |
| supabase-studio | 3.79 | 3.68 | 2.72 | 2.80 | 0.92 | 0.74 | 0.97 | 2.96x |
| t3code-server | 6.30 | 7.24 | 4.55 | 4.56 | 1.44 | 0.63 | 1.15 | 3.15x |
| vscode | 5.61 | 5.43 | 4.24 | 4.41 | 1.17 | 0.78 | 0.97 | 3.63x |

Single-threaded, wall seconds:

| project | pin | main | can | can w/o GC/THP | tsrs | can / main | tsrs faster than can |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| xstate-main | 1.03 | 0.98 | 0.83 | 0.87 | 0.48 | 0.85 | 1.73x |
| webpack | 1.70 | 1.73 | 1.38 | 1.41 | 0.86 | 0.80 | 1.61x |
| drizzle-orm | 2.12 | 2.03 | 1.72 | 1.72 | 0.99 | 0.85 | 1.75x |
| mikro-orm | 12.25 | 12.04 | 10.51 | 10.47 | 4.97 | 0.87 | 2.11x |
| cal-diy | 5.27 | 5.21 | 4.23 | 4.31 | 2.50 | 0.81 | 1.69x |
| formbricks-web | 7.16 | 7.04 | 5.76 | 5.86 | 3.23 | 0.82 | 1.78x |
| supabase-studio | 7.58 | 7.48 | 6.25 | 6.26 | 3.66 | 0.84 | 1.71x |
| t3code-server | 9.53 | 9.59 | 6.97 | 7.03 | 3.14 | 0.73 | 2.22x |
| vscode | 14.28 | 14.04 | 11.63 | 11.73 | 7.31 | 0.83 | 1.59x |

Peak RSS (GiB) and instructions (G, all threads) in the default mode:

| project | main peak | can peak | can w/o GC/THP peak | tsrs peak | main instr | can instr | can w/o GC/THP instr | tsrs instr |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| xstate-main | 0.65 | 0.68 | 0.59 | 0.25 | 31.0 | 19.0 | 24.6 | 9.9 |
| webpack | 1.04 | 1.02 | 0.88 | 0.44 | 50.0 | 31.9 | 42.6 | 17.0 |
| drizzle-orm | 1.50 | 1.47 | 1.29 | 0.61 | 76.2 | 42.7 | 55.3 | 20.0 |
| mikro-orm | 4.92 | 4.37 | 3.96 | 1.69 | 340.9 | 243.9 | 288.4 | 112.1 |
| cal-diy | 3.62 | 3.55 | 2.82 | 1.47 | 192.2 | 122.4 | 161.0 | 79.2 |
| formbricks-web | 4.81 | 4.99 | 4.07 | 1.70 | 234.5 | 153.1 | 198.9 | 77.2 |
| supabase-studio | 3.90 | 4.12 | 3.10 | 1.40 | 257.5 | 161.5 | 200.4 | 90.8 |
| t3code-server | 4.99 | 3.53 | 3.40 | 1.80 | 416.7 | 225.8 | 260.6 | 134.2 |
| vscode | 6.54 | 6.47 | 5.73 | 1.93 | 340.3 | 224.9 | 293.9 | 108.6 |

- His runtime commits trade memory back: without them his branch is 9-32% below main's peak (fewer allocations),
  with them it is back within -29% / +6% of main.
- Upstream main against the pin is within -6% / +2% except formbricks-web (+12%) and t3code-server (+15%), both only
  with 4 checkers (t3code: 366 G -> 417 G instructions at 4 checkers, unchanged single-threaded), so a partition
  effect upstream, not checker work.
- Errors are identical across the five builds on eight projects. drizzle-orm: pin and main print 10,846 errors with 4
  checkers and 10,845 single-threaded (a TS2769 at `drizzle-kit/tests/cli-check.test.ts(75,3)` depends on check
  history); tsrs prints the same 10,846 in both (its canonical history, notes/perf-order-independence.md); both of his
  builds print 10,845 in both modes. Not a checker-semantics change: the error disappears at his partition-weighting
  commit 8e217f4c0 (per-commit table below), which moves that file to a checker whose history hides it.

## His comparison: what he claims and how it reproduces

**The claim.** The tweet's image is a table for playwright, typeorm, excalidraw, trpc, vscode and sentry with
columns "stock c4", "ours c4", "rust c4", "vs rust c4" and the same at c8 (ms; "vs rust" = rust / ours − 1, e.g.
vscode c4 5,734 vs 5,881 ms = +2.6%). He claims a win on every cell: +2.6% to +51.5% at 4 checkers, +3.5% to +55.3% at
8. "Rust" is the "$400k" rewrite of the quoted tweet: Theo's tsc-rs (pingdotgg/ts-rust, "I burned ~$400k of Codex
tokens"), not tsrs; the only release before his tweet was npm `tsc-rs` 0.1.0 (2026-10-07 07:10 UTC). His commit
messages give the method: "interleaved runs on an 8-core Linux machine", 4 and 8 checkers, with all of the branch
applied (runtime commits included; with explicit GOGC/GOMEMLIMIT his GC policy steps aside, and nothing says he set
them). The branch has no benchmark script, and the replies give no machine, project commits or tsc-rs version (his
own reply links the branch and says he will upstream it).

**The inputs here.** HEAD of typeorm (`9f5a345`, `-p packages/typeorm`), excalidraw (`4c00f31`), trpc (`d756e59`) and
sentry (`f6a072c2`) as of 2026-10-09; playwright and vscode at the bench pins. TypeScript 7 rejects two of the
configs, so as in `bench/overlays/`: typeorm's `moduleResolution: node` (node10, removed) becomes `module: esnext,
moduleResolution: bundler`, and excalidraw's `baseUrl` becomes a `"*": ["./*"]` path. He must have made similar
changes; his are not published. sentry reports 12,513 errors (8,365 TS2339 and 3,749 TS6305 from unbuilt project
references) with every compiler; the others 0-658.

Median of 3 interleaved runs, ms; "ours w/o GC/THP" = `728ae2e5`:

| app | mode | stock (main) | ours (can) | rust (tsc-rs 0.1.0) | vs rust (Mac) | vs rust (his table) | ours w/o GC/THP | tsrs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| typeorm | c4 | 467 | 361 | 391 | +8.4% | +16.2% | 434 | 208 |
| excalidraw | c4 | 1,209 | 571 | 760 | +33.1% | +51.5% | 607 | 358 |
| trpc | c4 | 1,024 | 688 | 557 | −19.0% | +7.8% | 790 | 334 |
| playwright | c4 | 588 | 424 | 361 | −14.9% | +7.2% | 462 | 222 |
| sentry | c4 | 5,882 | 4,373 | 3,922 | −10.3% | +4.7% | 4,470 | 2,278 |
| vscode | c4 | 5,451 | 4,214 | 3,785 | −10.2% | +2.6% | 4,425 | 1,922 |
| typeorm | c8 | 392 | 297 | 308 | +3.7% | +17.1% | 327 | 165 |
| excalidraw | c8 | 1,382 | 520 | 675 | +29.9% | +55.3% | 540 | 263 |
| trpc | c8 | 856 | 629 | 515 | −18.1% | +12.2% | 673 | 261 |
| playwright | c8 | 625 | 410 | 294 | −28.3% | +14.4% | 426 | 179 |
| sentry | c8 | 5,291 | 3,714 | 2,888 | −22.3% | +3.5% | 3,822 | 1,782 |
| vscode | c8 | 4,402 | 3,280 | 2,796 | −14.7% | +15.0% | 3,580 | 1,271 |

Each tool at its own default (tsgo 4 checkers, tsc-rs 4, tsrs 9) and single-threaded:

| app | default: stock / ours / rust / tsrs | vs rust | single: stock / ours / rust / tsrs | vs rust |
| --- | --- | ---: | --- | ---: |
| typeorm | 498 / 368 / 390 / 154 | +6.0% | 1,106 / 952 / 926 / 618 | −2.8% |
| excalidraw | 1,215 / 595 / 753 / 267 | +26.5% | 1,748 / 1,270 / 1,425 / 938 | +12.2% |
| trpc | 1,016 / 696 / 556 / 256 | −20.2% | 2,008 / 1,634 / 1,434 / 976 | −12.2% |
| playwright | 624 / 425 / 376 / 156 | −11.5% | 1,257 / 1,059 / 940 / 640 | −11.3% |
| sentry | 6,091 / 4,507 / 3,904 / 1,747 | −13.4% | 13,087 / 10,576 / 9,272 / 6,670 | −12.3% |
| vscode | 5,312 / 4,184 / 3,885 / 1,170 | −7.2% | 14,014 / 11,635 / 11,614 / 7,324 | −0.2% |

- **Diagnostics:** stock tsgo, his tsgo, tsc-rs 0.1.0 and tsrs print identical (file, line, column, code) lists on all
  six projects in every mode (typeorm 31, excalidraw 0, trpc 658, playwright 13, sentry 12,513, vscode 371).
- **His gain over stock is smaller here than in his table:** 23-53% at c4 on the Mac, 35-60% in his table (vscode
  −23% vs −35%, playwright −28% vs −48%). That gap, and the c8 cells where his table wins and the Mac loses, fit the
  Linux-only part of his work: THP for the Go heap only acts on Linux, and prefaulting and the deferred first collection
  target the Linux zero-page and TLB-shootdown cost he measured ("removing it costs 19-36% wall"). An 8-core machine
  also gives tsc-rs less parallel parsing than 18 cores do.
- **Runtime share here:** "ours w/o GC/THP" carries 70-94% of his gain over stock at c4 on five of the apps and 31% on
  typeorm (a small run where the collector is a large share).
- **Verdict: partly.** Legit: his tsgo is much faster than stock tsgo on every project, prints the same diagnostics,
  and does beat tsc-rs 0.1.0 on excalidraw and typeorm. Not reproduced on the Mac: the wins on trpc, playwright,
  sentry and vscode, where tsc-rs is 10-28% faster at c4 and c8 (and 0-12% single-threaded). Whether they hold on his
  8-core Linux machine depends on the Linux-only runtime changes and on his unpublished project setups; this note
  cannot settle that without Linux runs. The claim does not concern tsrs, which is 1.6-2.6x faster than both at c4 and
  c8 and 1.4-1.7x single-threaded on these six projects.

## His commits one by one, in Go (Mac)

Each commit built on its own (`go build` at that commit), run single-threaded with `GOGC=off` so the collector does
not run and the count is the checker's own work (allocation savings still show, as fewer `mallocgc` calls).
Instructions retired, one run each (repeat runs of one binary move by about 0.5%). The first row is main; the others
are the change against the previous commit. The last column is drizzle-orm's error count with 4 checkers (normal GC).

| # | commit | xstate-main | webpack | drizzle-orm | excalidraw | drizzle, 4 checkers | subject |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| 00 | main | 19.21 G | 32.44 G | 41.24 G | 32.71 G | 10,846 | merge base |
| 01 | c8f9bc4ae | −1.9% | −1.3% | −2.4% | −1.5% | 10,846 | reduced allocations in member inheritance and mapType |
| 02 | 408129770 | −0.3% | +0.0% | −0.1% | −0.2% | 10,846 | skipped redundant parse task candidates |
| 03 | 532298bad | +0.1% | +0.4% | +0.7% | +0.6% | 10,846 | byte-equality fast path to path component matching |
| 04 | 57a6a9a62 | −0.1% | −1.0% | −1.2% | −1.1% | 10,846 | reduced relation cache key and maybe-stack overhead |
| 05 | 6e48288d9 | −1.0% | −1.2% | −1.1% | −1.1% | 10,846 | stored hot checker links in paged arenas |
| 06 | a3f430da0 | +0.3% | +0.2% | +0.4% | −0.3% | 10,846 | avoided resorting already sorted union inputs |
| 07 | 7749d0760 | −0.3% | +0.2% | +0.5% | −0.3% | 10,846 | reused member tables in never-reduction checks |
| 08 | ba343defd | −1.3% | −2.5% | −1.8% | −1.4% | 10,846 | precomputed sort keys for member ordering |
| 09 | 5d631e193 | −1.5% | −0.7% | −0.9% | −1.9% | 10,846 | merged union and intersection property caches |
| 10 | 97c6ea0d0 | −0.2% | −0.3% | +0.1% | +0.1% | 10,846 | normalized template literal spans in local buffers |
| 11 | 771aa8a03 | +0.1% | −0.0% | −0.9% | −0.4% | 10,846 | probed the template literal cache before normalizing |
| 12 | 6d557d0e3 | −2.9% | −2.9% | −1.3% | −1.0% | 10,846 | reused property order for generic instantiations |
| 13 | 15c911a9b | +0.3% | +0.0% | −0.2% | **−15.2%** | 10,846 | memoized outermost type printing per checker |
| 14 | 7dd635e6f | −0.1% | −0.1% | −0.8% | −0.1% | 10,846 | allocated common types from checker arenas |
| 15 | 628f51ceb | +0.2% | −0.0% | +0.4% | +0.1% | 10,846 | merged sorted unions without resorting |
| 16 | ddc71b880 | −0.0% | −0.2% | −0.5% | −0.3% | 10,846 | cheapened relater resets and simple relation checks |
| 17 | 8e217f4c0 | +0.3% | −0.2% | +0.3% | +0.0% | **10,845** | stopped weighting skipped files as checker work |
| 18 | 76097835c | −0.6% | −0.0% | −0.6% | −0.2% | 10,845 | packed simple relation keys into 64-bit map keys |
| 19 | a8c4fd445 | −2.4% | −0.4% | −1.1% | −0.8% | 10,845 | keyed alias-free mapper caches by type id |
| 20 | cf4a9c69c | −1.6% | −1.8% | −1.2% | −1.6% | 10,845 | indexed link store pages by offset instead of pointer |
| 21 | 6a4938280 | −0.5% | −0.3% | −1.1% | −0.2% | 10,845 | stopped creating symbol links on read-only lookups |
| 22 | 01a4839a8 | −3.8% | −3.4% | −2.9% | −2.5% | 10,845 | shared member layouts across generic instantiations |
| 23 | 1ba7d9fae | −0.2% | −0.2% | +0.8% | +0.2% | 10,845 | prefaulted fresh arena chunks and link store pages |
| 24 | 6a71d31de | +0.5% | +0.3% | +0.7% | +0.2% | 10,845 | restreamed the checker file partition |
| 25 | c3c9d329a | +0.1% | +0.3% | −1.1% | −0.1% | 10,845 | assigned node and symbol ids from per-checker blocks |
| 26 | e2c650f28 | −1.5% | −0.3% | −0.5% | +0.2% | 10,845 | instantiated members of generic instances on first use |
| 27 | 728ae2e5f | −0.1% | −2.2% | −1.1% | −1.7% | 10,845 | indexed merged symbols and symbol links by symbol id |
| 28 | 7ff2c357d | −0.3% | +0.5% | −0.1% | +0.3% | 10,845 | GOGC=300 under an adaptive limit (inactive under GOGC=off) |
| 29 | d062dbeca | −0.3% | −0.5% | −0.9% | −0.8% | 10,845 | deferred the first collection (inactive under GOGC=off) |
| 30 | 9288c4aaf | −0.5% | −0.2% | +0.0% | +0.1% | 10,845 | THP for the Go heap (Linux only) |

- Commits 01-27 remove 16-17% of tsgo's instructions on xstate-main, webpack and drizzle-orm and 27% on excalidraw.
  The member-order trio (ba343defd, 6d557d0e3, 01a4839a8) is the largest group, 5-9%; the type-printing memo is
  excalidraw's whole lead (−15.2%); his two union-sort commits are within noise on these four projects.
- drizzle-orm's history-dependent TS2769 disappears at 8e217f4c0, the partition-weighting commit: a scheduling effect,
  confirmed.

## What tsrs would gain: a probe

A temporary instrumented tsrs (branch `probe/can1357-estimates`, not pushed) counted calls and timed the code each
algorithmic change targets, single-threaded (`RAYON_NUM_THREADS=1`) on the nine bench projects. Times include the
probe's own timer calls, so they are upper bounds; shares are of the run's Check time.

| target (his commit) | what the probe found | upper bound |
| --- | --- | --- |
| type printing repeats (15c911a9b) | top-level `type_to_string` calls whose key was seen twice before (what a memo serves): drizzle-orm 5.3 of 725 ms, mikro-orm 15 of 5,606 ms, 0-3.6 ms elsewhere | <= 0.7% |
| union sort on already-sorted input (a3f430da0) | 0.14-0.33% of check (e.g. vscode 11 ms) | <= 0.35% |
| union sort, everything | mikro-orm 619 ms (~10% of check) in 29k unions of 9+ inputs, ~294 constituents in ~10 ascending runs, 45.5M comparisons; supabase-studio 38 ms, cal-diy 24 ms, vscode 10 ms in the same shape | the port below |
| `some_property_reduces_to_never` (7749d0760) | mikro-orm 871 ms inclusive (listing members 383, the ordered count map 106, lazy-constituent lookups 133, the check loop 215); cal-diy 224 (map 25), formbricks-web 251 (map 12) | map part 0.5-1.9%; his version needs every constituent's resolved members, which tsrs's lazy members avoid |
| `get_named_members` incl. sorting (ba343defd, 6d557d0e3) | 2-4% of check (formbricks-web 99 ms, cal-diy 72, webpack 25, vscode 140); `sort_symbols` alone 1-2.5% | the member-order candidate below |
| `is_type_reference_with_generic_arguments` (57a6a9a62) | 3.4M top-level calls and 5.0M steps on vscode; mostly the timer's cost | < 0.3% |
| maybe-stack lookups (57a6a9a62) | ~1.5M per run, nearly all with <= 8 entries | negligible |
| `get_merged_symbol` (728ae2e5f) | 32M calls on vscode, 12M on t3code-server, each an Fx lookup | not measured; a candidate for a later probe |

## Every commit, classified

"Runtime" = Go process or memory-management work with no tsrs counterpart (tsrs has no collector: leak arenas, exact
frees). "Scheduling" = which checker checks which file. "Checker" = the type checker's own algorithm or data
structures. "In tsrs" cites where tsrs already does the same or an equivalent. Estimates are tsrs single-threaded, from
the probe above; "Ported and measured" below has the measured ones.

| commit | what | class | output | in tsrs / verdict |
| --- | --- | --- | --- | --- |
| c8f9bc4ae | presized inherited member table; `mapTypeEx` allocates only after the first changed constituent; template literal spans cloned once per union expansion | checker (allocation) | same | Go allocation costs; Rust `Vec`s on mimalloc, no collector. Not measured; small. |
| 408129770 | skip building a parse-task candidate for a file that already has one | front end (allocation) | same | tsrs program construction differs (rayon parse pool). No. |
| 532298bad | byte-equality fast path before `EqualFold` in glob matching | front end | same | config/glob is ~1% of a run. No. |
| 57a6a9a62 | relation keys: simple key without the 192-byte key builder; `isTypeReferenceWithGenericArguments` memo bits; first 8 maybe-stack keys in the slice; object/object early exit in `isSimpleTypeRelatedTo` | checker | same | simple keys: in tsrs (`RelationKey::pair`, checker_09.rs `get_relation_key`). Generic-arguments memo: 3.4M calls / 5.0M steps on vscode, mostly timer overhead in the probe, <0.3%. Maybe stack: Go's cost was clearing a retained map; hashbrown's `clear` on an empty table is O(1) and tsrs unwinds the set as Go does. No. |
| 6e48288d9 | hot node and symbol links in id-indexed paged arenas instead of pointer-keyed Go maps | checker (data structure) | same | tsrs: `IdLinkStore` for `value_symbol_links` and `symbol_node_links`; the rest in Fx/hashbrown tables keyed by handle (links.rs), far cheaper than Go maps. All Fx hashing is 0.7-1.4% of instructions (docs/RUST.md "Techniques"). No. |
| a3f430da0 | `addTypesToUnion` skips the sort when the flattened inputs are already in order, else unstable sort; union plus one type by binary insertion; `compareTypeNames` alias short-circuit | checker | same | Not in tsrs (`add_types_to_union` stable-sorted every list). Ported, generalized: see "Ported". |
| 7749d0760 | `somePropertyReducesToNever` probes the constituents' member tables instead of building an ordered count map (<= 4 constituents) | checker | same | tsrs already skips one lazy constituent by name lookup (lazy members, #64526 port). The map is 106 of 871 ms in this function on mikro-orm (1.6% of check), 25 of 224 ms on cal-diy; his version needs every constituent's resolved member table, which defeats tsrs's lazy members. Est. 0.5-1% on 2-3 projects, below the bar. Note only. |
| ba343defd | `sortSymbols` with precomputed (file, file index) keys; one-pass partition in `getNamedMembers`; anonymous instantiations reuse the target's property order | checker | same | Not in tsrs (`compare_nodes` walks to the source file twice per comparison). Measured as part of the member-order candidate below. |
| 5d631e193 | one property cache with both lookup modes; `getApparentType` fast path for plain objects; fewer `getReducedType` calls | checker | same | tsrs: lazy property cache (notes/mem-lazy.md L6, checker_11.rs `get_union_or_intersection_property_lazy_cache`). Apparent-type fast path: a few flag tests in Rust. No. |
| 97c6ea0d0, 771aa8a03 | template literal spans in stack buffers; probe the template cache before normalizing | checker (allocation) | same | Go allocation costs; template-heavy code only. No. |
| 6d557d0e3 | instantiations of generic classes/interfaces reuse the target's (or first instantiation's) sorted property order, validated in O(n) | checker | same | Not in tsrs. Measured as part of the member-order candidate below. |
| 15c911a9b | memoize outermost `typeToString` per checker (guards: diagnostics, resolution cycles, member resolution in progress, instantiation budget and depth) | checker (memo) | same (his 3 new tests) | Not in tsrs. Upper bound from the probe (time in 3rd and later repeats): <= 0.7% of check on every bench project (drizzle-orm 5.3 of 725 ms; mikro-orm 15 of 5,606). Ported with tsrs's exactness rules (no objects created, shadow mode) on `perf/type-print-memo` and measured below. Note only; worth it where printing dominates (excalidraw: his 170,602 prints of 245 inputs; notes/perf-excalidraw-typefest.md). |
| 7dd635e6f | `TypeReference` and `LiteralType` from checker arenas; inherited tables cloned at final size | runtime (allocation) | same | tsrs allocates every type in thread arenas already (PORTING.md "Memory model"). No. |
| 628f51ceb | merge two sorted unions linearly; preallocate; pointer scan before binary search in small `containsType` | checker | same | Not in tsrs. Ported, generalized (merge of k runs). |
| ddc71b880 | pooled relater/inference state reset field by field; `isSimpleTypeRelatedTo` reordering; `getNormalizedType` early exit; lazy active-mapper caches | checker | same | Mostly Go write-barrier and map-clear costs. tsrs already builds no key for a fresh mapper (checker_11.rs `instantiate_type_with_alias_worker`). No. |
| 8e217f4c0 | skipped files get minimal weight in the checker partition | scheduling | history | tsrs: work stealing (#93) instead of a static partition; partition changes measured +-4% with no consistent winner (notes/perf-checker-scaling.md). No. |
| 76097835c | simple relation keys packed into a `uint64` map | checker | same | In tsrs (`RelationKey::pair`). |
| a8c4fd445 | alias-free active-mapper caches keyed by type id | checker | same | In tsrs (`active_mapper_cache_key`). |
| cf4a9c69c | link-store pages hold offsets into value blocks, not pointers | runtime (GC scan) | same | tsrs `IdLinkStore` is this design (groups of 16-bit offsets into 4,096-value chunks). |
| 6a4938280 | read-only lookups stop creating empty symbol links | checker (memory) | same | tsrs: reference kinds read without a record (`SymbolReferenceLinkStore::reference_kinds`), `try_get` in instantiation. Already. |
| 01a4839a8 | instantiations share their target's member layout instead of a name map | checker (memory) | same | tsrs lazy member tables (#64475 port, notes/lazy-members.md) avoid building instantiated tables in the first place. Overlaps; no. |
| 1ba7d9fae | prefault fresh arena chunks and link pages (one write per page) | runtime (Linux faults) | same | Measured and rejected for tsrs (docs/RUST.md: pre-faulting rejected, notes/linux-perf.md); tsrs advises its arena for THP (#89). |
| 6a71d31de | three FENNEL streaming passes for the checker partition | scheduling | history | tsrs: work stealing; locality assignment is `--checkerAssignment locality` (notes/perf-clustered-assignment.md). No. |
| c3c9d329a | node and symbol ids from per-checker blocks | runtime/memory | same | In tsrs (#94, notes/perf-round3.md). |
| e2c650f28 | instantiate members of generic instances on first use | checker | same (his new test) | In tsrs (lazy members, #64475 port, on by default). |
| 728ae2e5f | merged-symbol table and 12 symbol link stores indexed by symbol id | checker (data structure) | same | tsrs keeps `merged_symbols` in an `FxHashMap` (32M lookups on vscode single-threaded). Not measured; Fx lookups are a few ns. Candidate for a later probe. |
| 7ff2c357d | GOGC=300 under a soft memory limit (applies on macOS too) | runtime | same | No collector in tsrs. |
| d062dbeca | collector off until the program is bound, then 4x heap-in-use | runtime | same | No collector in tsrs. |
| 9288c4aaf | MADV_HUGEPAGE for the Go heap (Linux) | runtime (Linux) | same | tsrs: THP for the arena on Linux (#89); the mimalloc heap deliberately not advised (notes/mem-no-thp.md). |

## Ported and measured

Ranked by the probe, best first; each built on its own branch from main `cab0f26e`, checked for exactness with a
temporary shadow build (the new and the old computation side by side, abort on any difference) on 15 projects (the
nine above, mui-docs, playwright and his typeorm, excalidraw, trpc, sentry) single-threaded and in the default mode,
then measured against main: 5 interleaved reps, single-threaded instructions (`RAYON_NUM_THREADS=1`, all threads,
`proc_pid_rusage`; one binary repeats to 0.05-0.3%, up to 1.4% on cal-diy and vscode from a noisy first run), wall and
peak in both modes. Diagnostics were identical to main in every run.

| candidate (his commits) | branch | shadow | single-threaded instructions vs main | verdict |
| --- | --- | --- | --- | --- |
| 1. unions by merging sorted runs (a3f430da0, 628f51ceb, generalized) | `perf/union-sorted-inputs`, https://github.com/maschwenk/tsrs/pull/250 | 0 mismatches in ~9.3M unions | mikro-orm **−10.04%** (80.59 → 72.50 G; single-threaded wall −8.5%), cal-diy −0.55%, the other seven 0.00% to −0.20% | **ported** (draft) |
| 2. member order: keyed `sort_symbols`, anonymous instantiations and generic instantiations reuse the target's order (ba343defd, 6d557d0e3) | `perf/member-order` (pushed, no PR) | 0 mismatches in ~9.3M sorts and reuses; the conformance gate then caught an ordering bug (below), fixed | −0.10% to −0.87% (cal-diy −0.87%, webpack −0.84%, formbricks −0.56%, supabase −0.53%), vscode +0.71% (inside its 1.1% spread) | under 1% everywhere: **not ported** |
| 3. top-level type-printing memo (15c911a9b, with tsrs's rules: no object created, counters replayed, off under Go history, shadow mode) | `perf/type-print-memo` (pushed, no PR) | shadow clean (drizzle-orm 5,800 hits, excalidraw 42,597) | drizzle-orm −0.51%, webpack −0.17%, mikro-orm −0.13%, supabase within noise; **excalidraw −12.95%** (not a bench project; `Types` and `Instantiations` counters unchanged) | under 1% on every bench project: **not ported**; the code is there if a project like excalidraw joins the bench |

**1. Unions by merging sorted runs.** His two commits skip the sort when the inputs already arrive in order and merge
two unions directly; on his four projects (per-commit table) and in the probe that saves little. The probe found the
cost elsewhere: mikro-orm builds 29k unions from 9 or more inputs, about 294 constituents in about 10 ascending runs,
and Go's stable sort (insertion sort on blocks of 20, then SymMerge with rotations) spent 45.5M comparisons on them.
Each union input is sorted and unique and dropping `never` or nullable members keeps the order, so a new run can only
start where an input begins; `add_types_to_union` records those starts while flattening (one comparison per input) and
`merge_ascending_runs` merges the runs pairwise (at most n comparisons per round, ceil(log2 runs) rounds), with a
binary insertion for one type next to a run and a direct merge for two unions. Exact because `compare_types` ends with
the type id: distinct types never compare equal, so every correct sort of a list gives the same order, and identical
types (the only ties) end up adjacent for the existing dedup pass. Where `compare_symbols` is not a total order (the
auto-import alias resolver, `Program::source_files_complete`), the old stable sort runs unchanged. Gates on the branch:
`tools/regressions.sh` 28/28; conformance 13,458 / 12,779 / 12,779 in Go history and 13,458 / 12,778 / 12,778 in the
default mode (main's numbers); `tools/ci/determinism.sh` on xstate-main, webpack, drizzle-orm and the regression cases: 558 runs, all identical to the single-threaded output. Default-mode wall and peak did not move beyond noise on the Mac: at 9 checkers
mikro-orm's sorting is spread over the checkers and is not on the critical path. pr-verify on the pull request (Linux,
Depot 32 vCPU, release builds) agrees: identical diagnostics in 102 of 102 cells; single-threaded instructions
mikro-orm −9.91%, Compiler-Unions −1.64%, next-packages-next −1.33%, Compiler −0.83%, storybook −0.79%, mui-docs
−0.69%, nuxt −0.57%, cal-diy −0.50%, the other nine −0.32% to +0.16% (supabase-studio +0.16%, xstate-main +0.05%: one
comparison per input where nothing needs sorting); mikro-orm wall −5.0% (median over 1/4/16/32 checkers, −9.6% at one
checker).

**2. Member order.** In Go this trio is his largest checker gain (5-9% of instructions), because Go's sort calls a
closure that walks to each declaration's source file and hashes it in a Go map on every comparison, and because every
instantiation built and sorted a member map. In tsrs the same sorts are a smaller share (Rust's sort is adaptive,
`compare_nodes` already inlines the identity test, and lazy members (#64475 port) never build most instantiated member
tables), so the keyed sort and the order reuse together stay under 1%. One trap for whoever picks this up: the first
version listed an instantiation's members before marking them resolved and storing the table, where Go stores them
first. Listing calls `symbol_is_value`, which resolves aliases, and alias resolution can come back to the type: on
conformance/jsDeclarationsFunctionsCjs that printed two extra TS2303 ("Circular definition of import alias") for
CommonJS `module.exports.ii = module.exports.i`. The shadow projects never hit it; the conformance gate did. With the
flag and table set first (`49943ead`) the branch matches main exactly: regressions 28/28, conformance lists identical
to main's in both history modes.

**3. Type-printing memo.** The bench projects rarely print the same type three times; excalidraw's relation
elaboration does (notes/perf-excalidraw-typefest.md), where the memo removes 13% of single-threaded instructions.
This is the "lazy formatting of error arguments" item of notes/perf-round2-followups.md reached from another side.

**Not attempted (estimated under the bar):** `some_property_reduces_to_never` without the count map (7749d0760;
needs resolved member tables, conflicts with lazy members; map part 0.5-1.9% on mikro-orm), the generic-arguments memo
and inline maybe stack (57a6a9a62; < 0.3%), id-indexed link stores and merged symbols (6e48288d9, 728ae2e5f; Fx
lookups; `get_merged_symbol` runs 32M times on vscode and is worth its own probe), template literal buffers, relater
resets and the apparent-type fast paths (Go allocation and write-barrier costs).

## Output-changing changes, for when the pin moves

- None of his commits changes output on existing baselines; his four new tests pass identically on main and his
  branch. The pin and main differ on `excessiveInstantiationWhilePrintingReportedEachTime` by one truncated character,
  an upstream change.
- His partition commits (8e217f4c0, 6a71d31de) change which checker checks which file, which flips history-dependent
  diagnostics such as drizzle-orm's TS2769. tsrs has no static partition in its default mode (work stealing,
  canonical history), so nothing to adopt.
- Upstream since the pin, for the next pin move: #64553 ("Cache inferences made from type arguments") changes
  inference results (`correlatedUnions` baselines) and must come with the pin; #64649 (one node builder per emit
  resolver) concerns emit, not `--noEmit` checking.

## Not done

- Linux: no Depot runs (owner's choice for this round). His runtime commits are Linux-centric, so the Linux scoreboard
  can rank his branch better against tsc-rs than the Mac does; tsrs's lead over his tsgo there is not measured.
- His exact projects: the commits and TypeScript 7 config fixes he used are not published; the inputs above are HEAD
  as of 2026-10-09 with the minimal fixes described.
- `nuxt` (part of `tools/ci/determinism.sh`'s default set) is not checked out locally; determinism was run on
  xstate-main, webpack, drizzle-orm and the regression cases.

## Files

Scratch tables and logs (not committed): the interim log, the harness (`h2h.py`, `run.py`'s invocation plus
`proc_pid_rusage`), raw JSONL for the 450-run bench, the 360-run comparison on his projects, the per-commit runs and
the A/B runs.
