**Title:** give tuple references lazy member tables

**Base:** stacked on #64475 (needs its lazy member tables). In the patch stack it sits on #64475 + #64526 rebased onto main.

---

stacked on:

- https://github.com/microsoft/TypeScript/pull/64475

#64475 gives instantiated class and interface references a lazy member table but leaves tuple references out. a tuple reference that gets resolved in full instantiates its element properties and `length`, and resolves its base type `Array<E1 | E2 | ...>` with `this` = the tuple, also in full. the `this` argument is different for every tuple, so that's a new `Array` reference with ~40 instantiated members per tuple, nearly all of them never used

tuples are mostly asked for signatures, index infos and single members, which the lazy table already answers. their `Array<..., this>` base is an interface reference, so it gets its own lazy table and only the members that are looked up (`map`, `length`, ...) get instantiated

the change is the condition in `getReadyLazyMemberTableWorker`: `ObjectFlagsTuple` targets are allowed too. nothing else is tuple specific: tuple targets have declared members (`"0"`, `"1"`, ..., `length`) and base types like interfaces, and every consumer already goes through the lazy-aware accessors from #64475. (the old `source.objectFlags&ObjectFlagsTuple != 0` test was redundant with the `ClassOrInterface` one, tuple targets aren't classes or interfaces)

on our 38k-file program (37,942 files, 0 errors), tsgo built from main + #64475 + #64526, median of 3, one process at a time:

| | symbols | types | instantiations | heap after check | peak footprint |
|---|---|---|---|---|---|
| single threaded, before | 15,330,783 | 9,629,448 | 44,816,393 | 11.01 GB | 13.88 GB |
| single threaded, after | 13,297,217 (−13.3%) | same | same | 10.47 GB (−4.9%) | 13.12 GB (−5.5%) |
| 4 checkers, before | 22,811,135 | 16,185,197 | 89,863,935 | 16.32 GB | 19.50 GB |
| 4 checkers, after | 19,553,613 (−14.3%) | same | same | 15.41 GB (−5.5%) | 19.94 GB (noise, see below) |

heap is `Memory used` from `--extendedDiagnostics` (live heap after GC), peak footprint is from `/usr/bin/time -l`. the 4-checker peak spreads by ~1.5 GB between runs of the same build (GC timing), the heap number by less than 0.02 GB. check time is within noise (the machine had other load)

per path, single threaded / 4 checkers (counted in an instrumented port of the checker that gives the same symbol, type and instantiation counts as this patch): 66,727 / 100,248 tuples get a lazy table, 8,402 / 11,824 of them (13% / 12%) are later resolved in full anyway. the lazy tables get 2.82M more declared members, of which 0.66M are ever instantiated, so 77% of them are never created

same diagnostics on every run. checker and testrunner tests pass at this commit, `go test ./...`, `TS_TEST_PROGRAM_SINGLE_THREADED=false` and `-race` on the top of the stack (except `TestFSEventsWatchFileDifferentCasing`, a macOS file watcher test that times out under load on main too and passes on its own), lint and format are clean. added `tupleLazyMembers.ts` (element access, `length`, array methods through the `this`-typed base, optional/rest/named/readonly tuples, weak type and assignability checks) with baselines generated on unmodified main, so it pins that the output didn't change

used claude code to help write this, ive reviewed it
