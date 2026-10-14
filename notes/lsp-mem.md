# lsp-mem: memory of the long-lived server (phase 4, docs/LSP.md "Memory plan for a long-lived server")

Wave `mem` of the language-server port: regions for file versions, checkers and programs. The design and its two
corrections to the original plan are in docs/LSP.md; the auto-import registry's ownership and the remaining census
hits were fixed afterwards (notes/lsp-memfix.md). This note keeps the measurements and the census method.

## Driver

Driver: `tools/lsp-mem/lsp_mem.py` (opens one file, waits for its diagnostics, then N incremental edits inside a
function body: typing `let zz = 1; ` before a `return` character by character and deleting it again; each edit is
followed by `textDocument/diagnostic` and a hover, `--completion` adds a completion request at the edit point; RSS
of the server process sampled with `ps -o rss`; the server is ended by closing its input, `--exit-timeout` waits for
the census).

## Before and after (release build, 200 edits, RSS MiB at edit 0 / 20 / ... / 200)

Before, on base `lsp` 0c4b45a: tsrs grew 8.44 MiB/edit on xstate (`packages/core/src/createActor.ts`, 187 -> 1874)
and 18.2 MiB/edit on the private monorepo (a 408-line service file, 2750 -> 6397), while tsgo-ref plateaus (its GC
heap target). After merging `lsp` ad2064e (auto-import warming re-extracts packages on every edit) the base grew
faster: xstate 191 -> 3059 MiB (14.3 MiB/edit).

| session | server | 0 | 20 | 40 | 60 | 80 | 100 | 120 | 140 | 160 | 180 | 200 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| xstate | tsrs `lsp` ad2064e | 191 | | 932 (50) | | | 1646 | | | 2345 (150) | | 3059 |
| xstate | tsrs regions | 199 | 320 | 370 | 421 | 461 | 488 | 522 | 564 | 600 | 634 | 676 |
| xstate + completion requests | tsrs regions | 200 | 459 | 469 | 469 | 469 | 502 | 502 | 502 | 502 | 502 | 508 |
| xstate | tsgo-ref | 313 | 691 | 698 | 727 | 737 | 738 | 752 | 753 | 754 | 965 | 965 |
| private monorepo | tsrs `lsp` ad2064e | 2760 | 3001 | 3399 | 3808 | 4217 | 4629 | 5040 | 5451 | 5860 | 6273 | 6684 |
| private monorepo | tsrs regions | 2852 | 2874 | 2891 | 2911 | 2927 | 2953 | 2974 | 3014 | 3037 | 3068 | 3085 |
| private monorepo | tsgo-ref | 4920 | 5282 | 5725 | 6282 | 6702 | 7123 | 7445 | 7445 | 7446 | 7447 | 7449 |

The remaining slope of the regions build (xstate 2.4, private monorepo 1.2 MiB/edit) was the auto-import warm-up's
scratch in the thread arena; with a prepared registry (completion requests) the curve was flat, and notes/lsp-memfix.md
gave the registry an owner. Before the warm-up was merged the regions build was flat on both projects: xstate 192 ->
213 MiB, private monorepo 2860 -> 2867 MiB over 40 edits (0.17 MiB/edit). `TSRS_REGION_LOG=1` prints one stderr line per
program created / freed.

Startup footprint: one region per parsed file (40k on the private monorepo) first cost +540 MiB RSS (each separately
allocated chunk had a partly used page at each end: mimalloc writes the first word of the next block). Regions bump
upwards, carve their chunks from per-thread 1 MiB slabs and a file region is trimmed after binding: +110 MiB (2.86 vs
2.75 GiB at edit 0).

CLI cost of the region support (`tsrs -p <dir> --noEmit --incremental false`, 3 interleaved runs, instructions
retired): webpack 16.48-16.49 G vs 16.49-16.86 G (base), mui-docs 83.81-83.85 G vs 83.64-84.69 G (+0.2% against the
best base run; about half of it is the region branch in the bump allocator), wall and peak RSS within noise, output
byte-identical.

