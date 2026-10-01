# mem-recycle: giving provably dead arena memory back

Follow-up to notes/mem-census.md, which measured ~743 MB of the private monorepo's 6.3 GiB single-checker peak as
unreachable at exit. This pass reclaims the classes whose death can be proven locally, verified by the census.

## Mechanism (`tsrs_core::arena`)

bumpalo had no way to give memory back, so `P<T>` now sits on a small arena of our own (per thread, downward bump,
chunks doubling from 1 MiB like bumpalo's; same fast path, instructions unchanged within noise). Two ways back:

- **Free lists.** `tsrs_core::free!(p)` / `free_slice!(s)` put a dead block on a per-thread list for its size class
  (8-byte steps up to 512 bytes, alignment <= 8; other sizes are ignored). `P::new_recycled` /
  `alloc_slice_recycled` take a block of the right class first. Only the recycling sites (flow nodes, mappers,
  inference contexts and infos, candidate lists, recycled type lists) use them, so `P::new` is unchanged. The macros
  pass the block as an address: a `P<T>` argument is a protected `&T` for the call (the first version took `P<T>`;
  in a debug build the write of the free-list link through it was optimized away). Writes go through the chunk's
  exposed provenance.
- **Checkpoints.** `arena_checkpoint()` / `arena_rewind(cp)` move the bump pointer back when a speculative parse is
  rolled back, if the chunk is the same and no free and no `arena_pin()` happened since (`pin` = data allocated in
  the speculation was stored in a structure that is not rolled back).

Ids never depend on addresses (node and symbol ids are counters, flow nodes get ids in the checker, mappers and
contexts have none), so reuse cannot change counters or baselines; nothing orders by address.

Verification modes (no reuse in either):

- `TSRS_ARENA_POISON=1` (any build): freed blocks and rewound ranges are filled with 0xA5; a later read of a pointer
  field crashes, any other read changes output. Debug builds poison free-list blocks too and assert on reuse that
  the poison is intact (`debug_assert`-level check at every recycling allocation).
- The census build (`TSRS_CENSUS=1`, alloc-profile): frees and rewinds are recorded as would-free, never reused, and
  checked at exit (below). Arena chunks are mapped at 0x7c00_0000_0000+ there, zero-sized values get a dangling
  address (a ZST "allocation" returned the start of the previous block), and the stack below the freeing frame is
  scrubbed, all to keep stale words from looking like references.

## The census gate

1. **Precise program walk** (`TSRS_CENSUS_VERIFY=1`, crates/tsrs_cli/src/census.rs): every node (children, parent,
   eager JSDoc), every diagnostic, every symbol declaration and the whole flow graph (flow nodes of every node,
   antecedents, antecedent lists, flow node nodes) is checked against the would-free set. This is exact for the
   parser and binder frees.
2. **Strong mark** (crates/tsrs_core/src/alloc_profile/census.rs `check_would_free`): the conservative mark keeps
   thousands of freed blocks "reachable", because freed objects are exactly the ones whose addresses linger in
   dead stack slots, struct padding (`P::new` copies a value with its padding from the stack: `TypeParameter`,
   `LiteralType`, `MappedType`, `Diagnostic`, the empty `OnceCell<LazyMembers>` of a lazy member table), pooled
   vectors' spare capacity (scrubbed in census builds), empty tail slices pointing at the next block, and bytes of
   text or number pairs. So reachability is recomputed from the roots with the references the program stores: a
   48-bit pointer to a block start (tags on mapper / context words, length bits in mapper slice words, interior
   pointers into heap blocks and 8-aligned into arena lists), x8-encoded symbol entries in heap blocks. An edge into
   a freed block must also come from a block allocated before the free and not be 64 KiB-aligned (a stale pointer
   whose low bytes a small field overwrote). Every freed block reached that way is a violation; the report names
   the referrer, its offset and its neighbouring words (heap referrers by allocating function). On the private
   monorepo the strong mark covers 94% of the conservatively reachable bytes; the rest is lazily parsed JSDoc,
   string slices and similar. Positive control: with the escape barrier removed from `MapperCell::set`, one
   conformance test reports 35 violations (conditional types holding freed mappers).

Both must be 0. `TSRS_CENSUS_ASSERT=1` exits with status 3 otherwise.

## Step 1: parser speculation and binder labels

- Parser: `Parser::mark` takes an arena checkpoint and `rewind` rewinds it (the top-level-await reparse keeps its
  nodes and uses `rewind_keeping_nodes`). Pins: `reparse_list` pushes, `jsdoc_diagnostics` and the scanner's three
  number caches (all outlive a rollback). Everything else the parser keeps is truncated by `rewind` already.
  AST oracle unchanged (113/113 libs, 17,318/17,319 units, the known non-UTF-8 file), also with poison.
- Binder: branch labels of if statements, loops, conditional and logical expressions and optional chains that
  `finishFlowLabel` drops (and labels of expressions without flow effects, which Go discards), with their antecedent
  list cells, unless the label was ever used as an antecedent or a binder field still holds it.

Census (the private monorepo, single and 4 checkers, both lazy modes): 1.39M blocks / 47.3 MB freed or rewound;
precise walk 49.9M references, 0 freed (the strong mark is in step 2's numbers, which include these frees).
Conformance corpus (12,758 files through `tsrs --strict --target esnext`): precise walk 0 in every file.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, base | 6.237-6.241 (6.240) | 300.5-301.8 G |
| single, after | 6.192-6.202 (6.194, -0.046) | 301.1-301.2 G |
| 4 checkers, base | 8.373-8.400 (8.389) | 411.2-412.0 G |
| 4 checkers, after | 8.326-8.350 (8.336, -0.053) | 411.2-412.4 G |

(The arena swap alone: 6.244 / 8.372 GiB median vs base 6.234 / 8.390, instructions -0.4%.)

## Step 2: inference contexts and scratch mappers (the shared escape bit)

`TypeMapper` keeps an escaped bit in bit 2 of its second word (free in every encoding; the inference mapper's
`fixing` flag moved to bit 0) and `InferenceContext` one in bit 0 of its tail pointer. Every place that keeps a
mapper beyond the call that made it goes through a barrier that sets the bit, transitively (children of merged and
composite mappers; the context of an inference mapper, and from an escaped context every mapper it holds; mappers
created later for an escaped context are born escaped):

- `MapperCell` replaces `Cell<Option<P<TypeMapper>>>` in every struct field that held a mapper (`ObjectType`,
  `TypeParameter`, `ConditionalType` x2, `Signature`, the value-symbol links tail, the node builder context), so
  the compiler found every store; plus `ValueSymbolLinks::set_mapper` (erased word), lazy member tables, and the
  context's return / outer return mappers. Mappers are never stored in hash maps or closures (deferred mappers
  capture nodes and lists only); the active-mapper stack is popped before any free.
- An escaped mapper's children are escaped (children are fixed at construction), so the walk stops early.

Recycling sites (each frees only what it made, only when not escaped):

- `chooseOverload`: the candidate's inference context after each candidate (the loop body became
  `choose_overload_candidate`): its own inference mappers, infos, candidate lists (heap buffer and cell), info
  slice and tail. Infos belong to one context (clones copy them; `mergeInferences` takes them from a local list).
