# Lookup/insert cleanup (PR 338)

The original `codex/lookup-insert-once` change replaced repeated map searches throughout the repository and added
three combined lookup/insert APIs to `SymbolTable`. Its public-project measurements increased single-threaded
instructions on 11 of 17 projects. This follow-up rebases the refactor onto `d8b0959e` and investigates the
regressions; it does not change the compiler algorithms or optimization settings.
The final rebase is onto `94703321`; the intervening changes only update benchmark publishing and reports, with
no compiler-source changes.

## Original result

The [original Linux comparison](https://depot.dev/orgs/qw3zz3l8cs/workflows/5sb887p6mr) used the same release build
settings and two single-threaded instruction runs per binary. The [PR comparison](https://github.com/oxc-project/tsrs/pull/338#issuecomment-6111005799)
independently reported the same direction and size of changes. Examples:

| Public project | Base instructions | Original instructions | Change |
| --- | ---: | ---: | ---: |
| vscode | 85,364,489,969.5 | 85,395,834,870.5 | +0.0367% |
| mui-docs | 34,196,971,752 | 34,293,222,130 | +0.2815% |
| mikro-orm | 60,263,016,944 | 60,375,558,938 | +0.1868% |
| Compiler-Unions | 3,620,712,745.5 | 3,610,074,401 | -0.2938% |

The mui-docs base repetitions differed by 680 instructions, and the original repetitions by 224; the roughly
96-million-instruction increase is not run-to-run noise. An [isolated RSS rerun](https://depot.dev/orgs/qw3zz3l8cs/workflows/lk1zc2vtqd)
measured vscode default peak RSS at 2143.0 -> 2138.0 MiB. None of these results establishes a qualifying performance
win.

## Candidate and attribution

Restore the existing `SymbolMap::insert` implementation, the inline `SymbolTable` lookup path, and binder/checker
symbol creation. Remove `insert_if_absent`, `get_or_insert_new_symbol`, `get_or_insert_new_symbol_copy_name`, and
`Checker::record_new_symbol`. Retain the ordinary map-entry refactors and reuse the result of the two identical
member lookups in spread construction.

The saved macOS release binary supplies a concrete reason to investigate the symbol-table changes: the combined
helpers remain out of line, `insert_if_absent` saves registers in a 64-byte stack frame, and ordinary insertion
transfers from `SymbolMap::insert` into the extracted `insert_vacant`. A hit saved no second search in the original
lookup-then-insert callers. This assembly observation is not an attribution of the Linux regression by itself;
the grouped reversion is measured against both main and the rebased original.

## Cleanup result

The three-way Linux comparison on a 16-vCPU runner uses main `d8b0959e` and the same release settings for all
three builds. The cleanup retires fewer instructions than main on all 17 projects, by 0.021-0.312%; the rebased
original still regresses 11 projects. All diagnostic and exit-code comparisons match in both measured modes.
These are corrections to the requested refactor, below the threshold for a standalone performance optimization.
No wall-time or memory improvement is claimed.

| Public project | Main instructions | Original instructions | Cleanup instructions | Cleanup vs main | Default peak RSS MiB, main -> cleanup |
| --- | ---: | ---: | ---: | ---: | ---: |
| vscode | 83,964,724,642.0 | 83,992,206,384.5 | 83,917,696,475.5 | -0.0560% | 2139.8 -> 2137.2 |
| xstate-main | 5,546,984,470.0 | 5,549,172,511.0 | 5,544,306,072.0 | -0.0483% | 283.3 -> 284.2 |
| webpack | 11,048,182,861.5 | 11,056,720,141.5 | 11,041,895,007.5 | -0.0569% | 538.2 -> 537.9 |
| mui-docs | 33,201,183,663.5 | 33,285,294,572.0 | 33,177,576,845.5 | -0.0711% | 1637.9 -> 1641.3 |
| Compiler | 1,877,360,365.5 | 1,876,262,067.5 | 1,876,830,729.0 | -0.0282% | 100.0 -> 102.5 |
| Compiler-Unions | 3,547,512,972.0 | 3,535,817,592.5 | 3,536,461,528.0 | -0.3115% | 104.5 -> 104.2 |
| cal-diy | 29,007,989,540.0 | 29,030,047,116.5 | 28,998,063,271.0 | -0.0342% | 1937.4 -> 1942.1 |
| formbricks-web | 37,552,932,581.0 | 37,545,210,611.0 | 37,534,505,539.0 | -0.0491% | 2028.2 -> 2031.5 |
| supabase-studio | 40,544,862,107.5 | 40,569,286,110.5 | 40,531,668,017.5 | -0.0325% | 1731.4 -> 1729.8 |
| t3code-server | 40,415,439,261.0 | 40,444,262,162.5 | 40,396,408,217.0 | -0.0471% | 2147.3 -> 2145.4 |
| mikro-orm | 59,397,632,247.0 | 59,501,079,678.5 | 59,385,091,533.0 | -0.0211% | 2028.6 -> 2008.3 |
| next-packages-next | 16,835,152,309.0 | 16,849,331,471.0 | 16,824,737,279.0 | -0.0619% | 695.9 -> 695.7 |
| next-root | 12,400,367,782.5 | 12,404,281,765.0 | 12,389,855,940.0 | -0.0848% | 484.3 -> 485.0 |
| storybook | 9,172,839,785.5 | 9,170,558,180.5 | 9,161,856,727.0 | -0.1197% | 519.7 -> 518.8 |
| nuxt | 8,534,816,226.5 | 8,530,984,460.5 | 8,531,670,254.0 | -0.0369% | 516.6 -> 523.1 |
| playwright | 7,478,433,970.0 | 7,481,272,664.5 | 7,474,676,545.0 | -0.0502% | 500.1 -> 491.8 |
| drizzle-orm | 11,734,878,959.0 | 11,730,660,264.0 | 11,725,092,210.5 | -0.0834% | 808.0 -> 809.9 |

The grouped reversion establishes the net cost of the symbol-table/API changes in this patch; it does not
attribute a separate percentage to each helper or to the symbol-allocation refactor. Keep the existing symbol
lookup and insertion paths. Revisit combined symbol APIs only with an isolated, measured implementation that
preserves their hit-path cost.

## Allocation check

The historical comparison built main and cleanup in the same directory on macOS arm64 with allocation
instrumentation. It ran the pinned public Compiler and Compiler-Unions configurations with
`RAYON_NUM_THREADS=1 --singleThreaded --noEmit --incremental false --pretty false`. Diagnostics are byte-identical.
The allocation counters and rounded arena/heap sizes are unchanged:

| Public project | Heap allocations, main / cleanup | Arena requested, main / cleanup | Peak live heap, main / cleanup |
| --- | ---: | ---: | ---: |
| Compiler | 336,626 / 336,626 | 33.0 / 33.0 MB | 10.0 / 10.0 MB |
| Compiler-Unions | 374,986 / 374,986 | 31.9 / 31.9 MB | 12.8 / 12.8 MB |

## Validation

The Linux run compares the full conformance pass lists against main in Go history, canonical history with
parallel checking, and `TSRS_LAZY_MEMBERS=0`. All nine pass lists are identical: 13,458 diagnostic passes in each
mode; 12,779 type and symbol passes in Go/eager mode and 12,778 in canonical mode, with its existing expected
difference. There are no crashes or timeouts. All 29 regression cases and both content-mapper cases pass.

Local checking passes `cargo check --workspace --locked`, the `wasm32-wasip1` check, `tools/lint/ratchet.py`
(three baseline findings, none new), and `tools/lint/source.py`. The selected unit suites pass 515 cases with two
ignored; the two real FSEvents checks and one subprocess check require running outside the sandbox. The subprocess
failure was reproduced on main before the unrestricted rerun passed. Fourslash meets its existing gate: 4,066
passes, 63 expected unsupported cases, and 417 skips.

## Reproduction

Build main `d8b0959e`, the rebased original, and the cleanup in the same source and target directories, using the
pinned Rust 1.99.0 toolchain and `cargo build --release --locked -p tsrs_cli`. Apply each candidate independently
to main. Use all 17 pinned `bench/projects.json` projects.

Measure two interleaved runs per binary with `bench/count.py`, `--singleThreaded --noEmit --incremental false
--pretty false`, and `RAYON_NUM_THREADS=1`. Measure default peak RSS separately through a fresh GNU `time` process
(`%M`, KiB), without a checker override: one warmup and three retained interleaved runs. This avoids inheriting a
large Python driver's resident-memory floor in `wait4` results. Compare ordered diagnostics and exit codes within
each mode. The [cleanup run](https://depot.dev/orgs/qw3zz3l8cs/workflows/gctsj96kqd) retains patches, input hashes,
binary hashes, raw logs, and measurements in its `lookup-cleanup` artifact.
