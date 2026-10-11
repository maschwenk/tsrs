# mem-retire-minimum: keep retiring small checkers under `--maxMemory`

Question (2026-10-11): can the checker pass give memory back sooner, and what is the lowest peak RSS reachable on a
large private monorepo? The existing `--maxMemory` policy stops retiring a checker once its region is smaller than
128 MiB, even when the process remains above the requested target.

Answer: make the default minimum zero. At a 3 GiB target, continuing to retire small checker regions reduces peak RSS
from 3.3426 to 3.0481 GiB (-8.81%) for +7.34% instructions and +2.72% wall. This affects only the opt-in memory-target
mode. `TSRS_RETIRE_MIN` remains available to restore a nonzero floor when avoiding recomputation matters more than
meeting the target.

The lowest point measured was 2.1912 GiB with a 2 GiB target and a zero minimum, 51.83% below the uncapped peak. It is
not a good general setting: instructions were about 3x uncapped and wall was +68.5%. Below 2 GiB the peak rose again,
because repeatedly rebuilding fresh checker state became the peak. The target is a control, not a hard limit; the
front end and one set of fresh checker state cannot be retired during the pass.

## Why the old floor retained memory

Each checker allocates its retained types, symbols, links and heap tables in a region. Above the process target, the
pool retires the largest checker between files and replaces it with a fresh checker. The old 128 MiB floor prevented
retirement once every individual checker was below 128 MiB, although the small live regions collectively accounted
for several hundred MiB. A zero floor lets the same already-tested retirement path continue. It introduces no new
lifetime rule; the poison-mode regression test already exercised a retirement after nearly every file with
`TSRS_RETIRE_MIN=0`.

The original floor was chosen from a different, much larger workload: fresh checkers there rebuild hundreds of MiB
of shared library state, so small retirements buy little memory for substantial work. The new evidence is a workload
where the floor itself misses a 3 GiB target by about 11%, and an explicit requirement to prioritize maximum RSS.
Users who prefer the old trade can set `TSRS_RETIRE_MIN=128M`.

## Final measurement

Release build with symbols and debug information, macOS, default checker count. `hyperfine 2.0.0`, one warmup and
five measured runs per command. All commands used a cold no-emit check of a large private monorepo; nonzero status is
the workload's diagnostics. Instructions are hardware instructions retired.

| mode | wall, mean | instructions, mean | peak RSS, mean |
| --- | ---: | ---: | ---: |
| uncapped | 6.230 s | 256.760 B | 4.5488 GiB |
| `--maxMemory 3G`, 128 MiB minimum | 6.881 s | 317.578 B | 3.3426 GiB |
| `--maxMemory 3G`, zero minimum | 7.069 s | 340.894 B | 3.0481 GiB |
| `--maxMemory 2G`, zero minimum | 10.499 s | 768.708 B | 2.1912 GiB |

At 3 GiB, zero versus 128 MiB is +2.72% wall, +7.34% instructions, -8.81% peak RSS. Against uncapped, the new 3 GiB
behavior is +13.45% wall, +32.77% instructions and -32.99% peak RSS. The absolute-minimum 2 GiB point is +68.51%
wall, +199.39% instructions and -51.83% peak RSS against uncapped.

After changing the default and rebuilding, a three-run `--maxMemory 3G` check with no environment override measured
7.045 s, 338.301 B instructions and 3.0454 GiB peak RSS. Its plain diagnostics were byte-identical to the pre-change
uncapped output.

## Public benchmark matrix

The same-binary comparison was also run on all 17 projects in `bench/projects.json`, at the default checker count.
Each project's target was 75% of its uncapped peak RSS rounded down to 64 MiB (with a 64 MiB lower bound), so the
test applies comparable pressure instead of choosing one absolute target that most projects never reach.
`hyperfine 2.0.0` ran one warmup and three measured runs per variant. The two noisy neutral cases were repeated for
seven runs, and drizzle-orm was also repeated for seven runs in reverse order. macOS `/usr/bin/time -l` supplied
child-process instructions and maximum RSS. Deltas are zero minimum versus the old 128 MiB minimum; RSS is the mean
maximum RSS.

| project | target | old RSS, MiB | zero RSS, MiB | RSS | wall | instructions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 1536M | 2125.9 | 1564.2 | -26.42% | +3.33% | +9.67% |
| xstate-main | 128M | 268.0 | 224.0 | -16.39% | +28.15% | +35.06% |
| webpack | 320M | 478.4 | 385.8 | -19.36% | +20.24% | +26.04% |
| mui-docs | 1088M | 1525.3 | 1291.0 | -15.36% | +1.20% | +5.17% |
| Compiler | 64M | 77.5 | 77.4 | -0.05% | -0.51% | -0.04% |
| Compiler-Unions | 64M | 82.7 | 83.0 | +0.38% | -2.18% | -0.04% |
| cal-diy | 1216M | 1722.5 | 1277.3 | -25.84% | +6.27% | +24.99% |
| formbricks-web | 1344M | 1910.2 | 1398.8 | -26.77% | +15.71% | +46.64% |
| supabase-studio | 1152M | 1627.3 | 1196.6 | -26.46% | +10.44% | +38.78% |
| t3code-server | 1472M | 2429.9 | 2026.2 | -16.61% | +12.31% | +8.31% |
| mikro-orm | 1408M | 1897.6 | 1900.6 | +0.16% | -6.32% | -0.06% |
| next-packages-next | 448M | 641.8 | 488.3 | -23.92% | +5.30% | +4.23% |
| next-root | 320M | 463.0 | 366.8 | -20.79% | +2.68% | +6.79% |
| storybook | 320M | 482.0 | 388.1 | -19.49% | +4.00% | +10.52% |
| nuxt | 320M | 476.8 | 402.5 | -15.59% | +6.80% | +19.27% |
| playwright | 256M | 442.9 | 339.6 | -23.32% | +20.77% | +25.10% |
| drizzle-orm | 512M | 730.5 | 747.7 | +2.34% | -4.29% | +16.97% |

