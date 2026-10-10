# Exact heap-allocation counts per call site (2026-10-10)

Measurement only, no code change. Branch `exp/xctrace-alloc` on top of `perf/heap-churn` (#272, `580ba230`): the
`system-alloc` feature of `tsrs_cli` (no global allocator), a malloc interposer (`tools/perf/allocstacks.c`), the
aggregation script (`tools/perf/allocsites.py`) and the Time Profiler classifier (`tools/perf/tpshare.py`); recipe in
docs/DEBUGGING.md "Exact heap-allocation counts per call site". Release build, symbols kept, single-threaded
(`--singleThreaded`, `RAYON_NUM_THREADS=1`, `--noEmit --incremental false --pretty false`), Apple M5 Max, macOS 26.6.2,
Xcode 27.0 xctrace. Diagnostics byte-identical across the mimalloc build, the system-alloc build, the interposed run and
the re-signed binaries on all four projects.

## Totals and the three-way cross-check

| project | interposer (exact: malloc+calloc+realloc+memalign) | Instruments Statistics `count-total` | mimalloc `MIMALLOC_SHOW_STATS` (sum of bins, 3-digit rounding) | distinct 12-frame stacks | live at exit (Instruments) |
| --- | --- | --- | --- | --- | --- |
| xstate-main | 3,114,089 (malloc 2,821,266; calloc 23,922; realloc 268,856; memalign 45; free 2,606,705) | 3,114,188 | 3,096,930 (printed "3.0 M") | 150,366 | 238,723 |
| webpack | 3,689,631 (3,224,799 / 6,365 / 458,422 / 45; free 2,957,975) | 3,689,730 | 3,637,884 ("3.6 M") | 244,822 | 273,429 |
| cal-diy | 12,898,357 (11,364,961 / 48,086 / 1,485,265 / 45; free 10,619,699) | trace failed to save (3.3 GB of events) | 12,655,991 ("12.8 M") | 360,976 | n/a |
| vscode | 22,454,713 (19,353,183 / 56,684 / 3,044,801 / 45; free 17,882,274) | trace failed to save (2.96 GB of events) | 22,225,553 ("22.3 M") | 749,005 | n/a |

Instruments and the interposer differ by exactly 99 calls on both projects that recorded: the allocations dyld and
libSystem make before the interposer's constructor runs. mimalloc's bins are 0.5-1.4% lower: three-digit rounding per
bin, and an in-place `realloc` is not a new block there. The interposer counts every `realloc` call as one event.

Only 8% of the allocations are live at exit (xstate-main 238,723 of 3.1 M; webpack 273,429 of 3.7 M). What persists
is dominated by symbol-table buffers: `symbol.rs:348` (`EntryVec::set_capacity`, first allocation) 132,353 live on
xstate-main and 162,143 on webpack, then `SymbolMapExtra` boxes (`symbol.rs:516`), `CopyOnWrite` maps (`cow.rs:17`),
package.json values, `cachedvfs` entries and diagnostics. Instruments attributes a reallocated block to its first
allocation site, so `symbol.rs:351` (the `realloc` line) does not appear in the persistent list.

## Top sites, vscode (22,454,713 allocations; first frame inside a tsrs crate, file:line)

| # | count | % | bytes | mean B | site | what |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 2,168,667 | 9.66 | 127,117,320 | 59 | `tsrs_ast/src/symbol.rs:351` | `EntryVec::set_capacity` realloc (symbol-table growth) |
| 2 | 1,572,999 | 7.01 | 21,764,696 | 14 | `tsrs_ast/src/symbol.rs:348` | `EntryVec::set_capacity` first buffer |
| 3 | 1,153,572 | 5.14 | 353,530,272 | 306 | `tsrs_ast/src/symbol.rs:753` | `SymbolMap::pairs` (the `entries()` snapshot `Vec`) |
| 4 | 596,404 | 2.66 | 15,470,540 | 26 | `tsrs_checker/src/checker_11.rs:858` | `get_named_members` result `Vec` |
| 5 | 483,773 | 2.15 | 54,993,790 | 114 | `tsrs_core/src/tspath/path.rs:632` | `normalize_path` result `String` |
| 6 | 366,613 | 1.63 | 65,329,700 | 178 | `tsrs_checker/src/grammarchecks.rs:1177` | `check_grammar_object_literal_expression` `seen` map (String keys) |
| 7 | 363,658 | 1.62 | 8,220,700 | 23 | `tsrs_checker/src/checker_13.rs:177` | `add_types_to_union` `types` `Vec` |
| 8 | 354,758 | 1.58 | 1,612,788 | 5 | `tsrs_checker/src/inference.rs:1433` | `new_inference_context` inferences collect |
| 9 | 347,560 | 1.55 | 1,530,240 | 4 | `tsrs_checker/src/flow.rs:1817` | `get_union_or_evolving_array_type` `finalized_types` |
| 10 | 339,289 | 1.51 | 32,738,282 | 96 | `tsrs_core/src/tspath/path.rs:284` | `get_directory_path` result `String` |
| 11 | 336,425 | 1.50 | 2,387,488 | 7 | `tsrs_checker/src/checker_11.rs:1492` | `get_conditional_type_instantiation_ex` type-argument collect |
| 12 | 333,176 | 1.48 | 6,326,400 | 19 | `tsrs_checker/src/checker_07.rs:650` | `check_object_literal` `properties_array` |
| 13 | 325,436 | 1.45 | 1,301,744 | 4 | `tsrs_checker/src/checker.rs:574` | `LazyVec::push` first candidate (`vec![item]`) |
| 14 | 321,211 | 1.43 | 6,103,125 | 19 | `tsrs_core/src/jsnum/string.rs:23` | `Number::string` (`i.to_string()`) |
| 15 | 313,553 | 1.40 | 1,329,596 | 4 | `tsrs_checker/src/checker.rs:583` | `LazyVec::to_vec` clone |
| 16 | 310,371 | 1.38 | 658,987 | 2 | `tsrs_scanner/src/utilities.rs:87` | `get_text_of_node_from_source_text` copy |
| 17 | 277,382 | 1.24 | 1,234,576 | 4 | `tsrs_checker/src/inference.rs:1633` | `get_inferred_types` result |
| 18 | 269,664 | 1.20 | 4,322,880 | 16 | `tsrs_checker/src/checker_10.rs:677` | `get_signatures_of_symbol` result |
| 19 | 267,340 | 1.19 | 34,754,104 | 130 | `tsrs_core/src/tspath/path.rs:15` | `Path::new` `Arc<str>` (`to_path`) |
| 20 | 266,445 | 1.19 | 1,139,732 | 4 | `tsrs_checker/src/inference.rs:1710` | `union_object_and_array_literal_candidates` `candidates.to_vec()` |
| 21 | 244,645 | 1.09 | 2,053,716 | 8 | `tsrs_checker/src/checker_11.rs:1932` | `instantiate_list::<P<Symbol>>` changed-list `Vec` |
| 22 | 235,737 | 1.05 | 4,431,920 | 19 | `tsrs_checker/src/checker_13.rs:1139` | `filter_type` collect |
| 23 | 232,542 | 1.04 | 3,118,076 | 13 | `tsrs_ast/src/symbol.rs:899` | `SymbolTable::values` snapshot (`check_unused_locals_and_parameters`) |
| 24 | 225,984 | 1.01 | 17,412,240 | 77 | `tsrs_checker/src/checker_09.rs:2995` | `every_lazy_property` `seen` set |
| 25 | 211,410 | 0.94 | 23,316,494 | 110 | `tsrs_core/src/tspath/path.rs:124` | `combine_paths` result `String` |
| 26 | 207,567 | 0.92 | 967,220 | 5 | `tsrs_checker/src/flow.rs:1797` | `get_type_at_flow_branch_label` `antecedent_types[..].to_vec()` |
| 27 | 201,695 | 0.90 | 19,275,520 | 96 | `tsrs_checker/src/checker_09.rs:2838` | `prepare_lazy_members` `unaffected` |
| 28 | 191,776 | 0.85 | 6,952,352 | 36 | `tsrs_checker/src/checker_13.rs:251` | `add_types_to_union` `run_starts` |
| 29 | 185,231 | 0.82 | 740,924 | 4 | `tsrs_checker/src/types.rs:1771` | `TypeExt::distributed` `vec![self]` |
| 30 | 176,506 | 0.79 | 10,656,084 | 60 | `tsrs_core/src/collections/ordered_set.rs:20` | `create_union_or_intersection_property` `prop_set` |
| 31 | 171,444 | 0.76 | 732,672 | 4 | `tsrs_checker/src/checker_11.rs:1932` | `instantiate_list::<P<Signature>>` |
| 32 | 164,643 | 0.73 | 2,778,912 | 17 | `tsrs_checker/src/checker_10.rs:720` | `get_signature_from_declaration` `parameters` |
| 33 | 136,627 | 0.61 | 2,186,032 | 16 | `tsrs_checker/src/flow.rs:1863` | `get_type_at_flow_loop_label` `antecedent_types` |
| 34 | 135,661 | 0.60 | 2,616,440 | 19 | `tsrs_checker/src/checker_04.rs:2037` | `check_array_literal` `element_infos` |
| 35 | 135,661 | 0.60 | 1,308,220 | 10 | `tsrs_checker/src/checker_04.rs:2036` | `check_array_literal` `element_types` |
| 36 | 130,889 | 0.58 | 528,644 | 4 | `tsrs_checker/src/checker_02.rs:1600` | `symbol.declarations().to_vec()` |
| 37 | 121,674 | 0.54 | 16,428,736 | 135 | `tsrs_compiler/src/filesparser.rs:79` | `parseTask` path `String` |
| 38 | 121,098 | 0.54 | 20,586,105 | 170 | `tsrs_module/src/resolver.rs:1828` | `try_extension` `format!` |
| 39 | 120,277 | 0.54 | 1,924,432 | 16 | `tsrs_checker/src/checker_05.rs:2073` | `get_contextual_signature` `signature_list` |
| 40 | 116,151 | 0.52 | 4,579,767 | 39 | `tsrs_module/src/resolver.rs:362` | resolution cache key `module_name.to_string()` |

Per function (lines merged): `EntryVec::set_capacity` 3,741,666 (16.7%), `SymbolMap::pairs` 1,153,572 (5.1%),
`get_named_members` 596,404, `add_types_to_union` 555,434, `normalize_path` 483,773, the grammar `seen` map 366,653,
`new_inference_context` 354,758, `get_union_or_evolving_array_type` 349,094, `get_directory_path` 341,293,
`get_conditional_type_instantiation_ex` 336,425, `check_object_literal` 333,176, `LazyVec::push` 329,900,
`Number::string` 321,211, `LazyVec::to_vec` 313,553, `get_text_of_node_from_source_text` 310,371. 920 functions in
all; the 40th has 0.5%.

The same sites lead on xstate-main (3.1 M: `pairs` 7.6%, `set_capacity` 10.9%, `get_conditional_type_instantiation_ex`
4.6%, `every_lazy_property` 3.8%, `instantiate_list` 5.4%), webpack (3.7 M: `set_capacity` 11.8%, `pairs` 4.5%; the
JSDoc parser's `skip_whitespace_or_asterisk` `indents` `Vec` 3.5% and the `Rc` visitor hooks of `deepclone.rs`
4 × 49,255 are webpack-only) and cal-diy (12.9 M: `instantiate_list` 7.5%, `set_capacity` 8.3%,
`get_intersection_type_ex` 3.1%, template-literal `add_template_spans` 3.7%, path strings 25%).

## Mapping onto the arena-candidate inventory (vscode / xstate-main, share of all allocations)

| candidate | vscode | xstate-main | sites |
| --- | --- | --- | --- |
| SymbolTable `EntryVec` buffer | 3,891,500 (17.3%) | 352,177 (11.3%) | `symbol.rs:348/351`, `SymbolMapExtra` box `:516/:717` |
| `SymbolTable::entries()/values()/keys()` snapshots | 1,396,550 (6.2%) | 238,625 (7.7%) | `symbol.rs:753` (`pairs`), `:899`, `:895` |
| `LazyVec` candidate lists | 727,479 (3.2%) | 88,463 (2.8%) | `checker.rs:574/583` |
| small per-target instantiation tables | 84,876 (0.4%) | 8,846 (0.3%) | `types.rs:2215`, `packedmap.rs:60/79` (few allocations, 93 MB) |
| String keys / `to_string` membership tests | 735,401 (3.3%) | 76,066 (2.4%) | `Number::string`, `get_text_of_node_from_source_text`, `relater_1.rs:1623` |
| return stored slices (`instantiate_list` unchanged copy, `get_variances`) | 815,175 (3.6%) | 245,792 (7.9%) | `checker_11.rs:1932/1941`, `relater_1.rs:1953` |
| member-resolution build-then-copy | 655,970 (2.9%) | 103,841 (3.3%) | `checker_09.rs:2838/2854`, `relater_1.rs:1433` |
| per-call hash maps | 1,206,389 (5.4%) | 280,759 (9.0%) | grammar `seen`, `every_lazy_property`, `prop_set`, `some_property_reduces_to_never` |
| flow label lists | 729,599 (3.2%) | 3,808 (0.1%) | `flow.rs:1817/1797/1863` |
| `flow_type_cache` per nested check | 11,194 | 101 | `checker_04.rs:1270` |
| `LazyMappedTable` | 5,792 | 1,326 | `checker_10.rs:2131` |
| `InferMemo` | 19,198 | 3,333 | `infermemo.rs` |
| link-store chunks | 1,620 (122 MB) | 1,169 | `links.rs:100/202` |
| diagnostic argument strings | 53,037 | 37,917 | `diagnostic.rs:229`, node builder |
| other per-call checker lists (union/inference/conditional/call/expression) | 7,462,578 (33.2%) | 1,077,783 (34.6%) | the rows above from `checker_11.rs:858` down |
| front end: path strings, resolver, package.json | 3,274,169 (14.6%) | 390,163 (12.5%) | `path.rs:632/284/15/124`, `resolver.rs:1828/361/362`, `filesparser.rs:79` |
| front end: parser, scanner, binder | 453,918 (2.0%) | 67,468 (2.2%) | `jsdoc.rs:439`, `binder.rs:923`, `ast.rs:97/98` |
| unassigned | 930,268 (4.1%) | 136,452 (4.4%) | `relater_2.rs:321`, `extension.rs:197`, `utilities.rs:1358`, `checker_11.rs:2435/2446` |

Not in the inventory and worth a row: the contextual-type constituent lists of `checker_15.rs:795/666` (60,046 and
49,142 on xstate-main, 1.9% and 1.6%); `checker_14.rs:1561/1562/1490` template-literal spans (cal-diy 640 K, 5%);
`checker_02.rs:1600` `declarations().to_vec()` (131 K on vscode); `get_signatures_of_symbol` / `get_signature_from_declaration`
result lists (434 K on vscode); the path helpers that #272's path commit left (`normalize_path`, `get_directory_path`,
`combine_paths`, `to_path`: 1.3 M on vscode, 3.3 M incl. the resolver on cal-diy); `resolver.rs:361/362` cache-key
strings (2 × 116 K on vscode); `os.rs:58` a boxed `DirFS` per `root_for` call (67 K on vscode, 116 K on cal-diy);
`jsdoc.rs:439` (130 K on webpack); `tsrs_core/src/arena.rs:204` `track_drop` bookkeeping in leaf regions (12 K / 18 MB).

