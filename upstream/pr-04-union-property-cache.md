**Title:** don't copy union and intersection properties into the augmented property cache

**Base:** main. Independent of #64475 / #64526 (applies to main as is, and is the 4th commit of the stack on top of them).

---

`getUnionOrIntersectionProperty` keeps two caches per union/intersection type, one with and one without the function property augment. when a lookup without the augment creates a property that isn't partial, it's also copied into the augmented cache. that copy only matters to a later augmented lookup of the same name, and that lookup can read it from the other cache instead. also, `ast.GetSymbolTable` allocates the cache map on every first lookup, even when nothing ever gets stored

this reads the non-augmented cache on an augmented miss (non-partial entries only, like the copy) and allocates a cache on its first store. the result is the same in any order of lookups:

- augmented property created first, non-augmented later: the augmented cache has it and is checked first, the copy never happened before either (it was only made into an empty slot)
- non-augmented property created first: before, the augmented lookup found the copy; now it finds the same symbol in the other cache
- partial properties: never copied before, never read from the other cache now, so the augmented lookup still creates its own

nothing else reads these caches

on our 38k-file program (37,942 files, 0 errors), median of 3, one process at a time:

| | heap after check | allocs |
|---|---|---|
| main, single threaded | 14.04 GB → 13.85 GB (−1.4%) | 148.91M → 147.91M (−1.0M) |
| main, 4 checkers | 21.54 GB → 21.26 GB (−1.3%) | 248.76M → 247.32M (−1.4M) |
| with #64475 + #64526 and the other follow ups, single threaded | 10.37 GB → 10.25 GB (−1.2%) | 149.29M → 148.47M (−0.8M) |
| with #64475 + #64526 and the other follow ups, 4 checkers | 15.22 GB → 15.00 GB (−1.5%) | 250.83M → 249.48M (−1.3M) |

symbols, types and instantiations don't change (25,972,740 / 9,639,290 / 44,879,960 on main single threaded). heap is `Memory used` from `--extendedDiagnostics` (live heap after GC), allocs is `Memory allocs`. check time is within noise

per path (counted in an instrumented port of the checker with the same change, on top of the lazy member PRs): 2.37M cache entries fewer single threaded (3.39M with 4 checkers), and 70,673 augmented lookups are served from the other cache

same diagnostics on every run. checker and testrunner tests pass on main and on the stack (also with `TS_TEST_PROGRAM_SINGLE_THREADED=false`), the rest of `go test ./...` and `-race` on the top of the stack (except `TestFSEventsWatchFileDifferentCasing`, a macOS file watcher test that times out under load on main too and passes on its own), lint and format are clean. no new test, the change has no observable effect besides the memory

used claude code to help write this, ive reviewed it
