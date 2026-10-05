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

## 2. What was built

| piece | where |
| --- | --- |
| the reservation: 32 GiB at the fixed address 0x4000_0000_0000 (`mmap` `PROT_NONE` + `MAP_NORESERVE`, then `mprotect` per chunk), a mutex-guarded chunk allocator (bump + first-fit free set, coalescing), decommit by mapping `PROT_NONE` over a released chunk, abort with a message on exhaustion or when the address is taken | `crates/tsrs_core/src/reserve.rs` |
| `P<T>` as `NonZeroU32` + `PhantomData`; `P::new` allocates 8-aligned and padded to 8 (`p_layout`, also used by `alloc` and `free!`); `from_static` checked (inside the range, 8-aligned), `from_arena` unchecked for hot views; `to_bits` / `from_bits` (offset; address in plain mode), `key` / `from_key` (`PKey`: handle or address), `cast`; eq / hash / order by handle | `crates/tsrs_core/src/ptr.rs` |
| every thread-arena chunk and region slab from the reservation; thread chunks stop doubling at 64 MiB | `crates/tsrs_core/src/arena.rs` |
| `SP<T>` (static pointer, identity semantics) for the emit helpers; `empty_compiler_options()` instead of `P::from_static(&EMPTY_COMPILER_OPTIONS)`; `PSlot<V>` (8-aligned link-store elements) | `tsrs_core`, `tsrs_printer`, `tsrs_transformers`, `tsrs_ls`, `tsrs_checker::links` |
| cfg: `compressed_ptrs` from `build.rs` = default feature `compressed-ptrs` on a unix target and not `plain-ptrs`; downstream crates read `tsrs_core::COMPRESSED_PTRS` | `crates/tsrs_core/build.rs`, `Cargo.toml` |

Packed words, rewritten to `to_bits` / `key` (both modes) so none stores an address of a `P` target any more:
`NodeHeaderWord` (parent bits / 8 in 45 bits), the identifier word (flow node), `FlowNode.link`, symbol table
entries, the symbol's parent, `TypeSymbolWord`, `ValueSymbolLinks`, `TypeMapper`, `InferenceContext.rare`. The
layouts that now shrink with compressed handles:

