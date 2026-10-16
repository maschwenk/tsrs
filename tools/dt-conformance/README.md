# DefinitelyTyped conformance

This suite follows TypeScript's [`@typescript-bot run dt`](https://github.com/microsoft/DefinitelyTyped-tools/blob/main/.azure-pipelines/typescript-pr-dt.yml)
test path. It runs `@definitelytyped/dtslint-runner --selection all --expectOnly` on every package and each package's
declared `tsconfigs` matrix, once with the TypeScript commit pinned in `Cargo.toml` and once with tsrs. The full suite
runs on relevant pull requests and nightly in [Depot CI](../../.depot/workflows/dt-conformance.yml).

The comparison is differential: DefinitelyTyped does not have to be globally clean, but the two runs must finish the
same packages and produce exactly the same per-package failure text. The harness disables dtslint-runner's reporting
exception for its upstream expected-failure list so differences inside those packages remain visible. npm/ATTW and
general lint checks are intentionally skipped; `--expectOnly` still checks compiler diagnostics and `$ExpectType`.

One container holds the checkout, script-disabled install with DT's pinned pnpm version and both compiler runs. DT
intentionally disables its lockfile, as in TypeScript's own DT pipeline; sharing one installed tree keeps the
differential comparison consistent. dtslint may use child processes inside that container, but packages are not
split across containers or machines. The container always runs as a non-root user (including on a root-hosted CI
runner) with a read-only root, all capabilities dropped, no privilege escalation, no credentials or Docker socket,
and only read-only compiler/package inputs. Its private scratch directory is deleted after outputs are copied without
following symlinks.

For a local run, first prepare the same inputs as CI: a pinned TypeScript checkout with `packages/typescript` built and
`built/local/tsc`, plus staged `tsrs` and `tsrs-linux-x64-gnu` directories from `npm/build.mjs`. Then run:

```sh
docker build -t tsrs-dt-conformance tools/dt-conformance
python3 tools/dt-conformance/run.py plan --out /tmp/dt-conformance/manifest.json
python3 tools/dt-conformance/run.py run \
  --manifest /tmp/dt-conformance/manifest.json \
  --typescript /path/to/TypeScript \
  --candidate-package /path/to/staged/tsrs \
  --candidate-platform /path/to/staged/tsrs-linux-x64-gnu \
  --out /tmp/dt-conformance/results --scratch /tmp/dt-conformance/scratch
```

`report.md`, normalized and raw failure JSON, full logs, the external revisions and the upstream expected-failure
list are retained by CI for 14 days. DefinitelyTyped is excluded from the separate
[top-repository suite](../repo-conformance/README.md), which still selects 400 other repositories.
