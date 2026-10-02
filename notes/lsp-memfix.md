# lsp-memfix: auto-import registry ownership, census hits with completions (phase 4)

Wave agent `memfix`, branch `lsp-memfix` (from `lsp` 3be0092). Leftovers of notes/lsp-mem.md ("Needs from others",
"Doubts").

## 1. Auto-import registry ownership

| piece | where |
| --- | --- |
| scratch region per `Registry::clone_registry` (entered for the whole update, freed before it returns): resolvers, alias resolvers, extraction checkers, resolution caches, discovered packages' package.json entries | `tsrs_ls/src/autoimport/registry.rs` (`clone_registry` -> `clone_registry_in_scratch`) |
| package.json region per update for the entries `directories` keeps (`get_directory_package_json`, created on first use); `Registry::regions` = the distinct regions containing the directories' entries (`regions_of_directories`, `Region::containing_all`), so a region lives while any registry version refers into it | same |
| `Box::leak` of module resolver, alias resolver (x2 sites each) and `get_module_resolver`'s resolution host -> `tsrs_core::alloc` in the scratch region (dropped with it) | `registry.rs`, `util.rs` |
| `pin_registry_file` removed (files the update acquired are released by `dispose` as in Go and freed when nothing else holds them); snapshot clone no longer enters the thread arena around the update | `tsrs_project/src/autoimport.rs`, `snapshot.rs` |

The finished registry holds no arena pointer except the directories' package.json entries (buckets, indexes,
`Export`, entrypoints and specifier caches are heap values). Buckets and results are unchanged (same code path; the
only reordering: `update_directory` reads the package.json before `change_if` with the same condition, so the entry is
allocated in the package.json region).

