# mem-checker-scratch: per-checker garbage, free-list high-water and hash-table slack on the app projects

Question (2026-10-07, round 4, "beat bun check on memory"): notes/mem-per-checker-duplication.md 2c found that each
extra checker leaves 8-14% of what it allocates as unreachable, never-freed garbage and sends another 17-30% to the
arena free lists, and that every checker owns hash tables with up to 2x their entries in capacity. This note
attributes the garbage, measures what the free lists cost at their peak, and measures the tables' slack, on
t3code-server, formbricks-web and cal-diy (the projects where `bun check` uses the least memory relative to tsrs).

Result: nothing here clears the 5% peak bar of AGENTS.md at 32 checkers, and no code change lands except the census
fix in this PR.

- Half of the "garbage" was a census blind spot. Type lists held only by array mappers were never seen by the census.
  Fixed (section 1). With the fix, an extra checker leaves 3.3-5.5 MB of garbage (4.4-7.0% of what it holds), spread
  over about ten causes.
- The only exact fix of size is the `export *` resolution tables on formbricks and cal-diy. Prototyped: -1.9% peak
  at 32 checkers on 64-vCPU Linux on both.
- The free lists never hold more than 6 KB per checker. The scratch that goes through them is reused at once and
  costs nothing.
- The checker's hash tables are as small as hashbrown allows (shrinking to fit saves 0). Only a different table
  design could cut them, by at most 1.7-2.2% of peak at 32 checkers.

Base: origin/main 4f43c94, macOS, Apple M5 Max (18 cores), 16 KiB pages. Flags `--noEmit --incremental false --pretty
false --extendedDiagnostics --checkers N`, run from the bench checkouts (t3code-server `-p apps/server`,
formbricks-web `-p apps/web/tsconfig.typecheck.json`, cal-diy `-p apps/web`). MB = 2^20 bytes.

"Per extra checker" is (value at 16 checkers - value at 1) / 15. Object counts and bytes do not depend on load. The
census uses plain 8-byte pointers (`tsrs_core/plain-ptrs`), so its bytes are larger than a normal build's. Its
shares are the numbers to use. Linux numbers are from the Depot 64-vCPU runner (THP `madvise`).

## 1. A census blind spot: mappers' type lists

`pack_slice` in crates/tsrs_checker/src/mapper.rs stores an array mapper's list as its address shifted left by one,
with the length in the top 16 bits. That encoding came with the compressed-pointer mappers (6c01134).

Neither census mark decoded it:

- The conservative mark reads the low 48 bits, which give twice the address. So every list held only by a mapper
  counted as unreachable.
- The strong mark (the free-gate) also missed those edges. A freed list that an escaped array mapper still held would
  not have been reported.

Both marks now also decode `(w & (2^48 - 1) & !7) >> 1`. The conservative mark does this for words with a length in
the top 16 bits; the strong mark does it for `TypeMapper` words (crates/tsrs_core/src/alloc_profile/census.rs).
Effect on t3code-server:

| | 1 checker before / after | 16 checkers before / after |
| --- | --- | --- |
| arena unreachable at exit (freed blocks left out) | 61.8 / 42.5 MB | 204.2 / 109.2 MB |
| strongly reachable blocks (what the free-gate checks) | 19.36M / 20.00M (878.8 / 898.0 MB) | 47.96M / 50.75M (2,444 / 2,543 MB) |
| strongly reachable freed blocks (violations) | 0 / 0 | 0 / 0 |

So the free-gate now covers 0.64M more blocks at 1 checker and 2.8M more at 16, and it still finds no violation.
The same holds on formbricks-web and cal-diy at 1 and 16 checkers, on formbricks at 4 with `TSRS_CENSUS_VERIFY=1`, and
on t3code with `TSRS_LAZY_MEMBERS=0`. The figure in mem-per-checker-duplication.md 2c ("8-14% garbage") included these
lists. The `[P<Type>]` rows created at `create_lazy_member_table` (checker_09.rs:2792/2799, the lazy tables' mapper
lists) were the largest "garbage" rows, and they are live.