## Census gate

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
command line (routed to its owner); auto-import registry data referred to files it had released (fixed then by keeping
registry updates in the thread arena and pinning its files; replaced by a scratch region per update in
notes/lsp-memfix.md); pooled-checker tests and fourslash state baselines kept a
`&'static Program` beyond the owning project (Go's GC kept it alive).

Results on xstate (200-edit diagnostic + hover session): 2,585 regions freed (21.2M blocks, 1,067 MB would-free),
strong mark 0 violations, precise walk 0 (2.71M program references). Poison mode (`TSRS_ARENA_POISON=1`, freed regions
filled with 0xA5 and kept mapped): LSP oracle, `lsp` vs regions, 12 xstate files with edit rounds, diagnostic / hover /
definition / typeDefinition / references / completion / documentSymbol / signatureHelp: 23,604 responses, all equal
(3,878 completions included).

## Limitations noted at the time

- Idle-disposed checkers (30 s) kept their regions until the next program version: a long idle session without edits
  accumulated one region per idle cycle, and every canceled request one more. Fixed later: see "Disposed checkers"
  below.
- SOURCE_TEXTS slots (2^20) are not reused; after a million parses compact identifiers fall back to stored text.
- The census cannot see a freed heap `Program` referenced from live data (heap frees are not would-free); the pool
  guard and the fourslash/oracle runs are the evidence there.
- `exit` alone does not end `tsrs --lsp` when the client does not answer the server's `client/registerCapability`
  request sent while handling `initialized`. Not a divergence: Go blocks its dispatch loop on it the same way. With a
  client that answers, both exit with status 1 and "context canceled" (`tools/oracle/lsp/exit_check.py` compares the
  four combinations).

## Disposed checkers (later fix)

Before: a checker disposed by cancellation or the 30 s idle timeout was parked with its region until the program was
freed, so an unchanged program grew without bound (xstate, 100 canceled `textDocument/diagnostic` pulls on
`packages/core/test/types.test.ts`: 154 -> 871 MiB; a heavy generated file: +51 MB per idle cycle). Go drops the
checker and its GC frees it.

Freeing at disposal is not enough: Go's language service uses a checker's results after releasing it
(`GetSemanticDiagnostics` returns the diagnostics, the ls converts them afterwards), and another request can take the
idle checker, be canceled and dispose it meanwhile. So (checkerpool.rs):

- **Leases.** The dispatch loop puts a `checkerDataLease` into each message's context and makes it the thread's lease
  while the handler runs (call hierarchy and file rename take checkers with `context.Background()`, as in Go). Each
  acquisition registers the lease with the checker; the lease ends when the last context clone and scope are gone,
  after the handler returned. A disposed checker is freed when its last lease ends (at disposal if none is left).
- **Pinned.** A checker held without a lease (callers outside the dispatch loop, the API checker) is parked as before.
- **Escaped.** `arena::note_escape(addr)` / `Region::mark_escaped` mark a region whose data was stored in a pool- or
  program-lifetime structure; such a checker is parked as before. Sites: the pool's accumulated global diagnostics
  (every kept diagnostic's region, after each merge), the program's declaration diagnostics cache (when it stores
  diagnostics), the include processor's lazily made diagnostics (the target current when it fills them, when there
  are processing diagnostics). Audited and needing nothing: the program's other lazily filled fields (heap values,
  file pointers, or routed to the full build's region: symlink cache; package.json entries: `enter_table_owner`),
  snapshot and project lazily filled fields (heap values, filled without a checker held), source-file lazy data
  (`enter_owner`), process-wide statics (thread arena). Not covered: the project reference file mapper's
  `realpath_dts_to_source` (shared by program versions, filled lazily; pre-existing for checker regions freed with
  a version, only with project references).

`TSRS_REGION_LOG=1` also prints one line per disposed checker freed or parked (pinned / escaped).

Results (`fast` profile, MiB):

| repro | base | fix | tsgo-ref |
| --- | --- | --- | --- |
| xstate, 100 canceled diagnostic pulls, RSS every 20 cycles | 296, 439, 582, 727, 871 | 162, 163, 163, 162, 163 | 419, 421, 465, 466, 467 |
| heavy file, 150 canceled pulls, RSS at 50 / 100 / 150 | 349, 620, 893 | 84, 84, 84 | 210, 211, 217 |
| heavy file, diagnostics every 36 s (idle disposal each round), rounds 0-5 | 136, 188, 240, 294, 341, 394 | 137, 143, 143, 143, 141, 145 | 471, 849, 845, 756, 504, 623 |

Census (alloc-profile + plain pointers, `TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1`; a session over xstate and a small
`noLib` / `declaration` / `isolatedDeclarations` / missing `types` project: diagnostics with global, declaration and
include-processor diagnostics, hover, definition, references, implementation, rename, completion, signature help,
call hierarchy, inlay hints, semantic tokens, code actions, workspace symbols, 30+ canceled requests, two 36 s idle
waits; 15 disposed checkers freed, 1 parked as escaped, 70 MB of checker regions freed while their program stayed
current): precise walk 0; strong mark 8 violations, all of one class that the base build has too (39 in the same
session): `TypeMapper` (mapper.rs:514) freed by a recycling site while an `IndexInfo` (checker_12.rs:2268) keeps it,
inside a live chunk, i.e. a free-list free, not a region. Poison mode (`TSRS_ARENA_POISON=1`): the concurrency
stress scripts (rapid edits with pipelined and canceled requests, slow requests on replaced snapshots, requests
racing idle disposal), the canceled-pull repro (200 checkers freed) and the census session: no crash, no panic, no
missing response.

The census build could not run LSP diagnostics before this fix: `source_file_and_index_of_node` took the slot of its
4096-entry cache as `key * C >> 20`, out of range for a 64-bit plain-pointer key (index out of bounds on the first
union of declarations); the slot is masked now (compressed builds unchanged).
