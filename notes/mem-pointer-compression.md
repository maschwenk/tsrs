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
- `FlowNode` 24 -> 16: the antecedent / list tag sits in bit 31 of `text_index` in compressed mode (a handle has no
  spare bit; text indices are below 2^20 or `NO_SOURCE_TEXT`).
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

The 38k-file codebase, `--noEmit --incremental false`, base = origin/main, interleaved, medians of 3, peak = peak
memory footprint (`/usr/bin/time -l`):

| run | base peak GiB | compressed peak GiB | delta | base instructions | compressed instructions |
| --- | --- | --- | --- | --- | --- |
| 1 checker | 5.041 | 4.294 | -0.747 (-14.8%) | 298.0 G | 318.0 G (+6.7%) |
| 4 checkers | 6.670 | 5.733 | -0.936 (-14.0%) | 405.2 G | 431.5 G (+6.5%) |
| `--noCheck` | 1.822 | 1.643 | -0.179 (-9.8%) | 54.4 G | 52.4 G (noise: 51-62 G both) |
| opt-out, 4 checkers, go assignment (1 run) | 10.468 | 8.834 | -1.634 (-15.6%) | 492.0 G | 528.8 G (+7.5%) |

Wall time: only the first round ran on a quiet machine (load 6-7): 1 checker 17.66 -> 17.16 s, 4 checkers 7.38 ->
7.14 s, `--noCheck` 0.79 -> 0.80 s. Later rounds ran at load 13-43 (other agents) and are not comparable. So: more
instructions, no slower wall time where it could be measured; a quiet-machine rerun is still owed before claiming
anything about speed.

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
| `InferenceContext` | 1.43M | 64 | 56 | 87 | 76 |
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
- A base with a bit inside the offset range (0x4000_0001_0000), so LLVM keeps an `add` with a zero-extending operand
  instead of `mov` + `orr`: +0.4%, noise. Not kept.
- What is left (~+6.5%) is the dereference itself (zero-extend + combine with the base, ~1-2 instructions per access)
  spread over the whole checker; no single function stands out in a sampled profile.

### Not done

- `PSlice<T>` / `PStr` (offset + length, 8 bytes, position independent) for the remaining absolute slices and strings.
- The census (alloc-profile + `TSRS_CENSUS=1`) in compressed mode: it would have to decode 32-bit handles; compressed
  builds refuse it, plain builds keep it (docs/DEBUGGING.md).
- The fixed base needs a 47-bit user address space (x86-64, arm64 with 48-bit VA) and no `ulimit -v` below 32 GiB;
  otherwise `tsrs` aborts with a message naming `tsrs_core/plain-ptrs`. Windows builds fall back to plain pointers
  (build.rs; a `VirtualAlloc` reservation would do).
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
