# Checker-owned signatures and composites (2026-10-11)

This continues the complete ownership goal from `b67128bc`. Prior ownership/signature notes, the rejected list,
`docs/RUST.md` Techniques and the latest `mem-round4.md` summary were reviewed before changing storage. The
changed constraint is the explicit request to replace the complete memory model. This is an unfinished
migration-branch checkpoint, not a performance landing or proof of complete compiler-graph ownership.

## Ownership changes

Each checker owns `ArenaBuilder<Signature>` and `ArenaBuilder<CompositeSignature>`. Signature links, structured
and interface member arrays, call state, inference state, signature targets, composite members, cache keys/values,
API registries and LS/node-builder arguments carry eight-byte qualified keys. There is no `P<Signature>` or
`P<CompositeSignature>` and neither record is allocated in a region. Records, lazy boxes and array buffers run
ordinary Rust destructors. `Checker::signature` / `composite_signature` borrow the checker and reject foreign
storage keys. Callers retain keys or owned array snapshots across recursion, then resolve the record again.

Semantic signature IDs still follow the original counter, independent of storage slots and owner identities.
Composite metadata is shared by key when signatures are cloned; union/intersection component order, mapper
selection, deferred return/predicate resolution and recursion sentinels retain their original control flow.
Only internal packed-cache bucket hashes change from pointer bits to qualified-key hashes; this cache does not
iterate for observable output. The optional inference work-census groups by semantic signature ID instead of
pointer bits. No storage ID enters the wire ID or Go's type-instantiation hash.

API signature properties now acquire the persistent checker lease and validate the registry's checker identity
before resolving records. Response fields and wire IDs keep their order/meaning. The registry takes the semantic
ID explicitly; it stores a key and cannot dereference one. Unit cases cover stale/cross-project/released handles,
store growth, shared composites, target edges, semantic IDs, foreign stores and snapshots across replacement and
store destruction. The snapshot case retains only the array's key buffer, not the referenced records. A public
compile-fail case verifies that a signature borrow cannot become static.

The native signature, signature-link and inference-context headers remain 80, 32 and 72 bytes. Wasm headers grow
48 → 52, 16 → 28 and 36 → 40 respectively because qualified keys replace four-byte pointers. Both target builds
check the layout assertions. Manual heap reports include both record-vector capacities; RSS includes actual
vector/array/tail allocations. This stage adds no unsafe code or unchecked thread-trait implementation. Type,
mapper, inference-info, symbol and AST references inside records remain raw graph edges.

## Validation

Normal workspace all-targets, CLI/runner all-feature test targets and wasm32-wasip1 checks pass with no warnings.
Core's 104 unit cases and five compile-fail cases, AST's 22, checker's 26 and two compile-fail cases, compiler's
20 and two compile-fail cases, LS's 67, project's 105 with two old ignored cases, API's 77 unit/integration cases,
execute's four and native Wasm's five pass. Native LSP's 41, four CLI units, thirteen supported CLI integration
cases and 28 regression fixtures pass. The full CLI invocation also records the two existing leaf-retirement
counter failures; they are not counted as passes or changed to hide the backend gap. Linux-only RSS assertions
remain unvalidated on macOS. The ratchet retains three old findings and none new; the source inventory remains
617 files, 24 reviewed thread implementations and 74 old uncommented orderings. No baseline/inventory is raised.
Validation log hashes are recorded in `rust-typed-signature-validation.json`.

Every one of the 15,197 full conformance IDs and all six classifications match `b67128bc` and original Oxc:
diagnostics 13,462, types/symbols 12,779, JS 13,392, JS-map 149 and source-map-text 156 passes. Default-history
diagnostic/type/symbol classifications also match exactly, including their existing failures. Fourslash retains
the exact 4,066 pass, 63 fail and 417 skip sets. Completed conformance runs use a 120-second limit and report
no crashes/timeouts (`rust-typed-signature-classifications.json`). The eager-member opt-out also retains every
diagnostic/type/symbol classification versus a fresh `b67128bc` replay (13,458 / 12,779 / 12,779 passes,
with the same two diagnostic code-only cases and two diagnostic failures).

