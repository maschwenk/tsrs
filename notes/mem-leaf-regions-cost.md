# mem-leaf-regions-cost: what leaf-file freeing costs on small machines, and making it cheaper

Follow-up to notes/mem-free-leaf-files.md (PR 137) and PR 151, which turned leaf freeing off by default after the
README bench on the 8-vCPU runner showed vscode 10.88 -> 13.24 s single-threaded (+21.7%) and 2.87 -> 3.48 s at 4
checkers (+21.4%) for 8b3e4f4 (freeing on up to 16 checkers) against 7262f61. The question was where those 2.4 s go,
and how to make freeing cheap enough to keep on.

Short answer: the 2.4 s are not leaf freeing. Measured with one binary, freeing on against off, interleaved on the
same 8-vCPU machine, vscode costs 0-1.5% at 1, 4 and 8 checkers and system time stays at 0.2 s in every mode. The
bench pair compared two runners, the second one slower (tsgo, which did not change, was 6-27% slower on the same
cells), and two PGO builds whose user-space instructions differ by 1.8-3.5% on every project, including the ones that
have no file region at all (#153 found why: the CLI's `_exit` skipped writing the PGO training profiles). What freeing does cost grows with the number of threads: each call that gives pages back
flushes the TLB of every core running a checker, and the file regions' 4 KiB pages cost some TLB reach and some exit
time; at 32 checkers on the 64-vCPU runner that is about 2-4%. This change makes the give-backs fewer and the
leaf classification cheaper, and turns the default back on up to 16 checkers (PR 151 had set it to 0). With 20
interleaved runs per cell, freeing is now within 2% wall time at 1, 4 and 8 checkers on the 8-vCPU runner (vscode
-0.4% to +0.5% for 13-16% less peak memory) and at 16 on the 64-vCPU runner (-0.2% for 11% less), but not at 32
(+2.4-3.9%), which stays off by default.

## Method

`.depot/workflows/perf-probe.yml` now takes `runner` (default `depot-ubuntu-24.04-64`), `script` and `probe_args`
inputs and restores bench.yml's project caches, so a dispatched probe of a pushed branch runs on the 8-vCPU runner
without cloning anything:

    depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref <branch> \
      --input runner=depot-ubuntu-24.04-8 --input projects=vscode,formbricks-web \
      --input script=tools/perf/leafprobe.sh --input probe_args='--reps 20 --checkers 1,4,8'

`tools/perf/leafprobe.py` runs one `cargo build --release` binary under several environments (variants) in
interleaved order: `-p <project> --noEmit --incremental false --pretty false --extendedDiagnostics`, wall from
`os.wait4`, user and system CPU, peak RSS and minor faults from its rusage, the deltas of /proc/vmstat (huge-page
faults, compaction stalls, TLB flushes), the phase times; then per variant `perf stat` (page faults, dTLB load
misses, user and kernel cycles; the runners' PMU does not count instructions), `strace -f -c`, and `perf record` with
`perf diff` between variants. Variants: `off` (`TSRS_FREE_LEAVES=0`), `keep` (regions and leaf marks, nothing freed),
`on` (`TSRS_FREE_LEAVES=1`). Medians; "paired" is the median of each rep's on/off ratio. Runners: `depot-ubuntu-24.04-8`
(8 vCPU, 31 GB) and `depot-ubuntu-24.04-64`, both AMD EPYC 9R45, kernel 6.12, THP `madvise` (defrag `madvise`).

## Step 1: the 8-vCPU runner

Same binary (main 2ff00bc + the probe), 5 interleaved reps (h66k1nrfm0):

| project | checkers | off | keep | on | peak, on vs off | system CPU off / keep / on |
| --- | --- | --- | --- | --- | --- | --- |
| vscode | 1 | 12.75 s | 12.97 s (+1.8%) | 12.92 s (+1.4%) | -15.6% | 0.19 / 0.18 / 0.20 s |
| vscode | 4 | 3.381 s | 3.399 s (+0.5%) | 3.410 s (+0.9%) | -14.1% | 0.20 / 0.19 / 0.21 s |
| formbricks-web | 1 | 5.048 s | 5.050 s (+0.0%) | 5.158 s (+2.2%) | -3.9% | 0.14 / 0.16 / 0.17 s |
| formbricks-web | 4 | 1.788 s | 1.732 s (-3.1%) | 1.757 s (-1.7%) | -2.9% | 0.19 / 0.20 / 0.19 s |

The runner is noisy: in a second job (xk5z3l2rj7, 5 reps) the same `off` variant ranged 12.35-14.63 s on vscode
single-threaded, drifting up through the job, and the medians came out keep +7.6%, on +6.9%; interleaving keeps the
comparison fair, 5 reps do not settle 2%. Counters from that job (`perf stat`, 2 runs each, vscode single-threaded):

| | off | keep | on |
| --- | --- | --- | --- |
| user cycles | 55.39 G | 55.47 G | 55.32 G |
| kernel cycles | 0.80 G | 0.86 G | 0.87 G |
| dTLB load misses | 9.66 M | 10.15 M | 10.59 M |
| page faults | 18.5 k | 30.0 k | 29.3 k |
| kernel share of `perf record` samples | 1.57% | | 1.83% |

So single-threaded the whole kernel side is +0.07 G cycles (about 0.5% of the run): 11k more page faults for the
regions' pages (they are not advised for huge pages; the thread arenas take 168 fewer 2 MiB faults) and about 850 `madvise(MADV_DONTNEED)` calls for 409 MB. `strace -f -c`
(vscode, 1 / 4 checkers, off -> on): madvise 359 -> 1,004 / 371 -> 1,545 calls, mprotect 61 -> 642 / 230 -> 818, futex
64 -> 10,338 / 635 -> 10,908; total traced system-call time 0.18 -> 0.27 s single-threaded (strace inflates it). The
futex calls are the region owner lock's condvar notify, a system call on every unlock even with nobody waiting; fixed
here.

The README bench pair, tsrs against tsgo 7.0.2 in the same runs:

| project | mode | tsrs | tsgo (unchanged) | tsrs / tsgo |
| --- | --- | --- | --- | --- |
| vscode | single-threaded | +21.7% | +6.0% | +14.8% |
| vscode | 4 checkers | +21.4% | +27.3% | -4.6% |
| vscode | 8 checkers | +10.3% | +15.6% | -4.6% |
| formbricks-web | single-threaded | +24.2% | +25.3% | -0.9% |
| formbricks-web | 4 checkers | +15.8% | +23.1% | -5.9% |
| supabase-studio | single-threaded | +7.3% | +11.8% | -4.0% |
| xstate-main | single-threaded | +16.7% | +14.5% | +2.0% |

and single-threaded user-space instructions (bench/count.py) rose on every project: vscode +3.5%, webpack +2.8%,
Compiler +3.2%, mui-docs +1.9%, cal-diy +1.8%; webpack, mui-docs and Compiler have no file regions. That is a
different PGO build, not freeing (macOS, one binary: 109.4-110.5 G instructions in off, keep and on alike). Only
vscode single-threaded is out of line against tsgo, and the same-binary probes put it at 0-1.5%.

20 interleaved reps, same binary (main + the lock fix, before the other changes here; sspwn4wq64):

| project | checkers | off | on | median | paired | mean | peak |
| --- | --- | --- | --- | --- | --- | --- | --- |
| vscode | 1 | 12.88 s | 12.77 s | -0.9% | +0.8% | +0.7% | 1.866 -> 1.575 GiB (-15.6%) |
| vscode | 4 | 3.229 s | 3.233 s | +0.1% | +1.2% | +1.0% | 2.081 -> 1.786 GiB (-14.2%) |
| vscode | 8 | 1.920 s | 1.945 s | +1.3% | +1.4% | +1.2% | 2.228 -> 1.932 GiB (-13.3%) |
| formbricks-web | 1 | 5.600 s | 5.613 s | +0.2% | +1.6% | +2.2% | -4.0% |
| formbricks-web | 4 | 1.916 s | 1.928 s | +0.6% | +0.6% | +0.5% | -2.7% |
| formbricks-web | 8 | 1.281 s | 1.285 s | +0.3% | -0.2% | +0.5% | -2.6% |

## Where the cost is with many checkers (64-vCPU runner)

10 interleaved reps, same binary (l2qn0ckb27):

| project | checkers | off | keep | on | peak, on |
| --- | --- | --- | --- | --- | --- |
| vscode | 16 | 0.944 s | 0.953 s (+1.0%) | 0.961 s (+1.9%) | -11.4% |
| vscode | 32 | 0.660 s | 0.672 s (+1.8%) | 0.676 s (+2.3%) | -10.1% |
| formbricks-web | 16 | 0.755 s | 0.760 s (+0.6%) | 0.759 s (+0.5%) | -2.5% |
| formbricks-web | 32 | 0.711 s | 0.713 s (+0.3%) | 0.721 s (+1.4%) | -2.1% |

vscode, `perf stat` (2 runs each):

| checkers | | off | keep | on |
| --- | --- | --- | --- | --- |
| 16 | dTLB load misses | 5.44 M | 6.32 M | 7.50 M |
| 16 | user / kernel cycles | 63.70 / 1.73 G | 63.83 / 1.86 G | 64.56 / 1.90 G |
| 32 | dTLB load misses | 5.68 M | 6.71 M | 11.09 M |
| 32 | user / kernel cycles | 69.92 / 1.91 G | 70.63 / 2.01 G | 71.00 / 2.24 G |

Freeing nearly doubles the user-side dTLB misses at 32 checkers. Each give-back is a `madvise(MADV_DONTNEED)` over
more than 33 pages, so the kernel flushes the whole TLB of every core that runs a thread of the process (by
interprocessor interrupt on these runners), and every checker then refills its TLB, huge-page entries of the thread arenas
included. The cost is calls x threads: nothing single-threaded, about 1% at 32. The rest (`keep`) is the regions
themselves: `perf diff` of off against keep at 32 checkers is flat (no symbol moves more than 0.25% of the samples;
classify's lookups `get_source_file` and `get_resolved_module` +0.1% each), the `--extendedDiagnostics` sub-phases
are equal to the millisecond, and the difference is Check time +5.5 ms (classify's scan of 110k resolutions, about 3
ms, and the TLB misses of the 4 KiB-page trees) and +2-3 ms after Total time (process exit tears down the regions'
small pages).

## Variants

64-vCPU, 10 reps, same binary with throwaway switches (txwzqw5lmn; `TSRS_LEAF_EXPERIMENT`, removed again):

| variant | vscode 16 | vscode 32 | formbricks-web 16 | formbricks-web 32 | vscode peak at 16 / 32 |
| --- | --- | --- | --- | --- | --- |
| on | +2.4% | +2.3% | +1.5% | +0.9% | -11.5% / -10.4% |
| huge-page slabs (2 MiB, `MADV_HUGEPAGE`, given back whole) | +1.9% | +2.7% | +1.9% | +1.1% | -5.0% / -4.5% |
| pre-faulted slabs (`MADV_POPULATE_WRITE`) | +3.6% | +5.5% | +1.7% | +1.3% | -9.8% / -8.9% |
| freed, pages kept (`nogive`) | +2.6% | +4.1% | +1.3% | -0.4% | +0.6% / +0.8% |

On the 8-vCPU runner (xk5z3l2rj7) huge-page slabs were -0.5% / +0.8% / +1.1% at 1 / 4 / 8 checkers for -8.9% /
-8.0% / -7.3% peak (on: -15.6% / -14.0% / -13.3%). Huge-page slabs give back only slabs whose every region is freed:
the 225 predicted files that are not leaves (201 of them test helpers matched only by a directory pattern) and the
leaves not checked yet pin slabs, and the trees would otherwise need splitting a huge page. They keep half of the
memory win for no measurable time; not kept. Pre-faulting replaces faults that cost little with a call per slab:
slower. Reusing a freed region's range instead of retiring it was not tried: the cost it would remove (page faults,
give-backs) is not where the time is, and it would need the stale-address argument the retire avoids.

## What this change does

- **Region lock** (arena.rs `OwnerLock`): notifies a waiter only when one waits. vscode single-threaded with
  freeing: 10,338 -> 48 futex calls (71 with freeing off).
- **Large slabs for file regions** (`Region::new_scratch_in_large_slabs`, `LARGE_SLAB_SIZE` 16 MiB): one thread's
  file regions lie together, so retired neighbours form longer runs. vscode give-back calls (macOS, M5 Max):
  4 checkers 870 -> 350, 16 checkers 1,110 -> 870, 32 checkers 1,350 -> 1,350 (there the runs are cut by leaves
  not freed yet, not by slab boundaries). Untouched slab pages cost address space only. Single-threaded on Linux
  the slabs' `mprotect` calls go from 642 to 84.
- **Whole-span give-back** (`retired::spans_of`): a batch gives back each coalesced span it touched in one call,
  including parts given back before (they hold no memory). vscode: 16 checkers 870 -> 712, 32 checkers 1,355 ->
  1,092 calls; same pages, same peak (macOS). Larger batches were measured again and rejected: 64 / 256 MiB batches
  (with large slabs, before this) cut calls at 16 checkers from 870 to 712 / 507 but raise the peak by 50 / 140 MB.
- **Cheaper classification** (`fileregions::referred_files`): a resolved name is looked up by the name itself before
  normalizing it, the resolution maps are read on the worker pool without collecting the names first, and only files
  that can be leaves are kept. vscode at 16 checkers on the M5 Max: 5.3 -> 2.6 ms; it has its own line now,
  `Checkers: leaf files`.

Steps on the 64-vCPU runner, 10 reps each (wall against `off` in the same job):

| run | change | vscode 16: keep / on | vscode 32: keep / on | formbricks-web 16 / 32: on | `Checkers: leaf files` |
| --- | --- | --- | --- | --- | --- |
| l2qn0ckb27 | lock fix | +1.0% / +1.9% | +1.8% / +2.3% | +0.5% / +1.4% | |
| d15zjvqbtx | + large slabs, cheaper classification | +1.1% / +1.0% | +2.0% / +2.6% | +1.1% / +1.5% | |
| mfhjnlmqmw | + whole-span give-back | +0.5% / +1.0% | +0.4% / +1.3% | +1.5% / +1.4% | 4-5 ms |

At 32 checkers the same build measures anywhere from +1.3% to +2.8% from job to job; the per-run noise there is
about as large as the cost. Classification computed during checker creation (the last change) takes `Checkers: leaf
files` from 4-5 ms to about 1 ms on the M5 Max at 4-32 checkers (its referred-file scan now overlaps the 11 ms of
`Checkers: create` and `assign files`).

## Results: 20 interleaved runs per cell, final code (main 0257e3e merged)

8-vCPU runner (qjt1xzpx2p):

| project | checkers | off | on | median | paired | mean | Check time | peak |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| vscode | 1 | 12.384 s | 12.449 s | +0.5% | +0.8% | +0.0% | 10.501 -> 10.544 s | 1.867 -> 1.577 GiB (-15.6%) |
| vscode | 4 | 3.142 s | 3.133 s | -0.3% | -0.0% | -0.1% | 2.825 -> 2.811 s | 2.070 -> 1.784 GiB (-13.8%) |
| vscode | 8 | 1.943 s | 1.934 s | -0.4% | +0.2% | +0.2% | 1.613 -> 1.597 s | 2.218 -> 1.933 GiB (-12.8%) |
| formbricks-web | 1 | 5.449 s | 5.363 s | -1.6% | -0.6% | -1.6% | 4.065 -> 3.970 s | 1.327 -> 1.282 GiB (-3.4%) |
| formbricks-web | 4 | 1.838 s | 1.846 s | +0.4% | +0.5% | -0.1% | 1.569 -> 1.579 s | 1.663 -> 1.620 GiB (-2.6%) |
| formbricks-web | 8 | 1.209 s | 1.214 s | +0.4% | +0.5% | +0.4% | 0.946 -> 0.947 s | 1.911 -> 1.867 GiB (-2.3%) |

64-vCPU runner (wf74l06xh8; main 61fc500 merged, the same freeing code):

| project | checkers | off | on | median | paired | mean | Check time | peak |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| vscode | 16 | 0.974 s | 0.973 s | -0.2% | +0.9% | +0.5% | 0.796 -> 0.803 s | 2.513 -> 2.228 GiB (-11.3%) |
| vscode | 32 | 0.678 s | 0.697 s | +2.8% | +3.9% | +2.4% | 0.503 -> 0.520 s | 2.786 -> 2.501 GiB (-10.2%) |
| formbricks-web | 16 | 0.802 s | 0.792 s | -1.3% | -1.2% | -1.5% | 0.647 -> 0.635 s | 2.343 -> 2.301 GiB (-1.8%) |
| formbricks-web | 32 | 0.751 s | 0.745 s | -0.8% | -0.5% | -1.0% | 0.586 -> 0.583 s | 2.946 -> 2.908 GiB (-1.3%) |

Decision: `MAX_DEFAULT_CHECKERS` = 16 (as before PR 151). Every cell at 1-16 checkers is within 2% on every
statistic; vscode at 32 is not (+2.4% mean, +3.9% paired), so the 64-vCPU runner's default of 32 checkers keeps
freeing off unless `TSRS_FREE_LEAVES=1`. 16 covers machines up to 33 threads by default.

What remains at 32: the give-backs (vscode about 1,100 calls, each a TLB flush on every checker's core; batching
more cuts calls only by holding freed memory longer) and the regions' small pages. Ideas not tried: give back only
from a thread when few checkers still run (the end of the pass), which loses the memory win where the peak is;
`process_madvise` with a vector of ranges, whose single TLB flush per call needs a newer kernel than the runners'
6.12; AMD's broadcast TLB invalidation (`INVLPGB`), which newer kernels use and which would make each flush cheap
without a change here.

pr-verify (`depot ci run`, qj84d3jvkv; main 0257e3e against this branch, release builds, 3 reps, the default
switch, so freeing at 1, 4 and 16 checkers and not at 32): diagnostics identical in 102 of 102 cells (17 projects,
with the poisoned runs at 16 checkers).

| project | 1 checker | 4 | 16 | 32 (off) | peak at 1 / 4 / 16 | instructions (1 thread) |
| --- | --- | --- | --- | --- | --- | --- |
| vscode | -0.2% | +0.8% | +1.8% | -4.3% | -14.1% / -13.2% / -11.7% | +0.08% |
| formbricks-web | +0.5% | +1.2% | +0.8% | -3.2% | -2.6% / -2.7% / -1.8% | +0.11% |
| supabase-studio | -0.3% | -0.3% | +0.2% | -2.4% | -0.8% / -1.1% / -0.5% | +0.11% |
| t3code-server | -0.4% | -0.1% | +1.4% | +0.5% | -6.9% / -4.5% / -1.8% | +0.04% |
| xstate-main | +1.4% | -1.4% | +1.9% | -1.8% | +1.3% / +1.2% / -0.4% | +0.08% |

The other twelve projects' median wall change over the checker counts is -1.4% to +1.0%.

## Gates

- Output: `--pretty false` stdout and exit code of the branch with `TSRS_FREE_LEAVES=1` and with
  `TSRS_FREE_LEAVES=1 TSRS_ARENA_POISON=1` against main with `TSRS_FREE_LEAVES=0`, ten bench projects (vscode,
  xstate-main, webpack, mui-docs, Compiler, Compiler-Unions, cal-diy, formbricks-web, supabase-studio, t3code-server)
  at 1, 4 and 16 checkers on macOS: identical in all 90 runs, against main 2ff00bc and again against main 0257e3e
  after merging it. pr-verify above: 102 of 102 cells.
- `tools/regressions.sh` (22 cases) with `TSRS_FREE_LEAVES=1`, plain and poisoned: all pass, including
  `leaf-alternative-containers` and `leaf-structural-instantiation`.
- `cargo check --workspace` (no warnings), `tools/lint/ratchet.py`, `tools/lint/source.py`, `cargo test -p tsrs_core
  -p tsrs_compiler` (new: `a_batch_gives_back_whole_spans`), `cargo test -p tsrs_cli --test free_leaf_files`.
- The reachability census is not affected: `TSRS_CENSUS=1` turns file regions off (`leaf_settings_from_env`), and
  nothing here reuses a freed range (retired ranges are still never handed out again).
