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
depot ci dispatch --repo oxc-project/tsrs --workflow pr-verify.yml --ref idiomatic-unsigned-integers \
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

## Follow-up: verified internal flags, IDs and counts

The owner requested a further audit and selected the verified internal values, leaving Unicode and indentation
APIs for a separate review. Source revision `a8fa8de5` makes these replacements relative to `6720c98d`:

- Async-context flags, for-await hierarchy flags, formatting rule actions and semantic-token modifiers use `u32`.
  Their masks and bitwise operations are unchanged, including the full-width complement of formatting actions.
- Semantic-token type IDs, snippet tab-stop IDs and keyword-completion cache keys use `u32`, preserving their
  four-byte storage and existing numeric values. The cache structure and serialized data are unchanged.
- The array-slice and parameter-decorator factory helpers accept `usize`. Each has one caller, passing a
  collection index; the narrowing `as i32` casts are removed.
- Incremental nested-emit depth uses `usize`; its completion closure decrements once for each increment.
  Decorator lexical depth uses `u32`, retaining the compact field; it is decremented only when positive.

Signed indentation differences, Go rune helpers (including their invalid-input behavior), JavaScript `ToInt32`,
protocol/FFI integers, diagnostic codes and the exhausted relation budget remain signed. Enum representations
are outside this pass. These are domain corrections requested by the owner, not a claimed speedup.

Before/after layout checks on macOS arm64 using the compiled release libraries are identical:

| type | before, bytes | after, bytes |
| --- | ---: | ---: |
| `SnippetElement` | 8 | 8 |
| `Option<SnippetElement>` | 8 | 8 |
| Decorator `lexicalEntry` | 56 | 56 |
| Incremental `Program` | 160 | 160 |

Local validation:

- Workspace check and 91 checks in the affected crates pass; lint ratchet and source checks pass without baseline changes.
- Both conformance histories retain every previously passing case: 13,458 / 12,779 / 12,779 in reference history;
  13,458 / 12,778 / 12,778 in default history, with no crashes or timeouts.
- Fourslash retains 4,066 passes, 63 existing failures and 417 skips.
- The emit oracle compares 978 async, await, decorator and destructuring cases against pinned Go: 1,497 variants
  with emitted output match, and 75 variants emit nothing. Before/after reports are identical.
- All 31 steps across the five incremental fixture sequences match pinned Go and the previous PR binary.
  The local harness uses a scratch copy adapted for macOS's Bash/sed and canonical temporary paths; the repository
  harness is unchanged.

### Follow-up measurements

Depot run `zlb2zr9w6l` compares `6720c98d` with `a8fa8de5` on the same 32-vCPU Linux x86-64 runner,
using the workflow and measurement settings above. All 102 diagnostic and exit-code cells match. Every project's
instruction delta rounds to 0.000%; the largest absolute delta is 0.000042%, below the documented 0.001%
repeatability of the counter. The mixed-sign differences are measurement-neutral, not evidence of a speedup.

