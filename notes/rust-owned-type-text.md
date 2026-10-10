# Owned compiler text and literal snapshots (2026-10-11)

This continues the complete ownership goal from `8771b720`. Prior ownership results, the measured-and-rejected
list, `docs/RUST.md` Techniques and the latest `mem-round4.md` summary were reviewed. The changed constraint is
still the user's requirement to replace the custom memory model. This is an unfinished ownership checkpoint;
raw graph handles and the compatibility runtime remain. It is not a completed migration or a performance landing.

## Ownership changes

Literal values, bigint digits, type/template text, computed names, intrinsic names, type-predicate parameter
names, JSX namespace caches and widening/discriminant names use ordinary Rust-owned text. `TextView` retains an
immutable `Arc<String>`; moving a string retains its original buffer, and an empty view needs no allocation.
`TextCell` returns retained snapshots across field replacement and owner destruction. Both stay one word wide.
`SnapshotCell` stores non-Copy literal/evaluator results with short `RefCell` borrows and cloned snapshots.
The string-literal type cache borrows a value for lookup instead of cloning an owner for every hash operation.

AST literal/private-identifier fields and synthesized identifier text own their text. Their factories accept
short string borrows and copy into the record; getters borrow the record. Symbol names and exceptional symbol-table
keys own text too, and returned key snapshots retain it after the table and region drop. Symbol state bits are
ordinary fields, independent of the name's address. The old packed `OwnedTaggedStrCell` implementation and its
census decoder are removed. A native Symbol grows from 40 to 48 bytes; the Wasm size remains 32 bytes.

Binder, node-builder, emit/declaration, module-specifier and language-service names use owned fields or ordinary
local string collections. Ambient module names and source-file identifier/name tables own their storage in the
existing dropped source-file sidecar. The AST generator produces the new fields from the pinned schema and a
second regeneration is byte-identical. Redundant arena text copies feeding owned factories are removed.

The text modules forbid unsafe code. Lifetime cases retain text through replacement and region destruction and
confirm final-owner release. No new custom thread implementation, unsafe block or static lifetime bridge was added.
The source text of compact identifiers, source-file text, joined JSDoc/JSX text, graph arrays and raw `P` referents
still use legacy ownership. Retaining one of the new text values does not retain the graph it came from.

## Validation

Rust 1.99, a fresh Cargo target and the macOS 14.5 SDK were used:

- Workspace/test-target, CLI/runner all-feature and wasm32-wasip1 checks pass. Core has 104 unit cases and five
  compile-fail cases; AST 22; checker 21 and one compile-fail case; compiler 20 and two compile-fail cases; LS 67;
  project 105 with two old ignored cases; API 77 unit/integration cases; native Wasm five; printer 15; transformers
  three. The broader execute, declaration and documentation suites also pass.
- All 41 native LSP cases, three CLI API cases, API/CLI output equality, six default-emit cases, 28 regression
  fixtures and two profiling lifecycle cases pass.
- The final lint ratchet has three existing findings and none new. Source inventory is 617 files, 24 reviewed
  thread implementations and 74 old uncommented orderings. Neither lint baseline nor unsafe inventory changed.
- Every one of the 15,197 conformance IDs and all six full baseline classifications matches `8771b720` and original
  Oxc: diagnostic 13,462, types/symbols 12,779, JS 13,392, JS-map 149 and source-map-text 156 passes. No crashes or
  timeouts. Default-history diagnostics/types/symbols retain every classification, including known failures.
  Fourslash retains the exact 4,066 pass, 63 fail and 417 skip sets. Hashes are in
  `rust-owned-type-text-classifications.json`.

