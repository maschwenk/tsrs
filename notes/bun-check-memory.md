# bun-check-memory: what Bun's `bun check` does differently on memory, and what transfers

A code study of oven-sh/bun PR 44361 (`bun check`, a Rust port of typescript-go 7.0.2, MIT) made on 2026-10-07 to
answer "why does bun check use less peak memory than tsrs below tsrs's default thread count, and what of it transfers
under tsrs's rule that diagnostics must equal tsgo's". Six readers (AST and text, binder, types, parallel
architecture, allocation model, Bun's own measurements) each read Bun's source against tsrs's and the notes under
notes/mem-*.md; every candidate technique then went to two adversarial checks (are the byte claims right; does it
transfer). The study was stopped early for cost: 17 of the 30 checks ran. Statuses below say which. File:line
citations refer to Bun's `src/sema` at the PR head and to this repository at 626da9d.

Status (2026-10-10): the "tsrs" column describes 626da9d; several rows were followed up since. Leaf freeing landed for
CLI `--noEmit` runs with at most 16 checkers (`TSRS_FREE_LEAVES`, docs/DEBUGGING.md; notes/mem-free-leaf-files.md).
Bundled libs are parsed zero-copy from the binary's text (notes/mem-zero-copy-libs.md; crates/tsrs_compiler/src/host.rs).
Dense link tables: the 60-90 MB did not hold; the store layouts that paid landed (`InlineIdStore`, `KeyedLinkStore`,
`SymbolReferenceLinkStore` in crates/tsrs_checker/src/links.rs; notes/mem-dense-link-tables.md,
notes/mem-link-tables-landed.md). Flat symbol tables were not landed (PR 147
closed, notes/mem-flat-symbol-tables.md). Flow compaction was reverted (#191, notes/mem-flow-compaction.md). Read
those notes before proposing any of these rows again.

Measured context (vscode, 64 vCPU): at each tool's default tsrs 0.60 s / 2.87 GiB vs bun check 0.87 s / 2.85 GiB;
at 4 / 8 / 16 / 64 threads bun check uses 35% / 33% / 28% / 14% less; across ten projects tsrs is faster on all and
uses 1.18-1.90x bun's memory on cal-diy, formbricks-web, supabase-studio and t3code-server (README table of
2026-10-07). The gap is the base footprint (front end plus the first checker), not per-thread growth.

Leaf files (checked source files no other file imports; `TSRS_ASSIGNMENT_DUMP` in-degree, 2026-10-07): vscode 2,354
of 9,399 checked files, 37.4% of all AST nodes (99% test files); t3code-server 25.4%; formbricks-web 11.5%;
supabase-studio 8.9%; cal-diy 2.3%. This is the share Bun's leaf-freeing technique (below) can reach; whether a leaf
also "adds nothing" global was not counted.

## 1. How Bun uses less memory

The mechanisms are read from Bun's code; the byte split is inferred, because Bun's front-end-only peak (standalone/main.rs:519-524) was never published.

- **Smaller front end (read).** A per-kind-vector HIR: Expr is 24 B with no header, id, flags or parent (hir.rs:424-429), names are inline u32 atoms (hir.rs:461-467), lists are 8-byte ranges (hir.rs:97-108), parents are lazy (node.rs:1355-1433). The reader's ~430 MB rebuild against tsrs's ~700 MB tree is inferred; verdicts cut its components.
- **Bun frees what tsrs keeps (read).** Leaf-file HIR and binder output after the checking task (program.rs:4936-4949), task-local records no published entry reaches (task.rs:182-237), all but diagnostics in the last step (task.rs:98-99, 207-216), default-lib text (program.rs:4874-4881). tsrs never frees (ptr.rs:23-32).
- **Fewer checker objects (design read, bytes inferred).** Shared member shapes with no instantiated symbols (shape.rs:144-186, 791-865) and dense zero-page link tables (table.rs:1-11, 419-531), against tsrs's ~2.46M checker-created symbols (79 MB) plus links and members, ~214 MB by the reader.

Readers disagree whether the shared graph explains the base gap (parallel-arch: no; measure: yes); no verdict settled it.

## 2. Techniques

Ranked by midpoint saving, vscode. Front-end items save the same at 32 checkers (shared AST); checker-side items are one-checker estimates only.

