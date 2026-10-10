# perf-default-max-memory: `--maxMemory` defaults from the memory available at startup

Split from notes/perf-default-checkers-16.md (every core gets a checker up to 16), where it started: more checkers cost
memory, and without `--maxMemory` nothing bounds it. The measurements below were taken on that combined branch (commit
4afc2705), so at the new checker count.

## 1. The rule

Without `--maxMemory` / `TSRS_MAX_MEMORY`, `tsrs_execute` calls `derive_max_memory_from_available` before it loads a
program that allows checker recycling (the CLI's `--noEmit` check without declaration diagnostics, `--explainFiles` or
Go's check history). It reads the memory available now (`memsplit::available_memory`): `MemAvailable` from
`/proc/meminfo` on Linux, lowered to the tightest cgroup v2 `memory.max` of the process's cgroup and its ancestors
minus that cgroup's working set (`memory.current` minus `inactive_file`), because inside a container `/proc/meminfo`
shows the host; free plus inactive pages from `host_statistics64` on macOS (free includes the speculative pages);
nothing elsewhere. The target is 75% of it. `--maxMemory 0` turns the default off; `TSRS_AVAILABLE_MEMORY=<size>`
stands in for the measured value (tests, experiments).

At parse end (`memory_target`, when the pool is created) the target applies only if both hold:

- **The program could reach it.** Estimated peak = 3x the process's memory at parse end (the front end) + per checker
  the larger of 1/8 of the front end and 128 MiB. Fitted above every peak measured (macOS footprint, base binary
  this branch's binary, no target applied):

  | project | front end | peak at 2 / 4 / 8 / 16 checkers |
  | --- | ---: | --- |
  | vscode | 1,171 MiB | 1,709 / 1,804 / 1,939 / 2,087 |
  | formbricks-web | 730 | 1,238 / 1,439 / 1,688 / 1,994 |
  | supabase-studio | 523 | 1,050 / 1,180 / 1,387 / 1,689 |
  | cal-diy | 406 | 931 / 1,115 / 1,448 / 1,904 |
  | t3code-server | 391 | 946 / 1,187 / 1,784 / 2,147 |
  | mui-docs | 323 | 895 / 1,039 / 1,341 / 1,623 |
  | drizzle-orm | 237 | 421 / 477 / 562 / 783 |
  | webpack | 170 | 343 / 370 / 425 / 505 |
  | xstate-main | 100 | 198 / 221 / 254 / 310 |
  | the 38k-file codebase (notes/mem-recycle-checkers.md) | ~5.9 GB | 15.9 / 18.0 / 21.3 GB at 4 / 8 / 16 |

  The bench projects peak at 1.5-5.5x their front end and add 8-150 MiB per checker; the estimate is at least 23%
  above every cell (t3code-server at 8 checkers is the tightest: 2,197 MiB estimated for 1,784) and up to 2.8x above
  (vscode: a large front end, light checkers). On the bench machines it is far below the target: vscode, the largest
  front end, estimates 4.6 GiB at 8 checkers against ~22 GiB on the 8-vCPU runner and 5.7 GiB at 16 against ~43 GiB
  on the 16-vCPU one.
- **A checker could be retired.** Only a checker whose region holds at least 128 MiB (`retire_min`) is ever retired,
  so the estimate's growth beyond the front end (2x the front end), shared by the checkers, must reach that.

`--extendedDiagnostics` prints the decision: `Memory: available at start`, `default target`, `at parse end`,
`estimated peak` (MiB) and `default target applied` (0 or 1), plus `Checkers: retired` when a target applies; an
explicit `--maxMemory` prints `Memory: target`. The rows go to the tsrs-only table, after Go's; none of bench/run.py's
patterns match them.

## 2. What an armed target costs

Why the second condition, and why the estimate is not padded: an armed target that is never reached is not free.
Measured on the Mac of notes/perf-default-checkers-16.md section 2 (median of 3 interleaved runs, `--maxMemory 1000G`
= armed and never reached, or `TSRS_AVAILABLE_MEMORY=2G` = a 1.5 GiB default target):

| case | peak | instructions | retired |
| --- | ---: | ---: | ---: |
| t3code-server, 16 checkers, armed, never reached | 2,166 -> 2,545 MiB (+17%) | within noise | 0 |
| vscode, 16 checkers, armed, never reached | 2,099 -> 2,157 MiB (+3%) | within noise | 0 |
| cal-diy, webpack, 16 checkers, armed, never reached | +0-1% | within noise | 0 |
| t3code-server, 4 checkers, armed, never reached | 1,190 -> 1,463 MiB (+23%) | +10%, wall up too | 0 |
| t3code-server, 16 checkers, 2 GiB available (before the second condition) | 2,162 -> 2,537 MiB (+17%) | +0% | 0 |
| vscode, 4 checkers, 2 GiB available (target 1.5 GiB) | 1,793 -> 1,540 MiB (-14%) | +4% | 3-4 |

At 16 checkers no bench project's checker reaches the 128 MiB floor (they grow 57-110 MiB each: peak minus front end,
over 16),
so a target cannot retire anything there and only costs the armed mode's memory. With the second condition the
t3code-server 16-checker case now runs the plain pass. On t3code-server at 4 checkers the leaves-first queue order of
the armed mode is the instruction cost (`TSRS_LEAVES_FIRST=0`: instructions back to base, peak +13%); the per-checker
regions are the rest of the memory. The relation-cache limit is not involved (`TSRS_RELATION_CACHE_LIMIT=1e9` changes
nothing). Where the target does act (vscode at 4 checkers with 2 GiB available) it held the peak to the target with
3-4 retirements.

So the default target protects programs whose checkers grow large (the 38k-file codebase class: ~1-1.5 GB per checker),
on machines that cannot hold their peak. It does not bound the extra memory of 16 checkers on a bench-size project:
those checkers stay below the retirement floor, and on a machine short of memory for them the pass runs exactly as
before this change. The bench machines and a developer machine with a few GiB free never get it.

## 3. On the bench runners

`bench.yml` on the combined branch (Depot run `25p0w131bz`, notes/perf-default-checkers-16.md section 1b): all 280
tsrs runs of the 16-vCPU job print `Memory: default target applied: 0` (58.5 GB available, target ~43 GB). Their
`Memory: at parse end` rows give the Linux check of the estimate: at 16 checkers it is 1.5x (t3code-server) to 7x the
measured RSS peak on every project.

## 4. Open problem: the middle band

When the estimate is above the target but the real peak never gets there, the pass is armed but idle and still pays for
it (section 2: per-checker regions, leaves-first order). And at 16 checkers no bench project's checker reaches the
128 MiB retirement floor, so on exactly the programs where 16 checkers add memory the target cannot act. Candidates,
none done:

- Arm at runtime instead of at parse end: start plain and switch when the process crosses a fraction of the target.
  Needs late region entry, or abandoning the largest arena-born checker for a fork of the #281 seed (its growth stops,
  its memory is not reclaimed).
