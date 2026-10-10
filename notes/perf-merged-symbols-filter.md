# perf-merged-symbols-filter: a negative filter in front of `merged_symbols`

The probe in notes/perf-tsgo-can1357-speedups.md counted 32M `get_merged_symbol` calls on vscode and 12M on
t3code-server, each an Fx hash and a hashbrown probe of `Checker::merged_symbols`, and left it for a probe of its own.
This is that probe. Result: a per-checker Bloom filter in front of the map removes 0.13-0.62% of single-threaded
instructions across the 17 bench projects, and a perfect filter could not reach the 1% bar either. Not adopted
(AGENTS.md: under 1%, and it adds an invariant). Base: origin/main 83e19356.

## Census

A temporary build (env-gated counters in `get_merged_symbol` and `record_merged_symbol`, not committed), Mac,
`--singleThreaded`, one checker, whole run:

| project | calls | found in the map | merged symbols at the end |
| --- | ---: | ---: | ---: |
| vscode | 32.37M | 308,055 (0.95%) | 2,479 |
| t3code-server | 12.26M | 153,581 (1.25%) | 813 |
| webpack | 4.02M | 66,810 (1.66%) | 1,006 |
| xstate-main | 0.83M | 8,427 (1.02%) | 1,449 |
| cal-diy | 4.15M | 70,519 (1.70%) | 2,040 |
| formbricks-web | 6.45M | 184,159 (2.85%) | 2,621 |
| supabase-studio | 8.37M | 165,472 (1.98%) | 2,712 |

About 99% of calls ask about a symbol that was never merged, and the map holds a few thousand entries at most
(globals declared in several files, ambient modules, augmentations). Nothing is ever removed from it.

What a miss costs: the arm64 release build calls the out-of-line `get_merged_symbol` from 4 places and inlines it
everywhere else. The out-of-line copy runs 26 instructions on a miss (Fx multiply with a 4-instruction constant,
rotate, `h2`, group load, NEON compare, empty-byte test); with the filter in front (variant A below) it runs 14.

## Filter sizing

Share of the misses the filter lets through to the map (false positives), from the same build, which kept filters of
every size side by side. One hash: bit `(key * 0x9E3779B97F4A7C15) >> (64 - k)`. Two hashes: that bit and the next `k`
bits of the product. The last column is the filter that was benchmarked: one hash, `(key as u32) * 0x9E3779B1`, top 16
bits.

| project | 1 hash, 2^14 bits | 2^16 | 2^18 | 2 hashes, 2^14 | 2^16 | 2^18 | benchmarked (u32, 2^16) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 14.71% | 3.56% | 0.89% | 7.76% | 0.59% | 0.03% | 4.62% |
| t3code-server | 5.58% | 2.28% | 0.28% | 0.82% | 0.04% | 0.00% | 1.19% |
| webpack | 6.24% | 1.87% | 0.62% | 1.50% | 0.10% | 0.00% | 1.60% |
| xstate-main | 7.65% | 2.44% | 0.42% | 2.47% | 0.46% | 0.00% | 2.49% |
| cal-diy | 10.79% | 2.98% | 0.66% | 4.49% | 0.26% | 0.02% | 7.52% |
| formbricks-web | 15.12% | 3.91% | 0.97% | 7.34% | 0.85% | 0.03% | 4.03% |
| supabase-studio | 14.08% | 3.79% | 0.86% | 7.53% | 0.53% | 0.03% | 5.78% |

A false positive costs one map probe (about 25 instructions), so vscode's 1.5M with the benchmarked filter are about
0.04% of its instructions. That is why the benchmarked filter has one hash and 2^16 bits (8 KiB per checker, 72 KiB at
the 9 default checkers of an 18-core Mac, 256 KiB at 32); a second hash or 2^18 bits would recover a few hundredths of a
percent.

## Result

Two variants, each on its own branch, measured by `pr-verify` dispatched on the branch (Linux, Depot 32 vCPU,
`cargo build --release` of the branch and of main 83e19356, single-threaded instructions from `bench/count.py`, exact):

- A, `probe/merged-symbols-filter-16` (3a16dc4f): `MergedSymbolFilter`, a `Box<[u64; 1024]>` on `Checker`, set in
  `record_merged_symbol`, tested first in `get_merged_symbol`; 64-bit multiply.
- B, `probe/merged-symbols-filter-inline` (5a67fbe8): the same filter with a 32-bit multiply, `get_merged_symbol`
  `#[inline]` and the map probe in an `#[inline(never)]` helper, so call sites inline only the filter test (155 call
  sites of the helper in the arm64 build).

