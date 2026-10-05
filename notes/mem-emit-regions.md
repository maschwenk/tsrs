# mem-emit-regions: per-file scratch regions for emit

Branch `mem/emit-regions`, base origin/main 494b560. Goal: emit should need no more memory than checking, and
`webpack --declaration` must not run out of the 32 GiB compressed-pointer range (it aborted after ~50 s on base).
Output and diagnostics stay byte-identical.

## Result

`--noEmit false --incremental false --declaration --sourceMap` (webpack JS-only: no `--declaration`), every output
under `scratch/`, every run under `sandbox-exec` with a profile that denies writes to the corpora. Median of
interleaved runs (38k: 5, vscode: 7, the others: 3; tsgo on the 38k-file codebase and webpack `--declaration`: 1-2
runs). Peak = maximum RSS. The machine was shared at load 20-40 throughout, so walls carry a few percent of noise.

| corpus | wall base / new / tsgo (s) | emit base / new / tsgo (s) | peak base / new / tsgo (GiB) | check-only peak, new (GiB) |
| --- | --- | --- | --- | --- |
| the 38k-file codebase | 12.34 / 12.39 / 56.5 | 3.78 / 3.76 / 31.9 | 6.98 / **6.34** / 34.3 | 5.81 |
| vscode `src` | 4.03 / 4.08 / 10.03 | 1.33 / 1.36 / 4.68 | 3.06 / **2.79** / 11.69 | 2.30 |
| webpack, JS only | 0.63 / 0.62 / 1.16 | 0.31 / 0.31 / 0.41 | 0.49 / 0.48 / 1.30 | |
| webpack `--declaration` | aborts at 50.9 s / **120.7, 128.1** / 149.6, 154.8 | — | 39.2 at abort / **17.3, 17.1** / 49.0, 50.5 | |
| xstate | 0.09 / 0.09 / 0.20 | 0.04 / 0.04 / 0.09 | 0.18 / 0.18 / 0.42 | |

- **webpack `--declaration` completes.** Its 2,747 output files and 965 diagnostics are identical to tsgo-ref's
  (`TSRS_CHECKER_ASSIGNMENT=go`, 163.8 s, 16.5 GiB). Base wrote 2,308 files before the abort.
- **The 38k-file codebase:** emit adds 0.53 GiB over a check-only run, down from 1.17 GiB. Before, its
  `--noEmit --declaration` run alone took 6.56 GiB, because the declaration-diagnostics transform's garbage was kept.
  Now that run takes 5.91 GiB. Where the remaining 0.53 GiB goes is listed under "What remains".
- **Instructions retired:** +0.1% to +0.9% (38k 619.8 G -> 624.0 G, vscode 252.9 G -> 255.3 G). Between rounds the
  same binary varies by about ±0.5%.
- **Emit time:** vscode median +2.1%, minimum equal (1.291 -> 1.286 s). 38k median -0.6%.

## 1. Where emit's memory went (base)

The tooling: `TSRS_EMIT_MEM=1` prints, for each file, the arena bytes used in its region and outside it, plus the
reserve's use. The plain-ptrs alloc-profile build gives per-site tables, and a local label that is not committed
marked allocations made outside the scratch region while it was entered.

**The 38k-file codebase (27,503 emitted sources):**

| run | peak (GiB) |
| --- | --- |
| `--noEmit`, no `--declaration` | 5.80 |
| `--noEmit --declaration` | 6.56 |
| full emit | 6.98-7.01 |

- **`--noEmit --declaration` adds 0.76 GiB.** tsc computes declaration diagnostics whenever declarations are on,
  even with `--noEmit`. That runs a whole declaration transform per file, and all of it was kept.
- **During emit, the arena grew by 646 MB** (JS transform 106 MB, declaration transform 480 MB, printing 60 MB).
  The rest is heap:
  - Up to 6,997 transformed files waited for the print workers, because writing is the slow stage. Each waiting file
    held its emit contexts' tables.
  - Raw source maps stay in the `EmitResult` until the end, as in Go. They cost 0.15 GiB: the JS-only peak is 6.04
    with maps and 5.89 without.

**vscode:** check-only with declarations 2.30 GiB, emit 3.05 GiB. The arena delta was 366 MB, of which 250 MB was
factory nodes, and up to 1,710 files waited for printing.

**webpack `--declaration`:**

