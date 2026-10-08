# wasm-build: tsrs as a WebAssembly module (crates/tsrs_wasm, npm/tsrs-wasm)

tsc as a `wasm32-wasip1` module over a host file system, with a Node host (bin `tsrs-wasm`, an in-memory `files`
API with JSON diagnostics) and a browser example. Single-threaded. Not released: `@maschwenk/tsrs-wasm` is not
published. The design follows ts-rust's wasm build (pingdotgg/ts-rust, `crates/ts_wasm`, `npm/wasm`); the code here
is written from scratch for tsrs.

## What was built, and why

| Question | Choice | Reason |
|---|---|---|
| File system | Host imports (`tsrs_host.fs` / `fs_take`), no WASI preopens | One Rust path serves Node's real fs, in-memory maps and browsers; WASI files would need a second fs implementation (a WASI fs polyfill) for memory and browsers. Reads go through `tsrs_vfs::internal::Common`, as osvfs does, so BOM, UTF-16 and symlink handling are the native code. |
| Instances | Compile once, new instance per run | tsrs keeps process-wide state (thread-local arenas, the worker pool, interned strings) and linear memory never shrinks. |
| Target | `wasm32-wasip1` | Node and browsers instantiate it directly (wasip2 is a component); only clocks, random, `fd_write`, environ and `proc_exit` are used. |
| Emit, `--build` | Yes; `-b` builds sequentially; JSON diagnostics with `-b` are refused | Emit is the native code once the host can write. |
| Threads | `tsrs_core::NO_THREADS` (false on native) | One-thread rayon pool on the calling thread, `Program::single_threaded()` always true, `--singleThreaded` forced on the parsed options, `-b` builds then reports on the calling thread. tsrs output does not depend on the checker count, so one checker is exact. |
| Crates | `tsrs_execute` (the tsc driver, moved out of `tsrs_cli`), `tsrs_wasm` (cdylib only) | The driver could not stay in the binary crate (it would pull mimalloc, the LSP, the API server and the file watcher into the module); cargo skips LTO for a crate that is also an rlib. |
| 32-bit | cfg twins | `PackedStr` is the plain reference on 32-bit and `ThinSlice` a pointer and a length (its only long form is a lazy member list's, `lazylist`); `TypeMapper` keeps 16 bytes with `u64` words and its tags at bits 40-43; `P::pack` keeps 4-aligned addresses unshifted; the symbol bloom filter uses `usize::BITS`; thread-arena chunks stop doubling at 64 MiB. Every layout assert has a 32-bit twin. 64-bit code and layout are unchanged. |
| Libraries | The 113 embedded `lib.*.d.ts` stay as they are (3.79 MB raw) | Packing them (ts-rust: LZMA, 0.31 MB) adds a decoder and a lazy path that differs from native; it is the first size follow-up. |

## ABI and file-system protocol

Exports: `tsrs_input(len) -> ptr`, `tsrs_run() -> status`, `tsrs_output() -> ptr`, `tsrs_output_len() -> len`.
Request: NUL-separated UTF-8, `cwd \0 flags` then `\0 arg` per argument; flags 1 = JSON diagnostics (reply = the
API's `DiagnosticResponse` array, UTF-16 positions), 2 = case-insensitive host fs, 4 = stdout is a tty. tsc's text
goes to WASI fd 1. A panic prints Rust's message on stderr and exits with 5, native's status for a panicked driver
thread. A trap (a shadow-stack overflow faults as an out-of-bounds access, an allocation failure aborts with
`unreachable`, and V8 throws a `RangeError` when its own stack runs out) ends the run without `proc_exit`; `core.js`
catches it, prints `error: tsrs.wasm trapped (...)` on stderr and returns 5 (`EXIT_CRASHED`), so a crash never looks
like tsc's 1 (errors, outputs skipped).

`fs(op, ptr, len) -> i32`: ops 0 Read, 1 Stat (`<f|d|o> <size> <mtime ns>`), 2 ReadDir (`<f|d|l|o><name>\0`...),
3 Realpath, 4 Write (`path\0data`; the host creates parent dirs), 5 Append, 6 Remove (recursive, missing is ok),
7 Chtimes (`path\0atime\0mtime`). Result: n >= 0 bytes staged for `fs_take(ptr)`, -1 not found, -2-n an n-byte
error text. Host fs calls count in the same `--extendedDiagnostics` `FS:` rows as osvfs.

The Node host matches native tsrs where the two could differ: cwd is `process.cwd()` (native uses
`current_dir`, the real path); write errors are Rust's `io::Error` text (`"<strerror> (os error N)"`); realpath is
`realpathSync.native` on Linux and the symlink walk elsewhere; case sensitivity is probed on `process.execPath` as
osvfs probes its executable; build-status times are UTC on wasm (no `localtime_r`), so the gate runs native with
`TZ=UTC`.

## Stack sizes (measured)

Two stacks: the shadow stack in linear memory (address-taken locals; `-zstack-size`, first in memory so an overflow
traps), and the engine's native stack, which holds every wasm frame (a Node worker's `resourceLimits.stackSizeMb`).

Shadow-stack high water, `tools/wasm/diff.mjs --stack-census` (0xA5 fill before the run, scan after):

| input | max | note |
|---|---:|---|
| testdata/regressions (24) | 16,168 B | |
| 2,000-case conformance sample | 2,226,352 B | median 16-18 KB |
| bench projects (4) | 143,616 B | |
| fixtures, 20k-term `1+1+...` | 16,007,384 B | ~800 B per level |
| fixtures, 50k-term `1+1+...` | 40,007,384 B | the largest |

Rule (decided before measuring): shadow = max(32 MiB, 2 x the largest high water, rounded up to 8 MiB) = **80 MiB**.
Worker stack: every fixture matches native at 128 MB and the 50k-term one fails at 64 MB, so the smallest of
{64, 128, 256, 512} that passes at half its size is **256 MB** (ts-rust uses 256 too). At 80 MiB / 256 MB the module
handles a 100k-term expression (80.0 MB of shadow stack); 150k overflows the shadow stack (exit 5 with the trap
message), which native handles (exit 2).

## Size

`tools/wasm/build.sh` (rustc 1.99.0, fat LTO, one codegen unit, panic = abort, binaryen 133, build paths remapped and
checked). gzip -9; brotli quality 11 (node zlib).

| module | raw | gzip | brotli |
|---|---:|---:|---:|
| opt-level z, before wasm-opt | 10,370,230 | 2,565,028 | 1,731,226 |
| opt-level z, after `wasm-opt --flatten --rereloop -Oz -Oz` | 8,166,593 | 2,279,300 | 1,556,501 |
| opt-level s + inlinehint 150, before wasm-opt | 11,066,879 | 2,794,410 | 1,863,460 |
| **opt-level s + inlinehint 150, after `wasm-opt -Os` (shipped)** | **9,102,482** | **2,560,208** | **1,730,488** |
| ts-rust's module (copied from the baseline study, 2026-10-08) | 4,706,298 | 2,056,521 | 1,666,456 |

The embedded libraries are 3,791,746 bytes raw of that (ts-rust packs its libraries into 0.31 MB). Imports:
`tsrs_host.{fs,fs_take}` and WASI `random_get environ_get environ_sizes_get clock_time_get fd_close fd_fdstat_get
fd_filestat_get fd_prestat_get fd_prestat_dir_name fd_read fd_write path_open proc_exit sched_yield`; the `path_open`
/ `fd_read` family is reachable only from debug environment variables that read files (the JS shim answers them
with errors).

## Speed and memory

`tools/wasm/bench.mjs`, this Mac (Apple M5 Max, macOS 26.6, Node 24.20), 5 interleaved runs per engine and project,
load average 8-10 (other agents share the machine). Flags `--noEmit --incremental false --pretty false`; native
gets `--singleThreaded`. Cold = median first `tsc()` call in a new Node process (includes compiling the module);
warm = median of 5 later calls in one process. Native: median of 5 `/usr/bin/time -l` runs. Linear memory includes
the 80 MiB shadow stack. Seconds.

| project | native wall | native peak RSS | s cold | s warm | s warm / native | z warm | ts-rust warm | s linear memory | s Node maxRSS | ts-rust Node maxRSS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| xstate-main | 0.46 | 196 MiB | 1.06 | 0.92 | 2.01x | 1.10 | 3.50 | 264 MiB | 330 MiB | 616 MiB |
| webpack | 0.82 | 316 MiB | 2.01 | 1.84 | 2.25x | 2.26 | 7.50 | 420 MiB | 443 MiB | 970 MiB |
| Compiler | 0.13 | 59 MiB | 0.34 | 0.25 | 1.89x | 0.29 | 0.70 | 134 MiB | 214 MiB | 276 MiB |
| Compiler-Unions | 0.24 | 61 MiB | 0.52 | 0.42 | 1.76x | 0.51 | 1.13 | 136 MiB | 216 MiB | 276 MiB |

Geomean warm: s 0.65 s, z 0.78 s, ts-rust 2.14 s. ts-rust's webpack output differs from native tsrs and tsgo
(849 against 840 errors, baseline study), so its webpack time is not an identical-output comparison.

The table's ts-rust column was measured in that interleaved run (its reps: xstate-main 3.47-3.53 s, webpack
7.47-7.57 s), but it is the fastest of three sessions, and absolute times on this shared Mac move with the load.
Two more sessions on 2026-10-08: the same command, native and the s module built at 59d1f47 (main merged in), 5
interleaved runs, median [min-max] of the warm calls in seconds. Session B ran at load 21-25; session C started at 14
and ended at 7.

