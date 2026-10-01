<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), with tsgo 7.0.2 (npm `typescript@7.0.2`) and with tsrs at commit `aef6d9f20b9b` (the PGO-optimized `dist` build, built like the npm release binaries): `tsc -p <project> --noEmit`, median of 3 interleaved runs, on Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor).

**Default mode: 4 checker threads in both (tsrs also resolves members lazily, its default)**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 11.61 | 3.31 | 3.50x | 7.45 GiB | 3.34 GiB | 0.45x |
| xstate-main | 0 / 0 | 0.86 | 0.25 | 3.46x | 746 MiB | 552 MiB | 0.74x |
| webpack | 848 / 840 (ref 840) | 1.41 | 0.45 | 3.11x | 1.21 GiB | 755 MiB | 0.61x |
| mui-docs | 0 / 0 | 12.89 | 1.81 | 7.12x | 8.16 GiB | 1.54 GiB | 0.19x |
| Compiler | 43 / 43 | 0.27 | 0.08 | 3.34x | 225 MiB | 227 MiB | 1.01x |
| Compiler-Unions | 41 / 41 | 0.51 | 0.15 | 3.34x | 246 MiB | 223 MiB | 0.91x |

**`--singleThreaded`: one checker thread in both**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 30.10 | 10.46 | 2.88x | 6.69 GiB | 2.78 GiB | 0.41x |
| xstate-main | 0 / 0 | 1.66 | 0.64 | 2.62x | 620 MiB | 301 MiB | 0.49x |
| webpack | 848 / 840 (ref 840) | 3.01 | 1.13 | 2.67x | 955 MiB | 454 MiB | 0.48x |
| mui-docs | 0 / 0 | 14.51 | 3.45 | 4.21x | 3.67 GiB | 1.03 GiB | 0.28x |
| Compiler | 43 / 43 | 0.47 | 0.17 | 2.69x | 174 MiB | 99 MiB | 0.57x |
| Compiler-Unions | 41 / 41 | 0.82 | 0.33 | 2.45x | 209 MiB | 109 MiB | 0.52x |

errors: the number of type errors each compiler reports on the project; they must be equal (a bold cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster). peak memory: maximum resident set size. memory, tsrs / tsgo: below 1 = tsrs uses less.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Date: 2026-10-01 21:46 UTC. tsrs commit: `aef6d9f20b9b`. Numbers from shared CI machines are noisy; compare trends, not single runs. How it is measured: [`bench/README.md`](bench/README.md).
<!-- bench:end -->

# tsrs

A Rust port of the TypeScript 7 type checker. It is `tsc --noEmit`: same flags, same tsconfig, same diagnostics,
byte for byte. No emit, no language service.

It ports the Go implementation in [microsoft/TypeScript](https://github.com/microsoft/TypeScript) (`tsc/internal`)
at commit `b85298b6a81f` function for function, and the Go code is the specification: on the TypeScript conformance
suite the output matches the reference on 13,458 of 13,462 error baselines and on all 12,779 `.types` and `.symbols`
baselines; the four exceptions are test-harness artifacts. `docs/STATUS.md` has the details and the comparison on a
38k-file production codebase.

## Use it

```sh
npx -y @maschwenk/tsrs -p path/to/project        # macOS arm64/x64, Linux x64/arm64
npx -y @maschwenk/tsrs -p . --singleThreaded     # one checker thread (less memory)
```

or `pnpm add -D @maschwenk/tsrs` and run `tsrs` from scripts. `--extendedDiagnostics` prints the usual counters.
By default tsrs also runs with checker changes that are not merged upstream yet; `--noLazyMembers` turns them
off and gives the reference-identical mode. None of them changes any diagnostic (verified on the whole conformance
suite, errors, types and symbols):

* [build member tables of instantiated classes/interfaces lazily (microsoft/TypeScript#64475)](https://github.com/microsoft/TypeScript/pull/64475)
* [build members of keyof mapped types lazily (microsoft/TypeScript#64526)](https://github.com/microsoft/TypeScript/pull/64526)
* [give tuple references lazy member tables](upstream/pr-01-tuple-lazy-tables.md)
* [answer empty-object checks from lazy member tables](upstream/pr-02-empty-object-lazy-tables.md)
* [find unmatched properties without instantiating a lazy target's members](upstream/pr-03-unmatched-properties-lazy-target.md)
* [don't copy union/intersection properties into the augmented property cache](upstream/pr-04-union-property-cache.md)
* [instantiate conditional types without a combined mapper for the cache lookup](upstream/pr-05-conditional-instantiation-mapper.md)

The last five are Go patches prepared from this port, not yet opened upstream (`upstream/`).

## Build from source

```sh
cargo build --release -p tsrs_cli -p tsrs_testrunner
./target/release/tsrs -p path/to/project
./target/release/tsrs-test run --suite all --baselines types,symbols   # conformance suite, ~20 s
```

`CONTRIBUTING.md` covers the workflow; `docs/PORTING.md`, `docs/AST.md` and `docs/CHECKER.md` the conventions;
`tools/oracle/` the Go oracle programs that compare each stage against the reference.

## Provenance

Derivative work of microsoft/TypeScript (Copyright Microsoft Corporation), Apache-2.0 like the original
(`LICENSE`, `NOTICE`); the bundled `lib.*.d.ts` files are TypeScript's. Not affiliated with or endorsed by
Microsoft. The port was written largely by AI coding agents directed and reviewed by Max Schwenk; its correctness
rests on the evidence above rather than on review alone.
