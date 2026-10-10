# Rust-owned symbol-table storage (2026-10-10)

This continues the complete ownership goal from `8b6aad4f`. It is an unfinished branch checkpoint, not a
completed graph migration or a performance change ready to land. Before starting, the earlier ownership notes,
`mem-layout.md` section 4, `mem-dense-link-tables.md`, the symbol lookup evidence in `perf-checker-cpu3.md`,
`perf-round2-followups.md`, `mem-round4.md` section 6 and `docs/RUST.md` Techniques were reviewed. The changed
constraint is the user's requirement to remove the custom memory model. This keeps the existing lookup, hash,
Bloom-filter and growth algorithms; it does not retry a rejected search algorithm or shared checker graph.

`SymbolMap` now owns its entries with `Vec<SymbolMapEntry>` instead of a custom raw allocation/reallocation
buffer. Its extra storage is a Rust enum containing either filter bits or `Box<SymbolMapExtra>`, replacing
pointer tags, raw box reconstruction and manual destruction. Rust handles buffer/record cloning and dropping,
and derives thread traits from the contents. Four unchecked `Send`/`Sync` implementations disappear. The table
header grows from 24 to 40 bytes on 64-bit targets and 16 to 20 bytes on 32-bit targets; both assertions were
checked. Packed symbol entries, symbol identity, iteration order, odd-key positions and lookup behavior remain.

The symbol and key referents still belong to the legacy graph. In particular, packed native symbol pointers and
static key references have not become owner-qualified keys or borrowed graph views. A new clone case grows an
indexed table, adds an odd key, mutates and drops the original, transfers the clone to another thread, and checks
lookup/deletion order. Its legacy graph owner stays alive: it establishes container ownership, not general graph
lifetime safety. The complete ownership goal remains active.

## Measurements

Apple M3 Max / macOS / Rust 1.99.0, fat-LTO release builds without PGO and the Xcode 14.5 SDK. The pinned Compiler
workloads use `41f652ab2df5077b1115f73eaeefc2fe9f674132`. Five interleaved rounds per binary/project/mode follow
warm-up with no other validation jobs running. `/usr/bin/time -l` records retired instructions and peak RSS.
Exit codes and stdout hashes agree in every cell. All 80 samples, source/binary checksums and validation scope
are in `rust-owned-symbol-storage-results.json`; comparisons include original Oxc `3dac06d8`, owned roots
`8b6aad4f`, this stage and pre-Oxc `18175c0e`.

| Project/mode | Instructions, `8b6aad4f` → this stage | Peak RSS, `8b6aad4f` → this stage |
| --- | --- | --- |
| Compiler, single | 2.345353 → 2.337173 G (-0.35%) | 115.02 → 115.09 MiB (+0.07%) |
| Compiler, default | 2.944563 → 2.946976 G (+0.08%) | 162.70 → 161.36 MiB (-0.83%) |
| Compiler-Unions, single | 4.880589 → 4.858001 G (-0.46%) | 119.69 → 117.02 MiB (-2.23%) |
| Compiler-Unions, default | 6.182970 → 6.187649 G (+0.08%) | 162.47 → 163.45 MiB (+0.61%) |

These local changes are small and do not clear the project's threshold for a performance change. Cumulative
single-thread instructions remain +10.62% / +5.29% against original Oxc. Default RSS remains +107.79% / +99.07%
against pre-Oxc, and +12.99% / +15.30% against original Oxc. The full instruction/memory gates still fail. No
overall speed/memory improvement or completion is claimed. Typed graph ownership must replace the remaining
address-routing/thread-arena layer before fresh full-scope measurements can establish whether it is ready to land.

## Verification

AST 21, API 77, checker 18, compiler 20, incremental 6, language service 67, project 105 (two existing ignored)
and native LSP 41 cases pass, along with two compiler compile-fail doctests. All 28 regression
fixtures pass. Full six-baseline classifications over 15,197 pinned TypeScript variants exactly match both
original Oxc and `8b6aad4f`: diagnostics 13,462; types/symbols 12,779 each; JS 13,392; JS maps 149; sourcemap text
156. There are no crashes/timeouts. Default mode (`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`)
also has identical classification sets. Fourslash has the same 4,066 pass / 63 fail / 417 skip sets over 4,546
cases with native watcher access. No capability counts change; the README tables stay unchanged.

Workspace checks, release builds, all CLI features and wasm32-wasip1 pass. The ratchet has three baseline findings and none new;
source checks pass with 24 reviewed custom thread implementations, down from 28, and no baseline increases.
Validation uses the fresh `target/rust-ownership` directory. Linux/glibc API RSS assertions, census runtime,
tsctests scenario baselines and the full emit/incremental oracle audit have not been established by this stage.

Remaining work includes owned AST/type/symbol/signature/inference/snapshot/cache referents; borrowed graph views
instead of fabricated static references; config/parser host leaks and API orchestrator roots; parse-cache
accounting; remaining unchecked thread traits; and address-routing/thread-arena/recycling/rewind compatibility
APIs. The full CLI/LSP/API/incremental/emit/oracle/lifecycle and performance audit is still required.
