# lint-paydown-compiler: the lint ratchet in the compiler-side crates

Scope: every workspace crate except `tsrs_project`, `tsrs_ls`, `tsrs_lsp`, `tsrs_lsproto`, `tsrs_fswatch`
(notes/lint-paydown-project.md), `tsrs_api*` and the generated `tsrs_fourslash`. Goal: a smaller
`tools/lint/baseline.tsv` with identical output and no instruction cost. Branches: `lint/fix-main-ratchet` (#57, the
six findings over baseline on main), `lint/compiler-side-1` (bugs and determinism, clones, argument passing),
`lint/compiler-side-2` (stacked: `unsafe` hygiene, `inline_always`, `allow_attributes`).

## Per lint (compiler-side-1)

Counts are baseline findings in these crates: before = 19c6349, after = this branch (#57 included).

| Lint | Before | After | Fixed | `#[expect]` | How |
| --- | ---: | ---: | ---: | ---: | --- |
| `clone_on_ref_ptr` | 128 | 0 | 128 | 0 | `x.clone()` -> `Rc::clone(&x)` / `Arc::clone` / `Weak::clone`; three sites that coerce to `Arc<dyn FS>` name the concrete type. |
| `trivially_copy_pass_by_ref` | 53 | 0 | 53 | 0 | The transformers' `Resolver` (two 4-byte handles, `Copy`): its methods take `self`. |
| `iter_over_hash_type` | 52 | 2 | 8 | 42 | Below. |
| `assigning_clones` | 41 | 0 | 41 | 0 | `a = b.clone()` -> `a.clone_from(&b)`; checked that no site borrows the same `RefCell` on both sides. |
| `needless_pass_by_value` | 31 | 3 | 27 | 1 | Borrowed where only read, `Copy` derived for all-`Copy` structs, moved instead of cloned where the callee can consume (below). `io_error` keeps its signature for `map_err(io_error)`. |
| `disallowed_methods` | 8 | 0 | 0 | 8 | Each unchecked operation measured (below); all kept. |
| `cast_ptr_alignment` | 6 | 0 | 2 | 4 | Length prefixes read with `read_unaligned` (one load either way); node-header and free-list casts keep the cast with the alignment invariant as the reason. |
| `implicit_clone`, `redundant_clone`, `unnecessary_to_owned` | 13 | 0 | 13 | 0 | Dropped `.to_string()` / `.clone()` / `.copied()` of values already owned or `'static`. |
| `mut_from_ref` | 4 | 0 | 0 | 4 | The arena's `alloc_*` (`&self -> &mut T`): a bump allocator returns a fresh block per call, as bumpalo's `alloc` does. |
| `non_send_fields_in_send_ty` | 3 | 0 | 0 | 3 | The by-decree `Send` impls (`RegionInner`, `PooledChecker`, `CheckerSlot`), each with its existing SAFETY argument. |
| `large_enum_variant` | 2 | 0 | 1 | 1 | Test runner `Compiled::Result` boxed; `programCheckerPool` kept (one per program, the usual variant). |
| `disallowed_types`, `zero_sized_map_values` | 4 | 0 | 4 | 0 | std `HashMap` -> `FxHashMap` (never iterated); `FxHashMap<&str, ()>` -> `FxHashSet`. |
| single findings (`clone_on_copy`, `manual_contains`, `map_entry`, `set_contains_or_insert`, `stable_sort_primitive`, `empty_line_after_doc_comments`, `misnamed_getters`) | 7 | 0 | 6 | 1 | `misnamed_getters`: Go's `Generator.Sources()` returns `rawSources`. |
| `ref_as_ptr`, `borrow_as_ptr`, `ptr_as_ptr`, `ptr_cast_constness` | 81 | 57 | 24 | 0 | #57 only (arena.rs, ptr.rs, types.rs); the rest is compiler-side-2. |

Not touched here: `inline_always` 41, `unsafe_op_in_unsafe_fn` 34, `undocumented_unsafe_blocks` 26,
`allow_attributes` + `_without_reason` 22, `missing_safety_doc` 2.

Total: 559 -> 187 (-372, of which 23 in #57). `#[expect]`: 64 of the 349 cleared on this branch (18%): 42
`iter_over_hash_type`, 8 measured unchecked operations, 14 single sites whose code is intended as written (the arena
allocator signatures, alignment casts with an invariant, the three by-decree `Send` impls, two Go-shaped items). That is
above the ~15% the brief allows; the excess is `iter_over_hash_type`, where Go itself ranges over a map, the order
cannot be seen, and there is no Go order to switch to, so the alternative was to leave them in the baseline.

## Real bugs and determinism fixes

No wrong output was found on the gates (the trees below are identical), but five places let hash order decide a
result that could be seen. Go has the same randomness at each (it ranges over a Go map), so these make the port pick
one answer, not match Go:

- `get_primitive_type_alias_suggestions` (checker.go:1804, `primitiveTypeAliasSuggestions` is a map): the spelling
  suggestion for a global-lookup name took the first of two equally close primitive aliases in hash order. Now a
  list in declaration order.
- `PackageJson::range_dependencies` (packagejson.go `RangeDependencies` ranges over maps): string completions list
  dependency names in the order given. Each field's names are now visited sorted.
- `vfstest` `get_following_symlinks_worker` (vfstest.go:271): a path under two nested symlinks resolved through
  whichever came first in the map. Now the outer one, as a file system resolves it.
- scanner `token_to_text` (scanner.go:2254): filled from the `textToToken` map; built from the token tables now
  (checked: no two texts share a kind, so the result is the same).
- checker `find_in_map` (utilities.go:38) had no caller: `report_non_exported_member` walks the ordered symbol table.

## `iter_over_hash_type`

Each site was read next to its Go loop. All 52 range over a Go map or set in Go too, so there is no Go order to
copy. 42 got `#[expect]` with the reason the order cannot be seen: the loop only stores, deletes or ORs per key, adds
to a set, or adds diagnostics to a collection that drops duplicates and sorts on read with a comparator that only
ties on equal diagnostics (`DiagnosticsCollection::get_diagnostics`, `filter_and_sort_diagnostics`). Left (2): the
`Range` methods of `tsrs_projectutil`'s dirty maps hand their order to ~20 `tsrs_project` callers, some of which pick
the first match; whether that order matters is language-server work.

## `disallowed_methods`: the unchecked operations, measured

The 38k-file codebase, `--noEmit --incremental false`, instructions retired (`/usr/bin/time -l`), medians of 3
interleaved runs against the same commit with the operation replaced by its checked form:

| Site | Checked form | 1 checker | 4 checkers |
| --- | --- | --- | --- |
| `identifier::source_text` | `from_utf8` | 291 G -> 1,247 G (+328%: validates the whole file text per call) | |
| `Identifier::text` | `from_utf8` | 290.4 -> 295.2 G (+1.6%) | |
| `PackedStr::as_str`, `OwnedStrCell::get` | `from_utf8` | +5.2% (the three `from_utf8` together +6.8%, minus `Identifier::text`) | |
| link stores (`LinkStore::at`, `slot`, `IdLinkStore::at`) | indexing | 292.3 vs 295.3 G (noise) | 392.3 -> 394.5 G (+0.6%, every pair) |
| `Symbol::value_declaration` | indexing | +0.7% (notes/mem-small.md) | |

All are above the 0.3% threshold and stay unchecked, each with an `#[expect]` citing its row.

## needless_pass_by_value: what changed and what is left

Borrowed: the relater's `excluded_properties` (the set was cloned for every matching type in
`typeRelatedToDiscriminatedType`'s loop), the node builder's `sort_by_best_name` (cloned two `(symbol, String)`
pairs per comparison), `add_sub_task`'s `resolvedRef`, `filter_and_sort_diagnostics`, `emit_files`' options, the
pending-emit path, the parse-cache key and others. Moved instead of cloned: `new_resolution_data` takes the resolver
options and moves their three strings. Left (3): `Program::emit` and the module-specifier entry points
`get_module_specifiers_for_file_with_info` / `update_module_specifier`, whose callers are in `tsrs_api` / `tsrs_ls`.

## Cost

Instructions retired and peak memory footprint on the 38k-file codebase, main (19c6349) vs this branch before the
merge of main (f3205b0), medians of 3 interleaved runs, load 15-25; Types / Symbols / Instantiations identical:

| | main | branch | delta |
| --- | --- | --- | --- |
| 1 checker, instructions | 290.12 G | 289.89 G | -0.08% |
| 1 checker, peak | 4.241 GiB | 4.246 GiB | +0.1% |
| 4 checkers, instructions | 391.62 G | 391.93 G | +0.08% |
| 4 checkers, peak | 5.694 GiB | 5.693 GiB | 0.0% |

All within run-to-run noise (about +-1% / +-0.5%): the removed clones are not on paths that show up at this scale.

## Gates (compiler-side-1)

Against `origin/main` 1ab17ae built in the same worktree: `--baselines types,symbols` (default and
`TSRS_LAZY_MEMBERS=0`) 13,458 / 12,779 / 12,779, `--baselines js,jsmap,sourcemap` 13,462 / 13,392 / 149 / 156,
fourslash 4,066 pass / 63 fail: all result trees identical. `cargo test -p tsrs_cli` (memory_tests cfg'd out
locally): tsctests tsc 187/216, tsbuild 187/190, 0 crashes, pass/fail lists identical; one output file differs in a
`@iterator@<symbol id>` name, which also differs between two runs of the same binary. default_emit 6/6.