| Technique | Bun | tsrs | Saving | Effort | Fidelity risk | Status |
|---|---|---|---|---|---|---|
| Publish-and-discard shared graph | scratch freed at barrier | N retained | 400-700 MB if Bun's fidelity accepted (measure reader); parallel-arch: ~0 | months | high | unverified |
| Free leaf AST and binder output | free_tree | never frees | 80-250 MB if leaves are 15-30% of bytes; share unmeasured | 1-4 weeks | low if audit exact; use-after-free hazard | refuted-with-correction (AST, both); binder variant verified facts only |
| Shared shapes, no instantiated symbols | Prop plus mapper | Symbol and links per member | ~150 MB | months | high: symbol ids, counters | unverified |
| Names as inline atoms | Atom fields | 32 B Identifier nodes | 75-110 MB (was 115-140) | months, not layout-only | low | refuted-with-correction (both) |
| Dense link tables | ByNode/ById | 27 hashed stores | 60-90 MB | weeks-months | none in principle | unverified |
| Header-less rows, lazy parents | no header | 24 B header | 60-100 MB (was 210-270); AST rewrite | months | low | refuted-with-correction (both) |
| Drop unreachable records | OwnStore marks | never freed | tens of MB (~95 bound) | months | high | unverified |
| Lists as inline ranges | Span/IdList | NodeList plus slice | 10-44 MB; lenses disagree | weeks | medium: printer, LS, API codec | refuted-with-correction (both) |
| Hash-consed types | interned | per-call | 0-30 MB | months | high | unverified |
| 3 MB task stack | 3 MB | 512 MB | unknown; reader guess tens of MB | days | high if hit | unverified |
| Flat symbol tables | flat ranges | 24 B headers | 12-15 MB | days-1 week | low; keep insertion order | verified facts only |
| Inline single declaration | One(Decl) | 1-element slices | at most 9.7 MB, likely cancelled | days | none | unverified |
| Literal TokenFlags out of node | none kept | 40 B literal | 0-7.7 MiB; lenses disagree | days | none | refuted-with-correction (both) |
| Flow compaction | flat edges | per-label lists | 4-9 MB | days | low | verified both lenses (corrected) |
| Zero-copy lib text | text dropped | heap copy | ~2.8 MB | hours | none | Bun's drop refuted; zero-copy confirmed |

## 3. Refuted

- Instantiation caches keyed by result arguments: 0 MB; values do not determine keys (checker_11.rs:1294-1330). Pointer-to-slice variant 4-7 MB, inferred, facts lens only.
- Bun-style lib text drop: net 0-1 MB, medium risk; replaced by the zero-copy row.
- Header-less pieces alone: id-as-handle saves 0 B (8-byte rounding); lazy parents are net negative; token removal is a checker-API rewrite.
- Numeric literals into a table: 0 MB. String atoms rebuild the interner rejected for +9% parse (mem-round2.md "Global identifier interner (step 9, rejected)").

## 4. Recommended order of work

My sum of verified items needing weeks or less (leaf, tables, flow, lib, literals) is ~100-285 MB against a ~0.7 GiB gap; the largest rests on an unmeasured share.

1. **Measure (days).** First action: count-only pass of Bun's leaf test (program.rs:4551-4555, 5314-5323) over arena bytes on vscode and the four large app projects. Also get Bun's "peak of loading" (--timing harness) and the resident share of the 269 MB arena chunk tail.
2. **Small verified batch (~19-27 MB, layout-only).** First action: route bundled libs through read_embedded_file (embed.rs:51-54) and parse_source_file_static (host.rs:76-79). Then flow compaction, flat symbol tables.
3. **Leaf freeing for noEmit runs, only if step 1 shows a large share.** First action: per-file Regions in the CLI behind a retains-everything switch; guard printer.rs:571-630 and 827-850. Needs a decision to relax the census gate to "no dereference". Diagnostics unchanged if the audit is complete; not a Bun-determinism change.
4. **Verify checker-side items first.** First action: run the missing verdicts for dense link tables and shared shapes (shapes change which symbols exist).
5. **AST redesign (atoms, header-less, lists):** months-long; not before 1-4.

Publish-and-discard, hash-consed types, dropping unreachable records and the small stack need Bun-style determinism or a new exactness proof; not without Max's decision.

## 5. Inferred or not verified

- Inferred: Bun's HIR rebuild total; most Bun type sizes (few are asserted); Bun's 1.35 GiB extrapolation; vscode heap scaled from the monorepo.
- Not found: Bun's mimalloc settings (bun_alloc not in the checkout; per-block freeing rests on a doc comment, session.rs:3); a vscode leaf count; Bun's front-end-only peak; what its 21-34 MiB per thread is.
- 13 of 30 verdicts never ran; reader duplicates of leaf freeing and the bundled front-end item have none.
- Checker-side savings were never estimated at 32 checkers.
