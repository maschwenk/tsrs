# mem-checker-heap: the memory each checker holds outside the arena

Three memory rounds shrank the arena objects (types, symbols, nodes). This round looks at what each checker allocates
with malloc (mimalloc): caches, link-store tables, symbol-table entries, per-type maps, vector slack. Representation
only: what is cached, and when, does not change. Goal: -5 to -10% peak at 8 checkers without more instructions.

Corpus: the 38k-file codebase (workspace packages unbuilt, 40,543 errors), Linux x86-64 sandbox (18 vCPUs, 47 GiB),
origin/main 8f3ad97, `--noEmit --incremental false --extendedDiagnostics`.

## Tools (this change)

- `TSRS_HEAP_CENSUS=1` with `--extendedDiagnostics` (any build; `crates/tsrs_checker/src/heapcensus.rs`): after the
  statistics, one Markdown table per checker on stderr listing every hash table and vector the checker owns directly
  (its ~70 cache maps, the five relation caches, the 27 link stores split into slot table / page vector / dense and
  sparse pages, the lazy member and lazy mapped tables with what each owns, the printer's export indexes, the flow
  memo), with containers, entries, capacity, bytes per slot, load factor and heap bytes (hashbrown's layout: buckets x
  slot rounded to 16, plus control bytes; requested bytes, before mimalloc's size classes). `TSRS_HEAP_CENSUS_MIN`
  (bytes, default 1 MiB) is the smallest row printed. Nothing runs unless the variable is set.
- The alloc-profile heap sampler (`--features alloc-profile`, `TSRS_HEAP_PROFILE=1`) now works on Linux (symbols from
  `addr2line` over the `release` profile's line tables, inlined frames included; `/proc/self/maps` for the load
  address, data segments and `pthread_getattr_np` for the stack, so the census code also links there),
  `TSRS_HEAP_PROFILE_RATE=<bytes>` sets the sampling interval (default 256 KiB), and `TSRS_HEAP_PROFILE_TSV=<file>`
  writes every sampled stack per thread group (`main` = the CLI's `tsrs` thread, `checker-N`, `other` = parse and
  rayon threads) with its live, at-peak and cumulative bytes; stderr gets the per-group totals. This attributes the
  containers that arena objects own (conditional-type instantiation maps, member symbol tables, ...), which the
  census cannot reach. Containers that are folded into one function by the linker (identical generic code) show the
  wrong generic parameters in the innermost frames; the callers are right.

## Where a checker's memory is

| | 1 checker | 8 checkers (per checker) |
| --- | --- | --- |
| peak RSS (`/usr/bin/time`, release build) | 4.71 GB | 7.92 GB total |
| front end alone (`--noCheck`): RSS / arena chunks / heap | 1.98 GB / 1,541 MB / 302 MB | same, once |
| checker arena chunks (all arenas minus the front end's) | 1,704 MB | 441 MB |
| checker heap (heap at exit minus the front end's) | 1,023 MB | 297 MB |
| of which: containers the checker owns (census) | 679 MB | 185 MB (171-220) |
| of which: containers owned by arena objects and the rest (sampler) | ~345 MB | ~110 MB |

So an extra checker costs ~0.46 GB at 8 checkers: 60% arena, 40% heap. The heap part is what this round attacks.

## Ranked table: containers the checker owns (census), 1 checker

Rows of at least 1 MiB. `containers` > 1 means a group (pages, per-table boxes, inner maps).

| owner | entries/checker | capacity/checker | B/slot | load | MB/checker | MB all |
| --- | --- | --- | --- | --- | --- | --- |
| lazy member tables (Rc boxes) | 856,076 | 856,076 | 168 | 1.00 | 137.2 | 137.2 |
| object_type_instantiations (inner maps) | 1,958,256 | 2,660,629 | 24 | 0.74 | 72.6 | 72.6 |
| relation assignable (pairs) | 4,401,536 | 7,340,032 | 8 | 0.60 | 72.0 | 72.0 |
| intersection_types | 1,167,256 | 1,835,008 | 24 | 0.64 | 50.0 | 50.0 |
| value_symbol_links (dense pages) | 9,671,879 | 10,665,984 | 4 | 0.91 | 40.7 | 40.7 |
| cached_types | 939,685 | 1,835,008 | 12 | 0.51 | 26.0 | 26.0 |
| union_types | 774,754 | 917,504 | 24 | 0.84 | 25.0 | 25.0 |
| mapped_symbol_links (slots) | 954,287 | 1,835,008 | 8 | 0.52 | 18.0 | 18.0 |
| signature_links (slots) | 1,580,656 | 1,835,008 | 8 | 0.86 | 18.0 | 18.0 |
| lazy_member_tables | 856,076 | 915,895 | 16 | 0.93 | 17.0 | 17.0 |
| lazy member tables (declared symbol tables) | 1,191,369 | 1,312,047 | 8 | 0.91 | 16.8 | 16.8 |
| cached_signatures | 291,917 | 458,752 | 32 | 0.64 | 16.5 | 16.5 |
| string_literal_types | 311,789 | 458,752 | 32 | 0.68 | 16.5 | 16.5 |
| symbol_node_links (dense pages) | 3,866,755 | 4,052,992 | 4 | 0.95 | 15.5 | 15.5 |
| lazy member tables (unaffected names) | 822,298 | 822,298 | 16 | 1.00 | 12.5 | 12.5 |
| string keys (string_literal_types, undefined_properties, unresolved_symbols) | 12,904,839 | 12,904,876 | 1 | 1.00 | 12.3 | 12.3 |
| union_of_union_types | 127,446 | 229,376 | 40 | 0.56 | 10.3 | 10.3 |
| relation identity (pairs) | 607,229 | 917,504 | 8 | 0.66 | 9.0 | 9.0 |
| symbol_reference_links (slots) | 718,634 | 917,504 | 8 | 0.78 | 9.0 | 9.0 |
| type_node_links (slots) | 897,129 | 917,504 | 8 | 0.98 | 9.0 | 9.0 |
| discriminated_contextual_types | 217,799 | 229,376 | 24 | 0.95 | 6.3 | 6.3 |
| indexed_access_types | 154,789 | 229,376 | 24 | 0.67 | 6.3 | 6.3 |
| module_export_index | 278,668 | 557,176 | 32 | 0.50 | 6.2 | 6.2 |
| members_and_exports_links (slots) | 285,523 | 458,752 | 8 | 0.62 | 4.5 | 4.5 |
| node_links (slots) | 268,734 | 458,752 | 8 | 0.59 | 4.5 | 4.5 |
| relation strict_subtype (pairs) | 332,194 | 458,752 | 8 | 0.72 | 4.5 | 4.5 |
| relation subtype (pairs) | 261,360 | 458,752 | 8 | 0.57 | 4.5 | 4.5 |
| structured_type_base_constraints | 312,802 | 458,752 | 8 | 0.68 | 4.5 | 4.5 |
| flow_loop_cache | 76,883 | 114,688 | 32 | 0.67 | 4.1 | 4.1 |
| lazy mapped tables (members maps) | 41,382 | 81,127 | 32 | 0.51 | 3.4 | 3.4 |
| exports_by_target_index | 37,378 | 57,344 | 48 | 0.65 | 3.1 | 3.1 |
| lazy member tables (ordered properties) | 601,759 | 601,759 | 4 | 1.00 | 2.3 | 2.3 |
| alias_symbol_links (slots) | 206,776 | 229,376 | 8 | 0.90 | 2.3 | 2.3 |
| flow_node_reachable | 190,667 | 229,376 | 8 | 0.83 | 2.3 | 2.3 |
| subtype_reduction_cache | 36,043 | 57,344 | 32 | 0.63 | 2.1 | 2.1 |
| lazy mapped tables (Rc boxes) | 18,359 | 18,359 | 96 | 1.00 | 1.7 | 1.7 |
| array_literal_links (slots) | 104,781 | 114,688 | 8 | 0.91 | 1.1 | 1.1 |
| deferred_symbol_links (slots) | 58,865 | 114,688 | 8 | 0.51 | 1.1 | 1.1 |
| properties_types | 53,079 | 57,344 | 16 | 0.93 | 1.1 | 1.1 |

## Ranked table: containers the checker owns (census), 8 checkers

Per checker: mean over the 8 checkers (entries and capacity) and the sum. Rows of at least 1 MiB per checker.

| owner | entries/checker | capacity/checker | B/slot | load | MB/checker | MB all |
| --- | --- | --- | --- | --- | --- | --- |
| lazy member tables (Rc boxes) | 215,934 | 215,934 | 168 | 1.00 | 34.6 | 276.9 |
| value_symbol_links (dense pages) | 2,246,487 | 6,738,304 | 4 | 0.33 | 25.7 | 205.5 |
| relation assignable (pairs) | 1,551,155 | 2,064,384 | 8 | 0.75 | 20.2 | 162.0 |
| object_type_instantiations (inner maps) | 513,492 | 737,194 | 24 | 0.70 | 20.1 | 161.2 |
| intersection_types | 318,367 | 487,424 | 24 | 0.65 | 13.3 | 106.3 |
| union_types | 202,733 | 258,048 | 24 | 0.79 | 7.1 | 56.6 |
| symbol_node_links (dense pages) | 563,976 | 1,717,504 | 4 | 0.33 | 6.6 | 52.5 |
| module_export_index | 278,668 | 557,176 | 32 | 0.50 | 6.2 | 49.6 |
| lazy_member_tables | 215,934 | 286,327 | 16 | 0.75 | 5.3 | 42.2 |
| mapped_symbol_links (slots) | 262,632 | 430,080 | 8 | 0.61 | 4.2 | 33.8 |
| cached_signatures | 69,448 | 114,688 | 32 | 0.61 | 4.1 | 32.8 |
| lazy member tables (declared symbol tables) | 284,442 | 317,319 | 8 | 0.90 | 3.9 | 31.5 |
| cached_types | 218,356 | 258,048 | 12 | 0.85 | 3.7 | 29.6 |
| string_literal_types | 73,226 | 100,352 | 32 | 0.73 | 3.6 | 28.8 |
| signature_links (slots) | 226,687 | 329,728 | 8 | 0.69 | 3.3 | 26.0 |
| lazy member tables (unaffected names) | 209,744 | 209,744 | 16 | 1.00 | 3.2 | 25.6 |
| exports_by_target_index | 37,373 | 57,344 | 48 | 0.65 | 3.1 | 24.8 |
| union_of_union_types | 34,009 | 57,344 | 40 | 0.59 | 2.6 | 20.8 |
| type_node_links (slots) | 178,955 | 229,376 | 8 | 0.78 | 2.3 | 18.4 |
| string keys (string_literal_types, undefined_properties, unresolved_symbols) | 2,360,618 | 2,360,622 | 1 | 1.00 | 2.3 | 18.1 |
| symbol_reference_links (slots) | 121,293 | 200,704 | 8 | 0.60 | 2.0 | 16.0 |
| indexed_access_types | 45,978 | 64,512 | 24 | 0.71 | 1.8 | 14.3 |
| discriminated_contextual_types | 30,753 | 50,176 | 24 | 0.61 | 1.4 | 11.2 |
| alias_symbol_links (slots) | 66,532 | 114,688 | 8 | 0.58 | 1.1 | 8.8 |
| structured_type_base_constraints | 86,484 | 114,688 | 8 | 0.75 | 1.1 | 8.8 |

## Containers owned by arena objects (sampler, live bytes at exit by allocating function, 8 checkers, per checker)

Sampled every 64 KiB; heap blocks of the checker threads that the census rows above do not cover, grouped by the
first two checker frames above the container code.

| MB/checker | 1 checker MB | container | owner |
| --- | --- | --- | --- |
| 25.3 + 1.0 | 63.3 | `ConditionalRoot.instantiations` (`GoMap<CacheHashKey, P<Type>>`, 24-byte slots) | `getConditionalTypeInstantiation` |
| 6.2 + 4.1 + 2.9 + 1.0 | 25.1 + 22.2 + 13.3 + 11.7 | resolved member symbol tables (`SymbolTable` entry buffers, 8 B/entry, doubling from 4) | `resolve_lazy_members`, `add_inherited_members`, `resolve_object_type_members` |
| 4.3 + 2.0 + 2.0 | 20.7 + 10.0 + 5.6 | union/intersection property caches (symbol tables) | `get_union_or_intersection_property` |
| 4.5 + 3.4 | 11.6 | mapped-type member tables | `resolveMappedTypeMembers` |
| 4.4 + 4.2 + 2.8 + 1.5 | 8.7 + 17.1 + 4.6 | lazy declared-member tables and instantiated-symbol value links pages | `get_lazy_declared_member`, `new_instantiated_symbol` |
| 3.8 + 1.9 | 20.9 + 10.0 | object-literal member tables of expression re-checks | `check_expression_worker` |
| 2.6 | | signature instantiation maps (`GoMap<CacheHashKey, P<Signature>>`) | `getSignatureInstantiation` |
| 1.0 - 1.6 each | | id-link-store dense pages (4 KiB, allocated wherever a link is first made) | `get_type_of_alias`, ... |
| 1.4 | 20.5 | the node builder's emit-flag map (`EmitContext`), kept with the checker's `type_to_string` builder | `add_emit_flags` |
| | 26.1 | `cached_types` (also in the census) | |

Per-object container counts are not available from the sampler; the census rows cover the checker-level tables
exactly.

## Reading the tables: candidates

Largest first, at 8 checkers (per checker / total):

1. **Lazy member tables, 40 MB / 320 MB** (34.6 MB of `Rc<LazyMemberTable>` boxes + a 16-byte-slot map + what the
   tables own). Each table is a 168-byte heap block (152-byte struct + the `Rc` counts), which mimalloc rounds to
   192: four 16-byte slices, a 16-byte boxed name list, a 24-byte `OnceCell<Vec>`. 856K tables live with one checker
   (981K created, 125K dropped when resolved in full).
2. **`CacheHashKey`-keyed maps, ~75 MB / ~600 MB**: conditional-root instantiations 26, `object_type_instantiations`
   20, `intersection_types` 13, `union_types` 7, `indexed_access_types` 2, signature instantiations 3, ... Each slot is
   a 16-byte hash plus a 4-byte handle padded to 24.
3. **Id-keyed link stores' dense pages, 32 MB / 260 MB at load 0.33** (`value_symbol_links` 25.7, `symbol_node_links`
   6.6; one checker: load 0.91-0.95). The sparse form exists behind `TSRS_SPARSE_ID_PAGES`.
4. **Relation caches, 20 MB / 162 MB** (assignable; 8-byte slots already, load 0.75): nothing left in the
   representation short of a different table design.
5. **The printer's export indexes, 9-16 MB / 75-130 MB**, identical in every checker (`module_export_index`,
   `exports_by_target_index`: maps whose values are `Vec`s).
6. **String-keyed literal cache, 6 MB / 47 MB** (`string_literal_types`: 32-byte slots with a `String` key, plus the
   key text, which the literal type also holds).
7. Smaller: `cached_signatures` (32-byte slots), `union_of_union_types` (40), lazy tables' name lists (16 B/name).

## What was done (measured on origin/main 68997ed)

Six representation changes, each exact (diagnostics byte-identical in every run; `--extendedDiagnostics` Types /
Symbols / Instantiations identical single-threaded; conformance and fourslash result trees identical to main's,
`tools/regressions.sh` 8/8). Interleaved runs of `dist` builds (fat LTO, no PGO) on the 38k-file codebase, 5 rounds,
medians; "step" is against the previous row. Instructions: `perf stat -e instructions:u`, `--singleThreaded`; the
non-PGO column moves by up to +-0.4% between sessions for the same change (LTO inlining shifts: whole functions move
in and out of their callers), so decisions use the PGO column (builds trained as `.github/workflows/release.yml`
does, run without `--extendedDiagnostics`).

| change | peak, 1 checker | peak, 8 checkers | single-threaded peak | instructions, PGO | instructions, no PGO |
| --- | --- | --- | --- | --- | --- |
| main | 4.485 GiB | 7.567 GiB | 4.205 GiB | 210.99 G | 249.25 G |
| 1. lazy member tables in the arena, one-word slices | -1.29% | -1.65% | -1.26% | -0.05% | -0.11% |
| 2. `CacheHashKey` maps in packed 20-byte slots (`PackedMap`) | -0.70% | -1.76% | -0.99% | | +0.18% |
| 3. string literal cache stores only the types | -0.61% | -0.48% | -0.54% | | +0.03% |
| 4. printer export indexes without a `Vec` per target | -0.05% | -1.76% | -0.46% | (2-4 together) -0.09% | -0.02% |
| 5. narrow (16-bit) id-link-store pages | -0.69% | -0.95% | -0.64% | +0.13% | +0.41% |
| **1-5 against main** | **-3.30%** | **-6.44%** | **-3.83%** | **-0.004%** | +0.49% |
| (rejected) 6. symbol-table position index in 8/16 bits | -1.38% | -1.51% | -1.41% | +0.39% | +0.20% |

Wall time (medians, s): 8 checkers 13.56 -> 13.41 (1-5: 13.71), one checker 49.6 -> 50.1, single-threaded 54.5 ->
54.3: no change beyond this host's spread. The PGO pair main / 1-6 at 8 checkers: 7.572 -> 6.947 GiB (-8.3%),
wall 12.07 -> 11.59 s.

Change 6 is kept out: it saves 1.4-1.5% but costs +0.39% instructions with PGO, above the 0.3% bar. Where the cost
comes from is not established (the 8-bit search is inline, the wider ones out of line); branch
`heap/symtab-index` (on top of this stack) has it for a follow-up.

Rejected without a branch: sparse id pages by default for multi-checker CLI runs (`TSRS_SPARSE_ID_PAGES=1` on main:
-2.4% peak at 8 checkers, -1.2% at 4, but every lookup becomes a rank computation; +1.2% instructions at 8 checkers
in notes/mem-shared-base.md, not re-measured here at 8 checkers with `perf`).
Status (2026-10-10): `TSRS_SPARSE_ID_PAGES` no longer does anything: notes/mem-64.md replaced the id pages with
128-id groups and removed the sparse form; the switch is ignored (crates/tsrs_checker/src/links.rs).

## What remains per checker (8 checkers, after 1-5)

Heap per checker drops from ~297 MB to ~240 MB. The largest rows left, none of which has a cheap exact
representation change: the assignable relation cache (20 MB, already 8-byte slots at load 0.73), conditional-root
and object-type instantiation maps (~17 MB each, now 20-byte slots; the 128-bit key cannot be shortened exactly or
recovered from the value), resolved member symbol tables (~30 MB including their position indexes; change 6), the
id-link-store pages that do not fit 16 bits or stay sparse (~15 MB at load 0.33; sparse pages, above), intersection
and union caches (11 + 5 MB), link-store slot tables (~15 MB over 27 stores, 8 bytes per entry). The rest of an extra
checker is arena (~440 MB: types, symbols, mappers, link records), which this round did not touch.

Unverified: PGO instruction deltas per change for 2, 3 and 4 separately (measured together); instructions at 4 and
8 checkers (`perf` was run single-threaded only); the smaller bench projects (a vscode A/B was started and
interrupted).
