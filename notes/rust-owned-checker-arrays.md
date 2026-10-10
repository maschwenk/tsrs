# Owned checker cache and link arrays (2026-10-10)

This continues the full ownership goal from `b67899a0`. Prior ownership results, the rejected list,
`docs/RUST.md` Techniques and `mem-round4.md` section 6 were reviewed. The changed constraint remains the user's
requirement to remove the custom memory model. This is an unfinished ownership checkpoint; raw graph handles
and the compatibility runtime still remain. It is not a completed migration or a performance landing.

## Ownership changes

Deferred symbol constituents, switch-clause types and witnesses, variance results, containing-module lists,
anonymous type parameters, widening properties and template-text lists own their array storage. Mutable fields
use `ArrayCell` or `OptionArrayCell`; returned snapshots retain the array while recursive operations run. Optional
fields preserve nil versus a computed empty list. The strings and `P` entries inside these lists remain legacy.

Subtype-reduction, symbol-alias, per-file container and accessible-chain cache values use owned boxed arrays.
Callers needing a result across mutation copy it into a local vector before borrowing the checker again. These
cache values need no shared array header. Global built-in iterator lists retain shared views; array variance is
an ordinary fixed Rust array. Node-builder signature options own their modifier list.

Deferred mapper closures capture owned type-parameter views instead of an arena slice. Instantiation-expression
state borrows its arguments for the synchronous call; type-argument constraint checking keeps an owned local
vector. Array inputs are no longer promoted to static arena slices at these boundaries. `TypeNodeLinks` shrinks
from 32 to 24 bytes natively and from 16 to 12 on Wasm. The heap census counts owned cache-array values and the
built-in iterator array storage.

The remaining explicit static arrays in the checker are Rust static feature/literal tables and the process-wide
primitive-alias suggestion vector in a real `OnceLock`. That vector already owns its array; its symbols still use
the legacy thread arena. AST/symbol factories called by the checker still build legacy graph arrays. The absence
of static cache-array fields does not remove those graph boundaries.

## Validation

The pinned Rust 1.99 toolchain, fresh Cargo target and macOS 14.5 SDK were used:

- Workspace checks with test targets, CLI/runner all-feature checks and wasm32-wasip1 checks pass.
- Core's 103 unit and five compile-fail cases, checker's 20 unit and one compile-fail case, compiler's 20 unit and
  two compile-fail cases, LS's 67, project's 105 with two existing ignored cases, and API's 77 unit/integration
  cases pass. Execute's three ownership/hook cases and five native WASM cases pass. The empty tsctests entry
  runs no Go scenarios. Two profiling lifecycle/scope cases also pass.
- The 41 native LSP cases, three CLI API cases, API/CLI output equality, six default emit cases and all 28
  regression fixtures pass.
- The lint ratchet has three existing findings and no new ones. Source inventory remains 615 files, 24 reviewed
  thread implementations and 74 old uncommented orderings. No unsafe block or custom thread trait was added.
- All 15,197 full conformance IDs and six baseline classifications match `b67899a0` and original Oxc: 13,462
  diagnostic, 12,779 types/symbols, 13,392 JS, 149 JS-map and 156 source-map-text passes, with no crashes/timeouts.
  Default-history diagnostics/types/symbols match every prior classification, including existing failures.
  Fourslash's exact sets remain 4,066 passes, 63 failures and 417 skips. Hashes are in
  `rust-owned-checker-arrays-classifications.json`. Capability-table pass counts are unchanged.

The comparisons use the pinned reference fixtures already present, not a newly built tsgo. Full Go scenarios,
oracle audits and Linux allocator/proc RSS cases remain unavailable in this macOS fixture checkout.

Compiler census runs with one checker, four checkers, and four with eager members each report zero overlaps and
zero strongly reachable freed blocks. All account for 1,027,839 program references, 477,367 AST nodes and 38,233
binder symbols, with none into freed storage or unrecorded/unreachable. All return the expected diagnostic status
2. Binary/log hashes are in `rust-owned-checker-arrays-census.json`. The precise verifier covers AST/binder/recycling
references, not every type edge. These runs do not establish full API/LSP or 38k-file-codebase lifetime coverage.
The manual heap report includes the new owned-cache-array rows.

## Measurements

M3 Max macOS, the same `typescript-go` workloads at `41f652ab2df5077b1115f73eaeefc2fe9f674132`,
`--noEmit --incremental false --pretty false`, with and without `--singleThreaded`. Original Oxc, `b67899a0`,
this checkpoint and pre-Oxc binaries were warmed, then measured in five alternating-order rounds per cell without
other validation jobs. All 80 samples agree on exit status and stdout hash. Instructions, peak RSS, footprint,
wall times, executable hashes and source hashes are in `rust-owned-checker-arrays-results.json`.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.387480 → 2.384912 | -0.11% | 119.08 → 118.22 | -0.72% |
| Compiler | default | 3.064126 → 3.060342 | -0.12% | 163.03 → 162.14 | -0.55% |
| Compiler-Unions | single | 5.015461 → 5.009111 | -0.13% | 122.53 → 121.14 | -1.13% |
| Compiler-Unions | default | 6.429944 → 6.417982 | -0.19% | 163.88 → 162.64 | -0.75% |

The local changes stay within incremental regression limits, but no gain clears the performance-change landing
bar. Short macOS wall samples have 0.01-second resolution and do not replace the README's two-publish wall gate.
These no-emit samples do not establish API rebuild RSS bounds or the complete build-mode memory profile.

Cumulative single-thread instructions are +12.49% / +8.41% versus original Oxc. Default RSS is +108.88% /
+97.74% versus pre-Oxc (+15.74% / +14.65% versus original Oxc). The full migration's instruction and memory
gates still fail. This checkpoint must not land as a performance optimization or be called memory-preserving.

Full typed graph storage, graph strings and the allocation/runtime compatibility layer remain unfinished; owning
the array buffers does not make their raw referents safe. Complete those boundaries and recover the gates before
calling the memory model migration finished.
