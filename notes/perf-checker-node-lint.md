# Direct checker lint dispatch (2026-10-10)

The request is to accept headless configuration, check a node, dispatch its lint rules and emit diagnostics from
that work. The preceding implementation (`2fbb51f4`, `notes/perf-checker-lint.md`) still implemented lint as a
file-completion pass over binder-collected expression statements. This change replaces that architecture.

## Design and prior work

Reviewed `notes/perf-checker-lint.md`, the rejected-techniques list in `notes/perf-round2-followups.md`, `docs/RUST.md`
Techniques and the latest memory-round summary before changing the integration. The previous note required proof
of traversal coverage and upstream diagnostic order before trying per-node dispatch. The changed constraint is the
explicit request for direct checker integration and fewer layers; the imported suite and coverage checks provide
that proof. This is an architectural change, not a claimed performance optimization.

- Both CLI entry points deserialize the same `HeadlessConfig`. `LintConfig` prepares rule options once and enters
  through `ProgramOptions` before checkers are created. Empty rule lists still select files for mandatory checking.
- `check_expression_statement` checks the expression, then calls `lint_node`. The native rule enum and rule
  implementations live in `tsrs_checker::lint`; `tsrs_linter` only loads programs and requests semantic diagnostics.
- Remove `LintNodes`, the binder candidate vectors/sort, `LintHost`, `CheckFileHook`, `LintSession`, `Once`, the
  program-setup/host callbacks and diagnostic callbacks. There is no separate file-level rule runner.
- A checker-local node-link flag prevents repeat dispatch through inference or deferred checking. Checkers buffer
  rule diagnostics by file, and the compiler collects only files assigned to that checker. Linted declaration
  files stay whole rather than being split between checkers. Output sorting restores source order for deferred
  function bodies. There is no new thread or cache of rule candidates.
- TypeScript deliberately skips `with` bodies. Only that skipped subtree is explicitly visited for lint coverage;
  unreachable code, function/class bodies and ordinary statements use the existing checker traversal.
- Timing `calls` now counts node dispatches rather than a file-start call plus its nodes. The wire frame schema,
  diagnostics, suggestions and options remain compatible.

Shared-cell validation also exposed a lifecycle issue: the previous program's frozen allocation ranges could
remain active while parallel parsing bound the next program, before the checker's existing thaw. Reset that
debug-only boundary at program creation, before parsing/binding workers start. The checker pool still freezes
this program after binding. Serial test execution is still required for this process-global debug instrument.

## Measurements

macOS arm64, local release builds with the same toolchain and Command Line Tools SDK. Before is `2fbb51f4`; after
is the direct-dispatch working tree. Both include the existing unrelated Unicode working-tree changes.

The corpus and commands are the same as `notes/perf-checker-lint.md`: vscode at the `bench/projects.json` pin
`9cf0128b9822dcd8a533f08ef7312956a4390301`, 9,399 source files selected for `no-floating-promises`, both TypeScript
reporting switches on. One-checker headless uses a tsconfig override and `RAYON_NUM_THREADS=1`. Plain compiler
runs use `--noEmit --incremental false --pretty false`, adding `--singleThreaded` for the one-checker run.

`/usr/bin/time -l` records retired instructions and peak RSS. One warm-up pair, then three interleaved pairs per
mode. Values are medians; percentage changes are medians of paired changes. Wall time varied widely, especially
in default headless mode, so this is not a wall-time benchmark claim.

| Mode | Instructions before → after | Paired change | Peak RSS before → after | Paired change |
| --- | ---: | ---: | ---: | ---: |
| Headless, one checker | 101.785 → 101.741 G | -0.04% | 1931.83 → 1949.52 MiB | +0.91% |
| Headless, default | 113.264 → 113.344 G | +0.12% | 2256.30 → 2266.22 MiB | +0.34% |
| Plain compiler, one checker | 97.864 → 98.054 G | -0.08% | 1599.83 → 1597.84 MiB | -0.12% |
| Plain compiler, default | 108.549 → 107.441 G | -1.02% | 1954.83 → 1948.06 MiB | -0.35% |

Single-checker headless instruction changes range from -0.95% to +0.10%; plain default changes range from -1.09%
to +0.06%. These results do not establish a performance win. The reason to keep the code is the requested direct
integration and removal of the previous architecture. Peak memory rises slightly for headless; this is not a memory
optimization. No additional performance machinery is justified by these numbers.

All 16 before/after output pairs match: all 7,187 headless diagnostic frames as multisets in both modes, and
byte-identical plain compiler output. Artifacts are under ignored `target/scratch/tsgolint-review/`:
`measure-node-dispatch.py`, `node-dispatch-measurements.json`, `node-dispatch-measurement-summary.json` and the
`node-dispatch-*.out` / `.time` files.

## Validation

- All 188 runnable imported upstream cases and 112 original snapshots pass; the one original skip remains.
- 15 local checks cover mandatory checking with empty rules/reporting disabled, `noCheck`, `@ts-nocheck`,
  `skipLibCheck`, per-node deduplication, deferred/unreachable/skipped bodies, and per-file options across checkers.
- Eight CLI checks cover headless framing, overlays, native `--lint`, syntax errors and configured declaration files
  with splitting forced on/off. The upstream/local suites and protocol/native checks also pass with shared-cell
  validation after the lifecycle fix.
- AST, binder, compiler and CLI suites pass, excluding the same two pre-existing macOS-incompatible CLI memory
  tests described in `notes/perf-checker-lint.md`. All 28 standalone regression fixtures pass.
- Reference conformance remains 13,458 diagnostic passes and 12,779 each for `.types` and `.symbols`; comparison
  with the previous saved pass lists loses no passes, with no crashes or timeouts. The two existing diagnostic-code
  differences and two existing failures remain.
- Workspace and wasm32-wasip1 checks, lint ratchet and source checks pass. An additional compiler-only
  `--no-default-features` probe still fails on pre-existing missing split-check methods in the checker stub
  (`file_diagnostics_so_far`, `check_source_file_piece`, `add_piece_diagnostics`); those missing methods are unchanged.
- Oxlint 1.87.0 in `oxc1` reports the expected rule diagnostic through the release binary selected solely with
  `OXLINT_TSGOLINT_PATH`.
