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

macOS, M5 Max 18 cores, release builds. Other agents were running (load average 13-86), so wall time is not
reported, and instruction counts varied by up to 3% between identical runs.

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

`/usr/bin/time -l`, `--singleThreaded`, 8 interleaved runs each (min / median / max), base -> new:

| project | instructions retired | peak memory footprint |
| --- | --- | --- |
| vscode | 110.25 / 111.91 / 114.80 G -> 111.87 / 113.07 / 113.53 G | 1993.6 / 1998.8 / 1999.9 MB -> 1989.5 / 1991.5 / 1996.3 MB |
| formbricks-web | 54.47 / 54.82 / 55.12 G -> 54.98 / 55.53 / 56.17 G | 1412.5 / 1414.3 / 1415.5 MB -> 1405.4 / 1405.8 / 1408.2 MB |

Instructions are about +1% (medians +1.0% / +1.3%). Freezing costs ~0.2 G of formbricks-web's 20 G front end
(the heap frees of the old entry buffers, the header allocations). The rest is the frozen lookup path: vscode
does 14.5M frozen-table lookups (8.4M rejected by the header filter, 4.7M hits), formbricks-web 12.1M. A first
version without the length in the header and without the wide filter cost +1.6% on formbricks-web. Peak footprint
is 5-9 MB lower.

## Gates

- `--pretty false` stdout plus exit code byte-identical to the base on vscode, webpack, xstate-main, mui-docs, cal-diy,
  formbricks-web, supabase-studio, t3code-server, Compiler and Compiler-Unions at 1, 4 and 16 checkers (30/30).
- `TSRS_ARENA_POISON=1` at 16 checkers on vscode and formbricks-web: identical to the base.
- Census (`TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1 TSRS_CENSUS_ASSERT=1`, plain pointers), one checker: 0 violations,
  0 references to freed blocks, 0 unreachable symbols, 0 not recorded on both projects. The released tables
  are in the would-free set (4.18M / 4.39M blocks), so the strong mark found no owner still pointing at an old table.
  Diagnostics unchanged.
- Binder oracle (`tools/oracle/binder` against Go at the pinned commit, 12,760 test files): only the 8 known
  non-UTF-8 / BOM files differ, as on the base; the dump prints every table through `entries()`.
- Conformance suite (`--baselines types,symbols`): 13,458 error baselines and 12,779 `.types` / `.symbols` pass,
  pass list identical to the base.
- Unit test `symbol_table_freeze` (crates/tsrs_ast/src/ast.rs), with compressed and plain pointers.