| project | main | A | B | peak RSS, 1 thread (main -> B) |
| --- | ---: | ---: | ---: | ---: |
| vscode | 99.609 G | -0.543% | **-0.609%** | 1.56 -> 1.56 GiB |
| t3code-server | 48.501 G | -0.581% | **-0.620%** | 735 -> 735 MiB |
| webpack | 13.397 G | -0.520% | -0.564% | 312 -> 312 MiB |
| Compiler | 2.230 G | -0.530% | -0.583% | 59 -> 59 MiB |
| next-packages-next | 22.355 G | -0.433% | -0.455% | 445 -> 445 MiB |
| next-root | 15.099 G | -0.395% | -0.428% | 318 -> 318 MiB |
| playwright | 9.639 G | -0.306% | -0.336% | 243 -> 243 MiB |
| supabase-studio | 54.796 G | -0.261% | -0.290% | 872 -> 872 MiB |
| storybook | 12.030 G | -0.238% | -0.262% | 280 -> 280 MiB |
| formbricks-web | 47.372 G | -0.224% | -0.250% | 1.07 -> 1.07 GiB |
| xstate-main | 6.869 G | -0.216% | -0.244% | 183 -> 183 MiB |
| Compiler-Unions | 5.211 G | -0.216% | -0.237% | 63 -> 63 MiB |
| drizzle-orm | 13.981 G | -0.199% | -0.232% | 391 -> 391 MiB |
| cal-diy | 37.181 G | -0.188% | -0.207% | 766 -> 766 MiB |
| nuxt | 10.764 G | -0.185% | -0.205% | 280 -> 280 MiB |
| mikro-orm | 79.230 G | -0.130% | -0.140% | 1.10 -> 1.10 GiB |
| mui-docs | 46.438 G | -0.126% | -0.139% | 740 -> 740 MiB |

B saves about 19 instructions per call on x86-64 (vscode 0.607 G over 32.37M calls; webpack, cal-diy, formbricks-web and
supabase-studio the same; t3code-server 25). Peak RSS did not move single-threaded. Default mode, vscode on the Mac (9
checkers, 3 interleaved runs, variant A): peak 1,987 -> 1,974 MiB (medians; the runs spread 1,966-1,995 MiB, so this is
noise, and the filter adds 72 KiB). Wall time was one run per binary and project in these jobs and moved between -15%
and +23% in both directions; it says nothing at this size. Mac instruction counts (`/usr/bin/time -l`) were no use
either: main alone spread 98.53-99.63 G over three vscode runs, wider than the effect.

**Why no filter clears 1%.** The filter test itself costs something on every call: load the box pointer, multiply,
shift, load the word, test the bit, branch (9 instructions in B's out-of-line arm64 copy, 2 of them the constant; an
estimated 6-8 on x86-64, where the multiply takes an immediate). Adding that back to B's saving puts main's whole miss
path at about 25-27 instructions on x86-64 (31-33 on t3code-server), so even a test that cost nothing would save about
0.8-0.9% on vscode and t3code-server, the two projects with the most calls per instruction. Any other representation (an
id-indexed table as in tsgo's 728ae2e5f, a "maybe merged" bit in `Symbol`) still pays at least a load and a test per
call, so none clears the bar on the current bench.

## Exactness

The filter is exact by construction: a clear bit means `record_merged_symbol` never saw the symbol, so the map would
miss too; a set bit asks the map. `record_merged_symbol` is the only writer of `merged_symbols`. Checked anyway:

- pr-verify: identical diagnostics and exit codes in 34 of 34 cells for each variant (17 projects, one checker and the
  single-threaded count run).
- Mac, variant A against main: `--pretty false` output and the `--extendedDiagnostics` counters identical on vscode,
  t3code-server, webpack, xstate-main and cal-diy single-threaded (3 runs each); vscode diagnostics identical in the
  default mode (3 runs).
- `tools/regressions.sh` 28/28 (A and B).
- Conformance gate, B: 13,458 / 12,779 / 12,779 (errors / `.types` / `.symbols`) in Go history and 13,458 / 12,778 /
  12,778 in the default mode (`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`); the pass lists are
  identical to main's in both modes.
- `tools/lint/ratchet.py` and `tools/lint/source.py` pass on A.

## When to revisit

B saves about 19 instructions per call, so it clears 1% on a project that makes more than about 530 `get_merged_symbol`
calls per million instructions; vscode makes 325 and t3code-server 253. If a project like that joins the bench, or
merges become more common, branch B is the change (about 30 lines).
