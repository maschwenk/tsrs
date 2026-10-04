# perf-checker-cpu2: the checker's CPU time, round 2

Follow-up to notes/cpu-checker.md (round 1, -9.5% instructions). Same rules: representation and code layout only,
identical diagnostics, baselines and `--extendedDiagnostics` counters.

Gates (final head against the base ff92b7f, both built here with `cargo build --release`): the suite with
`--baselines types,symbols` in the default mode and with `TSRS_LAZY_MEMBERS=0` (13,458 error / 12,779 types+symbols
pass in both; whole `TSRS_TEST_RESULTS` trees identical, `summary.json` identical apart from timings), the suite
with `--baselines js,jsmap,sourcemap` (13,462 / 13,392 / 149 / 156 pass, trees identical), fourslash 4,066 pass /
63 fail with identical result trees, `cargo test -p tsrs_cli` (tsctests + default_emit; `api::memory_tests` cfg'd out
locally because it links glibc `malloc_trim`, which fails on macOS on main too), `RUSTFLAGS="-D warnings" cargo
+1.99.0 check --workspace --locked` clean. The 38k-file codebase: 0 errors and identical Types / Symbols /
Instantiations for every intermediate binary, one and four checkers, default mode, and for base and head with
`TSRS_LAZY_MEMBERS=0`; vscode and webpack (371 and 840 errors) print byte-identical output with base and head.

Measurement: `/usr/bin/time -l`, all binaries interleaved (order reversed every other round), medians of 3. The
machine was shared with two other heavy agents and Max's sessions (load 20-45 throughout), so instructions retired
moved by about +-1% between identical runs of the same binary and wall/cycles by +-10%; the per-commit wall deltas
below are mostly noise, the instruction deltas are the metric. Profiles: `samply record -r 4000
--unstable-presymbolicate` and a small reader of the profile JSON (exclusive / inclusive / callers tables,
per-address samples, a "share of samples in the first 64 bytes of the function" column, and a list of hot functions
with large stack frames joined from `objdump`). Allocation sites: the alloc-profile build with
`TSRS_HEAP_PROFILE=count` (notes/mem-census.md); building it with `CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed`
gives a dSYM, so `atos` resolves inlined frames to source lines (the plain release build keeps line tables only in
object files that LTO deletes).

## Result

The 38k-file codebase, `--noEmit --incremental false`, base ff92b7f vs head, 5 interleaved runs each at load 9-20
(the quietest window of the day):

| | base | head | delta |
| --- | --- | --- | --- |
| 1 checker, instructions | 300.4 G | 279.7 G | **-6.9%** |
| 1 checker, cycles | 107.8 G | 106.9 G | -0.9% (min -5.3%) |
| 1 checker, wall / check | 29.2 / 21.0 s | 26.8 / 20.9 s | -8% / -0.5% (noise: base check 19.7-23.8 s) |
| 1 checker, peak | 4.990 GiB | 4.988 GiB | 0.0% |
| 4 checkers, instructions | 408.9 G | 382.2 G | **-6.5%** |
| 4 checkers, cycles | 182.6 G | 172.4 G | -5.6% |
| 4 checkers, wall / check | 11.8 / 10.2 s | 10.5 / 9.0 s | -11% / -12% (noise: base wall 8.7-14.5 s) |
| 4 checkers, peak | 6.680 GiB | 6.680 GiB | 0.0% |

Instructions are the result; cycles and wall moved the same way on four checkers in every set (matrix below: cycles
-5.1%, wall -5.3%) but are within the noise on one checker (matrix: cycles -4.9%; a 5-run set at load 21-28:
+2.4%). The removed instructions are mostly cheap ones (prologues, epilogues, allocator fast paths), so cycles fall
less than instructions; the time that is left is memory latency.

Other projects (`bench/projects.json` checkouts, medians of 3, identical diagnostics and counters):

| project | 1 checker instructions | 4 checkers instructions | peak |
| --- | --- | --- | --- |
| vscode | 121.4 -> 116.4 G (-4.1%) | 127.6 -> 121.6 G (-4.7%) | unchanged (+-0.5%) |
| xstate-main | 8.46 -> 8.01 G (-5.4%) | 9.82 -> 9.32 G (-5.1%) | unchanged |
| webpack | 15.15 -> 14.58 G (-3.8%) | 16.74 -> 16.10 G (-3.8%) | unchanged |
| mui-docs | 60.2 -> 58.1 G (-3.5%) | 84.9 -> 79.3 G (-6.6%) | unchanged |

## Changes (one commit each)

All eight binaries interleaved, medians of 3 (load 20-45); "d prev" is each commit's instruction delta. Single-checker
instructions spread by up to +-1% between identical runs here (step 7's -2.0% is -0.4% in a separate 3-run
comparison), four-checker by +-0.5%. Wall columns are noise at this load. Peak: no step moved it by more than 0.2%.

