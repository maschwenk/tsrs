# Removing the unsigned-integer refactor's instruction regression

PR #306 converts source offsets and collection indices to unsigned types and gives synthetic positions an explicit
variant. Its first correctness-clean verification increased single-threaded instructions on every configured
project, by 0.086% to 1.002%. The owner requested removing every instruction increase, including changes below the
usual standalone performance threshold.

## Costs and corrections

- `TextPos` derived equality and ordering compare the enum discriminants before the source offsets. Its compact
  encoding already orders synthetic zero before source `offset + 1`. Equality, ordering and hashing now use that
  key directly, with safe pattern matching. An isolated optimized Linux x86-64 comparison of two source positions
  goes from 18 instructions on its normal path to 3 (`cmp`, `setb`, `ret`). This is a code-generation
  observation, not an attribution of the whole-project improvement to one operation.
- `expect_source`'s formatted panic set up formatting data before checking the variant. A cold helper keeps that
  setup on the synthetic-node failure path.
- `SourcePos::new` now checks the result of encoding through `NonZeroU32::new(offset.wrapping_add(1))`. The encoder
  and the unsupported-offset check share an increment and its zero flag, removing the separate comparison. It
  rejects the same `u32::MAX` offset, remains const, and also removes the constructor's unsafe operation.
- `TextRange::len` subtracts the encoded source positions directly: the common +1 cancels. It still returns `None`
  for either synthetic endpoint and rejects reversed source ranges, without first decrementing both operands.
- The relation budget stays signed. Go decrements it below zero while unwinding and considers any value <= 0
  exhausted. Treating it as a collection length required saturating subtraction on every recorded relation.
  `Cell<isize>` preserves its signed domain and permits ordinary decrements. Its storage size matches the PR's
  `Cell<usize>`.

The position and range layouts remain four and eight bytes. The changes keep the pinned Go algorithms and their
order of operations. Boundary coverage includes synthetic/source comparisons, offsets across `i32::MAX`, the
largest supported offset, mixed synthetic ranges and the two rejected inputs.

## Measurement

All comparisons use `.depot/workflows/pr-verify.yml`: release builds with fat LTO and one codegen unit, on the same
Linux x86-64 Depot 32-vCPU runner per comparison. Instructions come from `bench/count.py`, user space only, with
`--singleThreaded` and `RAYON_NUM_THREADS=1`. Peak RSS is recorded with the same run. Timed comparisons use three
interleaved repetitions at 1, 4, 16 and 32 checkers, plus the poisoned-arena diagnostic check.

The initial report is run `0qlzms6kgm`, `main@b3eb31f4cca0` against PR merge `f35be143aa24`. The first correction was
also screened in direct branch run `j5d45jdwvz`, against merge base `75e48e9cab54`. Those baselines differ: new numeric
conversion and compiler changes landed on main during this work. The final comparison includes those upstream
changes on both sides. Results from different comparisons are not an ablation of individual corrections.

Reproduce the final PR comparison with:

```sh
depot ci dispatch --repo maschwenk/tsrs --workflow pr-verify.yml --ref idiomatic-unsigned-integers \
  --input projects=all --input checkers=1,4,16,32 --input reps=3
```

## Final result

Run `rkp0vspqx9`: `main@7494a6a58f01` against PR merge `8a9bbbe735f0` (source head `2afe0f8d`). All 17 instruction deltas are negative; diagnostics and exit codes match in all 102 cells.

| project | instructions main, G | instructions PR, G | delta | RSS 1 thread, MiB: main → PR | RSS 16 checkers, MiB: main → PR |
| --- | ---: | ---: | ---: | ---: | ---: |
| vscode | 89.065486 | 89.048011 | -0.020% | 1600.20 → 1607.50 | 2168.38 → 2179.47 |
| xstate-main | 6.058917 | 6.037238 | -0.358% | 182.87 → 182.71 | 375.09 → 376.09 |
| webpack | 12.001877 | 11.993634 | -0.069% | 312.55 → 312.53 | 556.20 → 566.14 |
| mui-docs | 37.965007 | 37.945172 | -0.052% | 736.83 → 741.68 | 1664.70 → 1655.04 |
| Compiler | 1.965596 | 1.965231 | -0.019% | 59.41 → 59.71 | 143.05 → 146.14 |
| Compiler-Unions | 3.750360 | 3.748883 | -0.039% | 63.44 → 63.77 | 145.36 → 146.88 |
| cal-diy | 32.445000 | 32.392702 | -0.161% | 777.75 → 775.12 | 1948.95 → 1970.62 |
| formbricks-web | 42.162272 | 42.073734 | -0.210% | 1100.01 → 1101.47 | 2049.00 → 2055.28 |
| supabase-studio | 44.185349 | 44.123503 | -0.140% | 871.68 → 873.22 | 1767.09 → 1766.77 |
| t3code-server | 43.422703 | 43.392495 | -0.070% | 736.17 → 738.60 | 2225.08 → 2212.65 |
| mikro-orm | 66.105364 | 65.890088 | -0.326% | 1132.60 → 1133.46 | 2065.88 → 2059.88 |
| next-packages-next | 18.582518 | 18.552812 | -0.160% | 446.70 → 445.94 | 713.73 → 723.35 |
| next-root | 13.684883 | 13.659330 | -0.187% | 315.63 → 315.39 | 535.58 → 535.58 |
| storybook | 10.299121 | 10.268920 | -0.293% | 282.64 → 281.50 | 545.71 → 544.77 |
| nuxt | 9.357905 | 9.331577 | -0.281% | 280.35 → 280.18 | 551.02 → 555.05 |
| playwright | 8.399641 | 8.380239 | -0.231% | 244.23 → 244.28 | 535.58 → 535.58 |
| drizzle-orm | 12.654754 | 12.640914 | -0.109% | 392.54 → 394.08 | 840.80 → 843.73 |

Sixteen checkers is the default on this 32-vCPU runner. The largest timed-cell RSS change is Compiler at 16 checkers: 143.05 → 146.14 MiB (+2.16%). Wall medians across checker counts range from -3.9% to +2.1%; these three-repetition timings do not establish a wall-time speedup. The deterministic instruction comparison is the acceptance measure for the requested regression correction.

## Validation

- 473 unit and integration checks pass.
- Full conformance: 13,458 diagnostic passes, 12,779 types passes, 12,779 symbols passes; no crashes, timeouts or lost passing cases.
- Default checker history: 13,458 / 12,778 / 12,778, with the existing object-literal normalization difference; no lost passing cases.
- Fourslash gate: 4,066 passes, the existing 63 unsupported cases and 417 skips.
- Regression fixtures, generated code, Node API parity, macOS packaging and arena safety pass.
- Lint ratchet and source checks pass without baseline increases.
- WebAssembly differential gates pass; all 13 PR checks are green on the measured source revision.
