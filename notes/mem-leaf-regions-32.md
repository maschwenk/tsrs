# mem-leaf-regions-32: leaf-file freeing at 32 checkers, where its cost is and why it stays off there

Follow-up to notes/mem-leaf-regions-cost.md (PR 159), which left leaf freeing on by default up to 16 checkers
(`fileregions::MAX_DEFAULT_CHECKERS`) because at 32 checkers on the 64-vCPU runner it cost vscode +2.4-3.9% wall
time for 10% less peak memory. The goal here was to make it pay at 32: wall within 1% with peak at least 8% lower on
vscode and no loss on the app projects, so that the README's default run (32 checkers on that runner) could have it.

Short answer: it could not be shown. The cost that freeing adds at 32 checkers is the TLB-flush interprocessor
interrupts that each give-back (`madvise(MADV_DONTNEED)`) sends to every core running a checker: about 2% more CPU on
vscode. Making the calls from another thread or keeping each under the single-page flush limit does not reduce it;
making fewer calls does, but only by giving back less memory during the check (a 768 KiB hold: half the CPU, 8.6%
less peak instead of 10.8%). The wall time does not follow: in three jobs of 20-30 interleaved reps, main's freeing
measured +2.0%, +3.8% and +0.9% at 32 checkers on vscode, and freeing with no give-back at all +0.4%, +2.2% and
+0.3% (paired +2.0% and +1.2% where measured). No variant was within 1% with at least 8% less peak in more than one
job, so `MAX_DEFAULT_CHECKERS` stays 16 and the freeing code does not change. Kept: `tools/perf/leafprobe.py` gets a
paired-ratio, min-max and user+sys column, and a variant can run another binary (`BIN=<path>`, for base-against-new
probes).

## Method

`.depot/workflows/perf-probe.yml` dispatched on a pushed branch with `script=tools/perf/leafprobe.sh` (one release
binary, variants as environments, interleaved: every rep runs each variant once in turn), `--extendedDiagnostics`,
wall from `os.wait4`, peak RSS from rusage, then `perf stat -r 2` and `perf record` per variant. Runners:
`depot-ubuntu-24.04-64` and `depot-ubuntu-24.04-8`, AMD EPYC 9R45, kernel 6.12, THP `madvise`. The runner is a
paravirtualized guest (`__pv_queued_spin_lock_slowpath` in every kernel profile), so an IPI costs a VM exit on the
sender and an interrupt on every receiver. "paired" is the median over reps of variant / `off` in the same rep.

The variants were throwaway switches in `arena::retired` (branch commits 0c129f5, 426996f; reverted):

- `on`: `TSRS_FREE_LEAVES=1`, main's code (16 MiB batches, one `madvise(MADV_DONTNEED)` per retired span a batch
  touched).
- `keep`: regions and leaf marks, nothing freed. `nogive`: leaves freed (drops run, ranges retired), but no page
  goes back to the system.
- `small`: each call cut into pieces of at most 32 pages, under x86's `tlb_single_page_flush_ceiling` (33), so the
  receiving cores flush single pages with `invlpg` instead of their whole TLB.
- `bg`: the calls are made by one give-back thread, not by the checker that retired the batch.
- `batch64`: 64 MiB batches instead of 16.
- `minN`: during the pass a retired span is given back only once N KiB of it have not been given back yet; shorter
  spans wait until neighbours retired later make them long enough, or until the pass ends (`flush_retired`, when
  the checkers are idle). The clean version is commit 7c12387 (`arena::set_retired_hold`, with a unit test).

## Where the cost is (vscode, 32 checkers)

20 interleaved reps (s2ptb57wbm):

| variant | wall (median) | vs off | user / sys CPU | peak | vs off |
| --- | --- | --- | --- | --- | --- |
| off | 0.618 s | | 16.31 / 0.37 s | 2.642 GiB | |
| keep | 0.625 s | +1.2% | 16.33 / 0.41 s | 2.656 GiB | +0.5% |
| nogive | 0.620 s | +0.4% | 16.35 / 0.37 s | 2.656 GiB | +0.5% |
| on | 0.630 s | +2.0% | 16.58 / 0.42 s | 2.354 GiB | -10.9% |
| small | 0.634 s | +2.6% | 16.71 / 0.51 s | 2.353 GiB | -10.9% |
| bg | 0.629 s | +1.8% | 16.54 / 0.44 s | 2.355 GiB | -10.9% |
| batch64 | 0.629 s | +1.8% | 16.46 / 0.40 s | 2.411 GiB | -8.7% |

