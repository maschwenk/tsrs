<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2

tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, "tsgo"). Each row type-checks one project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on (vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions), or from a set of large open-source applications (cal-diy, formbricks-web, supabase-studio, t3code-server), with tsgo 7.0.2 (npm `typescript@7.0.2`) and with tsrs at commit `bd945842ada5` (the PGO-optimized `dist` build, built like the npm release binaries): `tsc -p <project> --noEmit`, median of 3 interleaved runs, on Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor).

**Default mode: 4 checker threads in both (tsrs also resolves members lazily, its default)**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 14.18 | 3.27 | 4.34x | 7.68 GiB | 2.39 GiB | 0.31x |
| xstate-main | 0 / 0 | 0.97 | 0.20 | 4.95x | 752 MiB | 425 MiB | 0.57x |
| webpack | 848 / 840 (ref 840) | 1.51 | 0.35 | 4.36x | 1.21 GiB | 590 MiB | 0.47x |
| mui-docs | 0 / 0 | 14.61 | 1.51 | 9.66x | 8.33 GiB | 1.19 GiB | 0.14x |
| Compiler | 43 / 43 | 0.33 | 0.09 | 3.66x | 224 MiB | 185 MiB | 0.83x |
| Compiler-Unions | 41 / 41 | 0.61 | 0.15 | 4.11x | 250 MiB | 183 MiB | 0.73x |
| cal-diy | 136 / 136 | 7.35 | 1.42 | 5.16x | 4.70 GiB | 1.39 GiB | 0.30x |
| formbricks-web | 0 / 0 | 8.97 | 1.64 | 5.48x | 5.80 GiB | 1.95 GiB | 0.34x |
| supabase-studio | 0 / 9 (ref 9) | 8.45 | 1.78 | 4.75x | 4.92 GiB | 1.46 GiB | 0.30x |
| t3code-server | 5 / 6 (ref 6) | 17.13 | 2.12 | 8.08x | 6.29 GiB | 1.47 GiB | 0.23x |

**`--singleThreaded`: one checker thread in both**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 36.78 | 11.77 | 3.13x | 6.66 GiB | 1.97 GiB | 0.30x |
| xstate-main | 0 / 0 | 1.81 | 0.62 | 2.90x | 618 MiB | 230 MiB | 0.37x |
| webpack | 848 / 840 (ref 840) | 3.24 | 1.18 | 2.74x | 947 MiB | 359 MiB | 0.38x |
| mui-docs | 0 / 0 | 16.72 | 3.30 | 5.07x | 3.80 GiB | 794 MiB | 0.20x |
| Compiler | 43 / 43 | 0.58 | 0.19 | 3.08x | 192 MiB | 81 MiB | 0.42x |
| Compiler-Unions | 41 / 41 | 1.00 | 0.37 | 2.70x | 204 MiB | 87 MiB | 0.42x |
| cal-diy | 136 / 136 | 10.61 | 3.44 | 3.08x | 2.75 GiB | 874 MiB | 0.31x |
| formbricks-web | 0 / 0 | 16.34 | 4.59 | 3.56x | 4.37 GiB | 1.41 GiB | 0.32x |
| supabase-studio | 0 / 9 (ref 9) | 14.74 | 5.22 | 2.82x | 3.13 GiB | 997 MiB | 0.31x |
| t3code-server | 5 / 6 (ref 6) | 20.79 | 4.79 | 4.34x | 3.39 GiB | 878 MiB | 0.25x |

**`--checkers 8`: 8 checker threads in both (how each compiler scales with more checkers)**

| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | tsrs peak memory | memory, tsrs / tsgo |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 359 / 371 (ref 371) | 11.65 | 1.86 | 6.28x | 8.77 GiB | 2.60 GiB | 0.30x |
| xstate-main | 0 / 0 | 0.89 | 0.15 | 5.84x | 937 MiB | 516 MiB | 0.55x |
| webpack | 848 / 840 (ref 840) | 1.43 | 0.25 | 5.77x | 1.49 GiB | 705 MiB | 0.46x |
| mui-docs | 0 / 0 | 17.21 | 1.50 | 11.48x | 13.19 GiB | 1.54 GiB | 0.12x |
| Compiler | 43 / 43 | 0.30 | 0.09 | 3.39x | 253 MiB | 231 MiB | 0.91x |
| Compiler-Unions | 41 / 41 | 0.51 | 0.16 | 3.17x | 265 MiB | 230 MiB | 0.87x |
| cal-diy | 136 / 136 | 7.54 | 1.14 | 6.62x | 6.16 GiB | 1.78 GiB | 0.29x |
| formbricks-web | 0 / 0 | 8.71 | 1.23 | 7.06x | 7.39 GiB | 2.31 GiB | 0.31x |
| supabase-studio | 0 / 9 (ref 9) | 9.17 | 1.19 | 7.73x | 6.97 GiB | 1.74 GiB | 0.25x |
| t3code-server | 5 / 6 (ref 6) | 17.69 | 2.13 | 8.31x | 8.51 GiB | 2.10 GiB | 0.25x |

