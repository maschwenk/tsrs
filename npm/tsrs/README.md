# `tsrs`

[![npm](https://img.shields.io/npm/v/tsrs)](https://www.npmjs.com/package/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](https://github.com/maschwenk/tsrs/blob/main/LICENSE)

`tsrs` is a drop-in replacement for the TypeScript 7 compiler and language server, written in Rust. It uses your
existing `tsconfig.json` and supports the familiar `tsc` command-line interface, while using less memory and checking
large projects much faster.

## Get started

```sh
pnpm add -D tsrs
pnpm exec tsrs -p path/to/project           # compile, like tsc
pnpm exec tsrs -p path/to/project --noEmit  # type-check only
```

Like `tsc`, `tsrs` emits JavaScript unless your configuration or command line says otherwise.

## Performance and memory

[![TypeScript 7 versus tsrs: wall time and peak memory on VS Code, T3 Code and MUI docs](https://raw.githubusercontent.com/maschwenk/tsrs/main/bench/large-projects.svg)](https://github.com/maschwenk/tsrs/blob/main/bench/RESULTS.md)

Across the [17-project benchmark suite](https://github.com/maschwenk/tsrs/blob/main/bench/RESULTS.md), `tsrs` is
**4.3–16.2× faster** and uses **54–79% less peak memory** than the official TypeScript 7 compiler. These results use
each compiler's default settings on the same 16-vCPU machine: 16 checker threads for `tsrs` and 4 for TypeScript.

You can trade speed for a lower memory ceiling:

- `--checkers N` chooses the number of checker threads. More threads are usually faster and use more memory.
- `--singleThreaded` minimizes memory use.
- `--maxMemory 12G` keeps a `--noEmit` check near the requested limit by recycling checker state, at a CPU cost.

See [CLI usage and memory tuning](https://github.com/maschwenk/tsrs/blob/main/docs/USAGE.md) and the
[benchmark methodology](https://github.com/maschwenk/tsrs/blob/main/bench/README.md) for details.

## Compatibility

`tsrs` follows TypeScript 7.1-dev. It matches **13,458 of 13,462 error baselines** and all **12,779 type and symbol
baselines**; the other four error-baseline differences are test-harness artifacts.

Supported today:

- type checking and JavaScript, declaration, source-map and declaration-map emit;
- incremental builds and project references with `--build`;
- the language server via `tsrs --lsp -stdio`;
- TypeScript 7's [unstable Node API](https://github.com/maschwenk/tsrs/blob/main/docs/NODE_API.md) and a
  [WebAssembly package](https://github.com/maschwenk/tsrs/blob/main/npm/tsrs-wasm/README.md).

Known limits:

- `--watch`, `--init` and `--showConfig` are not supported;
- prebuilt native packages support macOS arm64 and Linux x64/arm64 with glibc or musl, but not Windows or Intel
  macOS;
- language-server automatic type acquisition, telemetry and profiling are not ported.

See the full [compatibility table and evidence](https://github.com/maschwenk/tsrs/blob/main/docs/COMPATIBILITY.md)
for current status and known gaps.

## License

`tsrs` is a derivative of TypeScript (Copyright Microsoft Corporation), licensed under
[Apache-2.0](https://github.com/maschwenk/tsrs/blob/main/LICENSE). The bundled `lib.*.d.ts` files are TypeScript's.
See [NOTICE](https://github.com/maschwenk/tsrs/blob/main/NOTICE) for attribution. Not affiliated with or endorsed by
Microsoft.
