# mem-thp: mimalloc without transparent huge pages on its heap

Follow-up to notes/mem-64.md §3, which found that mimalloc v3 advises `MADV_HUGEPAGE` on the arenas it reserves, so on
a Linux host with THP `madvise` (Ubuntu's default, and the Depot runners') every 2 MiB block a thread touched is
resident in full. This note measures what to do about it and lands the plain answer: build mimalloc with its `no_thp`
feature (`MI_NO_THP`, crates/tsrs_cli/Cargo.toml). The tsrs arena (`tsrs_core::reserve`) keeps its own advice and huge
pages (notes/linux-x86-round.md). macOS has no THP and `MI_NO_THP` only removes Linux code: the macOS binary behaves the
same (vscode, 1 checker: 112.2-113.2 G instructions both builds, run-to-run spread).

All numbers: Depot `depot-ubuntu-24.04-64` (64 vCPUs), vscode, `cargo build --release` (not PGO; relative only),
tools/perf/probe.sh, medians of 3 reps (5 for the 8-CPU rows), interleaved per rep. "thp" is the build before this note,
"nothp" this one. Peak RSS is `ru_maxrss`; check is the `Check time` line; instructions from a perf counter over all
threads were identical in every variant (122.4-123.2 G at 4 checkers, 135.6-136.3 G at 16, 160.4-162.1 G at 64).
Diagnostics identical in every run (371 errors).

## Result (probe run 2kd946v1t6)

