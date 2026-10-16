# TypeScript compatibility

The goal is 100% conformance with TypeScript 7. This page records the current evidence and known gaps.

## Conformance

tsrs is a function-by-function port of the Go implementation of TypeScript (`microsoft/TypeScript`, commit
`b85298b6a81f`, TypeScript 7.1.0-dev). The Go code is the spec: any difference from it in diagnostics, their order,
message text or exit code is a tsrs bug, even when tsrs's output looks better.

On TypeScript's conformance suite, tsrs matches the reference on 13,458 of 13,462 error baselines (the other four are
test-harness artifacts) and on all 12,779 `.types` and `.symbols` baselines. It has also been run on a large private
monorepo. See the [status log](STATUS.md) for conformance details.

[Top repository conformance](../tools/repo-conformance/README.md) compares diagnostics against pinned tsgo on 400
popular TypeScript repositories after compiler changes merge into main. Actions reports include diffs and coverage.

## What it does and doesn't do

<!--
Updating this table (../AGENTS.md says when; commands and paths below are relative to the repository root):
- Measure numbers at the commit you describe: `tsrs-test run --suite all --baselines types,symbols,js,jsmap,sourcemap`,
  `tsrs-fourslash run` and `tools/oracle/emit/` (docs/EMIT.md, docs/LSP.md). If you copy a number from notes/ or a PR
  instead, say so in the PR description.
- Compare against tsgo built from the pinned commit (Cargo.toml, [workspace.metadata.typescript]), not the npm nightly.
- Keep the status column to a short state: `yes`, `no`, `ignored`, or the condition it needs (such as `--incremental`).
  Put numbers and caveats in the evidence column.
- Keep the root and npm README summaries, docs/EMIT.md, docs/LSP.md and docs/STATUS.md consistent with the table.
-->

"Identical" below means identical to tsgo built from the same pinned commit.

