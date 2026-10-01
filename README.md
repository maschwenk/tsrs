<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), with tsgo 7.0.2 (npm `typescript@7.0.2`) and with tsrs at commit `9e43575b3e46`: `tsc -p <project> --noEmit`, median of 3 interleaved runs, on Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor).

**Default mode: 4 checker threads in both (tsrs also resolves members lazily, its default)**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 12.29 | 4.21 | 2.92x | 7.79 GiB | 3.29 GiB | 0.42x |
| xstate-main | 0 / 0 | 0.96 | 0.31 | 3.14x | 751 MiB | 508 MiB | 0.68x |
| webpack | 848 / 840 (ref 840) | 1.47 | 0.59 | 2.47x | 1.21 GiB | 708 MiB | 0.57x |
| mui-docs | 0 / 0 | 14.55 | 2.29 | 6.36x | 8.27 GiB | 1.50 GiB | 0.18x |
| Compiler | 43 / 43 | 0.30 | 0.10 | 2.87x | 221 MiB | 209 MiB | 0.95x |
| Compiler-Unions | 41 / 41 | 0.55 | 0.20 | 2.76x | 247 MiB | 219 MiB | 0.89x |

**`--singleThreaded`: one checker thread in both**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 32.34 | 14.09 | 2.30x | 6.54 GiB | 2.79 GiB | 0.43x |
| xstate-main | 0 / 0 | 1.81 | 0.80 | 2.27x | 611 MiB | 313 MiB | 0.51x |
| webpack | 848 / 840 (ref 840) | 3.17 | 1.46 | 2.17x | 957 MiB | 466 MiB | 0.49x |
| mui-docs | 0 / 0 | 16.15 | 4.51 | 3.58x | 3.49 GiB | 1.04 GiB | 0.30x |
| Compiler | 43 / 43 | 0.51 | 0.24 | 2.16x | 177 MiB | 107 MiB | 0.60x |
| Compiler-Unions | 41 / 41 | 0.87 | 0.45 | 1.92x | 205 MiB | 101 MiB | 0.49x |

errors: the number of type errors each compiler reports on the project; they must be equal (a bold cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster). peak memory: maximum resident set size. memory, tsrs / tsgo: below 1 = tsrs uses less.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Date: 2026-10-01 13:47 UTC. tsrs commit: `9e43575b3e46`. Numbers from shared CI machines are noisy; compare trends, not single runs. How it is measured: [`bench/README.md`](bench/README.md).
<!-- bench:end -->

# tsrs

A Rust port of the TypeScript 7 type checker (the Go implementation in
[microsoft/TypeScript](https://github.com/microsoft/TypeScript), `tsc/internal`, at commit
`b85298b6a81f`). Type checking only: no emit, no language service.

This is a derivative work of TypeScript, which is licensed under Apache-2.0 (see `LICENSE`);
the bundled `lib.*.d.ts` files and the structure of the code come from that project.

## Provenance

- **Derivative work.** tsrs is a port of microsoft/TypeScript (Copyright Microsoft Corporation), licensed under
  Apache-2.0 like the original (`LICENSE`, `NOTICE`). The bundled `lib.*.d.ts` files are TypeScript's, unchanged, and
  the structure and names of the code follow the Go sources. This project is not affiliated with
  or endorsed by Microsoft.
- **The Go implementation is the specification.** Functions correspond one to one to Go functions at the pinned commit
  (`Cargo.toml` `[workspace.metadata.typescript]`); behavior that differs from Go is a bug, including message text
  and diagnostic order. The only intended deviations are internal (memory layout, lazy member resolution that can be
  switched off with `--noLazyMembers`), and they must not change any diagnostic.
- **How it was written.** The port was produced with heavy use of AI coding agents, directed and reviewed by Max
  Schwenk. Correctness is established by evidence rather than by review alone: the TypeScript conformance suite's
  reference baselines, Go oracle programs under `tools/oracle/` that compare scanner, parser, binder, module
  resolution, printer and `.types`/`.symbols` output against the Go code, checker counters that equal the reference
  exactly, mutation testing on a large private codebase, and the benchmark projects above. `docs/STATUS.md` has the
  numbers and how each was measured.

## Layout

- `docs/PORTING.md` — porting conventions
- `docs/AST.md`, `docs/CHECKER.md` — crate contracts
- `crates/` — the port, one crate per Go package group
- `tools/` — generators and Go oracle programs used to compare against the reference implementation
- `npm/` — the npm packages (`@maschwenk/tsrs` + per-platform binaries), versioning and the release workflow
- `CONTRIBUTING.md` — building, running the suite, and how changes land

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
38k-file private TypeScript monorepo (5.9M lines); `docs/DEBUGGING.md` describes the fix workflow and the oracles under `tools/oracle/`.
