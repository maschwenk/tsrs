# tsrs

A Rust port of the TypeScript 7 type checker (the Go implementation in
[microsoft/TypeScript](https://github.com/microsoft/TypeScript), `tsc/internal`, at commit
`b85298b6a81f`). Type checking only: no emit, no language service.

This is a derivative work of TypeScript, which is licensed under Apache-2.0 (see `LICENSE`);
the bundled `lib.*.d.ts` files and the structure of the code come from that project.

- `docs/PORTING.md` — porting conventions
- `docs/AST.md`, `docs/CHECKER.md` — crate contracts
- `crates/` — the port, one crate per Go package group
- `tools/` — generators and Go oracle programs used to compare against the reference implementation
- `npm/` — the npm packages (`@maschwenk/tsrs` + per-platform binaries), versioning and the release workflow

## Usage

```sh
cargo build --release -p tsrs_cli -p tsrs_testrunner
./target/release/tsrs -p path/to/project            # like `tsc --noEmit`; 4 checker threads by default
./target/release/tsrs -p path/to/project --singleThreaded --extendedDiagnostics
./target/release/tsrs-test run --suite all           # TypeScript conformance suite, error baselines
./target/release/tsrs-test run --suite all --baselines types,symbols
```

From npm (once published): `pnpm add -D @maschwenk/tsrs`, then `pnpm exec tsrs -p path/to/project`; see `npm/README.md`.

`docs/STATUS.md` has the current conformance numbers and the comparison against the reference compiler on a
38k-file production project; `docs/DEBUGGING.md` describes the fix workflow and the oracles under `tools/oracle/`.
