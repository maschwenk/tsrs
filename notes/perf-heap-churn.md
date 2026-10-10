# perf-heap-churn: short-lived heap allocations in a single-threaded check

Goal: cut the heap allocations a check makes and frees almost at once, without changing any output. vscode `-p src`,
one checker (`--singleThreaded`, `RAYON_NUM_THREADS=1`), on an M-series Mac (macOS, 18 cores, other agents building
at the same time), against main at e8c9f1f5 (the code of 15dd87f6; the branch is rebased onto 5a19aafa, which adds
only bench results).

Result: vscode's heap allocations 41.3M -> 22.3M (-46%), single-threaded instructions -2.0% to -4.0% on the five
bench projects measured, output byte-identical everywhere it was compared. Eight commits, each a site or a family of
sites where the port allocated and Go does not (or allocates once).

## Where the allocations were

mimalloc's own count (`MIMALLOC_SHOW_STATS=1` on the release binary; arena allocations are not in it), vscode:

| size class | before: count | peak live | allocated in total | after: count | allocated in total |
| --- | ---: | ---: | ---: | ---: | ---: |
| 8 B | 19.5M | 3.4 MiB | 149.3 MiB | 8.7M | 66.7 MiB |
| 16 B | 9.3M | 4.6 MiB | 142.2 MiB | 4.6M | 70.8 MiB |
| 32 B | 4.1M | 10.6 MiB | 125.4 MiB | 2.3M | 71.6 MiB |
| 48 B | 0.74M | 1.5 MiB | 33.9 MiB | 0.60M | 27.5 MiB |
| 64-160 B | 5.8M | 64.7 MiB | 635.4 MiB | 4.7M | 481.1 MiB |
| all | 41.3M | 427.8 MiB | 2.4 GiB | 22.3M | 2.0 GiB |

The small classes have tiny live peaks against a large total: temporaries (a `String` copy of a name, a `Vec` copy of
a list) freed right after use. The live heap and the arenas do not change (`TSRS_MEM_SPLIT=1`: 420.9 MiB live heap,
1071.7 MiB arena at check end, before and after).

## Method

- Count profile: the alloc-profile build of docs/DEBUGGING.md with `TSRS_HEAP_PROFILE=count` (every 1024th
  allocation's stack) and `TSRS_HEAP_PROFILE_TSV`, demangled with `rustfilt`, grouped by the first frame that is not
  the allocator or the standard library.
- **The profile as built names the wrong frame.** `heap_sample::on_alloc` drops the first three frames of
  `backtrace()` on the assumption that they are its own, but with inlining they are fewer, so the function that
  called the allocator is dropped too and every allocation is charged to its caller's call line (for example 1.9M
  "in `get_type_at_flow_node`", at flow.rs:555, the call of `is_matching_reference`, which allocated them: 5.1M over
  all its callers). Its library filter also tests the names as prefixes, which v0-mangled names (macOS `atos` output)
  never match. For this round a local, uncommitted patch dropped only the first frame, resolved each address with
  `atos -i` (the inline chain with file:line) and filtered library frames on the demangled name. Every site below
  comes from that profile; the patch is not part of this change.
- Instructions: `/usr/bin/time -l` ("instructions retired"), `-p <project> --noEmit --incremental false
  --extendedDiagnostics --pretty false --singleThreaded`, `RAYON_NUM_THREADS=1`, base and new interleaved, 3 runs
  each, medians. On this Mac the count is not exact run to run (up to 1% on vscode, 0.01-0.7% elsewhere); every
  difference below is at least twice the spread.
- Exactness: `--pretty false` output and exit status compared byte for byte (below).

## The sites and what changed

Allocations removed on vscode, from the count profile (sampled, so to about 0.05M); the commits are numbered in branch
order (`perf/heap-churn`):

