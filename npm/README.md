# npm packaging

tsrs ships on npm the way TypeScript 7 ships its native compiler (`typescript@7` / `@typescript/typescript-<os>-<arch>`):

| package | contents |
| --- | --- |
| `@maschwenk/tsrs` | `bin/tsrs` (Node launcher), `lib/` (binary lookup, `version.cjs`), `dist/` + `vendor/` (the `unstable/*` JS API, see below), `optionalDependencies` on every platform package |
| `@maschwenk/tsrs-<os>-<arch>` | the `tsrs` binary for one platform, with `os`/`cpu` (and `libc: glibc` on Linux) so package managers install only the matching one |

Platforms: `darwin-arm64`, `linux-x64` and `linux-arm64` (glibc; `darwin-x64` was published up to 0.2.1). `npm/build.mjs` and the launcher
also know `win32-x64` (see TODO).

The package name is set in one place, `npm/tsrs/package.json`; platform packages are always `<name>-<os>-<cpu>`, and
the launcher derives that name from its own `package.json` at runtime.

## How the launcher finds the binary

`bin/tsrs` -> `lib/tsrs.js` -> `lib/getExePath.js`:

(`bin/tsrs` is a CommonJS stub, via `bin/package.json`, that imports the ESM launcher: Node 16 cannot run an
extensionless ES module in a `"type": "module"` package. `npm/sdk/smoke-consumer.mjs --node <path>` checks the
launcher and the JS API on each given Node; 16.20.0, 18, 20, 22 and 24 pass.)

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
embedded at build time by `crates/tsrs_execute/build.rs`:

```
Version 7.1.0-dev (tsrs 0.1.0-ts7.1.0-dev.20260929, microsoft/TypeScript@b85298b6a81f)
```

To release a fix on the same TypeScript commit, bump `[workspace.package] version`; after porting a newer TypeScript
commit, update `[workspace.metadata.typescript]`.

## Building packages locally

