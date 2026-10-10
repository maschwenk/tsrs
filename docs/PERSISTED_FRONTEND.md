# Persisted front end (design, not built)

Status: design and measurement only. **Recommendation: do not build it now** (see "Recommendation"). Measurements
are in `notes/design-persisted-frontend.md`; the instrumentation is on branch `design/persisted-frontend-probe`.

## The idea

tsrs parses and binds every file on every run. Bundled `lib.*.d.ts` files and `node_modules` rarely change between
runs. Their parsed + bound form (nodes, node lists, identifiers, binder symbols and symbol tables, flow nodes,
source text) could be written once to a cache image and `mmap`ed on later runs, with each file validated by a
content hash. Parse + bind then turns into page faults, and only the pages the checker reads become resident.
Neither tsc nor tsgo does this.

The idea depends on 32-bit position-independent arena handles (`P<T>` as an offset from one process-wide base:
the default `compressed-ptrs` build, `tsrs_core/src/reserve.rs`, notes/mem-pointer-compression.md). Without handles,
every pointer in a mapped image would need relocating, which means writing every page and losing the laziness.

## What the measurements say (summary)

| question | answer (38k-file codebase) |
| --- | --- |
| share of the program that is cacheable (libs + `node_modules` + workspace `.d.ts`) | 27% of files, 38% of text, 26% of nodes, 29% of binder symbols, 28% of parse + bind output (0.41 of 1.49 GiB) |
| parse + bind CPU of those files | 0.33 s on one core, out of a single-threaded front end of 3.03 s; the rest is file-system calls and module resolution, which an AST cache does not remove |
| wall time saved, upper bound | 0.33 s at 1 thread, 0.08 s at 4, 0.04 s at 8, 0.02-0.03 s at 18 |
| pages of those files the rest of the run touches | full check: 73% read, 18.5% written (lazy node/symbol ids, lazy JSDoc); checker creation alone: 36% |
| memory a lazy mapping keeps out | ≤ 0.11-0.15 GiB of the CLI's 7.0 GiB peak; ~0.2-0.25 GiB of the language server's 2.4 GiB at first diagnostics |
| page-in cost (macOS, warm page cache) | 0.57 µs per 16 KiB page, vs 0.75 µs for the zero-fill fault the parser pays today; `read()` of a 0.5 GiB image takes 0.04 s |
| language server time to first diagnostics | 1.20 s (tsgo-ref 1.76 s); AST cache saves ~0.02-0.03 s of that at 18 threads |

## What has to be relocatable and deterministic

### Ids assigned at parse and bind

- **Node and symbol ids** are lazy. A 32-bit field in the node header and in `Symbol` (`ast.rs`, `symbol.rs`) is
  filled from global atomic counters `NEXT_NODE_ID` / `NEXT_SYMBOL_ID` on the first `get_node_id` /
  `get_symbol_id` (`tsrs_ast/src/utilities_1.rs`, Go `ast/utilities.go:17-45`), with 0 meaning unassigned. There are
  no merge ids: Go's `mergedSymbols` is keyed by pointer.
- **The binder assigns some ids itself.**
  - Private names: `get_symbol_name_for_private_identifier` builds the member key `"\x7f#<classSymbolId>@#name"`.
    The id is baked into a symbol name and a symbol-table key, and the checker recomputes the same string from
    `get_symbol_id(class)` at lookup time.
  - Pattern ambient modules with import attributes embed a node id in their name.
  - Namespaces: `get_module_instance_state` memoizes by node id, so binding a namespace assigns ids to its body's
    statements.
  - In the 38k-file codebase, 158 of 10,335 declaration files have private names, and 1,583 have namespaces.
- **Numeric values are observable.**
  - `compare_symbols` falls back to symbol ids (`tsrs_checker/src/utilities.rs`, Go `utilities.go:389`), and that
    feeds `sort_symbols` and the `compare_types` tie-breaks.
  - Unique-symbol property names embed the symbol id (`"\x7f@name@<id>"`), and the node builder counts their length
    toward truncation (`approximate_length`). The comment on `SymbolArenaLinkStore` (`links.rs`) says assignment
    order must match.
  - Node id values reach no checker output: `compare_nodes` uses file index + position. They are used as link keys,
    as printer `generated@id` placeholders (emit), and by the API encoder's sort.
