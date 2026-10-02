# mem-shared-base: what the extra checkers duplicate, and whether any of it can be shared

Experiment 4 of the checker-memory round (2026-10-02). On the private monorepo one checker peaks at 5.73 GiB and
the default four at 7.56 GiB (branch point de3beaf, `/usr/bin/time -l`). Each checker has its own arena, type and
symbol tables and caches (docs/PORTING.md "Threading"); Go has the same per-checker duplication. This note
measures what the extra 1.8 GiB is, evaluates three ways to reduce it, and lands one exact representation change
behind a switch.

Result in one paragraph: about 1.0 GB of the 4-checker excess is objects that another checker also built,
structurally identical (types 0.58 GB, symbols 0.27, signatures 0.08, relation entries 0.04, link records on
shared keys 0.04); 0.18 GB is page-table density of the two id-keyed link stores; the rest is per-checker
scratch and hash tables. Only 36% of the duplicated bytes are built purely from lib and node_modules
declarations; the rest involve project declarations (library generics instantiated with project types). Sharing
types across checkers cannot be made exact; the type-free binder facts that could be shared are 21 MB; a better
file assignment saves at most ~1% of the duplicates. The one exact win is in the id-keyed link stores:
`TSRS_SPARSE_ID_PAGES=1` stores sparse pages as a bitmap plus slots, -0.14 GiB (-1.8%) peak on 4 checkers and
-0.40 GiB (-4.2%) on 8, +0.45% / +1.2% instructions, nothing else changes. Default off.

## 1. The measurement: a cross-checker duplication census

`cargo build --release -p tsrs_cli --features assignment-stats`, run with `TSRS_ASSIGNMENT_STATS=dup` and
`--extendedDiagnostics` (crates/tsrs_checker/src/dupstats.rs, crates/tsrs_compiler/src/checkerpool_dupstats.rs;
compiled out otherwise). After checking, every type, checker-created symbol, signature and relation-cache entry of
every checker gets a checker-independent fingerprint (xxh3-128 over a key):

- Types: the data Go's `CompareTypes` orders types by (utilities.go:414): type flags, the object flags that are set
  at creation (not the lazily computed ones), the alias symbol and alias type arguments, and per kind: literal
  value and freshness; the declared type's symbol for classes/interfaces; element flags and labels for tuple
  targets; target + type arguments for references (node + mapper for deferred ones); symbol + target + mapper for
  anonymous, instantiation-expression and mapped types (the mapped type's fresh type-parameter mapping skipped as
  `CompareTypes` does); origin or the sorted constituent fingerprints for unions; the constituent list for
  intersections; symbol + target + mapper for type parameters; the components for index, indexed access,
  template literal, string mapping, substitution; root node + mapper for conditional types.
- Symbols as components: by first declaration (an AST node, shared by all checkers) and name, like
  `compareSymbols`; without declarations by name and flags.
- Checker-created symbols as objects: instantiated symbols by target + mapper, mapped-type members by mapped type
  + key type, union/intersection properties by containing type, reverse-mapped by their three types, others
  (object-literal members, ...) by declaration. Signatures: target + mapper, composite members, or declaration.
- Mappers: kind + source/target fingerprints; function mappers by function address; inference and deferred mappers
  by address (marked non-canonical, see below). Relation entries: relation, intersection state, result and the
  two types' fingerprints (the 0.3% hashed keys are counted as unique).
- Everything created while the checker initialized (same code over the same program in every checker) is
  identified by its creation index.

Bytes per object are estimated from the allocation sizes (type header + payload, resolved members record and its
slices and member table, type-argument / constituent lists, alias record, the mapper it holds; symbols 40 B +
value-links record + tails; signatures 88 B + slices; relation slots 10 B). Duplication of a fingerprint present
`c_i` times in checker `i` is `sum(c_i) - max(c_i)` copies: what perfect sharing would save, the ceiling.

