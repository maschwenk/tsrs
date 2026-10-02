# lsp-mem: memory of the long-lived server (phase 4, docs/LSP.md "Memory plan for a long-lived server")

Wave agent `mem`, branch `lsp-mem` (merged `lsp` ad2064e: actions + robust waves).

## Step 1: measurements (before any change)

Driver: `tools/lsp-mem/lsp_mem.py` (opens one file, waits for its diagnostics, then N incremental edits inside a
function body: typing `let zz = 1; ` before a `return` character by character and deleting it again; each edit is
followed by `textDocument/diagnostic` and a hover, `--completion` adds a completion request at the edit point; RSS
of the server process sampled with `ps -o rss`; the server is ended by closing its input, `--exit-timeout` waits for
the census).

Base `lsp` 0c4b45a (before the actions/robust merge), release build, 200 edits, RSS MiB at edit 0 / 20 / ... / 200:

| project (file) | server | 0 | 20 | 40 | 60 | 80 | 100 | 120 | 140 | 160 | 180 | 200 | MiB/edit |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| xstate (`packages/core/src/createActor.ts`) | tsrs | 187 | 354 | 521 | 689 | 858 | 1027 | 1196 | 1365 | 1534 | 1704 | 1874 | 8.44 |
| xstate | tsgo-ref | 313 | 691 | 698 | 727 | 737 | 738 | 752 | 753 | 754 | 965 | 965 | (GC heap target, plateaus) |
| private monorepo (a 408-line service file) | tsrs | 2750 | 2953 | 3324 | 3707 | 4088 | 4473 | 4858 | 5242 | 5627 | 6012 | 6397 | 18.2 |
| private monorepo | tsgo-ref | 4920 | 5282 | 5725 | 6282 | 6702 | 7123 | 7445 | 7445 | 7446 | 7447 | 7449 | (plateau from edit 120) |

After merging `lsp` ad2064e (auto-import warming re-extracts packages on every edit) the base grows faster: xstate
191 -> 3059 MiB over 200 edits (14.3 MiB/edit). The private monorepo was checked with `git status --short` before
and after every run: unchanged (0 entries).

## Step 2: regions (what was built)

Design and the two corrections to the original plan: docs/LSP.md "Memory plan for a long-lived server".

| piece | where |
| --- | --- |
| allocation target (thread-local `CURRENT`, scope stack), `Region` (own `Arena`: chunks, free lists, checkpoints, drop list), owner routing (`enter_owner`, registry of chunk ranges + adopted heap owners), `enter_thread_arena`, `on_free` hooks, upward bump + per-thread slabs + `trim` for regions, census hooks | `crates/tsrs_core/src/arena.rs`, `ptr.rs` |
| file regions (parse + bind in a fresh region, trimmed; owned by the cache entry via `on_evict`/`on_revive` and by program owners) | `tsrs_project/src/parsecache.rs`, `refcountcache.rs` |
| checker regions (create + hold; parked on dispose; `free_checkers`; the pool keeps the program owner alive while a checker is held and refuses use after the program was freed) | `tsrs_project/src/checkerpool.rs` (hooks only: `new_pooled_checker`, `enter_checker_region`, `dispose_checker`, `free_checkers`, `set_owner`) |
| program owner (version region, shared base region, file regions; frees checkers, `Program`, regions) | `tsrs_project/src/memregions.rs`, `project.rs` (`create_program`), `projectcollectionbuilder.rs` (one line) |
| `free_program` (drops the boxed `Program` and its per-version resolution host, which kept the compiler host and its snapshot file system alive) | `tsrs_compiler/src/program.rs` |

Startup footprint: one region per parsed file (40k on the private monorepo) first cost +540 MiB RSS (each separately
allocated chunk had a partly used page at each end: mimalloc writes the first word of the next block). Regions now
bump upwards, carve their chunks from per-thread 1 MiB slabs and a file region is trimmed after binding: +110 MiB
(2.86 vs 2.75 GiB at edit 0).

## Step 3: census gate

