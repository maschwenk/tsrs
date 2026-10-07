# mem-flow-compaction: flat label edges and one shared Start for bodyless signatures

Two layout-only changes to the binder's flow graph, from the "Flow compaction" row of notes/bun-check-memory.md
(what Bun's `bun check` does: one node-less Start per file, label antecedents as ranges of one per-file edge array).
Diagnostics are unchanged by construction and were checked byte for byte (Gates below). Base: origin/main 0b4d117.

## What changed

**Label antecedents.** Go links a label's antecedents through `FlowList` cells (8 bytes each with compressed
pointers, one arena block per cell). Now the binder keeps them in two binder-local vectors while it binds the file
(the label's link word holds its slot there), and when the file is bound writes every live label's antecedents into
one arena array per file: a run per label, `[len, edge, edge, ...]`, padded to 8 bytes because a `P` target must be
8-aligned. The label's link then points at its run, and `FlowNode::antecedents()` returns a `&'static [P<FlowNode>]`.
Recycled labels (notes/mem-recycle.md) are dropped from the slot table instead of freeing their cells. Go's
`combineFlowLists` for the finally label becomes a concatenation into a fresh slot; a reduce label now names the
label whose antecedents it borrows (`FlowReduceLabelData::antecedents`), which is the list Go shares: the binder only
appends to a label, and the try statement's return / exception / normal-exit labels get no antecedents after their
reduce labels are made. The checker's walks (`get_type_at_flow_node`, the branch and loop label handlers,
`is_reachable_flow_node_worker`, `is_post_super_flow_node_worker`, the flow memo's `ends_iteration` test) iterate the
slice instead of the list.

**Shared Start.** Every function-like control flow container without a body (method and call signatures, construct
signatures, function and constructor types, overloads, abstract and ambient members) got its own 16-byte Start node
whose `node` is never set (the binder sets it only for function expressions and object literal / class expression
methods). Now these use one Start per file (`uses_shared_flow_start`). Nothing can tell node-less Starts apart: the
checker answers the initial type at one, its shared-node caches and the flow memo are keyed by the walk's reference
and declared / initial types as well as the node, and flow node ids are assigned lazily. Constructors (always given a
return label, which references the Start) and class static blocks (immediately invoked) keep their own.

Go's Start can still become an antecedent inside a signature (a parameter initializer such as `m(x = a ? b : c): void`,
an IIFE in one, an optional chain in a computed name), which gives it Go's Referenced / Shared flags. To keep every
flow node's flags what Go has, the binder clears the shared Start's flags on entry to such a signature, and on exit, if
they are not plain `Start` any more, gives that signature a Start of its own with those flags and moves every reference
made inside it (edges, flow nodes made with an antecedent while inside, the signature's AST) to it, then restores the
flags of the enclosing signature (`unshare_flow_start`). This path never runs on the bench projects' diagnostics but
does in the conformance corpus.

**Not done:** switch-clause and reduce-label data as fields instead of synthetic nodes. They need a wider `FlowNode` or
a side table, and are 1,191 + 130 nodes (0.05 MB) on formbricks-web.

## Numbers

macOS, M5 Max 18 cores, release builds; other agents were running, so wall time is not reported.

Allocation profile, front end (`--noCheck`, alloc-profile build, `TSRS_ALLOC_PROFILE_TOP`). Counts are allocations,
including free-list reuse:

| project | row | before | after |
| --- | --- | --- | --- |
| vscode | `FlowNode` | 2,468,006 / 37.7 MB | 2,425,613 / 37.0 MB |
| vscode | `FlowList` -> `[FlowEdgeWord]` | 1,015,617 cells / 7.7 MB | 7,129 arrays / 2.8 MB |
| vscode | arena requested | 873.8 MB | 868.4 MB (-5.4) |
| formbricks-web | `FlowNode` | 525,746 / 8.0 MB | 313,532 / 4.8 MB |
| formbricks-web | `FlowList` -> `[FlowEdgeWord]` | 110,256 cells / 0.8 MB | 3,323 arrays / 0.3 MB |
| formbricks-web | arena requested | 571.5 MB | 568.8 MB (-2.7) |

vscode has few bodyless signatures outside the libs (42K shared Starts); formbricks-web, with its dependencies'
declaration files, has 212K. Heap current / peak are unchanged within 0.3 MB (the binder's edge vectors live only
while a file is bound).

`/usr/bin/time -l`, three runs each, base / new:

| project | instructions, `--singleThreaded` | peak footprint, `--singleThreaded` | peak footprint, `--noCheck` |
| --- | --- | --- | --- |
| vscode | 110.89-111.74 G / 110.97-111.30 G | 2049-2059 MB / 2047-2051 MB | 1257-1261 MB / 1260-1263 MB |
| formbricks-web | 53.48-54.43 G / 53.45-54.31 G | 1436.6-1436.7 MB / 1431.0-1433.3 MB | 1039-1042 MB / 1036-1037 MB |

Instructions are within run-to-run noise. Peak footprint at 4 checkers moves by less than its own spread (vscode
2266-2293 / 2279-2302 MB, formbricks-web 1814-1830 / 1816-1819 MB).

## Gates

- `--pretty false` stdout plus exit code byte-identical to the base on vscode, webpack, xstate-main, mui-docs, cal-diy,
  formbricks-web, supabase-studio, t3code-server, Compiler and Compiler-Unions at 1, 4 and 16 checkers (30/30).
- Conformance suite (`tsrs-test run --suite all --baselines types,symbols`): 13,458 error baselines and 12,779
  `.types` / `.symbols` pass, pass list identical to the base.
- `TSRS_ARENA_POISON=1` at 16 checkers on vscode and formbricks-web: output identical to the base.
- Census (`TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1 TSRS_CENSUS_ASSERT=1`, alloc-profile + `tsrs_core/plain-ptrs`, so the
  8-byte-key layout of the edge array ran too), one checker: vscode 40.1M and formbricks-web 24.4M program references
  checked (now including each label's edge run), 0 to freed blocks, 0 strong-mark violations, diagnostics unchanged.
- Binder oracle (`tools/oracle/binder`, Go built from the pinned commit) over the 12,760 conformance and compiler test
  files: 12,752 identical, the 8 others are the known non-UTF-8 / BOM files (positions differ by the lossy decoding),
  as before. The Rust dumper numbers a node-less Start once per innermost bodyless signature it is reached from, as
  Go's distinct Starts are numbered. A hand-written file with parameter initializers, IIFEs with try/finally, optional
  chains and nested function types in signatures (the unshare path) also dumps identically to Go.
