# lint-paydown-compiler: the lint ratchet in the compiler-side crates

Scope: every workspace crate except the project / language-service crates, `tsrs_api*` and the generated
`tsrs_fourslash` (#57 and two compiler-side branches). Goal: a smaller `tools/lint/baseline.tsv` with identical output
and no instruction cost. Result: 559 baseline findings in these crates -> 5; `#[expect]` cleared 64 of them, each with
a reason (42 `iter_over_hash_type` loops where Go itself ranges over a map and the order cannot be seen, the 8
measured unchecked operations below, 14 sites intended as written). No wrong output was found. Instructions and peak
memory on the 38k-file codebase were unchanged within noise (cumulative: -0.06% / -0.18% instructions with 1 / 4
checkers, peak -0.06% / +0.03%): the removed clones and the forced inlining are not on paths that show up at
this scale. Status (2026-10-10): the whole baseline is now 3 findings in 2 rows (tools/lint/baseline.tsv). Of the 5 left
here, the two `needless_pass_by_value` on `get_module_specifiers_for_file_with_info` / `update_module_specifier`
remain (specifiers.rs); the other three are gone.

One gate observation that recurs: in `cargo test -p tsrs_cli` tsctests, one output file differs in an
`@iterator@<symbol id>` name, which also differs between two runs of the same binary.

## Real bugs and determinism fixes

No wrong output was found on the gates (conformance, emit and fourslash result trees identical), but five places let hash order decide a
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

## inline_always: measured

No note measured the 43 `#[inline(always)]` (41 flagged, plus two in the `plain-ptrs` branches of ptr.rs) one by one.
All were turned into `#[inline]` together and measured against the same commit (release profile, medians of 3
interleaved runs, load 9-13, counters identical):

| | `inline(always)` | `inline` | delta |
| --- | --- | --- | --- |
| 1 checker, instructions | 290.47 G | 290.27 G | -0.07% |
| 4 checkers, instructions | 392.64 G | 391.59 G | -0.27% (lower in each pair) |

LLVM inlines these small accessors and arena fast paths without being forced, so none needs an `#[expect]`.