Fresh pinned-Go scenario replay retains 184/216 tsc and 187/190 tsbuild passes, 35 known failures and zero
crashes, with the same classification sets as `b67128bc` and original Oxc. Only the already-failing unsanitized
internal-symbol-name output differs locally (`rust-typed-signature-tsctests.json`). Watch/content-mapper scenarios
remain excluded; reference provenance is in `rust-owned-type-text-reference.json`.

A public fixture exercises generic/overloaded/rest/composite signatures, predicates, alias forms, Unicode,
bigint and private names. Against freshly built tsgo at the exact pin, diagnostics/status agree and all five
JS/declaration/map/build-info files, or three in declaration-only mode, are byte-identical
(`rust-typed-signature-emit.json`). This is not the full emit oracle or the 38k-file codebase audit.

## Measurements

Compiler profiling at one checker, four checkers and four with eager members reports zero overlaps and zero
strongly reachable freed blocks. Each accounts for 1,027,839 program references, 477,367 AST nodes and 38,233
binder symbols, with no freed/rewound or unrecorded/unreachable references and the expected diagnostic status 2
(`rust-typed-signature-census.json`). This verifier covers AST/binder/recycling references, not every type edge or
all API/LSP lifetimes. Extended-diagnostic runs exercise signature inference work-census and assignment profiling.
The public fixture's manual heap report records 13,282 signatures (capacity 16,384 × 80 = 1,310,720 bytes) and
three composites (capacity four × 16 = 64 bytes); these are vector bytes, excluding array buffers/rare tails.
The two profiling ownership/scope lifecycle cases also pass.

M3 Max macOS, Rust 1.99, public `typescript-go` workloads at
`41f652ab2df5077b1115f73eaeefc2fe9f674132`, `--noEmit --incremental false --pretty false`, with and without
`--singleThreaded`. Original Oxc, `b67128bc`, this checkpoint and pre-Oxc binaries were warmed, then measured
in five alternating-order rounds per cell without other build/validation jobs. All 80 samples agree on status
and stdout hash. Instructions retired, peak RSS, footprint, wall time and executable/changed-Rust-source hashes
are in `rust-typed-signature-results.json`. The source hashes identify the measured implementation; the patch
hash records the documentation state at measurement time, before this note was finished.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.463823 → 2.457442 | -0.26% | 123.56 → 122.73 | -0.67% |
| Compiler | default | 3.192700 → 3.168446 | -0.76% | 170.62 → 169.91 | -0.42% |
| Compiler-Unions | single | 5.302890 → 5.294970 | -0.15% | 129.12 → 125.06 | -3.15% |
| Compiler-Unions | default | 6.843000 → 6.814625 | -0.41% | 175.75 → 170.39 | -3.05% |

The incremental changes clear neither the 1% single-thread instruction bar nor the 5% default-memory bar.
Short/noisy macOS wall samples do not replace the README's two-publish wall gate or the Linux deterministic
instruction workflow. This does not establish complete build-mode/API rebuild memory bounds.

Cumulative single-thread instructions remain +16.28% / +14.74% versus original Oxc. Default peak RSS is
+117.92% / +108.11% versus pre-Oxc (+20.01% / +18.31% versus original Oxc). The complete migration's
instruction/memory gates still fail. This explicitly requested WIP checkpoint must not land as a performance
optimization or be described as memory-preserving.

## Remaining work

Type, mapper, inference-info/context, symbol, AST and flow records/edges still require typed graph stores and
explicit owners at recursive and external boundaries. Source text/static graph storage and the allocation
compatibility runtime remain, along with unchecked thread boundaries and the disabled leaf-retirement gap.
The complete API/LSP lifetime, Linux allocator/process RSS and larger-workload audits remain required. Recover
the complete migration's instruction/RSS gates and replace the remaining runtime before calling the goal
complete or landing it as a performance change.
