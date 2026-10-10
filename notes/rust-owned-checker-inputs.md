# Rust-owned checker inputs and file lists (2026-10-10)

## Scope and status

This continues the full memory-model migration from `7e9d0a83`. It is an unfinished branch checkpoint, not a
completed graph migration or a performance change ready to land. The prior ownership, residency and rejected
experiments were reviewed (`rust-owned-program-data.md`, `rust-owned-links.md`, `perf-round2-followups.md`,
`mem-round4.md` and `docs/RUST.md` Techniques). This changes production ownership boundaries under the user's
full migration requirement; it does not retry shared checker graphs or inference-scope reclamation.

- `ProgramData` owns the inputs and caches separately from the outer `Program` and its checker pool. Checkers,
  node-builder hosts, built-in compiler pools, project pools and pool factories retain this data through `Arc`.
  The data never retains a pool, preventing an outer program → pool → checker → program reference cycle.
- Program, processed-file, checker and alias-resolver file-list containers use `Arc<[P<SourceFile>]>`. Getter
  borrows follow the container owner; retained lists clone the owner. File parsing and program reuse no longer
  allocate fabricated static file slices. Reuse creates a distinct list after replacing files while retaining
  the same shared processed-file data. The alias resolver also loses its duplicate static root list.
- Auto-import alias resolvers and extraction pools use ordinary shared owners rather than arena-leaked
  resolver references. Node-builder contexts retain their module-specifier host owner through serialization.
- Programs are verified before input data is shared with pool factories; checkers remain lazy. A program root
  is leaked only after its pool factory succeeds, so factory unwinding drops the root and inputs normally.
- The legacy symlink-cache allocation route still uses the shared processed-file container's identity, not the
  new per-version input allocation. A case checks its region retention through reuse and final release.

The outer `Program` still uses `Box::leak` and `free_program`. Built-in checker arrays and leases still have
static/manual lifetime boundaries; project checker leases still use raw pointers and retain regions. Config,
AST, symbol, type and diagnostic entries inside the owned containers remain raw `P` edges. Retaining a file-list
array does **not** retain the source-file referents. Region/address routing, thread arenas and fabricated graph
borrows must still migrate across CLI, LSP/API, incremental and emit paths. No new unsafe thread implementation
or reviewed inventory entry is added. This checkpoint does not establish the full objective.

## Local measurements

Apple M3 Max / macOS / Rust 1.99.0, locked fat-LTO release builds without PGO, using the same pinned Compiler
projects and matching Xcode 14.5 SDK as the preceding stages. Five interleaved rounds after warm-up compare this
implementation, the previous checkpoint, original Oxc (`3dac06d8`) and pre-Oxc (`18175c0e`). The command is
`/usr/bin/time -l <binary> -p <project> --noEmit --incremental false --pretty false`; single mode adds
`--singleThreaded`, default mode uses no checker flag. All 80 runs return the expected status 2 and identical
diagnostics. `rust-owned-checker-inputs-results.json` records raw counters, executable hashes, base commit and
changed Rust source hashes. These are local macOS measurements, not Linux `bench/count.py` or headline publishes.

| Project | Mode | Instructions (G), previous → this stage | Change | Peak RSS (MiB), previous → this stage | Change |
| --- | --- | ---: | ---: | ---: | ---: |
| Compiler | single | 2.137013 → 2.136558 | -0.021% | 114.66 → 112.52 | -1.867% |
| Compiler | default | 2.509471 → 2.513706 | +0.169% | 143.94 → 146.94 | +2.084% |
| Compiler-Unions | single | 4.632952 → 4.632979 | +0.001% | 104.83 → 104.91 | +0.075% |
| Compiler-Unions | default | 5.744485 → 5.745497 | +0.018% | 147.72 → 146.61 | -0.751% |

The default Compiler peak adds 3 MiB against the previous checkpoint, exceeding the incremental RSS limit.
Cumulative Compiler single-thread instructions are +1.110% against original Oxc; default RSS is +2.776% /
+3.087% (+3.97 / +4.39 MiB) on Compiler / Compiler-Unions. Default RSS remains +88.23% / +78.96% against
pre-Oxc (78.06 → 146.94 MiB / 81.92 → 146.61 MiB). The memory and instruction gates still fail. Scheduling and
peak RSS vary between publishes; these short runs make no wall-time or isolated causal claim. The remaining
graph migration must recover the interim costs before landing, followed by Linux and larger-project measurements.

## Verification

Three new cases check independent input/list retention and final release, pool-factory panic cleanup, and the
shared-base symlink-cache route across reused versions. A compile-fail case rejects a file-list borrow escaping
its input owner. The API (77), checker (18), compiler (14), incremental (4), language-service (67), project (103,
two existing ignored) and LSP (41) cases pass. Workspace checks have zero warnings; wasm32-wasip1 and all checker
features compile. The lint ratchet has three baseline findings and none new; source checks retain the same 31
reviewed custom Send/Sync implementations. No lint baseline is raised. The optional compiler build without its
checker feature still fails with seven missing split-check methods; an archived `7e9d0a83` build reproduces all
seven errors. No such build is claimed to pass. Linux/glibc API RSS assertions remain unvalidated on macOS.

All six classifications over the 15,197 pinned TypeScript variants are identical to both original Oxc and
`7e9d0a83`: diagnostics 13,462; types/symbols 12,779 each; JS 13,392; JS maps 149; sourcemap text 156.
The run uses the 120-second per-case limit and has no crashes or timeouts. Default-history diagnostics and
types/symbols classifications also match both saved baselines, including the existing failures. All 28 regression
fixtures pass. This preserves result sets, not a claim that every variant passes.

Fresh full fourslash runs of this checkpoint and `7e9d0a83` have identical per-case pass/fail/skip sets:
4,066 / 63 / 417 out of 4,546. The existing failures are 55 content-mapper and eight `@tsc` cases. The comparison
uses this checkpoint's release executable and an archived previous-commit dev executable solely for fidelity;
no runtime comparison is claimed. Native FSEvents access is enabled for fourslash and LSP checks. No capability
count changes or new feature gaps are claimed; the README capability and benchmark tables stay unchanged.

Remaining completion work includes owning outer programs and checker leases, migrating all AST/type/symbol
referents and allocation boundaries, removing the pointer/static compatibility layer, then verifying the complete
CLI/LSP/API/incremental/emit/oracle and memory/performance scope. Full fourslash is now exercised, but it does not
substitute for those remaining ownership and measurement requirements.

The next checkpoint (`rust-owned-checker-leases.md`) removes the raw checker lease API, leaked built-in checker
arrays and checker-address region map. It does not migrate outer program roots or the raw AST/type referents.
