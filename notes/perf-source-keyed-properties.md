# perf-source-keyed-properties: walking a large target by the source's keys (2026-10-10, rejected)

Verdict: measured and rejected. Up to -5.5% single-threaded instructions on React applications, no wall-time change
on any bench project. Closed as #305.

## Idea

`propertiesRelatedTo` walks every property of the target and looks each name up in the source. Relating a small
object to a type with hundreds of optional properties (a style object against React's `CSSProperties`, ~800
optional properties; log data against an interface merged from hundreds of declarations) makes nearly every lookup a
miss. For a resolved object source, `get_property_of_type(source, name)` can only find one of its members or, for a
name `may_be_augment_member` lets through, a member of `Object` / `Function` (`get_property_of_type_worker`). So the
walk can visit only the target positions named like a source member plus the augment-named ones, in target order,
with the same loop body: the same properties are related in the same order, so results, caches, type creation order
and the first reported failure are unchanged (exact by construction; a debug assertion checked it on every use).

Implementation (branch `perf-source-keyed-props`, 116 lines): for a target of at least 64 properties and at least 4x
the source's members, a per-checker cache per target of a `hashbrown::HashTable<u32>` of property positions by name
and the augment-named positions, kept with the property list it was built from.

## Measurements

pr-verify bench (Linux, `main@c5ae5ef7`), single-threaded instructions and wall (median over 1/4/16/32 checkers):

| project | instructions | wall | 1-checker wall |
| --- | ---: | ---: | ---: |
| supabase-studio | -5.460% | -0.3% | -2.0% |
| mui-docs | -2.852% | +0.2% | -0.1% |
| cal-diy | -1.867% | +0.9% | +0.8% |
| formbricks-web | -1.775% | -0.3% | +0.2% |
| storybook | -0.941% | +0.5% | |
| others (12) | -0.04% to +0.25% | -1.1% to +1.2% | |

Diagnostics identical in all 102 cells. Peak RSS within noise. A large private monorepo: -1.2% instructions at 8
checkers, wall 17.96 -> 17.79 s.

Why no wall gain: the skipped work was hash probes into the source's small member table, which stays in L1; they
retire at high IPC and cost few cycles. The walk replaces hundreds of them with a few probes into the target's large
table. Instructions fall, cycles barely move; wall is set by the memory-bound work around it.

Variants measured on mui-docs (Mac, `main` 44.07 G): an `FxHashMap<&str, u32>` per target -2.5% (+2.6% peak RSS); a
slot vector over the target's member table with a sorted name-hash fallback -2.5% (+0.3% peak); a sorted (hash,
position) vector -1.3%; the hash table -2.6%; a pooled position buffer made no difference; invalidating the cache on
`members_reset_epoch` cost 0.8% (resets are frequent there; the augment positions stay a superset without it).

Revisit only with a cycle-level (not instruction) measurement that shows a gain, e.g. if the surrounding relation
work gets cheaper and these probes become a larger share of cycles.
