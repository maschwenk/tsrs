# perf-cost-cache-default: should the checker cost cache be on by default?

`--checkerCostCache <file>` (notes/perf-balance.md) balances the checkers on per-file check CPU measured by the previous
run. It stays opt-in because it writes a file. Three bench projects are bound by a few files that no static weight finds
(t3code-server, mui-docs; Compiler is bound by checker.ts, which the static weight does find), and the cache is the only
part of tsrs that knows those costs. This note reads what the cache does under work stealing (it predates stealing),
measures what it would give by default, measures its costs, and proposes a policy. A prototype of the policy is on branch
`probe/cost-cache-default`.

Short answer: the cache helps exactly the projects where several costly files share one checker. There it puts each
costly file on its own checker, and the costliest file starts at 0 s instead of 0.6 s. t3code-server: Check time -13% at
16 checkers and -18% at 8. mui-docs: -4% and -19%. The slowest checker's CPU drops 15-24%. On vscode, webpack, Compiler
and Compiler-Unions it changes nothing. The price is extra work where it helps: t3code-server's costly files share an
Effect type graph, and each checker that gets one builds that graph again (+4% instructions and +4% peak at 16
checkers, +17% and +9% at 8). Reads and writes take a few milliseconds. Output was byte-identical in every run.

## 1. What the cache does (main 5d3a17de)

- **Write** (`write_cost_cache`, checkerpool.rs, called at the end of `get_diagnostics_of_any_program`): with more than
  one checker, every checker pass reads the thread CPU clock before and after each file (`file_cpu`). After the passes the
  run writes `# tsrs checker cost cache v2 checkers=<n>` and then `<seconds>\t<checker>\t<path>`, one line per
  type-checked file, sorted by path. It writes a temporary file and renames it, and ignores failures. The key is the
  file's canonical absolute path (lowercased on a case-insensitive file system). The checker is the file's final owner,
  so a stolen file is recorded under its thief.
- **Read** (`locality_associations_with`): only in the locality assignment. It runs when an assignment is named
  `locality` and in the default mode, which steals. The run converts cached seconds to costs. A file without an entry
  costs its static weight times the cache's average nanoseconds per weight unit. Directory groups are formed on those
  costs. If the cache was written at the same checker count, each group starts on the checker that ran most of its cost
  last time and new groups go to the least loaded checker. Then up to 64 single groups move off the slowest checker
  (`refine_group_associations`). At another checker count the groups go through FENNEL on measured costs instead.
- **What it changes:** only the placement, meaning the groups and the checker each group starts on. The pool's
  `weights` are still `checked_file_weights`, so heavy-first ordering (`heavy_files_first`), the choice of which queue
  to steal from, and file splitting (splitcheck.rs) all still use the static weight.
- **Misses:**
  - A missing, unreadable or foreign file has no entry for any of the program's files. The run then uses the plain
    locality placement, so a cold run is the baseline.
  - A new file gets the scaled static weight.
  - An edited file keeps its old cost for one run. There is no content hash or mtime. The next write records the new
    cost, and a stale cost only moves a file.
  - A deleted file is dropped at the next write.
  - A cache from another checker count is still used for costs (measured in section 3).
- **Two gaps found here (fixed in the prototype):**
  - A run that checks only some files (incremental) rewrote the cache with only those files. The next full run then
    priced everything else by static weight.
  - A split declaration file recorded only the piece its owner ran. The cache saw a cheap file, so the next run would not
    split it.
- **Output:** a file's diagnostics do not depend on its checker or the visiting order
  (notes/perf-order-independence.md). The known exceptions are the open history-dependent programs in
  notes/open-history-dependence.md, whose output already depends on timing under stealing. The `--extendedDiagnostics`
  Types, Symbols and Instantiations counters and `Memory used` move with the placement, as they already do run to run
  under stealing.

## 2. Measured gain

Apple M5 Max, 18 cores (6 P + 12 E), 128 GiB, shared with about ten other agents: **load average 27-213 during these
runs** (each row lists its range).

- Binary: the branch's release build. `--noCheckerCostCache` runs main's code path; the cache arms add only the read and
  write.
