# Checker-owned type aliases (2026-10-11)

This continues the complete ownership goal from `9028184a`. The prior ownership/alias notes, rejected list,
`docs/RUST.md` Techniques and latest `mem-round4.md` summary were reviewed. The changed constraint is the
explicitly requested complete memory-model replacement. This is unfinished migration-branch work, not a
performance landing or evidence that the complete graph has Rust ownership.

## Ownership changes

Each checker owns an `ArenaBuilder<TypeAlias>`. Type headers, conditional roots, pending aliases and constructor
arguments retain eight-byte owner-qualified `TypeAliasKey` values. No `P<TypeAlias>` or region-allocated alias
record remains. Record access borrows the checker or its explicit alias store and rejects foreign keys. Records
and argument buffers run ordinary Rust destructors. Recursive callers retain keys or argument snapshots, never
a reference into the growing vector.

The alias symbol is an ordinary immutable field. Deferred references still replace the argument list after
instantiation; its `ArrayCell` preserves snapshots across that replacement. A pending alias contributes the same
symbol/type semantic IDs to cache hashes without allocating a record. It materializes once, retains its key
across vector growth, and rejects reuse with another store. Storage IDs never enter Go's cache hashes or type
numbering. Unit cases cover these properties, cross-owner hash equality and snapshots after store destruction.

Inference, union caches, type sorting, node builders, declaration serialization, profiling labels and API readers
receive the owner explicitly. The API alias-symbol property acquires the persistent checker and validates the
registered type's checker identity before reading metadata. Cache readers retain the original ordering and
pending-materialization behavior. The manual heap report includes the alias record vector.

An alias record remains 16 bytes natively and 8 on Wasm. The native type header stays 48 bytes; its Wasm header
grows from 28 to 32 because the qualified alias key replaces a four-byte pointer. The symbol/type entries inside
an alias remain raw graph edges. This stage adds no unsafe or unchecked thread-trait implementation.

## Validation

Normal workspace all-targets, CLI/runner all-feature test targets and wasm32-wasip1 checks pass with no warnings.
Core's 104 unit cases and five compile-fail cases, AST's 22, checker's 25 and one compile-fail case, compiler's
20 and two compile-fail cases, LS's 67, project's 105 with two old ignored cases, API's 77 unit/integration cases,
execute's four ownership/hook cases and native Wasm's five pass. The lint ratchet retains three existing findings
and none new; source inventory stays at 617 files, 24 reviewed thread implementations and 74 old uncommented
orderings. Baselines/inventory are not raised. Validation log hashes are in `rust-typed-alias-validation.json`.

All 15,197 full conformance IDs and all six classifications match `9028184a` and original Oxc: diagnostics
13,462, types/symbols 12,779, JS 13,392, JS-map 149 and source-map-text 156 passes. Default-history diagnostics,
types and symbols also retain every classification, including the old failures. Fourslash retains the exact
4,066 pass, 63 fail and 417 skip sets (`rust-typed-alias-classifications.json`). There are no crashes/timeouts
in the completed runs. Full conformance used a 120-second limit: the initial 20-second attempt timed out two
cases whose previous records took 39.3 and 54.1 seconds.

Native LSP's 41 cases, four CLI unit cases, thirteen supported CLI integration cases, 28 regression fixtures and
two profiling lifecycle cases pass. The two separately recorded leaf-retirement assertions below still fail.
The freshly built pinned tsgo emit oracle is byte-identical on a public alias fixture exercising generic,
conditional, union, indexed, deferred and template aliases, Unicode/bigint and private names: five files with
JS/declarations/maps/build-info, or three in declaration-only mode. Status and diagnostics agree
(`rust-typed-alias-emit.json`). This does not rerun the full emit oracle or 38k-file codebase.

The pinned Go replay retains all classifications from `9028184a` and original Oxc `3dac06d8`: tsc 184/216 and
tsbuild 187/190 passes, 35 known failures, zero crashes. The only local output difference is the already-failing
unsanitized internal-symbol-name scenario. Hashes are in `rust-typed-alias-tsctests.json`; pinned reference
provenance is in `rust-owned-type-text-reference.json`. Watch/content-mapper scenarios remain excluded.

