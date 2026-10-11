# perf-source-keyed-properties: relating a small object to a large options type (2026-10-10)

`propertiesRelatedTo` walks every property of the target and looks each name up in the source. For a target with
hundreds of optional properties and a source with a handful (a style object against React's `CSSProperties`, ~800
optional properties; log data against an interface merged from hundreds of declarations into ~1,800 optional
properties), nearly every lookup misses and the walk does nothing else at those positions. A work census of a large
private monorepo put relations to two such types at ~1.8 s of checker time across 8 checkers, about 23k and 49k
relations.

## Change

`properties_related_to_rest` (crates/tsrs_checker/src/relater_2.rs) walks only the target positions at which
`get_property_of_type(source, name)` can return a property, in target order, with the same loop body, when the
target has at least 64 properties and at least 4 times as many as the source has members, and nothing is excluded or
numeric-only. For a source that is a resolved object type (its reduced apparent type is itself), the lookup finds one
of its members or, for a name `may_be_augment_member` lets through, a member of `Object` / `Function`
(`get_property_of_type_worker`). So the positions are those of the target properties named like a source member, plus
those whose name passes `may_be_augment_member`. Every other position is a lookup that misses, after which the full
walk does nothing; skipping it removes no relation, adds none and reorders none, so results, relation caches, type
creation order and the first reported failure are those of the full walk.

Per large target, `Checker::target_property_positions` keeps a `hashbrown::HashTable<u32>` of the target's property
positions, found by name, and the positions whose name passes `may_be_augment_member`. An entry is kept with the
property list it was built from (a members reset makes a new list). It is not invalidated by `members_reset_epoch`:
the augment positions only need to include every name `Object` / `Function` has, which comes from the lib and does
not change when the filter is rebuilt (an unbuilt filter answers yes), and on mui-docs rebuilding every table after
each reset cost 0.34 G instructions (0.8%). `get_property_of_type_worker` says that this walk relies on how it finds a
property.

Variants measured on mui-docs (single-threaded instructions; main 44.07 G): an `FxHashMap<&str, u32>` per target,
-2.5% but +2.6% peak memory; a slot -> position vector over the target's member table with a sorted name-hash
fallback, -2.5% and +0.3% peak, 60 more lines; a sorted (name hash, position) vector only, -1.3% (the binary search
per source member); a pool for the position buffer, no difference (not kept); the hash table, -2.6%.

Debug builds assert the claim on every use: every position whose lookup finds a property is walked.

## Evidence

Mac (Apple M5 Max), instructions retired from `/usr/bin/time -l`, single-threaded, medians of 3 interleaved runs
after a warm-up; `bench/count.py` is Linux-only.

| project | before | after | peak memory |
| --- | ---: | ---: | --- |
| mui-docs | 44.07 G | 42.94 G (-2.6%) | 777-780 -> 778-785 MB |
| xstate-main | 6.179 G | 6.172 G (-0.1%) | |
| vscode | 89.4-90.6 G | 89.4-90.4 G (no change; measured on an earlier variant) | |

A large private monorepo, 8 checkers (medians of 3): 1.3723 T -> 1.3559 T instructions (-1.2%), wall 17.96 ->
17.79 s, peak 17.35 -> 17.36 GB.

Output: byte-identical on all four, and on the conformance suite (13,458 / 15,197 error baselines, all 12,779 `.types`
and `.symbols` as before, also with a debug build, so the assertion ran on every use), fourslash (4,066 passed, as
before) and the debug build on the private monorepo. `testdata/regressions/source-keyed-optional-properties`
checks the first failure of non-fresh and class sources whose failing properties are listed in the opposite order to
the target's, the weak-type check, and a target property only `Object` supplies (`valueOf`), against tsgo
(TypeScript 7.1.0-dev.20260929.1).

## Not done

Relations from a fresh object literal to a union of such types go through `typeRelatedToSomeType` first; they reach
this walk per constituent. The same walk could serve `get_unmatched_property` with `require_optional_properties`
(subtype relations of non-literal sources), which still looks up every optional target property; not measured.
