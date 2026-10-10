# tsrs

`tsrs` is a Rust port of the TypeScript 7 compiler and language server (the Go implementation in
microsoft/TypeScript). It does what `tsc` does: same diagnostics, same exit codes, same `tsconfig.json` handling, and
it emits by default like tsc (pass `--noEmit` to only type check). `tsrs --lsp -stdio` is the language server (what `tsgo --lsp -stdio` is). The package version names the TypeScript commit it ports: `0.10.0-ts7.1.0-dev.20260929` is tsrs 0.10.0
following TypeScript `7.1.0-dev.20260929`.

```sh
pnpm add -D tsrs
pnpm exec tsrs -p path/to/project              # like `tsc -p path/to/project` (emits)
pnpm exec tsrs -p path/to/project --noEmit     # like `tsc --noEmit -p path/to/project`
pnpm exec tsrs -p . --extendedDiagnostics      # counters and timings, like tsc
pnpm exec tsrs --checkers 8                    # checker threads (default: one per core up to 16, then half the cores, 4-32; fewer, down to 4, for small projects; 4 per --build project); --singleThreaded = 1
pnpm exec tsrs --version
pnpm exec tsrs --lsp -stdio                    # language server; editor setup: docs/LSP.md in the repository
```

Supported platforms: macOS arm64, Linux x64/arm64 (glibc). The binary comes from the optional
dependency `@ts-rs/<os>-<arch>`, installed automatically for the current platform. To run a locally built
binary through the same launcher, set `TSRS_BINARY=/path/to/tsrs`.

`require("tsrs")` exports `version`, `typescriptVersion` and `typescriptCommit`.

## JS API (unstable)

The package also ships TypeScript 7's programmatic API, copied unchanged from microsoft/TypeScript
`packages/typescript` at the commit the binary ports (see `UPSTREAM.json`), under the same subpaths as the `typescript`
7 package:

| subpath | contents |
| --- | --- |
| `tsrs/unstable/sync` | `API` with synchronous calls (MessagePack over the server's stdio) |
| `tsrs/unstable/async` | `API` with promise-returning calls (JSON-RPC) |
| `tsrs/unstable/ast` (`/is`, `/factory`, `/utils`, `/scanner`, `/visitor`, `/clone`) | AST types, guards, factory and traversal |
| `tsrs/unstable/fs`, `/proto` | filesystem callbacks/request filesystems, protocol types |

```js
import { API } from "tsrs/unstable/sync";

const api = new API({ cwd: process.cwd() });            // spawns `tsrs --api`; tsserverPath overrides the binary
const snapshot = api.createSnapshot({ openProject: "/abs/path/tsconfig.json" });
const project = snapshot.getConfiguredProject("/abs/path/tsconfig.json");
console.log(project.program.getSemanticDiagnostics("/abs/path/src/index.ts"));
project.program.emit();                                  // follows the project's compiler options
api.close();
```

The code is TypeScript's, with one intentional difference: if the server process dies, the async API rejects
every pending and later request with an error starting `API server connection lost: ` instead of leaving them
pending forever (see `npm/README.md` in the repository).

These entry points are unstable upstream and here. The API is served by the `tsrs` binary (`tsrs --api`), whose
coverage of the protocol is still incomplete; `npm/sdk/METHODS.md` in the repository tracks it per method.
TypeScript consumers need `esnext.disposable` (or a newer lib) for the declarations' `Symbol.dispose` members.
The JS API runs on the package's minimum Node (16.20; checked on 16.20, 18, 20, 22 and 24). Where Node has no
`Symbol.dispose` (before 18.18/20.4), call `close()` / `dispose()` instead of `using`.

Licensed under Apache-2.0. tsrs is a derivative work of TypeScript (Copyright (c) Microsoft Corporation); the
bundled `lib.*.d.ts` files and the structure of the code come from that project. See `LICENSE` and `NOTICE.txt`.
