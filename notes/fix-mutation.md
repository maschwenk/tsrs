# fix-project: mutation testing against Project

Goal: show that `tsrs` reports the same diagnostics as `tsgo-ref` on *wrong* code, not just 0 errors on Project.

## Harness (`tools/mutate/`)

- `mutate.py batch --seed N --count 280 [--heavy F] [--weights m=w,...]` picks N-seeded random program files (default
  45% "generics-heavy": zod schemas, ORM entities/repositories, workflows/activities, router/endpoint definitions;
  15% tests; rest random), applies one seeded mutation per file (one contiguous edit, checked against the pristine text
  and re-parsed so every file stays syntactically valid; logged in `mutations.json` with file/line/col/orig/repl),
  runs both compilers on the mutated clone, restores, and diffs the multiset of `(file, line, col, code)` plus the full
  message text (chain lines included).
- `rerun <batch> [--only ids] [--out dir]`, `bisect <batch> --key '[file,line,col,code]'`, `restore`, `summary`.
- `MUT_OURS=<binary>` switches our compiler; used to rerun everything with a node-builder build (real type printer).
- Mutators: prop_rename, prop_drop, str_change, swap_args, remove_arg, add_arg, remove_await, num_to_str,
  delete_return, rename_import_use, eq_incompat, remove_as, generic_arg, drop_type_args, flip_optchain (incl.
  removing `!`), remove_nullish, const_assign (const and readonly), remove_case, bad_method, member_sig
  (extends/implements members), zod_swap, prop_type_change, optional_toggle, remove_async.

## Batches

Batches 1-12 were first compared on keys with the placeholder printer (origin/main), then all 15 were compared on full
message text with `nb-integrate` at add8ff4 + its uncommitted printer wiring (same base as main d1170eb), frozen copy.
Batches 13-15 used `--heavy 0.7` and weights favoring the rarer mutators. Batch 16 ran main after parallel checking
became the default (4 checkers) to cover the checker-pool path.

| batch | sites | files | ref diags | key diffs (placeholder printer) | full-text diffs (node-builder build) |
| --- | --- | --- | --- | --- | --- |
| 1 | 238 | 238 | 761 | 0 | 0 |
| 2 | 279 | 279 | 505 | 2 missing / 2 extra | 0 |
| 3 | 284 | 284 | 516 | 0 | 0 |
| 4 | 281 | 281 | 504 | 0 | 0 |
| 5 | 282 | 282 | 552 | 1 / 1 | 0 |
| 6 | 276 | 276 | 858 | 0 | 0 |
| 7 | 284 | 284 | 762 | 1 / 1 | 0 |
| 8 | 283 | 283 | 732 | 0 | 0 |
| 9 | 278 | 278 | 746 | 0 | 0 |
| 10 | 278 | 278 | 520 | 1 / 1 | 0 |
| 11 | 281 | 281 | 425 | 1 / 2 | 0 |
| 12 | 281 | 281 | 1737 | 0 | 0 |
| 13 | 263 | 263 | 643 | - | 0 |
| 14 | 265 | 265 | 414 | - | 0 |
| 15 | 264 | 264 | 698 | - | 0 |
| 16 | 262 | 262 | 695 | 2 / 2 (cause 1; main 8a148e0, 4 parallel checkers) | 0 |
| total | 4,379 | 4,379 distinct | 11,068 | 9 / 10, 2 causes | 0 |

Mutator mix: rename_import_use 420, add_arg 335, bad_method 325, remove_arg 318, str_change 313, prop_rename 277,
prop_drop 275, remove_async 250, remove_await 206, generic_arg 197, swap_args 173, num_to_str 159, delete_return 149,
flip_optchain 138, zod_swap 110, eq_incompat 93, optional_toggle 83, member_sig 81, const_assign 66,
prop_type_change 58, remove_as 48, drop_type_args 22, remove_case 12, remove_nullish 9.

## Divergences and root causes

Both clusters are artifacts of the placeholder printer; neither is a checker bug.

1. **TS2345/TS2322 head kept where Go reports TS2740/TS2739 alone** (6 sites: MikroORM `em.getReference` overload
   failures in `orm/mongoMigration/translations/*`, `checkoutSettingsCollectionService.ts`; `Response` vs `Promise`
   in `get.test.ts`). `reportRelationError` (relater.go:4855) drops the head message when the next chain entry is a
   missing-properties message and `chainArgsMatch(generalizedSourceType, targetType)` holds; that compares *printed
   strings*. The source that fails is a different type object than the argument type (e.g. the union from the
   failed-overload return vs its `{ id: string } & Reference<...>` member) but prints identically in Go; the
   placeholder printer prints `type#<id>`, so the strings differ and the head survives. Standalone repro:
   `testdata/regressions/missing-props-head-suppression/` (expected output from tsgo-ref; the node-builder build
   matches it byte for byte, main does not yet).
2. **Duplicate TS2741** (`locationSplits.module.ts`): two identical messages at one position except for placeholder
   ids; Go's `SortAndDeduplicateDiagnostics` compares message text, so it keeps one.

Divergences left open: none.

## Counter delta (unmutated Project)

`--extendedDiagnostics`, reference `--singleThreaded`:

| | tsgo-ref | tsrs main (placeholder printer) | tsrs node-builder build |
| --- | --- | --- | --- |
| Files | 37,942 | 37,942 | 37,942 |
| Lines / Identifiers | 5,941,654 / 7,329,452 | same | same |
| Symbols | 25,973,354 | 25,966,437 | 25,973,354 |
| Types | 9,639,962 | 9,637,422 | 9,639,962 |
| Instantiations | 44,884,281 | 44,879,286 | 44,884,281 |

The node-builder build matches every counter exactly, so the main-branch delta (6,917 symbols, 2,540 types,
4,995 instantiations) is entirely types/symbols the Go node builder creates while printing (the checker prints
speculatively, e.g. for error messages in discarded overload candidates and `typeToString` calls that feed
diagnostics). A per-file type-creation bisect is unnecessary. (The previous agent's per-file comparison showed the
expected shape: some files where Go creates more types, and later files where ours creates a few more because Go
found them in caches its printer had already filled.)
