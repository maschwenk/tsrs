# perf-icount-assign: which checker checks which file, at 8 checkers

Question (2026-10-11, part of the instruction-count hill climb in notes/perf-icount-hillclimb.md): on the large private
monorepo (38k files) at `--checkers 8`, the default mode (work stealing) retires ~4% more instructions than
`--checkerAssignment locality` (no stealing), and both retire ~65% more than one checker. Can the assignment or the
stealing rule take some of that back without costing wall time? Output does not depend on the assignment
(notes/perf-order-independence.md), so only counters, instructions and timing can change.

Result in one paragraph: on this workload, no. The module-affinity defaults (mu 1, gamma 0.5, 3 passes) are the
optimum of every knob swept, exactly so in the Linux counter; stealing granularity, victim choice and gates on when to
steal are all within the run-to-run noise (~1% in the default mode). What stealing costs is a handful of first
touches: 1,806 stolen files cost 15.5 G on their owners and 30.9 G on the thieves, 10 files carry 75% of the
difference, and nothing static predicts which. The same mechanism is far worse on t3code-server, where five thieves
each paid ~8 G for one test directory at the end of the pass (119 G with stealing against 75 G without). One change
landed: from 8 to 15 checkers at most a quarter of the checkers steal from any one queue at a time. At 8 checkers:
t3code-server -20% instructions, mui-docs -4% (and no longer bimodal), cal-diy -0.6%, this workload -0.3% or nothing
(within the noise), vscode and supabase-studio unchanged; check time lower or unchanged on all six. Plus a diagnostic:
`TSRS_ASSIGNMENT_STATS=times` prints each checker thread's instructions on macOS.

Base: 98d8d60e. Mac: Apple M5 Max, `cargo build --release`, `/usr/bin/time -l` instructions retired (user + kernel),
shared with four other agents (load 10-60), so wall and CPU seconds move by 10-30% between minutes; instructions do
not, but the default mode's count does depend on the load (below). Linux: the DynamoRIO user-space counter in an
OrbStack VM (exact in locality mode). Workload command: `-p <tsconfig> --noEmit --incremental false --pretty false
--checkers 8`. Bench projects with the same flags from their checkout roots (vscode `-p src`, cal-diy `-p apps/web`,
t3code-server `-p apps/server`, supabase-studio `-p apps/studio`).

## Result

| | base 98d8d60e | branch | change |
| --- | --- | --- | --- |
| default mode, measure.sh, 3 runs, two sets | 365.6 / 361.8 / 362.3, 370.1 / 361.9 / 362.5 G | 361.1 / 362.6 / 361.1, 363.3 / 362.9 / 361.3 G | -0.3%, +0.1% (medians) |
| default mode, limit off / on, 6 interleaved runs | 362.8 G (mean, sd 1.6) | 361.7 G (sd 1.3) | -0.3% |
| `--checkerAssignment locality` | 348.2-349.7 G | 348.2-348.5 G | unchanged (no stealing) |
| single-threaded | 213.07-213.12 G | 213.05-213.22 G | unchanged |
| peak RSS, default mode | 6.39-6.43 GB | 6.38-6.45 GB | unchanged |
| check time, same 6 interleaved runs | 3.65 s | 3.60 s | within the noise |
| t3code-server at 8 checkers, limit off / on | 118.8 G, check 1.43 s | 95.2 G, 1.37 s | -20% |
| mui-docs at 8 checkers, limit off / on | 92.2-98.1 G, check 0.85 s | 91.8-97.5 G (5 of 6 at 91.8-92.0), 0.80 s | -4% (medians) |

Identity (all four md5s), conformance pass lists (default mode, locality, and with stealing under
`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical` with the limit off and forced to 1) and
`tools/regressions.sh` (29/29) unchanged.

## 1. What stealing costs on this workload

Per-checker instructions (the new `TSRS_ASSIGNMENT_STATS=times` line, macOS `thread_selfcounts`), one run each:

| | per checker (G) | sum | max |
| --- | --- | ---: | ---: |
| locality (no stealing) | 43.4 32.1 35.8 40.6 45.2 36.0 27.0 45.2 | 305.3 | 45.2 |
| default (stealing) | 40.9 39.3 39.6 40.4 39.4 41.3 40.5 40.1 | 321.6 | 41.3 |

