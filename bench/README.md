# Benchmarks: tsrs vs tsgo

`bench/run.py` type-checks the projects the TypeScript team benchmarks the Go compiler on
([microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), `cases/`) with tsrs and
with tsgo 7.0.2 (npm `typescript@7.0.2`), and reports wall time, peak memory and the error count of each.
`.github/workflows/bench.yml` runs it on every push to `main` and rewrites the table at the top of `README.md`.

```sh
cargo build --release -p tsrs_cli
python3 bench/run.py --local                                   # all projects, 3 reps, both modes
python3 bench/run.py --local --projects webpack,Compiler --reps 1
python3 bench/run.py --local --work-dir ~/bench-cache          # reuse clones across worktrees
python3 bench/run.py --local --readme README.md                # also rewrite the README block
```

Results go to `bench/results/<date>-<tsrs commit>[-local].{json,md}`; per-run compiler output to
`<work-dir>/logs/<timestamp>/`. The first run clones and installs the projects into `<work-dir>/solutions`
(default `bench/.work`, ~5 GB); later runs reuse them as long as `bench/projects.json` is unchanged.

## Suite mapping

Suite commit `41f652ab` (`bench/projects.json`). Each suite case is a `scenario.json` (`tsc -p <dir> --outdir <tmp>`)
plus a `setup.sh` (clone, install inside a network-restricted Docker sandbox). We run the same clone and install
commands without the sandbox, and check the same `-p` path.

| suite case | repository | commit | `-p` | install (from the case's `setup.sh`) |
| --- | --- | --- | --- | --- |
| `vscode` | microsoft/vscode | `9cf0128b9822` | `src` | `npm ci --ignore-scripts` |
| `xstate-main` | statelyai/xstate | `fbee62e7c158` | `.` | `npx <packageManager> install` (pnpm 10; runs its postinstall `preconstruct dev`, like the suite) |
| `webpack` | webpack/webpack | `b54c5e2aa26d` | `.` | `yarn install --ignore-scripts --ignore-engines`, then `yarn link` webpack into itself |
| `mui-docs` | mui/material-ui | `6780195595c1` | `docs` | `npx <packageManager> install --ignore-scripts` (pnpm 12) |
| `Compiler` | (in the suite repo) | suite commit | `cases/solutions/Compiler` | none |
| `Compiler-Unions` | (in the suite repo) | suite commit | `cases/solutions/Compiler-Unions/tsconfig.json` | none |

Commits: the suite pins only `mui-docs` (`6780195595c1`, "last MUI ref known to install in the sandbox"); the other
three clone the repository's HEAD at run time. We pin those to the commits used for the typescript-benchmarking
numbers in microsoft/TypeScript#64475 (gist `c83d9185c6961b6fb43d7d071ef39cf6`, `refs.txt`, late September 2026).
The suite runs yarn 1 from its Docker image; we run `npx yarn@1.22.22` (webpack's `packageManager`).

`Compiler` is defined in `cases/` but not scheduled in the suite's pipeline (`scripts/src/setupPipeline.ts` lists only
`Compiler-Unions`); it is kept because the #64475 measurements include it.

Not included: `angular-1` (its tsconfig uses options TypeScript 7 removed); the `*-1` variants (the suite's frozen
baseline copies of the same four projects at older pinned commits, e.g. vscode `f88bce8f`); `strada-compiler`,
`ts-pre-modules`, `strada-build-src`, `self-build-src-public-api` (TypeScript's own 5.x sources, two of them `tsc -b`
builds); the tsserver/LSP/startup scenarios (not `tsc` runs). All six included projects fit a standard runner.

## What is measured

- Invocation, identical for both: `-p <project> --noEmit --incremental false --extendedDiagnostics --pretty false`,
  in the default mode (both 4 checker threads; tsrs additionally resolves members lazily, its default) and with
  `--singleThreaded`. `--noEmit` instead of the suite's `--outdir` keeps the measurement to type checking; `--pretty
  false` makes the error lines parseable.
- tsgo: `npm install typescript@7.0.2`. Its `bin/tsc` is a Node launcher (`lib/tsc.js` -> `getExePath.js`) that
  `execve`s the Go binary `@typescript/typescript-<os>-<arch>/lib/tsc`; the harness runs that binary directly (checked
  to be a native executable printing `Version 7.0.2`), so Node startup is not part of the measurement.
- tsrs: the release build of the checked-out commit (`target/release/tsrs`).
- Per project: one untimed warm-up run (tsgo), then N reps (default 3); within a rep the two modes and the two
  compilers alternate, and the compiler order flips every rep. Tables report medians.
- Wall: process wall clock measured by the harness. Peak: max RSS from `wait4` rusage (the number `/usr/bin/time -v`
  prints). The JSON also has the `--extendedDiagnostics` check/total time, "Memory used", files, symbols, types and
  instantiations of every run. tsrs's default-mode counters are lower than tsgo's by design (lazy members); compare
  counters in `--singleThreaded` runs or with `--noLazyMembers`.
- Errors: every `error TSxxxx` line is counted, and the (file, line, col, code) lists are compared. A differing count is
  shown as **MISMATCH** in the table and a differing location set as "(locations differ)" — a correctness signal,
  not a performance one.
- tsrs ports a TypeScript 7.1-dev commit (`b85298b6`, `Cargo.toml`), not 7.0.2, so some differences are TypeScript
  version differences. When the two disagree, the harness also runs `typescript@7.1.0-dev.20260930.4` (the nightly
  built from `b85298b6` plus one CI-only commit; `reference` in `projects.json`) once: if it reports exactly tsrs's
  errors the cell reads `tsgo / tsrs (ref N)` and is not flagged. On 2026-10-01 that explains both differences
  (vscode 359 vs 371, webpack 848 vs 840: tsrs's lists equal the nightly's line for line). When the port moves to a
  newer TypeScript commit, update `reference` to the nightly built from it (the harness warns when they differ).

## CI

`bench.yml`: `ubuntu-24.04` (the standard GitHub-hosted runner; larger runners need a GitHub Team/Enterprise
organization and this repository belongs to a personal account), release build, projects restored from
`actions/cache` (key = hash of `bench/projects.json`), all projects, 3 reps, then commits `README.md` and
`bench/results/` as `github-actions[bot]` with `[skip ci]`. Pushes that touch only `README.md` / `bench/results/**`
do not trigger it; a newer push cancels a running bench. Shared-runner numbers are noisy.
