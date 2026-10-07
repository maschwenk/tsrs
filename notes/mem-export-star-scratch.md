# mem-export-star-scratch: no dead scratch per `export *` re-export

Found by notes/mem-per-checker-duplication.md (section 2e,
https://github.com/maschwenk/tsrs/pull/133). Every `export *` resolution left one arena record and one heap string per
re-exported name that nothing could reach once the resolution returned. Each checker resolves the module exports it
needs, so the leak grows with the checker count. The fix makes the records owned by the local table that holds them.
Output does not change. Peak memory at 32 checkers drops by 150 MiB on formbricks-web, 86 MiB on cal-diy and 36 MiB on
supabase-studio. vscode does not change.

Base: origin/main e4f82c0. macOS, M5 Max (18 cores). Peak = `/usr/bin/time -l` peak memory footprint. MB and MiB are
2^20 bytes.

## Mechanism

Go `getExportsOfModuleWorker` (checker.go:16471) collects name collisions between the modules of a module's
`export *` declarations in `lookupTable`, a `map[string]*ExportCollision` local to one `visit` call.
`extendExportSymbols` (checker.go:16558) inserts a record `{specifierText, exportsWithDuplicate}` for every name it
copies into `nestedSymbols`. When a name arrives again from a different symbol, it appends the export node. After the
loop, `visit` reports TS2308 for each record that has duplicates, and then the map goes out of scope. Go's GC frees the
records.

The port allocated each record with `P::new` in the leak arena (crates/tsrs_checker/src/checker_08.rs:1421 at e4f82c0),
with a heap `String` (the module specifier text from `get_text_of_node`) and a `Vec` inside `RefCell`s. The table is
dropped at the end of `visit`, but arena objects are never dropped. So the 64-byte record stayed allocated, and the
string it owned was never freed either.

The records never leave `visit`. They are created in `extend_export_symbols`, read in the report loop of the same
`visit` call, and the diagnostics copy the specifier text when they format the message. So the table can own them:

- `ExportCollisionTable` is `FxHashMap<String, ExportCollision>` (was `FxHashMap<String, P<ExportCollision>>`).
- `ExportCollision` is `{ specifier_text: String, exports_with_duplicate: Vec<P<Node>> }` without `RefCell`s
  (crates/tsrs_checker/src/checker.rs).
- `extend_export_symbols` inserts the record by value and appends through `get_mut`. The report loop reads the
  record in place instead of cloning the vector and the string first (crates/tsrs_checker/src/checker_08.rs).

Records and strings are dropped with the table. The Rust compiler checks that no reference outlives the table. No arena
memory is freed or reused, so there is nothing for the census free-gate or the poison mode to catch. Both were run
anyway (below). The control flow, the insertion order, the key type and the hashing are unchanged, so the table
iterates in the same order as before. Diagnostics are sorted on read in any case.

How many records there were, from the base binary's alloc profile (`ExportCollision`, 64 B each):

| project | 1 checker | 16 checkers | 32 checkers |
| --- | ---: | ---: | ---: |
| formbricks-web | 115,688 (7.1 MiB) | 1,161,656 (70.9 MiB) | ~2.13M (130.2 MiB) |
| cal-diy | 63,768 (3.9 MiB) | 679,752 (41.5 MiB) | ~1.31M (80.0 MiB) |
| supabase-studio | 81,384 (5.0 MiB) | 327,216 (20.0 MiB) | ~0.44M (26.8 MiB) |
| vscode | 0 | 0 | 0 |

The 32-checker counts are the MiB divided by 64 B. On formbricks at 16 checkers the census also found 1.23M specifier
strings (24.0 MiB) that were 99.9% unreachable, all allocated by `extend_export_symbols`.

## Measurement

Peak footprint, release build, 4 interleaved runs per cell (median, range in parentheses). Stealing moves the peak by
up to 70 MiB from run to run on cal-diy:

| project | checkers | before MiB | after MiB | change |
| --- | ---: | ---: | ---: | ---: |
| formbricks-web | 16 | 2,398 (2,388-2,402) | 2,300 (2,286-2,311) | -99 (-4.1%) |
| formbricks-web | 32 | 3,056 (3,047-3,072) | 2,906 (2,886-2,915) | -150 (-4.9%) |
| cal-diy | 16 | 2,036 (2,024-2,043) | 1,972 (1,962-1,998) | -64 (-3.1%) |
| cal-diy | 32 | 2,688 (2,648-2,717) | 2,602 (2,581-2,647) | -86 (-3.2%) |
| supabase-studio | 16 | 1,875 (1,871-1,879) | 1,846 (1,841-1,851) | -29 (-1.5%) |
| supabase-studio | 32 | 2,394 (2,385-2,398) | 2,357 (2,351-2,372) | -36 (-1.5%) |
| vscode (control) | 16 | 2,515 (2,513-2,517) | 2,515 (2,515-2,521) | 0 |
| vscode (control) | 32 | 2,790 (2,786-2,801) | 2,794 (2,792-2,807) | +4 |

The same with stealing off (`--checkerAssignment locality`, 1 run each), before -> after MiB:

| checkers | formbricks-web | cal-diy | supabase-studio | vscode |
| ---: | --- | --- | --- | --- |
| 16 | 2,300 -> 2,206 (-94) | 1,766 -> 1,722 (-44) | 1,814 -> 1,804 (-9) | 2,481 -> 2,463 (-18) |
| 32 | 2,878 -> 2,740 (-137) | 2,215 -> 2,131 (-84) | 2,325 -> 2,269 (-56) | 2,730 -> 2,717 (-13) |

Alloc-profile build (`--features alloc-profile`, `TSRS_ALLOC_PROFILE_TOP=20`). One run at 1 checker and the mean of
2 runs at 16 and 32. "Arena" is arena requested and "heap" is heap current at exit, both in MiB. The arena saving
should equal the base's `ExportCollision` row. The arena totals at 16 and 32 checkers also move by up to 35 MiB from
run to run with stealing, so the per-row number is the exact one.

| project | checkers | arena before -> after | heap before -> after | `ExportCollision` row before |
| --- | ---: | --- | --- | ---: |
| formbricks-web | 1 | 931.5 -> 924.4 (-7.1) | 422.3 -> 420.2 (-2.1) | 7.1 |
| formbricks-web | 16 | 1,584.5 -> 1,509.5 (-75.0) | 770.7 -> 749.7 (-21.0) | 74.9 |
| formbricks-web | 32 | 2,035.0 -> 1,900.2 (-134.9) | 1,014.2 -> 969.5 (-44.8) | 130.2 |
| cal-diy | 1 | 564.8 -> 560.9 (-3.9) | 232.1 -> 230.7 (-1.4) | 3.9 |
| cal-diy | 16 | 1,258.2 -> 1,237.4 (-20.8) | 648.2 -> 647.6 (-0.7) | 41.4 |
| cal-diy | 32 | 1,630.3 -> 1,538.7 (-91.7) | 904.8 -> 875.2 (-29.5) | 80.0 |
| supabase-studio | 1 | 651.0 -> 646.0 (-5.0) | 267.4 -> 265.8 (-1.6) | 5.0 |
| supabase-studio | 16 | 1,180.7 -> 1,160.6 (-20.1) | 569.5 -> 563.6 (-5.9) | 19.9 |
| supabase-studio | 32 | 1,496.2 -> 1,471.8 (-24.3) | 740.8 -> 740.5 (-0.4) | 26.8 |
| vscode | 1 / 16 / 32 | -0.4 / -3.9 / -1.5 | 0.0 / -2.2 / +0.5 | 0 |

The `ExportCollision` row is gone after the fix in every run. At 1 checker the row is below the top 20, so the
1-checker values in that column come from a `TSRS_ALLOC_PROFILE_TOP=60` run of the same base binary.

Instructions, formbricks-web `--singleThreaded`, 5 interleaved runs each: before median 58.01 G (57.68-58.92), after
57.91 G (57.72-58.89). That is no measurable change. The fix removes one arena allocation per record. The string
allocation stays, and now it is freed.

The machine was shared (load 13-31 on 18 cores), so wall times are not reported. Memory and instruction counts do not
depend on load.

## Gates

- Diagnostics: the full `--pretty false` output and the exit code are byte-identical (`cmp`) between the base binary
  and the new one on all ten bench projects at 1, 4 and 16 checkers. The projects are vscode, xstate-main, webpack,
  mui-docs, Compiler, Compiler-Unions, cal-diy, formbricks-web, supabase-studio and t3code-server (371 / 0 / 840 / 0 /
  43 / 41 / 136 / 0 / 9 / 6 errors).
- None of the bench projects reports TS2308. The only code that reads the records is the TS2308 report loop. A
  hand-written project exercises it: conflicting `export *` from three modules, a name that resolves to the same
  symbol through two paths, a module that exports the name itself, `export type *`, nested and cyclic `export *`,
  `export * as`, `default`, a missing module, and both quote styles. It produces 16 TS2308 diagnostics, including
  repeated duplicates of one name. The output is byte-identical between base and new at 1 and 4 checkers.
- `TSRS_ARENA_POISON=1` (alloc-profile build), 16 checkers, formbricks-web, cal-diy and supabase-studio: output and
  exit code identical to the base, no panic.
- Census (`--features alloc-profile,tsrs_core/plain-ptrs`, `TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1`), formbricks-web at 1
  and 16 checkers:
  - 0 strongly reachable freed blocks.
  - 0 program references to freed or rewound blocks.
  - No `ExportCollision` row.
  - No unreachable `get_text_of_node_from_source_text` strings (base at 16 checkers: 23.9 MiB).
- `cargo check --workspace --locked`: no warnings.
- `tools/lint/ratchet.py`: ok (11 findings, none new).
- `tools/lint/source.py`: ok.
- `cargo test -p tsrs_checker`: 3 passed.
- Not run locally: the conformance suite and fourslash. This worktree has no `ts-ref/tsc/testdata`. CI's conformance
  and fourslash gates run on the PR.
