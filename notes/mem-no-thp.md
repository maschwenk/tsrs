# mem-no-thp: mimalloc without transparent-huge-page advice on its heap

The change: `crates/tsrs_cli/Cargo.toml` builds mimalloc with the crate feature `no_thp`
(`mimalloc = { version = "0.1.52", features = ["no_thp"] }`). libmimalloc-sys 0.1.49 then compiles mimalloc v3 with
`MI_NO_THP` on Linux and Android; on other targets the feature does nothing. The tsrs arena (`tsrs_core::reserve`)
keeps its own huge-page advice. Cargo unifies features per build, so a binary built in the same `cargo build` as
`tsrs_cli` (such as `tsrs-test` in the PGO instrumented step) gets the feature too; `tsrs_core`'s optional mimalloc,
`tsrs_parser`'s and `tsrs_testrunner`'s own declarations are unchanged.

## Mechanism

mimalloc v3 reserves its OS memory in 1 GiB arenas and, when `allow_thp` is on (the default), calls
`madvise(MADV_HUGEPAGE)` on each mapping (`unix_mmap` in `src/prim/unix/prim.c`). On a host with THP in `madvise`
mode, such as the Depot bench runners, the kernel then backs every 2 MiB block that a thread touches with a huge
page. mimalloc gives each thread its own pages per size class (64 KiB small, 512 KiB medium, 4 MiB large), so each
partly filled thread-local page costs a whole 2 MiB of RSS. tsrs runs one parse thread per core plus two OS threads
per checker (creation and checking); on the 64-vCPU runner this amplification was about 0.7 GiB of the front end's
2.0 GiB and about 15 MiB of each checker's 33 MiB (`notes/mem-64.md` section 3). `MI_NO_THP` compiles that `madvise`
out. The arena keeps its advice because a bump allocator wastes at most the partly used 2 MiB block under each
thread's finger, and the arena does need huge pages (`notes/linux-x86-round.md`: 2-6% less wall time).

There is no runtime setting with the same effect. `MIMALLOC_ALLOW_THP=0` makes mimalloc call
`prctl(PR_SET_THP_DISABLE)` (`_mi_prim_mem_init`), which turns THP off for the whole process and so for the arena
too: +4-10% check time at 4 checkers. `MIMALLOC_ARENA_EAGER_COMMIT=0` gives the same memory as `no_thp` for +7%
check time at 4 checkers (`notes/mem-64.md` section 3).

## Linux, 64 vCPUs (from `notes/mem-64.md` section 3)

Depot `depot-ubuntu-24.04-64` (EPYC 9R45, THP `madvise`), vscode, release build, probe runs nh2qw0m6kr and
5kwpqvqln2, 2-3 reps. "Huge-page heap" is the binary with the two checker-side memory changes of `notes/mem-64.md`
(now on main), "no_thp" the same binary with this change. Medians (8 checkers: range of 3 reps).

| checkers | peak RSS, huge-page heap | peak RSS, no_thp | change | check s, huge-page heap | check s, no_thp | instructions G |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `--noCheck` | 2.01 GiB | 1.33 GiB | -34% | | | 23.5 / 23.5 |
| 1 | 2.75 GiB | 2.08 GiB | -24% | 10.46 | 10.56 (+1%) | 117.0 / 117.0 |
| 4 | 2.98 GiB | 2.27 GiB | -24% | 2.75 | 2.85 (+3.5%) | 123.3 / 123.4 |
| 8 | 3.19-3.21 GiB | 2.44-2.45 GiB | -24% | 1.41-1.44 | 1.46-1.48 (+3%) | 128.7-128.9 / 128.8-128.9 |
| 16 | 3.53 GiB | 2.63 GiB | -25% | 0.80 | 0.83 (+4%) | 136.2 / 136.5 |
| 64 | 4.96 GiB | 3.36 GiB | -32% | 0.675 | 0.678 (0%) | 161.2 / 160.5 |

Instructions are the same within the run-to-run spread of work stealing; only page-fault and TLB work changes. Per
extra checker (4 -> 64) the peak grows 18.0 MiB instead of 33.0 MiB. Diagnostics were identical in every variant
(371 errors, `error TS` lines compared). The check-time cost is +2-5% across reps at 4-16 checkers and nothing at 64;
the 20-rep head-to-head below also shows a front-end cost that this probe, run before PR #120 sped parsing up, hid.

## macOS sanity check (M5 Max, 18 cores, no THP)

`libmimalloc-sys` defines `MI_NO_THP` only for Linux and Android, so on macOS nothing should change. The mimalloc
object code of the base build (main, dc8bfa8) and this build is identical (`otool -tv` of `libmimalloc.a`; the files
differ only in build paths). vscode, `--checkers 4 --extendedDiagnostics --pretty false`, interleaved, 2 runs each,
peak memory footprint from `/usr/bin/time -l`:

| binary | peak footprint MiB | instructions G |
| --- | ---: | ---: |
| base | 2,173 / 2,170 | 120.8 / 118.8 |
| no_thp | 2,167 / 2,189 | 119.4 / 120.5 |