- **Exactness rule for a cached run.** In a single-threaded run every id must get the value it gets in a fresh run.
  With parallel binding and 4 checkers, values already vary from run to run. That forces three things:
  1. Images store every id field as 0.
  2. Loading a cached file replays its bind-time id consumption at the point in the bind sequence where the file
     would have been bound: `NEXT_NODE_ID.fetch_add(m)`, with the count recorded at write time.
  3. Files whose bind assigned symbol ids or baked an id into a name (private names; pattern ambient modules with
     attributes) are not cached in v1. They are 1.5% of declaration files. Patching their names on load would need
     new strings and table writes.
- **Lazy id writes dirty pages.** The checker writes ids into nodes and symbols of library files: 18.5% of cacheable
  pages after a full check (`assign_node_id`, `assign_symbol_id` are the top first writers). With `MAP_PRIVATE`
  those pages become private copies, which costs what they cost today and no more.
  - Pre-assigning ids in a reserved range at write time would avoid the copies, but it would change id values and
    relative order between library and project symbols. That can change output: the unique-symbol name lengths and
    the sort tie-breaks above.
  - Moving ids to a side table for mapped objects adds a lookup on the hottest path (every link-store access). Not
    worth it for 0.1 GiB.

### Address order

`P<T>` orders and hashes by address (by handle in the default `compressed-ptrs` build). The port has no `BTreeMap<P<_>>` and no
address-keyed sort (survey in this note and in mem-pointer-compression). The one `FxHashMap<P<Symbol>>` iteration
that reaches output is sorted by `sort_symbols` afterwards. Addresses already differ from run to run (which rayon
worker parses which file), so a dependency on address order would already show up as flakiness.

### The identifier text registry

There is no global identifier interner (mem-round2 measured one and rejected it). Instead:

- Identifiers and binder flow nodes carry a **text index**, a slot in `SOURCE_TEXTS` (2^20 slots, registered once per
  parse, in parse-start order; `tsrs_ast/src/identifier.rs`).
- An identifier's 64-bit word is the text index, or a flow node's address / 8 (bits 0..45), plus length and mode
  bits. The text itself is a slice of the file's source text, found by the node's end position.
- Flow nodes hold the text index (`FlowNode::new(.., text_index)`), so an identifier whose slot holds a flow pointer
  can still find its text.
- `SIDE_FLOW` handles the case where the indices differ (empty in practice).

A cached file must therefore get the same text index in every process. The design reserves the upper half of
`SOURCE_TEXTS` for cache ordinals: a cacheable miss is parsed with index `CACHE_TEXT_BASE + ordinal`, and a hit
registers its mapped text at that slot (one table write, no page writes). Files whose bind used `SIDE_FLOW` are not
cached. Text index values are not observable.

### Source text

In the CLI the text is today a leaked heap `String` (`parse_source_file_owned`). In regions it is copied into the
region. The cache holds it in the file's segment, first, so identifier slices and string literal texts point into the
segment. On a hit the loader still needs the bytes once, to validate (below).

### What is not arena memory today

Survey of `SourceFile`, node payloads, `Symbol`, `SymbolTable`, `FlowNode`, `NodeList`. Every item here must become
handle-based inside the segment, or be rebuilt live on load.