| project | session | native | s warm | ts-rust warm | ts-rust cold | ts-rust / s |
|---|---|---:|---:|---:|---:|---:|
| xstate-main | A (table above) | 0.46 | 0.92 [0.92-0.94] | 3.50 [3.47-3.53] | 3.86 | 3.79x |
| | B | 0.53 | 2.32 [1.33-2.71] | 8.67 [5.86-9.02] | 4.82 | 3.74x |
| | C | 0.58 | 1.18 [1.10-1.22] | 4.54 [4.13-4.81] | 4.92 | 3.84x |
| webpack | A | 0.82 | 1.84 [1.83-1.86] | 7.50 [7.47-7.57] | 8.10 | 4.07x |
| | B | 1.22 | 2.18 [2.14-2.38] | 11.53 [10.96-12.96] | 11.68 | 5.28x |
| | C | 1.10 | 2.23 [2.19-2.42] | 10.25 [10.12-10.67] | 11.06 | 4.60x |
| Compiler | A | 0.13 | 0.25 [0.24-0.25] | 0.70 [0.69-0.72] | 0.85 | 2.86x |
| | B | 0.17 | 0.31 [0.29-0.32] | 0.90 [0.79-0.92] | 1.09 | 2.92x |
| | C | 0.14 | 0.27 [0.27-0.28] | 0.77 [0.76-0.78] | 0.95 | 2.83x |
| Compiler-Unions | A | 0.24 | 0.42 [0.42-0.43] | 1.13 [1.12-1.14] | 1.30 | 2.66x |
| | B | 0.35 | 0.49 [0.48-0.49] | 1.33 [1.26-1.52] | 1.86 | 2.71x |
| | C | 0.28 | 0.48 [0.47-0.48] | 1.33 [1.29-1.46] | 1.57 | 2.78x |

