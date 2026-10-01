<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), with tsgo 7.0.2 (npm `typescript@7.0.2`) and with tsrs at commit `b539dd5d628a`: `tsc -p <project> --noEmit`, median of 3 interleaved runs, on Max's Mac, measured locally before the first CI run (Apple M5 Max, 18 cores, 128 GB, macOS arm64; shared with other jobs, load average ~27).

**Default mode: 4 checker threads in both (tsrs also resolves members lazily, its default)**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 6.20 | 2.48 | 2.50x | 7.49 GiB | 3.09 GiB | 0.41x |
| xstate-main | 0 / 0 | 0.52 | 0.20 | 2.56x | 743 MiB | 337 MiB | 0.45x |
| webpack | 848 / 840 (ref 840) | 0.86 | 0.39 | 2.22x | 1.19 GiB | 519 MiB | 0.43x |
| mui-docs | 0 / 0 | 8.33 | 1.69 | 4.94x | 7.38 GiB | 1.35 GiB | 0.18x |
| Compiler | 43 / 43 | 0.18 | 0.07 | 2.68x | 224 MiB | 104 MiB | 0.46x |
| Compiler-Unions | 41 / 41 | 0.36 | 0.14 | 2.64x | 246 MiB | 109 MiB | 0.44x |

**`--singleThreaded`: one checker thread in both**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 17.16 | 8.14 | 2.11x | 6.68 GiB | 2.75 GiB | 0.41x |
| xstate-main | 0 / 0 | 1.18 | 0.52 | 2.26x | 616 MiB | 268 MiB | 0.44x |
| webpack | 848 / 840 (ref 840) | 2.00 | 0.93 | 2.15x | 917 MiB | 430 MiB | 0.47x |
| mui-docs | 0 / 0 | 10.45 | 3.26 | 3.21x | 3.81 GiB | 1.01 GiB | 0.26x |
| Compiler | 43 / 43 | 0.32 | 0.14 | 2.23x | 195 MiB | 73 MiB | 0.38x |
| Compiler-Unions | 41 / 41 | 0.59 | 0.31 | 1.93x | 208 MiB | 75 MiB | 0.36x |

errors: the number of type errors each compiler reports on the project; they must be equal (a bold cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster). peak memory: maximum resident set size. memory, tsrs / tsgo: below 1 = tsrs uses less.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Max's Mac, measured locally before the first CI run (Apple M5 Max, 18 cores, 128 GB, macOS arm64; shared with other jobs, load average ~27). Date: 2026-10-01 12:28 UTC. tsrs commit: `b539dd5d628a`. How it is measured: [`bench/README.md`](bench/README.md).
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