- Flags: `--noEmit --incremental false --extendedDiagnostics --pretty false --maxMemory 0`, with
  `TSRS_ASSIGNMENT_STATS=times` (per-checker thread CPU) and `TSRS_FILE_TIMES` (start of the costliest file in its
  checker), under `/usr/bin/time -l` (instructions retired, peak footprint).
- Arms:
  - **none:** `--noCheckerCostCache`.
  - **cache:** the cache file is read and rewritten on every run, as a default would do.
  - **+weights:** the cache plus `TSRS_COST_CACHE_WEIGHTS=1`, a new experiment in which heavy-first, stealing and
    splitting use the measured costs.
- Two warm-up runs per cache arm before measuring.
- Default checker count here: 16 (4 for Compiler and Compiler-Unions, which have 41 files).
- Diagnostics: the `--pretty false` output's hash was the same in every run of every project and arm, about 700 runs
  (vscode 371 errors, t3code-server 6, webpack 840, Compiler 43, Compiler-Unions 41, mui-docs 0).

**Position bias.** The first batches alternated the arm order (A B C, then C B A). An A/A test (none, none, cache;
vscode, 8 rounds, load 49-58) put the same `none` arm 28% slower in Check time when it ran in the middle. The table
below therefore rotates positions (every arm runs first, middle and last equally often): 9 rounds, medians. CPU and
instruction counts did not show the bias, so the earlier batches are used for those only (second table).

Rotated, warm cache, medians of 9 (Check s / slowest checker CPU s / slowest / mean checker CPU / instructions G / peak MiB / start of the costliest file):

| project, checkers (load) | none | cache | + weights |
| --- | --- | --- | --- |
| t3code-server, 16 (48-57) | 2.81 / 1.50 / 1.73 / 126.4 / 2,245 / 0.64 s | **2.45 (-13%)** / 1.27 (-15%) / 1.34 / 131.6 (+4.0%) / 2,343 (+4.4%) / 0.07 s | **2.28 (-19%)** / 1.15 (-23%) / 1.21 / 136.4 (+7.9%) / 2,441 (+8.7%) / 0 |
| t3code-server, 8 (52-92) | 1.80 / 1.70 / 1.39 / 89.4 / 1,648 / 0.09 s | **1.48 (-18%)** / 1.43 (-16%) / 1.01 / 104.6 (+17%) / 1,803 (+9.4%) / 0.19 s | 1.51 (-16%) / 1.39 (-18%) / 1.01 / 100.9 (+13%) / 1,797 (+9.0%) / 0 |
| mui-docs, 16 (75-213) | 1.46 / 0.74 / 1.56 / 95.0 / 1,669 / 0.11 s | 1.40 (-4%) / 0.56 (-24%) / 1.15 / 95.0 (0%) / 1,654 (-0.9%) / 0.02 s | 1.37 (-6%) / 0.52 (-30%) / 1.09 / 97.2 (+2.4%) / 1,644 (-1.5%) / 0 |
| mui-docs, 8 (34-46) | 1.17 / 0.94 / 1.19 / 80.3 / 1,353 / 0.07 s | **0.95 (-19%)** / 0.80 (-15%) / 1.02 / 80.0 (-0.3%) / 1,339 (-1.0%) / 0.06 s | 0.90 (-23%) / 0.82 (-13%) / 1.02 / 88.4 (+10%) / 1,373 (+1.5%) / 0 |
| vscode, 16 (102-169) | 1.33 / 0.66 / 1.06 / 97.4 / 2,424 / 0.14 s | 1.33 (0%) / 0.66 (0%) / 1.04 / 97.9 (+0.4%) / 2,427 / 0.15 s | 1.41 (+6%) / 0.68 (+3%) / 1.06 / 98.1 (+0.6%) / 2,449 / 0 |

Alternating order (wall biased against the cache arm, so it is not reported). Medians of 6-12: slowest checker CPU s /
slowest / mean / instructions G / peak MiB:

