# Rust-owned link storage continuation (2026-10-10)

## Scope and status

The full objective remains removal of the Go-style memory model across CLI, LSP, API and incremental versions.
This stage is based on `61a9d268` and continues the implementation in `rust-owned-arenas.md`. It is an unfinished
migration branch: the AST/type graphs and production snapshot ownership still use the legacy layer, and the
performance gates below have not passed. This stage does not establish that the full objective is achieved.

All 26 previously arena-backed checker link-table fields now own their record storage; the reference-kind table
already stored its records directly in Rust hash-table slots. The five core link-store fields used by node builders
and emit resolvers also own their records, and the unused public paged store follows the same ownership contract.

- `IdLinkStore` keeps compact 16-bit offsets and the dense fallback, but owns groups through boxes and values
  through a typed vector. No chunk pointer or `PSlot::nth` remains. Keys retained across recursive resolution
  validate the table identity before access.
- `InlineIdStore` owns 32-value groups in a typed vector and stores four-byte local group keys in its two-level
  index. Exported keys include the group owner and offset; wide IDs have a separate qualified value key.
  Borrows cannot overlap vector growth. Existing semantic ID assignment and group-default behavior are retained.
- Core `LinkStore` and `PagedLinkStore` use owned typed vectors and `RefCell`-checked borrowed guards for shared
  node-builder receivers. Callbacks retain keys rather than guards. Clearing drops every record and gives the
  replacement vector a new identity, so a stale key cannot select a replacement record.
- `OwnedMap` and `OwnedPackedMap` replace every checker `GoMap`/`GoPackedMap`. They own lazy boxed collections
  with ordinary Rust destructors. Nil reads, hash functions, insertion order and the scratch-capacity policy are
  preserved. The unused raw map-alias API is removed; its only caller reset a map to nil and now calls `reset()`.
- The map conversion is necessary for owner correctness: their old address-based `scratch_contains` allocation
  selection could not identify records moved into Rust vectors. The new map lives and drops with its record.
- `ValueSymbolLinks` replaces erased pointers and low-bit mode tags with a Rust enum and an owned boxed rare tail.
  Typed fields preserve plain/synthetic/tail transitions. The record is now 40 bytes on 64-bit targets (previously
  24) and 20 on 32-bit targets (previously 12). The lazy map fields likewise cost 16 bytes instead of 8 on 64-bit
  targets. These interim size costs must be addressed alongside the remaining graph migration.
- All new storage modules and the rewritten core link-store module forbid unsafe code. `P` lookup keys and graph
  edges inside records remain legacy: owning their container does not prove ownership of their referents.

The earlier dense-link results (`mem-dense-link-tables.md`, `mem-link-tables-landed.md`), `docs/RUST.md` Techniques,
`perf-round2-followups.md` and `mem-round4.md` were reviewed. This retains the measured ID index layouts; it changes
ownership and access, under the user's full memory-model migration requirement. It does not retry shared checker
graphs, inference rollback or a speculative performance optimization.

## Local measurements

Same Apple M3 Max / macOS / Rust / locked fat-LTO build setup and pinned Compiler workloads as the first note.
Five interleaved rounds per project/mode/binary, after warm-up, use `/usr/bin/time -l`; single adds
`--singleThreaded`, default uses the CLI's ordinary checker count. The current implementation is compared with
original Oxc `3dac06d8` and pre-Oxc `18175c0e`. All 60 recorded outputs are byte-identical and return status 2.
Raw rows, executable hashes and current Rust source hashes are in `rust-owned-links-results.json`.

| Project | Mode | Instructions (G), Oxc → owned | Change | Peak RSS (MiB), Oxc → owned | Change |
| --- | --- | ---: | ---: | ---: | ---: |
| Compiler | single | 2.113277 → 2.136932 | +1.119% | 114.55 → 114.64 | +0.08% |
| Compiler | default | 2.479093 → 2.508953 | +1.204% | 143.98 → 146.16 | +1.51% |
| Compiler-Unions | single | 4.613661 → 4.634037 | +0.442% | 104.11 → 104.78 | +0.65% |
| Compiler-Unions | default | 5.716633 → 5.730574 | +0.244% | 143.25 → 145.61 | +1.65% |

The Compiler single-thread instruction increase exceeds the 1% incremental limit. Default-mode RSS increases
are 2.17 MiB and 2.36 MiB, both over 1% and 2 MiB. Default RSS is still +87.6% / +77.5% against pre-Oxc
(77.92 → 146.16 MiB / 82.02 → 145.61 MiB). These measurements fail the incremental regression gate and the full
migration's memory-preservation requirement. Do not land this as a performance change. They are cumulative
measurements, not an isolated attribution of each container or record-size change. The short wall times are not
used to claim a speed change. Linux counts, larger pinned projects and comparable headline publishes remain
required for the final implementation.

## Verification

- Workspace, wasm32-wasip1 and checker `site-counts,assignment-stats` checks pass with no warnings. The lint
  ratchet passes with 3 baseline findings and none new; source inventory is unchanged at 31 reviewed custom
  Send/Sync implementations. No baseline increase or generated-code change is needed.
- Core: 101 unit cases and 5 compile-fail cases; checker: 18; compiler: 9; incremental: 4; project: 102 with 2
  existing ignored cases; API: 77; LSP: 41. Ownership cases cover retained keys through growth/dense conversion,
  narrow/wide foreign keys, partial-construction unwinding, dropping map values and clearing stale keys. The
  original value-symbol state-transition test still passes. All 28 regression fixtures pass.
- All six result sets are identical to the original Oxc corpus over 15,197 pinned TypeScript variants, including
  pass/fail/skip classifications. Totals remain diagnostics 13,462; types/symbols 12,779 each; JS 13,392; JS maps
  149; sourcemap text 156. Final runs have no crashes or timeouts. The default-history types/symbols configuration
  is compared separately against its matching baseline. No capability count changes are expected or claimed.
- LSP watchers run outside the sandbox with pinned `ts-ref` fixtures. The five Linux/glibc API RSS assertions
  remain excluded and unvalidated on macOS. The full fourslash runtime and Linux memory gates are not run here.

## Next required work

Generate and wire safe per-kind AST and type payload tables, migrate parser/binder/checker accessors and owned
source text/slices, then retain concrete file versions and graph owners through program snapshots, lazy shared
initialization, checker pools and API/LSP handles. This must replace production lifetime boundaries, not merely
add storage that no production code uses. The fresh memory and instruction measurements need to be repeated on
that path and the interim layout costs recovered.

Completion remains contradicted by `ptr.rs` (raw `P` and unchecked thread traits), `arena.rs` (implicit/thread
owners and address registry), AST/type payload access (fabricated static borrows and raw casts), and
`program.rs` (`Box::leak` and `free_program`). The following stage in `rust-owned-program-data.md` removes
`SharedProgramData`, `free_unshared_program` and host manual frees; the program root still needs migration.
Once the remaining callers migrate, remove the
compatibility free/recycle/checkpoint APIs and allocation ownership recovery, then verify the complete CLI,
LSP/API, incremental, emit, fourslash, conformance and performance scope.