| item | today | in the image |
| --- | --- | --- |
| `P<T>` fields and the packed words (node parent, identifier slot, `SymbolParentWord`, `FlowNode.link`) | 32-bit handles in the default `compressed-ptrs` build (absolute with `plain-ptrs`) | handles; no fixup when the image is mapped at its window offset |
| slices and strings: `ThinSlice` (`NodeList.nodes`), `PackedStr` / `StrCell`, `OwnedSliceCell` (symbol declarations), `&'static [P<Node>]` in `SourceFile`, `&'static str` symbol names | absolute (pointer compression leaves them; its notes name `PSlice` / `PStr` as the follow-up) | `PSlice` / `PStr` (32-bit handle + length) are a **prerequisite** |
| `SymbolTable` entries | `EntryVec`: a `malloc` buffer of words holding symbol address / 8, plus a heap `HashTable` index above a size | arena-resident entry buffer of handles; the hash bits are name-based, so the index can be rebuilt or stored |
| `SourceFile` heap and sync fields: `parse_options` (`String`, `Path(Arc<str>)`), `OnceLock`s (line map, position map, identifier set, name table, declaration map), `jsdoc_cache` (map keyed by node address) + `jsdoc_mu`, `token_cache`, `bind_once`, `is_bound` | in the struct | split: a persisted `SourceFileRecord` of handles and scalars in the segment, and a live side struct built on load (below) |
| `pragmas` (`Pragma` with `String` / `FxHashMap`), `FileReference.file_name: String` | heap inside arena structs | `PStr`-based copies in the segment |
| pointers into the binary: symbol names that are `InternalSymbolName` constants, `&'static Message` in diagnostics, the `missing_list_nodes()` sentinel compared by address | `.rodata` / statics | names: arena copies (compared by value); the sentinel gets a fixed well-known handle in every process; files with parse or bind diagnostics are not cached |
| lazily filled data of a cached file (JSDoc nodes, line maps, identifier sets) | allocated in the owner region / thread arena | must never go into the mapped segment: the live side struct's region, or the thread arena in the CLI |

**Cross-file pointers: none.** The binder never touches another file or a program object; Go's `bindSourceFile` takes
only the file. Symbol `parent`, `file.symbol`, `locals`, `global_exports` and `Diagnostic.file` stay inside the file.
Resolved modules and `SourceFileMetaData` live in the `Program`. Merged and global symbols are checker-owned
(clones, `mergeSymbol`). So a segment is self-contained apart from the well-known handles.

**Process-global state a hit must restore:**

- the text-index slot;
- the id counters (replay);
- `tsrs_parser::jsdoc::init()`: lazy JSDoc panics if the parser never ran;
- the language server's region registry for the file's lazy data.

### What the cache key must include

Parsing and binding read no compiler options.

- **Binder:** Go's `binder.go` reads none. `alwaysStrict`, `preserveConstEnums`, `isolatedModules` and target are
  checker-side (`nameresolver.go`, `isInstantiatedModule`).
- **Parser:** its only inputs are `SourceFileParseOptions { file_name, path, external_module_indicator_options }`,
  the text and the `ScriptKind`.
  - `ExternalModuleIndicatorOptions { jsx, force }` comes from `moduleDetection`, `module` (and so `target`), `jsx`
    and the file's implied node format / `package.json` type. It is always empty for declaration files
    (`ast/parseoptions.go`).
  - This Go snapshot has no `SourceFileAffectingCompilerOptions`. The `affectsSourceFile` flags on option
    declarations are consumed only by the test runner.

| file kind | key |
| --- | --- |
| `.d.ts` / `.d.mts` / `.d.cts`, bundled libs | tsrs build id; `path` (case-folded per `useCaseSensitiveFileNames`) and `file_name`; script kind (from the extension); content hash |
| other `node_modules` files (12 in the 38k-file codebase) | the above plus `jsx` and `force` (the only parser-visible bits of the options; they also decide the top-level-await reparse) |

The **build id** is a hash of the tsrs version, the git commit, the Go reference commit (`b85298b6a81f`), the
`Kind` / data-tag discriminant tables, `size_of` / `offset_of` of every persisted struct and the page size. Any
layout change invalidates the whole cache, which is the only safe policy for a format that is the in-memory layout.

## Design

### Address space: a fixed window inside the handle space

Pointer compression reserves one 32 GiB region per process, with handles = (address − base) >> 3. An image is only
position-independent relative to base, so it must be mapped **at the same handle offset in every process**. Remapping
it elsewhere would mean adding a delta to every handle, which writes every page.

- **The window.** Handles from the first page up to a fixed cap `W` (e.g. 2 GiB, 1/16 of the space) belong to the
  cache. Published generations occupy `[first page, A)`, and a run appends its new segments at `A`. The live chunk
  allocator starts above `W`.
