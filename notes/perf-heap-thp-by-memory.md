# perf-heap-thp-by-memory: huge pages for the mimalloc heap when memory allows (not adopted)

The idea: `notes/mem-no-thp.md` traded speed for memory by not advising the mimalloc heap with `MADV_HUGEPAGE`. Turn
the advice back on at run time when the machine has memory to spare, and keep today's behaviour otherwise. Built,
measured on the 8-, 16- and 64-vCPU Depot runners, not adopted. The code is on branch `probe/heap-thp-by-memory`
(commit de2ecbfc).

In short:

- **The gain is real but only on large programs.** The seven projects that peak above 1.3 GiB at 8 checkers run
  1-4% faster, mostly 2-3.5% (same binary, advice off against on, interleaved: vscode at the default 32 checkers on
  64 vCPUs -2.9%, standard error 0.5%, 20 reps). The ten smaller ones gain nothing: within ±3%, in both directions.
- **Across the 17-project table it is about 1%.** Branch against merge base on the same runner (pr-verify): wall
  geomean -0.8% at 8 checkers and -1.1% at 16 on 16 vCPUs, -0.9% at the default 32 on 64 vCPUs. Below the 2% bar.
- **The memory cost is large everywhere.** Peak RSS +17-28% on the large projects on 16 vCPUs (+23-37% at the
  default on 64), +43-128% on the small ones: the extra is per thread, so it does not shrink with the program.
  Against `bun check` on the 16-vCPU table, tsrs's peak goes from below bun's on 11 of 17 projects to above it on 14.
- **Instructions do not move** (single-threaded counts within ±0.003%) and diagnostics are identical in all 153
  pr-verify cells. The time saved is check time and user CPU (fewer TLB misses); minor faults drop 2.5-6x, parse
  time and system time do not change.
- The rule armed on every bench machine (29.8, 56.8 and 246 GiB available against a 20 GiB threshold).

## What was built

- `crates/tsrs_cli/Cargo.toml`: mimalloc-safe without `no_thp`. libmimalloc-sys2 turns that feature into the CMake
  setting `MI_ALLOW_THP=OFF`, a compiled `allow_thp = 0`; the `madvise(MADV_HUGEPAGE)` in `unix_mmap` stays compiled
  in. With `allow_thp = 0` at start-up, `_mi_prim_mem_init` sets `has_transparent_huge_pages = false` and calls
  `prctl(PR_SET_THP_DISABLE, 1)`, so setting the option to 1 later changes nothing: a run-time switch needs the
  feature gone. Without it the compiled default on Linux is 2 (`FULL`: advise, and purge in 2 MiB units).
- `crates/tsrs_cli/src/main.rs`: a function in `.init_array` sets `allow_thp` with `mi_option_set_default` (so an
  explicit `MIMALLOC_ALLOW_THP` still wins): 1 when the memory available to the process is at least 20 GiB, else 0.
  `TSRS_HEAP_THP=0|1` forces it. `--extendedDiagnostics` prints `Heap huge pages` (0/1) and `Available memory (MiB)`
  in the tsrs-only table. The #254 `prctl(PR_SET_THP_DISABLE, 0)` moved into the same function.
- `tsrs_core::memsplit::available_memory()`: `MemAvailable`, lowered to what the tightest cgroup v2 `memory.max` of
  the process's cgroup and its ancestors leaves (current usage minus `inactive_file`). Same name, signature and
  semantics as the helper on `perf/default-checkers-16`, but it allocates nothing on Linux (stack buffers, `open` and
  `read`), so either branch can keep this version. Checked in a Linux container: `docker run -m 3g` reports 3,068
  MiB, an unlimited container the VM's 15,473 MiB.

Three things found on the way, worth knowing for any later change here:

