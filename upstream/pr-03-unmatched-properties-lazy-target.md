**Title:** find unmatched properties of a lazy target without instantiating its members

**Base:** stacked on #64475 (needs its lazy member tables) and on pr-01 / pr-02 (the numbers below are on top of them; the code applies without them).

---

stacked on:

- https://github.com/microsoft/TypeScript/pull/64475

`getUnmatchedPropertiesWorker(source, target, ...)` walks `getPropertiesOfType(target)` and asks `getPropertyOfType(source, name) != nil` for each required property. that instantiates members on both sides that nobody uses afterwards: the target gets resolved in full, and a source with a lazy member table instantiates every member that's looked up, even though only whether it exists matters. this is the relater's path for failing comparisons (union constituents, overload candidates), where all of that is dropped right after

when the target has a lazy member table this now:

- walks the properties `getPropertiesOfType(target)` would return, in the same order, but with declared members standing in for the instantiations the table hasn't created yet. `getPropertiesOfTypeLazily` builds that list the way `resolveLazyMembers` builds the member table (declared members, then `addInheritedMembers` over the base types' properties, then `getNamedMembers`), so the names, flags, declarations and order are the same. the list is kept on the table so base types share it
- without `matchDiscriminantProperties`, asks the source only whether it has the property (`hasPropertyOfType`, which returns a declared member as is instead of instantiating it; it only reads flags)
- looks up the real target property only where it's returned or reported, or where its type is compared (discriminants)

so the visited order, the reported properties and the source lookups are the same as before, only the member instantiations differ

on our 38k-file program (37,942 files, 0 errors), tsgo built from main + #64475 + #64526 + the two previous PRs, median of 3, one process at a time:

| | symbols | heap after check | allocs |
|---|---|---|---|
| single threaded, before | 13,082,184 | 10.41 GB | 149.04M |
| single threaded, after | 12,810,418 (−2.1%) | 10.37 GB (−0.4%) | 149.29M (+0.17%) |
| 4 checkers, before | 19,241,821 | 15.32 GB | 250.33M |
| 4 checkers, after | 18,691,689 (−2.9%) | 15.22 GB (−0.7%) | 250.83M (+0.2%) |

types (9,629,448 / 16,185,197) and instantiations (44,816,393 / 89,863,935) don't change. heap is `Memory used` from `--extendedDiagnostics`, allocs is `Memory allocs`. the extra allocations are the property lists (a member table and a slice per lazy table that's walked), in exchange for 272K / 550K fewer retained symbols. check time is within noise

per path (counted in an instrumented port of the checker that gives the same symbol, type and instantiation counts as this patch): before, targets with a lazy table were resolved in full 34K times single threaded (453K symbols), and source lookups instantiated 652K members, 61% of which never had their type resolved. after, 621,288 / 1,140,871 existence-only source lookups, 152,771 / 340,444 of them answered by a declared member that was never instantiated

same diagnostics on every run. checker and testrunner tests pass at this commit, `go test ./...`, `TS_TEST_PROGRAM_SINGLE_THREADED=false` and `-race` on the top of the stack (except `TestFSEventsWatchFileDifferentCasing`, a macOS file watcher test that times out under load on main too and passes on its own), lint and format are clean. added `lazyMembersUnmatchedProperties.ts` (missing property lists across declared and inherited members, the "and N more" form, private members, `implements`, discriminated unions in assignments and in inference) with baselines generated on unmodified main, so it pins that the output and the order of the reported properties didn't change

used claude code to help write this, ive reviewed it