## 2. Garbage per extra checker, by cause

`TSRS_CENSUS=1 TSRS_CENSUS_SKIP_FREED=1 TSRS_CENSUS_FRAMES=6` at 1 and 16 checkers. Freed blocks are left out (they
are reused in a normal build). The rows are the sampled arena stacks plus the heap blocks, grouped by the frames
above the allocation. MB per extra checker; the % is of what an extra checker holds at exit (arena plus live heap,
census bytes: 113.8 / 63.4 / 76.4 MB).

| cause | t3code-server | formbricks-web | cal-diy |
| --- | ---: | ---: | ---: |
| `export *` resolution: cloned and nested export tables | 0.17 (0.2%) | 2.34 (3.7%) | 1.18 (1.5%) |
| expression re-checks: object literal and spread types, members, symbols | 1.93 (1.7%) | 0.22 (0.3%) | 0.29 (0.4%) |
| lazy member tables dropped when resolved in full | 1.00 (0.9%) | 0.41 (0.6%) | 0.65 (0.9%) |
| inference contexts outside the recycling sites | 0.88 (0.8%) | 0.21 (0.3%) | 0.10 (0.1%) |
| type lists (instantiation keys and arguments) | 0.51 (0.4%) | 0.30 (0.5%) | 0.14 (0.2%) |
| global merges: declaration arrays copied on every append | 0.01 (0.0%) | 0.61 (1.0%) | 0.20 (0.3%) |
| relater: property lists in declaration order | 0.37 (0.3%) | 0.12 (0.2%) | 0.19 (0.3%) |
| mappers outside the recycling sites | 0.37 (0.3%) | 0.12 (0.2%) | 0.09 (0.1%) |
| widening contexts | 0.02 (0.0%) | 0.00 | 0.33 (0.4%) |
| node builder (diagnostic text), other | 0.17 | 0.11 | 0.13 |
| **total** | **5.48 (4.8%)** | **4.43 (7.0%)** | **3.32 (4.4%)** |

Exact totals from the unsampled tables are 4.45 + 1.07 / 2.08 + 2.65 / 1.55 + 1.80 MB (arena + heap).

Even if every row were freed, an extra checker would keep 95-96% of what it keeps now. At 32 checkers that is 31 x
3.3-5.5 MB in census bytes, roughly 100-130 MB in a compressed build (heap rows do not shrink), or 3.5-4.5% of the
peak. The pools above about 1% of the per-checker growth:

**`export *` resolution (formbricks 3.7%, cal-diy 1.5%).**

- What creates it: `getExportsOfModuleWorker`'s `visit` (checker_08.rs:1308) clones every visited module's export
  table (`symbol_exports.clone_table()`, Go `maps.Clone`) and builds a `nested_symbols` table per module with `export
  *`. Both are arena `SymbolTable`s with heap entry buffers.
- Why it is dropped: a nested module's table is merged into its importer's and then dropped. Go's GC frees it. Here
  arena objects are never dropped, so the heap buffers (entries, the hash index) leak too. Only the outermost result
  is kept, in the module's resolved-exports link.
- Exact fix: `visit` returns `SymbolTable` by value, `nested_symbols` is a local value, and `extend_export_symbols`
  takes `&SymbolTable`. Only the outermost table moves into the arena (`P::new`). Insertion order and contents do not
  change.
- Prototype: branch `mem/export-star-tables`, 0384607, measured below. Diagnostics are byte-identical at 16 and 32 on
  the three projects.