- `getConditionalType`: the `infer` contexts and the composite mappers made for them (`scratch_contexts` /
  `scratch_mappers`, recycled when the outermost call of a nesting returns; contexts first, since recycling reads
  their mapper fields). The context's own non-fixing mapper is recycled from the scratch list once
  `setNonFixingMapper` replaced it.
- `getConditionalTypeInstantiation` miss path: the type-argument mapper and its list (`alloc_slice_recycled`); a
  one-type list is unused by the simple mapper and always freed. The distributive `prependTypeMapping` mappers.
- `appendTypeMapping` in `getTypeOfMappedSymbol`, mapped-type index infos, `resolveMappedTypeMembers` key names and
  `getIndexTypeForMappedType`.
- `getInferredType`: the backreference mapper (and its list) and the merged mapper for type parameter defaults.

Census (the private monorepo, all four runs): 15.4M blocks / 351 MB freed single default (16.3M / 364 MB opt-out,
23.6M / 509 MB and 24.5M / 522 MB on 4 checkers); precise walk 0; strong mark 0 violations. Along the way the
census needed: pooled vectors scrubbed of stale entries (`census_scrub_slack`: the inference state pool, the
inference-context and active-mapper stacks, the scratch lists), dangling ZST addresses, and the header / padding
rules above; each of the strong references it reported was traced to one of those (referrer chains, neighbouring
words). Conformance corpus (12,758 files): precise walk 0 and strong mark 0 in every file. Suite trees identical
to the base in all three modes, also with poison; a debug-build suite (free-list poison asserts) passes the same
tests; private-monorepo output and counters identical in default / opt-out, 1 / 4 checkers, also with poison.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, step 1 | 6.194-6.197 (6.195) | 300.5-303.3 G |
| single, after | 5.889-5.894 (5.892, -0.303) | 301.6-303.0 G |
| 4 checkers, step 1 | 8.334-8.343 (8.336) | 410.0-412.4 G |
| 4 checkers, after | 7.876-7.884 (7.881, -0.455) | 412.3-414.1 G |

