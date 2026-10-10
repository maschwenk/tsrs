# Rust-owned type payloads (2026-10-10)

This continues the full ownership goal from `6c31d353`. It remains an unfinished branch checkpoint. Earlier
ownership notes, the rejected list in `perf-round2-followups.md`, `mem-round4.md` section 6 and `docs/RUST.md`
Techniques were reviewed. The changed constraint is the user's requirement to remove the custom memory model.
This is an ownership migration, not a speculative performance optimization or a completed Rust type arena.

## Ownership changes

`Type` owns a closed Rust enum with 20 boxed concrete payload variants. `TypeAlloc<T>`, payload-offset pointer
casts and fabricated static payload views are removed. Payload accessors and `TypeData` borrow their record.
Target helpers return a graph handle; callers keep that handle while borrowing the target's tuple/interface
payload. Helpers resolving declared and structured members preserve the caller's handle borrow. A compile-fail
case checks that a payload view cannot escape its record.

Symbol and alias edges occupy separate ordinary fields. The tagged alias/symbol record and its raw unpacking
are removed. Type headers still use `P<Type>` and the allocation compatibility layer; payload edges, alias
records and many slices remain legacy. The ordinary constructor permits the next type-store migration without
requiring a prefix allocation layout.

Resolved structured-member records and reference-instantiation tables use `OnceCell<Box<_>>`. Reference-table
`make()` retains the old reset behavior. Call-signature counts and index-info slices use separate fields rather
than a count-or-pointer tag. Union/intersection tails use an owned enum of lazy boxes instead of a tagged pointer
and casts; absent-tail reads and nil writes keep their previous behavior. Widening sibling lists use owned vectors.

The intermediate native type header grows from 24 to 48 bytes (28 on Wasm). Structured-member records grow from
32 to 40 bytes (24 to 32 on Wasm), and union/intersection payloads grow from 24 to 32 bytes (16 to 20 on Wasm).
Each type now owns a payload box, and every type header needs destruction. These costs must be recovered with
the full typed-store migration; this stage must not be described as memory-preserving.

## Allocation census

The profiler retains concrete layout metadata for normal Rust boxes in its live heap table. Box destruction
removes both the allocation and its layout. Owned boxes are not recorded as permanent arena blocks. Payload and
tail layouts skip scalar/discriminant words and identify remaining thin-slice edges. Feature forwarding enables
these labels in CLI profiling builds. A focused lifecycle case verifies registration and removal on drop.

The census exposed initial Oxc chunks being counted as heap blocks as well as their individual arena records.
Owner construction now marks that allocation as a chunk. Profiling scopes restore their preceding state when
nested; a focused case checks the outer scope remains active after the inner scope drops. Bounded overlap reports
identify any future duplicate records.

## Validation

Fresh Cargo target with the pinned Rust 1.99 toolchain and macOS 14.5 SDK:

- Workspace checks with test targets, CLI all-feature check and wasm32-wasip1 check pass.
- Core has 101 passing unit cases and five compile-fail cases; checker has 18 and one compile-fail case; compiler
  has 20 and two compile-fail cases; LS has 67; project has 105 with two existing ignored cases. API has 77 unit
  and integration cases. The two profiling lifecycle/scope cases pass in an alloc-profile build.
- Three CLI API cases, API/CLI build-output equality, six default emit cases, 41 native LSP cases and all 28
  regression fixtures pass. Execute's three ownership/hook cases and five native WASM cases pass. The empty
  tsctests entry returns successfully but runs no Go scenarios.
- The lint ratchet retains three existing findings and no new ones. Source checks retain 24 reviewed thread
  implementations and 74 old uncommented orderings. No custom thread implementation or unsafe code was added.
- Full six-baseline conformance has 15,197 entries: 13,462 diagnostic, 12,779 types/symbols, 13,392 JS, 149 JS-map
  and 156 source-map-text passes, with no crashes/timeouts. Every ID and classification is identical to original
  Oxc and `6c31d353`. Default-history diagnostics/types/symbols also have identical ID sets and classifications,
  including the existing failures. Fourslash keeps the same 4,066 passes, 63 failures and 417 skips as `6c31d353`.
  Comparison hashes are in `rust-owned-type-payloads-classifications.json`. Capability counts are unchanged.

These comparisons use the pinned reference fixtures already present, not a newly built tsgo. The Linux API RSS
cases require their Linux allocator/proc environment and were not counted as native macOS checks.

Compiler workload census runs with one checker, four checkers, and four with eager members each report zero
overlaps and zero strongly reachable freed blocks. Each checks 1,027,839 program references with none into freed
storage, and accounts for all 477,367 AST nodes and 38,233 binder symbols with none unreachable or unrecorded.
All exit with the workload's expected diagnostic status 2. Binary/log hashes are in
`rust-owned-type-payloads-census.json`. This is a local compiler census, not a full API/LSP lifetime audit or a
38k-file-codebase result. The verifier's precise walk covers AST/binder/recycling references, not every type edge.

## Measurements

M3 Max macOS, the same `typescript-go` workloads at `41f652ab2df5077b1115f73eaeefc2fe9f674132`,
`--noEmit --incremental false --pretty false`, with and without `--singleThreaded`. Four binaries (original Oxc,
`6c31d353`, this checkpoint and pre-Oxc) were warmed, then run in five alternating-order rounds per cell with no
other validation jobs. All 80 samples agree on exit status and stdout hash. Raw instructions, RSS, footprint,
wall times, executable hashes and source hashes are in `rust-owned-type-payloads-results.json`.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.337203 → 2.371546 | +1.47% | 115.17 → 115.98 | +0.71% |
| Compiler | default | 2.941348 → 3.014777 | +2.50% | 161.28 → 164.75 | +2.15% |
| Compiler-Unions | single | 4.864593 → 4.816768 | -0.98% | 117.08 → 122.08 | +4.27% |
| Compiler-Unions | default | 6.167844 → 6.149015 | -0.31% | 162.27 → 164.45 | +1.35% |

No local gain clears the performance-change landing bar. Compiler's single-thread instructions exceed the 1%
incremental regression limit. Both default RSS increases exceed 1% and 2 MiB (+3.47 and +2.19 MiB). The short
macOS time samples have 0.01-second resolution and do not replace the README's two-publish headline wall gate.
These no-emit workload samples do not establish API rebuild RSS bounds or the full build-mode memory profile.

Cumulative single-thread instructions remain +12.20% / +4.38% versus original Oxc. Default RSS remains
+110.84% / +100.25% versus pre-Oxc (+15.18% / +15.39% versus original Oxc). The full migration's instruction and
memory gates still fail. Do not call this checkpoint memory-preserving or land it as a performance optimization.

## Required remaining work

Type storage and graph edges still use `P`, as do AST/symbol/signature/mapper/flow/config/snapshot/cache records.
Static graph strings/slices, signature tails, static comparison callbacks, unchecked thread assumptions,
address-based allocation-owner routing and the implicit/thread arena compatibility layer remain. Owning payloads
does not make those edges safe, qualify their owners or replace the complete compiler memory model.

Complete the typed graph migration, delete the compatibility layer and recover the instruction/RSS gates. Full
pinned Go scenario/oracle audits, Linux API/runtime lifecycle checks and larger workloads remain required.
