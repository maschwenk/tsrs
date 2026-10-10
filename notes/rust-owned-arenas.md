# Rust-owned arena pilot (2026-10-10)

## Status and boundary

This is a measured first stage, **not the completed memory-model migration**. The worktree is based on
`codex/oxc-allocator-migration` at `3dac06d8`; the pre-migration comparison is `18175c0e`.

The safe storage implementation and 24 checker link tables are migrated. The complete parse/bind/check pipeline
runs through those tables, including recursive signature/type resolution and language-service cache restoration.
The AST, binder symbols, types, signatures, specialized id link stores, parse caches, program owners, snapshots,
emit scratch regions and API handles still use the legacy graph. No compatibility API or lifetime boundary has
been deleted prematurely.

The pilot meets the incremental regression limits against Oxc on the two measured projects. It does **not** meet
the plan's overall memory requirement against the pre-Oxc implementation. Oxc already raises default-mode peak
RSS by 75–85% there, and migrating these link tables does not recover that loss. Do not present this branch as a
completed, performance-preserving migration or merge it on these results. This is not evidence that a fully
indexed graph cannot meet the requirement: most of that graph has not been converted.

## Ownership implemented

- `arena_owner.rs` forbids unsafe code. It replaces the branch's unused heterogeneous raw-pointer API with
  homogeneous `Vec<T>` builders and sealed `Box<[T]>` tables retained by `Arc`. Values run ordinary destructors.
- `LocalKey<T>` is four bytes, including `Option`; owner-qualified `ArenaKey<T>` is eight bytes, including
  `Option`. Table identities and slots never wrap or get reused. Qualified lookup rejects another owner. A local
  key must remain paired with its original table; it is not a cross-owner identity.
- Access borrows an explicit store. A live reference cannot survive builder mutation or owner destruction.
  `Send`/`Sync` follow the stored type's traits; seven custom thread-trait implementations were removed.
- The 22 generic `LinkStore` fields and the two compact `KeyedLinkStore` fields own their records. Hash-table
  buckets retain four-byte local slots; recursive callers retain qualified keys and resolve them after recursive
  work. No shared references into a growing vector are retained. Signature-help cache restoration now retains
  keys instead of pointers to link records.
- Go's node/symbol/type numbering and algorithm order remain separate from these storage IDs. Lookup keys still
  use the legacy AST/symbol identities. Existing `P` fields inside records do not become safe just because their
  containing vector is owned; those edges still need migration.

The reviewed prior results were `docs/RUST.md` Techniques, `notes/perf-round2-followups.md`, and
`notes/mem-round4.md` (the reviewed revision has sections 1–6). This does not retry inference-scope rollback or
checker-graph sharing. The changed constraint is explicit owner-borrowed access with no new custom unsafe, not a
different Go checker algorithm.

## Measurements

Apple M3 Max, 14 cores, 36 GiB RAM, macOS 27.0.1, Rust 1.99.0. Both sides are locked release builds, fat LTO,
without PGO, using the matching Xcode 14.5 SDK to build mimalloc. Workloads are `Compiler` and `Compiler-Unions`
from typescript-benchmarking `41f652ab2df5077b1115f73eaeefc2fe9f674132`, as pinned in `bench/projects.json`.

Each comparison ran five interleaved rounds after a warm-up per binary/project/mode:

```sh
/usr/bin/time -l <binary> -p <project> --noEmit --incremental false --pretty false
# Single-threaded mode adds --singleThreaded; default mode adds no checker flag.
```

Tables show medians and changes in the medians. Raw counters, diagnostic hashes and binary hashes are in
`rust-owned-arenas-results.json`. All measured commands returned the same expected status 2 and byte-identical
diagnostic output. These short runs and macOS counters are an initial local gate, not a replacement for Linux
`bench/count.py`, the larger projects, or two comparable default-mode headline publishes. No wall-time gain is
claimed. This branch is not ready for a performance landing; the gain is below the repository's 5% memory bar.

### Pre-Oxc → original Oxc branch

| Project | Mode | Instructions (G) | Change | Peak RSS (MiB) | Change |
| --- | --- | ---: | ---: | ---: | ---: |
| Compiler | single | 2.146376 → 2.115575 | -1.435% | 57.52 → 114.55 | +99.16% |
| Compiler | default | 2.526129 → 2.485790 | -1.597% | 77.34 → 142.94 | +84.81% |
| Compiler-Unions | single | 4.822491 → 4.617453 | -4.252% | 59.91 → 104.09 | +73.76% |
| Compiler-Unions | default | 5.970922 → 5.723339 | -4.146% | 81.97 → 143.64 | +75.24% |

