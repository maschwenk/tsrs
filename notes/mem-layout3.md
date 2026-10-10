# mem-layout3: what is left in pure layout

Follow-up to notes/mem-layout.md, mem-round2.md, mem-round3.md and mem-small.md. Representation only: no checker
semantics, no change in creation counts. Question: how much of the private monorepo's peak is still pure layout
(padding, 16-byte slices, fields that are almost never set), and what is cheap to take.

Gates for every step: the suite with `--baselines types,symbols` (whole result trees, default, `TSRS_LAZY_MEMBERS=0`
and multi-threaded test programs), fourslash, and the private monorepo's output and `--extendedDiagnostics` counters
identical in default / opt-out x 1 / 4 checkers. Measurement: `/usr/bin/time -l`, interleaved, medians of 3, GiB = peak
memory footprint / 2^30; instructions vary by about +-1% between identical runs on the shared machine.

Status (2026-10-10): sizes here are as of b0602ae. Later work changed several of them: `Symbol` is 32 bytes with
compressed pointers (notes/mem-pointer-compression.md), so "Symbol 40 -> 32 needs a field out" below is done by
another route; `Signature` and `FlowNode` are smaller again with compressed pointers (size assertions in
crates/tsrs_checker/src/types.rs and crates/tsrs_ast/src/flow.rs). The step rationale still describes the code.

## Profile at the start (branch point de3beaf, single checker, default mode)

Arena 4,326 MB requested (5,531 MB on 4 checkers), Rust heap outside the arena 1,570 MB live, peak 5.74 GiB. The
largest rows: `Symbol` 506 MB (13.26M x 40 B), `TypeMapper` 256 MB (16 B, two words), `NodeAlloc<Identifier>` 241 MB,
value-symbol links 239 MB, `[P<Type>]` type lists 218 MB (count is semantics), `TypeReference` 149 MB, `Signature` 136
MB, `StructuredMembers` 126 MB, unions / intersections 114 / 98 MB, `NodeList` 95 MB (4.15M x 24 B).

Occupancy (counted once): unions with any field after `types` set: 103K of 1.07M; intersections 265K of 1.17M;
signatures with a rare tail 3.5%; resolved structured types with index infos 55K of 2.74M.

## Steps

Per step against the previous step's binary (base of step 1 = de3beaf):

| step | single GiB | 4 checkers GiB | instructions single / 4 |
| --- | --- | --- | --- |
| 1. `ThinSlice`: slice cells and node lists in one word | -0.110 (-1.9%) | -0.135 (-1.8%) | +0.4% / +0.4% |
| 2. `Signature`: `noTypePredicate` as a bit | -0.022 (-0.4%) | -0.021 (-0.3%) | +0.1% / 0.0% |
| 3. union / intersection rare fields in a kind-specific tail | -0.073 (-1.3%) | -0.094 (-1.3%) | +0.5% / +0.3% |
| 4. `StructuredMembers` 40 -> 32 | -0.011 (-0.2%) | -0.008 (-0.1%) | noise |
| 5. symbol table entries grow from 1 (superseded by main) | -0.051 (-0.9%) | -0.048 (-0.7%) | +0.4% / +0.1% |
| 6. `FileIncludeReason` boxed rare data (superseded by main) | -0.014 (-0.3%) | -0.017 (-0.2%) | noise |
| 7. `FlowNode` 32 -> 24 | 0.000 | -0.016 (-0.2%) | +0.2% / +0.3% |

### 1. `ThinSlice` (`tsrs_core::ptr`)

A `&'static [T]` in one word: the data pointer in the low 48 bits and the length in the high 16, like `PackedStr`.
A list of 2^16 elements or more (or at an address above 2^48) is stored as a pointer, tagged with bit 0, to an
arena copy of the `&[T]` itself, so `get` returns exactly the slice that was stored in both forms; `T` must be at
least 2-aligned (compile-time check). The word is never zero, so `Option<ThinSlice<T>>` is one word too
(`OptionThinSliceCell`). Used for `NodeList.nodes` (24 -> 16 bytes; `NodeList::new(loc, nodes)`, reads through
`.nodes()`), `Signature`'s two lists, `StructuredMembers`' three lists, union/intersection `types` and
`resolvedProperties`, `TypeReference.resolvedTypeArguments`, `TypeAlias.typeArguments`.

The 12-byte packed `SliceCell` stays where it packs with 4-byte fields for free (`Symbol.declarations`,
`InferenceContext.inferences`). A first version replaced it everywhere, including `Symbol`: no gain there and +1.6% /
+1.2% instructions (single / 4 checkers), from `get_type_arguments` and the other hot readers decoding a length with a
compare-and-branch for the long form. The landed `get` tests bit 0 (one predictable branch, the long form out of
line), and that version measured +0.4%.

### 2. `Signature`: the `noTypePredicate` sentinel as a bit