The static weights of the eight checkers are within 2.5% of each other (16.8-17.2 M), their instructions 27-45 G.
Stealing balances them (the pass ends ~10% sooner by instructions, 25-30% sooner by check time on a quiet minute: 3.6 s
against 5.1 s) at +16 G. Per file (`TSRS_FILE_TIMES` with each file's thread instructions, a locality run against a
default run): the 36,136 files that stayed home cost the same (+0.5 G); the 1,806 stolen ones cost 15.5 G on their
owners and 30.9 G on the thieves. The difference is concentrated: the 10 most expensive stolen files carry 75% of it,
the top 50 99%. They are small, ordinary files (a few hundred to a few thousand nodes, 2-27 imports) that cost
0.002-0.06 G at home and up to 4.5 G on the thief: the first file on that checker to need some large shared type graph.
By thief and victim the ratio ranges from 1.04 (409 files) to 23 (9 files).

Nothing static predicts it. The transitive import closure of almost every checked file is the same 24.6k files; the
worth of a stolen file's direct imports that its thief already holds does not track the ratio (0.15 held for the 1.04
pair, 0.50 for the 23 one); per checker, a fit of instructions to static weight plus the worth (node count^g) of the
modules held, g = 0, 0.5 or 1, leaves an rms error of 5.6-6.1 G, the spread itself. The costliest files at home are the
same kind: 2-8.5 G each, a few per checker, the first touch of a large graph.

Touched files (`TSRS_ASSIGNMENT_STATS=1`: files with any node or symbol links in a checker), summed over checkers:
57,540 with the locality assignment, 60,485 with stealing (+5.1%; types +5.4%, instantiations +6.7%); the branch
60,015 (one run each; types 18.25 M against 18.43 M). On t3code-server at 8 checkers: 6,563 static, 8,456 with
stealing, 7,579 on the branch (types 3.15 / 4.53 / 3.80 M, instantiations 24.9 / 49.1 / 35.0 M).

The default mode's count depends on the machine's load: the same binary measured 365.5 G (mean of 18 runs) at load
25-60 and 361.9 G (10 runs) at load ~13. Descheduled checkers make the run-time imbalance larger and more files move.

## 2. Knob sweep (experiment 1)

Base binary, env knobs, sequential sets at load ~25; default mode 3 runs (median), locality 1 run (2 for the base),
stolen files and check time from one `TSRS_ASSIGNMENT_STATS=times` run. Sets at load 25 read ~3 G high against the
quiet table above.

| setting | default G | locality G | stolen |
| --- | ---: | ---: | ---: |
| defaults (mu 1, gamma 0.5, 3 passes, sticky, divisor 200) | 367.0 | 351.3 | 1,791 |
| `TSRS_MODULE_AFFINITY=off` | 381.1 | 369.2 | 1,335 |
| mu 0.5 / 0.7 / 1.5 / 2 | 376.7 / 373.0 / 373.0 / 370.5 | 361.8 / 352.9 / 358.9 / 364.3 | |
| gamma 0.3 / 0.7 / 1 | 365.8 / 371.1 / 367.0 | 362.5 / 351.5 / 360.4 | |
| passes 0 / 1 / 10 | 370.7 / 369.4 / 371.6 | 358.8 / 353.5 / 353.7 | |
| `TSRS_STEAL_STICKY=0` | 367.0 | | 1,609 |
| `TSRS_HEAVY_SHARE_DIVISOR` 1 / 50 / 100 / 400 / 1000 | 362.6 / 364.1 / 366.4 / 365.8 / 368.9 | | |

Exact locality counts (Linux counter, one run each; repeat spread 0.02%): defaults 322.73 G; mu 0.8 329.77 (+2.2%),
0.9 323.65 (+0.3%), 1.1 327.99 (+1.6%), 1.25 326.95 (+1.3%); gamma 0.4 323.91 (+0.4%), 0.6 327.61 (+1.5%); passes 2
324.08 (+0.4%), 4 323.36 (+0.2%), 5 323.02 (+0.1%). The defaults are the minimum.

On the bench projects these knobs do nothing at 8 checkers: the affinity and the sticky thief apply there only from 16
checkers (they have under 24,000 checked files). The heavy-first divisor does apply: with no heavy files (divisor 1)
the bench projects took 0.7-4.3% fewer instructions at 8 checkers, and this workload 0.2-0.6%, under load. Not
changed: the rule exists for the tail at 11+ checkers, at 8 its wall effect was not separable from the load here, and
on t3code-server one 3-run set had the check 17% longer without it.