- **The Rust runtime allocates before `main`.** strace of `tsrs --version`: the first 1 GiB arena is mapped and
  advised after the runtime reads `/proc/self/maps` (the main thread's stack guard) and before `main`'s first line.
  mimalloc advises an arena once, when it reserves it, so a decision made in `main` comes too late for the first
  GiB, which holds the whole heap of a small program. Hence `.init_array`: it runs after mimalloc's own constructor
  (priority 101) and before the runtime. The unit-test binary leaves it out (`cfg(not(test))`).
- **libmimalloc-sys2's `mi_option_allow_thp` is 37, mimalloc 2's number.** In mimalloc 3.5's `mi_option_t` it is
  43; 37 is `mi_option_page_max_candidates`. The branch declares 43 itself, with a Linux test that reads the compiled
  default (2) through it.
- mimalloc's page map (4.3 MiB, mapped in its constructor) is never advised: its size is not a multiple of 2 MiB.

strace with each setting (Linux container, THP `madvise`): advice off maps the arena without `madvise`, on and
`MIMALLOC_ALLOW_THP=2` map it with `MADV_HUGEPAGE`. On webpack in the container, `AnonHugePages` peaked at 266 MiB with
the advice off (the tsrs arena's chunks) and 720 MiB with it on; RSS 469 -> 757 MiB.

## Same binary, advice off against on (perf-probe)

Depot `perf-probe.yml` with `tools/perf/leafprobe.py`, `cargo build --release` of the branch, variants
`TSRS_HEAP_THP=0` (off), `TSRS_HEAP_THP=1` (on) and `MIMALLOC_ALLOW_THP=2` (full), interleaved, 5 reps, medians;
"paired" is the median of the per-rep ratios. All runners AMD EPYC 9R45, kernel 6.12.109, THP `madvise`. At each
runner's default checker count:

| project | 8 vCPU, 8 checkers: wall (paired) / peak | 16 vCPU, 8 checkers | 64 vCPU, 32 checkers |
| --- | --- | --- | --- |
| vscode | -1.8% / 1.91 -> 2.20 GiB (+15%) | -2.6% / 1.92 -> 2.28 GiB (+19%) | -4.1% / 2.60 -> 3.45 GiB (+33%) |
| formbricks-web | -2.6% / 1.65 -> 1.95 GiB (+18%) | -2.5% / 1.68 -> 2.12 GiB (+27%) | -2.4% / 2.60 -> 3.61 GiB (+39%) |
| t3code-server | -1.7% / 1.75 -> 2.01 GiB (+15%) | -2.8% / 1.77 -> 2.10 GiB (+19%) | -5.1% / 2.71 -> 3.43 GiB (+27%) |
| supabase-studio | -2.2% / 1.36 -> 1.62 GiB (+19%) | -2.8% / 1.38 -> 1.69 GiB (+22%) | -2.5% / 2.19 -> 2.94 GiB (+35%) |
| xstate-main | +1.7% / 270 -> 476 MiB (+76%) | +2.1% / 287 -> 524 MiB (+83%) | +0.2% / 456 -> 938 MiB (+106%) |
| webpack | -1.4% / 436 -> 634 MiB (+45%) | +1.2% / 448 -> 696 MiB (+55%) | -0.2% / 692 -> 1188 MiB (+72%) |

- The other cells agree: large projects -1 to -6% (paired) at every checker count from 1 to 64, small ones within
  ±4% with no direction. With one checker (the parse still parallel) the large projects gain 1-5% for +3-7% peak.
- Where the time goes, 64 vCPU, 32 checkers: vscode parse 0.100 -> 0.097 s, check 0.418 -> 0.413 s, user CPU 14.51
  -> 14.29 s, minor faults 33.0k -> 7.0k; t3code-server check 1.361 -> 1.281 s. System time is the same. Parse no
  longer pays for the missing advice: since the pool was capped at 32 threads it does not queue on `mmap_lock`
  (`notes/perf-parse-heap-faults.md`), which was most of the +9% that `notes/mem-no-thp.md` measured.
- `allow_thp = 2` (purge in 2 MiB units) is no different from 1: cells differ by a few percent in either
  direction, at the same peak.
- vscode, 64 vCPU, 32 checkers, 20 interleaved reps (run qx1lf0r9hh), mean ± sd: 0.566 ± 0.010 s off, 0.550 ±
  0.009 s on, paired -2.9% (standard error 0.5%); peak 2.60 -> 3.46 GiB. That is the README's headline cell, where `bun check`
  peaks at 2.86 GiB.

## All 17 projects, branch against merge base on the same runner (pr-verify)

Release builds of main 83e19356 and the branch; the branch arms the rule on both runners. Wall change per cell,
peak in MiB:

| project | 16 vCPU, 8 checkers | 16 vCPU, 16 checkers | 64 vCPU, 32 checkers | 64 vCPU, 64 checkers |
| --- | --- | --- | --- | --- |
| vscode | -2.0% / 1962 -> 2322 | -1.6% / 2126 -> 2645 | -3.3% / 2.60 -> 3.45 GiB | -2.4% / 2.99 -> 4.37 GiB |
| mui-docs | -2.4% / 1331 -> 1558 | -2.7% / 1620 -> 1926 | -0.3% / 2.04 -> 2.57 GiB | -2.2% / 2.67 -> 3.53 GiB |
| cal-diy | -3.3% / 1475 -> 1764 | -1.9% / 1906 -> 2291 | -1.5% / 2.46 -> 3.13 GiB | -2.8% / 3.46 -> 4.55 GiB |
| formbricks-web | -2.9% / 1715 -> 2172 | -6.3% / 2004 -> 2566 | -4.4% / 2.62 -> 3.60 GiB | -3.1% / 3.57 -> 5.04 GiB |
| supabase-studio | -3.6% / 1416 -> 1734 | -2.6% / 1715 -> 2154 | -3.2% / 2.19 -> 2.94 GiB | -2.4% / 3.08 -> 4.27 GiB |
| t3code-server | -3.2% / 1810 -> 2134 | -1.0% / 2168 -> 2581 | -2.1% / 2.69 -> 3.42 GiB | -1.7% / 3.35 -> 4.45 GiB |
| mikro-orm | -1.0% / 1657 -> 1934 | -4.1% / 1985 -> 2325 | -2.5% / 2.61 -> 3.21 GiB | +1.0% / 3.33 -> 4.36 GiB |
| xstate-main | +0.0% / 291 -> 534 | +0.9% / 341 -> 638 | +2.6% / 468 -> 958 | +0.7% / 683 -> 1360 |
| webpack | +2.3% / 460 -> 714 | +1.9% / 531 -> 859 | +0.8% / 701 -> 1209 | +1.5% / 898 -> 1624 |
| Compiler | +2.4% / 115 -> 240 | +1.1% / 134 -> 306 | +1.1% / 176 -> 401 | +2.0% / 185 -> 428 |
| Compiler-Unions | +2.0% / 115 -> 232 | +0.0% / 133 -> 287 | +0.0% / 173 -> 392 | +0.0% / 180 -> 389 |
| next-packages-next | -1.4% / 621 -> 890 | -0.4% / 691 -> 1053 | -0.5% / 782 -> 1346 | +1.0% / 950 -> 1714 |
| next-root | +1.2% / 441 -> 687 | -0.5% / 487 -> 804 | +0.0% / 629 -> 1109 | +0.0% / 738 -> 1396 |
| storybook | -0.8% / 454 -> 705 | +0.0% / 513 -> 838 | -0.6% / 642 -> 1148 | -3.8% / 776 -> 1486 |
| nuxt | -0.4% / 445 -> 683 | +0.5% / 517 -> 845 | +0.0% / 634 -> 1119 | -0.6% / 796 -> 1437 |
| playwright | -0.5% / 441 -> 665 | +1.3% / 490 -> 792 | -0.9% / 635 -> 1114 | +2.5% / 834 -> 1496 |
| drizzle-orm | +0.4% / 595 -> 890 | -2.7% / 792 -> 1222 | +0.6% / 1102 -> 1838 | +0.0% / 1617 -> 2613 |
| **geomean** | **-0.8% / +44%** | **-1.1% / +50%** | **-0.9% / +61%** | **-0.6% / +66%** |

16 vCPU: 5 reps per cell (run n5cqgk061h); 64 vCPU: 3 reps, checkers 1, 4, 16, 32 and 64 (run bwv180tsbr; geomeans
-1.9% / -1.6% at 1 / 4 checkers, peak +31% / +36%). Single-threaded instructions changed by -0.003% to +0.001% on every
project, and diagnostics were identical in 51 of 51 and 102 of 102 cells.

## bench.yml from the branch (PGO `dist` build, run dr013g4s9c)

The bench workflow's `bench-result` for de2ecbfc, against the nine results on main since #254 (24441a30 to 7c3e1948,
whose single-threaded instructions agree within ±0.1%). Cross-run wall is noisy: each of those nine main runs,
compared with the median of the other eight, moves the 16-vCPU `wide` geomean by -4.0% to +8.1%, and the tsrs/bun
wall ratio by -1.4% to +1.1%. The branch: `wide` wall geomean -3.4%, tsrs/bun ratio -1.2%; `checkers16` -4.4% and
-2.3%. Peak geomean +45% (`wide`) and +50% (`checkers16`). The headline table, against `bun check` from the same run:

| project | tsrs/bun wall, main median -> branch | tsrs peak, main -> branch | bun peak | tsrs / bun peak, main -> branch |
| --- | --- | --- | ---: | --- |
| vscode | 0.901 -> 0.881 (-2.2%) | 1.93 -> 2.27 GiB (+18%) | 1.87 GiB | 1.03x -> 1.22x |
| xstate-main | 0.816 -> 0.807 (-1.1%) | 280 -> 514 MiB (+83%) | 394 MiB | 0.71x -> 1.31x |
| webpack | 0.700 -> 0.675 (-3.5%) | 455 -> 702 MiB (+54%) | 495 MiB | 0.92x -> 1.42x |
| mui-docs | 0.155 -> 0.152 (-2.2%) | 1.32 -> 1.54 GiB (+17%) | 11.32 GiB | 0.12x -> 0.14x |
| Compiler | 0.623 -> 0.644 (+3.5%) | 99 -> 197 MiB (+100%) | 221 MiB | 0.45x -> 0.89x |
| Compiler-Unions | 0.630 -> 0.637 (+1.0%) | 100 -> 196 MiB (+96%) | 232 MiB | 0.43x -> 0.85x |
| cal-diy | 0.568 -> 0.553 (-2.7%) | 1.44 -> 1.73 GiB (+21%) | 1.31 GiB | 1.10x -> 1.32x |
| formbricks-web | 0.824 -> 0.783 (-4.9%) | 1.68 -> 2.11 GiB (+26%) | 1.62 GiB | 1.04x -> 1.31x |
| supabase-studio | 0.581 -> 0.565 (-2.7%) | 1.38 -> 1.69 GiB (+22%) | 1.22 GiB | 1.14x -> 1.38x |
| t3code-server | 0.628 -> 0.603 (-4.0%) | 1.77 -> 2.10 GiB (+19%) | 1.13 GiB | 1.58x -> 1.86x |
| mikro-orm | 0.624 -> 0.617 (-1.2%) | 1.62 -> 1.89 GiB (+17%) | 1.40 GiB | 1.16x -> 1.35x |
| next-packages-next | 0.818 -> 0.812 (-0.8%) | 618 -> 896 MiB (+45%) | 746 MiB | 0.83x -> 1.20x |
| next-root | 0.742 -> 0.767 (+3.4%) | 436 -> 695 MiB (+59%) | 566 MiB | 0.77x -> 1.23x |
| storybook | 0.772 -> 0.770 (-0.4%) | 458 -> 707 MiB (+54%) | 604 MiB | 0.75x -> 1.17x |
| nuxt | 0.506 -> 0.501 (-1.1%) | 442 -> 693 MiB (+57%) | 541 MiB | 0.81x -> 1.28x |
| playwright | 0.767 -> 0.755 (-1.7%) | 413 -> 665 MiB (+61%) | 474 MiB | 0.87x -> 1.40x |
| drizzle-orm | 0.530 -> 0.532 (+0.4%) | 594 -> 887 MiB (+49%) | 730 MiB | 0.81x -> 1.21x |

The single-threaded counting run (`bench/count.py`, `RAYON_NUM_THREADS=1`) moved instructions by -0.09% to +0.07%, the
spread of two PGO trainings, and its peak by +2% (vscode) to +41% (Compiler): the regression flag would mark every
project's memory.

## The threshold

20 GiB is four times the largest peak any bench project reached with the advice on: 5.04 GiB, formbricks-web at 64
checkers on 64 vCPUs (pr-verify, 17 projects x 1-64 checkers). Four times is the headroom the bench machines are
sized with (`bench/README.md`, "Fixed machine spec"). The extra memory is per thread (each thread's partly filled
mimalloc pages each hold a 2 MiB page), so the cost of a large program grows with the checker count, not with its
size. The rule arms on every Depot runner measured and on any Linux machine with 20 GiB free; it stays off on a 16 GB
machine (GitHub's standard runner) and in a container limited below 20 GiB.

## Decision

Not adopted. The bar for wall time is 2% on the headline table. The table's geomean moves about 1% (same-runner
pr-verify: -0.8% / -1.1% on 16 vCPUs, -0.9% at the default on 64; bench.yml: tsrs/bun ratio -1.2%, inside the noise
of main's own runs). The large programs mostly clear it (-1 to -4%, vscode's headline cell -2.9%), at +17-37% peak,
but the ten small ones pay +43-128% for nothing, and the rule cannot tell them apart before the first arena is
reserved. The code stays on `probe/heap-thp-by-memory`.

What would change the answer:

- A rule that knows the program is large. One untried shape: leave the first arena (1 GiB, reserved before `main`)
  unadvised and turn the advice on for the arenas reserved after it, which only programs with more than about 1 GiB
  of mimalloc heap reach. Small programs would pay nothing; how much of the 2-3.5% the large ones keep is unmeasured.
- A decision that 2-3% of wall time on large programs is worth +17-37% peak, and more on small ones, on machines
  with 20 GiB free. Then de2ecbfc applies as it is.

## Runs

| run | what |
| --- | --- |
| zqgtm5nlh2, q2dk2vlhv5, krs8vknw95 | perf-probe on 8, 16 and 64 vCPUs: off / on / full, 6 projects, 5 reps |
| qx1lf0r9hh | perf-probe, 64 vCPU: vscode at 32 checkers, off / on, 20 reps |
| n5cqgk061h | pr-verify, 16 vCPU, 8 and 16 checkers, 5 reps, 17 projects |
| bwv180tsbr | pr-verify, 64 vCPU, 1/4/16/32/64 checkers, 3 reps, 17 projects |
| dr013g4s9c | bench.yml from the branch (de2ecbfc), `bench-result` artifact |
