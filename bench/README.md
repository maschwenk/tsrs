# Benchmarks: tsrs vs tsgo

`bench/run.py` type-checks the projects the TypeScript team benchmarks the Go compiler on
([microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), `cases/`) and four large
open-source applications (see "Application projects") with tsrs and with tsgo 7.0.2 (npm `typescript@7.0.2`), and
reports wall time, peak memory and the error count of each.
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

## Application projects

The suite has one application-shaped workload (`mui-docs`), and none of its projects leans on schema-validation
types. Four large applications fill that gap: three Next.js/React apps built on Zod and tested with Vitest, and an
Effect server (Effect Schema, `@effect/vitest`). Each is pinned to one commit and checks the tsconfig its own
`typecheck` script checks, so test files are part of the program.

| project | repository @ commit | `-p` | files / types (tsgo) | workload |
| --- | --- | --- | ---: | --- |
| `cal-diy` | calcom/cal.diy @ `54343aa685ae` | `apps/web` | 10,166 / 2.48 M | Zod 3, tRPC 11, Prisma-generated Zod types |
| `formbricks-web` | formbricks/formbricks @ `ec45c28e4d20` | `apps/web/tsconfig.typecheck.json` | 10,451 / 2.85 M | Zod 4, Vitest, Prisma 7 |
| `supabase-studio` | supabase/supabase @ `12afe3999951` | `apps/studio` | 12,546 / 2.22 M | Zod 3, Vitest, Next.js route types |
| `t3code-server` | pingdotgg/t3code @ `9bd1d8009a6b` | `apps/server` | 3,007 / 3.18 M | Effect 4, Effect Schema, `@effect/vitest` |

Each install command does what the app's own `typecheck` needs before `tsc` runs, with package scripts off:

- `cal-diy`: Yarn 4 install, then `turbo run post-install @calcom/trpc#build` (Prisma client, Zod and Kysely types,
  the platform packages, and the tRPC router declarations `apps/web` imports). `YARN_NM_MODE=classic` keeps a global
  Yarn configuration (hardlinked node_modules) from changing the install, and the package cache goes to `$TMPDIR` so
  it is not part of the project's CI cache.
- `formbricks-web`: pnpm install, then `turbo run build --filter='@formbricks/web^...'`, which generates the Prisma
  client and builds the workspace packages whose `dist/*.d.ts` the app imports. The build scripts call `pnpm`, so the
  command puts a `.bench-bin/pnpm` shim (`npx pnpm@<packageManager version>`) on `PATH`. `DATABASE_URL` is a dummy
  (Prisma's config requires one to generate; nothing connects).
- `supabase-studio`: pnpm install of `studio` and its workspace dependencies, then `next typegen` (the app's
  `pretypecheck`).
- `t3code-server`: pnpm install of `t3`, `@t3tools/scripts` (`apps/server`'s tsconfig includes `scripts/lib`) and
  their workspace dependencies. The repository's `prepare` script patches its TypeScript for the Effect language
  service, which neither compiler measured here runs.

**Overlays.** cal.diy and Formbricks still compile with TypeScript 5.9, whose tsconfig options TypeScript 7 removed
(`baseUrl`, `moduleResolution: node`, `target: es5`); unmodified, both stop at config errors before checking a
file. `bench/overlays/<name>/` holds the minimal TypeScript 7 replacements, copied over the checkout before the
install: `baseUrl` becomes a `"*": ["./*"]` path, `node` resolution becomes `bundler`, ES5 becomes ES2017.
cal.diy's generated tRPC declarations import `@trpc/server/dist/...`, which `bundler` resolution blocks through the
package's `exports`, so a `paths` entry maps it to the package directory as `node` resolution did (without it,
tsgo reports 727 errors instead of 136). Formbricks also gets the `next-env.d.ts` Next.js writes on `next dev`
(image module types). The overlay's content hash is part of the checkout marker and the CI cache key.

**Errors.** The applications are not error-free under TypeScript 7 with these settings (cal.diy: 136, mostly test
matchers and untyped resolver modules). As with vscode and webpack, the count is a correctness signal between the two
compilers. On 2026-10-06 (local, macOS arm64), tsgo 7.0.2 and tsrs reported identical errors on cal.diy (136) and
Formbricks (0); on Supabase Studio (0 vs 9) and t3code (5 vs 6), `typescript@7.1.0-dev.20260930.4` reported exactly
tsrs's errors (`(ref N)` in the table).

## What is measured

- Invocation, identical for both: `-p <project> --noEmit --incremental false --extendedDiagnostics --pretty false`,
  in the default mode (both 4 checker threads; tsrs additionally resolves members lazily, its default), with
  `--singleThreaded`, and with `--checkers 8` (the `checkers8` mode: how each compiler scales when given twice the
  default checkers; select modes with `--modes default,single,checkers8`, flags in `MODE_FLAGS` in `run.py`).
  `--noEmit` instead of the suite's `--outdir` keeps the measurement to type checking; `--pretty
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
build, and only when the CPU model and C library match (they pick different `memcpy`-style routines). When the Rust
compiler differs (results record `tsrs.rustc`; `rust-toolchain.toml` pins it), it prints the change as the upgrade's
measurement and flags nothing (`docs/RUST.md`, "Upgrading Rust"). A project up
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

## Head-to-head on a wide machine

`bench/compare.py` answers a different question from `run.py`: how tsrs, both TypeScript 7 builds and `bun check`
compare on one wide machine, the way Bun's `bun check` announcement measured them (vscode, Linux x64, 64 threads,
mean of 20 runs). It runs four compilers on the same checkouts:

- `tsgo`: `typescript@7.0.2` (as in `run.py`);
- `tsgo-dev`: `typescript@7.1.0-dev.20260930.4` (`reference` in `projects.json`), built from the TypeScript commit
  tsrs ports, so it is both the like-for-like Go baseline and the error oracle;
- `tsrs`: the binary given with `--tsrs`;
- `bun`: `bun check` from the binary given with `--bun` (1.4.3 canary or later), run as
  `bun check -p <project> --no-pretty --all` (`--all` turns off the grouping of repeated errors so every error line is
  counted).

Thread settings (`--threads`, default `default,4,8,16,all`): `default` passes no flag (tsgo and tsrs use 4 checker
threads, bun one thread per core); a number N passes `--checkers N` to tsgo and tsrs and `--threads N` to bun;
`all` is the machine's thread count. The knobs are not identical: `--checkers` sets only the checker threads (parsing
and binding still use every core), while bun's `--threads` caps all of its threads. Per project, one untimed warm-up
per compiler, then `--reps` (default 20) reps in which the compiler order rotates; the table reports mean ± standard
deviation, median and minimum wall time and mean peak RSS, plus whether each compiler's (file, line, column, code)
error list equals 7.1-dev's. bun prints its own file count, which leaves out the default `lib` files (13 fewer on
`Compiler`).

```sh
python3 bench/compare.py --tsrs target/release/tsrs --bun ~/.bun/bin/bun --projects vscode --reps 20
python3 bench/compare.py --tsrs ... --bun ... --projects Compiler --threads default,4 --reps 2   # smoke test
```

The Depot CI workflow `.depot/workflows/bench-compare.yml` (manual dispatch only) runs it on `depot-ubuntu-24.04-64`
(64 vCPU) with the PGO `dist` build of the commit (built and trained as in `bench.yml`) and Bun canary, and uploads
`bench/results/compare/<date>-<commit>-<threads>t.{json,md}` plus the logs as the `bench-compare` artifact.
