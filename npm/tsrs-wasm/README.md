# @maschwenk/tsrs-wasm

tsrs (a Rust port of the TypeScript 7 compiler) built as a WebAssembly module, with a small host for Node and
browsers. It runs `tsc` single-threaded; its output is byte-identical to native `tsrs --singleThreaded` on the
differential gate in the repository (`tools/wasm/diff.mjs`; numbers in
[notes/wasm-build.md](https://github.com/maschwenk/tsrs/blob/main/notes/wasm-build.md)). The package version names the
TypeScript commit it ports, as `@maschwenk/tsrs`'s does: `0.9.0-ts7.1.0-dev.20260929` is tsrs 0.9.0 following
TypeScript `7.1.0-dev.20260929`.

## Command line (Node 22+)

```
npx -y @maschwenk/tsrs-wasm -p .             # or `npm i -D @maschwenk/tsrs-wasm`, then `tsrs-wasm -p .`
npx -y @maschwenk/tsrs-wasm --version
```

It reads and writes the real file system, prints what `tsc` prints and exits with its status. A crash (a panic, or a
trap such as a stack overflow or running out of memory) exits with 5 and a message on stderr; tsc's own statuses are
0-4.

From a checkout of the repository, `tools/wasm/build.sh` writes `npm/tsrs-wasm/tsrs.wasm` and
`node npm/tsrs-wasm/bin/tsrs-wasm.js -p .` runs it.

## API

```js
import { tsc } from "@maschwenk/tsrs-wasm";

// The real file system (Node)
const { exitCode, stdout } = await tsc(["-p", "."], { cwd: "/path/to/project" });

// In-memory files, diagnostics as JSON (the TypeScript API's diagnostic shape, UTF-16 positions)
const r = await tsc(["/src/a.ts", "--noEmit"], {
    cwd: "/src",
    files: { "/src/a.ts": "const s: string = 1;" },
    diagnostics: "json",
});
r.diagnostics; // [{ fileName: "/src/a.ts", pos: 6, end: 7, code: 2322, category: 1, text: "...", ... }]
r.files;       // what the compiler wrote, path -> text (with emit)
```

Options: `cwd`, `files`, `env` (none by default), `diagnostics: "text" | "json"`, `tty`, `stdout` / `stderr`
(`"inherit"` writes to this process's fds; collected otherwise), `caseInsensitive`, `stackSizeMb`. The result has
`exitCode`, `stdout`, `stderr`, `diagnostics?`, `files?` and `memoryBytes` (the module's linear memory at the end,
which is its peak). `@maschwenk/tsrs-wasm/core` exposes the lower layer: `runTsc(module, request, host, io)` over
any `HostFileSystem`, and `memoryFileSystem(files)`.

## Runtime notes

- **Node**: every call runs in a fresh worker thread with a 256 MB stack (`resourceLimits.stackSizeMb`): the checker
  recurses deeply, and every wasm frame also uses the engine's native stack, so the main thread's stack is too
  small. The module is compiled once per process; each call gets a new instance (the compiler keeps process-wide
  state, and linear memory never shrinks).
- **Bun** ignores the worker stack limit, so there the call runs on the calling thread.
- **Browsers**: `browser.js` (the package's default export condition) works over in-memory files only. Run it in a
  Web Worker (see `examples/browser`, served with `node examples/browser/serve.mjs`). Safari's worker stack is
  smaller than Chrome's and Firefox's, so the deepest expressions overflow there first.
- One instance holds at most 4 GiB (wasm32). The bench projects (webpack: 420 MiB of linear memory) fit; a program
  that needs more than 4 GiB does not.

## Not supported

`--watch`, `--lsp`, `--api`, JSON diagnostics with `--build`, more than one checker or
builder (the flags are accepted; everything runs on one thread), `--locale` (ignored natively too).