The inherited branch removes compressed pointers, recycling, partial rewind and lazy declaration-member
conversion, and records every allocation's address and size. Registered regions additionally insert allocations
into the global address-range map. These are code-level differences worth profiling; the totals above do not
isolate how much memory each one causes.

### Original Oxc → safe generic and keyed link tables

| Project | Mode | Instructions (G) | Change | Peak RSS (MiB) | Change |
| --- | --- | ---: | ---: | ---: | ---: |
| Compiler | single | 2.115546 → 2.120820 | +0.249% | 114.55 → 113.03 | -1.32% |
| Compiler | default | 2.486840 → 2.496886 | +0.404% | 143.73 → 140.53 | -2.23% |
| Compiler-Unions | single | 4.616130 → 4.623591 | +0.162% | 104.11 → 105.34 | +1.19% |
| Compiler-Unions | default | 5.727075 → 5.730308 | +0.056% | 142.66 → 143.61 | +0.67% |

The single-threaded Compiler-Unions RSS increase is 1.23 MiB, below the gate's additional 2 MiB threshold. Both
default-mode RSS changes are within the regression limit. This supports the borrowed-access pattern in these
tables; it does not establish a result for AST/type payload lookup, file ownership, or the complete compiler.

## Verification

- Core ownership cases cover cyclic keys, vector growth, foreign/expired owners, record destructors, abandoned
  construction, shared snapshots, concurrent reads, `OnceLock` lazy initialization and teardown. Four compile-fail
  examples reject escaping borrows, mutation during a borrow, cross-type keys, and sharing non-`Sync` records.
- All checker unit cases pass, including 60,000 scrambled keyed insertions through rehashing. Core: 97 pass;
  checker: 10; compiler: 9; project: 102 with 2 existing ignored cases; API: 77; LSP: 41.
- The pinned TypeScript `b85298b6a81f772d080b0455de0ca9d744cd6fd6` corpus ran with all six baselines. Original Oxc
  and the final pilot have identical passing-case sets: diagnostics 13,462; `.types` and `.symbols` 12,779 each;
  `.js` 13,392; `.js.map` 149; `.sourcemap.txt` 156. No crashes or timeouts in the final 120-second-timeout runs.
  The types/symbols-only configuration separately retains its 13,458 diagnostic passes. The counts differ by
  runner configuration, not by this change. The README capability table therefore does not change.
- Default checker mode with `TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical` also has identical
  passing/failing sets against Oxc, including the existing canonical `.types`/`.symbols` difference for
  `objectLiteralNormalization`.
- Workspace and wasm32-wasip1 checks, lint ratchet, source inventory and all 28 regression fixtures pass. No baseline
  increase or generated-code change is needed.
- The API RSS test binary was found to call glibc's `malloc_trim` unconditionally and to read Linux `/proc`.
  It is now compiled only for Linux/glibc; its five RSS assertions were **not validated on macOS**. The other
  API lifetime/concurrency tests run on macOS. LSP FSEvents tests passed outside the sandbox; the first sandboxed
  run could not register its real watchers. Pinned LSP reference fixtures were supplied through `ts-ref`.

## Remaining work

1. Resolve the inherited pre-Oxc memory regression before a broader rollout or landing. Measure on the larger
   pinned projects and Linux; choose a migration path that does not take the native-pointer compatibility
   backend's memory increase as an acceptable new baseline.
2. Migrate the specialized id/inline stores, then generate safe per-kind AST/type payload tables. Convert the
   parser/binder/checker accessors and source-text/slice ownership; changing allocation alone is insufficient.
3. Wire concrete file versions, lazy shared data, program snapshots, checker pools, LSP/API handles and
   incremental reuse to strong owners. The core snapshot tests demonstrate the storage contract, not completion
   of those production lifetime boundaries.
4. After every caller has migrated, remove `P`, fabricated `'static` borrows, thread-arena leaks, address-based
   ownership recovery, unchecked thread traits and no-op free/recycle/checkpoint APIs. Then run the full
   conformance, fourslash, API/LSP, incremental, emit and performance gates on that complete implementation.