## Item 3 (measured only): object literal types of overload re-checks

A measurement-only census mode (local patch, not landed: `census_region_begin/end` around each argument's
`checkExpressionWithContextualType` + relation check in `isSignatureApplicable` without error reporting, reported
as would-free) on the private monorepo single: 34.2M blocks / 1,188 MB are allocated inside those regions (this
includes the mappers and contexts that step 2 already recycles). At exit, 2.11M of them are still referenced
directly from outside their region (a lower bound: blocks reached only through other region blocks are not
counted), through: heap tables (union / intersection / instantiation caches, link store tables: symbols 22 MB,
object types 15, references 14, unions 11, intersections 10, literal types 5 MB), value-symbol link chunks
(object types 12 MB, intersections 3.5, unions 2.7, mappers 1.9), signature and mapped-symbol links, and
structured members of cached types. So a region free would need an escape check on every cache insert and every
link store write (dozens of store kinds), not a few barriers; not attempted, as notes/mem-census.md expected.

## Side question: the 450-950 files parsed and dropped

Counted exactly (temporary instrumentation, not landed): 39,772 files parsed, 38,789 in the program, 983 dropped
(13.0 MB of text), the same in every run. All 983 are reachable only from the subtasks of the 17 files that
`getProcessedFiles` deduplicates by package id (`deduplicatePackages`: the same name@version installed at another
path becomes a redirect to the first copy, and the walk does not descend into the duplicate's imports). The
loader parses and resolves every subtask before that walk, so those files are parsed for nothing. Go does exactly
the same (`filesparser.go` `getProcessedFiles`; its comment notes that the duplicate was parsed and acquired
through the host). Not a race and not a tsrs bug; the run-to-run variation in the census came from its
conservative mark. Avoiding the parse would mean deduplicating at load time (a change to Go's algorithm).

## Not done / rejected

- More scratch sites (each < 10 MB in the census after step 2): `inferTypeArguments`' own context (5.5 MB) and
  return-mapper clones (escaped by the conservative return-mapper barrier), `getTailRecursionRoot`'s list and
  mapper (5 + 5 MB), `getInferredTypes` mappers (5.7 MB), `getObjectTypeInstantiation` miss lists (5.8 MB).
- Binder: only branch labels; condition flow nodes that end up unreferenced are not provable locally.
- `KnownSymlinks` path strings, node builder contexts, `ExportCollision` (each < 10 MB).
