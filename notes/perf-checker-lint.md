# Lint during semantic checking (2026-10-10)

Headless now always checks requested files, even with both reporting switches off or no supported rules.
Reporting switches select output only. The normal compiler opts in with `--lint <headless-config.json>`.
Build/incremental mode is rejected by that flag, since cached diagnostics do not replay lint rules.

## Design and prior work

Searched the performance notes and the rejected-techniques lists in `perf-round2-followups.md` and `docs/RUST.md`
before implementing this. There was no previous lint traversal experiment. This is an explicit behavior change:
headless must check and lint together, while preserving tsgolint's v2 protocol and rule behavior.

The compiler calls a per-program hook after each selected file's semantic/deferred checks, on the same checker,
before leaf AST reclamation. The checker keeps the file readable to type printers while the hook runs. A per-file
`Once` prevents duplicate rule execution on repeated semantic requests. Replacement programs do not inherit it.

The binder records expression statements only for files with configured rules. Binding can happen during parallel
loading, so a compiler-host wrapper enables collection immediately after parsing. Binding visits function
bodies before other statements; sorting candidates by start position and descending end restores source order.
The rule consumes this list instead of recursively walking the entire AST. This also covers unreachable code,
`with` bodies and deferred function/class bodies, which checker callbacks alone would miss or visit out of order.
No extra thread is introduced. Ordinary compiler runs allocate no candidate lists.

The old headless path, when semantic reporting was enabled, checked files serially before grouped parallel linting.
The new path performs both in the existing grouped checker task. In normal CLI mode, syntax/options errors still
select the usual compiler diagnostics, but do not bypass the explicitly requested check/lint pass.

## Measurements

macOS arm64, local release profile, same toolchain and Command Line Tools SDK for both builds. Before is
`32328b8d` plus the existing unrelated Unicode working-tree changes; after adds this feature. The unrelated
working-tree changes were kept in both builds. The project is vscode at
`9cf0128b9822dcd8a533f08ef7312956a4390301`, as pinned in `bench/projects.json`.

Headless requested all 9,399 non-declaration source files from the compiler's file list, with
`no-floating-promises` and both compiler-reporting switches enabled in both binaries. One-checker headless mode
sets `checkers: 1` through a tsconfig source override and `RAYON_NUM_THREADS=1`. Plain compiler runs use
`-p <vscode>/src --noEmit --incremental false --pretty false`, adding `--singleThreaded` and
`RAYON_NUM_THREADS=1` in one-checker mode. Program-diagnostic suppression was enabled equally for headless.

`/usr/bin/time -l` recorded retired instructions and maximum resident set size. One warm-up pair, then three
interleaved pairs per mode; the table gives medians and the median paired change. Instructions vary slightly
between runs, so the measured single-checker headless improvement range is included below. Other local work ran
during this experiment: wall times are contextual, not a README headline benchmark claim.

| Mode | Instructions before → after | Paired change | Peak RSS before → after | Paired change |
| --- | ---: | ---: | ---: | ---: |
| Headless, one checker | 104.540 → 101.850 G | -2.61% | 1928.41 → 1929.95 MiB | +0.08% |
| Headless, default | 113.646 → 112.893 G | -1.49% | 2236.06 → 2258.95 MiB | +1.02% |
| Plain compiler, one checker | 98.093 → 98.087 G | -0.03% | 1599.70 → 1597.77 MiB | -0.12% |
| Plain compiler, default | 108.719 → 108.715 G | -0.004% | 1944.36 → 1949.41 MiB | +0.37% |

The single-checker headless reduction was 1.80–2.73% in the three pairs, clearing the 1% instruction bar on a
pinned project. Default headless peak RSS increases about 1%; this is not a memory optimization. Default headless
wall medians were 11.94 → 2.14 s because semantic checking is now parallel instead of serial. This compares full
checking plus linting on both sides; it does not claim that mandatory checking is cheaper than the former lazy
lint-only mode with semantic reporting off. Plain compiler differences are within the observed variation.

All 7,187 headless diagnostic frames match before/after as multisets in both modes. Plain compiler diagnostics
are byte-identical. Artifacts and the measurement script are in the ignored
`target/scratch/tsgolint-review/` directory (`check-lint-measurements.json`, `measure-check-lint.py`).

## Validation

- 188 upstream rule cases and all 112 diagnostic snapshots pass; the original upstream skip is preserved.
- 14 local checks, including mandatory checking with empty rules/reporting off/`noCheck`/`@ts-nocheck`, one-time
  execution and binder candidate order/coverage; 7 CLI checks including overlays and native lint after syntax errors.
- Shared-cell checks pass with serial in-process test execution and for the CLI integration suite. Parallel
  independent programs in one test process are incompatible with the process-wide shared-allocation marker.
- AST, compiler and CLI integration suites pass. Two pre-existing CLI memory tests are excluded on macOS:
  `failed_builds_free_their_orchestrator` and `repeated_builds_memory` call glibc's `malloc_trim` in `rss_kib`;
  the crash report points to that missing symbol (PC 0). Their source is unchanged.
- All 28 standalone regressions pass. Cached reference conformance remains 13,458 error passes and 12,779 each
  for `.types` and `.symbols`, with no removed passes compared with the saved pre-change lists and no crashes/timeouts.
- Workspace and wasm32-wasip1 checks, lint ratchet and source checks pass.
- Oxlint 1.87.0 installed in the oxc1 checkout reports the expected rule diagnostic using only
  `OXLINT_TSGOLINT_PATH` to select this release binary. A nonexistent path fails, confirming subprocess selection.

Keep the binder candidate collection and file-completion hook. Do not replace them with callbacks from individual
expression checks without proving complete coverage and preserving the upstream diagnostic/suggestion order.