- **26 files take 0.65-3.6 GB each in the declaration transform.** Their printed output is tiny or absent: most fail
  with TS7056 (23 errors) or TS4094 (95), which block their `.d.ts`. One file
  (`lib/dependencies/esm/ESMAcceptImportDependency.js`) allocated 3.9 GB:

  | allocation | count | size |
  | --- | --- | --- |
  | `NodeBuilderContext` | 3.49M | 1.41 GB |
  | identifier nodes | 9M | 344 MB |
  | `recoveryBoundary` | | 274 MB |
  | strings | | 210 MB |
  | `TrackedSymbolArgs` | | 181 MB |
  | `SymbolTrackerImpl` | | 175 MB |
  | `PseudoType` | | 160 MB |
  | `ImportTypeNode` | | 150 MB |

- **The 3.49M contexts come from `is_symbol_accessible_worker`.** For every inaccessible result it builds
  `ErrorSymbolName` eagerly with `symbol_to_string_ex`. This happens while the node builder serializes inferred types
  of `Template` class expressions and other JS members. Those types reference the `Dependency`/`Module` classes,
  which each have hundreds of members, so each declaration builds about 1,000,000 characters of nodes. At that point
  the elision limit (`noTruncationMaximumTruncationLength`) stops it, and TS7056 follows. Every bad file goes through
  this twice: once in the declaration-diagnostics pass and once in emit.
- **Go does the same work.** tsgo-ref's `--pprofDir` memory profile shows 170.7 GB allocated over the run:
  - 99% is under `serializeTypeForDeclaration`.
  - 40% (70 GB) is under `symbolToStringEx` called from `isSymbolAccessibleWorker`.
  - The top sites are `printer.NewPrinter` (22.7 GB, one printer per `symbolToString`), `NodeBuilder.enterContext`
    (21.2 GB) and `getExistingNodeTreeVisitor` (12.3 GB).

  Its 49-50 GiB peak is that garbage before the GC collects it. No dropped Go cache is involved. The node builder's
  `serializedTypes` cache is ported and used as in Go, and every request builds a new node builder in Go too
  ("TODO: cache per-context").

## 2. Design: scratch regions

Each file's emit, and each file's declaration-diagnostics transform, gets a region:
`Region::new_scratch` + `enter_scratch` (`tsrs_core::arena`). The region is freed when the file has been printed and
written, or once its diagnostics are collected.

**What the region does:**

- **It is the allocation target in emit code.** The transformers, emit contexts, the printer, source-map scratch,
  the emit host and the emitter all allocate there.
- **Allocations that must outlive the file escape.** `arena::escape_scratch()` makes the target that was current
  when the region was entered the allocation target again: the thread arena, or an enclosing API region. It is
  called in three places:
  - **`CheckerSlot::with`.** Every emit -> checker call goes through it (`Resolver::lock`, the node builder's
    visitor callbacks). Types, symbols, signatures, links, mappers and checker caches therefore stay in the checker's
    arena.
  - **Every `EmitHost` method that reaches the program.** These fill program caches: package.json entries, symlinks,
    the exports-name memo.
  - **The diagnostic constructors and mutators** in `tsrs_ast::diagnostic`. Diagnostics are results, and the
    declaration-diagnostics cache keeps them.
- **Some allocations go to the region even from escaped checker code.** These use `P::new_scratch` /
  `alloc_*_scratch`:
  - **The nodes of a per-file emit context's factory.** That factory is created with `NodeFactory::scratch` set,
    through `new_scratch_emit_context`. The emit resolver's request node builders use it, so the nodes they build
    die with the file.
  - **The node builder's per-call state.** That covers `NodeBuilderContext`, `SymbolTrackerImpl`, `recoveryBoundary`,
    `wrappingTracker`, `SignatureToSignatureDeclarationOptions`, `SymbolAccessibilityResult` and pseudo types. It is
    dead after `exit_context` and is never cached.
  - **A request builder itself:** its `NodeBuilderImpl`, its link stores (`LinkStore::new_scratch`), its
    `id_to_symbol` map, and the text of the nodes it makes (`NodeFactory::alloc_text`).
  - **A `GoMap` table, when the map field itself is in the region.**
- **`TrackedSymbolArgs` is now a value.** It was an arena allocation in Go style, but nothing compares it by
  identity.
- **Scratch regions are not in the region registry.** Creating or freeing one takes no global lock, and a CLI
  process still has no registered region, so `enter_owner` / `enter_table_owner` stay a no-op there. To keep lazily
  filled data of an object outside the scratch region out of it, those two functions escape the scratch region
  unless the object lies inside it. That covers `SourceFile` line maps, the JSDoc cache and the package.json cache
  entries.
- **Backpressure:** at most 2,048 transformed files wait for printing (see the note on `EMIT_MAX_PENDING_PRINTS`).
- **Released 1 MiB slabs are kept for reuse.** Up to 64 stay committed instead of being decommitted at once; short
  regions churn slabs.