`perf stat` (2 runs each):

| variant | dTLB load misses | user cycles | kernel cycles |
| --- | --- | --- | --- |
| off | 5.01 M | 65.93 G | 1.56 G |
| keep | 5.69 M | 66.26 G | 1.63 G |
| nogive | 5.55 M | 66.56 G | 1.64 G |
| on | 9.86 M | 66.81 G | 1.81 G |
| small | 5.65 M | 67.44 G | 2.29 G |
| bg | 9.89 M | 67.50 G | 1.81 G |
| batch64 | 8.14 M | 66.58 G | 1.78 G |

`perf record` (all samples, kernel symbols): `asm_sysvec_call_function`, the handler of the flush IPI on the
receiving core, is 2.30% of the samples with `on`, 6.52% with `small`, and absent from `nogive` and `off`; the rest
of the kernel profile (`clear_page_erms` 0.6-0.7%, the paravirt spinlock 0.3-0.4%) is the same in all variants.

So, for vscode's ~1,050 give-back calls at 32 checkers:

- Each call interrupts every core running a checker; the receivers' handler time and the refill of their flushed
  TLBs (dTLB misses nearly double: 5.6 -> 9.9 M) are the cost, on all 32 cores, not on the caller. Making the calls
  from another thread (`bg`) changes nothing.
- Single-page flushes (`small`) keep the receivers' TLBs (dTLB misses back to 5.65 M) but take several times the calls
  (macOS, 16 KiB pages so 512 KiB pieces: 1,871 against 1,032; Linux pieces are 128 KiB), and the interrupts themselves cost more than the refills they save: kernel cycles
  1.81 -> 2.29 G, wall +2.6%.
- Larger batches cut few calls (the spans are cut by leaves not freed yet, not by batch boundaries) and hold more
  memory (-8.7% instead of -10.9%).
- Reusing retired regions as checker-arena chunks instead of giving them back (no system call, and the checker
  arenas grow by hundreds of MB during the check) was not built: the checkers' link stores
  (`tsrs_core::linkstore::LinkStore`, `FxHashMap<P<K>, P<V>>`) and other tables are keyed by object address and keep
  the entries of a freed leaf's nodes and symbols, so a new object at a reused address would find another object's
  links. That is why `Region::retire_on_free` never hands a range out again; proving that none of those entries is
  read after its file is freed is a checker-wide argument, out of reach here.
- Huge-page slabs and pre-faulting were measured in PR 159 (keep half the memory win / slower).

## Fewer calls: hold short spans

The calls follow the spans: on the M5 Max (16 KiB pages), vscode at 32 checkers gives back in 1,055 calls, 250 of
them over 1 MiB holding 70% of the bytes. Holding a span until N KiB of it are pending:

| hold | calls (macOS, vscode, 32 checkers) | of which at the pass end | held until the pass end |
| --- | --- | --- | --- |
| none (main) | 1,018-1,032 | 54 | 0 MB |
| 512 KiB | 392-407 | 110 | |
| 1 MiB | 276 | 135 | 59 MB |
| 2 MiB | 202-212 | 153 | 120 MB |
| 4 MiB | 184-190 | 163 | 201 MB |

Calls made at the end of the pass should be cheap (the checker threads are parked, and the kernel skips cores in
lazy TLB mode; not measured separately), but memory held to the end of the pass does not lower the peak, which is at the end of the check.

64-vCPU runner, 20 interleaved reps (rhrrgmhws4):

