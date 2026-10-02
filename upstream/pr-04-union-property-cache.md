don't copy union and intersection properties into the augmented property cache

`getUnionOrIntersectionProperty` keeps two caches per union/intersection type, one with and one without the function property augment. when a lookup without the augment creates a property that isn't partial, it also copies it into the augmented cache. the copy is only ever read by a later augmented lookup of the same name, which can read it from the other cache instead. also, `ast.GetSymbolTable` allocates a cache map on the first lookup, even when nothing ever gets stored

this reads the non-augmented cache on an augmented miss (non-partial entries only, like the copy) and allocates a cache on its first store. lookups return the same symbol in any order:

- augmented property created first, non-augmented later: the augmented cache has it and is checked first. the copy didn't happen before either (it only went into an empty slot)
- non-augmented property created first: before, the augmented lookup found the copy, now it finds the same symbol in the other cache
- partial properties: never copied before, never read from the other cache now, so the augmented lookup still creates its own

nothing else reads these caches

on our 38k-file program (37,943 files, 0 errors), median of 3, one process at a time:

| | heap after check | allocs |
|---|---|---|
| main, single threaded | 14.03 GB → 13.84 GB (−1.4%) | 148.86M → 147.86M (−1.0M) |
| main, 4 checkers | 21.25 GB → 20.96 GB (−1.4%) | 246.89M → 245.02M (−1.9M) |
| main + #64475 + #64526 + 3 unopened follow ups, single threaded | 10.36 GB → 10.23 GB (−1.3%) | 149.24M → 148.42M (−0.8M) |
| main + #64475 + #64526 + 3 unopened follow ups, 4 checkers | 15.07 GB → 14.84 GB (−1.5%) | 248.70M → 247.09M (−1.6M) |

symbols, types and instantiations don't change (25,972,751 / 9,639,290 / 44,879,960 on main single threaded). heap is `Memory used` from `--extendedDiagnostics` (live heap after GC; spreads by under 0.01 GB between runs single threaded, up to 0.15 GB with 4 checkers), allocs is `Memory allocs`. check time is within noise

per path (counted in an instrumented port of the checker with the same change, on that stack): 2.37M fewer cache entries single threaded (3.39M with 4 checkers), and 70,673 augmented lookups are served from the other cache

same diagnostics on every run. `go test ./...` passes (except the macOS fsevents tests in `internal/fswatch`, which time out here under load and pass on a rerun), the testrunner also with `TS_TEST_PROGRAM_SINGLE_THREADED=false`, lint and format are clean. no new test, the change has no observable effect besides memory

used claude code to help write this, ive reviewed it
