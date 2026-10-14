# watch-interface-refresh: a comment edit rechecked every file that declares an interface

Found 2026-10-13 while adding the watch benchmark (bench/watch.py, vscode at 9cf0128b9822, M3 Max). A comment edit
in a file nothing imports took 690 ms per cycle; notes/watch-macos.md had measured 48-59 ms before #349. With
`TSRS_CHECKER_ASSIGNMENT=go` the same edit took 70 ms.

## Cause

#349 (174697a6, "stabilize merged-interface error locations") reports TS2320/TS2430 only at the first declaration of a
merged interface on its virtual tsgo checker, so a file's cached semantic diagnostics depend on other files. To keep
incremental results equal to clean builds, `compute_program_file_changes` drops the cached diagnostics of every
type-checked file that contains an interface declaration when the slot signature, the file order, the lib-check
options or the deleted files change, or when any file is in the change set. The last condition holds in every watch
cycle: the edited file is in the change set. So each comment edit rechecked every such file (most of vscode) and walked
every file's AST to find them (`contains_interface_declaration`, the top function of the cycle's profile).

## Fix

A changed file can move a report site only by adding or removing a declaration of a merged interface, so it now
triggers the refresh only when its new or its old text (the old compiler program's file) contains an interface
declaration. Without the old text (a program loaded from build info) every changed file still triggers it. The other
conditions are unchanged: a change that moves checker slots (an edit that changes a file's size, a new `@ts-nocheck`)
changes the slot signature and refreshes as before.

## Numbers (vscode, `--watch -p src --noEmit`, M3 Max, `/usr/bin/time -l`)

A session with 20 comment edits minus a session with none, divided by 20:

| per comment edit | before (main 8dfcb14a) | after |
| --- | ---: | ---: |
| instructions retired | 41.9 G | 0.56 G |
| CPU (user + sys) | 5.5 s | 0.12 s |
| wall (report after the write) | 690 ms | 85 ms |

Peak RSS over the 20-edit session 3.51 -> 2.78 GB (the no-edit sessions: 2.78 GB both). The initial build is the same
(125 G instructions each). The bench projects' single-threaded counts do not change: the code runs only with an old
program.

## Checks

- `crates/tsrs_incremental/tests/merged_interface.rs`: a size-preserving comment edit in a file without interfaces
  keeps the interface caches (`Reused`); adding a declaration of the merged interface, and removing it again,
  refreshes them; diagnostics equal a clean build each time. Fails before the fix.
- `merged_interface_check_site` (CLI), the `tsrs_incremental` and `tsrs_execute` tests, `tools/regressions.sh`: pass.
- tsctests: the same 484 passing baselines with and without the fix.
- `tools/watch/watchcheck.py` on vscode (12 edits, including a hub export, a new file and an export type change):
  every cycle's diagnostics equal a fresh build.
