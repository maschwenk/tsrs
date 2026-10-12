# Top repository conformance

The [GitHub Actions workflow](../../.github/workflows/repo-conformance.yml) compares tsrs with tsgo built from the
TypeScript commit pinned in `Cargo.toml`, following TypeScript's
[`test top400`](https://github.com/microsoft/TypeScript/wiki/Triggering-TypeScript-Bot).
It selects 400 TypeScript repos by stars after compiler changes merge into `main`, and 20 for harness PRs, excluding
forks and archived repos. Manual dispatch accepts 1-400. Both compilers are built once; 20 shards run at most eight at a time.

The plan pins each repo's default-branch commit, and shards check out exactly that commit. Root dependencies are
installed with the declared package manager (`packageManager`, or an exact `devEngines.packageManager`) unless only
another manager's lockfile exists; then that lockfile's manager installs it. npm, pnpm and Yarn come from corepack;
Bun and nub from their per-platform npm packages. Undeclared pnpm versions follow the lockfile format, using pnpm 10
when package.json has a `pnpm` field or a `workspaces` field without `pnpm-workspace.yaml`, which newer pnpm reads
differently. Installs are frozen when a lockfile exists and unpinned (and marked so) when none does. Lifecycle scripts
are disabled and engine and platform checks are ignored, since only the files are needed. Yarn uses node_modules.
Package downloads are cached; installs and compilers receive no CI credentials. Other package managers, out-of-date
lockfiles and custom setup requirements are reported as setup failures.

The harness compares ordered diagnostics, exit codes, and loaded-file counts with `--noEmit` and fresh build info.
It normalizes checkout/compiler paths and CRLF, and excludes timing counters. Tracked `tsconfig.json` files are
checked shallowest first. When neither compiler loads a file for a config, the project references tsgo reports for it
(`--showConfig`) are checked next. Limits per repo: five projects that load files and ten configs in all; 180
seconds for setup (60 for cloning); 120 seconds for checking, with at most 60 seconds per compiler per config. tsrs
is not run when tsgo fails or times out. Timeouts kill child processes too.

Actions reports coverage and retains logs, commits, and diagnostic diffs for 14 days. Diagnostic differences and
tsrs failures after tsgo completes fail the report, as do missing results, results at another commit, or no
completed comparisons. Skipped, empty, and untested projects are excluded from pass counts. Identical output is
not the same as a project without type errors; the report counts both. Emit and build graphs are outside this check.

If a shard is lost (for example, its runner shuts down), dispatch the workflow with `rerun_run_id` set to that
run and `rerun_shards` to the lost shard indexes, within the 14-day artifact window. The rerun reuses that run's
manifest, compiler binaries and harness commit, checks the same repository commits, and reports the rerun shards
together with the earlier run's other shards. Runs whose manifest predates commit pinning cannot be rerun this way.

For a local run, install `corepack@0.36.0`, set `GH_TOKEN` or `GITHUB_TOKEN` for discovery, and build both compilers
at the manifest's commits. A saved manifest can be reused, and `--previous` adds shards from an earlier run:

```sh
python3 tools/repo-conformance/run.py plan --count 20 --shards 1 --out /tmp/top-repos/manifest.json
python3 tools/repo-conformance/run.py run --manifest /tmp/top-repos/manifest.json --shard 0 \
  --tsrs target/release/tsrs --tsgo /path/to/pinned/tsgo --out /tmp/top-repos/results
python3 tools/repo-conformance/run.py report --manifest /tmp/top-repos/manifest.json \
  --results /tmp/top-repos/results --out /tmp/top-repos/report.md
```

Run the harness tests with real compilers:

```sh
TSRS_CONFORMANCE_TSRS="$PWD/target/release/tsrs" TSRS_CONFORMANCE_TSGO=/path/to/pinned/tsgo \
  python3 -m unittest discover -s tools/repo-conformance -p 'test_*.py' -v
```