Found on the way (both made the private monorepo crash or leak once the update's memory was freed):

- **Program package.json cache filled from a freed region** (segfault at the second warm-up on the private monorepo:
  `Program::collect_package_names` -> `get_package_json_info` stored a scratch-region `InfoCacheEntry` in the
  program's resolution data cache). Also latent before this wave: `ResolutionData::clone_data` copies entry pointers
  into the next program version, so an entry added during a clone's construction (its version region) or from a
  checker region dangled after that version was freed. Fix: `InfoCache` remembers the cache it was cloned from
  (`origin`, `owner_addr`), and new entries go through `arena::enter_table_owner(owner_addr)`: the current target if
  the original cache lives in it (a full build; an update's own resolvers), else the thread arena. It never waits for a
  region lock: parse workers fill the cache while the building thread holds the full build's region (`enter_owner`
  would deadlock there).
- **`sort_symbols` panicked** in the extraction checker (pre-existing, base too): the alias resolver's
  `source_files` are only the root files, so `compareNodes` maps other files to index 0 (as Go's map lookup does) and
  `compareSymbols` is not a total order; Rust's sort panics ("does not correctly implement a total order"), Go sorts.
  On the private monorepo every completion request errored (132/200 in base and new), the registry was never prepared
  and the panicking update never disposed its host (acquired files leaked: base 55 MiB/edit with completions). Fix:
  `checker::Program::source_files_complete()` (default true, alias resolver false); then `sort_symbols` uses
  `goslices::sort_func` (Go's pdqsort). Programs keep Rust's sort: using Go's sort everywhere cost +1.4% instructions on
  webpack (16.56 vs 16.33 G).

## 2. Census hits with completions

Layouts from `-Zprint-type-sizes` (nightly, release).

| hit | referrer layout | verdict and fix |
| --- | --- | --- |
| keyword completion cache: heap block (`get_typescript_keyword_completions`) +472 -> freed `Node` | `CompletionItem` is 744 bytes; +472 is inside `data: Option<CompletionItemData>` (+312..+504, 192 bytes), which is `None` for keyword items (niche: `file_name`'s capacity at +312); neighbouring words were a return address and an arena pointer | stale payload bytes of a `None`. `tsrs_core::census_scrub_none` (census builds: zero a `None`'s bytes, then write `None` back) on all 18 option fields of every cached item (both caches and `all_keyword_completions`); the old `census_scrub_stack` there did not reach the clone path and is removed |
| registry entrypoints: `ArcInner<ResolvedEntrypoint>` +96 -> freed scratch `str` (4 hits) | struct at +16: 3 strings (0..72), `include_conditions` (72..104): hashbrown `RawTableInner` has `ctrl` first; +88 (= ctrl) was 0, i.e. `None`; +96/+104 are its payload | stale payload. Scrubbed where the registry keeps them (`extract_package`), and the bucket's option fields in `replace_bucket` |
| root `__DATA` +0x1f00 -> freed `InfoCacheEntry` (with and without completions) | `EMPTY_COMPILER_OPTIONS` (`LazyLock<CompilerOptions>`, 848 bytes at `__DATA`+0x1ca0): +0x260 = `paths: None` (+560..+616), payload word | stale: the static was first initialized during a registry update, copying its stack. Census builds initialize it before the server starts (`census::prepare_lsp`) |
| heap block from `get_conditional_type_instantiation_ex` +128, word `0xabf92f8032f45ea6` (plain session, once) | a hash in a checker cache; decoded as a `SymbolMapEntry` (odd bit set, len 36) its low 45 bits x 8 hit a freed `Symbol` start | chance match of the x8 symbol-entry rule (~1e-7 per random word). The rule now requires the referrer to be an entry buffer: its first word must also decode to a `Symbol` (an `EntryVec` buffer starts with entry 0). A debug run counted 0 rejected words; strong-mark coverage varies 1.14-1.45M blocks run to run either way |
| root stack +0x2a8 -> freed `TypeMapper` (one completion session) | inside the census's own frames on the main thread | stale stack word; `census_scrub_stack` before the LSP census |

The "module-resolver package.json block" doubt of notes/lsp-mem.md is the cache bug above (real, fixed), as far as
can be told: no such hit remains.

## RSS (release, `tools/lsp-mem/lsp_mem.py`, 200 edits, MiB at edit 0 / 20 / ... / 200)

| session | build | 0 | 20 | 40 | 60 | 80 | 100 | 120 | 140 | 160 | 180 | 200 | MiB/edit, edits 100-200 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| xstate | base (`lsp-mem` regions) | 196 | 314 | 354 | 398 | 449 | 479 | 512 | 566 | 605 | 645 | 682 | 2.0 |
| xstate | memfix | 199 | 296 | 322 | 339 | 346 | 349 | 349 | 352 | 352 | 354 | 355 | 0.06 |
| xstate + completions | base | 198 | 458 | 466 | 466 | 466 | 466 | 466 | 467 | 471 | 471 | 471 | 0.05 |
| xstate + completions | memfix | 199 | 423 | 427 | 432 | 432 | 433 | 434 | 434 | 434 | 434 | 435 | 0.02 |
| private monorepo | base | 2856 | 2883 | 2898 | 2912 | 2939 | 2959 | 2974 | 2999 | 3040 | 3059 | 3080 | 1.2 |
| private monorepo | memfix | 2856 | 2874 | 2880 | 2887 | 2889 | 2889 | 2889 | 2890 | 2890 | 2890 | 2890 | 0.01 |
| private monorepo + completions | base (132 errored requests) | 2854 | | | | | | | | | | 13871 | 55 |
| private monorepo + completions | memfix (0 errors) | 2870 | 4442 | 4474 | 4475 | 4476 | 4476 | 4476 | 4476 | 4476 | 4476 | 4476 | 0.0 |

The +1.6 GB step at the first completion on the private monorepo is the first full registry build (node_modules and
project buckets); the curve is flat afterwards. The private monorepo was read only: `git status --short` of its root
was empty before and after every run.

## Gates

- `cargo check --workspace --tests`: 0 warnings. `cargo test -p tsrs_ls -p tsrs_project -p tsrs_core -p tsrs_lsp
  -p tsrs_module -p tsrs_compiler`: all pass.
- Conformance: 13,458 pass / 2 codes / 2 fail, pass list identical; `TSRS_LAZY_MEMBERS=0 --baselines types,symbols`
  12,779 / 12,779, lists identical.
- Fourslash: 4,066 / 4,546, pass list identical to `lsp-mem`.
- Census (xstate, 200 edits, `TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1`): diagnostics + hover session 65,739 regions freed
  (45.2M blocks, 2,108 MB would-free), strong mark 0 violations, precise walk 0; with completion requests 4,864
  regions (22.8M blocks, 1,143 MB), 0 violations, precise walk 0. Four runs of each after the fixes: all 0 except one
  completion run with the census-frame stack word (fixed, then 0 in the two runs after).
- Poison mode (`TSRS_ARENA_POISON=1`), LSP oracle base vs memfix, 12 xstate files with edit rounds, diagnostic /
  hover / definition / typeDefinition / references / completion / documentSymbol / signatureHelp: 15,487 responses,
  all equal (2,532 completions).
- CLI (`tsrs -p webpack --noEmit --incremental false`, interleaved): instructions 16.32-16.38 G vs base 16.33-16.52
  G, output identical, peak RSS equal.

## Shared-file edits

- `tsrs_core`: `arena.rs` (`enter_table_owner`), `ptr.rs` + `lib.rs` (`census_scrub_none`),
  `alloc_profile/census.rs` (x8 symbol entries only from entry buffers).
- `tsrs_module`: `packagejson/cache.rs` (`InfoCache.origin`, `owner_addr`), `resolver.rs` (`get_package_json_info`
  allocates entries through `enter_table_owner`).
- `tsrs_checker`: `program.rs` (`Program::source_files_complete`, default true), `utilities.rs` (`sort_symbols`).
- `tsrs_ls`: `autoimport/{registry,util,aliasresolver}.rs`, `completions_3.rs` (census scrubs of cached items).
- `tsrs_project`: `autoimport.rs`, `snapshot.rs`.
- `tsrs_cli`: `lsp.rs`, `census.rs` (`prepare_lsp`, stack scrub before the LSP census).
- `docs/LSP.md`: memory plan table and mechanism, known gaps.

## Deviations

- Registry scratch / package.json regions and `Registry::regions` (not in Go; the GC collects an update's garbage).
- `InfoCache.origin` and `enter_table_owner`: an entry added to a program's package.json cache from outside the full
  build's region (a clone's construction, a checker, a parse worker) stays in the thread arena for the session (Go
  collects it with the last program version that copied it). Bounded by the distinct package.json paths looked up.
- `checker::Program::source_files_complete` (not in Go): selects Go's pdqsort for `sort_symbols` where the comparator
  is not total.

## Needs from others

- None for this work. The completion path on the private monorepo was not exercised before this wave (every request
  panicked); now it is, with Go's order for the extraction checker's symbol sort but no oracle comparison against
  `tsgo-ref` there yet.

## Doubts

- `census_scrub_none` relies on `p.write(None)` storing only the niche (the census shows it does); it is census-only.
- Strong-mark coverage varies between runs of the same session (timing: idle checkers, what is alive at exit).
- The private monorepo + completions peak (4.48 GiB) has no `tsgo-ref` comparison.
