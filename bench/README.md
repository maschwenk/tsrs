# Benchmarks: tsrs vs tsgo

`bench/run.py` type-checks the projects the TypeScript team benchmarks the Go compiler on
([microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), `cases/`) and eleven
open-source applications and libraries (see "Application projects") with tsrs, with tsgo 7.0.2 (npm `typescript@7.0.2`) and, where a
Bun binary is given, with `bun check`, and reports wall time, peak memory and the error count of each.
The Depot CI workflow `.depot/workflows/bench.yml` runs it on every push to `main`. It measures every project on the
fixed-spec 8-vCPU machine in three modes and on a 16-vCPU machine in two (with bun check); `bench/results/<date>-<commit>.md`
has every table, and the table at the top of `README.md` is the 16-vCPU machine's default-mode one: each compiler at
its own thread count, tsgo, tsrs and bun check side by side (`run.py --readme-modes wide`).

```sh
cargo build --release -p tsrs_cli
python3 bench/run.py --local                                   # all projects, 3 reps, default/single/checkers8 modes
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
types. Application projects include four large applications (three Next.js/React apps built on Zod and tested with Vitest, and an
Effect server), plus Bun's published benchmark set (mikro-orm, next.js root and packages/next, storybook, nuxt, playwright)
and one more popular library (drizzle-orm). Each is pinned to one commit.

| project | repository @ commit | `-p` | files | workload |
| --- | --- | --- | ---: | --- |
| `cal-diy` | calcom/cal.diy @ `54343aa685ae` | `apps/web` | 10,166 | Zod 3, tRPC 11, Prisma-generated Zod types |
| `formbricks-web` | formbricks/formbricks @ `ec45c28e4d20` | `apps/web/tsconfig.typecheck.json` | 10,451 | Zod 4, Vitest, Prisma 7 |
| `supabase-studio` | supabase/supabase @ `12afe3999951` | `apps/studio` | 12,546 | Zod 3, Vitest, Next.js route types |
| `t3code-server` | pingdotgg/t3code @ `9bd1d8009a6b` | `apps/server` | 3,007 | Effect 4, Effect Schema, `@effect/vitest` |
| **Bun benchmark set** | | | | |
| `mikro-orm` | mikro-orm/mikro-orm @ `97fb231` | `.` | 2,911 | TypeScript 7 monorepo, ORM with schema types |
| `next-packages-next` | vercel/next.js @ `fa8dcf3` | `packages/next` | 2,882 | Next.js core package (Bun measured 2,881 files) |
| `next-root` | vercel/next.js @ `fa8dcf3` | `.` | 3,549 | Next.js repo root monorepo (Bun measured 3,547 files) |
| `storybook` | storybookjs/storybook @ `48dfcc6` | `scripts` | ~1,039 | Storybook scripts (Bun measured 1,039 files) |
| `nuxt` | nuxt/nuxt @ `85b8d54` | `.` | ~3,901 | Nuxt framework monorepo (broader tsconfig than Bun's 839; same commit) |
| `playwright` | microsoft/playwright @ `d469960` | `.` | ~1,515 | Playwright testing framework (broader tsconfig than Bun's 706; same commit) |
| **Other popular libraries** | | | | |
| `drizzle-orm` | drizzle-team/drizzle-orm @ `15454db` | `.` | ~943 | Drizzle ORM (large popular project) |

**Original application projects:** Each install command does what the app's own `typecheck` needs before `tsc` runs, with package scripts off:

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

**Bun benchmark set and large popular projects:** Minimal installs without build steps (the projects' tsconfigs require only node_modules):

- `mikro-orm`: Yarn 4 install.
- `next-packages-next` and `next-root`: pnpm install with `--frozen-lockfile`.
- `storybook`: Yarn 4 install (scripts directory).
- `nuxt`: pnpm install.
- `playwright`: npm ci.
- `drizzle-orm`: pnpm install.

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
  in the default mode (no thread flag: tsgo uses 4 checker threads; tsrs every core up to 8, then half the cores,
  never fewer than 4 nor more than 32, and at most one per 32 type-checked files, so 8 on the 8-vCPU machine for all
  but small projects; tsrs additionally resolves members lazily, its default), with `--singleThreaded`, with
  `--checkers 8` (the `checkers8` mode: both compilers at 8 checker threads, twice tsgo's default), and on a 16-vCPU machine (see "CI") in the default mode again (the `wide` mode: the same flags as
  `default`, kept apart so that one result can hold both machines) and with `--checkers 16` (the `checkers16` mode: how
  each scales on a wide machine). Select modes with `--modes` (default `default,single,checkers8`; flags in `MODE_FLAGS`
  in `run.py`).
- `bun check` (`--bun <binary>`, Bun 1.4.3 canary or later) is a third column in every mode it is measured in: `bun check
  -p <project> --no-pretty --all`, with `--threads N` where tsgo and tsrs get `--checkers N` (bun's only thread knob,
  and it caps every thread). CI measures it on the 16-vCPU machine. Its error count is recorded, not compared: bun check
  follows TypeScript 7.0 (on vscode its errors equal tsgo 7.0.2's line for line), tsrs the 7.1-dev commit it ports.
  `--noEmit` instead of the suite's `--outdir` keeps the measurement to type checking; `--pretty
  false` makes the error lines parseable.
- tsgo: `npm install typescript@7.0.2`. Its `bin/tsc` is a Node launcher (`lib/tsc.js` -> `getExePath.js`) that
  `execve`s the Go binary `@typescript/typescript-<os>-<arch>/lib/tsc`; the harness runs that binary directly (checked
  to be a native executable printing `Version 7.0.2`), so Node startup is not part of the measurement.
- tsrs: by default the release build of the checked-out commit (`target/release/tsrs`). CI measures what the npm
  package ships instead: the PGO build of the `dist` profile (fat LTO, one codegen unit), built with the commands of
  the release workflow and trained with `.github/scripts/pgo-train.sh` (conformance suite + fourslash suite + xstate-main + webpack),
  then BOLT-optimized like the Linux release binaries (`.github/scripts/bolt.sh`, tsrs trained on xstate-main +
  webpack) on a machine of the bench spec (`--tsrs <binary> --tsrs-build pgo-dist`; notes/perf-pgo.md,
  notes/perf-binary-layout.md), in the build job; each
  project is then measured on a fresh machine of that spec where nothing else has run (the `measure` jobs, "CI").
- Per project: one untimed warm-up run (tsgo), then N reps (default 3); within a rep the two modes and the two
  compilers alternate, and the compiler order flips every rep. Tables report medians.
- Wall: process wall clock measured by the harness. Peak: max RSS from `wait4` rusage (the number `/usr/bin/time -v`
  prints). The JSON also has the `--extendedDiagnostics` check/total time, "Memory used", files, symbols, types and
  instantiations of every run. tsrs's default-mode counters are lower than tsgo's by design (lazy members); compare
  counters in `--singleThreaded` runs or with `--noLazyMembers`.
- Rows are sorted by the speedup vs tsgo, biggest first (`speedup_sort_key` in `run.py`); a failed row goes last.
- Bold in the speedup and memory columns marks a notable tsrs win: at least 5x faster, or at least 4x more memory
  efficient (tsgo's peak memory / tsrs's; `NOTABLE_SPEEDUP`, `NOTABLE_MEMORY` in `run.py`; compared at the printed two
  decimals). The memory columns are always `other tool's peak / tsrs's peak`, so above 1x is a tsrs win.
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

`bench/regressions.py <result>` (or `--latest`, the newest result) compares the counts with the result of the nearest
earlier benchmarked ancestor commit from the same runner label and build (the newest earlier result by date when git
cannot relate the commits), and only when the CPU model and C library match (they pick different `memcpy`-style routines; compared per
project, since a parallel run can land a project on another machine model, which `run.py --merge` records on the
project's cells). When the Rust compiler differs (results record `tsrs.rustc`; `rust-toolchain.toml` pins it), it prints the
change as the upgrade's measurement and flags nothing (`docs/RUST.md`, "Upgrading Rust"). A project up
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
so it is built for unattended runs: one run per push (the concurrency group is the commit, so a push during a run
starts its own run on its own machines and only a re-dispatch of the same commit replaces a queued twin; until
2026-10-07 one shared group let a later push replace the queued bench, and bursts of merges left most commits without
a results file), never cancelled, network steps retried, and the results commit re-created on top of the latest
`main` before pushing. Runs overlap and finish in any order, so `README.md` keeps the newest commit's table: a run
whose commit is an ancestor of the one the table came from (its last line names it) adds its results file and leaves
the table alone, and the regression flag compares a run with the nearest earlier benchmarked ancestor of its commit,
not with the newest file by date. A pull request that changes code must not carry a CI-skip marker in its title, body or commit subjects (CI's
`skip-marker` job, `tools/ci/skip_marker.py`): `gh pr merge --squash` copies them into the commit message, and such a
commit lands without CI and without a results file; notes-only pull requests carry the marker on purpose. A commit
whose bench was skipped or cancelled (before 2026-10-07, most commits of a burst of merges) is backfilled with `depot ci dispatch --repo maschwenk/tsrs --workflow bench.yml --ref main --input
commit=<sha>`: the build and measuring jobs check out that commit, the merge job runs the current scripts, the results
file is committed, and the README table is applied only when no newer commit's table is there.

**Noise**: the benchmark machines are shared cloud VMs. Historically, between publishes of the same code vscode's
64-vCPU wall moved by a median 2% and up to 10% on 2026-10-07 (median of 3). The single-threaded instruction count is
the deterministic measure (repeats to 0.001%). The `measure` jobs run 5 reps and `measure-wide` 10 for tsrs and
`bun check`, both with 3 for tsgo (`--reps N --tsgo-reps M`: tsgo takes 10-20x longer per run), and the README's
headline is the 20-run mean of `bench-compare.yml`, not one publish's median.

The Bun comparison uses 16 vCPU by default; 32 and 64 vCPU remain explicit choices in `bench-compare.yml`.
Runner vCPUs and checker threads are separate: `cpus=16` with `threads=64` runs 64 software workers on
16 vCPUs. This oversubscription is supported for experiments, including PR verification (`cpus=16`,
`checkers=1,16,32,64`), but does not provide 64 cores of compute.
The 8-vCPU instruction regression jobs retain their existing hardware. Historical 64-vCPU results keep their original
labels; history starts a new baseline when a cell's hardware or build setup changes. Backfills of commits whose
`run.py` predates `checkers16` measure only `wide` on the 16-vCPU runner.

**History**: `bench/history.py` prints one table per metric over `bench/results/*.json` in `main`'s first-parent
order, a column per project, each cell with its change against the previous benchmarked commit (with one run per
push, the previous merge): `--metrics instructions,peak,wall,wide_wall,wide_peak`, `--projects`, `--since <commit>`,
`--tsv`. Instructions and peak RSS are deterministic, so a `!` cell (past the regression thresholds) is the commit's
doing; the wall columns are the noisy ones and say whether the cost bought anything, and a `~` marks a wall or
wide-peak change next to an instruction change under 0.3%: the code did not change, that is the runner.

Four jobs (since 2026-10-07; until then one job did everything in sequence, and the measurement alone took 18-20 min
once the four application projects were in):

1. `build`, on one machine of the fixed spec: the PGO + BOLT `dist` build of the commit (below). The binary and the
   `rustc -V` that built it go up as the `tsrs-pgo-dist` artifact.
2. `measure`, one job per project of `bench/projects.json` (the matrix is `bench/run.py --print-projects`, so a new
   project gets a job without a workflow change), each on its own machine of the fixed spec: it restores the project's
   cache, downloads the binary and runs `bench/run.py --projects <name> --out-dir <dir>`. The result is the
   `bench-partial-<name>` artifact, the compiler output `bench-logs-<name>`. The jobs run at the same time but never
   share a machine, so a measurement is what it was in the sequential run: one project on an idle 8-vCPU machine.
3. `measure-wide`, one job on `depot-ubuntu-24.04-16` (16 vCPU): every project in the default mode (`wide`: each
   compiler at its own thread count, tsrs 8 checkers there) and in `--checkers 16` mode (`bench/run.py --modes
   wide,checkers16`), each with `bun check` from Bun canary as a third column (`--bun`). 16 checker threads would
   oversubscribe the fixed-spec machine. The job runs while the `measure` jobs do; it is the run's only 16-vCPU job. It
   restores the caches the `measure` jobs save (a project added to `projects.json` needs a restore step in it too). If
   it fails, the results file is published without the 16-vCPU sections and the README table is left as it was.
4. `merge`: `bench/run.py --merge <results...>` joins them into one result, projects in `bench/projects.json` order
   and modes in `MODE_FLAGS` order, after checking that the binary, the compilers and flags agree (the rep counts may differ, as the
   `measure` jobs' 5 and `measure-wide`'s 10 do; the result records every mode's in `mode_reps`) and that no
   (project, mode) is measured twice; then the regression flag and the results commit, as before. The machine that
   measured the most cells is the run's; a cell measured on another records its own `machine`. All `--checkers 16`
   cells do, and the table names their machine. A fixed-spec project measured on a different CPU model or C library
   (so far every `depot-ubuntu-24.04-8` has been the same model) is noted under the table, and the regression flag
   compares that project only with runs on the same model. A run started from a branch
   (`depot ci run --workflow .depot/workflows/bench.yml`) ends with the `bench-result` artifact: nothing is committed
   or commented.

**Fixed machine spec**: `depot-ubuntu-24.04-8`, 8 vCPU, 32 GB RAM, Linux x86_64, for every run, so numbers are
comparable over time (the `--checkers 16` section: `depot-ubuntu-24.04-16`, 16 vCPU, checked by its own job). Sizing: the largest peak measured is tsgo's default mode on vscode and mui-docs (7.5 GiB on an
18-core Mac, 6.9 GiB on Linux); 32 GB leaves 4x headroom, and 8 vCPUs cover tsrs's default 8 checker threads (tsgo's 4)
plus parallel parsing. The first step fails the job if `nproc`/`MemTotal`/arch differ; there is no fallback runner. (On GitHub's
standard 2 vCPU / 7 GB runner tsgo swapped on vscode: 146 s wall for a 50 s check.)

Caching (Depot Cache serves the `actions/cache` API on Depot CI, no special configuration): `Swatinem/rust-cache` for
`~/.cargo` and the dependency part of `target/`, which holds the PGO-instrumented build (workspace crates are
rebuilt: they are what is being measured). The final PGO build is not cached: its RUSTFLAGS contain the profile's
hash, which changes every run, so cargo rebuilds all of it anyway (as in the release workflow); one
cache for the suite checkout plus the npm-installed compilers (`typescript@7.0.2` and the reference nightly); one cache
per cloned project (checkout + `node_modules`) keyed on its pinned commit and install command
(`bench/run.py --print-cache-keys`), so changing one pin re-installs only that project. Each measuring job restores
the shared cache and its own project's; the build job restores the two training projects (xstate-main, webpack) and
saves the shared cache; `ci.yml`'s `check-and-test` job restores the shared cache and four projects (xstate-main, webpack,
nuxt, drizzle-orm) for the determinism gate, `tools/ci/determinism.sh` (notes/perf-order-independence.md "Gate"). The
build job, and `ci.yml`'s `check-and-test` and `arena-safety` jobs, restore the TypeScript
testdata (`ts-ref/tsc/testdata`, 22 MB compressed) from a cache keyed on the TypeScript commit; on a miss they run the
sparse checkout and save it. That saves about 4 s per job (2026-10-07: checkout 4.9-5.7 s, restore 1.4 s).

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

BOLT (2026-10-07, notes/perf-binary-layout.md) adds the LLVM tarball download, instrumenting tsrs, two training runs
and the rewrite: build job 7 min 21 s -> 7 min 42 s (Depot run `tq2x4vs94n` vs `main` at `0d3b6c08`), build start to
merge end 24 min 27 s -> 24 min 35 s.

Parallel layout, first run on `main` (2026-10-07, commit `ad65e977b0d7`, all caches warm): 13 min 51 s from the push
to the results commit, down from 26 min 35 s for the last sequential run (`bd945842ada5`, measurement 18 min 19 s).

| job | duration |
| --- | ---: |
| `build`: setup and cache restore | 27 s |
| `build`: instrumented build | 4 min 39 s |
| `build`: training run | 26 s |
| `build`: final build, package, upload | 2 min 37 s |
| `measure (vscode)`, the longest; measurement 295 s | 5 min 10 s |
| `merge` | 7 s |

The other measurements, in `partials` of the result: t3code-server 254 s, mui-docs 157 s, supabase-studio 143 s,
formbricks-web 122 s, cal-diy 91 s, webpack 34 s, xstate-main 15 s, Compiler-Unions 8 s, Compiler 5 s. All ten jobs
landed on the same CPU model. Each measuring job adds a runner start-up and cache restore (about 15 s for vscode) to
the runner minutes.

The measuring jobs still start when the build job ends. Starting them with the build and having them wait for its
artifact works on Depot CI (a job can download an artifact that a still-running job uploaded). It was tried on
2026-10-07 (Depot run `tdzf0g0cqn` vs `main`'s `ps_ghllxhtgmq`): measuring began 2-3 s after the build instead of
20 s (vscode) and 50 s (`measure-wide`) after it. But build start to merge end was 20 min 53 s vs 20 min 56 s,
because the 64-vCPU job's 13-minute measurement varies by more than that (762 s vs 779 s). And eleven runners, one of
them 64 vCPU, sat idle through the 7-minute build.

## Verifying a branch

`.depot/workflows/pr-verify.yml` answers the two questions a performance or refactoring branch has to answer before it
lands: does it still print exactly what main prints, and what did it do to time and memory? It runs on one
`depot-ubuntu-24.04-32` machine (32 vCPU; `--input cpus=64` opts into 64) and compares two `cargo build --release` binaries (not the PGO build:
relative comparisons only): the branch, and its merge base with `base` (default `main`), so commits that landed on
main after the branch was cut are not counted as the branch's.

`tools/perf/verify.py` does the measuring (it also runs locally: `tools/perf/verify.py --base <tsrs> --new <tsrs>`).
For every project in `bench/projects.json` and every checker count (default 1, 4, 16, 32), both binaries run
`-p <project> --noEmit --incremental false --extendedDiagnostics --pretty false --checkers N` three times, interleaved.
Per project it also counts the instructions of one single-threaded run of each (`bench/count.py`, as in the
regression flag), and runs the branch once at 16 checkers with `TSRS_ARENA_POISON=1` (freed arena memory is filled
and never reused, so a use after free crashes or changes the output). The result is a table per project (wall time,
peak RSS and check time, base vs branch, and the instruction counts), in the job summary and the `pr-verify` artifact
(`verify-out/verify.{md,json}` plus every run's output).

**What fails it**: any cell where the two binaries' diagnostics differ (every `error TS` line with its continuation
lines, in order) or their exit codes differ, in any run, including the single-threaded and the poisoned runs; or a run
that crashes. Timing and memory never fail it: wall time moves 2-4% between identical runs, and the instruction count
and single-threaded peak RSS are the numbers to quote in a pull request.

How to run it:

```sh
depot ci run --workflow .depot/workflows/pr-verify.yml      # from any branch or dirty worktree, all defaults
depot ci dispatch --repo maschwenk/tsrs --workflow pr-verify.yml --ref <pushed branch> \
  --input projects=vscode,webpack --input checkers=4,32 --input reps=5
```

or open a pull request into main that changes `crates/`: it then runs on every push to that pull request and keeps one
comment with the table up to date. A pull request that changes nothing under `crates/` (a `Cargo.lock` bump, the bench
harness) runs it only with the `verify` label. `depot ci run` takes no inputs (edit the defaults in your working copy
to narrow it), and Depot does not serve `actions/cache` to it (not tied to a ref), so it clones and installs every
project; a dispatched or pull request run restores the caches `bench.yml` saves.

Historical duration on 64 vCPU (10 projects x 4 checker counts x 3 reps, poison on), Depot run `8fp8bp2jk1`
(2026-10-07, `depot ci run`, so no caches): 10 min 44 s for the job, of which setup and project clone + install
2 min 39 s, both release builds (at the same time) 35 s, measurement 7 min 23 s. With the caches (a pull request
with the `verify` label, run `8hc5870p99`): 8 min 42 s, of which cache restores 34 s, builds 35 s, measurement
7 min 21 s; the comment job adds about 10 s.

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

Thread settings (`--threads`, default `default,4,8,16,all`): `default` passes no flag (tsgo uses 4 checker threads,
tsrs half the cores, at least min(cores, 8), at most 32, bun one thread per core); a number N passes `--checkers N` to tsgo and tsrs and
`--threads N` to bun;
`all` is the machine's thread count. The knobs are not identical: `--checkers` sets only the checker threads (parsing
and binding still use every core), while bun's `--threads` caps all of its threads. Per project, one untimed warm-up
per compiler, then `--reps` (default 20) reps in which the compiler order rotates; the table reports mean ± standard
deviation, median and minimum wall time and mean peak RSS, plus whether each compiler's (file, line, column, code)
error list equals 7.1-dev's. bun's file count is its own "checked N files" summary, which is not the program's file
count (13 fewer on `Compiler`, 1,028 fewer on vscode).

```sh
python3 bench/compare.py --tsrs target/release/tsrs --bun ~/.bun/bin/bun --projects vscode --reps 20
python3 bench/compare.py --tsrs ... --bun ... --projects Compiler --threads default,4 --reps 2   # smoke test
```

The Depot CI workflow `.depot/workflows/bench-compare.yml` (manual dispatch only) defaults to `depot-ubuntu-24.04-16`
(16 vCPU; pass `--input cpus=32` or `--input cpus=64` for explicit scaling experiments) with the PGO `dist` build of the commit (built and trained as in `bench.yml`) and Bun canary, and uploads
`bench/results/compare/<date>-<commit>-<threads>t.{json,md}` plus the logs as the `bench-compare` artifact.

First run, 2026-10-06 (Depot run `rnfhvvd79g`, vscode, 20 reps, `bench/results/compare/2026-10-06-2420b7ed410b-64t.md`):
at each compiler's default, bun check (64 threads) 0.85 s / 2.86 GiB, tsrs (8 checkers) 1.51 s / 3.25 GiB, tsgo 7.0.2
9.58 s / 7.51 GiB, tsgo 7.1-dev 7.99 s / 6.85 GiB; at 64 threads tsrs 0.89 s / 4.97 GiB. bun check's 359 errors equal
tsgo 7.0.2's line for line; tsrs's 371 equal 7.1-dev's.
