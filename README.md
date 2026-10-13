# `tsrs`

[![npm](https://img.shields.io/npm/v/tsrs)](https://www.npmjs.com/package/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](https://github.com/maschwenk/tsrs/blob/main/LICENSE)

`tsrs` is a TypeScript 7 compiler and language server written in Rust. Its goal is 100% compatibility, with less
memory use and faster type checking.

## Get started

```sh
pnpm add -D tsrs
pnpm exec tsrs -p path/to/project           # compile
pnpm exec tsrs -p path/to/project --noEmit  # check types only
```

The first command emits JavaScript. Use `--noEmit` when you only want to check types.

## Performance and memory

[![TypeScript 7 versus tsrs: wall time and peak memory on VS Code, T3 Code and MUI docs](https://raw.githubusercontent.com/maschwenk/tsrs/main/bench/large-projects.svg)](https://github.com/maschwenk/tsrs/blob/main/bench/RESULTS.md)

Across [17 projects](https://github.com/maschwenk/tsrs/blob/main/bench/RESULTS.md), `tsrs` is 4.3–15.8× faster and
uses 54–79% less peak memory than the official TypeScript 7 compiler. Both ran with their default settings on the
same 16-vCPU machine.

You can use less memory by giving up some speed:

- `--checkers N` sets the number of checker threads. Fewer threads use less memory.
- `--singleThreaded` uses one checker thread and the least memory.
- `--maxMemory 12G` keeps a `--noEmit` check near that limit, but makes the check slower.

See [CLI usage and memory tuning](https://github.com/maschwenk/tsrs/blob/main/docs/USAGE.md) and the
[benchmark methodology](https://github.com/maschwenk/tsrs/blob/main/bench/README.md) for details.

## Compatibility

`tsrs` follows TypeScript 7.1-dev. It matches 13,458 of 13,462 error baselines and all 12,779 type and symbol
baselines. The other four differences come from the test tools, not compiler output.

Supported today:

- type checking and JavaScript, declaration and source-map output;
- incremental builds and project references with `--build`;
- the language server via `tsrs --lsp -stdio`;
- TypeScript 7's [unstable Node API](https://github.com/maschwenk/tsrs/blob/main/docs/NODE_API.md) and a
  [WebAssembly package](https://github.com/maschwenk/tsrs/blob/main/npm/tsrs-wasm/README.md).

Known limits:

- `--watch`, `--init` and `--showConfig` are not supported;
- prebuilt binaries are available for macOS arm64 and Linux x64/arm64 with glibc or musl, but not Windows or Intel
  macOS;
- the language server does not support automatic type downloads, telemetry or profiling.

See the full [compatibility table and evidence](https://github.com/maschwenk/tsrs/blob/main/docs/COMPATIBILITY.md)
for current status and known gaps.

## License

`tsrs` is a derivative of TypeScript (Copyright Microsoft Corporation), licensed under
[Apache-2.0](https://github.com/maschwenk/tsrs/blob/main/LICENSE). The bundled `lib.*.d.ts` files are TypeScript's.
See [NOTICE](https://github.com/maschwenk/tsrs/blob/main/NOTICE) for attribution. Not affiliated with or endorsed by
Microsoft.
