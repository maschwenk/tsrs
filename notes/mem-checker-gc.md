# mem-checker-gc: could a tracing collector replace checker retirement? (2026-10-09)

Question (owner): `--maxMemory` (notes/mem-recycle-checkers.md) gets the 38k-file codebase under 9 GB at 8 checkers
(`8G`: 8.7 GB) by retiring whole checkers, for +35% instructions. Most of what a retired checker held was dead; could a
collector free that in place, without the rebuild?

Answer: not cheaply, and probably not lower. What is dead is not unreachable: it hangs off live per-type and
per-symbol state. Treating the identity caches as weak frees 3% more than plain garbage; adding member tables, node
links and the value-symbol link pages (an upper bound) frees ~37% of the program's memory at mid-run, about half of
the checkers' share. A collector that does that exactly needs weak-key semantics for every id-keyed link store (they
do not keep their keys) and per-type member tables that can be dropped and resolved again: a redesign of the checker's
data model, several weeks. Its gain over retirement would be CPU, not memory: the projection lands near the 8.7-9.7 GB
that `--maxMemory` 8G/9G already reaches.

Tools: branch `exp/checker-gc-census` (the use census of mem/use-census-apps merged onto `mem-notion`, plus the two
experiments below). Alloc-profile build with plain pointers (8-byte handles: absolute sizes ~25% above a release
build), the 38k-file codebase, 8 checkers, Mac.

## 1. How much is dead (epoch census)

Per 16-byte granule: the allocating checker thread and its epoch (files it had finished), and the latest epoch at
which the granule was read (`usebits` read marks). Of the arena bytes a checker had allocated by the time it had
checked a fraction t of its files, never read again after t:

| t | 0.1 | 0.2 | 0.3 | 0.4 | 0.5 | 0.6 | 0.7 | 0.8 | 0.9 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| allocated by t (GB, 8 checkers) | 1.9 | 2.8 | 3.9 | 4.7 | 5.6 | 6.6 | 7.4 | 8.5 | 9.6 |
| never read after t | 68% | 70% | 71% | 72% | 74% | 76% | 79% | 81% | 84% |

Never read at all: 3%. At t = 0.5 by allocation site: transient symbols 1.1 GB (76% dead), union and intersection
member lists and `IntersectionType` (79-93%), member symbol tables (81%), signatures (68%), `TypeMapper` (98-99%),
inference contexts (100%); value-symbol link pages 17% dead. (An uninstrumented read path would count as "never
read", so these are upper bounds.)

## 2. How much a collector could free (reachability census at a mid-run barrier)

`TSRS_GC_SIM_AT=0.5`: every checker stops after half its queue, and the conservative reachability census
(`TSRS_CENSUS=1`) runs from the program with some tables weak (reached, not followed): heap blocks allocated inside
`usebits::weak_scope` (the `PackedMap`/`GoPackedMap` identity caches, string literal tables, reference instantiation
tables) with `TSRS_GC_WEAK_CACHES=1`, and checker-thread arena blocks whose site or type matches
`TSRS_GC_WEAK_ARENA`.

| weak | arena unreachable | heap unreachable | weak blocks reached (dropped too) |
| --- | ---: | ---: | ---: |
| nothing (garbage) | 1.24 GB (11.2%) | 0.16 GB (4.2%) | |
| identity caches | 1.57 GB (14.2%) | 0.25 GB | 0.68 GB |
| + node links | 1.60 GB (14.5%) | 0.25 GB | 0.68 GB |
| + member tables, lazy tables, checker symbol tables, mapped-symbol links (S3) | 2.58 GB (23.3%) | 0.67 GB | 1.16 GB |
| + value-symbol link pages (S4, an upper bound: also drops binder symbols' declared types) | 3.17 GB (28.6%) | 0.68 GB | 1.68 GB |

Of 11.06 GB arena + 3.90 GB heap at the barrier. Even S4 leaves transient symbols 70% reachable and
`IntersectionType` 64% (held by the declared types of declarations and what they reach), and the front end
(identifiers, node lists, flow nodes: ~4.5 GB here) is all reachable, as it must be.

## 3. What an exact collector would need

- Identity caches as weak-value tables: an entry survives iff its value is reachable otherwise. Exact by
  construction (nothing alive can tell a recreated type from the dropped one), given that checking is deterministic,
  which the order independence of diagnostics already relies on. Worth 3% alone.
- Id-keyed link stores (value-symbol links, mapped-symbol links, ...) as weak-key tables: they store ids, not keys,
  so they need their keys (or a liveness bit per id) to know which entries belong to dead transient symbols.
- Per-type state (resolved members, lazy member tables, union property tables) droppable as a whole when none of
  its symbols is reachable from outside the type, and resolved again on demand.
- Precise or conservative tracing of compressed 32-bit handles and of the heap buffers arena objects own; block
  boundaries in region chunks (an allocation bitmap); a sweep that runs the region drop list for dead blocks; page
  release or compaction to lower RSS; poison-mode and conformance verification with a collection after every file.

## 4. Verdict

Below the bar for now: weeks of work whose gain over `--maxMemory` is mostly the 15-35% of instructions that
retirement spends rebuilding, at about the same memory. If revisited, start with the weak-key link stores (they hold
most of what S3 leaves reachable) and measure each step with `TSRS_GC_SIM_AT`.
