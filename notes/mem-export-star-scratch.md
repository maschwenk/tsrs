# mem-export-star-scratch: no dead scratch per `export *` re-export

Found by notes/mem-per-checker-duplication.md (section 2e,
https://github.com/maschwenk/tsrs/pull/133). Every `export *` resolution left one arena record and one heap string per
re-exported name that nothing could reach once the resolution returned. Each checker resolves the module exports it
needs, so the leak grew with the checker count. The fix makes the records owned by the local table that holds them.
Output does not change. Peak memory at 32 checkers dropped by 150 MiB on formbricks-web, 86 MiB on cal-diy and 36 MiB
on supabase-studio; vscode did not change.

Base: origin/main e4f82c0. macOS, M5 Max (18 cores). Peak = `/usr/bin/time -l` peak memory footprint. MiB = 2^20 bytes.

## Mechanism

Go `getExportsOfModuleWorker` collects name collisions between the modules of a module's `export *` declarations in
`lookupTable`, a `map[string]*ExportCollision` local to one `visit` call; `extendExportSymbols` inserts a record
`{specifierText, exportsWithDuplicate}` for every name it copies. After the loop, `visit` reports TS2308 for each
record with duplicates, and Go's GC frees the records.

The port allocated each record with `P::new` in the arena, with a heap `String` (the specifier text) inside. Arena
objects are never dropped, so the 64-byte record and its string stayed allocated. The records never leave `visit`,
so now the table owns them: `ExportCollisionTable` is `FxHashMap<String, ExportCollision>`, `ExportCollision` is
`{ specifier_text: String, exports_with_duplicate: Vec<P<Node>> }` without `RefCell`s (crates/tsrs_checker/src/checker.rs),
and both are dropped with the table. Control flow, insertion order, key type and hashing are unchanged.

Records before the fix (`ExportCollision`, 64 B each, base alloc profile):

| project | 1 checker | 16 checkers | 32 checkers |
| --- | ---: | ---: | ---: |
| formbricks-web | 115,688 (7.1 MiB) | 1,161,656 (70.9 MiB) | ~2.13M (130.2 MiB) |
| cal-diy | 63,768 (3.9 MiB) | 679,752 (41.5 MiB) | ~1.31M (80.0 MiB) |
| supabase-studio | 81,384 (5.0 MiB) | 327,216 (20.0 MiB) | ~0.44M (26.8 MiB) |
| vscode | 0 | 0 | 0 |

## Measurement

Peak footprint, release build, 4 interleaved runs per cell (median):

| project | checkers | before MiB | after MiB | change |
| --- | ---: | ---: | ---: | ---: |
| formbricks-web | 16 | 2,398 | 2,300 | -99 (-4.1%) |
| formbricks-web | 32 | 3,056 | 2,906 | -150 (-4.9%) |
| cal-diy | 16 | 2,036 | 1,972 | -64 (-3.1%) |
| cal-diy | 32 | 2,688 | 2,602 | -86 (-3.2%) |
| supabase-studio | 16 | 1,875 | 1,846 | -29 (-1.5%) |
| supabase-studio | 32 | 2,394 | 2,357 | -36 (-1.5%) |
| vscode (control) | 16 / 32 | 2,515 / 2,790 | 2,515 / 2,794 | 0 / +4 |

Instructions (formbricks-web `--singleThreaded`, 5 runs): 58.01 G -> 57.91 G median, no measurable change.
Diagnostics byte-identical on the ten bench projects at 1, 4 and 16 checkers and on a hand-written TS2308 project
(16 diagnostics); poison and census runs clean.
