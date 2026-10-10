# Checker-owned mapper records (2026-10-11)

This continues the complete memory-model migration from `e18a7bc1`. Prior mapper/ownership notes, the measured
and rejected list, `docs/RUST.md` Techniques and the latest memory-round summary were read before changing
storage. The changed constraint is the explicit complete ownership request. This is an unfinished branch
checkpoint, not proof of complete compiler-graph ownership or a performance landing.

## Ownership changes

Each checker owns a typed Rust vector of `TypeMapper` records. All mapper edges use eight-byte qualified
`TypeMapperKey` values, including type/signature/inference fields, merged/composite pairs, lazy member tables,
value-symbol links, active mapper stacks and node-builder scopes. There is no `P<TypeMapper>` or region mapper
allocation. Factories are checker methods; bootstrap construction allocates its placeholder into the store
before moving that store into the checker. Semantic type/signature IDs and instantiation keys remain unchanged.

`Checker::type_mapper` borrows its owner and rejects foreign keys. `apply_type_mapper` selects scalar values or
keys before recursive mutation; it does not retain a record reference while growing the vector. Deferred
callbacks use `Arc`, and the selected callback owner is retained before entering it with a mutable checker.
Array mapper comparison snapshots source/target lists before recursively comparing their types. Array mapping
itself borrows its immutable lists without copying. Merged/composite order and inference fixing behavior remain
unchanged. Records, callback vectors, payload boxes and array buffers run ordinary Rust destructors.

The deferred ownership case grows storage, checks source-buffer ownership and foreign-store rejection, then
drops the store while holding a selected callback. The captured object is released when that callback owner
also drops. A public compile-fail case prevents a mapper borrow from becoming static. Keys/array buffers and
callback owners do not retain their legacy type/symbol/AST referents.

Native mapper/context/signature/value-link/lazy-member headers remain 24/72/80/40/(symbol table +24) bytes.
Wasm changes are mapper 16 → 20, context 40 → 48, signature 52 → 56, value links 20 → 24 and lazy member table
(symbol table +12) → (symbol table +16), because qualified keys replace four-byte pointers. Both builds check
the layout assertions. Profiling treats merged/composite keys and mapper fields in type payloads as scalars.
Manual heap reporting includes the mapper record-vector capacity. No unsafe code or unchecked thread trait is
added; the existing unchecked checker boundary still requires its separate complete audit.

## Validation

Normal workspace all-targets, CLI/runner all-feature targets and wasm32-wasip1 checks pass without warnings.
Core 104 unit/five compile-fail, AST 22, checker 26/four compile-fail, compiler 20/two compile-fail, LS 67,
project 105 (two old ignored), API 77, execute four and native Wasm five cases pass. Checker's unit/compile-fail
cases also repeat after the profiling/layout edits. Four CLI units, thirteen supported CLI integrations, native
LSP 41 and all 28 regression fixtures pass. The two leaf-retirement counter cases still fail because the Oxc
backend disabled that path; these failures are recorded separately, not hidden or counted as passes. Linux
allocator/process RSS behavior remains unvalidated on macOS. The ratchet has three old findings and none new;
source inventory remains 617 files, 24 reviewed thread implementations and 74 old uncommented orderings, with
no baseline/inventory increase.

Every full conformance ID and all six classifications match `e18a7bc1` and original Oxc: diagnostics 13,462,
types/symbols 12,779, JS 13,392, JS-map 149 and source-map-text 156 passes. Default-history classifications match
exactly, including their existing failures. The paired `TSRS_LAZY_MEMBERS=0`/`TSRS_LAZY_INFERENCE_MAPPERS=0`
comparison also matches exactly. Fourslash retains 4,066 pass, 63 fail and 417 skip sets. No conformance
crashes/timeouts occur (`rust-typed-mapper-classifications.json`). Fresh pinned-Go scenario replay retains
184/216 tsc and 187/190 tsbuild passes, 35 existing failures and zero crashes; only the already-failing unsanitized
internal-symbol-name output differs (`rust-typed-mapper-tsctests.json`). Watch/content mappers remain excluded;
reference provenance is in `rust-owned-type-text-reference.json`.

