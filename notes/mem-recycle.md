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
precise walk 49.9M references, 0 freed; strong mark 0 (re-run with the final census). Conformance corpus (12,758
files through `tsrs --strict --target esnext`): precise walk 0 in every file.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, base | 6.237-6.241 (6.240) | 300.5-301.8 G |
| single, after | 6.192-6.202 (6.194, -0.046) | 301.1-301.2 G |
| 4 checkers, base | 8.373-8.400 (8.389) | 411.2-412.0 G |
| 4 checkers, after | 8.326-8.350 (8.336, -0.053) | 411.2-412.4 G |

(The arena swap alone: 6.244 / 8.372 GiB median vs base 6.234 / 8.390, instructions -0.4%.)