### Escape analysis (census, plain-ptrs alloc-profile build)

`TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1` records each freed region as would-free and marks from the program, the config
and the result diagnostics. Plain mode is needed: link-store keys and hash keys are addresses there, so the census
also sees keys. The first run on xstate found 84 violations, in four classes, all fixed:

1. **Request builders kept their link values and `GoMap` tables in the checker's arena.** The tables referred to
   scratch `SerializedTypeEntry`s and specifier strings. Fix: scratch link stores, and `GoMap` tables live where the
   field lives.
2. **`EmitResolver::jsx_links` is keyed by synthesized JSX factory names.** It is set by the JSX transform; values
   are scratch `ImportSpecifier` nodes.
3. **`ContainingSymbolLinks::accessible_chain_cache` entries can be keyed by a synthesized scope node.** The
   declaration transformer calls `is_symbol_accessible` with an enclosing declaration inside synthesized trees.

   Classes 2 and 3 are more than dangling pointers. Once the region is freed, its handles name new nodes, so a later
   lookup could hit a stale entry and return a wrong result. Go keys these maps by pointer and its GC keeps the nodes
   alive, so such an entry is never hit again there either. Fix: `Checker::forget_scratch_keyed_caches`, called when
   a file's transform ends. It clears `jsx_links` (every key is a synthesized name) and removes the recorded
   accessible-chain entries whose scope lies in the region (`arena::scratch_contains`).
4. **Stale words in hash tables after removals.** Census builds rebuild the touched tables; this is census-only.

After the fixes the census shows 0 violations:

| corpus | regions freed | arena blocks in them | precise walk |
| --- | --- | --- | --- |
| xstate | 343 | 178,662 | 0 |
| vscode | 13,335 | 6,843,099 | 0 references into freed blocks (40.35M checked) |

Both runs used the final design, with unregistered regions. The 38k-file census was not run: it needs about 35 GB on
a shared machine. Poison mode covers it instead.

`TSRS_ARENA_POISON=1` (already on main for regions) fills freed region memory with `0xA5` and never reuses it. Under
it, output and diagnostics are identical to base:

- xstate, vscode and the 38k-file codebase.
- The js/jsmap/sourcemap baselines, both single-threaded and with `TS_TEST_PROGRAM_SINGLE_THREADED=false`.
- webpack `--declaration` until the never-reused range runs out (abort at 36 s): the 2,328 files it wrote are
  identical to tsgo's.

**What stays outside the region during emit:**

- The 38k-file codebase: 54 MB over all files. These are checker data: link chunks, types, mappers.
- webpack `--declaration`: 4.8 GB over the run, about 300 MB per bad file. It is the identifier nodes and their text
  that `symbol_to_string` builds with the checker's cached `type_to_string` node builder. That builder's
  `serializedTypes` cache and `id_to_symbol` map keep its nodes, so they cannot go to the region. Go's cached builder
  keeps the same nodes alive through `idToSymbol`.

**Address range:** webpack `--declaration` allocated 45.8 GB in emit regions over the run (declaration-diagnostics
regions come on top). The reservation's high-water mark is 11.2-11.6 GiB, and 5.1 GiB was in use at the end: freed
ranges are reused. A single bad file's region peaks at 3.1 GB, and four checkers emit at once.

## 3. Gates

Base is 494b560 (binaries copied before any change); head is the branch.

- **Conformance**, `--baselines types,symbols`, default and `TSRS_LAZY_MEMBERS=0`: 13,458 pass, 2 codes, 2 fail;
  12,779 / 12,779 `.types` / `.symbols`. Result trees are identical to base; `summary.json` differs only in `ms`.
- **`--baselines js,jsmap,sourcemap`:** 13,392 / 149 / 156 pass, 0 fail. Trees are identical to base in four modes:
  single-threaded, `TS_TEST_PROGRAM_SINGLE_THREADED=false`, and both again under `TSRS_ARENA_POISON=1`.
- **Fourslash:** 4,066 pass, 63 fail, 417 skip. Result tree identical to base.
- **`cargo test -p tsrs_cli`:** green, with `api::memory_tests` cfg'd out locally (not committed).
  - tsctests: 374 / 32 / 1, the same lists as base. Of the result files, only `internal-symbolname-in-tsbuildInfo`
    differs, and it is in fail.txt on both sides (run-order symbol ids).
- **`RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked`:** clean, in both the compressed and the
  `tsrs_core/plain-ptrs` mode.