Inference memo shadow replay retains identical outputs and counters: Compiler 1,351 lookups/255 hits,
Compiler-Unions 1,513/287; every hit re-walks and checks candidate outcomes/effects without an assertion failure
(`rust-typed-mapper-memo.json`). The public generic/overloaded/composite/defaulted/constrained/contextual-callback
emit fixture retains byte-identical pinned-tsgo output: five normal files and three declaration-only files
(`rust-typed-mapper-emit.json`). These checks do not cover the full emit oracle, 38k-file codebase or complete
API/LSP lifetime audit. Validation log hashes are in `rust-typed-mapper-validation.json`.

## Measurements

Compiler profiling at one checker, four checkers and four with eager members reports zero overlaps and zero
strongly reachable freed blocks. Each accounts for 1,027,839 program references, 477,367 AST nodes and 38,233
binder symbols, without freed/rewound, unrecorded or unreachable references (`rust-typed-mapper-census.json`).
This verifier covers AST/binder/recycling references, not every typed graph edge or API/LSP lifetime. The two
profiling ownership/scope lifecycle cases pass. Extended-diagnostic runs exercise assignment/work profiling and
manual heap reporting (`rust-typed-mapper-extended.json`). Compiler has 28,746 mapper records in capacity 32,768
× 24 = 786,432 record bytes; the public fixture has 18,986 records with the same capacity. These are vector
bytes, excluding array/payload/callback buffers; peak RSS includes actual allocations.

M3 Max macOS, Rust 1.99, public `typescript-go` workloads at
`41f652ab2df5077b1115f73eaeefc2fe9f674132`, `--noEmit --incremental false --pretty false`, with and without
`--singleThreaded`. Original Oxc, `e18a7bc1`, this checkpoint and pre-Oxc binaries were warmed, then measured
in five alternating-order rounds per cell with no other build/validation jobs. All 80 samples agree on status
and stdout hash. Instructions retired, peak RSS, footprint, wall time and executable/changed-Rust-source hashes
are in `rust-typed-mapper-results.json`. Source hashes identify the measured implementation; the patch hash
records the documentation state at measurement time, before this note was finished.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.452083 → 2.415551 | -1.49% | 119.06 → 120.84 | +1.50% |
| Compiler | default | 3.156740 → 3.080392 | -2.42% | 168.64 → 164.41 | -2.51% |
| Compiler-Unions | single | 5.292045 → 5.251082 | -0.77% | 124.25 → 119.84 | -3.55% |
| Compiler-Unions | default | 6.813203 → 6.731281 | -1.20% | 168.56 → 164.75 | -2.26% |

Incremental 1% single-instruction gate: cleared in these local samples. Incremental 5% default-memory gate: not cleared.
Short macOS wall samples do not replace the README's two-publish wall gate or the Linux deterministic
instruction workflow. These samples do not establish complete build-mode/API rebuild memory bounds.

Cumulative single-thread instructions remain +13.87% / +13.73% versus original Oxc. Default peak RSS is
+111.58% / +100.69% versus pre-Oxc (+14.37% / +15.58% versus original Oxc). The complete migration's
instruction/memory gates still fail. This explicitly requested WIP checkpoint must not land as a performance
optimization or be described as memory-preserving.

## Remaining work

Type, symbol, AST and flow records/edges still need typed stores and explicit owners. Source text/static graph
storage, the allocation compatibility runtime, unchecked thread boundaries and disabled leaf retirement remain.
Complete API/LSP lifetime, Linux allocator/process RSS and larger-workload audits remain required, together
with recovery of the complete migration's instruction/RSS gates. The full ownership goal remains active.
