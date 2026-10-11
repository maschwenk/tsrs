# `tsrs`

[![npm](https://img.shields.io/npm/v/tsrs)](https://www.npmjs.com/package/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](https://github.com/maschwenk/tsrs/blob/main/LICENSE)

`tsrs` is a **drop-in replacement for the TypeScript 7 compiler and language server**, written in Rust.
Our goal is **100% conformance**, with much lower memory use and faster type checking.

## Get started

```sh
pnpm add -D tsrs
pnpm exec tsrs --noEmit
```

[![tsrs versus TypeScript 7: VS Code, 10,427 files, 12.7× faster and 73% less peak memory; T3 Code, 2,945 files, 10.0× faster and 65% less peak memory; MUI docs, 16,766 files, 14.2× faster and 79% less peak memory.](https://raw.githubusercontent.com/maschwenk/tsrs/main/bench/large-projects.svg)](https://github.com/maschwenk/tsrs/blob/main/bench/RESULTS.md#large-project-chart-snapshot)

## Compatibility

`tsrs` follows TypeScript 7.1-dev. It matches **13,458 of 13,462 error baselines** and all **12,779 type and symbol
baselines**; the four remaining error-baseline differences are test-harness artifacts.

`--watch`, `--init` and `--showConfig` are not supported. See [compatibility and evidence](https://github.com/maschwenk/tsrs/blob/main/docs/COMPATIBILITY.md)
for supported features and known gaps.

## Learn more

- [CLI usage and memory tuning](https://github.com/maschwenk/tsrs/blob/main/docs/USAGE.md)
- [Editor setup](https://github.com/maschwenk/tsrs/blob/main/docs/LSP.md#editor-setup)
- [Node API (unstable)](https://github.com/maschwenk/tsrs/blob/main/docs/NODE_API.md)
- [WebAssembly for Node and browsers](https://github.com/maschwenk/tsrs/blob/main/npm/tsrs-wasm/README.md)
- [Benchmarks and methodology](https://github.com/maschwenk/tsrs/blob/main/bench/README.md)
- [Release notes](https://github.com/maschwenk/tsrs/blob/main/docs/STATUS.md)

## Contributing

Issues and pull requests are currently disabled until we have the bandwidth to triage them.

See the [contributing guide](https://github.com/maschwenk/tsrs/blob/main/CONTRIBUTING.md)
for building from source and running the suites, the [Code of Conduct](https://github.com/maschwenk/tsrs/blob/main/CODE_OF_CONDUCT.md), and
[security policy](https://github.com/maschwenk/tsrs/blob/main/SECURITY.md) for reporting vulnerabilities.

## Acknowledgments

Thanks to [@JiangWeixian](https://github.com/JiangWeixian) for transferring the `tsrs` npm package name.

## License

`tsrs` is a derivative of TypeScript (Copyright Microsoft Corporation), licensed under [Apache-2.0](https://github.com/maschwenk/tsrs/blob/main/LICENSE).
The bundled `lib.*.d.ts` files are TypeScript's. See [NOTICE](https://github.com/maschwenk/tsrs/blob/main/NOTICE) for attribution.
Not affiliated with or endorsed by Microsoft.