ts-rust's warm time ranges from 3.50 to 8.67 s on xstate-main and from 7.50 to 11.53 s on webpack across the
sessions, and its warm median can exceed its cold one under load (session B, xstate-main). The ratio to tsrs's
module is steadier: 3.7-3.8x on xstate-main, 4.1-5.3x on webpack, 2.8-2.9x on Compiler, 2.7-2.8x on
Compiler-Unions. Quote the ratio, not the absolute ts-rust times.

**Opt-level** (rule fixed before measuring: ship s if its geomean warm time is at least 10% better than z's and its
gzip size at most 1.35x z's): s is 17% faster (0.65 / 0.78) at 1.12x the gzip size, so the module ships with
opt-level s and `-C llvm-args=-inlinehint-threshold=150`.

## The differential gate (`tools/wasm/gate.sh`, CI job `wasm`)

Native `--singleThreaded` against the module at the same absolute paths: exit code, stdout bytes, every written file
(sha256; symlinks as targets), stderr when both exit 0. Only build-status time prefixes are normalised.

| set | result |
|---|---|
| testdata/regressions | 25/25 same (native equals each `expected.txt`) |
| 2,000-case conformance sample (`tools/wasm/sample-2000.txt`, `tsrs-test materialize`), node fs | 1,998/1,998 same, 2 skipped (runExternalCode) |
| the same, in-memory fs | 1,993/1,993 same, 7 skipped (the 2, plus 5 cases with symlinks) |
| xstate-main (0 errors), webpack (840), Compiler (43), Compiler-Unions (41), 3 repeats | 4/4 same each time; checkouts unmodified |
| emit on Compiler and Compiler-Unions (`--declaration --sourceMap --declarationMap`) | 2/2 same, 300 files |
| fixtures (`tools/wasm/fixtures`: `-b` graph with up-to-date/verbose/edit/clean/force, incremental edits, maps + BOM + UTF-16 input, symlinked package, case-mismatched imports, write errors, `--checkers 4` cross-check, deep expressions to 50k terms) | 12/12 same |

138 of the 1,998 materialized cases carry an option the command line cannot express (the harness-only
`noTypesAndSymbols`, `fullEmitPaths`, tsconfig-only options); the case still runs, with that option dropped on both
sides.

## Not supported

`--watch` (refused as natively), `--lsp`, `--api`, JSON diagnostics with `--build`, `--checkerCostCache` (refused),
more than one checker or builder (accepted; everything runs on one thread), `--locale` (ignored natively too),
programs over 4 GiB of linear memory.

## 32-bit rules for new code

- A `size_of` assert gets a `cfg(target_pointer_width = "32")` twin next to the 64-bit one (measure the size with
  `cargo check --target wasm32-wasip1`; a `[(); 0] = [(); size_of::<T>()]` probe prints it).
- No 48-bit packing in a `usize`, and no tag bits in the low 3 bits of a plain pointer: on wasm32 arena objects are
  only 4-aligned. Use `u64` words, as `TypeMapper` and `P::pack` do.
- A new thread spawn needs a `tsrs_core::NO_THREADS` (or `single_threaded`) path: on `wasm32-wasip1`
  `std::thread::spawn` returns `Unsupported`, and a panic cannot be caught.
- `cargo check -p tsrs_wasm --target wasm32-wasip1 --locked` (with `-D warnings`) is in CI.

## Reproduce

```
rustup target add wasm32-wasip1 --toolchain 1.99.0; brew install binaryen     # binaryen 133
cargo build --release --locked -p tsrs_cli -p tsrs_testrunner
tools/wasm/build.sh                       # OPT=z for the size build; prints the size table
npm test --prefix npm/tsrs-wasm
tools/wasm/gate.sh                        # BENCH_SOLUTIONS / BENCH_SUITE point at the bench checkouts
node tools/wasm/bench.mjs --native target/release/tsrs --wasm s=npm/tsrs-wasm/tsrs.wasm \
  --project "webpack=<dir>:-p . --noEmit --incremental false --pretty false" [--ts-rust <repo>=<module>]
node npm/tsrs-wasm/examples/browser/serve.mjs   # the browser example
```

## Follow-ups (each needs its own measurement)

LZMA-packed libraries (ts-rust: 3.79 MB to 0.31 MB raw); function reordering for compression (ts-rust: -96 KB
gzip); `dyn` trims after a twiggy census; a `wasm32-wasip1-threads` build with real checker threads
(SharedArrayBuffer, a thread-spawn host, `NO_THREADS = false`); npm release wiring (owner); running the module in
tsrs's own conformance runner; WASI `std::fs` as a second host for wasmtime users.
