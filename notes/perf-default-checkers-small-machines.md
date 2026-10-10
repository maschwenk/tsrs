# perf-default-checkers-small-machines: every core gets a checker on machines of up to 8 cores

Without `--checkers`, tsrs chose `clamp(cores / 2, 4, 32)` checker threads (notes/perf-checker-64.md section 3 raised
the cap from 8 to 32 and kept the halving). On an 8-core machine that is Go's 4. The README bench runs every project on
an 8-vCPU runner in three modes, and its `--checkers 8` rows have been faster than its default rows on every project
but one since the project list grew. This note takes the numbers from one results file and changes the default to
`clamp(max(cores / 2, min(cores, 8)), 4, 32)`: every core up to 8, half the cores above that. Machines with 9-16 cores
keep 8 (half their cores rounds below it), 18 cores keep 9, 64 or more keep 32, so every number measured on the
18-core Mac and on the 64-vCPU runner is unchanged, including the README's headline table and the `bun check`
comparison. The small-program floor (one checker per 32 type-checked files beyond Go's 4) and the `-b` build-mode
default (Go's 4) are unchanged.

Superseded on 2026-10-10 by notes/perf-default-checkers-16.md: every core up to 16, with a default `--maxMemory`
target on machines short of memory.

## Evidence: the 8-vCPU runner, default (4 checkers) against `--checkers 8`

bench/results/2026-10-07-313e834a19e0.md (Depot `depot-ubuntu-24.04-8`, 8 vCPU / 32 GB, PGO dist build with BOLT,
median of 3; the default rows are the "Default mode" table, the 8-checker rows the "`--checkers 8`" table of that
file; the projects added for the Bun comparison have no default row in that file, only the 8-checker one):

| project | 4 checkers wall s | 8 checkers wall s | wall | 4 checkers peak | 8 checkers peak | peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 2.73 | 1.61 | -41% | 1.78 GiB | 1.93 GiB | +8% |
| xstate-main | 0.20 | 0.13 | -35% | 247 MiB | 291 MiB | +18% |
| webpack | 0.35 | 0.23 | -34% | 393 MiB | 446 MiB | +13% |
| mui-docs | 1.42 | 1.22 | -14% | 1.04 GiB | 1.31 GiB | +26% |
| Compiler | 0.07 | 0.07 | 0% | 96 MiB | 116 MiB | +21% |
| Compiler-Unions | 0.13 | 0.12 | -8% | 101 MiB | 122 MiB | +21% |
| cal-diy | 1.23 | 0.97 | -21% | 1.17 GiB | 1.50 GiB | +28% |
| formbricks-web | 1.41 | 0.96 | -32% | 1.61 GiB | 1.87 GiB | +16% |
| supabase-studio | 1.59 | 1.07 | -33% | 1.22 GiB | 1.43 GiB | +17% |
| t3code-server | 1.93 | 2.05 | +6% | 1.20 GiB | 1.80 GiB | +50% |
| mikro-orm | 2.16 | 1.26 | -42% | 1.37 GiB | 1.60 GiB | +17% |

Nine of eleven projects get 14-42% shorter; the two that do not are the smallest (Compiler, 0.07 s either way) and
t3code-server, whose Effect declarations every checker re-resolves on first use (notes/perf-heavy-files.md), so four
more checkers cost it 6% of wall and half again its memory. Peak memory grows 8-28% elsewhere, from 0.13-0.32x of
tsgo's to 0.10-0.33x (tsgo at its default of 4 checkers). The earlier results files show the same ordering on every
publish (bench/results/2026-10-07-*.md), and the regression flag of bench.yml compares single-threaded instructions,
which this does not change.

The halving came from the 38k-file codebase, where a checker costs ~0.4 GiB (notes/perf-checker-scaling.md), and from
the parse pool sharing the cores; on an 8-core machine the parse phase is over before the checkers start, and eight
checkers on vscode cost 0.15 GiB. Nothing above 8 cores was re-measured here: 16 cores keep 8 checkers because half of
16 is the same count, and the 18-core and 64-vCPU rules are the measured ones of the earlier notes.

## What changes for users

An 8-core CI runner or laptop now type-checks vscode-sized programs 30-40% faster at 8-28% more memory. `--checkers 4`
restores the old count. `-b` keeps Go's 4 per project. The bench's `--checkers 8` mode now equals its default mode on
the 8-vCPU runner; it stays as the row that pins the count.

## Gates

- `cargo test -p tsrs_compiler default_checker_tests`: the rule's table (1-128 cores).
- The rule is read once per program; diagnostics do not depend on the checker count (notes/perf-order-independence.md,
  pr-verify's 1/4/16/32-checker cells).
- bench/run.py `tsrs_default_checkers` mirrors the rule for the results tables; bench/compare.py and bench/README.md
  describe it.