| project, checkers (load) | none | cache | + weights |
| --- | --- | --- | --- |
| vscode, 8 (43-63) | 1.27 / 1.03 / 93.7 / 2,264 | 1.29 (+1%) / 1.03 / 93.5 / 2,268 | 1.30 (+2%) / 1.03 / 93.4 / 2,264 |
| webpack, 16 (57-61) | 0.090 / 1.16 / 15.7 / 509 | 0.090 / 1.15 / 15.8 (+1%) / 512 | 0.095 / 1.16 / 16.4 (+4.6%) / 534 |
| webpack, 8 (64) | 0.155 / 1.08 / 13.8 / 425 | 0.150 (-3%) / 1.05 / 14.0 (+1%) / 426 | 0.155 / 1.04 / 14.1 / 432 |
| Compiler, 4 (49-50) | 0.060 / 1.33 / 2.2 / 69 | 0.060 / 1.33 / 2.2 / 68 | 0.055 / 1.25 / 2.2 / 69 |
| Compiler, 8 (50) | 0.060 / 2.34 / 2.4 / 82 | 0.060 / 2.40 / 2.4 / 83 | 0.060 / 2.40 / 2.4 / 83 |
| Compiler-Unions, 4 (49) | 0.110 / 1.42 / 4.1 / 73 | 0.105 / 1.45 / 4.0 / 74 | 0.110 / 1.42 / 4.1 / 74 |
| Compiler-Unions, 8 (49) | 0.100 / 2.54 / 4.4 / 86 | 0.100 / 2.50 / 4.4 / 88 | 0.100 / 2.58 / 4.4 / 88 |
| t3code-server, 16, 3 batches pooled (28-90) | 1.42 / 1.68 / 126.1 / 2,236 | 1.20 (-15%) / 1.32 / 132.1 (+4.7%) / 2,353 (+5.2%) | 1.10 (-22%) / 1.22 / 134.8 (+6.9%) / 2,405 (+7.6%) |
| t3code-server, 8, 3 batches pooled (35-66) | 1.81 / 1.43 / 89.7 / 1,652 | 1.65 (-9%) / 1.06 / 108.2 (+21%) / 1,862 (+13%) | 1.42 (-22%) / 1.03 / 101.0 (+13%) / 1,790 (+8%) |

What the rows say:

- **t3code-server: several costly files on one checker.** Seven files cost 0.93-1.08 s of checker CPU each at 16
  checkers (`cli/theme.test.ts`, `cli/project.test.ts`, `src/server.ts`, `cli/pair.test.ts`, `src/bincli.ts`,
  `cli/server.ts`, `cli/app.test.ts`). Together that is half of the 14.2 s check, against a mean of 0.88 s per checker.
  Their static weights are low, so the locality placement puts five of them on one checker (14) and one more next to
  `src/ws.ts` (0.6 s) on checker 6. With stealing, the other checkers take them only after draining their own queues,
  because thieves take from the back. They start at 0.65-0.97 s (two `TSRS_FILE_TIMES` runs), and the pass ends about
  0.5 s after the mean checker. With the cache the measured costs split those groups, each costly file gets its own
  checker and starts first, and the slowest checker comes within 1.3x of the mean at 16 and 1.01x at 8.
- **The extra work is the duplication.** Each checker that runs one of these files builds the Effect type graph they
  share: +5 G instructions at 16 checkers, +15-19 G at 8. Two variants measured on t3code-server (6 rounds, not kept):
  - Leaving the placement alone and only ordering by measured cost (`order`) started the costliest file at 0 but left
    five on one checker: slowest checker CPU +6% at 16, -8% at 8.
  - Moving only the files above a quarter of a checker's share (`spread`) cost as many instructions as the full cache
    (+7% at 16, +25% at 8).
  So no placement runs these files in parallel without the duplicated graph. Sharing it would be a different fix
  (notes/mem-shared-base.md).
- **mui-docs: same shape, no duplication.** Five docs demos cost 0.45-0.51 s each, which is the mean checker's whole
  share at 16 checkers. Isolating them costs no instructions: their types do not overlap.
- **No gain where stealing already balances or the costly file already starts first.**
  - vscode's costliest file now costs 0.10 s (the inference memo cut `mapSessionEvents.test.ts`; notes/perf-heavy-files-infer-memo.md).
    Its slowest checker is 1.03-1.06x the mean in every arm, the perf-heavy-first-threshold.md tails are gone, and
    the 12.5% that notes/perf-balance.md measured was against the static assignment, before stealing.
  - webpack's floor is lib.dom.d.ts (0.056 s), split or first already.
  - Compiler and Compiler-Unions are bound by `checker.ts` (0.06 s and 0.10 s), which is first by static weight and
    starts at 0 in every arm.
