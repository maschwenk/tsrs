# `tsrs`

[![npm](https://img.shields.io/npm/v/tsrs)](https://www.npmjs.com/package/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

`tsrs` is a **drop-in replacement for the TypeScript 7 compiler and language server**, written in Rust.
Our goal is **100% conformance**, with much lower memory use and faster type checking.

## Get started

```sh
pnpm add -D tsrs
pnpm exec tsrs --noEmit
```

[![tsrs versus TypeScript 7: VS Code, 10,427 files, 13.2× faster and 72% less peak memory; T3 Code, 2,945 files, 9.9× faster and 65% less; MUI docs, 16,766 files, 14.4× faster and 79% less.](bench/large-projects.svg)](bench/RESULTS.md#large-project-chart-snapshot)

## Compatibility

`tsrs` follows TypeScript 7.1-dev. It matches **13,458 of 13,462 error baselines** and all **12,779 type and symbol
baselines**; the four remaining error-baseline differences are test-harness artifacts.

`--watch`, `--init` and `--showConfig` are not supported. See [compatibility and evidence](docs/COMPATIBILITY.md)
for supported features and known gaps.

## Learn more

- [CLI usage and memory tuning](docs/USAGE.md)
- [Editor setup](docs/LSP.md#editor-setup)
- [Node API (unstable)](docs/NODE_API.md)
- [WebAssembly for Node and browsers](npm/tsrs-wasm/README.md)
- [Benchmarks and methodology](bench/README.md)
- [Release notes](docs/STATUS.md)

## Contributing

Bug reports with a small reproduction and fixes are welcome. See the [contributing guide](CONTRIBUTING.md)
for building from source and running the suites, the [Code of Conduct](CODE_OF_CONDUCT.md), and
[security policy](SECURITY.md) for reporting vulnerabilities.

## Acknowledgments and license

Thanks to [@JiangWeixian](https://github.com/JiangWeixian) for transferring the `tsrs` npm package name.

`tsrs` is a derivative of TypeScript (Copyright Microsoft Corporation), licensed under [Apache-2.0](LICENSE).
The bundled `lib.*.d.ts` files are TypeScript's. See [NOTICE](NOTICE) for attribution.
Not affiliated with or endorsed by Microsoft.