Validation: summed over the 4 checkers the census sees 14,605,655 types (`--extendedDiagnostics`: 14,606,003; the
difference is initialization types counted once). The union of distinct fingerprints over the 4 checkers is
25.17M objects; one checker creates 27.51M objects with 24.98M distinct fingerprints. So the fingerprint
identifies "the same object" well: four checkers together know about as many distinct objects as one checker.
Two caveats are reported separately: fingerprints that occur more than once inside one checker ("collide",
e.g. object literal types re-created per contextual check, fresh type parameters; 131 MB of the duplicate bytes)
and fingerprints that contain an inference/deferred mapper or a cycle ("non-canonical", 175 MB).

The link stores keyed by shared objects (binder symbols, AST nodes, ids) are compared by key; the id-keyed stores
also report the pages each checker allocates.

### The private monorepo, 4 checkers (default locality assignment)

Peak 7.56 GiB vs 5.73 GiB single. Alloc profile (`--features alloc-profile`): arena requested 4,326 -> 5,531 MB
(+1,205 MB), heap outside the arena 1,596 -> 2,317 MB (+721 MB). The arena rows that grow most: `Symbol` +155 MB,
`TypeMapper` +145, `[P<Type>]` +128, `[ValueSymbolLinks]` chunks +98, references +73, signatures +61, unions
+54, intersections +53, anonymous object types +49, conditional types +42, `StructuredMembers` +40, inference
contexts and infos +63.

Census: 41.09M objects / 3,042 MB summed over the checkers (single checker: 27.51M / 2,091 MB).

| kind | objects (sum) | MB (sum) | duplicated copies | dup MB |
| --- | --- | --- | --- | --- |
| reference types | 3.23M | 405 | 1.07M | 138.8 |
| anonymous object types | 4.21M | 534 | 1.07M | 126.1 |
| union types | 1.57M | 240 | 0.54M | 84.5 |
| intersection types | 1.79M | 195 | 0.62M | 66.9 |
| conditional types | 1.06M | 174 | 0.39M | 65.5 |
| type parameters | 0.93M | 84 | 0.39M | 34.6 |
| mapped types | 0.42M | 97 | 0.14M | 33.9 |
| literal types | 0.86M | 52 | 0.25M | 15.3 |
| other types (class/interface, indexed access, index, tuple targets, ...) | 0.52M | 44 | 0.19M | 16.5 |
| **all types** | **14.61M** | **1,825** | **4.73M** | **582.1** |
| instantiated symbols | 7.40M | 457 | 2.45M | 150.5 |
| other symbols (object literal members, ...) | 2.63M | 178 | 0.56M | 45.5 |
| mapped-type members | 1.73M | 106 | 0.62M | 37.7 |
| union/intersection properties | 1.60M | 99 | 0.51M | 31.6 |
| **all checker symbols** | **13.37M** | **840** | **4.14M** | **265.5** |
| signatures | 2.35M | 274 | 0.72M | 83.6 |
| relation-cache entries | 10.75M | 103 | 4.17M | 39.8 |
| initialization objects | | | | 0.2 |
| **total** | **41.09M** | **3,042** | **13.70M** | **971.1** |

Duplicated bytes by origin (MB):

| | none | lib | node_modules | workspace packages | project |
| --- | --- | --- | --- | --- | --- |
| most specific declaration the fingerprint involves | 44.7 | 28.7 | 277.9 | 18.7 | 601.1 |
| file that declares the object's own (alias) symbol | 251.0 | 111.8 | 488.5 | 12.3 | 107.4 |

So 351 MB (36%) of the duplicated bytes are built only from lib and node_modules declarations (plus intrinsics
and literals); 620 MB involve a project or workspace declaration somewhere. Read by declaring file instead, 61% are
library-declared objects (zod, MikroORM, lib members, ...) but most of those are instantiated with project types
(`ZodObject<{ ...project shape }>`, entity references), which is why the first row puts them under "project".