- **+weights** gains another 5-8 points of slowest-checker CPU at 16 checkers on t3code-server and mui-docs. Results
  are mixed at 8 (mui +10% instructions). It is kept as an experiment (`TSRS_COST_CACHE_WEIGHTS=1`), not proposed.
- **Wall time on an idle machine was not measured.** On this host the rotated Check time follows the slowest
  checker's CPU on t3code-server and mui-docs at 8. At load 75-213 (mui at 16) the gain shrinks to -4%. The bench
  machines (8- and 64-vCPU Linux) need their own run before a default changes (section 5).

## 3. Costs

- **Read:** 0-5 ms ("Checkers: cost cache read", a new `--extendedDiagnostics` row inside Check time). Measured on
  vscode, mui-docs, t3code-server, webpack and Compiler, 4 runs each.
- **Write:** 1-11 ms after the check ("Checkers: cost cache write", inside Total time), and two thread-CPU clock reads
  per checked file during the check.
- **Size:** about 157 bytes per checked file. vscode 1.48 MB (9,400 lines), mui-docs 1.89 MB (13,101),
  t3code-server 0.23 MB, webpack 0.21 MB, Compiler 4.7 KB. The private monorepo was 5.5 MB in notes/perf-balance.md.
- **Convergence** (slowest checker CPU and instructions over the first reads):
  - t3code-server at 16 gains on the first read: 1.16-1.36 s against about 1.44.
  - At 8 the first read is no better (1.49-2.04 s) and already pays the instructions (+15 G). The gain shows from
    the third or fourth run (1.61-1.71 s).
  - mui-docs gains on the first read at 8 and gradually at 16.
- **Another checker count** (t3code-server, 3 trials each, load 51-77):
  - A 16-checker cache used at 8: first run 1.62-1.75 s against 1.57-1.73 s without, with 113-115 G instructions
    against 89 G (FENNEL on measured costs, the oscillation notes/perf-balance.md describes). The next run reads its
    own 8-checker placement.
  - An 8-checker cache used at 16 helps at once: 1.25-1.34 s against 1.43-1.60, 128-129 G against 124-126 G.
- **Edits:** keyed by path, so an edit costs at most one run with a stale cost for that file. A branch switch keeps
  the costs of the files both branches have.

## 4. Proposal

What tsc and tsgo do: `.tsbuildinfo` exists only with `incremental`, `composite` or `-b`. It goes to
`tsBuildInfoFile`, else `<outDir>/<config name>.tsbuildinfo`, else next to the config file
(`get_build_info_file_name`, outputpaths.rs). It can't hold the costs: tsc would reject the file, and most `--noEmit`
checks don't write one. Comparable tools split both ways:

- Default on: Jest (`$TMPDIR/jest_*`), Vite (`node_modules/.vite`), Next.js (`.next/cache`), Turborepo (`.turbo`),
  Nx (`.nx/cache`), Parcel (`.parcel-cache`).
- Opt-in: ESLint (`--cache`, `.eslintcache`), babel-loader (`cacheDirectory`, `node_modules/.cache/babel-loader`),
  webpack 5's file system cache (`node_modules/.cache/webpack`).

`node_modules/.cache/<tool>` under the nearest package.json is the find-cache-dir convention: always gitignored,
cleared by a reinstall, and inside the directory CI setups already cache.

The policy, as prototyped (execute.rs `default_checker_cost_cache`, checkerpool.rs `set_default_checker_cost_cache`):

- **Where:**
  - If `TSRS_CACHE_DIR` is set, the file goes there (for CI, read-only checkouts and sandboxes).
  - Otherwise it goes to `node_modules/.cache/tsrs/` in the nearest directory at or above the config file that has a
    package.json, if that directory has a `node_modules`.
  - The file name is `checker-costs-<FNV-1a 64 of the absolute config path>.txt`, one per config.
  - With neither, there is no cache: tsrs creates no `node_modules` and writes nowhere else. Compiler and
    Compiler-Unions (no package.json) stay cold.
  - An unwritable directory is skipped silently; the old file, if any, is still read.
- **Opt-out:** `--noCheckerCostCache`, or `TSRS_CHECKER_COST_CACHE=` (empty). `--checkerCostCache <file>` and
  `TSRS_CHECKER_COST_CACHE=<file>` work as before and take precedence.