## 3. Stealing granularity and victim choice (experiment 2)

Experimental knobs on the base, default mode, 3 runs per setting at load ~20-25 (each set has its own control):

| variant | default G | control |
| --- | ---: | ---: |
| steal a run of files from the back weighing 1/d of the victim's work left, d = 2 / 4 / 8 / 16 | 365.8 / 362.6 / 368.0 / 365.6 | 364.8 |
| same, d = 4, at most 1/16 of a share per claim | 364.5 | 364.8 |
| victim = the queue (with at least 1/k of the most work left) whose last file's modules the thief holds most of, k = 2 / 4 / 8 / 100 | 368.5 / 367.5 / 368.8 / 368.9 | 365.4 |
| start on a new victim only while it has 1/d of a share left, d = 8 / 16 / 32 / 64 | 362.5 / 361.1 / 362.2 / 365.5 | 363.7 |

A steal log (`thief`, `victim`, time, work left per steal) shows the stealing is not a tail effect: the cheapest checker
runs dry at ~70% of the pass and steals for the rest; thieves switch victims 0-2 times; on one victim's back two thieves
took turns, the thief changing 120 times over its 692 stolen files. Claiming runs of files (no interleaving) did not
lower the count, and private runs made the end uneven (d = 2: 2,473 files stolen instead of 1,800). The new-victim gate
looked like -0.8% in 18 interleaved runs at load 25-60 and was -0.1% in 10 runs at load ~13: it only trims the extra
stealing that load causes.

Rules that limit how many thieves share a victim, 6-18 interleaved runs (default mode, this workload): one thief per
queue unless it has a whole share left -0.3% to -0.5%; at most m thieves per queue (the first version of the landed
rule, section 4), m = 1: -0.5% to -0.7%, m = 2: -0.3% to -0.5%; check time and slowest-checker CPU unchanged. A rule
that lets others join once the first thief's time on a queue's files was at most twice its owner's per weight unit did
nothing on t3code-server (113.6 G): the early files of that queue were cheap for a cold checker, the last ones were not.

## 4. What landed: at most checkers/4 thieves per queue at a time, from 8 to 15 checkers

t3code-server at 8 checkers: the locality assignment checks it with 75 G (check 1.06-1.28 s); with stealing, 119 G
(1.27-1.71 s in the same interleaved sets). Per-file instructions show five files of one CLI test directory, 0.01-0.05 G
each on their owner, costing 7-9 G each on five different thieves: near the end every idle thief converged on the last
queue with work and each took one of them from its back, so each built the same type graph (on its owner the first file
of the directory built it once, 8.3 G). The owner itself then paid for it again on the one it kept, its first file of
the directory having been taken.

A first version limited the number of checkers that had ever taken from a queue (a bit per thief, never cleared). With
m = 1 or 2 it took 87 / 94 G on t3code-server, but it made mui-docs worse: a checker that took a few files from a queue
and moved on kept its place, the two checkers that do most of mui-docs' stealing were shut out of queues they would
have taken from, and every run landed in the slow mode that one run in three of the base hits (96-97 G, check 0.98 s
against 92 G, 0.85 s). The landed version counts the thieves of a queue now: a thief gives up its place when it moves to
another queue (`FileQueue::thieves`, `admit` / `release`); a thief that no queue with work admits stops, and the owners
finish their queues. Against no limit (same binary, `TSRS_STEAL_MAX_THIEVES=0`), interleaved, 8 checkers:

| project | runs | m = 2 (landed) | check time, m = 2 | m = 1 | check time, m = 1 |
| --- | ---: | --- | --- | --- | --- |
| t3code-server | 6 | 118.8 -> 95.2 G (-20%) | 1.434 -> 1.370 s | 85.6 G (-28%) | 1.235 s |
| mui-docs | 6 | 96.2 -> 92.0 G (-4%) | 0.847 -> 0.796 s | 91.6 G | 0.762 s |
| cal-diy | 6 | 68.2 -> 67.8 G (-0.6%) | 0.629 -> 0.609 s | 65.3 G (-4.3%) | 0.583 s |
| supabase-studio | 6 | 72.75 -> 72.72 G | 0.572 -> 0.574 s | 72.38 G | 0.561 s |
| vscode | 8 | 97.11 -> 97.17 G | 0.987 -> 0.976 s | 97.25 G | 0.988 s |
| this workload | 6 | 362.8 -> 361.7 G (-0.3%) | 3.650 -> 3.602 s | 361.8 G | 3.537 s |

