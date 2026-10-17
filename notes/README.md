# Engineering notes

These notes record measurements, design decisions, regressions and approaches that did not work. They are
evidence from a particular revision and machine; the [compatibility table](../docs/COMPATIBILITY.md) describes
current capabilities, and the [release notes](../docs/STATUS.md) describe shipped changes.

Allocation profiles and reachability-census results in older notes came from tooling that has been removed.
Their measurements remain historical evidence. For supported memory reports and ownership checks, use the
[current debugging guide](../docs/DEBUGGING.md#arena-recycling-checks).

Completed experiments' build wrappers, AST sizing table scripts and Node API probes and audit logs have also been
removed. Their last versions are available at commit `b620e8e5` (for example,
`git show b620e8e5:tools/sizing/compact_ast.py`). References to them below are historical; reusable measurement
tools, code generators and regression harnesses remain in `tools/`.

## Before changing performance or memory use

Start with the [measurement and landing rules](../AGENTS.md#performance-and-memory-changes), then read the
[techniques already in use](../docs/RUST.md#techniques). Search this directory for the affected subsystem before
starting another experiment.

- [Measured and rejected approaches](perf-round2-followups.md#measured-and-rejected-do-not-redo): check this list
  before retrying an idea. The follow-up sections are dated; they are not a live roadmap.
- [Rust string helpers](rust-string-helpers.md): equivalent splitting/whitespace contracts and measurements.
- [Serial work and added complexity](perf-serial-steps.md): why small gains may not justify a cache, thread or
  overlap between phases.
- [Checker instruction-count round 4](perf-icount-round4.md): the later checker measurements and validation.
- [Memory round 4](mem-round4.md): the latest numbered memory round against Bun.
- [Benchmark workflow](../bench/README.md) and [published results](../bench/RESULTS.md): reproduce measurements
  and compare the released build configuration.

## Design decisions

| topic | record |
| --- | --- |
| Compressed arena handles | [Pointer compression](mem-pointer-compression.md) |
| Reclaiming file trees after checking | [Leaf-file regions](mem-free-leaf-files.md) and [their cost](mem-leaf-regions-cost.md) |
| Emit scratch lifetimes | [Per-file emit regions](mem-emit-regions.md) |
| Shared checker graphs, an opt-in experiment | [Shared-graph design and measurements](spike-shared-graph.md) |
| Persisting parse and bind results, a deferred design | [Persisted front end](../docs/PERSISTED_FRONTEND.md) |
| Release build optimization | [PGO](perf-pgo.md) and [build configuration](perf-build-level.md) |
| Decimal number parsing | [Rust decimal parsing](rust-decimal-parsing.md) |

## Correctness and ownership

- [Remaining checker-history dependencies](open-history-dependence.md): cases where file order or checker
  history affects diagnostics.
- [Incremental diagnostic history](fix-history-dependent-diagnostics.md): cache reuse and diagnostic stability.
- [Arena recycling and protected borrows](fix-arena-recycle-uaf.md): the lifetime bug and its regression guards.
- [Protocol quoting styles](shared-go-compat.md): shared helpers retain each caller's escaping behavior.
- [Language-server memory ownership](lsp-mem.md) and [lifetime corrections](lsp-memfix.md): regions, snapshots
  and checker disposal.

## Subsystems and earlier work

For current implementation guidance, use the [emit guide](../docs/EMIT.md),
[language-server guide](../docs/LSP.md) and [Node API guide](../docs/NODE_API.md).
The initial [emit plan](../docs/history/EMIT.md) and [language-server port log](../docs/history/LSP.md)
are archived separately from those guides.

API binary responses use padded standard [Rust Base64 encoding](rust-base64.md).

Other useful entry points are [content mappers](contentmappers.md), [WebAssembly](wasm-build.md),
[performance round 2](perf-round2.md), [performance round 3](perf-round3.md),
[memory round 2](mem-round2.md) and [memory round 3](mem-round3.md).
