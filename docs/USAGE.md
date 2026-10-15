# Using tsrs

Install tsrs in your project, then run it in place of `tsc`:

```sh
pnpm add -D tsrs
pnpm exec tsrs -p path/to/project           # compile
pnpm exec tsrs -p path/to/project --noEmit  # type check only
pnpm exec tsrs -p . --extendedDiagnostics   # counters and timings
pnpm exec tsrs --version
```

You can also run it without installing it in the project: `pnpm dlx tsrs -p .`.
Your existing `tsconfig.json` controls compilation. tsrs emits JavaScript and declarations according to those
options by default, just like `tsc`; pass `--noEmit` when you only want type checking.

See [TypeScript compatibility](COMPATIBILITY.md) for supported features and known gaps, including unsupported
commands such as `--init` and `--showConfig`.

`tsrs --watch -p .` and `tsrs -b --watch` rebuild when files change, like `tsc`. On Linux a rebuild starts when the
editor closes the saved file, so it never checks a half-written file and adds no fixed delay. macOS (FSEvents) reports
no close, so there a rebuild starts when the save's event arrives (about 13 ms after it), and waits while the file is
still empty (a save that truncated it and has not written yet) or while a tool is writing several files (a formatter,
a checkout). A program that writes one file in several pieces with pauses between them can still start two rebuilds;
the second has the right result. When FSEvents is unavailable the watch uses kqueue, which needs a file descriptor
per watched file; tsrs raises its descriptor limit as far as the system allows and says so if that is not enough.

## Performance options

Alongside the supported `tsc` flags, tsrs offers options to control performance:

- `--checkers N` sets the number of checker threads. By default tsrs uses one per core up to 16 cores, then half the
  cores, from 4 to 32; a program with fewer than 32 type-checked files per checker gets fewer, down to 4, and never
  more checkers than files. `--build` uses 4 per project. More threads can be faster but use more memory. In the
  default Go-history mode the thread count can change output in the same history-sensitive cases as tsgo; exact
  comparisons normally pass `--checkers 4` to both compilers.
- `--singleThreaded` uses one thread, for the lowest memory use.
- `--checkerCostCache <file>` remembers how long each file took to check and uses that to balance threads on the next
  locality-mode run (up to ~15% faster on repeated runs).
- `--maxMemory <size>` (for example `12G`; `--noEmit` checks only) keeps the process near that much memory: above it,
  tsrs replaces its largest checker with a fresh one, trading CPU for memory. The diagnostics are the same.
- The default scheduler uses directory-local assignment, work stealing and declaration-file splitting. Checker cache
  history independently follows tsgo. `TSRS_HISTORY=canonical` changes only cache history. Naming an assignment
  disables work stealing; `--checkerAssignment go` selects tsgo's static FENNEL assignment.
- Lazy member tables from two not-yet-merged upstream changes are off by default. `--lazyMembers` or
  `TSRS_LAZY_MEMBERS=1` enables them; the patches are in [`upstream/`](../upstream/).

For canonical caches and lazy members, use `TSRS_HISTORY=canonical TSRS_LAZY_MEMBERS=1`. This does not change checker
scheduling; known output exceptions are listed in the
[compatibility table](COMPATIBILITY.md#what-it-does-and-doesnt-do).

## Native packages

The launcher installs the appropriate optional `@ts-rs/<os>-<arch>[-<libc>]` dependency for your platform.
See the [compatibility table](COMPATIBILITY.md#what-it-does-and-doesnt-do) for released platform support.
Set `TSRS_BINARY=/path/to/tsrs` to run a locally built binary through the same launcher.

The package version names the TypeScript commit it ports: `0.10.0-ts7.1.0-dev.20260929` means tsrs 0.10.0 following
TypeScript `7.1.0-dev.20260929`.

## Other ways to use tsrs

- [Editor setup](LSP.md#editor-setup): run the language server with `tsrs --lsp -stdio`.
- [JavaScript API](NODE_API.md#javascript-api-unstable): TypeScript 7's synchronous and asynchronous APIs.
- [WebAssembly](../npm/tsrs-wasm/README.md): single-threaded compilation in Node and browsers.
- [Build from source](../CONTRIBUTING.md#build): native and WebAssembly builds and the development workflow.
