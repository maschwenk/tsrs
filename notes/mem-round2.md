# mem-round2: representation changes for peak memory (after mem-layout and mem-lazy)

Same rules as notes/mem-layout.md: only layout / representation changes, no checker semantics. Gates for every
step: the suite (errors and `--baselines types,symbols`) byte-identical to the base commit in the default mode and
with `TSRS_LAZY_MEMBERS=0` (whole `target/test-results` trees compared); opt-out counters 25,973,354 / 9,639,962 /
44,884,281 single and 39,704,001 / 16,200,921 / 89,981,648 with `--checkers 4 --checkerAssignment go`; default
counters unchanged (12,811,032 / 9,630,120 / 44,820,708 single, 16,549,988 / 13,788,912 / 76,888,800 on 4
checkers); Project 0 errors.

Measurement: `/usr/bin/time -l` on the pristine Project checkout, interleaved rounds, "GiB" = peak memory
footprint / 2^30. The machine was shared with other agents (load 8 to 70), so check times are noisy; instructions
retired are the stable CPU-work measure.

## Profile at the start (main at e8d4196, default mode, single)

Arena 6,027 MB requested, heap outside the arena 2,094 MB live, peak 8.13 GiB.

| arena type | MB | count | B/each |
| --- | --- | --- | --- |
| Symbol | 880 | 12,811,033 | 72 |
| NodeAlloc<Identifier> | 419 | 7,844,376 | 56 |
| TypeMapper | 396 | 17,294,526 | 24 |
| TypeAlloc<ObjectType> | 344 | 3,006,176 | 120 |
| [ValueSymbolLinks] chunks | 337 | 10.5M values | 32 |
| TypeAlloc<TypeReference> | 289 | 2,106,919 | 144 |
| str (213 MB source text) | 274 | 3,006,153 | |
| [P<Type>] | 224 | 13,080,622 | 17 |
| TypeAlloc<UnionType> / <IntersectionType> | 187 / 173 | 1.02M / 1.13M | 192 / 160 |
| InferenceContext / InferenceInfo | 175 / 163 | 1.43M / 1.78M | 128 / 96 |
| Signature | 156 | 1,574,006 | 104 |

Heap live by allocating function: `SymbolMap::insert` 493 MB, instantiation `GoMap`s 226, `Relation::set` 190,
lazy member tables (`get_ready_lazy_member_table_worker` + `get_lazy_declared_member` + `resolve_lazy_members`)
271, pointer-keyed `LinkStore`s 104.

## Steps

### 1. `Symbol` 72 -> 64 bytes

`name` and `declarations` are now `OwnedStrCell` / `OwnedSliceCell<P<Node>>` (tsrs_core: an `OwnedCell` of a
`&'static str` / `&'static [T]` stored as a 4-byte-aligned pointer + `u32` length, 12 bytes, like `SliceCell`), so
they pack with the two `u32` flag words. Same `get` / `set`, `get` returns exactly what was set; no call site
changed.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 8.133 | 317-319 G |
| single, after | 8.040 (-0.09) | 315-318 G |
| 4 checkers, before | 10.92-10.97 | 424-429 G |
| 4 checkers, after | 10.79-10.84 (-0.12) | 424-427 G |
