**Title:** instantiate conditional types without a combined mapper for the cache lookup

**Base:** main. Independent of #64475 / #64526 (applies to main as is, and is the last commit of the stack on top of them).

---

`instantiateTypeWorker` builds `combineTypeMappers(t.mapper, m)` for every conditional type it instantiates. `getConditionalTypeInstantiation` only uses that mapper to map the outer type parameters to the type arguments for the cache key. on a miss it builds `newTypeMapper(outerTypeParameters, typeArguments)` from those, so the composite mapper is garbage right after the lookup either way

this adds `getConditionalTypeInstantiationEx(t, m1, mapper, ...)`, which maps the type parameters the way `CompositeTypeMapper.Map` does (`m1` first, then instantiate the result with `mapper` if `m1` changed it, else `mapper` alone). `getConditionalTypeInstantiation` calls it with `m1 == nil`, so the other callers don't change. (not `mapTypeWithCompositeMapper`: that goes through `getMappedType`, which first replaces a distributed type parameter with its constraint, and `CompositeTypeMapper.Map` doesn't)

on our 38k-file program (37,942 files, 0 errors), median of 3, one process at a time:

| | allocs | heap after check |
|---|---|---|
| main (+ the cache PR), single threaded | 147.91M → 143.56M (−4.35M, −2.9%) | same |
| main (+ the cache PR), 4 checkers | 247.32M → 238.50M (−8.8M, −3.6%) | same |
| with #64475 + #64526 and the other follow ups, single threaded | 148.47M → 144.13M (−4.34M, −2.9%) | same |
| with #64475 + #64526 and the other follow ups, 4 checkers | 249.48M → 240.56M (−8.9M, −3.6%) | same |

allocs is `Memory allocs` from `--extendedDiagnostics`. the composite mappers are garbage right away, so this saves allocations and GC work, not retained memory: heap after check, symbols, types and instantiations don't change. an instrumented port of the checker counts 4,340,555 composite mappers avoided single threaded on the stack, and go's malloc count goes down by 4,340,242. check time is within noise on this machine

same diagnostics on every run. checker and testrunner tests pass on main and on the stack (also with `TS_TEST_PROGRAM_SINGLE_THREADED=false`), the rest of `go test ./...` and `-race` on the top of the stack (except `TestFSEventsWatchFileDifferentCasing`, a macOS file watcher test that times out under load on main too and passes on its own), lint and format are clean. no new test, the change has no observable effect besides the allocations

used claude code to help write this, ive reviewed it
