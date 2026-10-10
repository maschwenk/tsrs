# mem-flow-compaction: flat label edges and one shared Start for bodyless signatures

**Reverted in https://github.com/maschwenk/tsrs/pull/191 (2026-10-07):** 377 lines of binder representation for -5.4 MB
(0.2% of vscode's peak), below the bar in AGENTS.md ("A performance change must pay for its complexity"). The
measurements below stand; do not re-land the mechanism for this gain.

Two layout-only changes to the binder's flow graph, from the "Flow compaction" row of notes/bun-check-memory.md
(what Bun's `bun check` does: one node-less Start per file, label antecedents as ranges of one per-file edge array).
Base: origin/main 0b4d117.

## What was built

- **Label antecedents** as one arena array per file (a run `[len, edge, ...]` per label) instead of Go's `FlowList`
  cells (8 bytes each, one arena block per cell). `combineFlowLists` for the finally label became a concatenation; a
  reduce label named the label whose antecedents it borrows (the binder only appends to a label, and the try
  statement's labels get no antecedents after their reduce labels are made).
- **One shared Start per file** for function-like containers without a body (signatures, function and constructor
  types, overloads, abstract and ambient members), whose Start node never has `node` set. Nothing can tell node-less
  Starts apart. Constructors and class static blocks kept their own. Where Go's Start becomes an antecedent inside a
  signature (a parameter initializer such as `m(x = a ? b : c): void`, an IIFE, an optional chain in a computed name)
  and so gets Referenced / Shared flags, the binder gave that signature its own Start and moved the references to it
  (`unshare_flow_start`). That path ran in the conformance corpus, not on the bench projects.
- Not done: switch-clause and reduce-label data as fields instead of synthetic nodes (1,191 + 130 nodes, 0.05 MB on
  formbricks-web).

## Numbers

macOS, M5 Max 18 cores, release builds; front end (`--noCheck`, alloc-profile build), allocations including
free-list reuse:

| project | row | before | after |
| --- | --- | --- | --- |
| vscode | `FlowNode` | 2,468,006 / 37.7 MB | 2,425,613 / 37.0 MB |
| vscode | `FlowList` -> `[FlowEdgeWord]` | 1,015,617 cells / 7.7 MB | 7,129 arrays / 2.8 MB |
| vscode | arena requested | 873.8 MB | 868.4 MB (-5.4) |
| formbricks-web | `FlowNode` | 525,746 / 8.0 MB | 313,532 / 4.8 MB |
| formbricks-web | `FlowList` -> `[FlowEdgeWord]` | 110,256 cells / 0.8 MB | 3,323 arrays / 0.3 MB |
| formbricks-web | arena requested | 571.5 MB | 568.8 MB (-2.7) |

Single-threaded instructions within run-to-run noise (vscode 110.89-111.74 G base, 110.97-111.30 G new); peak
footprint at 1 and 4 checkers moved by less than its own spread. Diagnostics, the conformance suite, poison runs, the
census and the binder oracle were identical to the base.