m = 1 saves more but costs vscode check time where there are more idle thieves than queues with work: 10 runs at 9
checkers, 0.784 s without a limit, 0.784 with m = 2, 0.802 with m = 1 (+2.3%); at 12 checkers 0.630 / 0.629 (m = 3) /
0.641 s (+1.7%). The sticky-bit version of m = 1 had cost vscode +7% at 16 and +3.8% at 4 checkers. So the limit is
checkers/4 (2 at 8-11, 3 at 12-15), and only below 16 checkers: from 16 on the sticky thief
(perf-clustered-assignment.md) applies to every project, and the limit there was measured only in the sticky-bit
version, 4 runs per cell (m = 2-4 at 16: t3code-server -0.7% to -15%, check time noisy). `TSRS_STEAL_MAX_THIEVES=<m>`
sets it at every count, `0` for none.

On the Linux counter, default mode, the sticky-bit binary with m = 2 against m = 0 (one run each, alternated): 335.24 /
334.06 G and 339.37 / 337.36 G (-0.4%, -0.6%; the default mode's spread there is 1.7%).

Risk: a queue whose thieves are all busy elsewhere is finished by its owner alone, and a thief that finds no queue it
may take from stops for the rest of the pass. In the measured cells the check time did not get longer; on a quiet
machine it was not measured (the Mac's load never went below 10).

## 5. A static balance (experiment 3)

`--checkerCostCache` (opt-in, measured per-file CPU from a locality run, then the default mode): 362.0 G against ~365 at
the same load, 1,400 files stolen instead of 1,800, locality 350.0 G. So better static balance helps a little, but it
needs a previous run. A static estimate cannot get there here (section 1: per-checker cost is set by which checkers
first touch a few large type graphs, and no static measure we tried tracks it), and the notes already reject static
cost models and fitted per-file models (notes/perf-round2-followups.md). Not pursued.

## Measured and rejected

- Affinity knobs other than the defaults (section 2): up to +2.2% locality (exact), +1-4% default.
- `TSRS_STEAL_STICKY=0`: unchanged (367.0 against 367.0 G).
- Stealing runs of files instead of single files (d = 2-16): within the noise, d = 2 moves more files.
- Victim by module overlap with the thief: +0.6% to +1.0%.
- Start on a new victim only while it has 1/d of a share left: -0.1% at load ~13 (only trims load-induced stealing).
- Let others join a queue once its first thief found its files cheap: t3code-server 113.6 G (m = 2: 94 G).
- One thief per queue at a time: vscode +2.3% check time at 9 checkers, +1.7% at 12. Counting every checker that ever
  took from a queue instead of the current thieves: mui-docs +4% instructions and +15% check time at 8 checkers.

## Found, not pursued

- The heavy-first order looked like 0.7-4.3% of the bench projects' instructions at 8 checkers (section 2), but most of
  that was the same end-of-pass pile-up: with the limit (its first version), without heavy files, vscode moves 97.09 ->
  97.05 G and t3code-server 93.5 -> 92.7 G (8 runs each), so it was left alone.
- The rest of t3code-server's stealing premium (95 G against 75 G static) and the same shape at 16+ checkers
  (136 G against 117 G with m = 2 at 16).
- Per-file instructions in `TSRS_FILE_TIMES` (here a local patch writing them in place of the wall seconds) would
  make the analysis in section 1 reproducible from a committed binary.

## Reproduce

```sh
# per-checker instructions (macOS), stolen files, check time:
TSRS_ASSIGNMENT_STATS=times tsrs -p <tsconfig> --noEmit --incremental false --pretty false --checkers 8 --extendedDiagnostics
# files touched per checker:
TSRS_ASSIGNMENT_STATS=1 tsrs ... --checkers 8 --extendedDiagnostics
# the limit off, and one thief per queue:
TSRS_STEAL_MAX_THIEVES=0 tsrs ... --checkers 8
TSRS_STEAL_MAX_THIEVES=1 tsrs ... --checkers 8
```