`tsrs --lsp` in the alloc-profile build runs the census at exit (`TSRS_CENSUS=1`, roots: server, session, current
snapshot, main stack, data segments; `TSRS_CENSUS_VERIFY=1` also walks the current snapshot's programs precisely).
Region frees are recorded as would-free ranges (memory kept, drops run), checked with the free-time filter.
Getting the strong mark to 0 needed, besides real fixes (below): gaps between slab carves and at region chunk starts
in profile builds (one-past-the-end pointers in arenas and the registry equal the next region's first block),
registry keys stored as address + 1 (vacated B-tree slots), stack scrubs before building a `Program` pool slot and a
`Checker` (uninitialized `OnceLock`/`Option` payloads), the census's own thread data untracked, MappedType's 4-byte
field at 104 as padding, and `Option<Vec/String>` None-niche payload words not counted as references.

Real bugs the census, poison mode and fourslash found on the way (all fixed): the source text table kept pointers to
freed texts (`unregister_source_text` on region free); config output-name maps
(`ParsedCommandLine.parse_input_output_names`) were filled inside a program or checker region but belong to the
command line (routed to its owner); auto-import registry data referred to files it had released (registry updates
stay in the thread arena, its files are pinned); pooled-checker tests and fourslash state baselines kept a
`&'static Program` beyond the owning project (Go's GC kept it alive).

## Results

All on the merged branch (release build, same driver; MiB at edit 0 / 20 / ... / 200):

| session | server | 0 | 20 | 40 | 60 | 80 | 100 | 120 | 140 | 160 | 180 | 200 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| xstate | tsrs `lsp` ad2064e | 191 | | 932 (50) | | | 1646 | | | 2345 (150) | | 3059 |
| xstate | tsrs regions | 199 | 320 | 370 | 421 | 461 | 488 | 522 | 564 | 600 | 634 | 676 |
| xstate + completion requests | tsrs regions | 200 | 459 | 469 | 469 | 469 | 502 | 502 | 502 | 502 | 502 | 508 |
| xstate | tsgo-ref | 313 | 691 | 698 | 727 | 737 | 738 | 752 | 753 | 754 | 965 | 965 |
| private monorepo | tsrs `lsp` ad2064e | 2760 | 3001 | 3399 | 3808 | 4217 | 4629 | 5040 | 5451 | 5860 | 6273 | 6684 |
| private monorepo | tsrs regions | 2852 | 2874 | 2891 | 2911 | 2927 | 2953 | 2974 | 3014 | 3037 | 3068 | 3085 |
| private monorepo | tsgo-ref | 4920 | 5282 | 5725 | 6282 | 6702 | 7123 | 7445 | 7445 | 7446 | 7447 | 7449 |

Program versions, their checkers and the old file versions are freed (`TSRS_REGION_LOG=1`: one "freed" line per
edit). The remaining slope (xstate 2.4, private monorepo 1.2 MiB/edit) is the auto-import warm-up the actions wave
runs after every edit when no completion has prepared the registry: its scratch stays in the thread arena (see Needs
from others); with a prepared registry (completion requests) the curve is flat. Before the merge (no warm-up) the
regions build was flat on both projects: xstate 192 -> 213 MiB, private monorepo 2860 -> 2867 MiB over 40 edits
(0.17 MiB/edit).

Gates:

- `cargo check --workspace --tests`: 0 warnings. `cargo test -p tsrs_core -p tsrs_project -p tsrs_lsp`: all pass
  (tsrs_core +3 region tests: scopes/owner routing/drops, region free lists and rewinds, slabs and trim).
- Conformance: 13,458 pass / 2 codes / 2 fail; `TSRS_LAZY_MEMBERS=0 --baselines types,symbols` 12,779 / 12,779.
- Fourslash: 4,066 / 4,546, pass list identical to `lsp` ad2064e.
- Census (xstate, 200-edit diagnostic + hover session): 2,585 regions freed (21.2M blocks, 1,067 MB would-free),
  strong mark 0 violations, precise walk 0 (2.71M program references). With completion requests added: 1 hit, the
  keyword completion cache (see Doubts); precise walk 0.
- Poison mode (`TSRS_ARENA_POISON=1`, freed regions filled with 0xA5 and kept mapped): LSP oracle, `lsp` vs regions,
  12 xstate files with edit rounds, diagnostic / hover / definition / typeDefinition / references / completion /
  documentSymbol / signatureHelp: 23,604 responses, all equal (3,878 completions included).
- CLI (`tsrs -p <dir> --noEmit --incremental false`, 3 interleaved runs, instructions retired): webpack 16.48-16.49 G
  vs 16.49-16.86 G (base), mui-docs 83.81-83.85 G vs 83.64-84.69 G (+0.2% against the best base run; about half of
  it is the region branch in the bump allocator), wall and peak RSS within noise, output byte-identical.


## Shared-file edits

- `tsrs_core`: `arena.rs`, `ptr.rs` (regions; `free!`/`free_slice!` pass `needs_drop`; `census_scrub_stack`),
  `lib.rs` (export), `alloc_profile.rs` + `alloc_profile/census.rs` (region would-free ranges, reporting by referrer,
  chain offsets, raw frames, padding/niche rules, thread data untracked).
- `tsrs_ast/src/ast.rs`: `SourceFile` lazy fills enter the file's region (`owner_region`, taken before the cache lock);
  `identifier.rs`: `unregister_source_text`.
- `tsrs_compiler/src/program.rs`: `free_program`; `get_symlink_cache` routes to the owner of `processed`; census scrub
  before `init_checker_pool`. `lib.rs`: export.
- `tsrs_checker/src/checker.rs` (`primitive_type_alias_suggestions`), `tsrs_pseudochecker/src/type_.rs` (static
  pseudo types), `tsrs_modulespecifiers/src/util.rs` (regex cache): process-wide statics allocate in the thread arena.
- `tsrs_tsoptions/src/parsedcommandline.rs`: `parse_input_output_names` routes to the command line's owner.
- `tsrs_ls/src/completions_3.rs`: census scrub before the keyword completion cache is filled.
- `tsrs_project`: `snapshot.rs` (registry update in the thread arena), `autoimport.rs` (`pin_registry_file`),
  `compilerhost.rs` (resolved project references in the thread arena), `checkerpool_test.rs` (setup keeps the project).
- `tsrs_fourslash/src/statebaseline.rs`: keeps the serialized programs' project values alive.
- `tsrs_cli`: `lsp.rs` + `census.rs` (census at server exit), `Cargo.toml` (depends on `tsrs_project`).

## Deviations

- Programs are freed when the last `Project` value referring to them drops (Rust ownership standing in for the GC),
  not at Go's refcount event; file regions are owned by program owners too. Checker regions die with the pool, not
  at idle/cancel disposal (disposed checkers are parked). Both explained in docs/LSP.md.
- A program shares its full build's `processed` data and project reference file mapper with its clones; those stay
  leaked (`Box::leak`, one per full build), as before.
- Files acquired by the auto-import registry keep one parse-cache reference for the session (Go would re-parse and
  collect them): the registry keeps arena data (its checkers' symbols, alias resolver caches) that refers to them,
  with no owner object. Registry updates allocate in the thread arena (never freed).
- `exit` alone does not end `tsrs --lsp` (pre-existing, also on the base): the driver closes the server's input.

## Needs from others

- `tsrs_ls` auto-import registry (actions): an owner for the arena data an update allocates (a region per registry
  version, shared with the versions that keep its buckets); then `pin_registry_file` can go. Today the warm-up
  re-extracts on every edit and its scratch stays in the thread arena (xstate: about 2 MiB per edit without
  completion requests; flat once a completion prepared the registry).
- `tsrs_ls` registry: `Box::leak` of an alias resolver and module resolver per extraction (heap, never freed).

## Doubts

- Census with completion requests: a few remaining strong-mark hits, from a `Vec<lsproto::CompletionItem>` in the
  process-wide keyword cache (lsproto values hold no arena pointers by type, so the word is uninitialized payload)
  and from a heap block reached through a module-resolver package.json block (an interior pointer into a freed node
  list; not traced to a field). The precise walk is 0 and poison-mode sessions are output-identical.
- Idle-disposed checkers (30 s) keep their regions until the next program version: a long idle session without edits
  accumulates one region per idle cycle.
- SOURCE_TEXTS slots (2^20) are not reused; after a million parses compact identifiers fall back to stored text.
- The census cannot see a freed heap `Program` referenced from live data (heap frees are not would-free); the pool
  guard and the fourslash/oracle runs are the evidence there.