- Make an armed-but-idle pass free: keep program order until a first retirement is needed, and cut the per-checker
  region slab overhead.
- Fit the estimate to medians instead of the maximum and accept an occasional late retirement.
- Fewer checkers when memory is short: derive the count from the available memory. The only variant that helps
  bench-size programs at 16 checkers.

## 5. Gates

- `cargo test -p tsrs_compiler default_memory_target`: the estimate covers every measured peak; the bench machines
  never get a default target; it applies to the 38k-file codebase on a 16 GB machine and not to t3code-server at 16
  checkers with 2 GiB available.
- `crates/tsrs_cli/tests/recycle_checkers.rs`: with `TSRS_AVAILABLE_MEMORY=1M` every testdata/regressions case
  retires checkers under the default target and prints tsgo-ref's expected output; `--maxMemory 0` turns it off.
- `tools/ci/determinism.sh` default set (xstate-main, webpack, nuxt, drizzle-orm, testdata/regressions) with the
  default target forced on and no retirement floor (`TSRS_AVAILABLE_MEMORY=1M TSRS_RETIRE_MIN=0`, e.g. 48 retirements
  on drizzle-orm at 4 checkers): 256 runs identical to the single-threaded output.

## 6. What remains

- The estimate is checked on Linux RSS only at 16 checkers (section 3); pr-verify's logs (1/4/16/32 checkers) carry
  `Memory: at parse end` next to each run's peak for the other counts.
- cgroup v1 limits are not read; Windows has no reading (no default target).
