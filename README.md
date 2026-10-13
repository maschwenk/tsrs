# `tsrs`

[![npm](https://img.shields.io/npm/v/tsrs)](https://www.npmjs.com/package/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](https://github.com/maschwenk/tsrs/blob/main/LICENSE)

`tsrs` is a drop-in replacement for the TypeScript 7 compiler and language server, written in Rust.
Our goal is 100% conformance, with much lower memory use and faster type checking.

## Get started

```sh
pnpm add -D tsrs
pnpm exec tsrs --noEmit
```

You can try `tsrs` today and give us feedback on performance, memory usage, and conformance.

## Conformance

`tsrs` currently targets the main branch of [microsoft/TypeScript](https://github.com/microsoft/typescript).
We aim to release a stable version against TypeScript 7.1 with 100% conformance.

We take conformance really seriously and are fixing the remaining 0.01% of issues we discover in existing projects.

`--init` and `--showConfig` are not supported. See [compatibility and evidence](https://github.com/maschwenk/tsrs/blob/main/docs/COMPATIBILITY.md)
for supported features and known gaps.

[![Performance](https://raw.githubusercontent.com/maschwenk/tsrs/main/bench/large-projects.svg)](https://github.com/maschwenk/tsrs/blob/main/bench/RESULTS.md)

## Learn more

- [CLI usage and memory tuning](https://github.com/maschwenk/tsrs/blob/main/docs/USAGE.md)
- [Editor setup](https://github.com/maschwenk/tsrs/blob/main/docs/LSP.md#editor-setup)
- [Node API (unstable)](https://github.com/maschwenk/tsrs/blob/main/docs/NODE_API.md)
- [WebAssembly for Node and browsers](https://github.com/maschwenk/tsrs/blob/main/npm/tsrs-wasm/README.md)
- [Benchmarks and methodology](https://github.com/maschwenk/tsrs/blob/main/bench/README.md)
- [Release notes](https://github.com/maschwenk/tsrs/blob/main/docs/STATUS.md)
- [Engineering notes](https://github.com/maschwenk/tsrs/blob/main/notes/README.md)

## Contributing

Issues and pull requests are currently disabled until we have the bandwidth to triage them.

See the [contributing guide](https://github.com/maschwenk/tsrs/blob/main/CONTRIBUTING.md)
for building from source and running the suites, the [Code of Conduct](https://github.com/maschwenk/tsrs/blob/main/CODE_OF_CONDUCT.md), and
[security policy](https://github.com/maschwenk/tsrs/blob/main/SECURITY.md) for reporting vulnerabilities.

## Acknowledgments

We thank the following organizations and contributors:

- [Depot CI](https://depot.dev) for sponsoring our continuous integration.
- [Owner.com](https://www.owner.com) for sponsoring the initial work.
- [Max Schwenk](https://github.com/maschwenk) for the port.
- [@walkerdb](https://github.com/walkerdb) for improvements to the project.
- [@JiangWeixian](https://github.com/JiangWeixian) for transferring the `tsrs` npm package name.

We extend special thanks to the [TypeScript team](https://www.typescriptlang.org/) for their support.

## License

`tsrs` is a derivative of TypeScript (Copyright Microsoft Corporation), licensed under [Apache-2.0](https://github.com/maschwenk/tsrs/blob/main/LICENSE).
The bundled `lib.*.d.ts` files are TypeScript's. See [NOTICE](https://github.com/maschwenk/tsrs/blob/main/NOTICE) for attribution.
Not affiliated with or endorsed by Microsoft.
