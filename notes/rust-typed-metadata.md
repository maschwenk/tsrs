# Typed predicate and index metadata stores (2026-10-11)

This continues the complete ownership goal from `fa2d06cd`. The prior ownership notes, rejected list,
`docs/RUST.md` Techniques and latest `mem-round4.md` summary were reviewed. The changed constraint remains
the explicitly requested complete memory-model replacement. This is an unfinished graph migration, not a
performance landing or evidence that the complete graph has Rust ownership.

## Ownership changes

Each checker owns typed `ArenaBuilder<TypePredicate>` and `ArenaBuilder<IndexInfo>` vectors. Records no longer
use `P::new`, raw-pointer identity or region drop sidecars. Edges are eight-byte owner-qualified keys on both
native and Wasm targets. Access borrows the explicit checker, rejects a foreign owner and returns a reference
tied to that borrow. Vector growth cannot invalidate a retained reference; recursive callers retain keys and
resolve them again. Records and buffers run ordinary Rust destructors with the checker.

Type-predicate metadata is immutable after construction. Signatures retain predicate keys, preserving the
uncomputed/no-predicate distinction and sentinel identity. A predicate remains 24 bytes natively and 16 on
Wasm. Index records remain 48/24 bytes; their existing cached-symbol cell and owned component arrays keep
their behavior. Structured, interface and lazy index-info arrays contain keys. Lookup/name helpers receive the
checker explicitly. API, node-builder, inference, relation, flow and language-service readers resolve through
the owner; API encoding releases the record borrow before recursively encoding a type.

Meaningful cases cover signature sentinel identity, vector growth, cache writes after growth, foreign owners
and retained predicate text after storage destruction. The manual heap report includes both record vectors.
The safe storage module forbids unsafe code; this stage adds no custom unsafe or thread implementation.
Types, symbols, AST components and declarations inside these records still use legacy graph edges. Owning
metadata records does not establish ownership of those referents.

## Validation

Workspace/test-target, CLI/runner all-feature and wasm32-wasip1 checks pass. Core's 104 unit cases and five
compile-fail cases, checker's 23 and one compile-fail case, compiler's 20 and two compile-fail cases, LS's 67,
project's 105 with two old ignored cases, API's 77 unit/integration cases, execute's ownership/hook cases and
native Wasm's five cases pass. The lint ratchet retains three existing findings and none new; source inventory
is unchanged at 617 files, 24 reviewed thread implementations and 74 old uncommented orderings.

The pinned Go scenario replay retains every classification from `fa2d06cd` and original Oxc `3dac06d8`:
tsc 184/216 and tsbuild 187/190 passes, 35 failures, zero crashes. Only the already-failing unsanitized
internal-symbol-name output differs. Hashes are in `rust-typed-metadata-tsctests.json`; reference provenance
is in `rust-owned-type-text-reference.json`. Watch and content-mapper scenarios are excluded.

All 15,197 full conformance IDs and all six baseline classifications match `fa2d06cd` and original Oxc:
diagnostics 13,462, types/symbols 12,779, JS 13,392, JS-map 149 and source-map-text 156 passes. There are
no crashes/timeouts. Default-history diagnostics/types/symbols retain every classification, including the old
failures. Fourslash retains the exact 4,066 pass, 63 fail and 417 skip sets. Hashes are in
`rust-typed-metadata-classifications.json`.

The 41 native LSP cases, three CLI API cases, API/CLI output equality, six default-emit cases, 28 regression
fixtures and two profiling lifecycle cases pass. The freshly built pinned tsgo emit oracle is byte-identical
on a public fixture exercising inferred/assertion predicates, a returned readonly index type, Unicode text,
bigint and private names: five JS/declaration/map/build-info files, or three in declaration-only mode. Status
and diagnostics also agree (`rust-typed-metadata-emit.json`). This does not rerun the 38k-file codebase.

## Measurements

Compiler census runs at one checker, four checkers and four with eager members each report zero overlaps and
zero strongly reachable freed blocks. Each accounts for 1,027,839 program references, 477,367 AST nodes and
38,233 binder symbols, with none to freed/rewound storage or unrecorded/unreachable. All return the expected
status 2; hashes are in `rust-typed-metadata-census.json`. The precise verifier covers AST/binder/recycling
references, not every type edge or all API/LSP lifetimes. The manual heap report now includes the two typed
record vectors; allocator/RSS measurements include their actual heap storage.

M3 Max macOS, the same `typescript-go` workloads at `41f652ab2df5077b1115f73eaeefc2fe9f674132`,
`--noEmit --incremental false --pretty false`, with and without `--singleThreaded`. Original Oxc, `fa2d06cd`,
this checkpoint and pre-Oxc binaries were warmed, then measured in five alternating-order rounds per cell
without other validation jobs. All 80 samples agree on exit status and stdout hash. Raw instructions, peak RSS,
footprint, wall times and executable/source hashes are in `rust-typed-metadata-results.json`.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.478727 → 2.476912 | -0.07% | 123.72 → 123.56 | -0.13% |
| Compiler | default | 3.204139 → 3.202054 | -0.07% | 167.91 → 169.06 | +0.69% |
| Compiler-Unions | single | 5.315900 → 5.322182 | +0.12% | 129.17 → 129.03 | -0.11% |
| Compiler-Unions | default | 6.854504 → 6.867660 | +0.19% | 172.39 → 172.09 | -0.17% |

These small incremental changes clear no performance landing bar. macOS wall samples are short/noisy and do
not replace the README's two-publish wall gate. The measurements do not establish API rebuild RSS bounds or
the complete build-mode memory profile.

Cumulative single-thread instructions remain +16.53% / +14.96% versus original Oxc. Default peak RSS is
+119.52% / +111.81% versus pre-Oxc (+23.67% / +21.30% versus original Oxc). The complete migration's
instruction/memory gates still fail. This is an explicitly requested unfinished migration-branch checkpoint;
it must not land as a performance optimization or be called memory-preserving.

## Remaining work

Type, signature, mapper, symbol and AST records/edges still require complete typed graph storage, with explicit
owners at callbacks and external boundaries. Source-file and compact-identifier text, graph slices and the
allocation compatibility runtime remain. The metadata stores add no static bridge to work around those
boundaries. Full API/LSP lifecycle, Linux allocator/process RSS and larger-workload validation remain required;
the complete migration's instruction/RSS gates must be recovered before completion or landing.
