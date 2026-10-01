# npm packaging

tsrs ships on npm the way TypeScript 7 ships its native compiler (`typescript@7` / `@typescript/typescript-<os>-<arch>`):

| package | contents |
| --- | --- |
| `@maschwenk/tsrs` | `bin/tsrs` (Node launcher), `lib/` (binary lookup, `version.cjs`), `optionalDependencies` on every platform package |
| `@maschwenk/tsrs-<os>-<arch>` | the `tsrs` binary for one platform, with `os`/`cpu` (and `libc: glibc` on Linux) so package managers install only the matching one |

Platforms: `darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64` (glibc), `win32-x64` (best effort).

The package name is set in one place, `npm/tsrs/package.json`; platform packages are always `<name>-<os>-<cpu>`, and
the launcher derives that name from its own `package.json` at runtime.

## How the launcher finds the binary

`bin/tsrs` -> `lib/tsrs.js` -> `lib/getExePath.js`:

1. `TSRS_BINARY` set: run that file (for local builds: `TSRS_BINARY=$PWD/target/release/tsrs pnpm exec tsrs ...`).
2. Otherwise resolve `<name>-<process.platform>-<process.arch>/package.json` from the launcher's own location (with
   pnpm that is the sibling link in `node_modules/.pnpm/<main>/node_modules/`), and run `tsrs`/`tsrs.exe` next to it.
   If that fails, exit 1 with a message naming the missing optional dependency (or listing the supported platforms).
3. Like TypeScript's `tsc`, on Node >= 22.15 the launcher `execve`s the binary (no Node process left, signals and the
   exit code are the binary's own). On older Node and on Windows it spawns the binary, forwards SIGINT/SIGTERM/SIGHUP/
   SIGQUIT/SIGBREAK, and exits with the binary's code or re-raises its terminating signal.

`require("@maschwenk/tsrs")` returns `{ version, typescriptVersion, typescriptCommit }` (TypeScript's
`lib/version.cjs` pattern).

## Versions

One source of truth, the workspace `Cargo.toml`:

```toml
[workspace.package]
version = "0.1.0"

[workspace.metadata.typescript]
version = "7.1.0-dev.20260929"                         # the TypeScript version the port follows
commit = "b85298b6a81f772d080b0455de0ca9d744cd6fd6"    # and its commit (CI checks out its tsc/testdata)
```

npm version: `<version>-ts<typescript version>` = `0.1.0-ts7.1.0-dev.20260929`. `tsrs --version` prints the same,
embedded at build time by `crates/tsrs_cli/build.rs`:

```
Version 7.1.0-dev (tsrs 0.1.0-ts7.1.0-dev.20260929, microsoft/TypeScript@b85298b6a81f)
```

To release a fix on the same TypeScript commit, bump `[workspace.package] version`; after porting a newer TypeScript
commit, update `[workspace.metadata.typescript]`.

## Building packages locally

```sh
cargo build --release -p tsrs_cli
node npm/build.mjs --binary aarch64-apple-darwin=target/release/tsrs --pack
# -> npm/dist/maschwenk-tsrs-darwin-arm64-<version>.tgz, npm/dist/maschwenk-tsrs-<version>.tgz, npm/dist/packages.json
```

