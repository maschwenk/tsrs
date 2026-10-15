# `tsrs`

[![npm](https://img.shields.io/npm/v/tsrs)](https://www.npmjs.com/package/tsrs)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](https://github.com/oxc-project/tsrs/blob/main/LICENSE)

`tsrs` is a drop-in replacement for the TypeScript 7 compiler and language server, written in Rust.
Our goal is 100% conformance, with much lower memory use and faster type checking.

## Get started

```sh
pnpm add -D tsrs
pnpm exec tsrs --noEmit
```

You can try `tsrs` today and give us feedback on performance, memory usage, and conformance.
Try checking and emitting code in the [browser playground](https://tsrs-playground.void.app).

## Conformance

`tsrs` currently targets the main branch of [microsoft/TypeScript](https://github.com/microsoft/typescript).
We aim to release a stable version against TypeScript 7.1 with 100% conformance. In our testing on existing projects,
we have yet to find a correctness bug in `tsrs`.

There are known order issues: the pinned Go compiler can produce different diagnostics and traces depending on
parallel parse, resolution, and check order, while `tsrs` canonicalizes some checker history by default. For the
closest comparison, use `--checkerAssignment go --noLazyMembers`; tsgo parser and resolver races can still vary
between runs.

`--init` and `--showConfig` are not supported. See [compatibility and evidence](https://github.com/oxc-project/tsrs/blob/main/docs/COMPATIBILITY.md)
for supported features and known gaps.

<a href="https://github.com/oxc-project/tsrs/blob/main/bench/RESULTS.md">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/oxc-project/tsrs/main/bench/large-projects-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/oxc-project/tsrs/main/bench/large-projects.svg">
    <img alt="Performance" src="https://raw.githubusercontent.com/oxc-project/tsrs/main/bench/large-projects.svg">
  </picture>
</a>

## Extra features

[`@tsrs/wasm`](https://github.com/oxc-project/tsrs/blob/main/npm/tsrs-wasm/README.md) provides WebAssembly support
targeting `wasm32-wasip1` (WASI Preview 1).

## Learn more

- [CLI usage and memory tuning](https://github.com/oxc-project/tsrs/blob/main/docs/USAGE.md)
- [Editor setup](https://github.com/oxc-project/tsrs/blob/main/docs/LSP.md#editor-setup)
- [Node API (unstable)](https://github.com/oxc-project/tsrs/blob/main/docs/NODE_API.md)
- [WebAssembly for Node and browsers](https://github.com/oxc-project/tsrs/blob/main/npm/tsrs-wasm/README.md)
- [Benchmarks and methodology](https://github.com/oxc-project/tsrs/blob/main/bench/README.md)
- [Release notes](https://github.com/oxc-project/tsrs/blob/main/docs/STATUS.md)
- [Engineering notes](https://github.com/oxc-project/tsrs/blob/main/notes/README.md)

## Contributing

Issues and pull requests are currently disabled until we have the bandwidth to triage them.

See the [contributing guide](https://github.com/oxc-project/tsrs/blob/main/CONTRIBUTING.md)
for building from source and running the suites, the [Code of Conduct](https://github.com/oxc-project/tsrs/blob/main/CODE_OF_CONDUCT.md), and
[security policy](https://github.com/oxc-project/tsrs/blob/main/SECURITY.md) for reporting vulnerabilities.

## Acknowledgments

We thank the following organizations and contributors:

<p>
  <a href="https://depot.dev">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/oxc-project/tsrs/main/assets/sponsors/depot-dark.svg">
      <img alt="Depot" src="https://raw.githubusercontent.com/oxc-project/tsrs/main/assets/sponsors/depot-light.svg" height="32">
    </picture>
  </a>
  &nbsp;&nbsp;&nbsp;&nbsp;
  <a href="https://www.owner.com">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/oxc-project/tsrs/main/assets/sponsors/owner-dark.svg">
      <img alt="Owner.com" src="https://raw.githubusercontent.com/oxc-project/tsrs/main/assets/sponsors/owner-light.svg" height="32">
    </picture>
  </a>
</p>

- [Depot CI](https://depot.dev) for sponsoring our continuous integration.
- [Owner.com](https://www.owner.com) for sponsoring the initial work.
- [Max Schwenk](https://github.com/maschwenk) for the port.
- [@walkerdb](https://github.com/walkerdb) for improvements to the project.
- [@JiangWeixian](https://github.com/JiangWeixian) for transferring the `tsrs` npm package name.

We extend special thanks to the [TypeScript team](https://www.typescriptlang.org/) for their support.

## License

`tsrs` is a derivative of TypeScript (Copyright Microsoft Corporation), licensed under [Apache-2.0](https://github.com/oxc-project/tsrs/blob/main/LICENSE).
The bundled `lib.*.d.ts` files are TypeScript's. See [NOTICE](https://github.com/oxc-project/tsrs/blob/main/NOTICE) for attribution.
Not affiliated with or endorsed by Microsoft.
