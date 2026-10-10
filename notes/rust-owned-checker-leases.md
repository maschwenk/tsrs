# Rust-owned checker leases and region retention (2026-10-10)

## Scope and status

This continues the full memory-model migration from `d06e507b`. The branch remains unfinished: the outer program
and AST/type/symbol referents still have legacy lifetime boundaries. The prior ownership, checker-lifetime and
memory evidence was reviewed (`rust-owned-checker-inputs.md`, `lsp-mem.md`, `emit-checker-thread-fix.md`,
`perf-round2-followups.md`, `mem-round4.md` section 6 and `docs/RUST.md` Techniques). This implements the user's
ownership requirement; it does not retry shared checker graphs, inference rollback or a new thread/cache scheme.

- `CheckerHandle` owns a `PooledChecker` exclusively. Taking a checker empties its slot; drop returns the same
  owner once. External pools provide `FnOnce(PooledChecker)` through the safe constructor. The `NonNull` lease
  constructor and unsafe lease dereferences are removed, including their fixture-pool callers.
- The built-in pool retains its slot array through `Arc`. Leases retain that same array independently of the
  pool, so `Box::leak`, manual array freeing and static mutex guards disappear. Slot access uses a short mutex
  borrow and condition-variable wait. Unwinding a held lease poisons later acquisitions, preserving the old
  mutex-guard behavior; poisoned returns wake all waiters.
- Project slots distinguish held from absent using their existing held tags. Request/file affinity, semaphore
  capacity, persistent API identity, cancellation, idle cleanup and global-diagnostic accumulation remain intact.
  The checker is returned before the existing release bookkeeping accesses it.
- Each project `PooledChecker` directly owns its region, dropped after the checker. Acquiring the checker selects
  that owner directly. The checker-address → region map and raw address recovery are removed. Disposed checkers
  remain parked with their regions until pool teardown because program-lifetime diagnostics can refer to them.
- The custom Send/Sync implementations for built-in slots and Sync for `PooledChecker` are removed. Slots inherit
  thread traits through their mutex contents. Whole-checker Send remains reviewed: its Rc values/closures still
  rely on exclusive transfer, and the raw graph/thread traits require later migration. Shared checker references
  no longer have a blanket Sync assertion. The reviewed source inventory falls from 31 to 28, with no additions.

Programs still use `Box::leak` / `free_program`, and retained input containers do not own every AST/config/type
referent. The raw graph, fabricated graph borrows, thread arenas and allocation/address compatibility APIs remain
in scope. Region scopes still select the legacy allocator for graph construction. This is not a claim that the
full CLI/LSP/API/incremental/emit ownership objective or its memory requirements have been achieved.

## Local measurements

Apple M3 Max / macOS / Rust 1.99.0, locked fat-LTO release builds without PGO and with the matching Xcode 14.5 SDK.
Five interleaved rounds after warm-up compare this checkpoint, `d06e507b`, original Oxc (`3dac06d8`) and pre-Oxc
(`18175c0e`) on the same pinned Compiler workloads. The command is `/usr/bin/time -l <binary> -p <project>
--noEmit --incremental false --pretty false`; single mode adds `--singleThreaded`, default uses no checker flag.
All 80 commands have expected status 2 and identical diagnostics. `rust-owned-checker-leases-results.json` records
raw counters, executable hashes, base commit and changed Rust source hashes. Measurement runs after validation
and compilation finish. These local macOS counters do not substitute for Linux `bench/count.py`, larger projects
or comparable headline publishes; no wall-time gain is claimed.

| Project | Mode | Instructions (G), previous → this stage | Change | Peak RSS (MiB), previous → this stage | Change |
| --- | --- | ---: | ---: | ---: | ---: |
| Compiler | single | 2.135894 → 2.136175 | +0.013% | 112.52 → 112.50 | -0.014% |
| Compiler | default | 2.513896 → 2.514778 | +0.035% | 147.41 → 146.66 | -0.509% |
| Compiler-Unions | single | 4.639808 → 4.639132 | -0.015% | 104.89 → 104.86 | -0.030% |
| Compiler-Unions | default | 5.739669 → 5.749916 | +0.179% | 146.58 → 146.34 | -0.160% |

This stage is within the local incremental regression limits against the preceding checkpoint. It does not clear
the performance landing bar or recover the full migration's costs. Against original Oxc, cumulative Compiler
single-thread instructions are +1.075%, and default RSS is +3.530% / +3.036% (+5.00 / +4.31 MiB) on Compiler /
Compiler-Unions. Against pre-Oxc, default RSS is +88.47% / +78.74% (77.81 → 146.66 MiB / 81.88 → 146.34 MiB).
The full memory and instruction gates still fail. Revisit landing after the remaining program/graph ownership
migration recovers these costs and the complete measurement scope is exercised.

## Verification

Five new cases cover exclusive handoff and checker identity, panic poisoning, retention of slots/inputs after
the outer root drops, project-pool/region retention by a lease, and canceled-checker region retention until pool
teardown. The root-drop case keeps the legacy graph in the thread arena; it proves Rust checker/container
ownership, not general graph reclamation. The shared-base region case now observes that owner's destructor
directly: a freed native address can already belong to another concurrent test. The reused-program last-host
case uses single-threaded parsing so temporary worker-held host clones do not affect its final-owner assertion.

Validation uses a fresh `target/rust-ownership` directory after archived previous-commit builds in the old shared
directory produced stale crate artifacts. Final API (77), checker (18), compiler (17), incremental (4), language
service (67), project (105, two existing ignored), and LSP (41) cases pass. The initial LSP run timed out waiting
for a native watcher update during concurrent compilation; the exact case and full suite pass on the unloaded
retry. Native FSEvents access is enabled for LSP/fourslash. Workspace checks have zero warnings, wasm32-wasip1 and
all checker features compile, the ratchet has three baseline findings and none new, and source checks pass with
28 reviewed custom thread implementations. No lint or atomic baseline increases. Linux/glibc API RSS assertions
remain unvalidated on macOS.

The six per-case classifications over 15,197 pinned TypeScript variants match both original Oxc and `d06e507b`:
diagnostics 13,462; types/symbols 12,779 each; JS 13,392; JS maps 149; sourcemap text 156. Runs use the 120-second
limit and have no crashes/timeouts. Default-history diagnostic and types/symbols classifications also match both
saved baselines, including existing failures. Full fourslash retains exactly the same 4,066 pass / 63 fail / 417
skip sets over 4,546 cases. All 28 regression fixtures pass. No capability counts change; README tables stay
unchanged. This is result-set preservation, not a claim that every variant or fourslash case passes.

Next, replace the remaining outer program ownership and its lifetime-fabricated host/root interfaces, then
migrate the AST/type/symbol referents, static graph borrows and allocator compatibility boundaries. Completion
still requires the complete runtime, oracle, lifecycle and memory/performance audit across the original scope.

The following root stage (`rust-owned-program-roots.md`) replaces compiler/incremental root leaks and manual
compiler freeing. Fidelity is preserved, but its instruction/RSS measurements regress. The full graph and allocator
compatibility migration remains unfinished.