- **Emit oracle over conformance cases, tsgo vs tsrs** (`tools/oracle/emit/cases.py`):
  - compiler: 6,537 pass, 1 fail. `regexInvalidUtf8WithUnicodeFlag` fails on base too.
  - conformance: 6,449 pass, 0 fail.
- **Monorepo emit oracle** (`monorepo.sh` without the git guard, run under the no-write sandbox): 101/101 packages,
  10,810 files identical to tsgo.
- **Corpora:** output trees and diagnostics are identical to base on xstate, vscode and the 38k-file codebase.
  webpack `--declaration` is identical to tsgo-ref (above).

## 4. Tried and rejected

- **Region as the target everywhere, with persistent checker allocations re-routed site by site.** This is the
  inverse design. Every checker cache and allocation site would have to be found: types, symbols, link chunks, slices
  and strings that end up in caches. Missing one is a use-after-free that output comparison may not catch. The design
  used instead escapes at the emit -> checker boundary and opts in only node-builder state. Its escape surface is
  that opt-in list plus emit-side code, and the census checks both.
- **A per-file instance of the checker's `type_to_string` builder during emit.** It would move the 4.8 GB webpack
  residue into regions. But `serializedTypes` is not transparent: a hit returns a clone of a result built at another
  depth, with its elisions. With a fresh builder, error-message text could differ from Go in rare cases. A variant
  that switches the shared builder's factory to scratch outside cache-eligible serializations was also rejected: a
  cached result can embed a node made earlier in the same call.
- **Building `ErrorSymbolName` lazily.** This would remove about 3.5M `symbol_to_string` calls per bad webpack file.
  But it changes when side-effecting serialization runs (caches it fills, entries it adds), so it is not exact by
  construction. Go does the eager work.
- **Dropping `EmitResult.source_maps` in the CLI.** It would save about 0.15 GiB on the 38k-file codebase, but Go
  keeps them, and they are part of the emit API's result.
- **Registered scratch regions.** This was the first version. Every region and chunk took the global registry write
  lock, and once a region existed every `enter_owner` / `enter_table_owner` did a registry lookup.
- **Smaller print backlogs.** With a bound of 256, the 38k peak was 6.21 GiB, but vscode emit was about 8% slower
  (minimum of 8 rounds): the checker threads stall. Peaks:

  | bound | 38k peak (GiB) | vscode peak (GiB) |
  | --- | --- | --- |
  | 256 | 6.21 | |
  | 2,048 | 6.35 | 2.79 |
  | unbounded | 6.59 | 2.80 |

## What remains

- **The 38k-file codebase still adds 0.53 GiB over check-only.**
  - Raw source maps kept in the `EmitResult`, as in Go: 0.15 GiB.
  - Up to 2,048 files waiting for the print workers: about 0.15 GiB.
  - Declaration-diagnostics leftovers: about 0.1 GiB (5.91 vs 5.81 GiB with `--noEmit`).
  - The 54 MB of checker data that emit computes.
  - mimalloc's retained free pages.
- **webpack `--declaration` takes 120-128 s against tsgo's 150-155 s, at 17 GiB against 49-50 GiB.**
  - It still allocates about 46 GB in emit regions, as much again in declaration-diagnostics regions, and 4.8 GB
    outside them. This is the pathological Go algorithm, faithfully ported.
  - With Go's checker assignment the longest checker thread gets more bad files: 163.8 s.
- **The census was not run on the 38k-file codebase** (memory). Poison mode and output identity cover it.
- **Poison mode cannot run webpack `--declaration` to the end,** because freed memory is never reused there.

## Reproducing

```sh
cargo build --release -p tsrs_cli                                          # compressed (default)
scratch/run1.sh <tsrs> <xstate|vscode|webpack|olympus> <tag> [flags]      # adds --declaration --sourceMap,
#   --outDir/--declarationDir/--tsBuildInfoFile under scratch/out/<tag>, --rootDir <corpus root>, runs under
#   /usr/bin/time -l sandbox-exec -f scratch/nowrite.sb (denies writes to bench-cache and the monorepo copy);
#   DECL=0 drops --declaration, TIMEOUT=<s> caps the run
TSRS_EMIT_MEM=1 <tsrs> ...                                                  # per file: region bytes, outside bytes;
                                                                            # at the end: reserve in use / high water
CARGO_TARGET_DIR=target/prof cargo build --release -p tsrs_cli --features alloc-profile,tsrs_core/plain-ptrs
TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1 <prof tsrs> ...                          # would-free violations, precise walk
TSRS_ARENA_POISON=1 <tsrs> ...                                              # freed regions filled, never reused
tsgo-ref ... --pprofDir <dir>; go tool pprof -sample_index=alloc_space -top <dir>/*-memprofile.pb.gz
```
