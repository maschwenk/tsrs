# mem-frontend: the front end's memory (parse + bind, `--noCheck`)

Experiment 5 of the checker-memory round (branch `mem/frontend`, base de3beaf). The private monorepo with `--noCheck`
peaked at 2.26 GiB (Go `tsc`: 4.65 GiB for the same phase), 40% of the one-checker peak. Gates: AST oracle and binder
oracle unchanged, suite trees identical in four modes, fourslash pass list identical, private-monorepo output and
counters identical. Nothing here frees or reuses memory, so the census free-gate does not apply.

## Where 2.26 GiB went (base)

Measured with the alloc-profile build (`TSRS_ALLOC_PROFILE_TOP=3000`: arena by type and site) and the heap profiler.
The heap profiler now also prints the live bytes per stack at the heap's high-water mark ("heap live at the heap
peak"): the loader's transient structures are gone at exit but set the peak. Footprint ~= arena requested (1,593 MB)
+ heap at its peak (637 MB counted) + allocator retention.

| what | MB | notes |
| --- | --- | --- |
| AST node headers | 531 | 23.2M nodes x 24 B (kind / tag / parent word, flags, lazy id, range) |
| AST payloads (non-identifier) | ~440 | `CallExpression` 76, `PropertyAssignment` 71, `PropertyAccessExpression` 68, `StringLiteral` 47, `TypeReference` 37, `Parameter` 36, `PropertySignature` 30, `ArrowFunction` 25, ... |
| identifiers | 245 | 7.89M x 32 B (header + one word: text length / text index / flow node); 122K `IdentifierWithText` x 40 |
| source text | 213 | heap, read once (`fs::read`, exact capacity), leaked as the file's text; no second copy (lazy JSDoc parses share it, line maps are lazy) |
| symbols | 181 | 3.94M x 40 B + 0.8M member/export tails x 40 B |
| node lists | 190 | 4.06M `NodeList` x 24 B (97) + `ModifierList` (10) + element slices (54) + symbol declaration slices (3.98M, 35.5) |
| flow nodes | 68 | 1.92M `FlowNode` x 32 B + 632K `FlowList` x 16 B |
| symbol tables | ~65 | 1.1M headers x 24 B in the arena + 8-byte entries on the heap (31 + 9 MB at exit) |
| file loader (transient, at the heap peak) | ~250 | 202,797 `parseTask`s (one per import edge, 40,021 loaded) x 416 B in a doubled `Vec` (~100), per-file program maps and include reasons keyed by cloned path strings (~65), prefetch results (~65), per-file `IndexMap`s (~15) |
| `FileIncludeReason` | 28 | 203K x 144 B, each with its own copy of the referencing file's path |
| module resolution | ~55 | `ResolvedModule` 87K x 152 B (13), resolved / original path strings (17), `cachedvfs` existence caches (19 heap), package.json (1.6 arena + ~5 heap) |
| `SourceFile` structs | 29 | 40K x 768 B |
| binder strings | 7 | external module symbol names `"<path>"` (Go does the same) |
| JSDoc | 6 | eager JSDoc nodes only; TS files parse JSDoc lazily |
| parse diagnostics | 9 | all rolled back by speculation rewinds (reclaimed in normal builds) |

By text, 64% of the program is project source (28.5K files, 137 MB), 32% `.d.ts` under `node_modules` (9.4K files,
68 MB), 3% other `node_modules` files, 2% workspace `dist` `.d.ts`; the libs are ~4 MB. The structures are the same
everywhere (declaration files have more `PropertySignature` / `MethodSignature` / `TypeReference` nodes and fewer flow
nodes); nothing node_modules-specific dominates.

Questions from the brief, answered:

- **Line maps**: lazy (`OnceLock`) as in Go; nothing builds them during parse or bind. `--extendedDiagnostics`' `Lines`
  statistic built every file's map at the end of the run (Go does too: `len(file.ECMALineMap())`), adding ~24 MB at
  the arena's largest. Fixed (step 4).
- **JSDoc parsing mode**: this Go version has no `JSDocParsingMode`; for TS/TSX files `withJSDoc` only flags the node
  and parses lazily unless the comment has `@see` / `@link` (parser.go `SetHasLazyJSDoc`, jsdoc.go); JS files parse
  eagerly. tsrs matches it exactly (`parser_1.rs`, `jsdoc.rs`); 6 MB of JSDoc nodes. Nothing to fix.