- **Only in the default scheduling mode:**
  - A named assignment (`--checkerAssignment locality|go|...`, `TSRS_CHECKER_ASSIGNMENT`) keeps the placement a
    function of the program alone, for the oracles and for reproducible counters.
  - The default mode's counters and stolen files already vary from run to run.
  - One checker (`--checkers 1`, `--singleThreaded`, the WebAssembly build) reads and writes nothing, as before.
- **Commands:**
  - Single-project CLI runs only.
  - `--build` keeps its 4 checkers per project and no default: the path is one per process today and would have to
    move to each project's pool.
  - `--watch` keeps its first placement across rebuilds (`inherited_associations`), so a cache would only be written.
    It gets no default.
  - The language server has its own checkers and no pool state; unchanged.
- **Incremental runs:** the write keeps the previous entries of checked files the run did not check.
- **CI:** a runner without a restored `node_modules/.cache` runs cold, which is today's behaviour. Restoring a stale
  cache only moves files. Keys are absolute paths, so a runner that checks out to a different path misses entirely;
  config-relative keys would fix that. GitHub's runners use a stable workspace path.
- **Exactness:** output unchanged by construction and in every run above. The counters move, as they already do.

Status: the prototype passes its test (crates/tsrs_cli/tests/cost_cache_default.rs, which covers the default file, a
second run's identical output, opt-out, named assignment, no `node_modules` and `TSRS_CACHE_DIR`) and the rest of
`cargo test --release -p tsrs_cli`. One failure, `npm_lib_path` (`/private/var` against `/var` on macOS), fails on
pristine main too. The checkerpool unit tests pass. It is not proposed for main yet (section 5).

## 5. Risks and what would decide it

- **Memory and work where it helps.** t3code-server +4.4% peak at 16 checkers and +9.4% at 8, with +4% and +17%
  instructions. t3code-server is the project where tsrs already loses to bun on memory (notes/mem-round4.md).
  mui-docs pays nothing.
- **Unverified on the bench hardware.** At 32 checkers on the 64-vCPU runner, t3code-server's check was 1.25 s
  against a one-file floor near 1.1 s, so less is left there than on this Mac.
  - What would decide it: a perf-probe run of t3code-server, mui-docs, supabase-studio and vscode at 8, 16 and 32
    checkers, warm cache against `--noCheckerCostCache`.
  - Flip the default if two projects gain at least 2% wall with peak memory within 5%.
- **The bench changes meaning.** With a default cache, `bench/run.py`'s second and later reps read the first rep's
  cache. The regression table should pass `--noCheckerCostCache` (cold, today's numbers), with a warm row if wanted.
  The tsrs_cli tests and tools that run the CLI in a tree with `node_modules` would write there (harmless; they pass
  either way).
- **History-dependent programs** (TanStack router, sequelize, rxjs, dify): their output already depends on timing
  under stealing. A cache makes a cold and a warm run differ more systematically. This is not a new kind of
  nondeterminism, but more visible.
- **Edge cases:**
  - A cache from another checker count costs one run (+29% instructions on t3code-server going from 16 to 8).
  - Stale files of old configs stay in `node_modules/.cache/tsrs` until a reinstall.
  - Yarn PnP projects (no `node_modules`) get no cache.
  - Concurrent runs of one config: the last writer wins; the rename keeps the file whole.

## Reproduce

```sh
B=target/release/tsrs   # this branch
F="--noEmit --incremental false --extendedDiagnostics --pretty false --maxMemory 0"
TSRS_ASSIGNMENT_STATS=times $B -p apps/server $F --noCheckerCostCache                         # none
TSRS_ASSIGNMENT_STATS=times $B -p apps/server $F --checkerCostCache /tmp/t3.cache              # cache (run 3+ times)
TSRS_COST_CACHE_WEIGHTS=1 TSRS_ASSIGNMENT_STATS=times $B -p apps/server $F --checkerCostCache /tmp/t3w.cache
TSRS_FILE_TIMES=/tmp/ft.tsv $B -p apps/server $F --noCheckerCostCache   # per-file checker, wall, CPU in completion order
```

Rotate the arms' positions between rounds (an A/A test showed 28% between positions on this host) and read the
`checker group cpu seconds` row alongside Check time.
