# Contributing to tsrs

tsrs is a Rust port of the TypeScript 7 type checker (the Go code in microsoft/TypeScript, `tsc/internal`).

Issues and pull requests are currently disabled until we have the bandwidth to triage them. This guide covers
building from source, reproducing bugs and the checks required for changes. When contributions reopen, a small
reproduction will be the most useful starting point for a bug report.

The port was written largely by AI coding agents, directed and reviewed by Max Schwenk. Its correctness rests on
the [conformance results](docs/COMPATIBILITY.md#conformance) rather than on review alone.

## The fidelity rule: Go is the spec

The Go implementation at the commit in `Cargo.toml` (`[workspace.metadata.typescript]`) is the specification.

- A difference from `tsgo` at that commit (diagnostics, their order, message text, exit code) is a tsrs bug, even
  when the tsrs output looks more reasonable. A fix makes tsrs do what the Go code does, not what seems right.
- Port the Go function you are fixing; keep its name, structure and control flow (`docs/PORTING.md`). Do not
  "improve" the algorithm. Internal-only differences (data layout, caching) are fine when no diagnostic changes.
- If the Go code itself is wrong, report it upstream; tsrs follows it until the pinned commit moves.

## Build

Requirements: rustup (it installs the Rust version `rust-toolchain.toml` pins), Node.js 24 (`.node-version`), pnpm 12.10.1 (for `npm/build.mjs` and the JS tooling) and git.
Run `pnpm install --frozen-lockfile` from the repository root to install the npm workspace tooling.

```sh
cargo build --profile fast -p tsrs_cli -p tsrs_testrunner
./target/fast/tsrs -p path/to/project             # like `tsc` (emits; add --noEmit to only type check)
cargo check --workspace                           # must be 0 errors, 0 warnings (CI sets build.warnings=deny)
cargo check -p tsrs_wasm --target wasm32-wasip1   # the WebAssembly build still compiles (notes/wasm-build.md)
cargo clippy --workspace --locked --exclude tsrs_fourslash --config 'build.warnings="deny"' -- -D dead-code
tools/lint/source.py                              # unsafe Send/Sync inventory, comments on weakened atomic orderings
tools/gen-check.sh                                # generated code matches its generator (change the generator, then --write)
```

Use the `fast` profile (release optimization, no LTO, incremental) while editing and testing: a checker edit rebuilds
`tsrs` and `tsrs-test` in about 5 s instead of about 3 minutes with `--release`, and the conformance suite gives the
same results in the same time (notes/dev-loop-build.md). Build `--release` for performance measurements: its fat LTO
and single codegen unit are what ships.

The language-server test runner and WebAssembly module have separate builds:

```sh
cargo build --profile fast -p tsrs_fourslash
tools/wasm/build.sh  # requires the wasm32-wasip1 target and wasm-opt
```

## Run the tests

The conformance suite reads the reference baselines from a checkout of microsoft/TypeScript at the pinned commit in
`ts-ref/` (git-ignored):

```sh
commit=$(node npm/build.mjs --print-typescript-commit)
git clone --filter=blob:none --sparse https://github.com/microsoft/TypeScript ts-ref
git -C ts-ref sparse-checkout set tsc/testdata && git -C ts-ref checkout "$commit"

./target/fast/tsrs-test run --suite all                          # summary table; lists in target/test-results/
./target/fast/tsrs-test run --filter <substring>                 # one test or a cluster
./target/fast/tsrs-test show <suite/name>                        # expected vs actual
./target/fast/tsrs-test run --suite all --baselines types,symbols # type and symbol baselines
./target/fast/tsrs-fourslash run                                # language-server suite
TSRS_TEST=target/fast/tsrs-test .github/scripts/conformance-gate.sh # what CI enforces: errors, .types, .symbols
tools/regressions.sh target/fast/tsrs                            # testdata/regressions (CI runs it too)
cargo test -p tsrs_core -p tsrs_scanner -p tsrs_cli              # unit and CLI tests (see .depot/workflows/ci.yml)
```

CI runs the gate twice: in tsgo's check history, which the baselines need, and in the default mode
(`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`, where `conformance/objectLiteralNormalization`'s
`.types` / `.symbols` differ by design).

A change must not lose passing tests: compare `target/test-results/pass.txt` before and after
(`comm -23 <(sort before.txt) <(sort target/test-results/pass.txt)` must print nothing). `docs/DEBUGGING.md`
describes the full workflow, the opt-out mode (`TSRS_LAZY_MEMBERS=0`) that must stay reference-identical, and the
Go oracle programs in `tools/oracle/`.

For a bug outside the suite, add a reproduction under `testdata/regressions/<name>/` (`a.ts`, `tsconfig.json`, and
`expected.txt` with `tsc --pretty false` output from the reference compiler).

## Landing changes

- Changes target `main`. CI (`.depot/workflows/ci.yml`, on Depot CI) runs on pushes
  to `main`, on pull requests into `main` (drafts too) and by manual dispatch, and must pass. Depot CI does not run
  pull requests from forks, so those run only the macOS job of `.github/workflows/node-api.yml`, after a
  maintainer's approval, and never get repository secrets.
- Maintainers land small commits directly on `main` (rebase onto `origin/main`, `cargo check --workspace`, re-run the
  affected tests and the full suite, then `git push origin HEAD:main`; never force-push). See `docs/DEBUGGING.md`.
- Commit messages: `<area>: <what and why>`, plus the conformance totals when they change.
- Contributions are licensed under Apache-2.0 (`LICENSE`, section 5).

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). Security issues: see [SECURITY.md](SECURITY.md).
