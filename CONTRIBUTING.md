# Contributing to tsrs

tsrs is a Rust port of the TypeScript 7 type checker (the Go code in microsoft/TypeScript, `tsc/internal`). Bug
reports with a small reproduction are the most useful contribution; fixes are welcome too.

## The fidelity rule: Go is the spec

The Go implementation at the commit in `Cargo.toml` (`[workspace.metadata.typescript]`) is the specification.

- A difference from `tsgo` at that commit (diagnostics, their order, message text, exit code) is a tsrs bug, even
  when the tsrs output looks more reasonable. A fix makes tsrs do what the Go code does, not what seems right.
- Port the Go function you are fixing; keep its name, structure and control flow (`docs/PORTING.md`). Do not
  "improve" the algorithm. Internal-only differences (data layout, caching) are fine when no diagnostic changes.
- If the Go code itself is wrong, report it upstream; tsrs follows it until the pinned commit moves.

## Build

Requirements: rustup (it installs the Rust version `rust-toolchain.toml` pins), Node.js (for `npm/build.mjs`) and git.

```sh
cargo build --release -p tsrs_cli -p tsrs_testrunner
./target/release/tsrs -p path/to/project          # like `tsc` (emits; add --noEmit to only type check)
cargo check --workspace                           # must be 0 errors, 0 warnings (CI uses -D warnings)
cargo check -p tsrs_wasm --target wasm32-wasip1   # the WebAssembly build still compiles (notes/wasm-build.md)
tools/lint/ratchet.py                             # no new clippy findings (docs/RUST.md)
tools/lint/source.py                              # unsafe Send/Sync inventory, comments on weakened atomic orderings
tools/gen-check.sh                                # generated code matches its generator (change the generator, then --write)
```

## Run the tests

The conformance suite reads the reference baselines from a checkout of microsoft/TypeScript at the pinned commit in
`ts-ref/` (git-ignored):

```sh
commit=$(node npm/build.mjs --print-typescript-commit)
git clone --filter=blob:none --sparse https://github.com/microsoft/TypeScript ts-ref
git -C ts-ref sparse-checkout set tsc/testdata && git -C ts-ref checkout "$commit"

./target/release/tsrs-test run --suite all                       # summary table; lists in target/test-results/
./target/release/tsrs-test run --filter <substring>              # one test or a cluster
./target/release/tsrs-test show <suite/name>                     # expected vs actual
.github/scripts/conformance-gate.sh                              # what CI enforces: errors, .types, .symbols
tools/regressions.sh                                             # testdata/regressions (CI runs it too)
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

- External contributors: open a pull request against `main`. CI (`.depot/workflows/ci.yml`, on Depot CI) runs on pushes
  to `main`, on pull requests into `main` (drafts too) and by manual dispatch, and must pass. Depot CI does not run
  pull requests from forks, so those run only the macOS job of `.github/workflows/node-api.yml`, after a
  maintainer's approval, and never get repository secrets.
- Maintainers land small commits directly on `main` (rebase onto `origin/main`, `cargo check --workspace`, re-run the
  affected tests and the full suite, then `git push origin HEAD:main`; never force-push). See `docs/DEBUGGING.md`.
- Commit messages: `<area>: <what and why>`, plus the conformance totals when they change.
- Contributions are licensed under Apache-2.0 (`LICENSE`, section 5).

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). Security issues: see [SECURITY.md](SECURITY.md).