Broader checks found stale AST/binder oracle example callers from the owned-text stage; they now use retained
ambient-name snapshots and short symbol-name borrows. Normal workspace all-target checks compile the examples.
Enabling every feature on every workspace target still conflicts between the parser oracle's mimalloc global
allocator and core's profiling global allocator; that combination is not claimed to pass. CLI/runner all-feature
checks are recorded separately.

The complete CLI unit invocation exposed the old Linux-only RSS sampler calling `malloc_trim` on macOS. Native
crash reports, including one before this stage, show an address-zero call from `memory_tests::rss_kib`. The FFI,
sampler and two RSS cases now compile only for Linux/glibc, matching the API RSS harness. Their Linux assertions
remain unvalidated here; the output-equivalence case still runs on macOS. The complete CLI integration invocation
also fails two existing leaf-retirement counter assertions. Retirement is disabled by the Oxc backend and is
still required by those cases; they are not counted as passes or changed to conceal the gap. The same missing
retirement counters and byte-identical diagnostics were verified against original Oxc and `9028184a`
(`rust-typed-alias-runtime-gaps.json`).

## Measurements

Compiler census runs at one checker, four checkers and four with eager members each report zero overlaps and
zero strongly reachable freed blocks. Each accounts for 1,027,839 program references, 477,367 AST nodes and
38,233 binder symbols, with none to freed/rewound storage or unrecorded/unreachable. All return the expected
status 2 (`rust-typed-alias-census.json`). The precise verifier covers AST/binder/recycling references, not every
type edge or all API/LSP lifetimes. Extended-diagnostic profiling exercises the alias readers in assignment stats
and work-census labels. The single-checker manual heap report includes 377 alias records, capacity 512, 16 bytes
per slot (8 KiB of vector storage). Allocator/RSS measurements include argument buffers and actual heap storage.

M3 Max macOS, the same `typescript-go` workloads at `41f652ab2df5077b1115f73eaeefc2fe9f674132`,
`--noEmit --incremental false --pretty false`, with and without `--singleThreaded`. Original Oxc, `9028184a`,
this checkpoint and pre-Oxc binaries were warmed, then measured in five alternating-order rounds per cell
without other validation jobs. All 80 samples agree on exit status and stdout hash. Instructions, peak RSS,
footprint, wall times and executable/source hashes are in `rust-typed-alias-results.json`.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.464156 → 2.463508 | -0.03% | 123.56 → 123.56 | 0.00% |
| Compiler | default | 3.197663 → 3.196328 | -0.04% | 171.94 → 169.50 | -1.42% |
| Compiler-Unions | single | 5.305199 → 5.302625 | -0.05% | 129.05 → 129.11 | +0.05% |
| Compiler-Unions | default | 6.841757 → 6.840390 | -0.02% | 174.06 → 170.78 | -1.89% |

These incremental changes clear no performance landing bar. Short/noisy macOS wall samples do not replace
the README's two-publish wall gate. These measurements do not establish API rebuild RSS bounds or the
complete build-mode memory profile.

Cumulative single-thread instructions remain +16.51% / +14.90% versus original Oxc. Default peak RSS is
+117.40% / +108.15% versus pre-Oxc (+18.93% / +19.95% versus original Oxc). The complete migration's
instruction/memory gates still fail. This is an explicitly requested unfinished migration-branch checkpoint;
it must not land as a performance optimization or be called memory-preserving.

## Remaining work

Type, signature, mapper, symbol and AST records/edges still require complete typed graph storage and explicit
owners at callbacks/external boundaries. Source text/static graph storage and the allocation compatibility
runtime remain. The full API/LSP lifetime, Linux allocator/process RSS and larger-workload audits remain
required. Recover the complete migration's instruction/RSS gates and replace the remaining runtime before
calling the goal complete or landing it as a performance change.
