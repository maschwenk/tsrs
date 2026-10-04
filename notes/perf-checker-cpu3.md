# perf-checker-cpu3: the checker's CPU time, round 3

Follow-up to notes/cpu-checker.md (round 1) and notes/perf-checker-cpu2.md (round 2). Same rules: representation and
code layout only; identical diagnostics, baselines and `--extendedDiagnostics` counters; no struct changes size.
Base: origin/main 6edd0f0 (after the compiler-side lint paydown; compressed 32-bit handles on).

Gates (head c63508b against origin/main 6edd0f0, both built here with `cargo build --release`, reference checkout at
the pinned b85298b6a): `tsrs-test run --suite all --baselines types,symbols` in the default mode and with
`TSRS_LAZY_MEMBERS=0`: 13,458 error baselines pass (2 codes, 2 fail), 12,779 types, 12,779 symbols, whole
`TSRS_TEST_RESULTS` trees identical and `summary.json` identical apart from the per-test `ms`; `--baselines
js,jsmap,sourcemap`: 13,462 / 13,392 / 149 / 156 pass, trees identical; fourslash 4,066 pass / 63 fail, result trees
identical; `cargo test -p tsrs_cli` (`api::memory_tests` cfg'd out locally, it links glibc `malloc_trim`): tsctests
tsc 187 / 216, tsbuild 187 / 190, 0 crashes, same pass lists (one output differs in an `@iterator@<symbol id>` name
in both directions between runs, as notes/lint-paydown-compiler.md found), default_emit 6 / 6;
`RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked` clean; `tools/lint/ratchet.py` ok. No struct size
changed (no layout edits); no new `unsafe`.

## Result

The 38k-file codebase, `--noEmit --incremental false`, base 6edd0f0 vs head, release profile, 3 interleaved rounds
(medians), load 2-7 (a quiet window):

| | base | head | delta |
| --- | --- | --- | --- |
| 1 checker, instructions | 288.51 G | 276.31 G | **-4.2%** |
| 1 checker, cycles | 102.5 G | 99.9 G | -2.6% |
| 1 checker, wall / check | 15.12 / 14.33 s | 14.68 / 13.86 s | -2.9% / -3.3% |
| 1 checker, peak | 4.245 GiB | 4.241 GiB | -0.1% |
| 4 checkers, instructions | 391.33 G | 371.79 G | **-5.0%** |
| 4 checkers, cycles | 129.2 G | 128.0 G | -1.0% |
| 4 checkers, wall / check | 6.63 / 5.77 s | 6.45 / 5.56 s | -2.7% / -3.7% |
| 4 checkers, peak | 5.707 GiB | 5.700 GiB | -0.1% |

Per-round paired instruction deltas (each round's head run against that round's base run): -4.35 / -4.23 / -4.18%
with one checker, -5.12 / -4.99 / -4.48% with four. Most of what was removed is prologues, epilogues, key-buffer
zeroing and out-of-line calls on fast paths, which are cheap instructions; cycles fall about half as much.

Other projects (`bench/projects.json` checkouts, same session, medians of 3; output byte-identical base vs head, the
same counters in every run):

| project | 1 checker instructions | 4 checkers instructions |
| --- | --- | --- |
| vscode (`src`, 371 errors) | 118.46 -> 116.25 G (-1.9%) | 124.21 -> 121.77 G (-2.0%) |
| webpack (840 errors) | 15.20 -> 14.88 G (-2.1%) | 16.91 -> 16.48 G (-2.5%) |

Peak on these two moves by up to +-0.5% between runs of the same binary (vscode four checkers: base 2.245-2.258 GiB,
head 2.258-2.271); no change here allocates differently (allocation-free code-layout changes, identical counters).

## Changes (one commit each)

All eight binaries interleaved in one session, medians of 3; "d prev" is each commit's instruction delta. Peak: no
step moved the 38k-file codebase by more than 0.4% (four checkers run to run: +-0.3%).

| step | 1 checker: instr G | d prev | peak GiB | 4 checkers: instr G | d prev | peak GiB | vscode d prev (1 / 4) | webpack d prev (1 / 4) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| base 6edd0f0 | 288.51 | | 4.245 | 391.33 | | 5.707 | | |
| 1 symbol table search | 285.25 | -1.13% | 4.247 | 385.99 | -1.36% | 5.692 | -0.84 / -0.89% | -0.79 / -1.01% |
| 2 instantiation key | 281.83 | -1.20% | 4.246 | 378.23 | -2.01% | 5.699 | -0.28 / -0.32% | -0.46 / -0.48% |
| 3 key builder | 279.98 | -0.66% | 4.249 | 377.11 | -0.30% | 5.689 | -0.08 / -0.13% | -0.13 / -0.18% |
| 4 instantiateTypeWithAlias | 279.44 | -0.19% | 4.246 | 376.10 | -0.27% | 5.686 | -0.03 / -0.01% | -0.07 / 0.00% |
| 5 recursion identity | 277.49 | -0.70% | 4.242 | 373.47 | -0.70% | 5.692 | -0.15 / -0.18% | -0.27 / -0.36% |
| 6 getTypeOfSymbol | 276.90 | -0.21% | 4.245 | 372.68 | -0.21% | 5.686 | +0.37 / +0.46% * | -0.07 / -0.18% |
| 7 same-string shortcuts | 276.31 | -0.21% | 4.241 | 371.79 | -0.24% | 5.700 | -0.88 / -0.90% * | -0.33 / -0.36% |
| total | | -4.23% | -0.1% | | -4.99% | -0.1% | -1.87 / -1.96% | -2.14 / -2.52% |

\* vscode step 6's runs spread by 0.8% (116.49-117.40 G one checker), so steps 6 and 7 there are better read
together: -0.5 / -0.4%.

Counters for every binary on the 38k-file codebase: 9,630,120 types / 12,811,032 symbols / 44,820,708 instantiations
(one checker), 13,788,912 / 16,549,988 / 76,888,800 (four), 0 errors. `TSRS_LAZY_MEMBERS=0`, base and head:
25,973,354 symbols / 9,639,962 types / 44,884,281 instantiations single and 39,704,001 / 16,200,921 / 89,981,648 on
four checkers with `--checkerAssignment go`.

1. **Symbol table search** (`tsrs_ast::symbol`). `SymbolMap::position` saved seven register pairs before the Bloom
   filter test that ends 52% of all lookups (74M of 143M; site counts below). It now computes the hash and tests the
   filter inline, and calls the out-of-line `search` only past the filter. A hit compares the key with the stored
   name by pointer and length first (`same_text`): 43% of the 48M hits use the very string the symbol was named with
   (an instantiated property looked up by its declaration's name), which skips the byte comparison and the load of
   the name's bytes. `insert` hashes the name once. Making `position` out of line but small instead measured -0.83%
   against -1.18% for the inline version.
2. **Instantiation cache key** (`instantiateTypeWithAlias`). Without an alias (nearly every call) the active-mapper
   cache key is the type id and a zero byte; hashing those five bytes from a stack array gives the same xxh3-128 key
   without zeroing `keyBuilder`'s 192-byte buffer and two out-of-line calls (`write_alias_arg`, `hash`). Round 1
   measured not zeroing the buffer alone at -0.2%; the calls were the larger part.
3. **Key builder**: `write_alias_arg` was an out-of-line call (0.8% of check samples) that saved five register pairs
   to write the zero byte of a missing alias; `keyBuilder::hash` was not inlined because of its overflow branch. Both
   rare cases are out of line now (every key builder benefits: union, intersection, indexed access, type
   instantiation, conditional keys).
4. **`instantiateTypeWithAlias`**: the out-of-line copy saved five register pairs for the alias type-argument loop
   before testing the cached `CouldContainTypeVariables` bits. It tests the bits first; the checks that call out
   (computing the bits, the alias arguments) are in `instantiate_type_with_alias_slow`, so every fast path returns
   or tail-calls the worker without a frame.
5. **Recursion identity** (`isDeeplyNestedType`): `hasMatchingRecursionIdentity` (called for every type on the
   relation stacks) and `getRecursionIdentityTarget` saved registers for their recursive cases (intersections, indexed
   access, instantiated mapped types) before returning the common answer; the common cases are inline now.
6. **`getTypeOfSymbol`**: it inlined the link lookups of its instantiated and variable/property cases and the mapped
   symbol functions, so every call saved six register pairs. It now reads the cached resolved type of those two cases
   first, without creating links or assigning a symbol id (`SymbolArenaLinkStore::try_get_if_id_assigned`: a symbol
   without an id or links falls through to the original path, which assigns both exactly as before), and
   tail-calls everything else; `getTypeOfMappedSymbol` / `getTypeOfReverseMappedSymbol` are `#[inline(never)]`.
7. **Same-string shortcuts**: `SymbolMap::insert` compared each new key with its symbol's name byte by byte to decide
   whether it is an odd key (it is nearly always that very string); `compareTypeNames` / `compareSymbols` order
   distinct symbols by name, which are often the same declaration-name string. Both decide equal strings by pointer
   and length first.

## Profile

`samply record -r 4000 --unstable-presymbolicate`, one checker, exclusive samples under `Checker::get_diagnostics`:

| base | % | head | % |
| --- | --- | --- | --- |
| `SymbolMap::position` | 5.23 | `instantiate_type_with_alias_worker` | 3.03 |
| `instantiate_type_with_alias_worker` | 3.15 | `get_property_of_type_worker` (now holds the inlined filter test) | 2.83 |
| `get_apparent_type` | 2.54 | `get_apparent_type` | 2.57 |
| hashbrown `reserve_rehash` | 2.43 | `SymbolMap::search` | 2.55 |
| `memcmp` | 2.26 | hashbrown `reserve_rehash` | 2.39 |
| `LinkStore::get` | 1.84 | `LinkStore::get` | 2.10 |
| `get_property_of_type_worker` | 1.80 | `compare_types_same_flags` | 1.72 |
| `compare_types_same_flags` | 1.67 | `memcmp` | 1.71 |
| hashbrown `HashMap::insert` | 1.63 | hashbrown `HashMap::insert` | 1.65 |
| `signatures_of_structured_type` | 1.51 | relation `lookup` | 1.59 |

Shares are of a check-phase profile, so what kept its absolute cost gains share. `memcmp` called from symbol-table
hits went 0.82% -> 0.50% of samples, from `SymbolMap::insert` 0.30% -> below 0.1%; `CacheHashKey::hash_128` left the
top list (1.16% -> below 0.5%: the five-byte keys inline). Samples on register save / restore instructions
(`stp`/`ldp` of x19-x30 against `sp`, `sub/add sp`) went 15.3% -> 14.4% of check samples; what is left is spread
thinly (the largest, `SymbolMap::search` and `get_apparent_type`, ~0.37% of all samples each).

Symbol table lookups (site counts, one checker, whole run): 142.6M lookups; linear tables 97.4M (74.1M ended by the
filter, 2.0M other misses, 21.3M hits at an average position of 3.3), hashed tables 45.2M (18.7M misses, 26.5M
hits). Of the 47.8M hits, 20.7M had the same pointer and length as the stored name; 64% of hit names are at most 8
bytes, 93% at most 16.

## Interned names (candidate 1 of the brief): not done

What a hit costs now: the entries' cache line, the symbol (for its name word), and for the 57% of hits whose key is a
different string than the stored one, the name's bytes and a `bcmp` call. Interning (one canonical string per name,
compared by handle) would remove only that last part: about 27M `bcmp` calls of short names (~0.5 G instructions,
0.2%) plus the name's cache line, and the symbol's line is loaded by nearly every caller right after the lookup
anyway (`get_type_of_symbol`, flags). The keys that differ come from use-site identifiers (`x.foo`, references), so
they would have to arrive interned: the parser or binder interning every identifier, which notes/mem-round2.md built
and rejected (+9% parse time from hashing and locking 7.8M identifiers). Looking keys up in an interner at lookup
time would add a hash-table probe per lookup. Pointer equality takes the cases that are already canonical for free.

Reusing the name's hash across the tables of one property lookup (own members, then `Function`, then `Object`): 70.5M
`getPropertyOfType` calls against 122.7M filter tests, so at most ~0.5 rehash per call, ~0.3%, and it would need a
hashed-name type threaded through the lazy member tables and the name resolver hook. Not done.

## Handle -> reference round trips (candidate 2): little on arm64

On this Mac a dereference is `add x, base, w, uxtw #3` and a field load; the base is one `mov` per function. Counted
over the whole binary weighted by samples: base materializations 0.38%, `add base + handle << 3` 0.56%, `ubfiz`
shifts 0.30% of check samples. The hot functions dereference each handle once; redundant re-derivations of the same
handle were single instructions (`get_apparent_type`: one). The x86 costs in notes/mem-pointer-compression.md section
6 (zero-extends of argument handles, shifts back in `as_p()` / `from_arena`) have no arm64 counterpart of that size,
and no x86 machine was used here. Fewer non-inlined calls taking `&self` (changes 4-6) remove such conversions on x86
too, but that was not measured.

## Tried and rejected

- **Inline fast paths III** (`get_apparent_type` with the instantiable case out of line, `TypeMapper::map` with the
  simple and array kinds frameless, `is_generic_mapped_type`, `maybe_type_of_kind`, `get_normalized_type`): each
  had 20-55% of its samples on register saves, together -0.08% paired median (4 rounds, `--singleThreaded`). Their
  samples on saves are mostly skid from the first load of the type (a cache miss), not the saves themselves.
- **`ReferenceInstantiations` hashing argument handles instead of type ids** (growing these tables rehashes every
  entry and reads each argument type: ~1% of check samples in `reserve_rehash` / `prepare_resize`): no change in
  instructions or cycles (paired median -0.07%).
- **`position` out of line but small** (`#[inline(never)]`): -0.83% against -1.18% inline (change 1).

## Measuring

Instructions retired from `/usr/bin/time -l` on macOS include kernel work (page faults for ~4 GiB, file reads), and
idle rayon pool threads spin for a while after the parse: the same binary moved by up to 1.3% between runs at load
10-20 even with `--singleThreaded`, and the first run of a session retires ~1% more (file cache). For deciding single
changes: `--singleThreaded` with `RAYON_NUM_THREADS=1` (3 threads instead of 20), a discarded warm-up run, 4
interleaved rounds, and the median of per-round paired deltas. The table above is the brief's measurement (one and
four checkers, medians of 3) in one quiet session (load 2-7), where it was consistent round to round.

## Reproduce

```sh
cargo build --release -p tsrs_cli
cd <38k-file codebase> && /usr/bin/time -l tsrs -p . --noEmit --incremental false --extendedDiagnostics --pretty false --checkers 1
RAYON_NUM_THREADS=1 /usr/bin/time -l tsrs -p . --noEmit --incremental false --pretty false --singleThreaded
samply record -r 4000 --unstable-presymbolicate -s -o prof.json.gz tsrs -p . --noEmit --incremental false --checkers 1
CARGO_TARGET_DIR=target/sc cargo build --release -p tsrs_cli --features site-counts   # symbol table counters: a local patch, not committed
```