- `Symbol` 40 -> 32 bytes: the two tag bits (tail present, value declaration is the first declaration) moved from
  the parent word into the top of the name word (`OwnedTaggedStrCell`: address, 14-bit length, 2 tags; longer names
  go out of line like `PackedStr`'s long form), the parent field is a `PKey`, and `declarations` is an
  `OwnedPSliceCell` (a `ThinSlice` in compressed mode, the 12-byte `SliceCell` otherwise).
- `FlowNode` 24 -> 16: no tag bit at all; a node with `Label` flags holds antecedents, any other node its antecedent
  (Go gives antecedents only to labels and creates labels without an antecedent; asserted on write).
- `ValueSymbolLinks` 24 -> 16 (two handles and a bits word with the mode); `LinkSlot` 12 -> 8 (keyed by handle).
- `TypeMapper` stays 16: its two words now hold `to_bits` with the kind in the low 3 bits (no address round trip on
  decode); lists of `P<Type>` are only 4-aligned now, so list words store the address shifted left by one. Two
  handles in 8 bytes would leave no room for the kind and the escape bit.
- Everything else shrinks by itself: every `P` / `Option<P>` field and every `[P<_>]` element is 4 bytes.

Position independence: a `P<T>` and its `to_bits` / `key` are offsets from the reservation base in compressed mode,
and the only way to build a `P` from an integer is `from_bits` / `from_key` (unsafe, offset-based). The rule that
packed words store `to_bits` / `key` and never `addr()` is in docs/PORTING.md. Not position independent yet (what a
persisted front end still needs): slices and strings (`&'static [T]`, `ThinSlice`, `SliceCell`, `PackedStr`, the
symbol name word), `SymbolTable` entries (heap), `SourceFile`'s heap fields, `&'static` references to arena objects
that are not `P`, and the checker-only raw-pointer words (`CountOrIndexInfos`, `UnionOrIntersectionRareWord`,
`SignatureRareWord`).

## 3. Gates

All against origin/main ff92b7f built in this worktree, in both modes (compressed default and
`--features tsrs_core/plain-ptrs`):

- suite `--baselines types,symbols`, default and `TSRS_LAZY_MEMBERS=0`: 13,458 error baselines pass (2 codes, 2
  fail, as on main), 12,779 / 12,779 types / symbols; `target/test-results` trees identical, `summary.json`
  identical apart from timings.
- suite `--baselines js,jsmap,sourcemap`: 13,392 / 149 / 156 pass; trees identical.
- fourslash: 4,066 pass / 63 fail / 417 skip; result trees identical.
- `cargo test -p tsrs_cli` (tsctests, api tests, default_emit) green; `memory_tests` cannot link on macOS on main
  either (glibc `malloc_trim`) and were skipped. `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked`
  clean in both modes.
- The 38k-file codebase: diagnostics and `--extendedDiagnostics` counters (Files, Lines, Identifiers, Symbols, Types,
  Instantiations) identical in every run below, including opt-out (`TSRS_LAZY_MEMBERS=0`) on 4 checkers with
  `--checkerAssignment go`.

## 4. Results

First round (the PR's first version; section 5 has the final numbers). The 38k-file codebase, `--noEmit
--incremental false`, base = origin/main, interleaved, medians of 3, peak = peak memory footprint (`/usr/bin/time -l`):

| run | base peak GiB | compressed peak GiB | delta | base instructions | compressed instructions |
| --- | --- | --- | --- | --- | --- |
| 1 checker | 5.041 | 4.294 | -0.747 (-14.8%) | 298.0 G | 318.0 G (+6.7%) |
| 4 checkers | 6.670 | 5.733 | -0.936 (-14.0%) | 405.2 G | 431.5 G (+6.5%) |
| `--noCheck` | 1.822 | 1.643 | -0.179 (-9.8%) | 54.4 G | 52.4 G (noise: 51-62 G both) |
| opt-out, 4 checkers, go assignment (1 run) | 10.468 | 8.834 | -1.634 (-15.6%) | 492.0 G | 528.8 G (+7.5%) |

Wall time: only the first round ran on a quiet machine (load 6-7): 1 checker 17.66 -> 17.16 s, 4 checkers 7.38 ->
7.14 s, `--noCheck` 0.79 -> 0.80 s. A second set of 5 interleaved rounds (final binary) ran at load 19-26 (other
agents): 1 checker 23.77 -> 25.17 s (+5.9%), 4 checkers 10.96 -> 10.54 s (-3.8%), peaks and instructions as above
(-14.9% / -14.1%, +6.4% / +6.7%). Read: the single-threaded run pays for the instructions, four checkers gain from
the smaller working set; a quiet-machine rerun is still owed before claiming either.

Arena by type (alloc-profile build, 1 checker; arena requested 3,866 -> 3,080 MB, -20%; chunk total 6,317 -> 3,437
MB because thread chunks stop doubling at 64 MiB). Rows above 29 MB in the base:

| type | count | B/each before | after | MB before | after |
| --- | --- | --- | --- | --- | --- |
| `Symbol` | 12.81M | 40 | 32 | 489 | 391 |
| `TypeMapper` | 16.16M | 16 | 16 | 247 | 247 |
| `[ValueSymbolLinks]` (chunks) | 2,463 | 24/value | 16/value | 231 | 154 |
| `NodeAlloc<Identifier>` | 7.56M | 32 | 32 | 231 | 231 |
| `[P<Type>]` | 11.91M | 18 avg | 9 avg | 211 | 105 |
| `TypeAlloc<ObjectType>` | 3.01M | 48 | 40 | 138 | 115 |
| `TypeAlloc<TypeReference>` | 2.11M | 64 | 48 | 129 | 96 |
| `Signature` | 1.57M | 72 | 56 | 108 | 84 |
| `[P<Node>]` | 8.76M | 12 avg | 6 avg | 103 | 51 |
| `[P<Symbol>]` | 2.71M | 36 avg | 18 avg | 95 | 47 |
| `InferenceContext` | 1.43M | 64 | 48 | 87 | 66 |
| `InferenceInfo` | 1.78M | 48 | 32 | 81 | 54 |
| `StructuredMembers` | 2.64M | 32 | 32 | 80 | 80 |
| `SymbolTable` | 3.49M | 24 | 24 | 80 | 80 |
| `NodeAlloc<PropertyAssignment>` | 0.99M | 72 | 48 | 68 | 45 |
| `TypeAlloc<ConditionalType>` | 0.63M | 112 | 72 | 68 | 44 |
| `NodeList` | 3.95M | 16 | 16 | 60 | 60 |
| `NodeAlloc<PropertyAccessExpression>` / `<CallExpression>` | 1.18M / 1.15M | 48 / 48 | 40 / 40 | 54 / 53 | 45 / 44 |
| `TypeAlloc<IntersectionType>` / `<UnionType>` | 1.13M / 1.02M | 48 / 48 | 48 / 48 | 52 / 47 | 52 / 47 |
| `TypeAlloc<TypeParameter>` | 0.56M | 80 | 56 | 43 | 30 |
| `FlowNode` | 1.84M | 24 | 16 | 42 | 28 |
| `TypeAlloc<LiteralType>` / `<MappedType>` | 0.58M / 0.27M | 64 / 112 | 56 / 72 (merged by identical code folding in the profile) | 36 / 29 | 50 together |
| `SymbolTables` | 0.79M | 40 | 24 | 30 | 18 |

The 32 GiB cap: the worst run above (opt-out, 4 checkers) has 6.0 GB of arena chunks, i.e. 6.0 GB of the
reservation in use (5x headroom).

Language server (`tools/lsp-mem`, RSS MiB):

| session | base start | base end | compressed start | compressed end |
| --- | --- | --- | --- | --- |
| xstate, 200 edits, diagnostics + hover + completion | 187 | 412 | 175 | 363 |
| the 38k-file codebase, 40 edits, diagnostics + hover | 2,479 | 3,989 | 2,256 | 2,992 |

The server behaves the same (0 errors, regions freed every edit); on the large project RSS falls after the first
edits with compressed pointers because a released region slab is decommitted at once, where mimalloc kept freed
region memory around.

### Instruction cost, and what was tried

- First version: `base` in an `AtomicPtr` global. +25% instructions, +47% wall time, parse +70%: LLVM never merges or
  hoists relaxed atomic loads, so every dereference reloaded it. A fixed base address makes `base` a constant (one
  `orr` with a shifted operand per dereference): +11%.
- Packed words that went address -> `&T` -> checked `from_static` on every decode (mapper data, type symbol word,
  inference context tail) now go through `from_bits`: +9% -> +7%. The release-mode check in `from_static` cost ~2%
  while it sat on mapper decoding; hot views of arena objects (`Node::as_p`, link slots, node / type allocation
  headers) use `from_arena` or `cast` and skip it.

## 5. Follow-up round: instructions, quiet-machine timing, slices, census

### Where the remaining instructions came from

Method: instructions retired (`/usr/bin/time -l`) on vscode's `src` (`--singleThreaded`, 12 s, +7.9% for the PR head
against a plain-ptrs build of the same commit, the same ratio as the 38k-file codebase), medians of 3 interleaved;
the run-to-run spread of one binary is about +-0.5% at load 20+ (page-fault work in the kernel counts too). Hot
functions from `sample` profiles, codegen read with `objdump` for the hottest ones (`compare_types`,
`get_apparent_type`, `find_ancestor`, `compare_nodes`, `get_type_at_flow_node`, `NodeLinkStore::get`,
`get_property_of_type_worker`). Per-function hardware instruction counts were not available: the CPU Counters
instrument records PMI samples without call stacks in manual mode here, so attribution is by experiment, one change at
a time:

| step | vscode instructions vs plain | what |
| --- | --- | --- |
| PR head | +7.9% | |
| base `0x4001_0000_0000` | +7.1% | one `movz`, and bit 32 overlaps the offset range, so LLVM emits `add x, base, w, uxtw #3` instead of `mov w, w` + `orr` (the zero-extension of a handle passed in a register was a separate instruction) |
| `P::pack` / `unpack` | +6.2% | node parents, identifier flow slots, symbol table entries and type symbol words keep the handle in the low 32 bits and read it with no mask (it was `to_bits` / 8 in 45 bits: mask, shift, truncate, and a 45-bit zero test next to the 32-bit one); `as_source_file_p` and reduce-label views skip the `from_static` range check |
| link stores by handle | +4.3..5.0% | `LinkStore` / `IdLinkStore` keep each chunk's first slot as a `P` and index with handle arithmetic (`P::array_add`); before, every lookup turned the element's address back into a handle (`mov` + `add` + `lsr`) |
| flow labels | -0.3 pt (noise level) | `antecedent()` / `antecedents()` decided by the `Label` flags, not by a tag bit in `text_index` (one load less per step) |
| id link store | noise | wide-id hash lookup out of line, chunk index unchecked (applies to both modes) |

Final, the 38k-file codebase against origin/main (5 rounds, below): +3.9% one checker, +5.3% four checkers. What is
left is the dereference itself: in the hot functions, sample-weighted, 1.2% of the instructions are dereference adds,
0.6% base materializations (`movz`, once per function that dereferences), 0.5% `ubfiz` address shifts where LLVM
folded a field offset into the constant, 0.2% zero-extends; that is ~2.5 of the ~4.5 points, and the rest is spread
(more spills and calls in a few functions whose inlining changed, none above 0.3%). The <2% target is not reached on
arm64: every pointer chase costs one `add` that a plain pointer does not, and only loads at offset 0 of an 8-byte
field fold it (`ldr x, [base, w, uxtw #3]`). On x86-64 the base can live in a register and `[base + idx*8 + disp]` is
one addressing mode, so the cost there should be the base register's pressure only (not measured: no x86 machine).
Chunk hand-out is not a factor: thread chunks are at most 64 MiB, ~150 `mprotect` calls per run.

### Wall time (load 8-13; it did not go below 6 for a whole run within 45 minutes)

5 interleaved rounds, base = origin/main ff92b7f, new = branch head:

| run | binary | wall s median (range) | peak GiB | instructions | page reclaims (first run) | page faults (first run) |
| --- | --- | --- | --- | --- | --- | --- |
| 1 checker | base | 19.23 (18.56-20.44) | 5.033 | 302.7 G | 331,990 (330,990) | 668 |
| 1 checker | compressed | 19.50 (18.18-21.63) | 4.285 (-14.9%) | 314.4 G (+3.9%) | 282,807 (282,807) | 1 |
| 4 checkers | base | 8.42 (7.65-9.29) | 6.684 | 403.2 G | 440,315 (439,542) | 669 |
| 4 checkers | compressed | 8.35 (8.11-8.86) | 5.727 (-14.3%) | 424.6 G (+5.3%) | 377,365 (377,587) | 1 |

Wall time is the same within the ranges (one checker +1.4% median, four checkers -0.8%). Page reclaims (minor
faults) drop 15% with the smaller arena. Major faults: ~670 per base run (first and later runs alike), 1 per
compressed run; the cause was not investigated. During this measurement the corpus changed under the runs (`Lines` 5,941,654 / 5,941,656, flipping in both
binaries, another session editing a file); runs with the same `Lines` have identical counters and diagnostics.

### `PSlice` / `PStr`

Not done, because the premise no longer holds on today's layout: the slices the brief lists (type lists, node list
elements, symbol declarations, `resolved_type_arguments`, signature parameter lists) and `PackedStr` are already one
word (`ThinSlice`, `OptionThinSliceCell`, mem-layout3 step 1), so an 8-byte `PSlice` would not shrink them. Going
through the alloc-profile type table for types above 0.5M allocations, the only remaining 12- or 16-byte slice field
is `InferenceContext.inferences` (a 12-byte `SliceCell`, 1.43M contexts): in compressed mode it is now a one-word cell
(`tsrs_core::PSliceCell`), `InferenceContext` 56 -> 48 bytes, arena requested 3,079.5 -> 3,068.6 MB (-11 MB; most
contexts are recycled, so the peak delta is within noise). A position-independent `PSlice` remains useful for the
persisted front end (node lists, declarations, identifier text), where it needs a decision on 4-byte-aligned lists
(offsets in 4-byte units cover only 16 GiB, or 33-bit offsets with 31-bit lengths) and on empty / static slices,
whose identity `same_slice` observes.

