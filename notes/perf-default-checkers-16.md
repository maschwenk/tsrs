# perf-default-checkers-16: every core gets a checker up to 16; a default `--maxMemory` target

Without `--checkers`, tsrs ran `clamp(max(cores / 2, min(cores, 8)), 4, 32)` checker threads
(notes/perf-default-checkers-small-machines.md): every core up to 8, then half the cores. The comment justified the
halving with "the other half parse", but parsing and checking are sequential phases: by the time the checkers start,
the parse pool is idle. The README headline bench moved to a 16-vCPU runner on 2026-10-08, where that rule gives 8,
and the same bench already measured `--checkers 16` on that machine. The rule is now
`clamp(max(cores / 2, min(cores, 16)), 4, 32)`:

| cores | 4 | 8 | 12 | 16 | 18 | 32 | 34 | 48 | 64+ |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| before | 4 | 8 | 8 | 8 | 9 | 16 | 17 | 24 | 32 |
| after | 4 | 8 | 12 | 16 | 16 | 16 | 17 | 24 | 32 |

The small-program floor (one checker per 32 type-checked files beyond Go's 4) and `-b`'s 4 per project are
unchanged. Up to 8 cores and from 32 cores on nothing changes, so the 8-vCPU fixed-spec tables, pr-verify (32 vCPU)
and the 64-vCPU numbers are unaffected. Leaf freeing stays on (it is on up to 16 checkers, fileregions.rs).

More checkers cost memory, so the change comes with a default for `--maxMemory` (#265): without the flag, the CLI's
`--noEmit` check derives a target from the memory available at startup and applies it only where it can do something
(section 3).

## 1. Evidence: the 16-vCPU runner, 8 against 16 checkers (before the change)

bench/results/2026-10-10-7e1b71a15874.md (Depot `depot-ubuntu-24.04-16`, 16 vCPU / 63 GB, PGO + BOLT `dist` build,
median of 10; the 8-checker cells are its "Default mode on a 16-vCPU machine" table, the 16-checker cells its
`--checkers 16` table, bun at its default of 16 threads):

| project | tsrs 8 wall | tsrs 16 wall | wall | tsrs 8 peak | tsrs 16 peak | peak | bun wall | bun peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 1.38 s | 0.85 s | -38% | 1.92 GiB | 2.08 GiB | +8% | 1.52 s | 1.87 GiB |
| supabase-studio | 0.95 | 0.68 | -28% | 1.38 | 1.68 | +22% | 1.60 | 1.22 |
| webpack | 0.19 | 0.14 | -26% | 455 MiB | 533 MiB | +17% | 0.27 | 497 MiB |
| cal-diy | 0.84 | 0.66 | -21% | 1.44 | 1.87 | +30% | 1.45 | 1.31 |
| formbricks-web | 0.85 | 0.67 | -21% | 1.68 | 1.95 | +16% | 1.02 | 1.61 |
| mui-docs | 1.06 | 0.88 | -17% | 1.30 | 1.58 | +22% | 7.33 | 11.67 |
| t3code-server | 1.69 | 1.42 | -16% | 1.77 | 2.13 | +20% | 2.70 | 1.12 |

Every project gets 16-38% faster for 8-30% more peak memory.

## 1b. The branch on the bench workflow

`bench.yml` dispatched on this branch (Depot run `25p0w131bz`, commit 4afc2705, PGO + BOLT `dist` build, artifact
`bench-result`, not published), against the published run of its base 9c905fb (bench/results/2026-10-10-9c905fb5ce10.md).
The `wide` table (the README headline: each tool at its default on `depot-ubuntu-24.04-16`, median of 10) now runs
tsrs at 16 checkers:

| project | base wall (8) | branch wall (16) | wall | base peak | branch peak | peak | bun wall | bun peak | vs bun, wall | vs bun, peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 1.48 s | 0.84 s | -43% | 1.93 GiB | 2.07 GiB | +8% | 1.53 s | 1.87 GiB | 1.82x faster | 1.11x |
| mikro-orm | 1.15 | 0.78 | -32% | 1.61 | 1.94 | +20% | 1.78 | 1.41 | 2.28x | 1.37x |
| next-packages-next | 0.30 | 0.21 | -31% | 617 MiB | 694 MiB | +13% | 0.37 | 740 MiB | 1.77x | 0.94x |
| supabase-studio | 0.93 | 0.66 | -29% | 1.38 | 1.68 | +22% | 1.60 | 1.22 | 2.41x | 1.38x |
| webpack | 0.19 | 0.14 | -25% | 456 MiB | 533 MiB | +17% | 0.28 | 496 MiB | 1.97x | 1.07x |
| formbricks-web | 0.84 | 0.65 | -23% | 1.68 | 1.96 | +17% | 1.02 | 1.62 | 1.57x | 1.21x |
| storybook | 0.21 | 0.16 | -23% | 460 MiB | 514 MiB | +12% | 0.28 | 613 MiB | 1.75x | 0.84x |
| playwright | 0.17 | 0.13 | -23% | 413 MiB | 494 MiB | +20% | 0.22 | 477 MiB | 1.73x | 1.04x |
| cal-diy | 0.85 | 0.66 | -22% | 1.44 | 1.86 | +29% | 1.50 | 1.30 | 2.27x | 1.43x |
| mui-docs | 1.06 | 0.89 | -16% | 1.30 | 1.58 | +22% | 7.43 | 10.33 | 8.37x | 0.15x |
| t3code-server | 1.70 | 1.45 | -15% | 1.77 | 2.13 | +20% | 2.78 | 1.12 | 1.92x | 1.91x |
| nuxt | 0.21 | 0.18 | -15% | 442 MiB | 519 MiB | +17% | 0.44 | 542 MiB | 2.47x | 0.96x |
| next-root | 0.21 | 0.19 | -11% | 438 MiB | 483 MiB | +10% | 0.30 | 568 MiB | 1.60x | 0.85x |
| drizzle-orm | 0.21 | 0.19 | -8% | 594 MiB | 804 MiB | +35% | 0.41 | 733 MiB | 2.11x | 1.10x |
| xstate-main | 0.13 | 0.13 | +4% | 280 MiB | 281 MiB | 0% | 0.16 | 395 MiB | 1.25x | 0.71x |
| Compiler | 0.07 | 0.07 | +4% | 98 MiB | 99 MiB | +1% | 0.12 | 221 MiB | 1.59x | 0.45x |
| Compiler-Unions | 0.13 | 0.13 | +4% | 99 MiB | 102 MiB | +4% | 0.21 | 232 MiB | 1.59x | 0.44x |

Fourteen projects are 8-43% faster for 10-35% more peak memory; xstate-main, Compiler and Compiler-Unions keep their
count (the file-count floor) and move by one rounding step. The 8-vCPU tables (8 checkers before and after) are
unchanged: default-mode peaks within 11 MiB (0.7%), single-threaded instructions within -0.16..+0.07% (the
regression flag compares those). All 280 tsrs runs of the 16-vCPU job print `Memory: default target applied: 0`
(58.5 GB available, target ~43 GB). Their `Memory: at parse end` rows give the Linux check of the estimate: at 16
checkers it is 1.5x (t3code-server) to 7x the measured RSS peak on every project.

## 2. This Mac (indicative only)

Apple M5 Max, 18 cores (6P + 12E), 128 GB, macOS; release build (fat LTO, no PGO); `tsrs -p <p> --noEmit
--incremental false --extendedDiagnostics --pretty false`; `/usr/bin/time -l` peak memory footprint and instructions
retired; median of 5 interleaved runs. Other agents were building on the machine (1-minute load 15-23 on 18 cores), so
the wall column is noisy. The base (origin/main 9c905fb) runs 9 checkers here, the branch 16:

| project | 9 wall | 16 wall | wall | 9 check | 16 check | 9 peak | 16 peak | peak | instructions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 1.11 s | 0.83 s | -25% | 0.88 s | 0.58 s | 1,972 MiB | 2,098 MiB | +6% | +4% |
| cal-diy | 0.77 | 0.65 | -16% | 0.58 | 0.49 | 1,496 | 1,892 | +26% | +22% |
| formbricks-web | 0.85 | 0.79 | -7% | 0.59 | 0.51 | 1,732 | 1,984 | +15% | +15% |
| supabase-studio | 0.94 | 0.77 | -18% | 0.69 | 0.50 | 1,416 | 1,686 | +19% | +17% |
| t3code-server | 1.59 | 1.45 | -9% | 1.48 | 1.32 | 1,831 | 2,157 | +18% | +12% |
| mui-docs | 1.28 | 1.12 | -12% | 0.95 | 0.75 | 1,393 | 1,614 | +16% | +10% |
| webpack | 0.18 | 0.13 | -28% | 0.13 | 0.09 | 443 | 499 | +13% | +8% |
| xstate-main | 0.11 | 0.11 | 0% | 0.08 | 0.08 | 247 | 246 | 0% | -1% |
| drizzle-orm | 0.22 | 0.20 | -9% | 0.13 | 0.10 | 608 | 785 | +29% | +17% |

Error counts identical in every run. xstate-main gets the same count from both rules (the file-count floor keeps it
below 9). The instruction column is the duplicated first-touch work of seven more checkers, the cost the
`--checkers 16` rows always had; single-threaded instructions do not change (webpack and xstate-main within the Mac's
+-1% run-to-run noise; pr-verify has the Linux counts).

## 3. The default memory target

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

Why the second condition, and why the estimate is not padded: an armed target that is never reached is not free.
Measured on this Mac (median of 3 interleaved runs, `--maxMemory 1000G` = armed and never reached, or
`TSRS_AVAILABLE_MEMORY=2G` = a 1.5 GiB default target):

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

## 4. The memory trade against bun check

On the 16-vCPU runner (section 1), tsrs at 8 checkers used less memory than bun on webpack and mui-docs and 3-58% more
on the other five; at 16 checkers it uses less only on mui-docs, and 7% (webpack) to 90% (t3code-server) more on the
other six: vscode 1.11x bun's peak (was 1.03x), formbricks-web 1.21x (1.04x), supabase-studio 1.38x (1.13x), cal-diy
1.43x (1.10x), t3code-server 1.90x (1.58x). In exchange it is 1.5-2.4x faster than bun on those projects instead of
1.1-1.7x (vscode 1.79x, was 1.10x). The README headline (the `wide` table) will show the larger memory ratio from the
next publish on.

## 5. Gates

- `cargo test -p tsrs_compiler default_`: the count rule's table (1-128 cores); the estimate covers every measured
  peak; the bench machines never get a default target; it applies to the 38k-file codebase on a 16 GB machine and not
  to t3code-server at 16 checkers with 2 GiB available.
- `crates/tsrs_cli/tests/recycle_checkers.rs`: with `TSRS_AVAILABLE_MEMORY=1M` every testdata/regressions case
  retires checkers under the default target and prints tsgo-ref's expected output; `--maxMemory 0` turns it off.
- `tools/ci/determinism.sh` default set (xstate-main, webpack, nuxt, drizzle-orm, testdata/regressions): 576 runs
  identical to the single-threaded output. Again with the default target forced on and no retirement floor
  (`TSRS_AVAILABLE_MEMORY=1M TSRS_RETIRE_MIN=0`, e.g. 48 retirements on drizzle-orm at 4 checkers): 256 runs identical.
- `tools/regressions.sh`: 28 of 28. `tools/lint/ratchet.py` and `tools/lint/source.py` pass.
- bench/run.py `tsrs_default_checkers` mirrors the new rule for the table titles; bench/compare.py, bench/README.md,
  README.md, npm/README.md, npm/tsrs/README.md and docs/PORTING.md state it.

## 6. What remains

- pr-verify runs on 32 vCPU, where the count is 16 before and after, so only bench.yml's `wide` table shows the change.
- The estimate is checked on Linux RSS only at 16 checkers (section 1b); pr-verify's logs (1/4/16/32 checkers) carry
  `Memory: at parse end` next to each run's peak for the other counts.
- The armed mode's own cost (per-checker regions and leaves-first order: +3-23% peak, up to +10% instructions when no
  checker is retired) is why the default target is gated. Arming the expensive parts only once the process first
  nears the target would let it apply earlier and on more programs.
- A memory-aware checker count (fewer checkers when the estimate at the default count exceeds the target) would be the
  lever for bench-size projects on small machines, which the retirement floor leaves unprotected; not done.
- cgroup v1 limits are not read; Windows has no reading (no default target).
- t3code-server still pays for every extra checker re-resolving its Effect declarations (notes/perf-heavy-files.md);
  the per-project count of notes/mem-per-checker-duplication.md section 5 is not adopted.
