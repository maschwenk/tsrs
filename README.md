<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), with tsgo 7.0.2 (npm `typescript@7.0.2`) and with tsrs at commit `113396a1537f`: `tsc -p <project> --noEmit`, median of 3 interleaved runs, on Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor).

**Default mode: 4 checker threads in both (tsrs also resolves members lazily, its default)**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 12.42 | 4.08 | 3.05x | 7.67 GiB | 3.29 GiB | 0.43x |
| xstate-main | 0 / 0 | 0.96 | 0.29 | 3.28x | 751 MiB | 512 MiB | 0.68x |
| webpack | 848 / 840 (ref 840) | 1.50 | 0.63 | 2.39x | 1.19 GiB | 706 MiB | 0.58x |
| mui-docs | 0 / 0 | 13.70 | 2.23 | 6.16x | 8.31 GiB | 1.49 GiB | 0.18x |
| Compiler | 43 / 43 | 0.31 | 0.10 | 2.95x | 223 MiB | 205 MiB | 0.92x |
| Compiler-Unions | 41 / 41 | 0.57 | 0.20 | 2.83x | 249 MiB | 209 MiB | 0.84x |

**`--singleThreaded`: one checker thread in both**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 33.36 | 12.79 | 2.61x | 6.57 GiB | 2.79 GiB | 0.42x |
| xstate-main | 0 / 0 | 1.90 | 0.77 | 2.46x | 616 MiB | 317 MiB | 0.52x |
| webpack | 848 / 840 (ref 840) | 3.38 | 1.43 | 2.36x | 939 MiB | 465 MiB | 0.50x |
| mui-docs | 0 / 0 | 16.89 | 4.33 | 3.90x | 3.48 GiB | 1.04 GiB | 0.30x |
| Compiler | 43 / 43 | 0.57 | 0.24 | 2.37x | 177 MiB | 109 MiB | 0.61x |
| Compiler-Unions | 41 / 41 | 0.94 | 0.46 | 2.04x | 205 MiB | 101 MiB | 0.49x |

errors: the number of type errors each compiler reports on the project; they must be equal (a bold cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster). peak memory: maximum resident set size. memory, tsrs / tsgo: below 1 = tsrs uses less.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Date: 2026-10-01 19:45 UTC. tsrs commit: `113396a1537f`. Numbers from shared CI machines are noisy; compare trends, not single runs. How it is measured: [`bench/README.md`](bench/README.md).
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
