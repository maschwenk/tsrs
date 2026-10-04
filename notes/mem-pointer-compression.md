# mem-pointer-compression: 32-bit arena handles for `P<T>`

Branch `mem/pointer-compression`, base origin/main ff92b7f. Goal: `P<T>` becomes a 32-bit handle (an offset in
8-byte units into one process-wide reserved region), so every arena struct that stores `P<T>` shrinks, `Option<P<T>>`
stays 4 bytes (handle 0 is never handed out), and handles are position independent (the foundation for an mmap-able
persisted front end). Representation only: identical diagnostics, baselines and `--extendedDiagnostics` counters.

## 1. Survey and plan (written before the code)

### How `P<T>` is used today

`P<T>` is `#[repr(transparent)] struct P<T: ?Sized>(&'static T)` (`crates/tsrs_core/src/ptr.rs`): 8 bytes, `Copy`,
`Deref`, eq / hash / order by address, `Option<P<T>>` 8 bytes through the reference niche. No crate uses an unsized
`P<T>` (no `P<str>`, `P<[T]>`, `P<dyn _>`). Conversions to and from raw addresses, by kind:

| kind | sites | what changes |
| --- | --- | --- |
| `P::new`, `P::new_recycled` | everywhere | allocate 8-aligned (handles count 8-byte units), return a handle |
| `P::from_static(&'static T)` on an arena object: `as_p()` of `Node`, `Relater`, `InferenceContext`, `EmitContext`; `TypeAlloc` / `NodeAlloc` header casts; `as_source_file()` / `as_flow_reduce_label_data()` payload views; `&ModifierList.list` | ~25 | address -> handle (checked: inside the region and 8-aligned) |
| `P::from_static` on link-store elements (`tsrs_checker::links` chunks, `tsrs_core::PagedLinkStore` pages) | 4 | element addresses must be 8-aligned: chunks of `Aligned8<V>` in compressed mode |
| `P::from_static` on a **static** (`printer::*_HELPER` emit helpers, also inside `static` initializers; `EMPTY_COMPILER_OPTIONS`) | 15 | not representable: emit helpers become `SP<EmitHelper>` (a static pointer with the same identity semantics), `EMPTY_COMPILER_OPTIONS` a lazily arena-allocated `P` |
| `P::from_static(self)` with `&'static self` that came from `tsrs_core::alloc` (resolution data, transformers, incremental program, `superAccessState`) | 6 | fine if the object is in the arena (`alloc` also aligns to 8); a heap object fails the check (caught by the gates) |
| packed words that keep an address and tag bits: `NodeHeaderWord` (parent / 8 in 45 bits), identifier word (flow node / 8), `SymbolParentWord` (48-bit address + 2 tags), `FlowNode.link` (low tag), `TypeMapper` (two tagged words), `ValueSymbolLinks` (`P<()>` words whose address carries mode bits), `TypeSymbolWord`, `SignatureRareWord`, `CountOrIndexInfos`, `UnionOrIntersectionRareWord`, `InferenceContext.rare` | 11 types | go through `P::to_bits` / `P::from_bits` (below) instead of `expose_provenance` / `with_exposed_provenance` |
| `addr()` | 90 (37 in the arena, 10 in the census) | still the real address: region lookup (`Region::containing`, `enter_owner`), census, debug output |
| hashing by address | `LinkStore` slots (`key: usize`), `FxHashMap<P<_>, _>` everywhere | hash the handle; link slots can key by the 32-bit handle (12 -> 8 bytes) |
| order by address | `impl Ord for P` only; no `BTreeMap<P<_>>`, no `sort_by_key(addr)` | order by handle = order by address inside the region |

Go sorts by id, never by pointer, and the port has no address-keyed sort; any output that depended on address order
would already vary with ASLR. Handles make hashes and orders deterministic across runs, which can only remove
nondeterminism. The gates compare whole result trees, so a hidden dependency would show.

### Arenas

- **Thread arenas** (`tsrs_core::arena`, one per OS thread, leaked): downward bump in chunks that double from 1 MiB,
  taken from `std::alloc` (mimalloc in the binaries). The checker pool's per-checker arenas are these (one thread per
  checker), as are the parse/bind rayon workers'.
- **Regions** (LSP, `tsrs_api`, `tsrs_cli` build orchestrator, auto-import scratch): upward bump in chunks carved from
  per-thread 1 MiB slabs (or a slab of their own above 256 KiB), released to `std::alloc` when every chunk carved from
  a slab is freed. A registry maps chunk ranges to regions by address.
- **Free lists** (mem-recycle): per arena, intrusive lists of dead blocks inside its chunks. Address-internal; no
  change.
- **Census / poison modes**: census builds map chunks with `mmap` at a fixed high hint so 48-bit words that look like
  pointers are recognizable.

