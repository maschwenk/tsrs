# Rust-owned build tasks and command inputs (2026-10-10)

This continues the full ownership goal from `f41586f0`. It remains an unfinished branch checkpoint. Earlier
ownership evidence, the rejected list in `perf-round2-followups.md`, `mem-round4.md` section 6 and `docs/RUST.md`
Techniques were reviewed. The changed constraint is the user's requirement to remove the custom memory model.
This is not a checker-sharing or caching optimization, and it is not ready to land as a performance change.

## Ownership changes

Build tasks are ordinary Rust-owned records in `ArenaBuilder<Arc<BuildTask>>`. The path map and upstream edges
store qualified typed keys. Resolving a key takes a short store lock and clones a record lease before callbacks,
waits, recursive graph growth or checker work. Circular dependency edges cannot retain an ownership cycle.
Semantic graph traversal, ordering, task reuse, signals and build-info transfer follow the same control flow.
The store is append-only for an orchestrator's lifetime; the API already replaces its orchestrator on a build.

Tasks no longer live in a legacy allocation region that an unreported program might retain. The orchestrator's
manual extraction of unreported program roots on drop disappears. Ordinary field destruction releases programs
even when a task result lock is poisoned. The remaining Drop implementation only prints optional region stats.
A lifecycle case creates circular dependency keys, retains an unreported incremental program in a task result,
poisons its result lock, then verifies that both task records, the program and its allocation region are released.

Build options own their system, parsed command and testing hooks. API handles own their input system and command;
returned build outcomes retain them through the orchestrator. The existing replacement/disposal case now verifies
these inputs with weak owners as well as consuming the old diagnostic/source text. CLI and WASM entry points
own their systems. Reporters retain owned system/sink/callback state when needed; ordinary helpers borrow it.
System wrappers forward the complete interface, including overridden clocks and diagnostic sinks.

The tsctests harness owns its systems and testing trace state. The trace keeps its callback owner until it drops;
a focused case verifies the fake clock overrides, package-json trace-cache order and final host release. Scenario
workers use scoped threads instead of manufacturing a static scenario reference. The full Go scenario dump was
not available, so the empty tsctests entry is not evidence of tsctests baseline parity.

## Returned WASM diagnostics

Native WASM checks exposed another diagnostic lifetime bug: after compilation the command driver discarded the
compile result and its graph owner before a diagnostic sink encoded collected diagnostics. The JSON encoder
failed with an unknown diagnostic message. `CommandLineResult` now retains the actual compiler or incremental
compile-result owner. WASM keeps that result through JSON encoding. The existing encoder-equality case passes;
a new case consumes type-error diagnostics after the system's initial owner is transferred to the command,
then confirms the final result drop releases the system. This fixes a raw diagnostic escape boundary; diagnostic
fields themselves still use legacy graph pointers.

## Validation

Fresh Cargo target with the pinned Rust 1.99 toolchain and macOS 14.5 SDK:

- Workspace checks with test targets, CLI all-feature check and wasm32-wasip1 check pass.
- Two execute ownership cases, the scoped testing trace/clock case and five native WASM cases pass. The empty
  tsctests runner also returns successfully, but it ran no Go scenarios.
- Three CLI API cases, API/CLI build-output equality and six default emit cases pass.
- 41 native LSP cases and 28 regression fixtures pass.
- The lint ratchet has three existing findings and no new ones. Source checks retain 24 reviewed thread
  implementations and 74 old uncommented orderings; no new custom thread trait or weakened ordering was added.
- Full six-baseline conformance has 15,197 entries: 13,462 diagnostic, 12,779 types/symbols, 13,392 JS, 149 JS-map
  and 156 source-map-text passes, with no crashes/timeouts. Every classification is identical to original Oxc and
  `f41586f0`. Default-history diagnostics/types/symbols also have identical ID sets and classifications, including
  the existing failures. Fourslash has the same 4,066 passes, 63 failures and 417 skips as `f41586f0`.

These are the pinned reference fixtures already present in the checkout, not a newly built tsgo comparison. The
Linux API RSS cases require their Linux allocator/proc environment; they were not counted as native macOS checks.

## Measurements

M3 Max macOS, the same `typescript-go` workloads at `41f652ab2df5077b1115f73eaeefc2fe9f674132`,
`--noEmit --incremental false --pretty false`, with and without `--singleThreaded`. Four binaries (original Oxc,
`f41586f0`, this checkpoint and pre-Oxc) were warmed, then run in five alternating-order rounds per cell with no
other validation jobs. All 80 samples agree on exit status and stdout hash. Raw instructions, RSS, footprint,
wall times, binary hashes and source hashes are in `rust-owned-build-tasks-results.json`.

| Project | Mode | Instructions, billions before → after | Change | Peak RSS, MiB before → after | Change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.337985 → 2.337321 | -0.03% | 115.05 → 115.19 | +0.12% |
| Compiler | default | 2.951051 → 2.950636 | -0.01% | 162.59 → 160.25 | -1.44% |
| Compiler-Unions | single | 4.870839 → 4.869307 | -0.03% | 116.42 → 117.08 | +0.56% |
| Compiler-Unions | default | 6.186000 → 6.205870 | +0.32% | 162.19 → 162.50 | +0.19% |

Local single-thread instruction changes are -0.03% on both projects; default RSS changes -1.44% / +0.19%.
These are below the performance-change landing bar. Short macOS time samples have 0.01-second resolution;
they are not a replacement for the README's two-publish headline wall gate. These no-emit workload samples also
do not establish API rebuild RSS bounds or the full build-mode memory profile.

Cumulative single-thread instructions are still +10.60% / +5.49% versus original Oxc. Default RSS is still
+105.61% / +98.10% versus pre-Oxc (+13.51% / +13.88% versus original Oxc). The full migration's instruction and
memory gates still fail. Do not call this checkpoint memory-preserving or land it as a performance optimization.

## Required remaining work

AST/type/symbol/config/snapshot/cache edges still use raw `P`, packed pointers, fabricated static graph borrows
and address-based allocation-owner lookup. The thread arenas and compatibility allocation layer remain. Owning
task/input roots does not migrate those referents, remove their unchecked thread assumptions, or establish full
compiler graph ownership. Independently retained testing programs still need a host/root lifetime audit.

Complete the typed graph owner migration and delete the raw compatibility memory model. Run the full Go
scenario/oracle audits, Linux API/runtime/census lifecycle checks and larger workloads, then recover and measure
all instruction/RSS gates before declaring the full goal achieved or landing it.