### Census in compressed mode

Left as is: compressed builds refuse `TSRS_CENSUS=1`; use `--features alloc-profile,tsrs_core/plain-ptrs` (plain
census on the branch head: strong mark 0 violations, precise walk 48,191,467 references, 0 to freed blocks). Teaching
the strong mark handles is not cheap: it scans every 4-byte-aligned word conservatively, and as 32-bit handles every
id, position, count and flag word between 8,192 and the arena's top handle points at some block, so freed blocks
would be "reached" by integers and the violation count would be noise unless every arena type registered its
scalar fields; the packed words would also need handle-specific decoders (`to_bits` byte offsets in mappers and
value-symbol links, `pack` in headers). The precise walk (`TSRS_CENSUS_VERIFY`) uses `addr()` and would work.

## 6. Linux x86-64: zero-based handles

Branch `perf/zero-based-handles`. On Linux x86-64 (the Forge sandbox: Intel Xeon 8259CL, 18 vCPU, kernel 7.2) the
compressed build cost +7% instructions and +6.6% wall against `plain-ptrs` (section 5 guessed x86 addressing modes
would make it free). The idea tested: put the range in the low address space with base 0, as the JVM's zero-based
compressed oops do, so the address is `handle << 3` and nothing else. All x86 numbers are `cargo build --release`
(not the `dist` profile), three binaries from one session: plain = branch head with `plain-ptrs`, today =
origin/main 0545afd, zero = branch head.