| checkers | wall s thp -> nothp | check s thp -> nothp | peak RSS GiB thp -> nothp |
| --- | --- | --- | --- |
| 4 | 2.833 -> 2.929 (+3.4%) | 2.649 -> 2.708 (+2.2%) | 3.035 -> 2.277 (-25.0%) |
| 16 | 0.960 -> 1.037 (+8.0%) | 0.767 -> 0.779 (+1.6%) | 3.599 -> 2.643 (-26.6%) |
| 64 | 0.687 -> 0.803 (+17%) | 0.469 -> 0.462 (-1.5%) | 5.017 -> 3.363 (-33.0%) |
| 4 on 8 CPUs (`taskset -c 0-7`, the README runner's size) | 3.278 -> 3.365 (+2.7%) | 2.916 -> 2.979 (+2.2%) | 2.343 -> 2.139 (-8.7%) |

The same comparison in the two earlier runs: check +3.5% / +5.7% / -0.4% and +1.0% / +3.2% / +1.1%, wall +5.5% / +8.2%
/ +10.6% and +2.4% / +5.8% / +9.4% at 4 / 16 / 64; peak RSS within 0.01 GiB of the table. So: **-25% / -27% / -33% peak
RSS on the 64-vCPU machine, -9% on an 8-CPU one, for +1-3.5% check time at 4-16 checkers (none at 64) and +3-17% wall.**
The wall cost outside the checker is the parallel parse (Parse time 0.110 -> 0.126-0.158 s: 64 threads touching fresh
4 KiB pages) and process exit (wall minus Total time 0.05 -> 0.10 s at 64: the kernel frees 4 KiB pages one by one).
Minor faults: 8 K -> 29 K at 4 checkers, so the user-time cost is TLB misses on the heap, not faults.

At 64 checkers nothp is still below bun check's 0.85-0.88 s wall on this machine (bench/results/compare/), and its
3.36 GiB is closer to bun's 2.9 GiB than 5.0.

## What the amplification is

mimalloc v3 sizes: small pages 64 KiB (objects up to 10 KiB), medium 512 KiB (up to 84.7 KiB), large 4 MiB (up to
512 KiB), larger objects a page of their own; each thread keeps one partly filled page per size class it used. Under THP
the untouched part of a medium or large page costs memory as soon as anything in its 2 MiB block is touched. The variants
below locate it: small pages carry almost none of it, medium and large pages about half each, huge objects none.

### mimalloc options (probe run csf31k297d; the same as `mi_option_set` before the first allocation)

| variant | wall s 4 / 16 / 64 | check s 4 / 16 / 64 | peak RSS GiB 4 / 16 / 64 |
| --- | --- | --- | --- |
| thp | 2.956 / 1.006 / 0.691 | 2.759 / 0.789 / 0.468 | 3.035 / 3.574 / 5.016 |
| nothp | 3.119 / 1.088 / 0.764 | 2.855 / 0.834 / 0.466 | 2.283 / 2.636 / 3.370 |
| nothp, `MIMALLOC_PURGE_DELAY=-1` (no purge refaults) | 3.083 / 1.087 / 0.778 | 2.846 / 0.810 / 0.474 | 2.296 / 2.641 / 3.365 |
| thp, `MIMALLOC_ARENA_MAX_OBJECT_SIZE=512` (pages over 512 KiB from the OS, unadvised) | 3.034 / 1.071 / 0.831 | 2.817 / 0.817 / 0.537 | 2.343 / 2.702 / 3.456 |
| thp, `MIMALLOC_ARENA_MAX_OBJECT_SIZE=64` | 3.135 / 1.094 / 0.781 | 2.911 / 0.820 / 0.513 | 2.337 / 2.699 / 3.463 |
| thp, `MIMALLOC_PAGE_COMMIT_ON_DEMAND=1` | 2.968 / 0.983 / 0.692 | 2.765 / 0.780 / 0.467 | 3.033 / 3.600 / 5.008 |
| `MIMALLOC_ALLOW_THP=0` (THP off for the process, tsrs arena too) | 3.373 / 1.218 / 0.894 | 3.021 / 0.847 / 0.490 | 2.118 / 2.457 / 3.097 |

The arena-size option gets the memory but pays an mmap/munmap per page (+15% check at 64: `mmap_lock` across 64
threads). Commit on demand changes nothing (the arenas are committed eagerly on Linux). Notes/mem-64.md §3 has
`MIMALLOC_ARENA_EAGER_COMMIT=0` (same memory as nothp, +7% check at 4) and `MIMALLOC_PURGE_DELAY=0` / `=100`.

### A second heap on 4 KiB pages (probe run 5gc0v7hkvf; code in commit 6c77d25 of this branch, reverted)

A `GlobalAlloc` over mimalloc that sends allocations in a size range to a first-class heap (`mi_heap_new_in_arena`)
whose exclusive arena is an 8 GiB `MAP_NORESERVE` mapping advised `MADV_NOHUGEPAGE` (`mi_manage_os_memory_ex`), the
default heap keeping THP. "thp" here is the same binary with the split off.

| objects on 4 KiB pages | wall s 4 / 16 / 64 | check s 4 / 16 / 64 | peak RSS GiB 4 / 16 / 64 |
| --- | --- | --- | --- |
| none (thp) | 2.872 / 0.964 / 0.689 | 2.679 / 0.761 / 0.461 | 3.025 / 3.579 / 5.028 |
| all (nothp build) | 2.942 / 1.020 / 0.754 | 2.706 / 0.785 / 0.466 | 2.276 / 2.628 / 3.383 |
| large: 84.7-512 KiB | 2.863 / 0.979 / 0.702 | 2.659 / 0.764 / 0.464 | 2.676 / 3.064 / 4.060 |
| large and huge: > 84.7 KiB | 2.934 / 1.003 / 0.737 | 2.707 / 0.765 / 0.469 | 2.697 / 3.073 / 4.089 |
| huge: > 512 KiB | 2.910 / 1.043 / 0.696 | 2.700 / 0.765 / 0.463 | 3.006 / 3.557 / 4.982 |
| medium, large and huge: > 10 KiB | 2.937 / 1.077 / 0.747 | 2.718 / 0.776 / 0.466 | 2.372 / 2.720 / 3.463 |

The large-object split costs nothing measurable (check -0.7% / +0.4% / +0.7%) but gets only 47% / 54% / 59% of nothp's
saving (-12% / -14% / -19% peak RSS); moving medium objects too gets 90% of it, at nothp's time cost or more (Parse time
0.142-0.154 s against nothp's 0.127-0.137 s). The medium pages are where the trade is: they carry half the
amplification and the parse phase wants them on huge pages. Neither split met "most of the saving for <= 1% check time",
so the plain feature landed; the large-object split is the option if the 4-checker time matters more than the 64-checker
memory (about 45 lines; the commit above has it).

Not tried: fewer allocating threads (the 64 parse workers hold most of the amplification at 4 checkers; exiting them, or
a smaller pool, would let checker threads reuse their partly filled pages; a front-end change), and freeing the parse
workers' heaps before checking (`mi_collect` frees only empty pages; the cost is in partly filled ones).

## Gates

- Diagnostics: full `--pretty false` output before the statistics block identical to the base build for vscode,
  webpack and xstate-main at 1, 4 and 16 checkers on macOS (371 / 840 / 0 errors); vscode identical between all Linux
  variants at 4 / 16 / 64 checkers and at 4 on 8 CPUs.
- `cargo test --release -p tsrs_cli`: the integration tests (`default_emit`, `derived_variance`, `flow_memo`) pass;
  the unit-test binary does not link on macOS (`api::memory_tests` call glibc `malloc_trim`), as on main.
- `tools/lint/ratchet.py` and `tools/lint/source.py`: ok, no new findings.