| project | checkers | off | on | nogive | min512 | min1024 | min2048 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| vscode | 32 | 0.609 s | +3.8% (paired +2.6%), -10.8% | +2.2% (+2.0%), +0.5% | +2.5% (+1.7%), -9.4% | +3.0% (+2.1%), -7.8% | +2.5% (+1.0%), -5.4% |
| vscode | 16 | 0.903 s | +1.4% (+0.7%), -11.8% | +0.9% (+0.6%), +0.4% | +1.0% (+1.1%), -10.6% | +0.7% (+0.0%), -9.4% | +0.8% (+0.3%), -6.7% |
| formbricks-web | 32 | 0.657 s | -2.4% (-2.0%), -1.4% | -0.2% (-0.3%) | +0.0% (+1.4%), -0.6% | +0.1% (-1.1%), -0.2% | -0.7% (+0.6%), -0.3% |
| formbricks-web | 16 | 0.706 s | -0.3% (-0.8%), -2.0% | +0.4% (+0.5%) | -0.5% (+0.0%), -0.9% | +0.5% (-0.0%), -0.6% | -0.4% (-0.4%), -0.1% |
| t3code-server | 32 | 1.524 s | -1.0% (-1.4%), -1.9% | -1.3% (-1.7%) | -0.8% (+0.6%), -1.6% | -0.4% (+1.2%), -1.0% | -1.4% (+0.2%), -0.8% |
| t3code-server | 16 | 1.679 s | +0.1% (-0.5%), -2.7% | +0.6% (+1.5%) | +0.8% (+1.4%), -1.7% | -1.1% (+0.3%), -1.6% | +0.6% (+0.9%), -0.9% |
| supabase-studio | 32 | 0.613 s | +1.7% (+1.5%), -0.5% | +1.7% (+1.7%) | +2.0% (+2.1%), +0.1% | +1.9% (+2.0%), +0.3% | +1.2% (+1.3%), +0.2% |
| supabase-studio | 16 | 0.768 s | +1.2% (+0.9%), -0.5% | -0.2% (-0.1%) | -0.6% (-0.1%), +0.1% | +0.1% (-0.0%), +0.3% | +0.5% (+0.7%), +0.4% |

(each cell: median wall vs `off`, paired in parentheses, peak vs `off`.) vscode user cycles at 32 checkers
(`perf stat`, 2 runs): off 66.41 G, nogive 66.46 G, min2048 66.79 G, min512 67.08 G, on 68.15 G; kernel 1.55 /
1.66 / 1.69 / 1.71 / 1.82 G. The hold removes most of the CPU the give-backs cost, in proportion to the memory it
stops giving back during the check, but the wall time at 32 checkers does not follow the CPU: in this job freeing
with no give-back at all (`nogive`) is +2.2% median, +2.0% paired on vscode, and +1.7% on supabase-studio, whose
leaves are 2.8% of its nodes (28 MB), against +0.4% for the same variant in the job before. The floor, the regions and the freeing
themselves at 32 checkers, is 0.3-2% and as large as the noise between jobs.

## A third job, 30 reps (fzrhjrgxqv)