## The allocator's share of cycles (Time Profiler, mimalloc build, vscode single-threaded)

7,520 samples at 1 ms over a 9.14 s run (main thread + the checker thread): innermost frame in mimalloc 148 (1.97%:
`mi_malloc_aligned` 39, `mi_free` 32, `_mi_theap_realloc_zero` 22, `_mi_page_malloc_zero` 11, page free-list and
collect 18), Rust allocator shims (`RawVec::grow`, `finish_grow`) 7 (0.09%), `memmove` inside a reallocation 18
(0.24%), hashbrown rehash 3 (0.04%); a mimalloc frame anywhere in the innermost eight frames: 177 (2.35%). So removing
every malloc and free from a single-threaded vscode check can buy at most about 2.4% of samples, before the work that
replaces them (bump allocation, free-list pushes, the copies that growth still needs) is paid for. Per-allocation cost
implied by the sample share: 2.35% × 9.14 s / 22.5 M ≈ 10 ns per allocation on this machine.

## Caveats

- Counts are exact and repeatable (a second xstate-main run gave the same totals); the Time Profiler share is one
  recording on a machine that was also building another checkout.
- The interposer's stacks are return addresses minus one, symbolicated through `atos -i`; a site inside a generic or
  closure is named by the inlined frame (`pairs [in entries]`) and the enclosing real symbol.
- Instruments' Allocations List export holds persistent allocations only; its Statistics detail is the only exact
  total it exposes, and traces above about 2 GiB of events cannot be saved (cal-diy, vscode). Without Developer Mode
  the target must be re-signed with `get-task-allow` or xctrace hangs in `liboainject.dylib` before `main`.
- `/usr/bin/time` and `env` are SIP-restricted: a `DYLD_INSERT_LIBRARIES` run started through them is silently not
  interposed (the output file is not written).
