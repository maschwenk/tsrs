<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), with tsgo 7.0.2 (npm `typescript@7.0.2`) and with tsrs at commit `d5e59e8a4c6a` (the PGO-optimized `dist` build, built like the npm release binaries): `tsc -p <project> --noEmit`, median of 3 interleaved runs, on Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor).

**Default mode: 4 checker threads in both (tsrs also resolves members lazily, its default)**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 11.91 | 3.42 | 3.48x | 7.77 GiB | 3.13 GiB | 0.40x |
| xstate-main | 0 / 0 | 0.85 | 0.25 | 3.34x | 747 MiB | 525 MiB | 0.70x |
| webpack | 848 / 840 (ref 840) | 1.37 | 0.47 | 2.91x | 1.21 GiB | 729 MiB | 0.59x |
| mui-docs | 0 / 0 | 12.66 | 1.79 | 7.05x | 8.18 GiB | 1.47 GiB | 0.18x |
| Compiler | 43 / 43 | 0.26 | 0.08 | 3.22x | 219 MiB | 237 MiB | 1.09x |
| Compiler-Unions | 41 / 41 | 0.51 | 0.15 | 3.33x | 248 MiB | 219 MiB | 0.88x |

**`--singleThreaded`: one checker thread in both**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 31.67 | 10.87 | 2.91x | 6.62 GiB | 2.59 GiB | 0.39x |
| xstate-main | 0 / 0 | 1.66 | 0.64 | 2.60x | 613 MiB | 287 MiB | 0.47x |
| webpack | 848 / 840 (ref 840) | 2.96 | 1.14 | 2.59x | 938 MiB | 443 MiB | 0.47x |
| mui-docs | 0 / 0 | 14.16 | 3.47 | 4.08x | 3.41 GiB | 1000 MiB | 0.29x |
| Compiler | 43 / 43 | 0.46 | 0.17 | 2.67x | 183 MiB | 101 MiB | 0.55x |
| Compiler-Unions | 41 / 41 | 0.82 | 0.34 | 2.43x | 207 MiB | 111 MiB | 0.54x |

errors: the number of type errors each compiler reports on the project; they must be equal (a bold cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster). peak memory: maximum resident set size. memory, tsrs / tsgo: below 1 = tsrs uses less.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Date: 2026-10-02 13:20 UTC. tsrs commit: `d5e59e8a4c6a`. Numbers from shared CI machines are noisy; compare trends, not single runs. How it is measured: [`bench/README.md`](bench/README.md).
<!-- bench:end -->

# tsrs

A Rust port of the TypeScript 7 type checker and language server. As a compiler it is `tsc --noEmit`: same flags,
same tsconfig, same diagnostics, byte for byte. As a language server it is `tsgo --lsp`: `tsrs --lsp -stdio`. No emit
(`docs/EMIT.md` has the plan).

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
`--checkerCostCache <file>` (opt-in) records per-file check times in `<file>` and balances the checker threads on
them in the next run (a few percent to ~15% less wall time on repeated runs; it never changes diagnostics).
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

## Language server

`tsrs --lsp -stdio` is a port of the TypeScript 7 language server (Go's `internal/lsp`, `internal/ls`,
`internal/project`): diagnostics, hover, definitions, references, rename, completions with auto-imports, signature
help, symbols, semantic tokens, folding, inlay hints, call hierarchy, code fixes, organize imports, formatting,
file watching and cancellation.

* TypeScript's fourslash tests: 4,066 of 4,546 pass; the 63 failures need content mappers (not ported) or `tsc -b`
  with emit, and the other 417 are skipped in the Go implementation too.
* Replaying the same editor session against the Go server and comparing the JSON responses: 13,869 of 13,869
  identical on xstate (diagnostics, hover, definition, references, completion, signature help, symbols, with edits).
* Memory on the 38k-file codebase over a 200-edit session: 2.9 GiB and flat (0.01 MiB per edit); the Go server goes
  from 4.9 to 7.4 GiB in the same session.

Neovim 0.11+:

```lua
vim.lsp.config('tsrs', {
  cmd = { 'tsrs', '--lsp', '-stdio' },
  filetypes = { 'typescript', 'typescriptreact', 'javascript', 'javascriptreact' },
  root_markers = { 'tsconfig.json', 'jsconfig.json', 'package.json', '.git' },
})
vim.lsp.enable('tsrs')
```

VS Code: the TypeScript 7 extension starts whatever executable named `tsgo` it finds in `js/ts.tsdk.path`, so point
that setting at a directory holding a `tsgo` symlink to the tsrs binary. `docs/LSP.md` has both setups in full, the
design, the test tables and the known gaps.

## Build from source

```sh
cargo build --release -p tsrs_cli -p tsrs_testrunner
./target/release/tsrs -p path/to/project
./target/release/tsrs-test run --suite all --baselines types,symbols   # conformance suite, ~20 s
cargo build --release -p tsrs_fourslash && ./target/release/tsrs-fourslash run   # language-service tests, ~5 s
```

`CONTRIBUTING.md` covers the workflow; `docs/PORTING.md`, `docs/AST.md`, `docs/CHECKER.md` and `docs/LSP.md` the conventions;
`tools/oracle/` the Go oracle programs that compare each stage against the reference.

## Provenance

Derivative work of microsoft/TypeScript (Copyright Microsoft Corporation), Apache-2.0 like the original
(`LICENSE`, `NOTICE`); the bundled `lib.*.d.ts` files are TypeScript's. Not affiliated with or endorsed by
Microsoft. The port was written largely by AI coding agents directed and reviewed by Max Schwenk; its correctness
rests on the evidence above rather than on review alone.