```sh
cargo build --release -p tsrs_cli
npm ci --prefix npm     # build-only compiler for the JS API
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

## JS API (`unstable/*` exports)

`npm/tsrs/src`, `npm/tsrs/vendor` (vscode-jsonrpc, MIT, with its license) and `npm/tsrs/test` are TypeScript 7's JS API
package (`packages/typescript` in microsoft/TypeScript) at the pinned commit, copied by `npm/sdk/sync-upstream.mjs`
(byte for byte, except a few listed test edits; `npm/tsrs/UPSTREAM.json` holds the commit and sha256 of every file).
After bumping `[workspace.metadata.typescript]`, check out that commit as `ts-ref` and rerun it;
`node npm/sdk/sync-upstream.mjs --check` fails if the copy drifted.

```sh
npm ci --prefix npm                        # build-only tools: typescript@7 (compiler), @types/node, tinybench
node npm/build.mjs --sdk-only              # npm/tsrs/src -> npm/tsrs/dist (also part of every package build)
cd npm/tsrs && TSRS_BINARY=/path/to/server node --conditions @typescript/source --test 'test/**/*.test.ts'
node npm/sdk/smoke-consumer.mjs            # after build.mjs --pack: offline install into a temp consumer, typecheck, sync+async compile
node npm/sdk/method-coverage.mjs --server tsgo=/path/to/pinned/tsgo --server tsrs=target/release/tsrs  # -> npm/sdk/METHODS.md
```

The tests and the inventory can run against a tsgo built from the pinned commit (`cd ts-ref/tsc && go build -o
/tmp/tsgo ./cmd/tsc`) as the reference server. The SDK spawns `getExePath()` (so `TSRS_BINARY` applies) unless the
caller passes `tsserverPath`.

Deliberate patches (the only differences from the pinned sources; each patch file explains itself, and
`sync-upstream.mjs` applies them strictly and records the original sha256 in `UPSTREAM.json` `patchedFiles`):

- `npm/sdk/patches/async-client-connection-loss.patch` (`src/api/async/client.ts`): upstream, if the server dies or
  closes the connection while the async client is open, in-flight and queued requests never settle. Now every
  pending and later request rejects with an `Error` whose message starts with `API server connection lost: `.
  A normal `close()` is unchanged; `close()` after a crash rejects promptly with that error if it had snapshots to
  release.
- `npm/sdk/patches/vscode-jsonrpc-send-request-write-error.patch` (vendored vscode-jsonrpc 9.0.2): a request written
  to a dead server also became an unhandled rejection that terminated Node; the request's own rejection is kept.

The sync client is unpatched: after a server crash its next call throws at once with the raw pipe error.
`npm/sdk/smoke/consumer.mjs` kills the real server with work in flight to cover all of this (the unpatched
package hangs those requests and crashes on the unhandled rejection).

Wire contract the server has to speak (from the pinned `tsc/cmd/tsc/api.go`, `tsc/internal/ipc`, `src/api/options.ts`):

- argv: `--api [--async] --cwd <dir> --useCaseSensitiveFileNames=<bool> [--callbacks=<names>] [--timing]
  [--runExternalCode] [--pipe <path>]` (`--pipe` only on Windows for the sync client). stdout carries only protocol
  bytes; stderr is inherited.
- sync: every message is a MessagePack 3-array `[uint8 type, bin method, bin payload]`; types 1 request, 2 callback
  response, 3 callback error (client to server), 4 response, 5 error, 6 callback call (server to client). Payloads are
  JSON text, except binary responses (`getSourceFile` and friends: the AST encoding, protocol version 9 in the top
  byte of the metadata word).
- async: JSON-RPC 2.0 with `Content-Length` framing; filesystem callbacks are server-to-client requests.
- handshake: the first request is `initialize` with params `null`; the result is
  `{"useCaseSensitiveFileNames": <bool>, "currentDirectory": "<cwd>"}`. There is no version negotiation: client and
  server must come from the same commit.

## Releasing

`.github/workflows/release.yml` runs on a pushed tag `v<workspace version>` or `v<npm version>` (e.g. `v0.1.0`):

1. checks the tag against `Cargo.toml`, and runs `cargo check --workspace`;
2. builds release binaries for macOS arm64 and Linux x64 and arm64
   (`ubuntu-22.04`), and smoke-runs the native ones (`--version`, exit code 2 on a type error);
3. assembles and packs with `npm/build.mjs`, uploads the tarballs as the `npm-packages` artifact, and publishes the
   platform packages, then the main package, with `--access public --tag latest` (skipping any already on the
   registry, so a failed run can be re-run).

The conformance suite does not gate releases; it runs in `ci.yml`.

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

`.depot/workflows/ci.yml` (Depot CI; push to main and manual dispatch): `cargo check --workspace` with warnings denied,
`cargo test` for the fast crates, a release build, the conformance and fourslash gates, an npm install smoke test of
the packed tarballs, and the lint ratchet.

## Adopting it in a pnpm workspace

After a release is on npm (version below as an example):

```sh
# 1. Add it next to the TypeScript compiler. If the workspace sets pnpm's minimumReleaseAge, either wait or list
#    the main package and the platform packages in minimumReleaseAgeExclude.
pnpm add -D @maschwenk/tsrs@0.1.0-ts7.1.0-dev.20260929
#    Review the pnpm-lock.yaml diff: a non-frozen install can re-resolve unrelated entries.

# 2. Run it.
pnpm exec tsrs --version
pnpm exec tsrs -p path/to/project --noEmit --extendedDiagnostics

# 3. Switch a tsc script to tsrs: the flags are the same. Like tsc, tsrs emits unless the options say otherwise
#    (`--noEmit` for a typecheck-only script).
```

tsrs runs 4 checker threads by default, like tsgo (`--singleThreaded` for one). `GOMEMLIMIT` has no effect on tsrs.
`--extendedDiagnostics` prints the same counters as tsgo (`Memory used` is the process RSS), so scripts that parse
them keep working.

## TODO

- win32-x64: uncomment its matrix entry; Windows has never been built or run.
- Make the conformance suite a release gate again (`.github/scripts/conformance-gate.sh`, already green in `ci.yml`
  with a 120 s per-test timeout).
- Provenance: needs the repository to be public (or switch to npm trusted publishing).
- Optionally target an older glibc for Linux (e.g. `cargo zigbuild --target x86_64-unknown-linux-gnu.2.17`).