The complete pinned upstream archive and Go toolchain are now available. tsgo was built from
`b85298b6a81f772d080b0455de0ca9d744cd6fd6`, not an npm nightly. Archive, executable, schema and module hashes are
in `rust-owned-type-text-reference.json`. The recorder produced 522 scenario JSON files; 406 non-watch scenarios
without content mappers were replayed. All fixture baselines were checked against the archive byte for byte.
Current, `8771b720` and original Oxc `3dac06d8` have exactly the same classifications: tsc 184/216 and tsbuild
187/190 passes, 35 failures, zero crashes. Only the already-failing internal-symbol-name output differs between
binaries, as its unsanitized symbol ID varies. Hashes are in `rust-owned-type-text-tsctests.json`.

The fresh audit corrects the README's old 187/216 tsc figure. Three casing scenarios already fail in original Oxc:
tsrs reports TS1149 where pinned tsgo reports TS1261. The other failures are the previously documented CLI
outputs and internal-symbol-name case. These are pre-existing gaps, not new failures from this ownership stage.
The pinned CLI emit oracle also matches all output bytes, diagnostics and statuses for a public fixture exercising
Unicode text, bigint, template types and private names: five JS/declaration/map/build-info files, or three in
declaration-only mode. This does not rerun the unavailable 38k-file codebase.

## Measurements

Compiler census runs at one checker, four checkers and four with eager members each have zero overlaps and zero
strongly reachable freed blocks. Each accounts for 1,027,839 program references, 477,367 AST nodes and 38,233
binder symbols; none points to freed/rewound storage or is unrecorded/unreachable. All return diagnostic status 2.
Executable/log hashes are in `rust-owned-type-text-census.json`. This verifier covers AST/binder/recycling
references, not every type edge; it does not establish full API/LSP or 38k-file-codebase lifetime coverage.
The allocator profile counts actual live heap; the manual per-container report does not fully attribute the
inner string allocations of shared text views.

M3 Max macOS, the same `typescript-go` workloads at `41f652ab2df5077b1115f73eaeefc2fe9f674132`,
`--noEmit --incremental false --pretty false`, with and without `--singleThreaded`. Original Oxc, `8771b720`,
this checkpoint and pre-Oxc binaries were warmed, then measured in five alternating-order rounds per cell with
no other validation jobs running. All 80 samples agree on status and stdout hash. Instructions, peak RSS,
footprint, wall times, executable hashes and source hashes are in `rust-owned-type-text-results.json`.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.386435 → 2.470189 | +3.51% | 118.23 → 123.72 | +4.64% |
| Compiler | default | 3.076291 → 3.237038 | +5.23% | 161.73 → 168.80 | +4.37% |
| Compiler-Unions | single | 5.018390 → 5.321559 | +6.04% | 121.27 → 129.17 | +6.52% |
| Compiler-Unions | default | 6.431835 → 6.869388 | +6.80% | 161.83 → 179.28 | +10.78% |

The owned text and changed Symbol layout add measurable costs. Neither an instruction nor a memory gain clears
any performance landing bar; Compiler-Unions default RSS also regresses more than 5% incrementally. Short macOS
wall samples have 0.01-second resolution and do not substitute for the README's two-publish wall gate.
These no-emit samples do not establish API rebuild RSS bounds or a complete build-mode memory profile.

Cumulative single-thread instructions are +16.53% / +15.18% versus original Oxc. Default peak RSS is
+117.89% / +118.47% versus pre-Oxc (+20.23% / +25.08% versus original Oxc). The full migration's instruction
and memory gates still fail. This is a work-in-progress checkpoint on the explicitly requested migration branch;
it must not land as a performance optimization or be described as memory-preserving.

## Remaining work

Complete typed graph stores and owner-qualified edges are still required. `P` references, compact-identifier
source-text registration, AST graph slices, manual region/address bookkeeping, recycling compatibility and
unchecked graph thread boundaries remain. The precise compiler census does not cover all type edges or the full
API/LSP lifecycle. Linux allocator/process RSS coverage and the larger workload remain required. Complete those
boundaries and recover the instruction/RSS gates before calling the migration finished.