| step | 1 checker: instr G | d prev | wall s | peak GiB | 4 checkers: instr G | d prev | wall s | peak GiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| base ff92b7f | 302.4 |  | 34.7 | 4.988 | 404.8 |  | 10.43 | 6.669 |
| 1 symbol table filter | 301.5 | -0.3% | 36.1 | 4.990 | 400.7 | -1.0% | 9.88 | 6.680 |
| 2 temporary lists | 298.7 | -0.9% | 39.3 | 4.990 | 397.3 | -0.9% | 10.02 | 6.675 |
| 3 inline fast paths I | 292.4 | -2.1% | 34.4 | 4.988 | 386.8 | -2.6% | 9.60 | 6.676 |
| 4 metadata by reference | 291.9 | -0.2% | 34.8 | 4.990 | 387.2 | +0.1% | 9.91 | 6.682 |
| 5 getObjectTypeInstantiation | 290.4 | -0.5% | 35.7 | 4.991 | 386.3 | -0.2% | 9.84 | 6.680 |
| 6 inline fast paths II | 287.3 | -1.1% | 35.7 | 4.991 | 381.0 | -1.4% | 9.79 | 6.684 |
| 7 compareTypes | 281.7 | -2.0% | 29.9 | 4.990 | 378.5 | -0.6% | 9.88 | 6.675 |
| total | | -6.9% | | +0.04% | | -6.5% | | +0.09% |

Counters for every binary: 9,630,120 types / 12,811,032 symbols / 44,820,708 instantiations (one checker),
13,788,912 / 16,549,988 / 76,888,800 (four). Opt-out mode (`TSRS_LAZY_MEMBERS=0`), base and head: 9,639,962 /
25,973,354 / 44,884,281 single and 16,200,921 / 39,704,001 / 89,981,648 on four checkers with `--checkerAssignment go`.

1. **Symbol table Bloom filter** (`tsrs_ast::symbol`). A `SymbolMap` is 24 bytes: entries pointer, `u32` length and
   capacity, and `Option<Box<SymbolMapExtra>>` (hash index once a table has more than 16 entries, keys that differ
   from their symbol's name). Nearly every table has no extra, so that word now holds either the box (an even
   address) or a 64-bit Bloom filter of the keys' hashes (two bits per key; bit 0 set as the tag). A lookup the
   filter rejects returns without reading the entries, which are a second cache line; round 1 counted 54% of the
   144M lookups as misses in linear tables (every property miss walks the apparent-type chain down to `Object`'s
   7-entry members table). No size change. Instructions -0.3% / -1.0% (one / four checkers; the hash is now computed
   up front instead of only after a length match), `SymbolMap::position` 6.3% -> 5.1% of check samples; the rest of its time is hits (entries,
   then the symbol and its name for the comparison) and the hashed tables.
2. **Temporary lists** (`instantiate_type_worker`, `findApplicableIndexInfo`, `getCrossProductIntersections`,
   `mapType`, template literal spans, lazy member tables). Heap allocations on the 38k-file codebase, one checker:
   156.2M -> 129.6M (-17%). The largest: `instantiateType` of a type reference or union copied the argument /
   constituent list even when nothing changed (Go returns its input slice; 14.5M copies) - now no list when nothing
   changed and a pooled buffer (`Checker::free_type_lists`) when something did; `findApplicableIndexInfo` allocated
   an 8-slot `Vec` per call even for types without index signatures (5.6M); the cross product rebuilt its
   constituent list per combination (4M); template literal spans formatted every literal into a temporary `String`.
3. **Inline fast paths I** (`getReducedType`, `couldContainTypeVariables`, `instantiateType`). Each returns early for
   most calls (not a reducible union / intersection; the cached `CouldContainTypeVariables` bit; nothing to
   instantiate) but first paid a large function's prologue and epilogue (`get_reduced_type` had a 272-byte frame
   and saved every callee-saved register before testing two flags). The early returns are `#[inline]` wrappers now,
   the rest `#[inline(never)]` workers. -2.1% / -2.6%.
4. **Source file metadata by reference** (`Program::get_source_file_meta_data`). It cloned two strings per call;
   `resolveExternalModule` asks for it for every import specifier during checking. The `checker.Program` trait still
   returns a value (its callers are cold).
5. **`getObjectTypeInstantiation`**: `contains_key` + `insert` + index + `get_mut` on the per-target map became one
   `entry` probe on the hit path; the type-argument list is a pooled buffer.
6. **Inline fast paths II**: `resolveStructuredTypeMembers` returns resolved members without calling the worker
   (416-byte frame); lazy member table creation moved out of `getReadyLazyMemberTable`, so the lookup has a small
   frame; `getLateBoundSymbol` and `getLazyMappedTable` inline their early returns.
7. **`compareTypes`** decides identity and the type-flags order inline (163M calls, round 1; most end at the flags
   difference); names and structure out of line. -0.4% to -2.0% (noise; four checkers -0.6%).

## Profile now (head, one checker, exclusive samples in the check phase)