| project | instructions before | instructions after | delta | RSS 1 thread, MiB: before → after | RSS 16 checkers, MiB: before → after |
| --- | ---: | ---: | ---: | ---: | ---: |
| vscode | 89,048,037,150 | 89,048,017,408 | -0.000022% | 1606.87 → 1606.60 | 2183.80 → 2179.99 |
| xstate-main | 6,037,238,585 | 6,037,238,014 | -0.000009% | 182.75 → 182.81 | 377.79 → 376.39 |
| webpack | 11,993,633,402 | 11,993,633,079 | -0.000003% | 312.25 → 312.71 | 567.69 → 567.00 |
| mui-docs | 37,945,173,114 | 37,945,172,488 | -0.000002% | 741.47 → 741.84 | 1651.69 → 1662.90 |
| Compiler | 1,965,231,643 | 1,965,231,819 | +0.000009% | 59.71 → 60.01 | 144.45 → 146.70 |
| Compiler-Unions | 3,748,884,054 | 3,748,883,926 | -0.000003% | 63.96 → 63.69 | 150.21 → 146.87 |
| cal-diy | 32,392,707,114 | 32,392,703,906 | -0.000010% | 775.07 → 774.90 | 1957.63 → 1947.42 |
| formbricks-web | 42,073,719,636 | 42,073,718,974 | -0.000002% | 1101.93 → 1102.44 | 2049.64 → 2052.12 |
| supabase-studio | 44,123,504,851 | 44,123,503,556 | -0.000003% | 873.46 → 873.41 | 1768.79 → 1773.12 |
| t3code-server | 43,392,481,494 | 43,392,494,888 | +0.000031% | 738.43 → 738.56 | 2249.11 → 2244.96 |
| mikro-orm | 65,890,074,922 | 65,890,087,715 | +0.000019% | 1133.77 → 1132.95 | 2070.90 → 2071.31 |
| next-packages-next | 18,552,812,429 | 18,552,813,057 | +0.000003% | 445.95 → 445.74 | 720.42 → 721.14 |
| next-root | 13,659,330,601 | 13,659,329,939 | -0.000005% | 315.36 → 315.39 | 536.05 → 536.05 |
| storybook | 10,268,921,410 | 10,268,925,714 | +0.000042% | 281.33 → 281.47 | 543.83 → 543.78 |
| nuxt | 9,331,579,002 | 9,331,577,980 | -0.000011% | 280.39 → 280.00 | 547.61 → 546.79 |
| playwright | 8,380,239,907 | 8,380,239,435 | -0.000006% | 244.16 → 244.21 | 536.05 → 536.05 |
| drizzle-orm | 12,640,914,954 | 12,640,914,582 | -0.000003% | 394.55 → 394.60 | 845.27 → 842.89 |

At the default 16 checkers, the median project RSS delta is -0.009% and the median wall delta is +0.464%.
Single-threaded RSS changes range from -0.415% to +0.510%. These three-repetition wall/RSS measurements and the
unchanged layouts do not establish a repeatable performance regression; no performance improvement is claimed.

## Follow-up: unsigned Rune and internal enum representations

At the owner's request, the next internal-domain audit makes `stringutil::Rune` a `u32`. Although Go defines `rune`
as `int32`, every producer in this port returns a decoded Unicode value, `RuneError`, a UTF-16 surrogate preserved
for JavaScript/WTF-8 handling, or another non-negative invalid code point. End-of-input is represented separately by
the decoded byte count or `Option`, never by a negative rune. The old generic `AsRune` conversion layer and its four
implementations are removed; Rune APIs take `u32` directly. The scanner decodes straight to `u32` instead of
decoding to `char` and converting back, and the regexp flag lookup no longer narrows through `u8::try_from`.
Positive invalid code points, surrogates and `RuneError` retain their previous behavior.

Purely internal, non-negative state enums now use `repr(u32)` while retaining their four-byte layouts. Signed
compiler/watch configuration and script kinds, diagnostics, process exit status, indentation and source-map deltas,
JavaScript `ToInt32`, protocol/FFI integers and the relation budget remain signed. The LSP protocol crates, schemas,
generated types and generator are unchanged; language-service edits are mechanical Rune call-site adaptations only.

### Local measurement

A local macOS arm64 screen compares `f970b59a` with the working tree. Both release binaries use fat LTO, one codegen
unit and the same profiling-enabled release settings. Six interleaved single-threaded vscode runs use the configured
benchmark checkout and `/usr/bin/time -l`; diagnostics without extended statistics are byte-identical.

The macOS retired-instruction counter was bimodal for both binaries, so the unstratified medians are not a valid
comparison. The baseline's low mode was 87,422,909,472 instructions (one sample) and its high mode ranged from
88,363,987,372 to 88,376,261,308 (five); the new binary's low mode ranged from 87,415,582,512 to 87,468,533,701
(four) and its high mode from 88,349,344,491 to 88,398,502,270 (two). The high-mode means differ by +0.0034%, which
is measurement-neutral. Peak RSS has the same 1,677,115,392 to 1,677,148,160-byte range before and after. This does
not establish a speedup or a regression; a Linux `bench/count.py` run is still the deterministic acceptance measure.

Local validation: workspace and WebAssembly checks pass; 297 affected-crate tests pass (one ignored negative Miri
control); all 29 regression fixtures pass; conformance retains 13,458 diagnostic, 12,779 types and 12,779 symbols
passes with no crashes or timeouts; fourslash retains 4,066 passes, 63 existing failures and 417 skips. All generators
reproduce the working diff, and the lint ratchet and source checks pass without a baseline increase.