Plan: **one region for everything**. At the first chunk request the process reserves 32 GiB + one page of address
space (`mmap(PROT_NONE, MAP_PRIVATE | MAP_ANON | MAP_NORESERVE)`), stores its base in a global, and keeps the first
page unused (so offset 0, handle 0, is never an object: the `Option` niche). Every chunk of every thread arena and
every region slab comes from this reservation through one chunk allocator (a mutex around a bump pointer and a
first-fit free set of released ranges; chunk requests are rare: >= 1 MiB each), committed with
`mprotect(PROT_READ | PROT_WRITE)`. A released slab is decommitted (`mmap(MAP_FIXED, PROT_NONE)` over it, which drops
the pages on both macOS and Linux) and its range goes back to the free set, so the long-lived server reuses address
space instead of exhausting it. Running out of the reservation aborts with a message that names the cap and the
arena total. One `base` serves every thread, arena and region, so a handle is meaningful everywhere, and deref is
`base + (handle << 3)` with `base` a global (one load, an add).

Thread-arena chunks stop doubling at 64 MiB in compressed mode: the reservation counts address space, and a doubled
2 GiB tail chunk of a checker arena would waste most of a gigabyte of it per thread (untouched pages cost no memory,
but they cost reservation).

### Threading

`base` is written once, before the first chunk exists; every `P` that reaches another thread was created after that
and handed over through the synchronization that hands over any data (thread spawn, channels, locks), so a relaxed
load of `base` sees it. The chunk allocator is a mutex (taken per chunk, not per allocation). Nothing else about the
threading contract changes: `P<T>` stays `Send + Sync` by decree.

### The 32 GiB cap

Worst measured: the private monorepo, opt-out mode, 4 checkers, 11.4 GiB peak footprint (arena + heap); the arena
part on 4 checkers in default mode was 5.5 GB requested (mem-layout3), opt-out ~1.5x that. With 4-byte handles the
arena shrinks, and with the 64 MiB chunk cap the reservation in use stays within a few hundred MB of the arena's
requested bytes (one partly used chunk per thread). That leaves ~3x headroom for the CLI. The language server frees
regions and reuses their ranges; its exposure is fragmentation of released ranges (slabs are 1 MiB, large chunks
vary), which the first-fit free set coalesces. If 32 GiB ever gets close, the unit can become 16 bytes (64 GiB) by
changing one shift, at the cost of 16-byte alignment for `P` targets.

### Slices and strings

`&'static [T]` (16 bytes), `ThinSlice` / `SliceCell` / `PackedStr` (8-12 bytes) stay absolute pointers in this
change. The natural follow-up is `PSlice<T>` (32-bit offset + 32-bit length, 8 bytes, position independent) and a
`PStr` on the same model; `[P<Type>]` / `[P<Node>]` elements already halve with this change because the elements
are handles. Done only if time allows.

### Position independence

Handles are offsets, so every `P<T>` field is position independent by construction: `P<T>` has no constructor from
an integer except `P::from_bits` (unsafe), and `to_bits` returns the offset (in pointer mode, the address). The packed
words of the front end (node header parent, identifier flow slot, symbol parent word, flow node link) store
`to_bits`, not addresses. What still stores absolute addresses in front-end arena objects after this change, i.e.
what a persisted front end still needs: slices and strings (above), `SymbolTable` entries (a heap `HashTable`),
`SourceFile`'s heap fields (`String`s, `Vec`s, `OnceLock`s, the JSDoc cache), `&'static` fields to arena objects
that are not `P`, and source text pointers (`PackedStr` into the file text). Checker objects are never persisted;
their packed words move to handles only where that shrinks them (`TypeMapper`).

### Feature and comparability

Cargo feature `tsrs_core/compressed-ptrs`, off by default until the gates pass, so the old representation stays
buildable for comparison. Downstream crates read `tsrs_core::COMPRESSED_PTRS` (a `const bool`) for size assertions,
so they need no features of their own. The API that is common to both modes:

- `P::new`, `P::new_recycled`, `Deref`, `get`, `addr` (the real address), `ptr_eq`, `Eq` / `Hash` / `Ord`.
- `P::from_static(&'static T)`: checked in compressed mode (inside the region, 8-aligned), else a cast. Not `const`
  in compressed mode.
- `P::to_bits(self) -> usize` / `unsafe P::from_bits(usize) -> P<T>`: the offset from the base (compressed) or the
  address (pointers); 8-aligned and below 2^48 either way, so the packed words keep their bit budgets and tag bits.
  `P::handle(self) -> u32` exists only in compressed mode, for the (u32 handle, u32 metadata) rewrites.
- `SP<T>`: a pointer to a `static`, with `P`'s identity semantics (for the emit helpers).

The census (alloc-profile + `TSRS_CENSUS=1`) decodes 48-bit words as pointers; in compressed mode it would need to
decode 32-bit handles, which is a separate project. Compressed builds refuse `TSRS_CENSUS=1` with a message; the
allocation profile itself (per-type bytes) works in both modes.

### Steps

1. This note.
2. `tsrs_core`: the reservation and chunk allocator, `P` as a handle behind the feature, `to_bits` / `from_bits`,
   `SP`, `Aligned8` link chunks, 8-aligned `alloc`; the front-end packed words through `to_bits`.
3. Workspace compiles in both modes; gates in compressed mode against origin/main.
4. Measure (1 / 4 checkers, `--noCheck`, interleaved, median of 3), alloc-profile per-type sizes before / after,
   the LSP driver; then flip the default if the gates pass.
