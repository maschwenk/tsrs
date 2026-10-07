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
on the 8-vCPU README benchmark (4 checkers) it would show as a small wall-time regression.

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

Pending: the `bench-compare` workflow on this branch (vscode, 20 reps, default / 4 / 8 / 16 / 64 threads).
