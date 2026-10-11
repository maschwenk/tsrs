# mem-default-checkers-8: restore the eight-checker small-machine ceiling

Status (2026-10-12): the default machine rule is again
`clamp(max(cores / 2, min(cores, 8)), 4, 32)`. This supersedes the default decision in
`notes/perf-default-checkers-16.md`; explicit `--checkers 16` remains available.

## Decision

The default briefly gave every core a checker through 16 cores. That was deliberately speed-first: on the
16-vCPU Linux runner, going from 8 to 16 cut wall time 8-43% across the large projects while adding 8-35% peak RSS
(`notes/perf-default-checkers-16.md`). The default is now memory-balanced instead. Checker state and commonly used
library types are duplicated per checker, and the higher default made tsrs use more memory than `bun check` on most
of the README's 16-vCPU rows. The owner asked to improve ordinary uncapped runs after the earlier-freeing experiments
could not move their peak (`notes/mem-retire-minimum.md`). That changed priority is why the old 8-checker rule is being
revisited.

The new rule gives every core a checker through 8 cores, then half the cores, from 4 to 32. The existing program-size
cap still permits only one checker beyond four per 32 type-checked files, so small programs do not gain checkers.
Build mode remains at four. On a 16-vCPU machine the default is 8 rather than 16; on 18 it is 9 rather than 16; on
32 and 64 it stays 16 and 32. `--checkers N` is unchanged for users who prefer the speed-first point.

## All public benchmark projects: 12 checkers against 8

Apple Silicon macOS, 12 available CPUs, the pull request's release build with fat LTO; same binary and inputs for both
counts. Each command was `tsrs -p <project> --noEmit --incremental false --extendedDiagnostics --pretty false`.
`hyperfine 2.0.0` ran one warmup and three measured executions. The table uses the three-run arithmetic mean;
`/usr/bin/time -l` recorded child peak RSS and instructions retired. Diagnostic text matched between the two counts
on all 17 projects after excluding the count-dependent extended-diagnostic counters.

The previous default selected 12 checkers on the 14 rows below. The first command pins that count with
`--checkers 12`; the second is the implemented no-flag default, which reported eight checker groups. These are the
direct before/after results:

| project | 12 RSS MiB | 8 RSS MiB | RSS | 12 wall s | 8 wall s | wall | 12 instr G | 8 instr G | instructions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 2,097 | 1,992 | -5.02% | 1.965 | 2.046 | +4.10% | 95.35 | 92.30 | -3.20% |
| webpack | 468 | 428 | -8.54% | 0.176 | 0.187 | +6.38% | 14.56 | 13.79 | -5.28% |
| mui-docs | 1,502 | 1,325 | -11.80% | 2.585 | 2.559 | -0.98% | 90.19 | 79.82 | -11.50% |
| cal-diy | 1,684 | 1,443 | -14.28% | 1.652 | 1.661 | +0.56% | 68.41 | 60.06 | -12.20% |
| formbricks-web | 1,859 | 1,703 | -8.36% | 1.781 | 1.738 | -2.41% | 67.05 | 61.34 | -8.51% |
| supabase-studio | 1,587 | 1,396 | -12.02% | 2.139 | 2.153 | +0.66% | 73.93 | 66.47 | -10.09% |
| t3code-server | 1,967 | 1,550 | -21.17% | 1.397 | 1.361 | -2.57% | 116.69 | 88.65 | -24.03% |
| mikro-orm | 1,872 | 1,646 | -12.04% | 1.000 | 0.976 | -2.43% | 84.05 | 75.29 | -10.42% |
| next-packages-next | 630 | 584 | -7.39% | 0.463 | 0.462 | -0.19% | 21.63 | 20.51 | -5.19% |
| next-root | 452 | 421 | -6.84% | 0.653 | 0.652 | -0.20% | 19.20 | 18.28 | -4.81% |
| storybook | 462 | 429 | -7.00% | 0.481 | 0.469 | -2.38% | 16.41 | 15.56 | -5.22% |
| nuxt | 470 | 412 | -12.30% | 0.544 | 0.539 | -0.80% | 16.58 | 14.33 | -13.59% |
| playwright | 423 | 386 | -8.78% | 0.145 | 0.167 | +15.31% | 12.27 | 11.37 | -7.35% |
| drizzle-orm | 721 | 564 | -21.83% | 0.498 | 0.505 | +1.42% | 18.72 | 16.23 | -13.28% |

Across these 14 projects, peak RSS falls on every row: median -10.29%, mean -11.24%, range -21.83..-5.02%.
Instructions also fall on every row: median -9.30%, mean -9.62%, range -24.03..-3.20%. Wall time has median -0.20%
and mean +1.18%, with a -2.57..+15.31% range. Playwright's 22 ms is the only double-digit wall regression; vscode
and webpack are +4.10% and +6.38%. The earlier 16-vCPU results show that the slowdown from 16 all the way to 8 is
larger on that machine. This is a default footprint decision, not a claim that fewer checkers always finish sooner.

All 17 projects were measured. The remaining three are not affected by the code change: the old and new defaults
both select 4 for Compiler, 4 for Compiler-Unions and 7 for xstate-main. Their explicit `--checkers 8` measurements
were intentionally excluded from the aggregate because that flag bypasses the default's program-size cap.

## Why not six

The initial same-binary screen also measured `--checkers 6`. On the 14 affected projects it saved a median 17.08%
peak RSS and 15.04% instructions against 12, but wall time worsened by a median 3.22% and a mean 6.44%. The tail is
too expensive for the default: vscode +18.3%, webpack +20.0%, next-packages-next +22.9%, mikro-orm +13.0%, and
playwright +10.2%.
Eight captures a qualifying memory reduction everywhere without that wall-time cliff.

## Gates

- `cargo test -p tsrs_compiler default_checker_tests`: machine-rule table from 1 through 128 cores.
- `tools/lint/ratchet.py` and `tools/lint/source.py`.
- Release `recycle_checkers` test for the `--maxMemory` change carried by the same branch.
- Actual-default release rerun on representative affected projects, in addition to the same-binary matrix above.