Zero clears the 5% memory bar on 13 of 17 projects, with reductions from 15.36% to 26.77% and a median reduction of
19.36% across the full matrix. Compiler, Compiler-Unions and mikro-orm are neutral: the first two are only 78-83 MiB,
and mikro-orm recorded no retirements. Every project's diagnostic locations matched between variants.

Drizzle is the important limit. The old floor retired no checker; zero retired 9-11. A seven-run old-then-zero block
was +4.89% RSS, and the reverse-order block was -0.16%; combining the 14 runs was +2.34% RSS and +16.97%
instructions. The 512 MiB target is below the workload's non-retirable front end plus fresh-checker footprint, so
rebuilding cannot reach it and provides no reproducible RSS reduction. Parallel wall time was noisy enough to show a
4.29% speedup despite the instruction increase; it is not evidence of a CPU win. This is why `--maxMemory` remains a
target rather than a hard bound, and why callers whose target is below the workload's attainable floor may still
prefer `TSRS_RETIRE_MIN=128M`.

## Target and minimum sweeps

Three-run target sweep with the old 128 MiB minimum:

| target | wall | instructions | peak RSS |
| --- | ---: | ---: | ---: |
| 3.5 GiB | 6.867 s | 301.868 B | 3.5680 GiB |
| 3 GiB | 6.763 s | 316.516 B | 3.3689 GiB |
| 2.5 GiB | 6.776 s | 317.907 B | 3.4141 GiB |
| 2 GiB | 6.752 s | 316.950 B | 3.4747 GiB |
| 1.5 GiB | 6.736 s | 317.023 B | 3.4815 GiB |

The flat 2.5-1.5 GiB results identify the old minimum, rather than the requested target, as the limiter.

Two-run minimum sweep at a 3 GiB target (one warmup):

| minimum | wall | instructions | peak RSS |
| --- | ---: | ---: | ---: |
| 128 MiB | 6.730 s | 315.659 B | 3.3546 GiB |
| 96 MiB | 6.887 s | 336.666 B | 3.1089 GiB |
| 64 MiB | 6.957 s | 338.639 B | 3.0641 GiB |
| 32 MiB | 6.853 s | 334.887 B | 3.0943 GiB |
| zero | 6.886 s | 334.919 B | 3.0390 GiB |

No intermediate minimum saved work consistently; zero had the lowest and least variable peak.

With a zero minimum, a coarse target sweep found 3.0594 / 2.6031 / 2.2134 / 2.3558 GiB at targets of 3 / 2.5 / 2 /
1.5 GiB. A second sweep in 128 MiB steps bracketed the minimum: targets of 1.75 / 1.875 / 2 / 2.125 / 2.25 GiB
peaked at 2.3800 / 2.2327 / 2.2251 / 2.2905 / 2.3400 GiB. The repeated five-run 2 GiB result above was 2.1912 GiB.

## Other earlier-freeing options

These were measured and rejected before changing the retirement floor:

- **Finish checker regions when their queues empty:** -0.03% peak RSS. Work stealing made every queue finish within
  about 70 ms, after the peak (`notes/mem-finish-checkers.md`).
- **Relation cache thresholds:** at `--maxMemory 4G`, never / 2M / 1M / 500k / 250k entries all peaked between
  4.0327 and 4.0409 GiB. With an unreachable 100 GiB target (no checker retirement), the best threshold was 500k at
  4.6499 GiB versus 4.6781 GiB with no clearing (-0.60%). Clearing after every file gave 4.6260 GiB (-1.12%) but
  raised instructions 4.15%. Selectively clearing a subset cannot improve the memory upper bound of clearing all.
- **Broader leaf-file regions:** uncapped predicted placement peaked at 4.5537 GiB, versus 4.8064 GiB with regions
  off, 4.8227 GiB with regions kept, and 4.5950 GiB with every file in a region. The prediction covered 99.5% of exact
  leaf nodes, so broader patterns have less than 0.5% leaf-node headroom while paying region overhead for non-leaves.
- **Free a leaf at the earliest diagnostic boundary:** a prototype performed include-directive processing first and
  freed the region before the no-emit diagnostic vector filter. Five runs peaked at 4.5569 GiB, versus the 4.5537 GiB
  baseline. The filter interval is too short to overlap meaningful live leaf memory. Reverted.
