# Top repository conformance

The [GitHub Actions workflow](../../.github/workflows/repo-conformance.yml) compares tsrs with tsgo built from the
TypeScript commit pinned in `Cargo.toml`, following TypeScript's
[`test top400`](https://github.com/microsoft/TypeScript/wiki/Triggering-TypeScript-Bot).
It selects 400 TypeScript repos by stars after compiler changes merge into `main`, and 20 for harness PRs, excluding
forks and archived repos. Manual dispatch accepts 1-400. Both compilers are built once; 20 shards run at most eight at a time.

Each repo is cloned once and its commit recorded. Root dependencies are installed with npm, pnpm, or Yarn, using
the declared version and disabling lifecycle scripts. Yarn uses node_modules. Package downloads are cached;
installs and compilers receive no CI credentials. Unsupported package managers or custom setup requirements are
reported as setup failures.

The harness compares ordered diagnostics, exit codes, and loaded-file counts with `--noEmit` and fresh build info.
It normalizes checkout/compiler paths and CRLF, and excludes timing counters. Limits per repo: five tracked
`tsconfig.json` files, shallowest first; 180 seconds for setup (60 for cloning); 120 seconds for checking, with
at most 60 seconds per compiler per config. Timeouts kill child processes too.

Actions reports coverage and retains logs, commits, and diagnostic diffs for 14 days. Diagnostic differences and
tsrs failures after tsgo completes fail the report, as do missing results or no completed comparisons. Skipped,
empty, and untested projects are excluded from pass counts. Emit and build graphs are outside this check.

For a local run, install `corepack@0.36.0`, set `GH_TOKEN` or `GITHUB_TOKEN` for discovery, and build both compilers
at the manifest's commits. A saved manifest can be reused:

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
