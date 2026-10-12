# Top repository conformance

[Top repository conformance](../../.github/workflows/repo-conformance.yml) runs on GitHub-hosted Ubuntu runners
after compiler changes merge into `main`. It selects 400 non-archived, non-fork TypeScript repositories by GitHub
stars, snapshots the list once, and compares tsrs with tsgo built from the exact TypeScript commit in `Cargo.toml`.
This is inspired by TypeScript's [`test top400`](https://github.com/microsoft/TypeScript/wiki/Triggering-TypeScript-Bot)
and [`typescript-error-deltas`](https://github.com/microsoft/typescript-error-deltas).

The workflow builds both compilers once, then spreads the repositories across 20 shards with at most eight shards
running concurrently. Changes to this harness get a 20-repository PR smoke run. Manual dispatch accepts 1-400 repos.
Each compiler-changing main commit has its own run; later merges do not cancel it. Documentation and generated
benchmark commits do not trigger the sweep.

For each repository, the harness clones the default branch once and records its exact commit. It installs root
dependencies using the pinned package manager when declared (npm, pnpm, or Yarn), disables lifecycle scripts, and
uses node_modules for Yarn. Unsupported package managers (including Bun) and projects requiring generation or custom
setup are reported as setup failures. Package-manager configuration may still execute code; installs and compilers receive no CI credentials.
Package downloads are cached, while cloned repositories and installed dependencies are removed after each check.

It checks up to five tracked `tsconfig.json` files, preferring shallower paths, with the same `--noEmit` invocation
on both compilers. Each invocation gets fresh build info, including composite projects. The comparison preserves
diagnostic order, positions, codes, and multiline messages and also checks exit codes and loaded-file counts.
Only checkout/compiler paths and CRLF are normalized; timing and allocation counters are excluded.

Setup gets 180 seconds per repository, with at most 60 seconds for cloning. Checking gets 120 seconds per
repository; each compiler gets the same limit, at most 60 seconds per config. A timeout kills the whole process
group. Shards have a 120-minute limit and save results after each repository, so interrupted runs retain evidence.

The report job publishes a coverage table to the Actions summary and uploads the manifest, repository commits,
raw install/compiler logs, and diagnostic diffs for 14 days. Differences and tsrs crashes/timeouts after the
reference compiler completes fail the report.
Missing shards/results, wrong compiler revisions, and runs with no completed comparison also fail. Setup failures,
reference failures, config limits, and empty projects are explicitly reported and do not count as passes. A green
report means the completed comparisons matched, not that every selected repository was fully checked.

This is a diagnostic comparison on real code, not evidence of emit or build-graph parity. No project-specific
source patches, generated-file workarounds, or accepted diagnostic differences are applied.

To reproduce a run locally, use the saved manifest and binaries built at its two commits:

```sh
python3 tools/repo-conformance/run.py plan --count 20 --shards 1 --out /tmp/top-repos/manifest.json
python3 tools/repo-conformance/run.py run --manifest /tmp/top-repos/manifest.json --shard 0 \
  --tsrs target/release/tsrs --tsgo /path/to/pinned/tsgo --out /tmp/top-repos/results
python3 tools/repo-conformance/run.py report --manifest /tmp/top-repos/manifest.json \
  --results /tmp/top-repos/results --out /tmp/top-repos/report.md
```

Install `corepack@0.36.0` for dependency setup. Discovery accepts `GH_TOKEN` or `GITHUB_TOKEN`; authenticated
requests avoid GitHub's low unauthenticated search limit. Run the harness tests and real compiler controls with:

```sh
TSRS_CONFORMANCE_TSRS="$PWD/target/release/tsrs" TSRS_CONFORMANCE_TSGO=/path/to/pinned/tsgo \
  python3 -m unittest discover -s tools/repo-conformance -p 'test_*.py' -v
```
