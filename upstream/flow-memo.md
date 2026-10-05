checker: remember complete flow-analysis results across walks of the same reference

**Status:** written up only, not opened upstream. Prototyped in tsrs (`crates/tsrs_checker/src/flowmemo.rs`,
notes/perf-flow-union-inference.md), where it is on by default and produces byte-identical output.

## The pattern

`getFlowTypeOfReference` walks the flow graph backwards from the reference (`getTypeAtFlowNode`). Its caches are per
walk (`sharedFlows`, reset by every `getFlowTypeOfReference`) and, per reference key, at loop labels (`flowLoopCache`).
A function that mentions the same variable or property path R times therefore walks the common chain R times: the
cost is R × the distance to the declaration. On webpack (`lib/`, JS with JSDoc) and vscode (`src`) a work census
attributes 16-19% of one checker's check time to flow walks, and 64-67% of the walk steps revisit a
(reference key, flow node) pair an earlier walk of the same key already computed.

## The change

Keep the result of every recursive `getTypeAtFlowNode` call (a "frame": the type at the node it started at, for this
reference) in a table keyed by (flow node, reference key), and consult it at the start of each frame. The reference
key is the one `getFlowReferenceKey` builds (root symbol, property names, declared type, initial type, flow
container) extended by the reference's syntactic shape: tsrs keys only identifiers, `this` and plain property-access
chains on them, which is everything that the walk reads from the reference besides two things handled below. Also
consult at the node that ends a frame's iteration (a condition, a switch clause, a label with several antecedents)
and at "checkpoints", every fourth node the iteration passes, chosen by hashing the node so that walks starting at
different places agree on them; a frame's answer is the same at all of them, so it is stored under each.

A result may be stored only if Go, walking the same sub-graph for the same key at any later time, computes the same
type and changes nothing observable on the way. In tsrs that is enforced as follows; every point is needed for exact
output on the conformance suite or one of the measured projects.

1. **Nothing transient.** A frame is not stored if it consumed: a loop label in process (the incomplete types),
   an in-progress or circular type resolution (`pushTypeResolution` finding its target on the stack, the "is
   resolving" checks), a re-entered `getResolvedSignature`, a reduce label (try/finally), the depth limit, or the
   five-level inline limit of `narrowType`; or a `sharedFlows` entry whose computation consumed one of these. This is
   tracked with serial numbers: every frame, walk, loop-stack entry and type resolution gets one; an event carries the
   serial of its source and taints the active frames younger than that source. A loop label's frame is older than
   its stack entry, so the loop's result (which Go caches in `flowLoopCache` for good) is not tainted by its own back
   edges, while every frame computed under them is; a resolution's final result likewise. A `sharedFlows` value whose
   source is no longer active (Go reuses values computed during a loop's back edge after the loop finished, through
   `break` paths) taints every active frame.
2. **Nothing specific to the reference node.** `getNarrowableTypeForReference` (assignments in `getInitialOrAssignedType`)
   reads the reference's position when the assigned type contains a type variable with a union constraint, and
   `isConstantReference` of a property access reads the property symbol that node resolved to (two `o.v` with the same
   key can resolve to different symbols after narrowing `o`). Either taints the frames of the current walk.
3. **The same instantiation counters.** `checkExpressionEx` resets `instantiationCount` and every instantiation can
   count toward TS2589 or hit the depth check. A frame is not stored if `totalInstantiationCount` changed during it or
   an instantiation was active when it began (an active mapper's cache can turn a hit into a miss later); if the count
   may have been reset during it (it was 0 at the start, or changed), it is used only while the count is 0.
4. **No new identities.** A frame that created a type, symbol or signature is not stored: an object literal assigned to
   an auto-typed `let` gets a fresh type on every evaluation, so Go's later walk returns a new object (and union order
   follows type ids).
5. **The same caches.** A frame that read `flowTypeCache` is used only while that cache is current (it is reset by
   `checkExpressionCached` and loop back edges; within one, an expression's entry never changes). A frame is used only
   when no loop analysis is in progress, no instantiation is active, and the current walk has no transient
   `sharedFlows` value.
6. **The depth limit.** Each entry stores the height of the sub-walk it stands for, computed as an upper bound (hits
   count with the height they stand for), and is used only if `depth + height < 2000`, so TS2563 fires in Go's place.
   A memo hit does not leave in `sharedFlows` the shared nodes Go's walk of that sub-graph would have cached, so a
   later part of the same walk can be deeper than Go's; if a walk that used the memo reaches the limit, it is redone
   without the memo.
7. **Faithful `sharedFlows`.** Go records only the last shared node of a frame's iteration; a hit at a node that does
   not end the iteration records nothing (a later visit computes or hits again).

What it does not need: the result is the same object as Go's (every type constructor on the way is interned once
condition 4 holds), so union order and printed types are unchanged; diagnostics produced inside skipped sub-walks
were produced by the first walk (Go deduplicates them).

## Results (tsrs, one checker, check-phase instructions, Apple M-series)

| project | before | after | delta |
| --- | --- | --- | --- |
| webpack | 10.93 G | 10.18 G | -6.9% |
| vscode (`src`) | 89.5 G | 86.7 G | -3.1% |
| 38k-file private monorepo | 227.7 G | 226.1 G | -0.7% |
| mui (`docs`) | 43.8 G | 43.6 G | -0.5% |
| xstate | 5.35 G | 5.36 G | +0.1% |

These are paired runs of builds made before merging the latest main. The final whole-process runs (median of 3,
release builds) gave the following instruction deltas:

| project | 1 checker | 4 checkers |
| --- | --- | --- |
| webpack | -5.0% | -4.5% |
| vscode | -2.2% | -2.1% |
| 38k-file monorepo | -0.4% | -0.3% |
| mui | -2.1% | +0.1% |
| xstate | +0.1% | +0.05% |

Peak memory moved by less than 0.5%.

The cost is paid per walk and per frame: computing the key, the bookkeeping and the stores. It pays off where
functions are long and mention the same variables often. It is neutral on code made of short functions.

## To reproduce in Go

Hook `getTypeAtFlowNode`: compute the extended key once per `getFlowTypeOfReference` (only for the shapes above), keep
the taint/height registers in the checker (saved and restored around each frame), and add the event calls at
`getTypeAtFlowLoopLabel` (in-process hit, stack push), `findResolutionCycleStartIndex` (found), the reduce label,
`narrowType`'s inline limit, `getInitialOrAssignedType` and `isConstantReference`. A shadow mode that walks every hit
again and compares (by identity) is what found conditions 4 and 7; run it over the conformance suite and large
projects before trusting the fast path.
