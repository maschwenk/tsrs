instantiate conditional types without a combined mapper for the cache lookup

`instantiateTypeWorker` builds `combineTypeMappers(t.mapper, m)` for every conditional type it instantiates. `getConditionalTypeInstantiation` only uses that mapper to map the outer type parameters to the type arguments for the cache key, and on a miss it builds `newTypeMapper(outerTypeParameters, typeArguments)` from those. so the composite mapper is garbage right after the lookup either way

this adds `getConditionalTypeInstantiationEx(t, m1, mapper, ...)`, which maps the type parameters the way `CompositeTypeMapper.Map` does (`m1` first, then instantiate the result with `mapper` if `m1` changed it, else `mapper` alone). `getConditionalTypeInstantiation` calls it with `m1 == nil`, so the other callers don't change. (not `mapTypeWithCompositeMapper`: that goes through `getMappedType`, which first replaces a distributed type parameter with its constraint, and `CompositeTypeMapper.Map` doesn't)

on our 38k-file program (37,943 files, 0 errors), median of 3, one process at a time:

| | allocs | heap after check |
|---|---|---|
| main, single threaded | 148.86M → 144.51M (−4.35M, −2.9%) | same |
| main, 4 checkers | 246.89M → 238.12M (−8.8M, −3.6%) | same |
| main + #64475 + #64526 + 3 unopened follow ups, single threaded | 148.42M → 144.08M (−4.34M, −2.9%) | same |
| main + #64475 + #64526 + 3 unopened follow ups, 4 checkers | 247.09M → 238.41M (−8.7M, −3.5%) | same |

allocs is `Memory allocs` from `--extendedDiagnostics`. the composite mappers are garbage right away, so this saves allocations and GC work, not retained memory: heap after check, symbols, types and instantiations don't change. an instrumented port of the checker counts 4,340,555 composite mappers avoided single threaded on that stack, and go's malloc count there goes down by 4,340,793. check time is within noise

same diagnostics on every run. `go test ./...` passes (except the macOS fsevents tests in `internal/fswatch`, which time out here under load and pass on a rerun), the testrunner also with `TS_TEST_PROGRAM_SINGLE_THREADED=false`, lint and format are clean. no new test, the change has no observable effect besides allocations

used claude code to help write this, ive reviewed it