Duplicated bytes by the number of checkers that hold the fingerprint: 2 checkers 209 MB, 3 checkers 149 MB,
**all 4: 613 MB (63%)**. With 16 checkers the duplicates total 3,061 MB and 867 MB of them are held by all 16
(58 MB per checker that every group of ~1,700 files builds again), 2,021 MB (66%) by 11 or more of the 16.

Link records whose key is a shared object, duplicated across checkers (MB, `sum - distinct keys` records x
record size incl. slot): type-node links 10.5, value links of binder symbols 6.4, alias links 5.5, resolved
export tables (entries) 5.0, signature links 3.9, symbol-node links 3.6, module links 3.1, symbol-reference links
1.9, members/exports links 1.4 (+0.2 tables), declared-type links 1.1, others < 1; total 44.3 MB.

Id-keyed link stores (`value_symbol_links` by symbol id, `symbol_node_links` by node id) allocate 4-byte slots
for whole pages of 1,024 ids. Ids come from process-wide counters, so with several checkers each checker's ids
are interleaved with the others' at a granularity finer than 16 ids, and every checker allocates nearly every page:
pages 190.7 + 47.6 MB on 4 checkers (43.7 + 15.9 MB single), 912 + 113 MB on 16. Smaller pages do not help
(16-id pages: 118 + 28 MB on 4 checkers).

With Go's FENNEL assignment (`TSRS_CHECKER_ASSIGNMENT=go`) the duplicates are 1,413 MB (914 MB held by all 4);
the locality assignment already removed 442 MB of them.

### Where the 1.8 GiB goes

| | MB |
| --- | --- |
| peak difference (7.56 - 5.73 GiB, in MiB) | ~1,880 |
| structurally identical types, symbols, signatures, relation entries (census, estimated bytes) | 971 |
| link records and tables on shared keys | 44 |
| id-page density of the two id-keyed stores | 179 |
| rest: per-checker scratch (inference contexts and mappers, rolled-back object literal types), hash tables whose entries follow the duplicated objects (union / intersection / instantiation caches, member-table entries, lazy member tables), allocator retention | ~690 |

The census is a ceiling for sharing: the duplicated objects would have to be shared with identical content.

## 2. Design options

### (a) A shared, immutable declared-shape layer for binder-level facts

Candidates: what each checker computes into its own links from binder data only, without type ids: alias
targets, resolved export tables, symbol resolution of nodes, member/export tables, const-enum values, global
merges. Measured ceiling (duplicated records on shared keys, 4 checkers): alias links 5.5 + resolved export
tables 5.0 + symbol-node links 3.6 + module links 3.1 + symbol-reference links 1.9 + members/exports links and
tables 1.6 + merged symbols 0.2 + enum values 0.2 + node links 0.4 = **21.5 MB (0.3% of the peak)**. Even that is
not all shareable: alias targets and export tables can be checker-created merged symbols (global and module
augmentations are merged per checker, and a shared table must not hand checker B a symbol whose links checker A
owns); `AliasSymbolLinks.referenced` / `type_only_declaration` and the reference kinds are per-checker semantics
(unused-import and emit decisions) and resolution reports errors at the use site. A shared layer would add a
synchronized publish path (a `OnceLock` per key or sharded tables) on the name-resolution hot path and a new
invariant (the layer holds only binder objects) for at most ~20 MB. Not worth building; not prototyped.

### (b) Sharing the types of a base layer (lib.d.ts + node_modules)

Ceiling: the 351 MB of duplicates whose fingerprint involves only lib / node_modules declarations (4.4% of the
4-checker peak), assuming every such type, member, signature and relation entry could be shared with identical
content. Why it cannot be made exact:

- Types are not immutable after creation. Members, base types, declared members, base constraints, apparent,
  widened and regular types, union property caches and the instantiation tables of generic targets
  (`InterfaceType.instantiations`, `object_type_instantiations`, conditional roots) are filled lazily by whichever
  code asks first. A shared type would need synchronized mutation of all of them, or a frozen, fully resolved base
  layer.
