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

### A2. Relation cache keys: type-id pairs packed into 8-byte slots

Go keys the relation caches by the 128-bit xxh3 hash of the `getRelationKey` bytes. The simple form
(`'s'`, source id, target id, intersection state; 99.7% of keys) is a triple of small numbers, so
`get_relation_key` now returns `RelationKey::Pair` (source id, target id below 2^28, state below 2^2, packed into
58 bits; an injective mapping) without building and hashing the key bytes, and `RelationKey::Hashed` (the same
128-bit hash as before) for everything else. `Relation` keeps the pairs in a `hashbrown::HashTable<u64>` whose slot
is `key << 6 | result bits` (8 bytes + 1 control byte per slot instead of 17 + 1) and the hashed keys in the
previous map; `size()` counts both. The relater's maybe-key stack and set hold `RelationKey`s. The forms never
alias each other: Go's `'s'` and `'g'` byte strings differ, and a simple key is `Pair` exactly when it fits.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A1) | 6.852-6.858 (6.852) | 321-323 G |
| single, after | 6.766-6.770 (6.768, -0.08) | 320-321 G |
| 4 checkers, before (A1) | 9.13-9.18 (9.148) | 435-436 G |
| 4 checkers, after | 8.965-8.991 (8.971, -0.18) | 432-434 G (-0.8%) |
| opt-out single / 4 checkers (go assignment), after | 9.26 / 14.12 | 344 / 515 G (-1.2%, -1.6%) |

### A3. AST node header 32 -> 24 bytes

23.0M nodes on Project (the AST is shared by all checkers). The header held the kind (`u16`), the data tag (`u8`),
flags, the range, the parent pointer and a 64-bit id, 32 bytes with padding. Now the kind (9 bits), the data tag
(8 bits) and the parent (address / 8 in 45 bits: nodes are 8-aligned and user-space addresses are below 2^48;
asserted when a parent is set) share one word, and the id is stored in 32 bits like symbol ids (a 64-bit counter,
`NodeId` stays `u64`, panic past `u32::MAX`). The parent's provenance is exposed on store and recovered with
`with_exposed_provenance`. Only the parent part is written after creation, under the same owner-only contract as
before (`OwnedCell`). `node.kind` became `node.kind()` and `node.parent.get()` / `.set(..)` `parent()` /
`set_parent(..)` (mechanical, 1,950 sites; `tools/gen-ast/gen-ast.ts` emits the new forms, `generated.rs`
regenerated). AST oracle: 113/113 libs and 17,318/17,319 test units identical (the one is the known non-UTF-8
file), as before.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A2) | 6.759-6.763 (6.760) | 315-316 G |
| single, after | 6.587-6.592 (6.592, -0.17) | 317-318 G (+0.5%) |
| 4 checkers, before (A2) | 8.96-9.01 (8.966) | 429-431 G |
| 4 checkers, after | 8.79-8.82 (8.812, -0.15) | 431-432 G (+0.4%) |
| opt-out single / 4 checkers (go assignment), after | 9.08 / 13.95 | 339 / 517 G |

### A4. Symbol table entries 16 -> 8 bytes

Counted once (capacity deltas of every `SymbolMap`): 465 MB of entry capacity and 74 MB of hash index on Project
single, 19.7M inserts. An entry was (symbol pointer, 32-bit key hash, 32-bit key length). It is now one word: the
symbol's address / 8 in 45 bits (same platform assumption and provenance handling as A3), an odd-key flag, the
key length capped at 63 (6 bits) and 12 bits of the key hash. Linear scans (tables up to 16 entries) still compare
the length first and hash the probe only when a length matches, then the 12 hash bits, then the text; the hash
index (larger tables) recomputes an entry's full hash from its key when it rehashes (on growth, and when it is
built at 17 entries) or removes. Same order, same `set`-keeps-key and `delete` behavior; a unit test covers keys
longer than 63 bytes in linear and indexed tables.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A3) | 6.589-6.594 (6.589) | 318-322 G |
| single, after | 6.372-6.375 (6.373, -0.22) | 319-322 G (+0.3%) |
| 4 checkers, before (A3) | 8.79-8.82 (8.818) | 432-436 G |
| 4 checkers, after | 8.53-8.57 (8.551, -0.27) | 433-434 G (+0.2%) |
| opt-out single, before / after | 9.08 / 8.51 (-0.58) | 340 / 340 G |
| opt-out 4 checkers (go assignment), after | 13.09 (was 13.95) | 524 G |