errors: the number of type errors each compiler reports on the project; they must be equal (a bold cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / tsrs wall (above 1 = tsrs faster). peak memory: maximum resident set size. memory, tsrs / tsgo: below 1 = tsrs uses less.

**Scaling: wall time of `--singleThreaded` / wall time of `--checkers 8`, per compiler**

| project | tsgo | tsrs | tsrs scaling / tsgo scaling | tsgo memory, 8 checkers / 1 | tsrs memory, 8 checkers / 1 |
| --- | ---: | ---: | ---: | ---: | ---: |
| vscode | 3.16x | 6.34x | 2.01x | 1.32x | 1.32x |
| xstate-main | 2.03x | 4.08x | 2.02x | 1.52x | 2.25x |
| webpack | 2.26x | 4.76x | 2.10x | 1.62x | 1.96x |
| mui-docs | 0.97x | 2.20x | 2.26x | 3.47x | 1.98x |
| Compiler | 1.95x | 2.15x | 1.10x | 1.32x | 2.86x |
| Compiler-Unions | 1.97x | 2.31x | 1.17x | 1.30x | 2.65x |
| cal-diy | 1.41x | 3.02x | 2.15x | 2.24x | 2.09x |
| formbricks-web | 1.87x | 3.72x | 1.99x | 1.69x | 1.64x |
| supabase-studio | 1.61x | 4.40x | 2.74x | 2.23x | 1.79x |
| t3code-server | 1.17x | 2.25x | 1.91x | 2.51x | 2.45x |

scaling: how many times faster a compiler gets going from one checker thread to eight; memory: peak memory with 8 checkers divided by peak memory with one.

(ref N): tsgo 7.0.2 and tsrs disagree, but `typescript@7.1.0-dev.20260930.4`, built from the TypeScript commit tsrs ports (`b85298b6`), reports exactly tsrs's errors: a TypeScript 7.0 vs 7.1-dev difference, not a tsrs bug.

Runner: Depot CI `depot-ubuntu-24.04-8` (8 vCPU, 31 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Date: 2026-10-06 19:39 UTC. tsrs commit: `bd945842ada5`. Numbers from shared CI machines are noisy; compare trends, not single runs. How it is measured: [`bench/README.md`](bench/README.md).
<!-- bench:end -->

# tsrs

A Rust port of the TypeScript 7 compiler and language server. As a compiler it is `tsc`: same flags, same tsconfig,
same diagnostics, byte for byte, and it emits by default like tsc does (JavaScript, declarations, source maps,
tsbuildinfo, `-b`), unless the options turn that off (`--noEmit`, `emitDeclarationOnly`, `noEmitOnError`). As a
language server it is `tsgo --lsp`: `tsrs --lsp -stdio`. `docs/EMIT.md` describes the emit port. `tsrs --api` serves TypeScript 7's Node API
(`unstable/sync`, `unstable/async`), with the gaps listed below and in `docs/NODE_API.md`.

It ports the Go implementation in [microsoft/TypeScript](https://github.com/microsoft/TypeScript) (`tsc/internal`)
at commit `b85298b6a81f` function for function, and the Go code is the specification: on the TypeScript conformance
suite the output matches the reference on 13,458 of 13,462 error baselines and on all 12,779 `.types` and `.symbols`
baselines; the four exceptions are test-harness artifacts. These counts are in Go-compatible mode (`--checkerAssignment
go`). By default, tsrs makes output independent of how files are split over checkers, which changes one test's
`.types` / `.symbols` / `.d.ts` baselines by design (notes/perf-order-independence.md). `docs/STATUS.md` has the details and the comparison on a
38k-file production codebase.

## What it does and doesn't do

Starting with `0.3.0`, the npm package includes default emit, incremental builds and the `unstable/*` Node API.
Use `--noEmit` for type checking only; `TSRS_EMIT` no longer changes behavior. "Match" and "identical" below are
against tsgo built from the same pinned commit.

| feature | status | evidence and notes |
| --- | --- | --- |
| Type checking (`tsc --noEmit`) | yes | 13,458 of 13,462 error baselines and all 12,779 `.types` / `.symbols` baselines match; same diagnostics on the 38k-file codebase |
| Checker threads, `--singleThreaded`, `--pretty`, `--extendedDiagnostics`, `--listFiles`, `--listFilesOnly` | yes | checkers steal unstarted files from the busiest checker: -11 to -17% wall at 4-8 checkers on the 38k-file codebase (notes/perf-checker-stealing.md) |
| Output independent of the checker count and assignment | yes (tsgo: no) | diagnostics and `.d.ts` identical for `--checkers 1`-16 and 20 random assignments on the error-rich corpora; `--checkerAssignment go` keeps tsgo's history-dependent output for byte-identity (notes/perf-order-independence.md) |
| Options TypeScript 7 removed (ES5 target, AMD/UMD/System modules, `node10`/`classic` resolution, `baseUrl`, …) | rejected, as in tsgo | same TS5102 / TS5108 errors |
| JavaScript emit | yes (since `0.3.0`) | all of tsgo's script transforms: type erasure, import elision, enums, namespaces, const enum inlining, CommonJS and ES modules, JSX, legacy and standard decorators, class fields, `using`, async and `for await`, and lowering for older targets (object rest/spread, `?.`, `??`, `**`, logical assignment). 13,392 `.js` baselines pass, 0 fail |
| Declaration emit (`.d.ts`) | yes (since `0.3.0`) | part of the `.js` baselines |
| Source maps, declaration maps | yes (since `0.3.0`) | 149 `.js.map` and 156 `.sourcemap.txt` baselines pass, 0 fail |
| Incremental compilation (`.tsbuildinfo`) | yes (since `0.3.0`) | tsbuildinfo files identical, also for `--noEmit` programs |
| `--build` (project references, `--builders`, `--clean`, `--dry`, `--force`, `--verbose`) | yes (since `0.3.0`) | 187 of 190 `tsbuild` and 187 of 216 `tsc` scenario baselines pass; the failures are CLI outputs tsrs does not port (`--help`, `--init`, `--showConfig`, `--locale`, `--generateTrace`, the `--version` line) and one message that prints an internal symbol name. Finished projects stay in memory (Go frees them), so peak memory grows with the graph |
| Emit on the 38k-file codebase | yes (since `0.3.0`) | all 96 packages built with `tsc`: 9,257 output files (JS, `.d.ts`, maps, `.tsbuildinfo`) byte-identical, same diagnostics and exit codes |
| `--watch` (also `-b --watch`) | no | exits with "not supported" |
| `--init`, `--showConfig` | no | exits with "not supported"; `--help` prints a short usage, not tsc's |
| `--locale` | ignored | messages are English only |
| `--generateTrace` | ignored | no trace is written |
| Language server (`--lsp -stdio`) | yes | 4,066 of 4,546 fourslash tests pass; 13,869 of 13,869 responses identical in an xstate editor session (below) |
| Content mappers, automatic type acquisition, telemetry, pprof requests (language server) | no | not ported |
| `--api` (the IPC server behind TypeScript 7's Node API: `unstable/sync` MessagePack, `unstable/async` JSON-RPC) | yes, with gaps | pinned upstream client suites against a release build of the integration branch: `test/sync/api.test.ts` 339/339, `test/async/api.test.ts` 348/348, `ast` 111/111, `astnav` 4/4 + 4/4, `api-generators` 43/43; suite passes are not byte-level response parity. Not implemented: CPU/heap profiling requests; `getCurrentLanguageServerSnapshot` returns the standalone-session error (no LSP-attached API session); no Windows named pipes. Per-method status in `docs/NODE_API.md` |
| Prebuilt binaries | macOS arm64, Linux x64/arm64 (glibc) | no Windows or Intel macOS binary |

## Use it

```sh
npx -y @maschwenk/tsrs -p path/to/project            # macOS arm64, Linux x64/arm64; emits like tsc
npx -y @maschwenk/tsrs -p path/to/project --noEmit   # type check only
npx -y @maschwenk/tsrs -p . --singleThreaded         # one checker thread (less memory)
```

or `pnpm add -D @maschwenk/tsrs` and run `tsrs` from scripts. `--extendedDiagnostics` prints the usual counters.
Without `--checkers`, tsrs runs half as many checker threads as the machine has cores, at least tsgo's 4 and at most
8 (4 for small programs and in `-b` build mode); diagnostics do not depend on the count, the `--extendedDiagnostics`
counters and peak memory do.
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

The last five are Go patches prepared from this port (`upstream/`); the last two were opened and withdrawn as too small on their own (microsoft/TypeScript#64600, #64601).

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