- **Per-process setup.** At process start, before the first chunk exists, the loader opens the cache index, learns
  `A` and maps each generation file `MAP_FIXED | MAP_PRIVATE` over its range of the `PROT_NONE` reservation. The
  range from `A` to `W` stays reserved for this run's misses. A process without a cache reserves no window.

What this needs from pointer compression (landed; these are additions to it):

1. A start-up hook that sets the size of the reserved low window before the first chunk is handed out. The chunk
   allocator never hands out window ranges. A miss's segment is committed at the append point on demand, with a
   mutex-guarded bump, like the chunk allocator.
2. `map_at(handle_offset, fd, file_offset, len)`: `MAP_FIXED | MAP_PRIVATE` over the reservation (works on macOS
   and Linux; Windows would need placeholder-based `MapViewOfFile3`).
3. The guarantee that a handle is `(addr − base) >> 3`, the same in every process, with an 8-byte unit and handle 0
   never used. Segments start on 16 KiB boundaries (the largest page size in use).
4. `PSlice<T>` / `PStr` for the front end's slices and strings, and arena-resident symbol-table entries. Without them
   segments still hold absolute addresses.
5. A fixed handle for the `missing_list_nodes()` sentinel (and any other identity-compared front-end static), the
   same in every process. Allocating it first in the window works.

### Image layout

One cache directory per build id, e.g. `$XDG_CACHE_HOME/tsrs/fe/<build-id>/`, holding:

- `index`: a small file, rewritten atomically. It has a header (magic, format version, build id, page size, append
  point `A`, generation) and a per-file table sorted by key: key, content hash (xxh3-128 of the text), `stat`
  fingerprint (dev, inode, size, mtime ns, ctime ns), segment handle range, `SourceFileRecord` handle, text handle and
  length, text ordinal, node ids consumed at bind, flags.
- `seg-<offset>.bin`: append-only generations. Each covers a contiguous handle range `[A_k, A_{k+1})` and is mapped
  at it. A generation holds whole segments, each 16 KiB-aligned:
  - the text;
  - the nodes, in parse order;
  - the binder output (symbols, tables, flow nodes);
  - the `SourceFileRecord`.

**One segment per file, page-aligned**, so a file that is never read costs no resident memory, and dropping a file
from the index frees whole pages. The probe measured this layout. Packing small files together would save the
page-rounding (516 MiB page-rounded vs 413 MiB packed here) but would let one file's reads page in its neighbours.
For one-page files that hardly matters, because checker creation reads every file anyway.

### Writing: parse misses straight into the window

A cacheable file that misses gets a cache ordinal and a segment at the current append point of the window, which
in the writing process is backed by ordinary anonymous memory. It is parsed and bound into a region whose single
chunk is that segment (the region-per-file mechanism the language server already uses), with text index
`CACHE_TEXT_BASE + ordinal`. When binding finishes, the segment's bytes are already at their final handles: they
are written out verbatim. **There is no relocation walk.**

Before writing, a verifier (debug and `TSRS_FE_CACHE_VERIFY=1`) walks the file's graph with the generated child
visitors plus symbols, tables and flow nodes. It asserts that every handle points inside the segment or to a
well-known handle, and that no id field is set (ids from the binder are zeroed and recorded).

The snapshot is taken right after binding, before any checker writes ids or JSDoc into those pages.

At the end of the run the process publishes:

1. It takes `flock` on `index.lock`.
2. It re-reads `index`. If the generation is the one it started from, it writes `seg-<A>.bin.tmp` with its new
   segments and fsyncs it.
3. It renames the temp file into place.
4. It writes `index.tmp` and renames it over `index`.

A process that lost the race (another run published a generation at the same offset) drops its segments; they stay
valid in its own memory and the next run retries. Readers never see a partial state, and an `mmap`ed old generation
stays valid after a later one is published.

### Loading: hit or miss per file

The file loader calls `host.get_source_file(opts)`. The cache wraps that call: the default host in
`tsrs_compiler/src/host.rs`, the `--build` host's `.d.ts` cache in `tsrs_execute/src/build/host.rs` and the language
server's `parsecache` all go through it. Steps:

