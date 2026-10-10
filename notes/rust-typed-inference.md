# Checker-owned inference records (2026-10-11)

This continues the complete ownership request from `de874e59`. Prior inference/ownership notes, the measured
and rejected list, `docs/RUST.md` Techniques and the latest memory round summary were read before changing
storage. The changed constraint is the explicit complete memory-model migration request. This is an unfinished
migration-branch checkpoint; the full graph ownership and performance gates remain open.

## Ownership changes

Each checker owns typed Rust vectors for inference contexts, candidate-info records and scratch states. All
references to these three records are qualified eight-byte keys; no `P<InferenceContext>`, `P<InferenceInfo>` or
`P<InferenceState>` remains. Mapper callbacks and recursive inference resolve keys through the checker. Context
mapper creation receives its key explicitly, removing the unsafe cast that manufactured a context self pointer.
Fixing/non-fixing mappers remain lazy and unique per context; the eager opt-out retains its original order.

The Go-style linked scratch free list is replaced by a standard Rust vector of keys in the same LIFO order.
Returning a scratch key clears the same candidate/source/target/priority/stack fields. No record is reused until
its key is returned. Records, candidate buffers, callbacks and rare tails run ordinary Rust destructors. Context
and info headers remain 72/48 bytes on native and 40/28 on Wasm. The scratch record loses its linked-list pointer.
Type and mapper references inside these records remain legacy graph edges.

The public context accessor borrows its checker and rejects foreign storage keys. Unit coverage retains an
inference-key array view across growth/replacement and store destruction, verifies owner rejection and releases
an actually captured callback when the store drops. The retained view owns only keys, not their referents. A
compile-fail case prevents a context borrow from becoming static. Deep-recursion checks snapshot the two scratch
stacks before mutating the checker. This adds no unsafe code or unchecked thread-trait implementation.

Profiling layouts treat inference/alias/signature keys as scalar IDs and mapper function addresses as scalars,
not data pointers. Manual heap reporting includes all three new record-vector capacities and the scratch pool.
These corrections do not turn the existing census into a complete typed-graph tracer.

## Validation

Normal workspace all-targets, CLI/runner all-feature targets and wasm32-wasip1 checks pass without warnings.
Core 104 unit/five compile-fail, AST 22, checker 26/three compile-fail, compiler 20/two compile-fail, LS 67,
project 105 (two old ignored), API 77, execute four and native Wasm five cases pass. Supported CLI and native LSP
checks pass, as do all 28 regression fixtures. The full CLI invocation still records the two existing leaf-retirement
counter failures; they are not hidden or counted as passes. Linux-only allocator/RSS behavior remains unvalidated
on macOS. The ratchet has three old findings and none new; source inventory remains 617 files, 24 reviewed thread
implementations and 74 old uncommented orderings, with no baseline/inventory increase.

Every full conformance ID and all six classifications match `de874e59` and original Oxc: diagnostics 13,462,
types/symbols 12,779, JS 13,392, JS-map 149 and source-map-text 156 passes. Default-history classifications match
exactly, including their existing failures. A paired previous/current replay with both `TSRS_LAZY_MEMBERS=0`
and `TSRS_LAZY_INFERENCE_MAPPERS=0` also matches exactly. Fourslash retains 4,066 pass, 63 fail and 417 skip sets.
No conformance crashes/timeouts occur (`rust-typed-inference-classifications.json`). Fresh pinned-Go scenario
replay retains 184/216 tsc and 187/190 tsbuild passes, 35 existing failures and zero crashes; only the already-failing
unsanitized internal-symbol-name output differs (`rust-typed-inference-tsctests.json`). Watch/content mappers
remain excluded; exact reference provenance is in `rust-owned-type-text-reference.json`.

Inference memo shadow replay has identical outputs and counters before/after: Compiler 1,351 lookups/255 hits,
Compiler-Unions 1,513/287. Every hit re-walks and checks candidate outcomes/effects; no assertion fails
(`rust-typed-inference-memo.json`). A public emit fixture adds defaulted/constrained type parameters and contextual
callback inference to the prior generic/overloaded/composite/alias/text fixture. All five normal output files and
three declaration-only files match freshly built pinned tsgo byte for byte (`rust-typed-inference-emit.json`).
These checks do not cover the full emit oracle, 38k-file codebase or complete API/LSP lifetime audit.

## Measurements

Compiler profiling at one checker, four checkers and four with eager members reports zero overlaps and zero
strongly reachable freed blocks. Each accounts for 1,027,839 program references, 477,367 AST nodes and 38,233
binder symbols, without freed/rewound, unrecorded or unreachable references (`rust-typed-inference-census.json`).
This verifier covers AST/binder/recycling references, not every typed graph edge or API/LSP lifetime. The two
profiling ownership/scope lifecycle cases pass. Extended-diagnostic runs exercise assignment/work profiling and
heap reporting (`rust-typed-inference-extended.json`). Compiler has 7,963 contexts (capacity 8,192 × 72 = 589,824
record bytes), 8,606 infos (capacity 16,384 × 48 = 786,432 bytes) and two scratch records (capacity four × 152 =
608 bytes). The LIFO key vector has capacity four × eight = 32 bytes. These are vector bytes, excluding candidate,
array and rare-tail buffers; peak RSS includes those allocations. Validation log hashes are recorded in
`rust-typed-inference-validation.json`.

M3 Max macOS, Rust 1.99, public `typescript-go` workloads at
`41f652ab2df5077b1115f73eaeefc2fe9f674132`, `--noEmit --incremental false --pretty false`, with and without
`--singleThreaded`. Original Oxc, `de874e59`, this checkpoint and pre-Oxc binaries were warmed, then measured
in five alternating-order rounds per cell without other build/validation jobs. All 80 samples agree on status
and stdout hash. Instructions retired, peak RSS, footprint, wall time and executable/changed-Rust-source hashes
are in `rust-typed-inference-results.json`. The source hashes identify the measured implementation; the patch
hash records the documentation state at measurement time, before this note was finished.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.456783 → 2.449434 | -0.30% | 122.73 → 119.06 | -2.99% |
| Compiler | default | 3.167439 → 3.150534 | -0.53% | 170.09 → 166.17 | -2.31% |
| Compiler-Unions | single | 5.295143 → 5.290545 | -0.09% | 125.05 → 124.25 | -0.64% |
| Compiler-Unions | default | 6.812975 → 6.826820 | +0.20% | 169.88 → 170.67 | +0.47% |

The incremental changes clear neither the 1% single-thread instruction bar nor the 5% default-memory bar.
Short/noisy macOS wall samples do not replace the README's two-publish wall gate or the Linux deterministic
instruction workflow. This does not establish complete build-mode/API rebuild memory bounds.

Cumulative single-thread instructions remain +15.87% / +14.66% versus original Oxc. Default peak RSS is
+113.13% / +108.33% versus pre-Oxc (+18.95% / +19.19% versus original Oxc). The complete migration's
instruction/memory gates still fail. This explicitly requested WIP checkpoint must not land as a performance
optimization or be described as memory-preserving.
## Remaining work

Type, mapper, symbol, AST and flow records/edges still need typed graph stores and explicit owners. Source
text/static graph storage, the allocation compatibility runtime, unchecked thread boundaries and disabled
leaf-file retirement remain. Complete API/LSP lifetime, Linux allocator/process RSS and larger-workload audits
remain required, along with recovery of the complete migration's instruction/RSS gates. The full goal is active.