| feature | status | evidence and notes |
| --- | --- | --- |
| Type checking (`--noEmit`) | yes | 13,458 of 13,462 error baselines and all 12,779 `.types` / `.symbols` baselines match |
| Multithreaded checking, `--singleThreaded`, `--pretty`, `--extendedDiagnostics`, `--listFiles`, `--listFilesOnly` | yes | every core up to 16, then at least 16 or half the cores, at most 32 checkers, capped for small programs; build mode uses four; 0.13.0 enables module affinity from 8 checkers on programs with at least 24,000 checked files ([experiment note](../notes/perf-icount-hillclimb.md)) |
| Memory target (`--maxMemory <size>`) | `--noEmit` checks, opt-in | Replaces the largest checker after crossing the target, trading CPU for memory; testdata/regressions identical with a checker retired after nearly every file, also in poison mode ([experiment note](../notes/mem-recycle-checkers.md)) |
| Same output regardless of thread count | yes, with known exceptions (tsgo: no) | a few projects still depend on file order through tsgo's own logic ([experiment note](../notes/open-history-dependence.md)); `--checkerAssignment go` reproduces tsgo exactly, together with `--noLazyMembers` where an instantiation depth limit is reached |
| JavaScript emit | yes | all of tsgo's transforms; 13,392 `.js` baselines pass, 0 fail |
| Declaration emit (`.d.ts`) | yes | part of the `.js` baselines |
| Source maps, declaration maps | yes | 149 `.js.map` and 156 `.sourcemap.txt` baselines pass, 0 fail |
| Incremental builds (`.tsbuildinfo`) | yes | Go-history and API formats unchanged; default CLI adds an optional canonical-slot signature for safe warm reuse; old caches upgrade once ([details](../notes/fix-history-dependent-diagnostics.md)) |
| `--build` (project references) | yes | 189 of 192 `tsbuild` and 191 of 223 `tsc` scenario baselines pass; the failures are CLI commands tsrs doesn't port (`--help`, `--init`, …). Uses more memory than tsgo on large graphs |
| Multi-package emit | yes | Run on a large private monorepo |
| Content mappers (`.vue`, `.svelte`, `.astro` via `contentMappers`) | CLI and `-b`: yes. Language server: no | all 9 content-mapper scenario baselines identical; the 15 content-mapper conformance cases are not run yet ([experiment note](../notes/contentmappers.md)) |
| Language server (`--lsp -stdio`) | yes | 4,066 of 4,546 fourslash tests pass |
| Allocation profiling (`alloc-profile` build) | no | the counting allocator, heap sampler and reachability census have been removed; `TSRS_MEM_SPLIT`, `TSRS_HEAP_CENSUS` and `TSRS_ARENA_POISON` remain ([debugging](DEBUGGING.md)) |
| Automatic type acquisition, telemetry, profiling (language server) | no | not ported |
| Node API server (`--api`) | yes, with gaps | upstream client test suites pass; request-filesystem Go fixtures exercise the production implementation in default tests; node ids use live allocation addresses, with table-local ordering for repeated-node indexes; no profiling requests, no Windows named pipes ([Node API](NODE_API.md)) |
| npm packages (`tsrs`, `@tsrs/<os>-<arch>[-<libc>]`) | yes | 0.14.0: macOS arm64 and Linux x64/arm64 GNU/musl, selected by libc; earlier Linux packages used unsuffixed GNU names, and releases before 0.12.0 used `@maschwenk`. 0.14.1: the platform packages ship the default libraries next to the binary, which reads them from there like the npm tsgo, so `--listFiles`, diagnostics and tsbuildinfo name real lib paths instead of `bundled:///libs/...`. The binaries report TypeScript's version as the npm nightly built from the pinned commit (7.1.0-dev.20260930.4; 0.14.0 reports `7.1.0-dev`), so tsbuildinfo files, `typesVersions` and `--version` agree with that tsgo |
| WebAssembly (`@tsrs/wasm`) | yes | single-threaded; renamed in 0.12.0; byte-identical to native `--singleThreaded` on a 2,000-case conformance sample ([experiment note](../notes/wasm-build.md)) |
| Browser playground | local/static build | [vendored official TypeScript Playground page and styles](../playground/README.md): tsrs WASM checking and emit, virtual files, errors, JS/declarations, examples, Run/Logs, share links and compatible plugins; pinned visual-source integrity, native-equivalence integration checks and production browser checks. Basic editing only: no semantic autocomplete/hover, AST/type inspection, inlay hints, or automatic package type acquisition |
| Options TypeScript 7 removed (ES5 target, AMD/UMD/System, `baseUrl`, …) | rejected, as in tsgo | same TS5102 / TS5108 errors |
| `--watch`, `-b --watch` | yes; Linux and macOS (Windows: exits with "not supported") | 39 of 42 `tscWatch` and all 65 `tsbuildWatch` scenario baselines pass; the three failures are `--help` and `--init`. Watches with inotify on Linux (tsgo uses fanotify where the kernel allows it, inotify otherwise) and FSEvents or kqueue on macOS. On Linux a cycle starts when the writer closes the saved file, where tsgo starts at the first write (often the truncation) and checks a half-written file. A watch session frees each replaced program and each parsed file version nothing refers to any more (notes/watch-memory.md), and `-b --watch` reuses parsed files whose text is unchanged; tsgo reparses every file each cycle. Ctrl-C ends the session |
| `--init`, `--showConfig` | no | exits with "not supported"; `--help` prints a short usage |
| `--locale` | ignored | messages are English only |
| `--generateTrace` | no | warns "Failed to start tracing" (tsgo's message when it can't write a trace) and type-checks as without the flag |
| Ctrl-C, SIGTERM | `--lsp` and `--api`: yes. Command line: stops at once | the language server exits with status 1 ("context canceled"), the API server with status 0 after the next message or EOF, as in tsgo. tsgo's command line catches the signals too but finishes a non-incremental run anyway; tsrs keeps their default action so a check can be interrupted. Windows: default action everywhere |
| Prebuilt binaries | yes | macOS arm64, Linux x64/arm64 (glibc and musl since 0.14.0); no Windows or Intel macOS |
