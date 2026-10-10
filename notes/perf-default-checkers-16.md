# perf-default-checkers-16: every core gets a checker up to 16

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
regression flag compares those). The run's binary also carried a default `--maxMemory` target, since split out
(notes/perf-default-max-memory.md); all 280 tsrs runs of the 16-vCPU job print `Memory: default target applied: 0`, so
these numbers are the count's alone.

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

## 3. The memory trade against bun check

On the 16-vCPU runner (section 1), tsrs at 8 checkers used less memory than bun on webpack and mui-docs and 3-58% more
on the other five; at 16 checkers it uses less only on mui-docs, and 7% (webpack) to 90% (t3code-server) more on the
other six: vscode 1.11x bun's peak (was 1.03x), formbricks-web 1.21x (1.04x), supabase-studio 1.38x (1.13x), cal-diy
1.43x (1.10x), t3code-server 1.90x (1.58x). In exchange it is 1.5-2.4x faster than bun on those projects instead of
1.1-1.7x (vscode 1.79x, was 1.10x). The README headline (the `wide` table) will show the larger memory ratio from the
next publish on.

## 4. Gates

- `cargo test -p tsrs_compiler default_`: the count rule's table (1-128 cores).
- `tools/ci/determinism.sh` default set (xstate-main, webpack, nuxt, drizzle-orm, testdata/regressions): 576 runs
  identical to the single-threaded output.
- `tools/regressions.sh`: 29 of 29. `tools/lint/ratchet.py` and `tools/lint/source.py` pass.
- bench/run.py `tsrs_default_checkers` mirrors the new rule for the table titles; bench/compare.py, bench/README.md,
  README.md, npm/README.md, npm/tsrs/README.md and docs/PORTING.md state it.

## 5. What remains

- pr-verify runs on 32 vCPU, where the count is 16 before and after, so only bench.yml's `wide` table shows the change.
- Nothing bounds the extra memory of 16 checkers on a machine short of it. The default `--maxMemory` target and its
  open problems (fewer checkers when memory is short among them) are in notes/perf-default-max-memory.md.
- t3code-server still pays for every extra checker re-resolving its Effect declarations (notes/perf-heavy-files.md);
  the per-project count of notes/mem-per-checker-duplication.md section 5 is not adopted.