- A frozen, pre-resolved layer means eager resolution: lib + node_modules declarations resolved in full, which the
  lazy-member work showed is mostly never needed (83% of declared members of instantiated references are never
  instantiated, notes/lazy-members.md), so the layer would cost more than the duplicates it removes. And eager
  resolution changes the order in which types are created.
- Type ids are per checker, and ids are observable: `CompareTypes` (union constituent order) falls back to ids
  when all other data ties (fresh vs regular literals, object types with the same symbol and no mapper); relation,
  union, intersection and instantiation caches are keyed by id lists; Go panics on comparing types of different
  checkers. A shared id space would make the ids of shared types depend on which checker resolved them first,
  i.e. on thread scheduling.
- Resolution depends on checker state: circularity detection uses the resolving checker's resolution stack;
  instantiation depth limits and "excessively deep" errors are reported at the resolving checker's `currentNode`;
  diagnostics produced during lazy resolution would be produced once instead of in every checker that needs them.
- The layer is not closed under use: instantiating a base generic with a project type argument creates a
  project-level type whose target's caches would have to point from shared into checker-local memory.
- Relation entries between base types (5.7 MB of the duplicates) depend on the relation's checker-local state
  (maybe-stacks, intersection state), so even with shared types they would stay per checker.

Verdict: infeasible exactly, and the ceiling (4.4%) does not justify an inexact attempt.

### (c) Reducing duplication without sharing

Assignment. 63% of the duplicated bytes are held by all four checkers; with 16 checkers 66% are held by 11 or more,
so most duplicates are needed by files everywhere. To bound what an assignment could still do, the 16-checker run
recorded the set of checkers holding each fingerprint (`TSRS_DUP_MASKS`), and every partition of its 16 locality
groups into 4 groups of 4 (2,627,625 of them) was scored with "copies = groups touched - 1" per fingerprint.
Model: best 860 MB, median 988, worst 1,069 MB of duplicates. Running the predicted best and worst for real
(`TSRS_CHECKER_ASSIGNMENT=file:`): best 960 MB of duplicates (default locality 971 MB), worst 1,188 MB; peak best
7.56 GiB vs locality 7.61 GiB (-0.05 GiB, -0.7%; medians of 3 in one later round), instructions -1.6%. The
model ranks partitions correctly but the best one is only 1% fewer duplicates than the directory-locality
default: assignment has been exhausted (consistent with notes/mem-assignment.md, where edge-cut refinement gained
nothing). An assignment derived from a previous run's census, like the cost cache, could buy ~0.7% peak; not
pursued.

Id pages. The 179 MB of id-page density is a pure multi-checker artifact: no object is duplicated, the pages are
just sparse. This one has an exact fix (below).

Two-phase schemes (one checker resolves the common core first) do not help without sharing: the later checkers
still build their own copies.

## 3. Prototype: sparse id pages (`TSRS_SPARSE_ID_PAGES=1`, default off)

crates/tsrs_checker/src/links.rs. With the switch and a pool of more than one checker (`set_multiple_checkers`,
called by `checkerpool::create_checkers`), an `IdLinkStore` page starts sparse: a 1,024-bit bitmap of the ids
present, the number of set bits before each 64-bit word, and the slots of the present ids in id order
(`SparseIdPage`, 160 B + 4 B per entry, growing by a quarter). A lookup is a bit test plus a popcount; an insert
shifts the slots after it. At 768 entries a page converts to the dense form. Slots, their first-access order,
the values and the ids are unchanged: only the structure that maps an id to its slot differs, so nothing the
checker computes can change. Single-checker pools and the language server never create sparse pages.

Measurements (the private monorepo, base binary vs candidate with the switch, interleaved, medians of 3):