1. Not cacheable (extension, options key, or a flagged file) → parse as today.
2. Look up the key in the index (loaded at start-up). Missing → miss.
3. Validate:
   - **Fast path:** the `stat` fingerprint matches and is older than the index write time by more than the
     timestamp granularity (git's racy-index rule).
   - **Otherwise:** read the file and compare xxh3-128.

   A mismatch is a miss.
4. **Hit:** register the text slot, create the live side struct (`parse_options` from `opts`, empty `OnceLock`s,
   `bind_once` completed, `is_bound` set) and replay the id consumption at the file's place in the bind order. The
   loader returns the `SourceFile` node at its window handle; the persisted record and the live struct are found
   from it, the live struct through a dense table indexed by text ordinal.
5. **Miss:** parse as described under "Writing".

The program, checker and language service see a normal bound `SourceFile`. Nothing on the checker's paths knows
whether a file came from the cache, except the lazy-data routing (owner region for the live struct, never the
segment).

### Invalidation

- Content hash per file, build id per cache. A `node_modules` upgrade turns into misses for the changed files and
  appends new segments.
- Dead segments stay in old generations until the cache is compacted: when dead bytes exceed half of `A`, or `A`
  reaches the cap `W`, the next run starts a new cache directory, and old directories are deleted by age.
- **CI.** Do not upload the image as a CI cache artifact. At ~0.4-0.5 GiB for this codebase (perhaps 0.1-0.15 GiB
  compressed; not measured), downloading it costs seconds, against a saving of ~0.1 s. It pays only on persistent
  runners, or baked into the runner image next to `node_modules`.

### `--build` and the language server

- **`--build`.** Go's orchestrator already shares `.d.ts` and `.json` `SourceFile`s between the projects of one
  build in memory (`execute/build/host.go:55`). The persisted cache sits under that host, so it helps across processes and
  runs, not within one build. Up-to-date projects are skipped by tsbuildinfo and parse nothing either way.
- **Language server.** The parse cache (`tsrs_project/src/parsecache.rs`) refcounts files and gives each version a
  region.
  - A hit's region holds only the live side struct and the lazy data; the segment is never freed.
  - Eviction drops the live struct, and the OS reclaims clean window pages under pressure.
  - Several server processes (several editor windows) share clean pages.
  - Segments for files first parsed by the server can be published at idle time.
  - `node_modules` changes reach it through the watcher; the content hash keeps it correct regardless.

### Proving correctness

1. **AST oracle.** Add a `--via-cache` mode to the Rust `ast_oracle` example: parse → write the segment → map it in a
   *fresh process* → dump hash. It must hash identical to a fresh parse on the 113 lib files, the split test units and
   all 38,958 files of the private monorepo (`tools/oracle/ast/run.py`). The same for the binder oracle
   (`tools/oracle/binder`).
2. **Suite** (`tsrs-test run --suite all --baselines types,symbols`).
   - Runs: cache off, then cache on cold (populating), then cache on warm (all hits). The whole
     `target/test-results` trees must be identical across the three.
   - Modes: default, `TSRS_LAZY_MEMBERS=0`, and `TS_TEST_PROGRAM_SINGLE_THREADED=false`.
   - Also: fourslash with the cache on, and the `.js` / declaration emit baselines.
3. **Private monorepo.**
   - Diagnostics and `--extendedDiagnostics` counters must be identical with the cache on and off, with 1 and 4
     checkers.
   - The `types-dump` walk (docs/DEBUGGING.md, the quick tier per change, the full tier before landing) compared
     cache-on against cache-off must give identical per-expression types.
4. **Id sequence.** A debug assertion: in a single-threaded run, `NEXT_NODE_ID` / `NEXT_SYMBOL_ID` at exit are equal
   with the cache on and off. This is the check for the replay rule.
5. **Robustness.** Truncated or overwritten generation files, a wrong build id, a stale index, concurrent writers (two
   runs started together), and an edited `node_modules` file must each produce misses, never crashes or wrong
   diagnostics. Segment bytes are not checksummed on every load, because that reads every page. `index` carries a
   checksum, generations are only ever renamed into place, and `TSRS_FE_CACHE_VERIFY=1` checksums the segments.

## Recommendation

**Do not build it now.** Estimated wins on the 38k-file codebase, net of the cost of a hit:

| environment | today | saved by the cache | rests on |
| --- | --- | --- | --- |
| this 18-core Mac, cold full check | 6.87 s, 7.0 GiB peak | ~0.01-0.03 s (≤ 0.4%, inside the noise); ≤ 0.11-0.15 GiB | per-class CPU at 1 thread (0.33 s cacheable) scaled to 18 threads; bind share of the measured 0.035 s; touched pages 72.8% |
| 4-8 vCPU CI runner, cold full check | front end 1.1 s (4 threads) / 0.97 s (8) on this Mac's cores; the runner's cores are slower (not measured) | ~0.06-0.1 s at 4 vCPU, ~0.03-0.05 s at 8; ≤ 0.15 GiB. **Negative** if the image must be downloaded first | the same CPU split at 4 and 8 threads; front-end wall from the `RAYON_NUM_THREADS` runs |
| language server cold start (open one file) | 1.20 s, 2.4 GiB | ~0.02-0.03 s (18 threads), ~0.06-0.08 s (4), ~0.3 s (1); ~0.2-0.25 GiB at start, plus ~0.22 GiB shared per additional server process | LSP runs at 1/4/8/18 threads; checker creation touches 36% of cacheable pages |

**Effort**, on top of pointer compression (landed), in focused agent time:

- `PSlice` / `PStr` for the front end: 1-2 weeks (hundreds of sites: `NodeList.nodes` alone has ~300).
- Arena-resident symbol-table entries: 2-3 days.
- `SourceFile` split into a persisted record and a live struct: 3-5 days (44 fields).
- Static references: 1-2 days.
- Text-index reservation and id replay: 2 days.
- Window mapping: 1-2 days.
- Writer, index, generations and locking: 4-6 days.
- Loader in the three hosts: 3-4 days.
- Verifier, oracle modes, gates: 4-6 days.

**Total: about 5-8 weeks**, then a standing tax. Every new AST or binder field must be persistable, and every
upstream sync that touches `SourceFile` or the binder must keep the record in step. Go has no equivalent to port
from.

**Risks:**

1. Exactness: id replay, lazy state written into shared files, and digit-count effects of ids on truncation. The id
   rule above is subtle, and a mistake shows only as a rare printed-type difference.
2. Hidden absolute pointers in a segment (heap, `.rodata`, another region) read in a later process are silent garbage,
   not crashes. The verifier must be complete, and it must be kept complete as fields are added.
3. Stale or corrupted images give wrong diagnostics, not misses, unless the build id covers every layout input and
   publishing stays atomic.
4. Multi-process writers, network file systems, Windows (fixed-address mapping inside a reservation), and the window's
   share of the 32 GiB handle space.
5. The win is inside the measurement noise on the main development machine, so regressions in the cache path would be
   hard to see.

**Better targets for the same goals:**

- **Program construction.** On this Mac it is 0.71 s of "Parse time" at 18 threads and 2.4 s on one core. The
  parse + bind CPU inside it is a minority: 1.29 s of the 3.03 s single-threaded front end. The rest is the include
  glob, ~114k `stat`s, ~40k reads and module resolution. A persisted resolution/file-list cache, or answering
  `file_exists` from cached directory listings (notes/speed-frontend.md), aims at roughly 5-10x the time an AST
  cache can save, needs no AST format, and is independent of pointer compression.
- **Many processes over the same `node_modules`** (a CI running one `tsrs -p` per package, where library parse +
  bind is 55-99% of each small package's parse + bind): run them as one `--build`, which already shares declaration
  files in memory.

**Revisit when:**

- `PSlice` / `PStr` and arena-resident symbol tables land for their own memory reasons. That removes the largest
  effort item.
- A workload appears where library parse dominates at scale, such as many concurrent language servers on one
  machine, where page sharing pays.