### Codegen

The base 0x4001_0000_0000 does not fit a 32-bit displacement, so LLVM materializes it with a 10-byte `movabs` and
uses `[base + idx*8]`; it folds the field offset into the constant, so two fields of one object cost two `movabs`
(`get_reduced_type`, all of it):

```
plain                      today                                zero-based
mov    0x8(%rsi),%eax      mov    %esi,%eax                     mov    %esi,%eax
                           movabs $0x400100000008,%rcx          mov    0x8(,%rax,8),%ecx
                           mov    (%rcx,%rax,8),%ecx
test   $0x18000000,%eax    test   $0x18000000,%ecx              test   $0x18000000,%ecx
...                        ...                                  ...
testb  $0x2,0xf(%rsi)      movabs $0x40010000000f,%rcx          shl    $0x3,%rax
                           testb  $0x2,(%rcx,%rax,8)            testb  $0x2,0xf(%rax)
```

With base 0 the field offset goes into the displacement (`[idx*8 + disp32]`) and the constant disappears; what stays
is the zero-extension of a handle that arrives in a register (`mov %esi,%eax`: the SysV ABI leaves the upper half of
a 32-bit argument undefined) and, where LLVM keeps the address live, a `shl $3`. A handle loaded from memory needs no
extension (32-bit loads clear the upper half), so a pointer chase costs nothing on x86-64: one `mov 16(,%rcx,8),%edi`
per hop, the same instruction count as plain. Per access, from a probe crate on `tsrs_core` (`cargo rustc --emit asm`;
instructions beyond plain's single load):

| access | x86-64 today | x86-64 zero | aarch64 today (Linux and macOS) | aarch64 zero (Linux) |
| --- | --- | --- | --- | --- |
| one field through a handle in a register | `mov` + `movabs` | `mov` | `ubfiz` + `mov` + `movk` | `ubfiz` |
| each further hop of a chain | 0 (base hoisted) | 0 | `add x, base, w, lsl #3` | `lsl` |
| constant per function and field-offset group | `movabs` + a register | none | `mov` + `movk` + a register | none |

The whole binary: 11,992 `movabs` of the base; text instructions plain 3,559,838, today 3,598,574 (+1.09%), zero
3,579,866 (+0.56%). arm64 has no scaled-index-plus-displacement mode (`ldr w, [x, w, uxtw #2]` scales by the access
size only), so on arm64 zero-based saves the base constants and their register, not the per-hop instruction.

### Where the remaining instructions are

`perf record -e instructions:u -c 2000003`, one checker, each sampled instruction classified from `objdump` (samples
include skid, so read the deltas, not the absolute shares):

| category | plain | today | zero |
| --- | --- | --- | --- |
| all samples | 119,965 | 128,008 (+8,043) | 124,897 (+4,932) |
| `movabs` of the base | 0 | 1,557 | 0 |
| register moves (`mov r32,r32`, `mov r64,r64`, same-register zero-extend) | 12,245 | 14,361 | 14,002 |
| handle <-> address (`shl $3`, `lea (,r,8)`, `shr $3`) | 849 | 692 | 2,333 |
| push / pop and stack moves | 24,746 | 26,401 | 25,280 |

Zero-based removes the base (2.7 of the 7.0 points), not most of the cost. Of the 4.1 points left (in samples), 1.5 are extra
register moves (a 32-bit move is how x86 zero-extends, so copying a handle and extending it are one instruction, and
the handle and its address are often live together), 1.2 are handle <-> address shifts, 0.45 extra spills, and ~1
point is spread over everything else (the compressed layouts' packed words, inlining differences). These come from
the representation itself: every time a handle becomes a `&T` (a method on `&self`, an argument passed to a function
that is not inlined) it is extended and shifted, and every `as_p()` / `from_arena` shifts back. The top functions by
32-bit register move samples are the checker's hot entry points (`get_apparent_type`, `get_property_of_type_worker`,
`get_resolved_symbol`, `is_simple_type_related_to`, `get_type_of_symbol`); about a quarter of the 32-bit register
moves sit in the first 12 instructions of a function (arguments). Two things could still reduce it, neither
measured here: the release profile (`dist`: fat LTO, one codegen unit, PGO) inlines across crates and removes call
boundaries; and APIs that take the handle instead of `&T` on the hottest paths.

### Address space per target

- **Linux x86-64**: the range is 4-32 GiB (`FIRST` = 4 GiB, so 28 GiB usable; the worst measured run uses 6 GB).
  Seen in the sandbox: a PIE executable maps at 0x55..-0x56.. (0x5555_5555_4000 with ASLR off) with its brk heap
  right after it, shared libraries and default `mmap` below the stack near 0x7f.., mimalloc's own range at 2 TiB.
  The legacy layout (`ulimit -s unlimited`) maps bottom-up from TASK_SIZE/3 (~42 TiB). A non-PIE executable loads at
  0x400000 with its brk heap after it (randomized to 0x1930a000 in one run, i.e. ~400 MiB), the reason `FIRST` is 4
  GiB and not 256 MiB; `MAP_32BIT` users (some JITs) live in the low 2 GiB. tsrs is PIE (rustc's default for
  `*-linux-gnu`), the release targets are glibc only, and the npm package runs tsrs as a child process (no
  `cdylib`), so no foreign executable shares its address space. Checked in the sandbox with the zero build: ASLR off
  (`setarch -R`), `ulimit -s unlimited`: both run. `ulimit -v` 16 GiB: `tsrs: could not reserve the arena address
  range (compressed pointers: one 28 GiB address range at 0x100000000..0x800000000 for every arena; ...)`. A mapping
  placed at 5 GiB first (`LD_PRELOAD` constructor): `tsrs: the arena address range is taken by another mapping (...)`.
  Both abort, as before. The reservation uses `MAP_FIXED_NOREPLACE` (Linux 4.17+; older kernels treat it as a hint,
  which the address check catches). Not compatible (from ASan's documented layout, not tried): AddressSanitizer, whose x86-64 shadow covers 2
  GiB-16 TiB (the high base at 64 TiB was outside it); such builds need `plain-ptrs`. Not tried: gVisor (the sandbox is a microVM
  with a real kernel).
- **Linux aarch64** (release target `aarch64-unknown-linux-gnu`): same cfg, same range. PIE executables load at 2/3
  of the address space and `mmap` works top-down, so 4-32 GiB is free; kernels with a 39-bit address space (512 GiB)
  have no 64 TiB at all, so the old high base would abort there and the low range fits. Codegen from cross-compiled
  `--emit asm` only (table above); no aarch64 Linux machine was available to run or measure.
- **macOS arm64**: `__PAGEZERO` covers the low 4 GiB and the executable, dyld shared cache and malloc zones sit above
  it, so no contiguous low 32 GiB exists. Keeps the fixed base (`cfg(not(target_os = "linux"))`), where it costs no
  wall time (section 5). This Mac, the 38k-file codebase, one checker, origin/main vs branch head, 2 rounds:
  289.1 / 289.0 G vs 289.4 / 289.1 G instructions (noise), identical counters.
- **Windows**: plain pointers, unchanged.

What changed in code: `reserve::BASE_ADDR` (0 on Linux, 0x4001_0000_0000 elsewhere), `reserve::FIRST` (the first
offset handed out: 4 GiB on Linux, 64 KiB elsewhere), the reservation covers `BASE_ADDR + FIRST .. BASE_ADDR +
RESERVE`, `reserve::at(off)` builds a pointer from the integer when the base is 0 (`base().add(off)` from a null
base would be UB), `from_static` / `from_arena` reject offsets below `FIRST`. Packed words, tag bits, `pack`,
`to_bits` and `key` store handles or offsets, which do not depend on the base; arena addresses stay above `u32::MAX`
on every target; `SP<T>`, `frozen.rs` and the alloc-profile poison check use real addresses and are unaffected.

### Measurements

7 interleaved rounds (order rotated from round 4), medians, `/usr/bin/time` max RSS, `perf stat` user+kernel
instructions and cycles; diagnostics (40,543 errors, same md5), Symbols, Types and Instantiations identical in all 42
runs:

| run | plain | today | zero-based |
| --- | --- | --- | --- |
| 1 checker wall | 63.79 s | 65.63 s (+2.9%) | 65.77 s (+3.1%) |
| 1 checker instructions | 253.5 G | 271.2 G (+7.0%) | 264.3 G (+4.3%) |
| 1 checker cycles | 210.8 G | 220.3 G (+4.5%) | 221.0 G (+4.8%) |
| 1 checker max RSS | 5.29 GiB | 4.47 GiB (-15.6%) | 4.47 GiB (-15.6%) |
| 4 checkers wall | 25.78 s | 27.78 s (+7.8%) | 26.96 s (+4.6%) |
| 4 checkers instructions | 345.9 G | 369.7 G (+6.9%) | 360.4 G (+4.2%) |
| 4 checkers cycles | 277.8 G | 293.2 G (+5.5%) | 288.6 G (+3.9%) |
| 4 checkers max RSS | 6.96 GiB | 5.89 GiB (-15.4%) | 5.89 GiB (-15.3%) |

Wall and cycles are noisy on this host: one binary's wall varies 6-14% across the session. Paired per round, zero vs
today cycles: median -0.7% with one checker (rounds from -2.8% to +2.4%), -2.0% with four (-3.3% to +1.3%). So the
instruction saving (2.7 points) shows up at most as ~2% cycles with four checkers and not measurably with one;
compressed handles still cost 3-5% cycles on x86-64 either way, against 15% less memory.

Gates: in the sandbox, `tsrs-test run --suite all --baselines types,symbols` with the zero build and with a
`plain-ptrs` build (reference checkout at the pinned commit, as CI does): 13,458 error baselines pass (+2 codes, 2
fail), 12,779 / 12,779 types / symbols, `test-results` trees identical; `tsrs_core`'s reservation tests pass with
base 0. On this Mac (high base, unchanged) against origin/main: conformance (default and
`TSRS_LAZY_MEMBERS=0`), emit baselines and fourslash trees identical, tsctests 374 / 32 / 1 with identical lists.

## Not done

- Instructions below +2% on arm64 (section 5: what is left is one `add` per pointer chase).
- x86-64: the 4 points zero-based leaves (section 6: zero-extends and shifts at handle <-> reference conversions);
  the `dist` profile was not measured.
- `PSlice<T>` / `PStr` for position independence of the front end (section 5).
- The census strong mark in compressed mode (section 5).
- The reservation needs no `ulimit -v` below 32 GiB and, outside Linux, a 47-bit user address space (arm64 with 48-bit
  VA); on Linux the range is 4-32 GiB (section 6), which also rules out AddressSanitizer. Otherwise `tsrs` aborts
  with a message naming `tsrs_core/plain-ptrs`. Windows builds fall back to plain pointers (build.rs; a
  `VirtualAlloc` reservation would do).
- Smaller layouts that compressed handles would allow but need a representation change: the `Type` header 24 -> 20
  bytes (symbol word as a handle plus a record byte; gains only where the payload is 4-aligned, at most ~38 MB),
  `TypeMapper` below 16 bytes (needs a place for kind and escape bits), the checker's raw-pointer tail words.

## Reproducing

```sh
cargo build --release -p tsrs_cli                                    # compressed (default)
cargo build --release -p tsrs_cli --features tsrs_core/plain-ptrs    # references, as before
cd <38k-file codebase> && /usr/bin/time -l <tsrs> -p . --noEmit --extendedDiagnostics --pretty false --incremental false --checkers 1
CARGO_TARGET_DIR=target/prof cargo build --release -p tsrs_cli --features alloc-profile   # per-type table: TSRS_ALLOC_PROFILE_TOP=3000
python3 tools/lsp-mem/lsp_mem.py --cmd "<tsrs> --lsp -stdio" --project <dir> --file <file> --edits 200 --every 20
```
