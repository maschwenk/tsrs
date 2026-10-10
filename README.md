# tsrs

[![npm](https://img.shields.io/npm/v/tsrs)](https://www.npmjs.com/package/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

tsrs is a Rust port of the TypeScript 7 compiler and language server. It's a drop-in replacement for `tsc`: same
flags, same tsconfig, same errors.

On the vscode codebase it type-checks **18x faster than tsc 7** using about a third of the memory, and 1.6x faster than
`bun check`. On TypeScript's own test suite it produces the same errors as the reference compiler in all but 4 of
13,462 tests.

## Quick start

```sh
pnpm dlx tsrs -p path/to/project           # compile, like tsc
pnpm dlx tsrs -p path/to/project --noEmit   # type check only
```

Or install it with `pnpm add -D tsrs` and run `tsrs` in place of `tsc`. Prebuilt binaries are available for
macOS arm64 and Linux x64/arm64. There is also a [WebAssembly build](#webassembly) for Node and browsers.

## Performance

Type-checking vscode (10,427 files) on a 64-core Linux machine, each tool with its default settings, averaged over 20
runs ([source](bench/results/compare/2026-10-07-ae33f10b1b8b-64t.md)):

| | time | peak memory |
| --- | ---: | ---: |
| tsc 7 (tsgo 7.0.2) | 9.79 s | 7.52 GiB |
| `bun check` (Bun 1.4.3 canary) | 0.87 s | 2.86 GiB |
| **tsrs** | **0.53 s** | **2.64 GiB** |

The tables below cover seventeen projects on a 16-core machine. CI regenerates it on every push to `main`, so expect
the numbers to move by a few percent between runs.

<!-- bench:start -->
## Benchmark: tsrs vs tsgo 7.0.2 vs bun check

Time and peak memory to type-check each project (`-p <project> --noEmit`) on Depot CI `depot-ubuntu-24.04-16` (16 vCPU, 63 GB RAM, Linux x86_64, AMD EPYC 9R45 96-Core Processor). Median of 10 interleaved runs (3 for tsgo).

- **tsrs**: commit `6c38fb3c6df8`, PGO-optimized `dist` build, built like the npm release binaries.
- **tsgo**: npm `typescript@7.0.2`, the official Go compiler. tsrs ports a later 7.1-dev commit (`b85298b6`).
- **bun check**: Bun 1.4.4-canary.1+71d0d4399.

Projects: vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions are from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the suite the TypeScript team benchmarks tsgo on; the other 11 are large open-source applications.

### Default settings on 16 vCPUs

No thread flags: tsgo runs 4 checker threads; tsrs runs 16 (its default: all cores up to 16, then half the cores, at most 32; fewer on small projects); bun check runs one thread per core.

**Wall time**

| project | tsgo | tsrs | bun check | tsrs vs tsgo | tsrs vs bun |
| --- | ---: | ---: | ---: | ---: | ---: |
| mui-docs | 12.39 s | 0.89 s | 7.61 s | **13.89x faster** | 8.53x faster |
| next-packages-next | 2.80 s | 0.21 s | 0.36 s | **13.57x faster** | 1.77x faster |
| mikro-orm | 10.34 s | 0.78 s | 1.91 s | **13.18x faster** | 2.44x faster |
| vscode | 10.59 s | 0.88 s | 1.62 s | **12.09x faster** | 1.85x faster |
| storybook | 1.57 s | 0.14 s | 0.27 s | **11.04x faster** | 1.89x faster |
| formbricks-web | 6.70 s | 0.65 s | 1.01 s | **10.32x faster** | 1.55x faster |
| supabase-studio | 6.60 s | 0.64 s | 1.61 s | **10.29x faster** | 2.51x faster |
| t3code-server | 14.47 s | 1.44 s | 2.75 s | **10.04x faster** | 1.91x faster |
| webpack | 1.39 s | 0.15 s | 0.30 s | **9.36x faster** | 2.00x faster |
| playwright | 1.12 s | 0.13 s | 0.22 s | **8.72x faster** | 1.74x faster |
| drizzle-orm | 1.74 s | 0.20 s | 0.42 s | **8.65x faster** | 2.09x faster |
| cal-diy | 5.40 s | 0.62 s | 1.48 s | **8.64x faster** | 2.36x faster |
| next-root | 1.49 s | 0.17 s | 0.28 s | **8.51x faster** | 1.61x faster |
| nuxt | 1.29 s | 0.17 s | 0.43 s | **7.54x faster** | 2.54x faster |
| xstate-main | 0.84 s | 0.13 s | 0.17 s | **6.60x faster** | 1.31x faster |
| Compiler-Unions | 0.53 s | 0.10 s | 0.21 s | **5.02x faster** | 2.00x faster |
| Compiler | 0.28 s | 0.07 s | 0.11 s | 4.18x faster | 1.73x faster |

**Peak memory**

| project | tsgo | tsrs | bun check | tsrs vs tsgo | tsrs vs bun |
| --- | ---: | ---: | ---: | ---: | ---: |
| mui-docs | 7.54 GiB | 1.60 GiB | 11.69 GiB | **4.72x less** | 7.32x less |
| next-packages-next | 1.79 GiB | 692 MiB | 745 MiB | 2.64x less | 1.08x less |
| mikro-orm | 6.42 GiB | 1.95 GiB | 1.41 GiB | 3.30x less | 1.38x more |
| vscode | 7.65 GiB | 2.08 GiB | 1.87 GiB | 3.67x less | 1.12x more |
| storybook | 1.41 GiB | 517 MiB | 607 MiB | 2.79x less | 1.17x less |
| formbricks-web | 5.60 GiB | 1.96 GiB | 1.62 GiB | 2.86x less | 1.21x more |
| supabase-studio | 4.69 GiB | 1.68 GiB | 1.22 GiB | 2.79x less | 1.38x more |
| t3code-server | 6.17 GiB | 2.15 GiB | 1.11 GiB | 2.87x less | 1.94x more |
| webpack | 1.17 GiB | 534 MiB | 493 MiB | 2.24x less | 1.08x more |
| playwright | 1.05 GiB | 492 MiB | 479 MiB | 2.19x less | 1.03x more |
| drizzle-orm | 1.73 GiB | 814 MiB | 734 MiB | 2.17x less | 1.11x more |
| cal-diy | 4.29 GiB | 1.86 GiB | 1.30 GiB | 2.31x less | 1.43x more |
| next-root | 1.29 GiB | 488 MiB | 567 MiB | 2.72x less | 1.16x less |
| nuxt | 1.15 GiB | 518 MiB | 539 MiB | 2.28x less | 1.04x less |
| xstate-main | 742 MiB | 280 MiB | 392 MiB | 2.65x less | 1.40x less |
| Compiler-Unions | 239 MiB | 101 MiB | 229 MiB | 2.36x less | 2.27x less |
| Compiler | 219 MiB | 98 MiB | 219 MiB | 2.23x less | 2.23x less |

**Errors.** tsgo and tsrs report the same errors on 9 of 17 projects. The other 8 differ only because TypeScript changed between 7.0.2 and the 7.1-dev commit tsrs ports: `typescript@7.1.0-dev.20260930.4`, built from that commit, reports exactly tsrs's errors (tsgo / tsrs): vscode 359 / 371, webpack 848 / 840, supabase-studio 0 / 9, t3code-server 5 / 6, next-packages-next 6 / 5, next-root 441 / 438, nuxt 3 / 4, drizzle-orm 10845 / 10846. bun check, which follows TypeScript 7.0 like tsgo, differs from tsgo on webpack (837 errors, tsgo 848).

Rows are sorted by speedup over tsgo. Wall time is the process's wall clock; peak memory is its maximum resident set size. Bold marks a win over tsgo of at least 5x in time or 4x in memory.

Shared CI machines vary by a few percent between runs of the same code; the single-threaded instruction counts in `bench/results/` are the exact measure. Date: 2026-10-10 21:11 UTC. tsrs commit: `6c38fb3c6df8`. Method: [`bench/README.md`](bench/README.md).
<!-- bench:end -->

### Against bun check at equal thread counts

The tools pick different thread counts by default, so here each one gets the same number (vscode, 64-core machine, 20
runs each):

| threads | tsgo 7.0.2 | tsrs | `bun check` |
| ---: | ---: | ---: | ---: |
| 4 | 9.75 s, 7.43 GiB | 2.43 s, 1.83 GiB | 4.57 s, 1.46 GiB |
| 8 | 6.13 s, 7.99 GiB | 1.35 s, 1.97 GiB | 2.47 s, 1.60 GiB |
| 16 | 4.54 s, 8.67 GiB | 0.81 s, 2.13 GiB | 1.49 s, 1.85 GiB |
| 64 | 4.01 s, 11.58 GiB | 0.44 s, 3.06 GiB | 0.87 s, 2.86 GiB |

tsrs is about 1.9x faster at every thread count; tsrs uses 7-25% more memory. `bun check` follows TypeScript
7.0 and tsrs follows 7.1-dev, so their error counts can differ slightly; each matches its own reference exactly.

[`bench/README.md`](bench/README.md) explains how everything is measured and how to rerun it. The published numbers
use the PGO-optimized release build, which is 7-14% faster than a plain `cargo build --release`.

## Correctness

tsrs is a function-by-function port of the Go implementation of TypeScript (`microsoft/TypeScript`, commit
`b85298b6a81f`, TypeScript 7.1.0-dev). The Go code is the spec: any difference from it in diagnostics, their order,
message text or exit code is a tsrs bug, even when tsrs's output looks better.

On TypeScript's conformance suite, tsrs matches the reference on 13,458 of 13,462 error baselines (the other four are
test-harness artifacts) and on all 12,779 `.types` and `.symbols` baselines. On the 38k-file codebase it reports the
same diagnostics and emits the same 9,257 files byte for byte. [`docs/STATUS.md`](docs/STATUS.md) has the details.

## What it does and doesn't do

"Identical" below means identical to tsgo built from the same pinned commit.

| feature | status | evidence and notes |
| --- | --- | --- |
| Type checking (`--noEmit`) | yes | 13,458 of 13,462 error baselines and all 12,779 `.types` / `.symbols` baselines match; same diagnostics on the 38k-file codebase |
| Multithreaded checking, `--singleThreaded`, `--pretty`, `--extendedDiagnostics`, `--listFiles`, `--listFilesOnly` | yes | every core up to 16, then at least 16 or half the cores, at most 32 checkers, capped for small programs; build mode uses four; 0.13.0 enables module affinity from 8 checkers on programs with at least 24,000 checked files (notes/perf-icount-hillclimb.md) |
| Memory target (`--maxMemory <size>`) | `--noEmit` checks, opt-in | the 38k-file codebase at 8 checkers: peak 18.0 GB without it; `--maxMemory 12G` 12.9 GB (+11% instructions), `10G` 10.8 GB (+16%), `8G` 8.7 GB (+35%), same diagnostics; testdata/regressions identical with a checker retired after nearly every file, also in poison mode (notes/mem-recycle-checkers.md) |
| Same output regardless of thread count | yes, with known exceptions (tsgo: no) | a few projects still depend on file order through tsgo's own logic (notes/open-history-dependence.md); `--checkerAssignment go` reproduces tsgo exactly |
| JavaScript emit | yes | all of tsgo's transforms; 13,392 `.js` baselines pass, 0 fail |
| Declaration emit (`.d.ts`) | yes | part of the `.js` baselines |
| Source maps, declaration maps | yes | 149 `.js.map` and 156 `.sourcemap.txt` baselines pass, 0 fail |
| Incremental builds (`.tsbuildinfo`) | yes | identical tsbuildinfo files |
| `--build` (project references) | yes | 189 of 192 `tsbuild` and 191 of 223 `tsc` scenario baselines pass; the failures are CLI commands tsrs doesn't port (`--help`, `--init`, …). Uses more memory than tsgo on large graphs |
| Emit on the 38k-file codebase | yes | all 96 packages: 9,257 output files byte-identical, same diagnostics and exit codes |
| Content mappers (`.vue`, `.svelte`, `.astro` via `contentMappers`) | CLI and `-b`: yes. Language server: no | all 9 content-mapper scenario baselines identical; the 15 content-mapper conformance cases are not run yet (notes/contentmappers.md) |
| Language server (`--lsp -stdio`) | yes | 4,066 of 4,546 fourslash tests pass; 13,869 of 13,869 responses identical in a recorded editor session |
| Automatic type acquisition, telemetry, profiling (language server) | no | not ported |
| Node API server (`--api`) | yes, with gaps | upstream client test suites pass; no profiling requests, no Windows named pipes ([`docs/NODE_API.md`](docs/NODE_API.md)) |
| npm packages (`tsrs`, `@ts-rs/<os>-<arch>`) | yes | since 0.12.0; macOS arm64 and Linux x64/arm64; earlier releases used `@maschwenk` |
| WebAssembly (`@ts-rs/wasm`) | yes | single-threaded; renamed in 0.12.0; byte-identical to native `--singleThreaded` on a 2,000-case conformance sample (notes/wasm-build.md) |
| Options TypeScript 7 removed (ES5 target, AMD/UMD/System, `baseUrl`, …) | rejected, as in tsgo | same TS5102 / TS5108 errors |
| `--watch` | no | exits with "not supported" |
| `--init`, `--showConfig` | no | exits with "not supported"; `--help` prints a short usage |
| `--locale` | ignored | messages are English only |
| `--generateTrace` | no | warns "Failed to start tracing" (tsgo's message when it can't write a trace) and type-checks as without the flag |
| Prebuilt binaries | macOS arm64, Linux x64/arm64 (glibc) | no Windows or Intel macOS |

## Options

tsrs accepts every `tsc` flag it supports. A few extra ones control performance:

- `--checkers N` sets the number of checker threads. By default tsrs uses one per core up to 16 cores, then half the
  cores, from 4 to 32; a program with fewer than 32 type-checked files per checker gets fewer, down to 4, and never
  more checkers than files. `--build` uses 4 per project. More threads are faster but use more memory. The thread
  count doesn't change the diagnostics, apart from the known exceptions in the table above.
- `--singleThreaded` uses one thread, for the lowest memory use.
- `--checkerCostCache <file>` remembers how long each file took to check and uses that to balance threads on the next
  run (up to ~15% faster on repeated runs).
- `--maxMemory <size>` (for example `12G`; `--noEmit` checks only) keeps the process near that much memory: above it,
  tsrs replaces its largest checker with a fresh one, trading CPU for memory. Useful on very large programs (the
  38k-file codebase: 18.0 GB without it, 10.8 GB at `10G` for 16% more instructions). The diagnostics are the same.
- `--checkerAssignment go` splits files across threads the way tsgo does, for byte-identical output with tsgo.
- `--noLazyMembers` turns off a set of checker optimizations that are not yet merged upstream. They never change
  diagnostics; the patches are in [`upstream/`](upstream/).

## Language server

`tsrs --lsp -stdio` is a port of the TypeScript 7 language server, with diagnostics, hover, go to definition,
references, rename, completions with auto-imports, code fixes, formatting and the rest. It passes 4,066 of 4,546 of
TypeScript's fourslash tests (most of the rest are skipped by the Go implementation too). Over a 200-edit session on
the 38k-file codebase it stays flat at 2.9 GiB, where the Go server grows from 4.9 to 7.4 GiB.

Neovim 0.11+:

```lua
vim.lsp.config('tsrs', {
  cmd = { 'tsrs', '--lsp', '-stdio' },
  filetypes = { 'typescript', 'typescriptreact', 'javascript', 'javascriptreact' },
  root_markers = { 'tsconfig.json', 'jsconfig.json', 'package.json', '.git' },
})
vim.lsp.enable('tsrs')
```

VS Code: the TypeScript 7 extension runs whatever `tsgo` executable it finds in `js/ts.tsdk.path`. Point that setting
at a directory containing a `tsgo` symlink to the tsrs binary. [`docs/LSP.md`](docs/LSP.md) has the full setup and the
known gaps.

## WebAssembly

`@ts-rs/wasm` runs tsrs in Node 22+ or in a browser. It is single-threaded and its output matches native
`tsrs --singleThreaded` byte for byte.

```sh
pnpm dlx @ts-rs/wasm -p .
```

```js
import { tsc } from "@ts-rs/wasm";
const { exitCode, diagnostics } = await tsc(["-p", "."], { files: { "/tsconfig.json": "{}", "/a.ts": "let x: string = 1;" }, diagnostics: "json" });
```

It runs at about half the speed of native single-threaded tsrs, and the module is 2.58 MB gzipped. In a one-off
comparison it was 2.7x to 7x faster than [ts-rust](https://github.com/pingdotgg/ts-rust)'s WebAssembly build and used
1.2x to 2.6x less memory ([notes/wasm-build.md](notes/wasm-build.md#nine-projects-2026-10-08)). It doesn't support
`--watch`, `--lsp`, `--api` or multiple threads. See [`npm/tsrs-wasm/README.md`](npm/tsrs-wasm/README.md) for the API.

## Build from source

```sh
cargo build --release -p tsrs_cli -p tsrs_testrunner
./target/release/tsrs -p path/to/project
./target/release/tsrs-test run --suite all --baselines types,symbols   # conformance suite, ~20 s
cargo build --release -p tsrs_fourslash && ./target/release/tsrs-fourslash run   # language server tests, ~5 s
tools/wasm/build.sh                                                     # WebAssembly build (needs wasm32-wasip1, wasm-opt)
```

## Docs

- [`CONTRIBUTING.md`](CONTRIBUTING.md): the development workflow and porting rules.
- [`docs/STATUS.md`](docs/STATUS.md): status log and release notes.
- [`docs/EMIT.md`](docs/EMIT.md): JavaScript, declaration and source map emit.
- [`docs/LSP.md`](docs/LSP.md): the language server.
- [`docs/NODE_API.md`](docs/NODE_API.md): the Node API.
- [`docs/DEBUGGING.md`](docs/DEBUGGING.md): tracking down a difference from tsgo.
- [`bench/README.md`](bench/README.md): how the benchmarks work.
- `notes/`: one note per performance experiment, with numbers.

## Contributing

Bug reports with a small reproduction are the most useful contribution, and fixes are welcome. Any difference from tsgo
counts as a bug. See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md) for reporting vulnerabilities,
and the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

A derivative of microsoft/TypeScript (Copyright Microsoft Corporation), licensed Apache-2.0 like the original
([`LICENSE`](LICENSE), [`NOTICE`](NOTICE)). The bundled `lib.*.d.ts` files are TypeScript's. Not affiliated with or
endorsed by Microsoft. The port was written largely by AI coding agents, directed and reviewed by Max Schwenk; its
correctness rests on the test results above rather than on review alone.
