# watch-tsgo-slots: a reused program takes over the import graph of tsgo's default-pool slots

After notes/watch-interface-refresh.md, a comment edit on vscode (9cf0128b9822, `--watch -p src --noEmit`, M3 Max)
took 85 ms per cycle, against 48-59 ms before #349 (notes/watch-macos.md). Timers around the cycle's phases put
27 ms in `program_to_snapshot`'s canonical check signature: the hash of each file's slot on tsgo's default
four-checker pool (`checkerpool::tsgo_default_check_slots`, #349). Every program builds that table once, and every
watch cycle has a new program. Of the 21 ms the table took, 19 ms was the import graph (`get_import_adjacency`:
the resolved module of every import of every file, serially, because the table can be built while a checker is
held) and 1.4 ms the FENNEL pass over it.

## Change

A program made by `reuse_program` has the files of the program it reuses at the same indices, and the reparsed
file has the same imports (the precondition `inherited_associations` already relies on), so its import graph is the
same. `Program::tsgo_import_adjacency` keeps the graph; `reuse_program` copies it from the old program, and the slots
are computed from it. The weights and the FENNEL pass are computed again: the edit can change the reparsed file's
text length, which is part of its weight (`a_reused_program_computes_the_tsgo_slots_of_a_fresh_one` has an edit that
moves a slot). Inheriting the slot table itself would be wrong for such edits.

## Numbers (vscode, `/usr/bin/time -l`, a 20-edit session minus a no-edit session, divided by 20)

| per comment edit | before (main cd9ca379) | after |
| --- | ---: | ---: |
| instructions retired | 0.555 G | 0.382 G |
| wall (report after the write) | 85 ms | 48 ms |

Wall falls more than instructions: the graph was built serially on the cycle's critical path. Peak RSS over the
session is unchanged (2.77-2.79 GB both). The initial build is unchanged (124.4-124.5 G instructions). The bench
projects' single-threaded counts do not change: the graph is reused only by programs `reuse_program` makes.

## Checks

- `a_reused_program_computes_the_tsgo_slots_of_a_fresh_one` (tsrs_compiler): the reused program shares the old
  program's graph and computes the slots a fresh program of the edited files computes, for an edit that moves a slot.
- tsrs_compiler, tsrs_incremental, tsrs_execute tests, `merged_interface_check_site`, `tools/regressions.sh`: pass.
- tsctests: the same 484 passing baselines.
- `tools/watch/watchcheck.py` on vscode (12 edits): every cycle's diagnostics equal a fresh build.
