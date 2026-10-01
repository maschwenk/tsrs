**Title:** answer empty object type checks from lazy member tables

**Base:** stacked on #64475 (needs its lazy member tables) and on the tuple PR (pr-01). Applies without the tuple PR too, the numbers below are on top of it.

---

stacked on:

- https://github.com/microsoft/TypeScript/pull/64475

`isEmptyObjectType` and the empty object test in `removeSubtypes` both do `isEmptyResolvedType(resolveStructuredTypeMembers(t))`. for an instantiated reference with a lazy member table, that resolves the table in full (every member instantiated, every inherited one merged in) just to find out it has a property

a type with a lazy member table is an instantiated reference, so it's never `anyFunctionType`, and the table already has its signatures and index infos. so this adds `isEmptyStructuredType`, which answers from the table (`hasPropertiesOfStructuredType` doesn't instantiate anything) and otherwise does what both call sites did before

on our 38k-file program (37,942 files, 0 errors), tsgo built from main + #64475 + #64526 + the tuple PR, median of 3, one process at a time:

| | symbols | heap after check | peak footprint |
|---|---|---|---|
| single threaded, before | 13,297,217 | 10.47 GB | 13.12 GB |
| single threaded, after | 13,082,184 (−1.6%) | 10.41 GB (−0.5%) | 12.92 GB (−1.5%) |
| 4 checkers, before | 19,553,613 | 15.41 GB | 19.94 GB |
| 4 checkers, after | 19,241,821 (−1.6%) | 15.32 GB (−0.6%) | 19.12 GB (noise) |

types (9,629,448 / 16,185,197) and instantiations (44,816,393 / 89,863,935) don't change. heap is `Memory used` from `--extendedDiagnostics`, peak footprint is from `/usr/bin/time -l` and spreads by ~1.5 GB between runs with 4 checkers. check time is within noise

per path (counted in an instrumented port of the checker that gives the same symbol, type and instantiation counts as this patch): 30,572 empty object queries single threaded are answered by a table that would otherwise have been resolved in full, ~10K of the lazy tables that were resolved in full were resolved for `isEmptyObjectType` alone

same diagnostics on every run. checker and testrunner tests pass at this commit, `go test ./...`, `TS_TEST_PROGRAM_SINGLE_THREADED=false` and `-race` on the top of the stack (except `TestFSEventsWatchFileDifferentCasing`, a macOS file watcher test that times out under load on main too and passes on its own), lint and format are clean. added `lazyMembersEmptyObjectType.ts` (subtype reduction of array literals that mix primitives with empty and non-empty generic interfaces and classes, assignability to empty interfaces, spreads) with baselines generated on unmodified main, so it pins that the output didn't change

used claude code to help write this, ive reviewed it