- **Interning**: identifier and literal text already points into the source text (`PackedStr`, compact identifiers);
  a global identifier interner was measured and rejected in mem-round2 (+9% parse time). The duplicated strings were
  paths: every task, include reason, program map key and `SourceFileParseOptions` had its own copy (fixed, step 1).
- **Module-resolution caches**: tsrs's `ResolvedModule` keeps no failed-lookup lists (nothing to drop); package.json
  entries are small (1.6 MB).
- **Speculation rewinds**: 6.83M rewinds reclaim 29 MB; 1,130 are skipped because a pin happened (3 MB). The census
  finds 110 MB unreachable at the end, of which 45 MB is what rewinds and the binder's label recycling already give
  back; the rest (~65 MB: identifiers 10, flow nodes 6, parameters 5, binding elements 5, type references 5, ...) is
  scattered.
- **Region trimming** (LSP file regions) is CLI-neutral: the CLI never creates a region.

## Changes

| step | change | `--noCheck` peak (median of 3) |
| --- | --- | --- |
| 1 | loader tasks: loaded-only fields in a lazily allocated box (416 -> 208 B); `FileIncludeReason` stores the referencing path as one arena string per file (144 -> 56 B); `tspath::Path` wraps `Arc<str>`; tasks of one file share path and name; parse options reuse the task's path | 2.265 -> 2.070 GiB |
| 2 | per-file task map: a small vector instead of an `IndexMap`; symbol tables grow 1, 2, 4, ... (485K binder tables hold one symbol, 283K two) | 2.06 GiB (noise); one checker -0.05 GiB |
| 3 | rare AST fields in a tail allocated only when set (`NodeAllocRare`, header bit; generator `RARE_FIELDS`): `?.` 0.1% of 1.24M calls and 2.7% of 1.27M property accesses, call type arguments 0.5%, parameter `...` 1.8% / initializer 1.4%, property signature initializer 0%, ... | 2.052 -> 2.010 GiB |
| 4 | `--extendedDiagnostics` `Lines` counted without building and keeping every line map | (`--extendedDiagnostics` only) 2.043 -> 2.011 GiB |

Every change is representation only: comparisons stay by string value, getters return the same values, capacities are
not observable.

## Result

Interleaved, 3 rounds, base = de3beaf, final = this branch (machine shared, load 15-80):

| run | base peak GiB | final peak GiB | delta | base instructions | final instructions |
| --- | --- | --- | --- | --- | --- |
| `--noCheck` | 2.255-2.276 (2.270) | 2.014-2.019 (2.014) | -0.256 (-11.3%) | 56.4-59.7 G | 55.1-56.3 G |
| one checker | 5.714-5.740 (5.725) | 5.463-5.488 (5.480) | -0.245 (-4.3%) | 308.5-311.0 G | 312.1-318.9 G |
| `--singleThreaded` | 5.613-5.629 (5.616) | 5.424-5.441 (5.436) | -0.180 (-3.2%) | 305.5-308.9 G (307.9) | 307.0-312.4 G (310.6, +0.9%) |
| four checkers | 7.565-7.595 (7.577) | 7.302-7.323 (7.311) | -0.266 (-3.5%) | 422.6-429.0 G | 424.1-427.1 G |
| opt-out, one checker (1 run) | 7.568 | 7.318 | -0.250 | 331.5 G | 336.5 G |
| opt-out, four checkers, go assignment (1 run) | 11.433 | 11.153 | -0.280 | 504.2 G | 503.0 G |

