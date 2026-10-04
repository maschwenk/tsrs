# lint-paydown-compiler: the lint ratchet in the compiler-side crates

Scope: every workspace crate except `tsrs_project`, `tsrs_ls`, `tsrs_lsp`, `tsrs_lsproto`, `tsrs_fswatch`
(notes/lint-paydown-project.md), `tsrs_api*` and the generated `tsrs_fourslash`. Goal: a smaller
`tools/lint/baseline.tsv` with identical output and no instruction cost. Branches: `lint/fix-main-ratchet` (#57, the
six findings over baseline on main), `lint/compiler-side-1` (bugs and determinism, clones, argument passing),
`lint/compiler-side-2` (stacked: `unsafe` hygiene, `inline_always`, `allow_attributes`).

Result: 559 baseline findings in these crates -> 5 (#57: 23, part 1: 349, part 2: 182). `#[expect]` cleared 64 of
the 531 on the two compiler-side branches (12%). No wrong output was found; five places where hash order could pick
a visible result now use a fixed order (below). Instructions and peak memory unchanged (below).

## Per lint

Counts are baseline findings in these crates: before = 19c6349, after = compiler-side-2.

| Lint | Before | After | Fixed | `#[expect]` | Part | How |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| `clone_on_ref_ptr` | 128 | 0 | 128 | 0 | 1 | `x.clone()` -> `Rc::clone(&x)` / `Arc::clone` / `Weak::clone`; three sites that coerce to `Arc<dyn FS>` name the concrete type. |
| `trivially_copy_pass_by_ref` | 53 | 0 | 53 | 0 | 1 | The transformers' `Resolver` (two 4-byte handles, `Copy`): its methods take `self`. |
| `iter_over_hash_type` | 52 | 2 | 8 | 42 | 1 | Below. |
| `ref_as_ptr`, `borrow_as_ptr`, `ptr_as_ptr`, `ptr_cast_constness` | 81 | 0 | 81 | 0 | #57, 2 | `std::ptr::from_ref` / `from_mut`, `&raw const` / `&raw mut` (also the `ptr::eq(message, &raw const diagnostics::X)` identity tests), `.cast()`, `.cast_mut()`. Same pointers. |
| `assigning_clones` | 41 | 0 | 41 | 0 | 1 | `a = b.clone()` -> `a.clone_from(&b)`; checked that no site borrows the same `RefCell` on both sides. |
| `inline_always` | 41 | 0 | 41 | 0 | 2 | All -> `#[inline]`, measured (below). |
| `unsafe_op_in_unsafe_fn` | 34 | 0 | 34 | 0 | 2 | Each unsafe op in an `unsafe fn` is in its own block whose SAFETY comment names the part of the function's contract it uses. The `plain-ptrs` branches too (`cargo check --features tsrs_core/plain-ptrs` is clean). |
| `needless_pass_by_value` | 31 | 3 | 27 | 1 | 1 | Borrowed where only read, `Copy` derived for all-`Copy` structs, moved instead of cloned where the callee can consume (below). `io_error` keeps its signature for `map_err(io_error)`. |
| `undocumented_unsafe_blocks` | 27 | 0 | 27 | 0 | 2 | Below. |
| `allow_attributes` + `_without_reason` | 22 | 0 | 22 | 0 | 2 | Removed, not converted: `let_unit_value` and `too_many_arguments` are in `clippy::all` (allow), `non_upper_case_globals` / `non_camel_case_types` are allowed in `[workspace.lints.rust]`, so all 11 `#[allow]`s were no-ops. |
| `disallowed_methods` | 8 | 0 | 0 | 8 | 1 | Each unchecked operation measured (below); all kept. |
| `cast_ptr_alignment` | 6 | 0 | 2 | 4 | 1 | Length prefixes read with `read_unaligned` (one load either way); node-header and free-list casts keep the cast with the alignment invariant as the reason. |
| `implicit_clone`, `redundant_clone`, `unnecessary_to_owned` | 13 | 0 | 13 | 0 | 1 | Dropped `.to_string()` / `.clone()` / `.copied()` of values already owned or `'static`. |
| `mut_from_ref` | 4 | 0 | 0 | 4 | 1 | The arena's `alloc_*` (`&self -> &mut T`): a bump allocator returns a fresh block per call, as bumpalo's `alloc` does. |
| `non_send_fields_in_send_ty` | 3 | 0 | 0 | 3 | 1 | The by-decree `Send` impls (`RegionInner`, `PooledChecker`, `CheckerSlot`), each with its SAFETY argument. |
| `large_enum_variant` | 2 | 0 | 1 | 1 | 1 | Test runner `Compiled::Result` boxed; `programCheckerPool` kept (one per program, the usual variant). |
| `missing_safety_doc` | 2 | 0 | 2 | 0 | 2 | `free_program` / `free_unshared_program` had `// # Safety` comments; now doc comments. |
| `disallowed_types`, `zero_sized_map_values` | 4 | 0 | 4 | 0 | 1 | std `HashMap` -> `FxHashMap` (never iterated); `FxHashMap<&str, ()>` -> `FxHashSet`. |
| single findings (`clone_on_copy`, `manual_contains`, `map_entry`, `set_contains_or_insert`, `stable_sort_primitive`, `empty_line_after_doc_comments`, `misnamed_getters`) | 7 | 0 | 6 | 1 | 1 | `misnamed_getters`: Go's `Generator.Sources()` returns `rawSources`. |
| `significant_drop_in_scrutinee` | 0 (1 on main) | 0 | 1 | 0 | #57 | `new_slab` pops the slab cache into a local, so the lock is released before the `Box` allocation. Not a deadlock before (the body allocates with the global allocator, not the arena). |

`#[expect]` share: 64 of 531 (12%), within the ~15% guide over both branches, though part 1 alone was 18% (64 of
349): 42 `iter_over_hash_type`, where Go itself ranges over a map, the order cannot be seen and there is no Go order
to switch to; 8 measured unchecked operations; 14 single sites whose code is intended as written (the arena allocator
signatures, alignment casts with an invariant, the three by-decree `Send` impls, two Go-shaped items).

Left (5): `iter_over_hash_type` in the two `Range` methods of `tsrs_projectutil`'s dirty maps, and
`needless_pass_by_value` on `Program::emit`, `get_module_specifiers_for_file_with_info` and `update_module_specifier`
(below). Rows lowered by hand (files with platform `cfg`s whose fixed lines are not gated; Linux has no finding there
either): `checkerpool.rs` (non_send_fields, borrow_as_ptr, ptr_cast_constness, ref_as_ptr, undocumented_unsafe_blocks),
`osvfs/os.rs` (assigning_clones, which is in the macOS-only `walk_symlinks`, and needless_pass_by_value),
`cli/lsp.rs` (needless_pass_by_value), `reserve.rs` (inline_always).

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

## undocumented_unsafe_blocks and unsafe_op_in_unsafe_fn

Most of the 27 undocumented sites were the second line of an `unsafe impl Send` / `unsafe impl Sync` pair whose
first line had the comment; each now says what makes its own impl sound (the by-decree `P` contract, a `&'static
str`, the checker reached only under its mutex, a buffer only read through `&`). Three SAFETY comments sat above a
struct literal instead of on the field whose initializer is the `unsafe` block (`FrozenCell` borrows), so clippy did
not see them; they moved to the field. The 34 `unsafe_op_in_unsafe_fn` blocks state what the caller's contract
provides at that line: the `P` handle constructors rely on the reservation never handing out its first granule (so a
handle is never 0); the free paths name the `Box::leak` the pointer came from; the arena free list relies on
`free_class` (size a multiple of 8) and the 8-alignment check before `push_free`.

## inline_always: measured

No note measured the 43 `#[inline(always)]` (41 flagged, plus two in the `plain-ptrs` branches of ptr.rs) one by one.
All were turned into `#[inline]` together and measured against the same commit (release profile, medians of 3
interleaved runs, load 9-13, counters identical):

| | `inline(always)` | `inline` | delta |
| --- | --- | --- | --- |
| 1 checker, instructions | 290.47 G | 290.27 G | -0.07% |
| 4 checkers, instructions | 392.64 G | 391.59 G | -0.27% (lower in each pair) |

LLVM inlines these small accessors and arena fast paths without being forced, so none needs an `#[expect]`.

## Cost

Instructions retired and peak memory footprint on the 38k-file codebase, medians of 3 interleaved runs; Types /
Symbols / Instantiations identical in every run.

Part 1: main 19c6349 vs f3205b0 (load 15-25):

| | main | part 1 | delta |
| --- | --- | --- | --- |
| 1 checker, instructions | 290.12 G | 289.89 G | -0.08% |
| 1 checker, peak | 4.241 GiB | 4.246 GiB | +0.1% |
| 4 checkers, instructions | 391.62 G | 391.93 G | +0.08% |
| 4 checkers, peak | 5.694 GiB | 5.693 GiB | 0.0% |

Cumulative: main 19c6349 vs compiler-side-2 (aac1080 code, the inline change and the hand-lowered rows included;
load 8-19):

| | main | part 2 | delta |
| --- | --- | --- | --- |
| 1 checker, instructions | 290.03 G | 289.86 G | -0.06% |
| 1 checker, peak | 4.243 GiB | 4.241 GiB | -0.06% |
| 4 checkers, instructions | 392.23 G | 391.54 G | -0.18% |
| 4 checkers, peak | 5.689 GiB | 5.691 GiB | +0.03% |

All within run-to-run noise (about +-1% one checker, +-0.5% four): the removed clones and forced inlining are not on
paths that show up at this scale.

## Gates

Part 1, against `origin/main` 1ab17ae built in the same worktree; part 2, against part 1 as merged (bc434d0, whose
code is part 1's): `--baselines types,symbols` (default and `TSRS_LAZY_MEMBERS=0`) 13,458 (2 codes, 2 fail) / 12,779
/ 12,779, `--baselines js,jsmap,sourcemap` 13,462 / 13,392 / 149 / 156, fourslash 4,066 pass / 63 fail: all result
trees identical (summary.json apart from timings). `cargo test -p tsrs_cli` (memory_tests cfg'd out locally, as on
main): tsctests tsc 187/216, tsbuild 187/190, 0 crashes, pass/fail lists identical; one output file differs in a
`@iterator@<symbol id>` name, which also differs between two runs of the same binary. default_emit 6/6.
`cargo test -p tsrs_core` (dev and release) 91 / 90 pass. `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace`
clean, ratchet ok.
