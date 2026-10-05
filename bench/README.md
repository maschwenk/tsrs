# Benchmarks: tsrs vs tsgo

`bench/run.py` type-checks the projects the TypeScript team benchmarks the Go compiler on
([microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), `cases/`) with tsrs and
with tsgo 7.0.2 (npm `typescript@7.0.2`), and reports wall time, peak memory and the error count of each.
The Depot CI workflow `.depot/workflows/bench.yml` runs it on every push to `main` and rewrites the table at the top of `README.md`.

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
- tsrs: by default the release build of the checked-out commit (`target/release/tsrs`). CI measures what the npm
  package ships instead: the PGO build of the `dist` profile (fat LTO, one codegen unit), built with the commands of
  the release workflow and trained with `.github/scripts/pgo-train.sh` (conformance suite + xstate-main + webpack) on
  the bench runner itself (`--tsrs <binary> --tsrs-build pgo-dist`; notes/perf-pgo.md). Training and measurement
  share the project checkouts in `bench/.work`; the training processes have exited before the first measured run.
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

## Regression flag

Wall time on a shared VM moves 2-4% between runs with no code change (2026-10-04, 64 runs: the tsrs/tsgo ratio moved
2.0% run to run at the median, and a README-only commit showed a 3.4% step), so it cannot show a small regression. Each
run therefore also counts the user-space instructions of one single-threaded tsrs type check per project
(`bench/count.py`, Linux `perf_event_open`; untimed, after the timed runs; `--singleThreaded` and
`RAYON_NUM_THREADS=1`). That count repeats to about 0.001% (five runs on the bench machine: 0.0000-0.0005% spread),
so any change in it comes from the code. The PGO profile is retrained every run; two trainings of one commit gave
different profiles and binaries whose counts differed by 0.002-0.046%, well under the threshold. tsgo is not counted:
the Go runtime makes its count vary 2-4%.

`bench/regressions.py --latest` compares the counts with the newest earlier result from the same runner label and
build, and only when the CPU model and C library match (they pick different `memcpy`-style routines). A project up
more than 1% is a regression: the step prints a warning and comments on the pull request merged in between, or, when
the run covers more than three merges, on the commit. It never fails the job; a deliberate trade (memory for CPU, say)
needs no action. Results store the count under `projects.<name>.single.tsrs.instructions`.

The same untimed run records the process's peak RSS (`ru_maxrss` via `wait4`). With one thread it repeats to
0.04-0.39% (five runs per project: webpack 0.04%, mui-docs 0.07%, xstate 0.10%, Compiler 0.24%, Compiler-Unions 0.39%;
the small projects move by a few hundred KiB), so peak memory is flagged when it rises by more than 1% and more than
2 MiB. Results store it as `projects.<name>.single.tsrs.max_rss_bytes`.

## CI

The benchmark is a [Depot CI](https://depot.dev/docs/ci/overview) workflow, `.depot/workflows/bench.yml` (this
repository is connected to Depot CI; Depot's runners for GitHub Actions only serve organization-owned repositories, and
a probe of `depot-ubuntu-*` / `depot-macos-latest` labels from GitHub Actions stayed queued). It runs on every push to
`main` except pushes that touch only `README.md` / `bench/results/**`, and on `workflow_dispatch`. Nobody waits on it,
so it is built for unattended runs: one bench at a time, never cancelled (a push during a run queues the newest
commit), network steps retried, and the results commit rebased onto the latest `main` before pushing.

**Fixed machine spec**: `depot-ubuntu-24.04-8`, 8 vCPU, 32 GB RAM, Linux x86_64, for every run, so numbers are
comparable over time. Sizing: the largest peak measured is tsgo's default mode on vscode and mui-docs (7.5 GiB on an
18-core Mac, 6.9 GiB on Linux); 32 GB leaves 4x headroom, and 8 vCPUs cover the 4 checker threads plus parallel
parsing. The first step fails the job if `nproc`/`MemTotal`/arch differ; there is no fallback runner. (On GitHub's
standard 2 vCPU / 7 GB runner tsgo swapped on vscode: 146 s wall for a 50 s check.)

Caching (Depot Cache serves the `actions/cache` API on Depot CI, no special configuration): `Swatinem/rust-cache` for
`~/.cargo` and the dependency part of `target/`, which holds the PGO-instrumented build (workspace crates are
rebuilt: they are what is being measured). The final PGO build is not cached: its RUSTFLAGS contain the profile's
hash, which changes every run, so cargo rebuilds all of it anyway (as in the release workflow); one
cache for the suite checkout plus the npm-installed compilers (`typescript@7.0.2` and the reference nightly); one cache
per cloned project (checkout + `node_modules`) keyed on its pinned commit and install command
(`bench/run.py --print-cache-keys`), so changing one pin re-installs only that project.

Job duration split, 2026-10-01:

| | GitHub-hosted `ubuntu-24.04` (2 vCPU / 7 GB), before | Depot CI `depot-ubuntu-24.04-8`, all caches warm |
| --- | --- | --- |
| runner start, toolchain, rust-cache restore | ~20 s | 11 s |
| `cargo build --release -p tsrs_cli` | 80-134 s | 34 s |
| restore project caches | 45-85 s (one 5 GB cache) | 14 s (5 caches) |
| clone + install projects (cold cache only) | 151 s, then 91 s to save the cache | 0 s |
| measurement (`bench/run.py`, 6 projects x 2 modes x 2 compilers x 3 reps + warm-ups) | never finished: vscode alone took 19 min (tsgo swapped), cancelled | 424 s |
| commit + push results | | 2 s |
| total | | 8 min 10 s (measurement 87%) |

The first Depot run (cold caches: clone, install and cache save for all projects) took 11 min 3 s. What is left in
a warm run is the tsrs build (it changes with every commit) and the measurement.

Since the bench measures the PGO `dist` build (2026-10-01), the 28-34 s `cargo build --release` step is replaced by
the PGO pipeline, measured on Depot run `ps_h83310tmld`: TypeScript testdata sparse checkout ~14 s, instrumented build
of tsrs + tsrs-test 100 s, training run 15 s, profile merge < 1 s, final build 74 s, so ~3 min more per run (job 7 min
13 s -> 10 min 4 s). The dependencies, the only part rust-cache can reuse, compile in ~3 s; the time is the
workspace crates with one codegen unit and fat LTO, which change with every commit.