`resolvedTypePredicate` is nil until resolved, then the checker's `noTypePredicate` sentinel for every signature
without a predicate. The sentinel is bit 0 of the rare-tail word; a real predicate lives in the tail (`SignatureRare`).
`resolved_type_predicate(no_type_predicate)` returns `Some(no_type_predicate)` when the bit is set, and
`set_resolved_type_predicate(p, no_type_predicate)` sets the bit exactly when `p` is the sentinel.

### 3. Unions and intersections: rare fields in a kind-specific tail

Everything after `types` moved into a tail allocated on the first non-nil write: `UnionOrIntersectionRare` (resolved
properties, both property caches) at the start of `UnionRare` (reduced, regular, origin, key property name,
constituent map) or `IntersectionRare` (apparent type, unique-literal instantiation), `repr(C)`. The kind is bit 0 of
the tail word, set when an intersection's data is constructed, so the shared accessors allocate the right tail.
Getters return the zero value without a tail, like the unset Go fields. 112 -> 48 and 88 -> 48 bytes.

### 4. `StructuredMembers` 40 -> 32 bytes

The call-signature count and the index infos share a word: `count << 1 | 1` while no non-empty index-info list was
set, else a pointer to `{index_infos, call_signature_count}`. One difference from "returns exactly what was set": an
empty index-info list set before any non-empty one reads back as `&[]` (Go never compares it by identity).

### 5 and 6 (superseded)

Main's front-end pass (278b139) landed both independently: symbol table buffers growing 1, 2, 4, ... and
`FileIncludeReason`'s rare data behind a `P`.

### 7. `FlowNode`: antecedent and antecedents share a word

Go sets `Antecedent` on every flow node except labels and `Antecedents` only on labels, so they share one word: the
antecedent, or the antecedent list with bit 0 set (`set_antecedents()` asserts the node has no antecedent).

## Census registration

The encodings are registered with `tsrs_core::census_layout` (docs/DEBUGGING.md "census free-gate") through two
`CensusField` kinds: `Thin { off }` (a `ThinSlice` word: a reference only while the length bits are non-zero, or with
bit 0 the long list's `&[T]` record) and `LowTag { off, mask }` (an address with low tag bits). Final head: 0
references to freed blocks and 0 strong-mark violations in all four private-monorepo runs; positive control (escape
barrier removed from `MapperCell::set`) reported 835,737 violations. Conformance corpus (12,758 files through
`tsrs --strict --target esnext`, final census binary, 10 in parallel): precise walk 0 and strong mark 0 in every
file. 9 files did not report 0 on the first run under the parallel load (a missing or non-zero line) and reported 0
on the retry and on three more single runs each.

## Result

Interleaved, medians of 3 (opt-out: 1 run), base = main b0602ae, final = this branch merged with main:

| run | main peak GiB | final peak GiB | main instructions | final instructions |
| --- | --- | --- | --- | --- |
| default, single | 5.474 | 5.272 (-0.203, -3.7%) | 309.0 G | 311.0 G (+0.6%) |
| default, 4 checkers | 7.304 | 7.022 (-0.282, -3.9%) | 422.8 G | 425.3 G (+0.6%) |
| opt-out, single | 7.312 | 7.094 (-0.218, -3.0%) | 332.9 G | 335.2 G (+0.7%) |
| opt-out, 4 checkers (go assignment) | 11.166 | 10.813 (-0.353, -3.2%) | 502.2 G | 505.8 G (+0.7%) |

Against the branch point de3beaf (before main's front-end pass), the same comparison was -0.270 GiB (-4.7%) single
and -0.359 GiB (-4.7%) on 4 checkers; the difference is the two steps main landed independently (5 and 6) and
overlap with main's NodeList-adjacent front-end savings. Cost: ~0.16% instructions per 1% of peak overall; steps 3
(+0.5% single / +0.3% on 4 checkers for 1.3%) and 7 (single-threaded instruction medians 310.3 G vs 310.0 G, i.e.
within noise, for 0-0.2%) are the ones closest to the stop rule.

## Not done, and the floor

Rejected or left after counting:

- `ConditionalType` rare fields (set on 18-35% of 665K): 3-5 MB.
- `TypeParameter` (`target` / `mapper` on 89%), `ObjectType` (`target` / `mapper` on 57%), `IndexedAccessType`:
  nothing rare enough.
- `InferenceInfo` padding (6 bytes): most infos are recycled (mem-recycle), so the live share is small.
- `Signature.resolvedMinArgumentCount` / `minArgumentCount` as `i16`: range checks on every write for ~6 MB.

The floor in pure layout was close. Above 20 MB, what was left was already minimal for what it holds (mappers, type
lists, link records, identifiers, symbol tables) or needed a semantic change: type lists and mappers (~470 MB single)
are created on instantiation-cache misses, so only fewer instantiations shrink them; fewer `[ValueSymbolLinks]`
records needs lazier link creation; a compact AST (32-bit child indices) would be a rewrite of `P<Node>` (later
4-byte handles did this for every pointer, notes/mem-pointer-compression.md). `Symbol` 40 -> 32 was listed here as
needing a field out; pointer compression got there instead (status line above).