| | formbricks-web | cal-diy | t3code-server |
| --- | --- | --- | --- |
| macOS peak footprint, 32 checkers, 4 interleaved runs | 2,841 -> 2,763 MiB (-78, -2.8%) | 2,513 -> 2,484 (-29, -1.1%) | 2,750 -> 2,744 (noise) |
| macOS, 16 checkers | 2,191 -> 2,152 (-39, -1.8%) | 1,938 -> 1,933 | 2,224 -> 2,195 |
| Linux 64 vCPU, 32 checkers, 5 interleaved runs: peak RSS | 2.873 -> 2.817 GiB (-1.9%) | 2.625 -> 2.576 (-1.9%) | 2.773 -> 2.770 (-0.1%) |
| Linux, 4 / 1 checkers | -0.7% / -0.2% | -1.4% / -0.3% | -0.1% / +0.1% |
| Linux wall, 32 checkers | 0.674 -> 0.689 s | 0.626 -> 0.624 | 1.441 -> 1.473 |

The two release builds also differ by 2% in wall on t3code, where the change does almost nothing. That is the
build-to-build noise clusterprobe.sh warns about, not a cost. Below the bar, so not landed. It is the one exact fix
of size here, if a later change brings the rest of this path along. In particular, the outermost tables (3.9 MB per
extra checker on formbricks, reachable) are each checker's copy of the barrel files' merged exports. That is the
shared-graph question, out of scope here.

**Expression re-checks (t3code 1.7%).** Object literal and spread types, their members, property symbols and member
tables, created by `checkExpressionWithContextualType` under `isSignatureApplicable` / `inferTypeArguments` /
`instantiateSignatureInContextOf` and dropped after the candidate. notes/mem-recycle.md item 3 and
notes/mem-scoped-arenas.md measured this class: the fresh types escape through union, intersection, instantiation and
relation caches and link stores, so a free needs an escape check on every cache insert. Not exact without that; no
prototype.

**Lazy member tables resolved in full (0.6-0.9%).**

- What happens: when `resolve_lazy_members` replaces a lazy table with resolved members, the table leaves
  `lazy_member_tables`. Its arena record, its `declared` symbol table (heap; `get_lazy_declared_member` filled it) and
  its slices stay behind.
- Possible fix: clearing `declared` on removal is cheap. It is not provably exact, because callers of
  `get_ready_lazy_member_table` hold the `P<LazyMemberTable>` across calls that can resolve the type in full; a stale
  lookup would then miss instead of crashing.
- Size: about 0.75 MB per extra checker on t3code (census bytes), roughly 20 MB at 32 checkers. Not prototyped.

**Inference outside the recycling sites (t3code 0.8%).** Mostly the context, infos and candidate lists of
`instantiateSignatureInContextOf` (from `compareSignaturesRelated` and
`instantiateTypeWithSingleGenericCallSignature`). A recycling site after `getSignatureInstantiation(...,
getInferredTypes(context))`, gated on the escape bit like `inferTypeArguments`' (notes/mem-scoped-arenas.md item 1),
would be exact by the same argument. That is about 0.6 MB per extra checker in census bytes, roughly 12-19 MB at 32
checkers (0.5%). Not prototyped.

**Declaration arrays on global merges (formbricks 1.0%).** `Symbol::append_declarations` (symbol.rs:111) copies the
whole array on every append. Merging `declare global` and module augmentations across files (`merge_global_symbol`
in `new_checker`) is therefore quadratic in garbage, where Go's amortized `append` is linear. Freeing the old copy
needs proof that no other symbol shares it (`set_declarations_static` shares slices). An amortized buffer needs a
capacity field. Size: about 0.3 MB per extra checker in a compressed build (4-byte entries), about 10 MB at 32
checkers. Not prototyped.

## 3. Free lists and recycled pools at their peak

The alloc profile (`--features alloc-profile`, compressed pointers, normal reuse) now prints, per arena, the bytes on
the free lists at exit and at their peak, the sum of the per-size-class peaks, bytes reissued, and bytes the
recycling sites bumped because their list was empty. Checker arenas only; at 1 checker the check runs on the `tsrs`
thread.

