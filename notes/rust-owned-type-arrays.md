# Owned type arrays, mappers and inference state (2026-10-10)

This continues the full ownership goal from `94fea904`. It is an unfinished branch checkpoint. The prior
ownership notes, rejected list, `mem-round4.md` section 6 and `docs/RUST.md` Techniques were reviewed before
starting. The changed constraint remains the user's requirement to remove the custom memory model. This is
an ownership migration, not a completed typed type arena or a performance change ready to land on main.

## Ownership changes

Type arguments, union/intersection members, tuple element information, interface members and bases, signature
parameters and composites, conditional-root parameters and index-info components own their array storage.
`ArrayCell<T>` retains a thin `Arc<Vec<T>>`; `ArrayView<T>` keeps that storage alive while a caller reads it or a
subrange. Replacing a field, recursively computing another cache entry or dropping the field owner cannot
invalidate the array view. `OptionArrayCell<T>` preserves nil versus a computed empty array. These helpers
forbid unsafe code. Their one-word fields retain the compact native record layout; the Arc/vector allocations
have costs that must be measured. Views retain array storage, not the referents of the legacy graph handles.

Checker, language-service and JSON API callers retain views and borrow their slices for short operations. Lazy
member tables own their ready payload and ordered-property arrays. Variance stack entries own their retained
parameter views. Array constructors and setters no longer promote these inputs into static arena slices.

Type mappers own a closed Rust enum. Array and deferred payloads own their input lists and callback vectors;
views borrow the mapper. Packed addresses, slice-length tags, header casts and static mapper-data views are
removed. Signature and inference tails use lazy owned boxes. Comparison callbacks use owned Arc closures;
inference candidate lists use lazy owned vectors. Manual escape flags, inference/mapper recycling and their
scratch retention lists are removed. Records and payloads are destroyed by their Rust owners.

The native signature record is 80 bytes and the inference context 72 bytes; on Wasm they are 48 and 36. Native
resolved-member and union/intersection payloads stay at 40 and 32 bytes. Their Wasm layouts shrink to 20 and 16.
Lazy member-table headers have three owner/edge words in addition to their symbol table; ready payloads are
boxed. The allocation census follows ordinary owned array pointers and checks which mapper enum variants
contain a second graph edge. It no longer decodes the removed mapper/inference tags.

## Remaining boundaries

Type, mapper, signature, inference, AST and symbol edges still use legacy `P` handles and the region allocation
compatibility layer. Type-header storage has not migrated to the typed store. Static strings, template-text
arrays and other link/cache arrays remain. Array views can retain legacy handles beyond their graph owner;
these handles are not safe standalone graph leases. TLS arena retention, raw graph access, snapshot roots and
unchecked thread implementations remain part of the active full migration.

## Validation

The pinned Rust 1.99 toolchain, fresh Cargo target and macOS 14.5 SDK were used:

- Workspace checks with test targets, CLI and runner all-feature checks, and wasm32-wasip1 checks pass.
- Core has 103 passing unit cases and five compile-fail cases; checker has 20 and one compile-fail case;
  compiler has 20 and two compile-fail cases; LS has 67; project has 105 with two existing ignored cases.
  API has 77 unit/integration cases. Execute's three ownership/hook cases and five native WASM cases pass.
  The empty tsctests entry runs no Go scenarios. The two profiling lifecycle/scope cases pass.
- New cases verify retained arrays through field replacement and owner destruction, nil versus computed empty
  arrays, and destruction of captures in deferred mapper and inference comparison callbacks. Final core/checker
  checks repeat all 123 unit cases and six compile-fail cases after the ownership edits.
- Three CLI API cases, API/CLI output equality, six default emit cases, 41 native LSP cases and all 28 regression
  fixtures pass. Native macOS checks do not cover the Linux allocator/proc RSS cases.
- The lint ratchet has three existing findings and no new ones. Source checks report 615 files, 24 reviewed thread
  implementations and 74 old uncommented orderings. No unsafe block or custom thread implementation was added.
- All 15,197 full conformance IDs and all six baseline classifications match `94fea904` and original Oxc:
  13,462 diagnostic, 12,779 types/symbols, 13,392 JS, 149 JS-map and 156 source-map-text passes. There are no
  crashes/timeouts. Default-history diagnostics/types/symbols also match every prior classification, including
  existing failures. Fourslash's exact sets remain 4,066 passes, 63 failures and 417 skips. Hashes are in
  `rust-owned-type-arrays-classifications.json`. User-visible capability counts are unchanged.

The README's stale leaf-retirement paragraph is corrected: the Oxc migration already disabled that path, and
`TSRS_FREE_LEAVES` has no effect. Its capability table and `docs/STATUS.md` now identify complete compiler-graph
ownership as unfinished. No capability-table pass count changes in this checkpoint.

The comparisons use the pinned reference fixtures already present, not a newly built tsgo. Full Go scenarios,
oracle audits and Linux API RSS checks remain unavailable in this macOS fixture checkout.

Compiler census runs with one checker, four checkers, and four with eager members each report zero overlaps and
zero strongly reachable freed blocks. All account for 1,027,839 program references, 477,367 AST nodes and 38,233
binder symbols, with none into freed storage or unrecorded/unreachable. All return the expected diagnostic status
2. Binary/log hashes are in `rust-owned-type-arrays-census.json`. The precise verifier covers AST/binder/recycling
references, not every type edge. These runs do not establish full API/LSP or 38k-file-codebase lifetime coverage.

## Measurements

M3 Max macOS, the same `typescript-go` workloads at `41f652ab2df5077b1115f73eaeefc2fe9f674132`,
`--noEmit --incremental false --pretty false`, with and without `--singleThreaded`. Original Oxc, `94fea904`,
this checkpoint and pre-Oxc binaries were warmed, then measured in five alternating-order rounds per cell without
other validation jobs. All 80 samples agree on exit status and stdout hash. Instructions, peak RSS, footprint,
wall times, executable hashes and source hashes are in `rust-owned-type-arrays-results.json`.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.369569 → 2.379333 | +0.41% | 115.98 → 119.08 | +2.67% |
| Compiler | default | 3.027098 → 3.054835 | +0.92% | 162.80 → 163.62 | +0.51% |
| Compiler-Unions | single | 4.820385 → 5.011084 | +3.96% | 122.08 → 122.53 | +0.37% |
| Compiler-Unions | default | 6.162805 → 6.416158 | +4.11% | 165.56 → 164.08 | -0.90% |

No gain clears the performance-change landing bar. Compiler-Unions' single-thread instruction increase exceeds
the 1% incremental regression limit. Compiler's single-thread RSS increase exceeds 1% and 2 MiB (+3.09 MiB).
Short macOS wall samples have 0.01-second resolution and do not replace the README's two-publish wall gate.
These no-emit samples do not establish API rebuild RSS bounds or the complete build-mode memory profile.

Cumulative single-thread instructions are +12.59% / +8.53% versus original Oxc. Default RSS is +109.52% /
+99.75% versus pre-Oxc (+15.39% / +15.17% versus original Oxc). The full migration's instruction and memory
gates still fail. This checkpoint must not land as a performance optimization or be called memory-preserving.

Complete typed graph storage and qualified graph edges, remove static graph strings/slices and the compatibility
allocator/runtime, and recover the instruction/RSS gates. Owning arrays and callbacks does not retain their raw
referents or complete the memory model migration.