| run | base peak GiB | candidate peak GiB | base instructions | candidate instructions |
| --- | --- | --- | --- | --- |
| 4 checkers | 7.564 | 7.428 (-0.137, -1.8%) | 422.1 G | 424.0 G (+0.45%; pairwise +1.1, +1.4, +4.5 G) |
| 1 checker | 5.726 | 5.732 (noise; no sparse page is created) | 309.7 G | 310.4 G (noise) |
| 8 checkers (same binary, switch off / on) | 9.446 | 9.046 (-0.40, -4.2%) | 530.2 G | 536.4 G (+1.2%) |
| opt-out, 4 checkers, go assignment (1 run) | 11.368 | 11.085 (-0.28, -2.5%) | 505.5 G | 512.0 G (+1.3%) |

The enum dispatch alone (every page dense behind the new enum) costs nothing measurable (420.5-420.7 G vs
419.0-421.6 G); the instructions are the rank computation and the shifting inserts. The saving grows with the
checker count because every checker pays 4 bytes per id of the whole id space with dense pages.

Gates (all with the switch on):

- Conformance `tsrs-test run --suite all --baselines types,symbols`, default and `TSRS_LAZY_MEMBERS=0`,
  single-threaded and `TS_TEST_PROGRAM_SINGLE_THREADED=false` (where every test program has 4 checkers, so sparse
  pages are used): whole `target/test-results` trees identical to the base binary except the `ms` timing fields
  of summary.json (13,458 error baselines, 12,779 `.types`, 12,779 `.symbols` pass).
- Fourslash: 4,066 pass / 63 fail, result trees identical to the base binary.
- The private monorepo: output and `--extendedDiagnostics` counters identical to the base binary with 1 and 4
  checkers (17,314,530 symbols / 14,606,003 types / 80,372,489 instantiations on 4), and opt-out with 4 checkers
  and the go assignment (40,847,730 / 16,681,463 / 91,689,770); diagnostics identical between 1 and 4 checkers;
  `TSRS_ARENA_POISON=1` identical. No arena memory is freed or reused (a page converted to dense drops a heap
  box), so the census free-gate does not apply.
- `cargo test -p tsrs_checker links::tests`: sparse and dense stores map every id of a scrambled insert order to
  the same slot before and after densification (a positive control that skips the per-word count update fails
  it).
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked` clean (also `-p tsrs_cli --features
  assignment-stats`).

## Complexity / risk

- Census: ~700 lines behind the `assignment-stats` feature (dupstats.rs, checkerpool_dupstats.rs, small
  accessors); nothing in normal builds. The fingerprints mirror `CompareTypes`; a new type kind or a new field
  that is part of a type's identity would need to be added there to keep the census honest.
- Sparse pages: ~150 lines in links.rs, one call in the pool. Invariant: `slots[rank(i)]` is the slot of id `i`,
  with `before` kept equal to the popcount of the preceding words (debug-asserted on insert, unit-tested). A mode
  flag set per pool creation is process-global; two pools created concurrently with different checker counts
  (the test runner) may get either form, which is harmless since both map ids identically.
- Interactions: with the layout experiment, if it changes `IdLinkStore` (both edit the same struct; the sparse
  form is orthogonal to the value layout); overload-cache rollback and use-census laziness reduce object counts,
  which shrinks the census numbers but not the id-page density; front-end changes do not touch it. A future change
  that reserved id blocks per checker would make pages dense again but changes id values, which are observable
  (unique-symbol names, node-builder length accounting); the sparse form avoids that question.

## Verdict

Sharing across checkers is not worth pursuing in tsrs: the exact candidate (binder facts) is worth ~20 MB, the
large candidate (base-layer types) cannot be exact and is worth at most 4.4% even if it could, and the assignment
is already within ~1% of the best partition of its groups. Sparse id pages are the one exact, cheap,
self-contained win the measurement turned up: -1.8% peak on 4 checkers, -4.2% on 8, for +0.45% / +1.2%
instructions. Turning them on by default for multi-checker pools is reasonable if the instruction cost is
acceptable; the switch is off on this branch.
