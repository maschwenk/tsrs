# mem-flat-symbol-tables: frozen binder symbol tables

The "Flat symbol tables" row of notes/bun-check-memory.md. Bun's `bun check` turns every binder table into a
(start, len) range of one per-file entries list at the end of binding. tsrs had one `SymbolTable` type for binder and
checker: a 24-byte arena header (`EntryVec` pointer / length / capacity, plus the filter-or-extra word) and a heap
entry buffer growing 1, 2, 4, ... (notes/mem-round3.md A4, A11). Base: origin/main 249561e. Layout only; diagnostics
are unchanged and were checked byte for byte (Gates).

## Does anything write to a binder table after binding?

No. Every `SymbolTable::set` / `delete` call in the workspace was traced to its receiver (a read-only audit of the
checker, binder, language service, API, compiler, transformers and declarations crates):

- The checker writes only to tables it created: `globals` (`checker.rs`, `with_capacity`), new tables
  (`SymbolTable::new` / `with_capacity` in checker_03/07/08/09/10/12/14/15, inference, jsx, services, printer,
  utilities `create_symbol_table`), the pattern-augmentation tables, late-bound tables, type property caches, and
  `clone_table` results.
- `merge_symbol` / `merge_symbol_table` write into `globals`, a new combined table, or a transient symbol: a
  non-transient target goes through `clone_symbol` first, which clones its tables (checker_07.rs:1732-1736,
  1917-1918). `get_exports_of_module` works on a clone (checker_08.rs:1322). `add_inherited_members` always gets a
  clone or a new table. `combine_symbol_tables` may hand a binder table on unchanged, and nothing writes to it.
- The binder's late passes run inside `bind_once`. `nameresolver.rs` / `referenceresolver.rs` (used by the checker,
  emit resolver and auto-import) only read. The language service and API crates only read. The other crates'
  `.set` / `.delete` calls are on maps and JSON, not tables.

So the frozen form needs no write path, and a write to a frozen table panics (`map_mut`). The conformance suite, the
ten bench projects at 1 / 4 / 16 checkers and the binder oracle corpus ran without one. A table the checker creates
later on a binder symbol (`get_members` of a symbol that has none) is a new, mutable table.

## What changed

- `SymbolTable` keeps today's mutable form, now `repr(C)` down to `EntryVec::ptr`, so that its first word is always
  the entry buffer's address (below 2^48). It uses a local `UnsafeCell` with `FrozenCell`'s contract instead of a
  `FrozenCell`, whose layout is free in checked builds.
- A frozen table (`FrozenTable`) is one 8-byte word that a `P<SymbolTable>` handle can point at: bit 63 tags it. It
  holds the length (bits 58..63), a 26-bit filter of the keys derived from the 12 hash bits each entry already
  stores (no rehash), and the handle of its run in one exactly sized per-file arena array of entries
  (`[FrozenEntry]`). Tables of more than 4 entries keep their mutable form's 64-bit filter in the word before the
  run: a 26-bit filter fills up for them, and without it most misses searched the whole run. Every `SymbolTable`
  method reads the first word, then takes the frozen path (slices of the run) or today's path.
- At the end of `bind_source_file` the binder replaces each table it made (it records the owner of every table it
  creates: a node's `locals`, a symbol's `members` / `exports`) by its frozen form (`freeze_symbol_tables`), points
  the owner at it, and gives the old 24-byte table back to the arena (`release_symbol_table`). It frees the heap
  entries; the next file's tables reuse the block (`SymbolTable::new_recycled`). Tables with a hash index (more than
  16 entries) or a key that is not its symbol's name stay mutable: 13,758 of 608,224 binder tables on vscode. Bun
  gives those one per-file hash map; that is not done here.
- Iteration stays insertion order and lookups compare by text as before. A clone of a frozen table is mutable.
- Census builds (plain pointers) register the frozen word as a tagged pointer and read the frozen-entries array like
  heap entry buffers (`tsrs_core/src/alloc_profile/census.rs`, strong mark).

## Numbers

Allocation profile: macOS, M5 Max 18 cores, release builds (other agents were running, load average 13-86; byte counts
do not depend on load).

Allocation profile, front end (`--noCheck`, alloc-profile build). "Arena used" is the arena chunks minus their unused
tails. "Arena requested" counts free-list reuse as new requests, so it does not show the reuse of released tables:

| | vscode before | vscode after | formbricks-web before | formbricks-web after |
| --- | --- | --- | --- | --- |
| `SymbolTable` (24 B) allocations | 608,224 / 13.9 MB | 608,224 / 13.9 MB, 594,466 released and reused | 564,751 / 12.9 MB | 564,751 / 12.9 MB, 551,426 released |
| `FrozenTable` (8 B) | - | 594,466 / 4.5 MB | - | 551,426 / 4.2 MB |
| `[FrozenEntry]` arrays | - | 10,341 / 12.0 MB | - | 10,615 / 10.7 MB |
| heap current | 183.8 MB | 170.6 MB | 280.0 MB | 267.7 MB |
| heap peak | 238.9 MB | 225.8 MB | 321.5 MB | 309.2 MB |
| arena used | 868.0 MB | 871.8 MB | 575.7 MB | 580.0 MB |
| arena requested | 873.8 MB | 890.3 MB | 571.5 MB | 586.4 MB |