| # | site (Go function) | removed | what the port did -> now |
| --- | --- | ---: | --- |
| 1 | `getAccessedPropertyName`, called twice per pair by `isMatchingReference` | 5.1M | copied the name node's text into a `String` -> returns `Cow<'static, str>`, owned only for computed names |
| 2 | `tspath.GetDeclarationFileExtension` (from `isOnlyImportableAsDefault`), `GetBaseFileName`, `GetDirectoryPath`, `CombinePaths`, `ContainsPath` (from `Program::common_source_directory`, once per root file), `simpleNormalizePath` | 3.0M | normalized copy + base-name copy + extension copy per call; a `String` per path component; `replace("/./")` copying paths with only `..` -> returns `&str` (new `base_file_name`), borrows when there is no backslash, one reservation per combined path, component slices, `replace` only when there is a `/./` |
| 3 | `getEffectiveCallArguments`, `resolveCall`'s `typeArguments`, `getContextualTypeForArgument` | 2.0M | copied `node.Arguments()` / `node.TypeArguments()` per call resolution and per contextually typed argument, cloned again in `chooseOverload` -> `Cow` of the stored list and the stored slice |
| 4 | `isNumericLiteralName`, `getContextualTypeForElementExpression` | 0.9M | `Number::string()` (a `"NaN"` `String` for every non-numeric name) and `index.to_string()` -> `Number::string_equals`, `stringutil::format_int` on the stack |
| 5 | `getEffectivePropertyNameForPropertyNameNode` (every property of every object literal in `checkGrammarObjectLiteralExpression`) | 0.8M | copied the stored name -> `Cow` |
| 6 | `Symbol::append_declarations` (the binder's `append(symbol.Declarations, node)`) | 2.3M | built the concatenation in a heap `Vec`, then moved it into the arena -> `alloc_slice_concat` copies both parts into one arena slice (same arena bytes) |
| 7 | `getTypeWithThisArgument` 1.2M, `reorderCandidates` 1.1M, `excludeProperties` 0.56M, `findApplicableIndexInfo` 0.6M, `getContextualCallSignature` 0.35M | 3.8M | fresh `Vec`s -> a buffer from `free_type_lists`, a new `free_signature_lists` pool, the input slice when nothing is excluded, `SmallVec<[_; 4]>` for the two lists that are almost always zero or one long |
| 8 | `fillMissingTypeArguments` (from `getSignatureInstantiation`) | 0.4M | copied the arguments when none was missing -> `Cow` |

`smallvec` is new as a dependency of `tsrs_checker` (docs/RUST.md "Techniques" moves it from "Not tried" to "In
place"). Only the two lists above use it: for every other short list in the profile, returning the stored data or a
pooled buffer removed the allocation without it.

## Before and after

Single-threaded, medians of 3 interleaved runs:

| project | instructions before | after | change | heap allocations before | after | change | max RSS change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 98.22G | 96.10G | -2.15% | 41.3M | 22.3M | -46% | +0.17% |
| webpack | 13.37G | 12.99G | -2.84% | 5.1M | 3.6M | -29% | +0.14% |
| xstate-main | 7.32G | 7.02G | -4.04% | 3.8M | 3.1M | -18% | +0.25% |
| cal-diy | 39.59G | 38.32G | -3.23% | 16.2M | 12.9M | -20% | +0.04% |
| t3code-server | 46.87G | 45.91G | -2.06% | 21.9M | 16.2M | -26% | +0.06% |
| formbricks-web | | | | 19.1M | 14.3M | -25% | |
| supabase-studio | | | | 26.1M | 21.6M | -17% | |

vscode in the default mode (18 cores): instructions 109.4G -> 106.4G (-2.8%, medians of 3), max RSS within the
run-to-run noise (1981-1998 MiB before, 1965-1989 after). The +0.04-0.25% single-threaded max RSS is not in the live
data: live heap, heap pages and arena are equal or lower at check end with `TSRS_MEM_SPLIT=1`.

Wall time was not measured on purpose: other agents were building on the machine.

## Exactness

- `--pretty false` output and exit status byte-identical, single-threaded, on vscode, webpack, xstate-main, cal-diy,
  t3code-server, formbricks-web and supabase-studio; vscode also in the default multi-checker mode.
- `tools/regressions.sh`: 28/28.
- Conformance, base and new built from the same tree state: `tsrs-test run --suite all --baselines types,symbols`
  (13,458 / 12,779 / 12,779, the gate's minimums) and the default mode
  (`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`, `--baselines types,symbols,js,jsmap,sourcemap`):
  every pass list identical (`comm` prints nothing either way).
- `tools/lint/ratchet.py` (none new), `tools/lint/source.py`, `tools/gen-check.sh`, `cargo check -p tsrs_wasm --target
  wasm32-wasip1`, `cargo test -p tsrs_core -p tsrs_scanner -p tsrs_module -p tsrs_ast -p tsrs_tsoptions` (a new
  `test_base_file_name` covers the offsets `base_file_name` slices at, with backslash and trailing-separator paths).
  `cargo test -p tsrs_cli` does not link on macOS on main either (`api.rs` calls glibc's `malloc_trim`).

## What is left, and why this stopped

After the change the count profile is diffuse: apart from symbol-table growth no site above 1.2M of the 22.3M, and
each of the larger ones is either stored data, allocated by Go too, or needs a wider change than one allocation:

- `SymbolMap`'s `EntryVec` growth, 3.7M (alloc and realloc): symbol tables, kept for the whole run; the 1, 2, 4, then
  +50% growth policy is tuned for memory (symbol.rs). Not churn.
- `SymbolTable::entries` / `values` snapshots, 1.35M (`getNamedMembers`, `everyLazyProperty`, lazy member tables,
  `checkUnusedLocalsAndParameters`): the snapshot is what lets the callee mutate the table during the walk; dropping it
  needs a proof that none of those callees reaches the same table.
- Results that Go allocates too: `getNamedMembers`' list (0.6M), `normalize_path`'s and `get_directory_path`'s result
  strings (0.5M, 0.35M), `to_path` (a `String`, then the `Arc<str>` of the `Path`, 0.55M), union and inference lists
  (`addTypesToUnion`, `newInferenceContext`, `getConditionalTypeInstantiation`: 0.3-0.4M each), number-literal property
  names (`Number::string`, 0.29M).
- `scanner::get_text_of_node` copies the node's text (0.28M; Go returns a substring), but it has 58 callers, several
  of which keep a `String`.
- Module resolution: the cache key's two `String`s per lookup (0.23M, also on hits) would need a borrowed-key lookup,
  `try_extension`'s `format!` (0.12M).

Hash table growth, by count: 1.2M in total, the largest the per-call maps of `checkGrammarObjectLiteralExpression`
(0.34M), `everyLazyProperty` (0.23M) and `createUnionOrIntersectionProperty` (0.18M). The long-lived tables a byte
profile shows growing (`Relation::set`, the keyed link stores, the resolver's `ModeAwareCacheKey` map) grow a few
times each over the run: many megabytes, few allocations, so they are not in this count and pre-sizing them was not
tried. `is_reachable_flow_node_worker` has no per-call map at this commit; its `flow_node_reachable` is one map per
checker for the whole run.

Arena-backed temporaries (docs/RUST.md "Not tried") were not needed for any of these sites.
