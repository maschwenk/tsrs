# mem-round3: peak memory, third pass (representation and lazy creation)

Follow-up to notes/mem-layout.md, notes/mem-lazy.md and notes/mem-round2.md. Track A changes only the
representation (no checker semantics); track B prototypes Go-portable "created and never used" candidates on the
type side behind `TSRS_LAZY_*` switches.

Gates for every step: the suite (errors and `--baselines types,symbols`) byte-identical to the base commit, as whole
`target/test-results` trees, in the default mode, with `TSRS_LAZY_MEMBERS=0`, and with
`TS_TEST_PROGRAM_SINGLE_THREADED=false`; opt-out counters 25,973,354 / 9,639,962 / 44,884,281 single and
39,704,001 / 16,200,921 / 89,981,648 with `--checkers 4 --checkerAssignment go`; default counters unchanged
(12,811,032 / 9,630,120 / 44,820,708 single, 16,549,988 / 13,788,912 / 76,888,800 on 4 checkers); Project 0 errors.

Measurement: `/usr/bin/time -l` on the pristine Project checkout, rounds interleaved, medians of 3, "GiB" = peak
memory footprint / 2^30. The machine was shared with two other agents (load 8 to 60), so wall and check times are
noise; instructions retired are the CPU-work measure.

## Occupancy at the start (main at dc59d8e, default mode, single)

Counted once with throwaway instrumentation (all types registered at creation, all value-symbol link slots walked
at exit, relation caches inspected):

| structured types | count | any resolved member set |
| --- | --- | --- |
| object (anonymous, instantiated) | 3,006,176 | 2,194,820 (73%) |
| type references | 2,106,919 | 197,751 (9%) |
| intersections | 1,133,593 | 25,399 (2%) |
| unions | 1,020,584 | 660 (0.06%) |
| mapped | 272,161 | 153,702 (56%) |
| interfaces, tuples, reverse mapped, ... | 40,545 | 25,343 |

`ValueSymbolLinks` slots: 11.05M, of which 0.98M are never written (all four words empty), 1.14M have only
`resolved_type`, 1.67M only `target` + `mapper`, 3.05M `resolved_type` + `target` + `mapper`, and 2.50M a rare tail.
No two-tier split pays: `target`/`mapper` are set on 61% of the slots.

Relation caches: 5.10M assignable, 0.61M identity, 0.34M strict-subtype, 0.27M subtype, 0.04M comparable entries
(hit rates 47%, 61%, 36%, 23%, 64%). Of 16.06M keys computed, 16.02M have the simple form
(`'s'`, source id, target id, intersection state) and 43.7K the generic-reference form (`'g'`).

## Track A

### A1. Resolved members of structured types in a record allocated on first write

`StructuredType` (every object, reference, union and intersection type) held Go's five resolved-member fields
inline (48 bytes). They now live in `StructuredMembers`, allocated by the first setter call; the type keeps one
pointer. Getters return the zero values (nil members, empty slices, count 0) while it is absent, exactly what the
unset fields read; after the first write every getter returns exactly what was last set (any write allocates, so
an empty slice that was set is returned as set). 4.9M of 7.6M structured types are never resolved. Call sites
changed mechanically from `.members.get()` / `.members.set(..)` & co. to `members()` / `set_members(..)`.
TypeAlloc<ObjectType> 96 -> 56 bytes, <TypeReference> 120 -> 80, <UnionType> 160 -> 120, <IntersectionType>
136 -> 96; resolved types pay 8 bytes more (pointer + 48-byte record).

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 7.020-7.024 (median 7.021) | 323-325 G |
| single, after | 6.851-6.855 (6.851, -0.17) | 323-326 G |
| 4 checkers, before | 9.39-9.48 (9.410) | 435-439 G |
| 4 checkers, after | 9.13-9.15 (9.151, -0.26) | 435-439 G |
| opt-out single / 4 checkers (go assignment), after | 9.35 / 14.30 | 348 / 523 G |