To settle the floor: `off`, `keep`, `nogive`, `on` and a 768 KiB hold (the threshold that keeps about 8.5% of
vscode's peak), 30 interleaved reps, same binary. Median wall vs `off` (paired), user+sys CPU, peak vs `off`:

| project | checkers | off | keep | nogive | on | min768 |
| --- | --- | --- | --- | --- | --- | --- |
| vscode | 32 | 0.590 s, 15.68 s | -0.4% (-0.2%), 15.69 s | +0.3% (+1.2%), 15.79 s | +0.9% (+1.1%), 15.99 s, -10.8% | +1.1% (+1.2%), 15.82 s, -8.6% |
| vscode | 16 | 0.892 s, 14.18 s | +1.1% (+1.2%), 14.27 s | +1.4% (+1.3%), 14.31 s | +1.6% (+1.2%), 14.36 s, -11.7% | +1.4% (+1.2%), 14.30 s, -9.8% |
| formbricks-web | 32 | 0.652 s | +1.1% (+0.8%) | +1.5% (+2.1%) | +0.9% (-0.4%), -1.4% | +0.1% (-0.1%), -0.2% |
| formbricks-web | 16 | 0.694 s | +1.3% (+1.3%) | +0.4% (+0.5%) | +0.8% (+0.7%), -1.6% | +1.8% (+0.9%), -0.4% |
| t3code-server | 32 | 1.509 s | -0.9% (-1.0%) | -0.4% (+0.1%) | -1.0% (-0.7%), -1.8% | -0.6% (-1.0%), -1.0% |
| t3code-server | 16 | 1.630 s | +0.4% (+1.2%) | +0.9% (+0.1%) | -0.6% (-1.2%), -3.0% | +0.5% (+0.5%), -1.6% |
| supabase-studio | 32 | 0.610 s | +0.5% (+0.4%) | +1.6% (+1.1%) | +0.3% (+0.8%), -0.6% | +0.5% (+0.1%), +0.2% |
| supabase-studio | 16 | 0.757 s | +0.3% (+0.5%) | +0.9% (+0.7%) | +0.7% (+0.5%), -0.8% | +0.6% (+0.6%), +0.2% |

Here main's own `on` is +0.9% median, +1.1% paired on vscode at 32 checkers, against +2.0%, +3.8% (+2.6% paired)
in the two jobs before and +2.8% (+3.9% paired) in PR 159's. The same variant moves by 3 points from job to job on
the same runner type, more than any change measured here. What is stable is the CPU: give-backs add about 2% user+sys
on vscode at 32 checkers (15.68 -> 15.99 s here, 16.42 -> 16.89 s and 16.31 + 0.37 -> 16.58 + 0.42 s before), a
hold of 512-768 KiB halves that (15.82 s, 16.67 s), and freeing with no give-back costs 0.5-1%.

Pooled over the three jobs, vscode at 32 checkers, median of the per-job medians: `nogive` +0.4% (paired +1.2-2.0%
where measured), `on` +2.0%. So the floor without any system call is near 1% already, and the give-backs add about one
more point; the hold takes that point down to half a point at the price of 2-3 points of peak.

## The 8-vCPU runner (c213kh54jd)

Main's freeing and a 192 KiB hold (24 KiB per checker at 8 checkers, the per-checker rate of the 768 KiB hold at 32),
20 interleaved reps; median wall vs `off` (paired), user+sys CPU, peak vs `off`:

| project | checkers | off | on | min192 |
| --- | --- | --- | --- | --- |
| vscode | 1 | 13.166 s, 13.17 s, 1.869 GiB | +2.0% (+0.8%), 13.44 s, -15.7% | +0.7% (+0.6%), 13.17 s, -15.5% |
| vscode | 4 | 3.224 s, 13.63 s, 2.068 GiB | +0.8% (-0.1%), 13.76 s, -14.0% | +1.2% (+1.3%), 13.74 s, -13.7% |
| vscode | 8 | 1.898 s, 14.57 s, 2.217 GiB | +1.7% (+1.7%), 14.75 s, -13.1% | +1.6% (+1.6%), 14.77 s, -12.9% |
| formbricks-web | 1 | 5.483 s | -3.8% (-1.0%), -3.1% | +0.3% (+0.4%), -3.1% |
| formbricks-web | 4 | 1.687 s | +0.2% (+0.2%), -2.8% | -0.7% (-0.5%), -2.6% |
| formbricks-web | 8 | 1.109 s | +0.0% (+0.4%), -2.3% | +0.9% (-0.2%), -2.2% |

Up to 8 checkers the give-backs are not where the cost is (few cores to interrupt): the hold changes neither wall
nor CPU measurably and gives back nearly the same memory. These rows match PR 159's (vscode within 0-2%, -13 to -16%
peak), so the default up to 16 checkers stands.

## Decision

`MAX_DEFAULT_CHECKERS` stays 16: leaf freeing is on by default up to 16 checkers and off above unless
`TSRS_FREE_LEAVES=1`. The hold (commit 7c12387) is not kept: at the thresholds that keep 8% of vscode's peak (512 KiB
to 1 MiB) its wall time is not distinguishable from `on` at 32 checkers in any job (+2.5% / +3.8% median in
rhrrgmhws4, +1.1% / +0.9% in fzrhjrgxqv), only its CPU is lower, and at 2 MiB, where it costs least, it keeps half of
the memory win. Turning freeing on at 32 checkers needs evidence that its wall cost is within 1% in more than one job; the
three jobs here and PR 159's put it at +0.9% to +3.8%.

What would change this:

- A kernel that flushes without interrupts. AMD's broadcast invalidation (`INVLPGB`, which newer kernels than the runners'
  6.12 use for these flushes on Zen 3 and later) would make each give-back one instruction on the caller and no interrupt on the other
  cores; the runners are on 6.12. `process_madvise(MADV_DONTNEED)` over a vector of ranges with one flush per call is
  also newer than 6.12 (PR 159 checked).
- Proving that no checker table is read at a freed leaf's addresses, which would allow reusing retired ranges as
  checker-arena chunks with no system call at all.
- A layout that keeps the leaves one checker frees next to each other (the spans are cut by leaves of other checkers
  not freed yet), which needs the checker assignment before the files are parsed.