| project | checkers | reissued MB | bumped by recycling sites MB | on lists at exit KB | peak on lists KB (sum of per-arena peaks) | sum of class peaks KB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| t3code-server | 1 | 106.5 | 33.5 | 0.2 | 5.6 | 8.0 |
| t3code-server | 16 | 537.4 | 115.6 | 11.1 | 70.0 | 85.4 |
| formbricks-web | 1 | 62.6 | 17.8 | 0.4 | 3.4 | 3.9 |
| formbricks-web | 16 | 173.1 | 53.7 | 10.8 | 41.5 | 50.2 |
| cal-diy | 1 | 32.4 | 16.5 | 0.8 | 4.8 | 5.2 |
| cal-diy | 16 | 90.8 | 54.4 | 13.9 | 37.4 | 46.4 |

At most 6 KB per checker ever sits on the free lists. The scratch is freed and taken again almost at once: each
`chooseOverload` candidate reuses the previous candidate's blocks, and the conditional-type scratch is returned when
the outermost `getConditionalType` returns, at shallow nesting. Neither one deep inference nor scratch kept until the
end of a file builds up. Reissued bytes per extra checker (28.7 MB on t3code) match mem-per-checker-duplication.md 2a
("of which reissued", 27.9). They cost no memory.

The bumped bytes are recycling-site allocations that found the list empty. They are the blocks that stay live
(escaped mappers and contexts) or end up in the census's garbage rows above.

The checker's heap pools (`free_type_lists`, the active/free type-mapper caches, `scratch_*` lists, the infer memo's
key pool, `instantiation_stack`, `inference_context_infos`) hold under 0.1 MB per checker in `TSRS_HEAP_CENSUS`.

So no mechanism holds tens of MiB per checker at its peak, and there is nothing to bound.

## 4. The checker's hash tables: entries against capacity

`TSRS_HEAP_CENSUS=1 TSRS_HEAP_CENSUS_MIN=0`, release build. These are the containers the checker owns directly,
summed over the checkers. "Single tables" are the checker-level hash maps, one per checker. "Grouped" rows are
per-object maps and link-store pages (object-type instantiation inner maps, lazy mapped member maps, value-symbol link
groups, symbol-node value groups).

| project | all containers, 1 / 16 / 32 checkers | per extra checker (1 -> 32) | single tables at 32: MB, mean load | shrunk to fit | at load 7/8 (non-power-of-two) | grouped rows at 32: MB, entries x slot |
| --- | --- | ---: | --- | --- | --- | --- |
| t3code-server | 69.0 / 300.9 / 375.0 MB | 9.9 MB | 219.9, 0.72 | 222.0 | 156.9 (-63) | 148.4, 92.6 |
| formbricks-web | 74.8 / 195.7 / 271.3 | 6.3 | 162.0, 0.72 | 163.1 | 113.0 (-49) | 106.8, 66.7 |
| cal-diy | 59.2 / 194.6 / 251.4 | 6.2 | 154.0, 0.72 | 156.6 | 109.9 (-44) | 90.6, 54.3 |

The largest rows at 16 checkers (MB summed, load), t3code / formbricks / cal-diy:

- object-type instantiation inner maps: 39.6 (0.70) / 31.3 (0.68) / 29.8 (0.69)
- value-symbol link narrow groups (arena, 2-byte slots): 26.6 (0.50) / 14.2 (0.59) / 15.7 (0.62)
- assignable relation: 25.7 (0.74) / 20.0 (0.63) / 26.0 (0.71)
- mapped-symbol link slots: 23.6 (0.66) / 14.1 (0.68) / 14.6 (0.66)
- `lazy_member_tables`: 21.3 (0.65) / 4.8 / 1.8
- `cached_types`: 16.0 (0.77) / 8.6 / 7.2
- `union_types`: 15.4 (0.64) / 12.4 (0.79) / 20.2 (0.65)

Every table above 1 MiB grows only (or loses few entries), so hashbrown's power-of-two sizing already gives each its
minimum. What each lever would save:

- **Shrinking where a table stops growing (the end of the check):** nothing. The minimum size computed from the
  entries equals today's size (the "shrunk to fit" column comes out a little higher only from rounding in the
  estimate).