Parse only (`ast_oracle bench 1` over the private monorepo's 38,958 files): 15.546-15.551 G -> 15.495-15.528 G
instructions (-0.2%), heap allocations 1.22M -> 1.14M. Total instructions move within this machine's +-1.5% noise;
the median single-threaded pair is +0.9% (candidates: two extra reallocations for tables that grow past one and two
entries, the rare-tail bit test in getters, `Arc` reference counts on path clones).

Profile after (`--noCheck`): arena 1,593 -> 1,536 MB requested (node payloads 752 -> 697, `FileIncludeReason` 28 ->
11); heap at its peak 637 -> 427 MB counted.

## Gates

- AST oracle (`tools/oracle/ast/run.py`, Rust vs Go): 113/113 libs, 17,318/17,319 split test units (the known non-UTF-8
  file), 38,958/38,958 private-monorepo files identical.
- Binder oracle (`tools/oracle/binder`): private monorepo 38,958/38,958 identical to Go; test corpus 12,750/12,758 with
  the same 8 differences as the base binary (hashes of all 12,758 files identical base vs final).
- Suite with `--baselines types,symbols`: `target/test-results` trees identical to the base (timings in
  `summary.json` aside) in default, `TSRS_LAZY_MEMBERS=0`, `TS_TEST_PROGRAM_SINGLE_THREADED=false` and both (13,458
  error baselines, 12,779 types / symbols). (A first parallel-mode run under load 133 timed out two heavy tests; both
  pass when rerun.)
- Fourslash: 4,066 pass / 63 fail / 417 skip, pass, fail and skip lists identical to the base.
- Private monorepo: diagnostics (16 error lines from stale workspace builds) and `--extendedDiagnostics` counters
  identical for default 1 / 4 checkers and opt-out 1 / 4 checkers (go assignment); `Lines` unchanged.
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked` clean; `cargo test` of tsrs_core, tsrs_ast,
  tsrs_compiler, tsrs_vfs, tsrs_module pass.

## The floor

Of the final 2.01 GiB (2.16 GB), about 1.85 GB is the AST and the binder output as they are defined:

- node headers 531 MB: kind, data tag and parent already share one word; flags (32 bits, all in use), the lazily
  assigned id (checker link stores are keyed by it) and the range remain, and the header must stay a multiple of 8
  (every payload holds pointers). Dropping a field means a side table on a hot path.
- payload fields ~400 MB: child pointers and the binder's symbol / locals / flow-node slots, the same fields as Go's
  structs minus the rare ones moved out here.
- identifiers 245 MB: header plus one word.
- source text 213 MB: kept verbatim, as in Go (diagnostics, identifier text and lazy JSDoc read it).
- symbols 181 MB, node lists 190 MB, flow nodes 68 MB, symbol tables ~65 MB.

Not done (each judged against its size):

- **Cell fields that are almost never set** (`PropertyAssignment` modifiers / postfix token / type: 0% of 1.03M,
  24.7 MB; `FunctionLikeBase.full_signature` 0%; `MethodDeclaration` postfix / asterisk ~0%): the reparser can write
  them after construction (JS files), so a tail allocated at construction does not work; it would need a side table
  plus a second header bit. ~35 MB for a new invariant on writes; the next candidate if more is wanted.
- **Single-declaration symbols** (3.1M one-element declaration slices, ~25 MB): storing the node inline and returning
  a slice into the symbol would let a held `declarations()` slice change under its holder after an append (Go's old
  slice stays valid). Rejected.
- **NodeList packing** (24 -> 20 bytes, or elements inline): the bump allocator's 8-byte alignment eats the 4 bytes;
  inline elements change the public `nodes` slice field (312 sites) and sentinel-slice identity. ~30 MB at most.
- **Smaller loader / resolver leftovers**, each ~10-20 MB: `parseTask.package_id` (64 B per task), the per-file
  `SourceFileParseOptions.file_name` copy (9 MB), resolved-path strings in `ResolvedModule` (17 MB), `cachedvfs`
  existence-probe keys (19 MB, needed for the cache).
- **Deduplicating packages at load time** (983 files parsed and dropped, 13 MB of text): changes Go's algorithm
  (notes/mem-recycle.md).
- Flow nodes from conditions that end up unreferenced: not provable locally (notes/mem-recycle.md).

## Reproducing

- `--features alloc-profile` build, `TSRS_ALLOC_PROFILE_TOP=3000` for the arena tables, `TSRS_HEAP_PROFILE=1
  TSRS_HEAP_PROFILE_TOP=5000` for the heap; the new "heap live at the heap peak" section is the one to read for
  transient structures. Inlined frames: `dsymutil` the binary and `atos -i` (the profiler prints one frame per site).
- Field occupancy (the rare-field percentages above) came from a throwaway walk over the bound program (every node of
  the listed kinds, counting non-nil fields); not committed.