Same within the run-to-run spread. Check time (2.68 / 2.00 s base, 2.93 / 2.12 s no_thp) is not comparable: another
job was using the machine (load average 9-16). `error TS` lines identical in all four runs (371 errors).

## 64 threads, head-to-head (`bench/compare.py`)

Depot `depot-ubuntu-24.04-64`, vscode, PGO `dist` build, mean of 20 interleaved reps per cell, `bun check`
1.4.3-canary.1+bbdc5a519 in both runs. Main: run xlz11b8b2f on the perf/64 head db191be (the same tree as main
dc8bfa8), 2026-10-06 23:53 UTC. no_thp: run wb4933tkmt on this branch (30e4520), 2026-10-07 00:18 UTC; the full table
with tsgo is `bench/results/compare/2026-10-07-30e4520db237-64t.md`. tsrs's default is 32 checkers on 64 cores.

| threads | compiler | wall s, main | wall s, no_thp | change | peak RSS, main | peak RSS, no_thp | change |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| default | tsrs | 0.547 ± 0.013 | 0.602 ± 0.012 | +10.1% | 4.13 GiB | 2.94 GiB | -28.8% |
| default | bun check | 0.837 ± 0.025 | 0.848 ± 0.014 | +1.3% | 2.87 GiB | 2.85 GiB | -0.6% |
| 4 | tsrs | 2.360 ± 0.025 | 2.514 ± 0.060 | +6.5% | 3.03 GiB | 2.29 GiB | -24.6% |
| 4 | bun check | 4.305 ± 0.027 | 4.394 ± 0.074 | +2.1% | 1.46 GiB | 1.45 GiB | -0.3% |
| 8 | tsrs | 1.311 ± 0.016 | 1.394 ± 0.022 | +6.3% | 3.23 GiB | 2.43 GiB | -24.7% |
| 8 | bun check | 2.341 ± 0.017 | 2.380 ± 0.019 | +1.6% | 1.59 GiB | 1.59 GiB | -0.1% |
| 16 | tsrs | 0.798 ± 0.010 | 0.858 ± 0.012 | +7.5% | 3.58 GiB | 2.64 GiB | -26.4% |
| 16 | bun check | 1.412 ± 0.013 | 1.446 ± 0.038 | +2.4% | 1.85 GiB | 1.85 GiB | 0.0% |
| 64 | tsrs | 0.541 ± 0.011 | 0.585 ± 0.011 | +8.1% | 5.02 GiB | 3.38 GiB | -32.6% |
| 64 | bun check | 0.823 ± 0.021 | 0.845 ± 0.014 | +2.7% | 2.87 GiB | 2.86 GiB | -0.5% |

tsrs by phase (`--extendedDiagnostics`, means; "rest" is process wall minus tsrs's total time, i.e. start-up and
exit):

| threads | parse s, main -> no_thp | check s, main -> no_thp | rest s, main -> no_thp |
| --- | --- | --- | --- |
| default | 0.096 -> 0.123 | 0.379 -> 0.397 (+4.7%) | 0.028 -> 0.038 |
| 4 | 0.095 -> 0.131 | 2.204 -> 2.314 (+5.0%) | 0.022 -> 0.029 |
| 8 | 0.096 -> 0.128 | 1.152 -> 1.195 (+3.7%) | 0.023 -> 0.030 |
| 16 | 0.096 -> 0.126 | 0.636 -> 0.657 (+3.3%) | 0.025 -> 0.033 |
| 64 | 0.095 -> 0.123 | 0.363 -> 0.368 (+1.4%) | 0.034 -> 0.045 |

Read:

- Peak RSS is what the probe predicted: 2.29 / 2.43 / 2.64 / 3.38 GiB at 4 / 8 / 16 / 64 checkers (probe: 2.27 /
  2.44 / 2.63 / 3.36). At the default tsrs now peaks at 2.94 GiB against bun's 2.85 GiB (was 4.13).
- The wall-time cost is larger than the probe's check-time numbers suggested. The second runner was slightly slower
  overall (bun +1-3%, tsgo 7.0.2 0-3% on the same cells), so the fair comparison is against bun on the same runner:
  the tsrs / bun wall ratio went 0.653 -> 0.710 at the default (+8.7%), and +4.4% / +4.6% / +5.0% / +5.3% at 4 / 8 /
  16 / 64. tsrs is still 1.41x faster than bun check at the default (was 1.53x).
- Most of the extra time is the front end: parse takes 27-37 ms more at every checker count (+28-39%; tsgo's parse
  moved +1-7% between the runners). The likely cause, not profiled here, is that the 64 parse threads now fault their
  heap in 4 KiB pages instead of 2 MiB ones. The probe saw a similar absolute cost (`--noCheck` total 0.31-0.32 ->
  0.33-0.34 s) when parse took 0.26 s; since PR #120 parse takes about 0.1 s, so the same cost is now about a third
  of it. Check time is +1-5%, as in the probe, and start-up plus exit +7-11 ms.