| base | % | head | % |
| --- | --- | --- | --- |
| `SymbolMap::position` | 6.30 | `SymbolMap::position` | 5.11 |
| `instantiate_type_with_alias` | 2.98 | `instantiate_type_with_alias_worker` | 3.09 |
| hashbrown `reserve_rehash` | 2.50 | `LinkStore::get` | 2.42 |
| `compare_types` | 2.29 | `memcmp` | 2.05 |
| `memcmp` | 2.19 | hashbrown `reserve_rehash` | 2.05 |
| `get_apparent_type` | 2.18 | `get_apparent_type` | 2.03 |
| `get_reduced_type` | 2.06 | relation cache `lookup` | 1.67 |
| `LinkStore::get` | 2.02 | hashbrown `HashMap::insert` | 1.59 |
| relation cache `lookup` | 1.57 | `get_object_type_instantiation` | 1.58 |
| hashbrown `HashMap::insert` | 1.56 | `get_property_of_type_worker` | 1.39 |
| `memmove` | 1.46 | `memmove` | 1.23 |
| `get_ready_lazy_member_table_worker` | 1.15 | `TypeMapper::map` | 1.16 |
| `resolve_structured_type_members_worker` | 1.13 | `signatures_of_structured_type` | 1.14 |
| `TypeMapper::map` | 1.11 | `compare_types_same_flags` | 1.12 |
| `mi_malloc` | 1.02 | `is_simple_type_related_to` | 0.90 |

`get_reduced_type`, `could_contain_type_variables` and the resolved-members check no longer appear as functions (they
are inlined into their callers); `mi_malloc` + `mi_free` went from 1.9% to 1.4%. Shares are of a profile of the
check phase only, so a function that kept its absolute cost gains share as others shrink.

## Tried and rejected

- A 1,024-entry direct-mapped cache in front of `lazy_member_tables` (the lazy-member reference -> table map,
  ~1M entries): 94% of 22.2M probes hit the cache, but the worker's share of samples did not move (1.28% vs 1.31%)
  and instructions did not change: a reference's member queries come in runs, so the map's bucket and entry are
  already in cache on repeats. The worker's samples are its first loads of the reference and its target. Storing
  the table in the type (8 bytes per reference, ~17 MB) would remove the same probe, so it was not tried.
- Allocation sites after change 2 are all below 3% of the remaining 130M allocations (intersection construction's
  ordered set, `SymbolTable::entries` snapshots, binder tables, `PendingTypeAlias` lists); each is < 0.1% of
  instructions. Snapshots stay: whether the table can change during the iteration would have to be proven per site.

## Looked at, nothing to take

- String-built cache keys: none left in the checker. Relation, instantiation, union / intersection, indexed access,
  template literal and conditional caches use `keyBuilder`'s binary layout hashed with xxh3-128 (Go's layout; round 1
  inlined the writers); `ReferenceInstantiations` hashes type ids directly; the `String`-keyed maps
  (`string_literal_types`, `undefined_properties`, ...) are keyed by the semantic name and looked up by `&str`.
- Hashers: every checker / binder / symbol-table map is Fx (`FxHashMap`, `HashTable` with Fx or xxh3 hashes); no
  SipHash on a hot path.
- Locks: `pthread_mutex_*` 0.05% of check samples; `RefCell` borrow flags in the relation caches are a compare each.
- `getTypeNamesForErrorDisplay` (typeToString) is ~1% of check samples on a project with no errors: relation errors
  under `@ts-expect-error` / `@ts-ignore` are still built and then dropped, as in Go.
- Module resolution during checking (`resolveExternalModule` 1.5% of check samples): `GetSourceFile` normalizes the
  resolved file name to a path on every call (`to_path`), as Go does; change 4 removed the copies, the rest is Go's
  work.
- `Type::target`, `TypeMapper::map`, link stores, `get_recursion_identity_*`: their samples are the first load of the
  type / symbol (cache misses), not instructions; the struct sizes and orders are the arena agents' territory (round
  1 found `#[inline]` on the accessors made no difference).

## What remains

- `SymbolMap::position` (~5% of check samples): hits read the entries, then the symbol and its name bytes for the
  string comparison. Interned names (a pointer comparison) would save the name's line; that is a parser / binder
  change.
- `instantiate_type_with_alias_worker` (~3%): active-mapper cache probe and key building; the per-mapper caches are
  dropped when one instantiation grew them past 4x (round 1), and growing them again is a visible share of
  `reserve_rehash` (2% of check samples in all, spread over ~10 maps that Go also grows).
- Further inline fast paths: each remaining candidate in the frame list is below ~0.3% of samples.
- PGO (the release build) already lays out hot paths; these changes were measured on the plain release profile like
  round 1. A PGO comparison was not run.

## Reproduce

```sh
cargo build --release -p tsrs_cli
cd <38k-file codebase> && /usr/bin/time -l tsrs -p . --noEmit --incremental false --extendedDiagnostics --pretty false --singleThreaded
samply record -r 4000 --unstable-presymbolicate -s -o prof.json.gz tsrs -p . --noEmit --incremental false --singleThreaded
CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed CARGO_TARGET_DIR=target/prof cargo build --release -p tsrs_cli --features alloc-profile
TSRS_HEAP_PROFILE=count TSRS_HEAP_PROFILE_TOP=30000 target/prof/release/tsrs -p . --noEmit --incremental false --singleThreaded
```