`--binary <triple>=<path>` can repeat; `--artifacts <dir>` picks up `<dir>/<triple>/tsrs[.exe]` (the release
workflow's layout). The main package lists exactly the platforms that were given, so a release never depends on a
platform package that was not published. `node npm/build.mjs --print-version` prints the version.

To try local tarballs in a pnpm project, the simplest route is to depend on both directly:

```sh
pnpm add -D @maschwenk/tsrs@file:/abs/npm/dist/maschwenk-tsrs-<version>.tgz \
            @maschwenk/tsrs-darwin-arm64@file:/abs/npm/dist/maschwenk-tsrs-darwin-arm64-<version>.tgz
```

(The launcher then finds the platform package at the project root. A pnpm `overrides` entry pointing
`@maschwenk/tsrs-darwin-arm64` at the tarball reproduces the registry layout exactly, but changing `overrides` makes
pnpm re-resolve the whole lockfile.)

## Releasing

`.github/workflows/release.yml` runs on a pushed tag `v<workspace version>` or `v<npm version>` (e.g. `v0.1.0`):

1. checks the tag against `Cargo.toml`;
2. builds release binaries: macOS arm64 and x64 (cross-compiled on the arm64 runner), Linux x64 (`ubuntu-22.04`) and
   arm64 (`ubuntu-22.04-arm`), Windows x64 (allowed to fail; the main package then omits `win32-x64`), and smoke-runs
   each native one (`--version`, exit code 2 on a type error);
3. gates on the conformance suite on Linux x64 (`.github/scripts/conformance-gate.sh`: at least 13,458 pass, no crash or
   timeout; raise `MIN_PASS` there when fixes land);
4. assembles and packs with `npm/build.mjs`, uploads the tarballs as the `npm-packages` artifact, and publishes the
   platform packages, then the main package, with `--access public --tag latest` (skipping any already on the
   registry, so a failed run can be re-run).

`workflow_dispatch` runs the same with `npm publish --dry-run` by default.

What Max must configure before the first release:

- Repository secret `NPM_TOKEN` (Settings -> Secrets and variables -> Actions): an npm granular access token with
  read and write access to the packages (or an automation token) for an account that can publish to the scope.
- The scope must exist: `@maschwenk` is the npm user scope of an npm account named `maschwenk`. Otherwise create an npm
  org, or change `name` in `npm/tsrs/package.json`.
- Provenance: npm only accepts `--provenance` from public repositories. The workflow adds `--provenance` automatically
  when the repository is public and omits it while it is private.
- Linux binaries are built on Ubuntu 22.04, so they need glibc >= 2.35 at most (a build on Debian 12 needed 2.34);
  Alpine/musl is not supported.

`.github/workflows/ci.yml` (push to main, pull requests): `cargo check --workspace` with warnings denied, `cargo test` for
the fast crates, a release build, the conformance gate, and an npm install smoke test of the packed tarballs.

## Adopting it in the private monorepo (owner/owner)

After the first release is on npm (version below as an example):

```sh
# 1. Catalog entry, and an exemption from the repo's 7-day minimumReleaseAge (or wait a week).
#    In pnpm-workspace.yaml:
#      catalog:
#        '@maschwenk/tsrs': '0.1.0-ts7.1.0-dev.20260929'
#      minimumReleaseAgeExclude:
#        - '@maschwenk/tsrs@0.1.0-ts7.1.0-dev.20260929'
#        - '@maschwenk/tsrs-darwin-arm64@0.1.0-ts7.1.0-dev.20260929'
#        - '@maschwenk/tsrs-darwin-x64@0.1.0-ts7.1.0-dev.20260929'
#        - '@maschwenk/tsrs-linux-x64@0.1.0-ts7.1.0-dev.20260929'
#        - '@maschwenk/tsrs-linux-arm64@0.1.0-ts7.1.0-dev.20260929'
#        - '@maschwenk/tsrs-win32-x64@0.1.0-ts7.1.0-dev.20260929'

# 2. Project devDependency: add `"@maschwenk/tsrs": "catalog:"` to devDependencies in apps/project/package.json
#    (next to "@typescript/native"), then:
pnpm install
#    In a disposable clone, any non-frozen `pnpm add`/`pnpm install` also re-resolved unrelated entries
#    (`@datadog/datadog-ci` 5.4.0 -> 5.24.1, `@ai-sdk/anthropic`'s zod peer); review the pnpm-lock.yaml diff.

# 3. Run it.
pnpm exec tsrs --version
pnpm exec tsrs -p apps/project --extendedDiagnostics
(cd apps/project && pnpm exec tsrs -p .)

# 4. Switch the Project typecheck script from tsc to tsrs (the four command lines; flags are the same).
perl -pi -e 's/\btsc (?=--version|\$single)/tsrs /' apps/project/scripts/typecheck.sh
pnpm --filter project typecheck
```

`typecheck.sh` passes `--singleThreaded` by default (TSC_SINGLE_THREADED=0 to drop it); tsrs runs 4 checker threads
without it, like tsgo. `GOMEMLIMIT` has no effect on tsrs. `--extendedDiagnostics` prints the same counters as tsgo
(`Memory used` is the process RSS), so `ci/emit-ts-diagnostics-measures.mts` keeps working.