Net front end: about -9.4 MB on vscode and -8.0 MB on formbricks-web, below the study's 12-15 MB estimate. The
entry buffers' slack and heap blocks were the larger part (13 MB of heap). The 16-byte header saving is mostly
spent on the 8-byte `FrozenTable` objects and the wide filters.

pr-verify (`.depot/workflows/pr-verify.yml`, Linux 64 vCPU, base d94c544, new e6e8ee8): diagnostics identical in
60 of 60 cells (ten projects at 1 / 4 / 16 / 32 checkers) and in the poisoned-arena run of every project.
Single-threaded instructions (`bench/count.py`, exact to about 0.001%) and peak RSS:

| project | instructions | single-threaded peak RSS |
| --- | ---: | ---: |
| vscode | 113.793 -> 114.534 G (+0.65%) | 1.86 -> 1.85 GiB (-0.6%) |
| xstate-main | 7.365 -> 7.441 G (+1.04%) | 191 -> 190 MiB |
| webpack | 14.002 -> 14.095 G (+0.67%) | 311 -> 309 MiB |
| mui-docs | 50.938 -> 51.914 G (+1.92%) | 744 -> 741 MiB |
| Compiler | 2.370 -> 2.380 G (+0.42%) | 58 -> 58 MiB |
| Compiler-Unions | 5.474 -> 5.496 G (+0.41%) | 62 -> 62 MiB |
| cal-diy | 41.053 -> 41.573 G (+1.27%) | 820 -> 817 MiB |
| formbricks-web | 52.747 -> 53.430 G (+1.29%) | 1.32 -> 1.32 GiB (-0.4%) |
| supabase-studio | 64.477 -> 66.103 G (+2.52%) | 944 -> 939 MiB |
| t3code-server | 57.738 -> 58.023 G (+0.49%) | 828 -> 825 MiB |

Peak RSS with 4 to 32 checkers moves by -0.7% to +3.4%, within the run-to-run spread of those cells.

The instructions are the cost. Freezing costs about 0.2 G of formbricks-web's 20 G front end (macOS, `--noCheck`):
the heap frees of the old entry buffers and the header allocations. The rest is the frozen lookup path (vscode does
14.5M frozen-table lookups, 8.4M rejected by the header filter, 4.7M hits; formbricks-web 12.1M) and the frozen-form
test on every table operation. Steps measured on the way (single-threaded instructions):

| version | vscode | formbricks-web | supabase-studio | mui-docs |
| --- | ---: | ---: | ---: | ---: |
| frozen lookup inlined into every caller | +1.01% | +1.91% | +3.95% | +2.76% |
| frozen lookup out of line (this branch) | +0.65% | +1.29% | +2.52% | +1.92% |

Earlier local versions were worse still: no length in the header (frozen iteration had no size hint), no wide
filter, and filters rehashed from the names at freeze time. Ways to take more off, not tried: a wider header filter
(the 26 bits miss more often than the mutable form's 64), freezing in a bulk pass without the per-table header
allocation, or keeping the binder's tables in the frozen form from the start (no release). At these numbers the trade
is about 8-9 MB of front-end memory (0.3-0.6% of single-threaded peak) for 0.4-2.5% more instructions.

## Gates

- pr-verify (above): 60 of 60 cells identical, poisoned-arena runs identical. Locally (macOS, before the rebase onto
  #139): `--pretty false` stdout plus exit code byte-identical on the ten projects at 1, 4 and 16 checkers (30/30);
  `TSRS_ARENA_POISON=1` at 16 checkers on vscode and formbricks-web identical.
- Census (`TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1 TSRS_CENSUS_ASSERT=1`, plain pointers), one checker: 0 violations,
  0 references to freed blocks, 0 unreachable symbols, 0 not recorded on both projects. The released tables
  are in the would-free set (4.18M / 4.39M blocks), so the strong mark found no owner still pointing at an old table.
  Diagnostics unchanged.
- Binder oracle (`tools/oracle/binder` against Go at the pinned commit, 12,760 test files): only the 8 known
  non-UTF-8 / BOM files differ, as on the base; the dump prints every table through `entries()`.
- Conformance suite (`--baselines types,symbols`): 13,458 error baselines and 12,779 `.types` / `.symbols` pass,
  pass list identical to the base.
- Unit test `symbol_table_freeze` (crates/tsrs_ast/src/ast.rs), with compressed and plain pointers.