- **A right-sized initial capacity:** no memory either, since the final power of two is the same. It would save
  rehash work only, and the final sizes vary 10x between projects, so they cannot be guessed.
- **A different growth policy:** hashbrown cannot do it (power-of-two buckets, load at most 7/8). A table that grows
  in smaller steps and stays near 7/8 load would make the single tables 29-31% smaller: -44 to -63 MB at 32 checkers
  (1.7-2.2% of the 2.57-2.91 GB Mac peaks). The grouped rows are already 0.6-0.7 full of small maps, whose fixed
  per-map cost (16 control bytes, 4-bucket minimum) is the rest. Below the bar, and it means replacing hashbrown for
  about 25 tables.

Rehash cost, single-threaded: macOS `sample`, the compiler thread, release build. `reserve_rehash` is 2.68% /
3.34% / 2.44% of the samples (29 of 1,081 / 84 of 2,518 / 92 of 3,771) on t3code / formbricks / cal-diy. It is spread
over hundreds of small per-object maps and local sets. The largest callers are `ReferenceInstantiations::add` (0.4%),
the conditional-root instantiation maps (0.2%) and the local sets of `every_lazy_property` and
`check_grammar_object_literal_expression`. The checker-level tables above 1 MiB (`Relation::set`, link stores) are
at most 0.2% each, so presizing them would buy well under 1% of instructions.

## 5. Not measured / inferred

- The 32-checker garbage figures are 31 x the 1 -> 16 slope. For `ExportCollision` (#135) the slope from 16 to 32
  was 87% of the slope from 1 to 16.
- Compressed-build bytes for the garbage rows are estimated (arena rows about halve, heap rows do not); only the
  `export *` fix was measured in a normal build.
- Mac wall times are not used (load 8-36 on 18 cores during the runs). Linux wall in section 2 is 5 interleaved runs
  of release builds (not PGO/BOLT).
- The lazy-member-table, inference and declaration-array fixes were not prototyped; their sizes are census rows.
- Rehash shares come from one sampled run per project (about ±0.5% absolute at these sample counts).

## Reproduce

```sh
CARGO_TARGET_DIR=target/census cargo build --release --locked -p tsrs_cli --features alloc-profile,tsrs_core/plain-ptrs
CARGO_TARGET_DIR=target/prof cargo build --release --locked -p tsrs_cli --features alloc-profile
cargo build --release --locked -p tsrs_cli
cd <bench checkout>
# section 2: garbage by cause (one census at a time: 4-13 GB)
TSRS_CENSUS=1 TSRS_CENSUS_SKIP_FREED=1 TSRS_CENSUS_FRAMES=6 TSRS_CENSUS_TSV=c-N.tsv \
  target/census/release/tsrs -p <tsconfig> --noEmit --incremental false --pretty false --checkers {1,16}
# section 3: free lists ("-- arena free lists" in the alloc profile)
target/prof/release/tsrs -p <tsconfig> --noEmit --incremental false --pretty false --checkers {1,16}
# section 4: tables
TSRS_HEAP_CENSUS=1 TSRS_HEAP_CENSUS_MIN=0 target/release/tsrs ... --extendedDiagnostics --checkers {1,16,32}
# section 2 prototype on Linux: push a branch with the change and tools/perf/scratchprobe.sh (branch
# mem/export-star-tables has it), then
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref <branch> \
  --input projects=formbricks-web,cal-diy,t3code-server --input script=tools/perf/scratchprobe.sh \
  --input probe_args='--reps 5 --checkers 1,4,32'
```

The grouping of the census TSV rows into causes is a short script over the "arena by type and allocating function
<- callers" and "heap by allocating function <- caller" tables: per row, (unreachable at 16 - unreachable at 1) / 15,
classed by the first matching frame (`get_exports_of_module_worker`, `append_declarations`, widening, inference types,
mappers, `create_lazy_member_table` / `get_lazy_declared_member`, `get_lazy_properties_in_order`, expression
checking, type lists), with frames demangled by `rustfilt`.
