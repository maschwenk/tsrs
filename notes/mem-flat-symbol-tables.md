# mem-flat-symbol-tables: frozen binder symbol tables

> Not landed (PR 147, closed 2026-10-07): 8-9 MB less front-end heap on vscode and formbricks-web for +0.4% to +2.5%
> single-threaded instructions and about 340 lines across the binder and the symbol tables is below the complexity bar
> in AGENTS.md (the author's own read was the same). The branch `mem/flat-symbol-tables` keeps the code; a bulk freeze
> without per-table header allocations, or a wider filter, is what could change the trade ("Numbers" below).

The "Flat symbol tables" row of notes/bun-check-memory.md. Bun's `bun check` turns every binder table into a
(start, len) range of one per-file entries list at the end of binding. tsrs had one `SymbolTable` type for binder and
checker: a 24-byte arena header and a heap entry buffer growing 1, 2, 4, ... (notes/mem-round3.md A4, A11). Base:
origin/main 249561e. Layout only; diagnostics were unchanged byte for byte.

## Does anything write to a binder table after binding?

No (at 249561e). Every `SymbolTable::set` / `delete` call in the workspace was traced to its receiver:

- The checker writes only to tables it created: `globals`, new tables (`SymbolTable::new` / `with_capacity`, utilities
  `create_symbol_table`), pattern-augmentation tables, late-bound tables, type property caches, and `clone_table`
  results.
- `merge_symbol` / `merge_symbol_table` write into `globals`, a new combined table, or a transient symbol: a
  non-transient target goes through `clone_symbol` first, which clones its tables. `get_exports_of_module` works on a
  clone. `add_inherited_members` always gets a clone or a new table. `combine_symbol_tables` may hand a binder table on
  unchanged, and nothing writes to it.
- The binder's late passes run inside `bind_once`. `nameresolver.rs` / `referenceresolver.rs`, the language service
  and the API crates only read.

So a frozen form needs no write path. On the branch a write to a frozen table panicked, and the conformance suite, the
ten bench projects at 1 / 4 / 16 checkers and the binder oracle corpus ran without one. A table the checker creates
later on a binder symbol (`get_members` of a symbol that has none) is a new, mutable table.

## What was built

At the end of `bind_source_file` each binder table (except those with a hash index, more than 16 entries, or a key
that is not its symbol's name: 13,758 of 608,224 on vscode) was replaced by an 8-byte `FrozenTable` word (tag bit,
length, a 26-bit key filter from the stored hash bits, the handle of its run in one per-file `[FrozenEntry]` array);
tables of more than 4 entries kept a 64-bit filter before the run. The 24-byte mutable header went back to the arena
for reuse. Every `SymbolTable` method tested the first word for the frozen form.

## Numbers

Front end (`--noCheck`, alloc-profile build, macOS M5 Max): net about -9.4 MB on vscode (heap peak 238.9 -> 225.8
MB, arena used 868.0 -> 871.8 MB) and -8.0 MB on formbricks-web (321.5 -> 309.2 MB, 575.7 -> 580.0 MB), below the
study's 12-15 MB estimate. The 16-byte header saving was mostly spent on the 8-byte `FrozenTable` objects and the wide
filters.

pr-verify (Linux 64 vCPU, base d94c544, new e6e8ee8): diagnostics identical in 60 of 60 cells and in the poisoned-arena
runs. Single-threaded instructions (`bench/count.py`): vscode +0.65%, xstate-main +1.04%, webpack +0.67%, mui-docs
+1.92%, Compiler +0.42%, Compiler-Unions +0.41%, cal-diy +1.27%, formbricks-web +1.29%, supabase-studio +2.52%,
t3code-server +0.49%; single-threaded peak RSS vscode 1.86 -> 1.85 GiB (-0.6%), formbricks-web -0.4%, the
others 0-5 MiB lower. Peak RSS at 4 to 32 checkers moved by -0.7% to +3.4%,
within the spread of those cells.

The instructions are the cost: freezing (about 0.2 G of formbricks-web's 20 G front end) plus the frozen lookup path
and the frozen-form test on every table operation. With the frozen lookup inlined into every caller it was worse
(vscode +1.01%, formbricks-web +1.91%, supabase-studio +3.95%, mui-docs +2.76%). Not tried: a wider header filter,
freezing in a bulk pass without per-table header allocations, or keeping binder tables frozen from the start.
