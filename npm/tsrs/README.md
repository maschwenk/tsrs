# @maschwenk/tsrs

`tsrs` is a Rust port of the TypeScript 7 compiler and language server (the Go implementation in
microsoft/TypeScript). It does what `tsc` does: same diagnostics, same exit codes, same `tsconfig.json` handling, and
it emits by default like tsc (pass `--noEmit` to only type check). `tsrs --lsp -stdio` is the language server (what `tsgo --lsp -stdio` is). The package version names the TypeScript commit it ports: `0.1.0-ts7.1.0-dev.20260929` is tsrs 0.1.0
following TypeScript `7.1.0-dev.20260929`.

```sh
pnpm add -D @maschwenk/tsrs
pnpm exec tsrs -p path/to/project              # like `tsc -p path/to/project` (emits)
pnpm exec tsrs -p path/to/project --noEmit     # like `tsc --noEmit -p path/to/project`
pnpm exec tsrs -p . --extendedDiagnostics      # counters and timings, like tsc
pnpm exec tsrs --checkers 8                    # checker threads (default 4); --singleThreaded = 1
pnpm exec tsrs --version
pnpm exec tsrs --lsp -stdio                    # language server; editor setup: docs/LSP.md in the repository
```

Supported platforms: macOS arm64, Linux x64/arm64 (glibc). The binary comes from the optional
dependency `@maschwenk/tsrs-<os>-<arch>`, installed automatically for the current platform. To run a locally built
binary through the same launcher, set `TSRS_BINARY=/path/to/tsrs`.

`require("@maschwenk/tsrs")` exports `version`, `typescriptVersion` and `typescriptCommit`.

Licensed under Apache-2.0. tsrs is a derivative work of TypeScript (Copyright (c) Microsoft Corporation); the
bundled `lib.*.d.ts` files and the structure of the code come from that project. See `LICENSE` and `NOTICE.txt`.
