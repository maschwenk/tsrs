# perf-checker-algorithms: where the checker does more work than it needs to (round 4)

Rounds 1-3 (notes/cpu-checker.md, perf-checker-cpu2.md, perf-checker-cpu3.md) made the port execute Go's algorithm
faster. This round may change the algorithm, as long as diagnostics, `.types`/`.symbols` baselines and the TS2589 /
TS2859 behaviour stay identical. Step 0 was to find out where the work goes by source-level pattern rather than by
function; step 1 was exact shortcuts for the patterns that matter.

Corpora: the 38k-file codebase ("big"), vscode (`src`, 371 errors), webpack (840 errors), mui-docs and xstate-main
(`bench/projects.json` checkouts), one checker unless noted, 18-core Apple Silicon, shared machine (load 4-25).

## Step 0: the work census

`TSRS_WORK_CENSUS=<out.md>` (branch `perf/algo-census`, not for merging) wraps the checker's pattern entry points in
spans and keys them by source identity: conditional types by root alias (with the distribution fan-out and how many
constituents came out `never`), mapped types by declaration and key count, generic calls by callee (and repeats of the
same signature + argument types + mode + contextual type), relations by target symbol and relation kind (cache hit
rates, pairs seen under several relations, and for union targets how the constituent loop exits), flow walks by
function (steps per walk, sampled re-walks), union construction / removeSubtypes / indexed access / keyof by size.

- A span's inclusive time counts for its key only when no span with the same key encloses it ("key incl"), and for
  its category only when no span of that category encloses it; self time excludes every nested span. Categories
  overlap (a relation inside a conditional inside a call), so the shares below do not add up to 100%.
- Spans read `Instant` (~12 ns here). `thread_cpu_seconds` costs ~105 ns a read, too much for 28.5M spans; with one
  checker the thread's wall time and CPU time agree (`TSRS_ASSIGNMENT_STATS=times` checks that per run). Each span's
  own bookkeeping (map updates, key building) is measured and charged to the enclosing span; the rest of a span's cost
  is calibrated at startup and subtracted. With the census on, big retires ~18% more instructions; the census-off
  build retires the same instructions as main (276.8 G).
- Shares are of the census's own check time, so they are biased upward a little for patterns made of many short spans
  (unions, relations, indexed access). Rankings and counts are what to read.

### Top patterns

Share = key-inclusive census time / check time; counts are exact.

| # | pattern | big | vscode | webpack | mui | xstate | count (big) | shape |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | relation to a union target (`typeRelatedToSomeType`) | 8.9% | 1.5% | - | 16.2% | - | 1.2M calls | source x target constituents; see "Row 1" |
| 2 | structural relation to zod class types (`ZodType`, `refine`) | 10.4% / 7.1% | 0.3% | - | - | - | 18.8k | whole generic class compared member by member |
| 3 | the same generic call re-inferred at another call site (same signature, argument type ids, mode, contextual type) | ~8% (309k of 599k `inferTypeArguments`) | ~8% (155k/273k) | 2.4% | 8.5% | 3.7% | 309k | M call sites x one key (vitest `expect`/`spyOn`, ORM `ref`, `DisposableStore.add`) |
| 4 | conditional types over a discriminated union (`Extract`, `Exclude`, hand-written) | 2.6% (`Extract`, 433k instantiation misses, 93% `never`) + 4.7% two private aliases built on it | <0.1% | 0.2% | 1.0% (`Exclude`) | <0.1% | 433k | N constituents x M discriminant values |
| 5 | mapped types over large key sets | 4.2% resolve, 9.3% property types | 0.3% | 0.4% | `Partial` over a 1,276-key intersection: 6.9% for 2 resolutions; 1,196 x 257-512 keys 5.1% | 2.9% | 154k | keys x constituents of the modifiers type |
| 6 | flow analysis | 3.9% | 16.1% | 19.0% | 2.0% | 1.8% | 3.5M walks, 21.9M steps | 63-67% of steps revisit a (reference, flow node) already walked for that reference |
| 7 | union construction | 9.3% (self 6.8%) | 4.2% | 9.0% | 12.2% | 5.2% | 4.4M calls | mostly 2-4 inputs; `removeSubtypes` 1-5% |
| 8 | indexed access | 12.5% (self 3.3%) | 1.1% | 1.8% | 4.8% | 6.2% | 3.55M | mostly non-union objects |
| 9 | `keyof` | 3.8% | 0.2% | 1.2% | 1.9% | 1.5% | 934k | |
| 10 | type of `export default <expression>` (candidate B) | 0.05% | 0 | - | 0 | 0 | 2,495 | with one checker the work only moves; see "Candidate B" |

Other measurements: relation caches hit 48% (big, assignable) to 59% (mui); 1.1% (big) to 8.2% (vscode) of
structurally compared (source, target) pairs are compared under more than one relation; flow walks are short (most
1-32 steps, the longest 2,540).

### Category totals (incl% = outermost span of the category, self% = minus nested spans)

| category | big calls / incl% / self% | vscode | webpack | mui | xstate |
| --- | --- | --- | --- | --- | --- |
| conditional instantiation (miss path, incl. distribution) | 1,908,402 / 23.0 / 1.6 | 96,065 / 3.2 / 0.2 | 12,352 / 3.7 / 0.2 | 34,167 / 13.5 / 0.4 | 26,542 / 12.1 / 0.9 |
| getConditionalType | 2,441,916 / 22.5 / 10.6 | 120,414 / 3.2 / 1.8 | 18,945 / 3.6 / 1.8 | 212,492 / 13.1 / 3.5 | 28,819 / 11.9 / 6.9 |
| resolveMappedTypeMembers | 153,702 / 4.2 / 2.2 | 8,407 / 0.3 / 0.3 | 2,281 / 0.4 / 0.3 | 5,369 / 14.7 / 13.8 | 7,128 / 2.9 / 2.1 |
| getTypeOfMappedSymbol | 740,217 / 9.3 / 0.7 | 49,006 / 0.4 / 0.1 | 8,207 / 0.3 / 0.1 | 144,714 / 5.8 / 0.7 | 10,828 / 3.3 / 0.5 |
| resolveCall | 1,219,008 / 66.7 / 4.4 | 1,106,390 / 55.0 / 7.0 | 55,971 / 24.4 / 2.9 | 50,253 / 76.5 / 4.3 | 22,868 / 76.2 / 8.3 |
| inferTypeArguments | 599,449 / 41.0 / 9.5 | 273,313 / 23.5 / 9.2 | 7,585 / 8.6 / 2.5 | 15,932 / 34.8 / 3.4 | 16,351 / 43.3 / 11.2 |
| checkExpressionWithContextualType | 2,037,540 / 42.0 / 14.7 | 1,935,724 / 42.8 / 20.7 | 84,291 / 18.5 / 9.0 | 121,913 / 41.5 / 16.9 | 44,441 / 40.2 / 21.8 |
| structuredTypeRelatedTo | 6,396,710 / 33.9 / 23.6 | 1,529,650 / 11.6 / 10.3 | 228,382 / 24.9 / 21.3 | 625,068 / 48.1 / 28.6 | 157,026 / 26.8 / 23.4 |
| getFlowTypeOfReference | 3,520,262 / 3.9 / 3.5 | 3,759,552 / 16.1 / 13.5 | 289,014 / 19.0 / 16.6 | 266,849 / 2.0 / 1.4 | 59,755 / 1.8 / 1.6 |
| getUnionType (worker) | 4,400,127 / 9.3 / 6.8 | 1,537,585 / 4.2 / 2.3 | 206,233 / 9.0 / 4.0 | 546,882 / 12.2 / 9.0 | 163,642 / 5.2 / 4.2 |
| removeSubtypes | 104,276 / 2.7 / 0.8 | 115,148 / 1.9 / 1.4 | 11,692 / 5.0 / 2.3 | 12,572 / 3.8 / 0.8 | 1,784 / 1.0 / 0.5 |
| getIndexedAccessType | 3,553,374 / 12.5 / 3.3 | 529,874 / 1.1 / 0.9 | 74,228 / 1.8 / 1.5 | 213,092 / 4.8 / 1.6 | 166,369 / 6.2 / 4.7 |
| getIndexType (keyof) | 934,058 / 3.8 / 1.9 | 85,370 / 0.2 / 0.1 | 18,782 / 1.2 / 0.8 | 27,867 / 1.9 / 0.5 | 22,630 / 1.5 / 0.7 |
| export assignment type | 2,495 / 0.05 / 0.01 | 5 / 0 / 0 | - | 372 / 0 / 0 | 3 / 0 / 0 |
| variable initializer type | 466,198 / 36.9 / 3.5 | 394,661 / 15.8 / 4.9 | 37,988 / 11.0 / 4.5 | 22,159 / 32.0 / 0.8 | 9,247 / 54.9 / 2.4 |

## What the N x M conditional work actually looks like

The brief's hypothesis was that `Extract<U, { kind: K }>` over a large union is evaluated per constituent in the
distribution of `getConditionalTypeInstantiation`. In big that path runs only 2,787 times for `Extract`. The 433k
evaluations come from a different path with the same shape:

1. `Extract<Every, { service: S; name: N }>` appears inside a generic alias, so it is resolved once with `S` and `N`
   still generic. The check type is a 590-member union, so it distributes right there; each constituent's extends
   type is generic, so each comes back as a deferred conditional type. The result is a union of 590 deferred
   conditional types.
2. Each of the 294 keys of a mapped type instantiates that union. `instantiateType` of a union instantiates every
   constituent; each is a deferred conditional type, so `getConditionalTypeInstantiation` misses its cache (key: the
   constituent and the now concrete `{ service: 'x'; name: 'y' }`) and `getConditionalType` relates one constituent to
   the extends type, which fails on the discriminant.

A mapped type over keys with `Extract<Ev, { kind: K }>` in its template (the brief's synthetic benchmark) has exactly
this shape too. So a shortcut keyed on the distribution loop would not see the real cases; the per-constituent
relation check inside `getConditionalType` is where both paths meet.

## A1: the definitely-false test without the relater (`relater_prefilter.rs`)

In a distributive conditional type, `getConditionalType` asks `isTypeAssignableTo(permissive(check), permissive(extends))`
to detect a definitely false extends check. For an object (or intersection) check type and a non-generic object
extends type that is not a type reference, the relater fails in `propertiesRelatedTo` at the first property whose
comparison fails; when that property's two types are not structured (literal, enum literal, unique symbol, primitive,
`null`, `undefined`, `never`), the comparison is `source == target || isSimpleTypeRelatedTo(source, target)` after
fresh-literal normalization, and records nothing.

The prefilter walks the relater's path for exactly that shape and calls the same functions in the same order:
`isSimpleTypeRelatedTo`, the relation-cache probe of `isTypeRelatedTo`, `getNormalizedType` of both sides, the weak-type
check, `getRelationKey` and the cache probe of `recursiveTypeRelatedTo` (with its maybe-key checks), for an intersection
source the `someTypeRelatedToType` loop over its constituents (each the same walk with `IntersectionStateSource`), the
alias-variance and tuple tests, `getApparentType`, `getUnmatchedProperty`, `getPropertiesOfType(target)`, per property
`getPropertyOfType`, the modifier test, `getNonMissingTypeOfSymbol` of both properties and `addOptionality`, and for
an intersection source `getEffectiveConstraintOfIntersection` at the end. Every side effect the relater has up to the
failing property (synthetic property creation, lazy member and property type resolution, instantiations) therefore
happens here, in the same order. It stops before anything the relater does that it does not reproduce:

| condition | why |
| --- | --- |
| the conditional is distributive | where the N x M work is; keeps the attempt rate low elsewhere (2M early exits, 0.46M proofs, 0.14M late exits on big) |
| check type is an object or intersection, extends type an object that is not a reference, mapped, reverse-mapped or instantiation-expression type | references to the same generic would be related by variances; mapped targets take other arms of `structuredTypeRelatedTo` |
| `isSimpleTypeRelatedTo` false, no cached success, normalization changes nothing | otherwise the relater answers differently from here on |
| not a fresh object literal source; target not weak | excess-property and common-property checks are not reproduced |
| no alias shared by source and target with type arguments; no single-element generic tuples; source not a type variable, index, conditional, template literal or string mapping | alias variance probing and the other arms of `structuredTypeRelatedToWorker` |
| target is not an object literal type | `propertiesRelatedTo` would check excess source properties |
| no private or protected property on the way | `isValidOverrideOf` and the declaration comparison are not reproduced |
| a property pair with a structured or instantiable type before the failing one | that comparison is a nested relation |
| every property relates | the relater goes on to signatures and index signatures |
| `getEffectiveConstraintOfIntersection` returns a constraint | the relater compares the constraint |
| the relation budget `(16M - size) / 8` is at most 256 | the emulated recursion must not run out where the relater would |

When it proves the relation false it records the relation-cache entries the relater records (`Failed` plus the
reliability flags that cached sub-results propagate, for the pair and, for an intersection source, for each
constituent paired with the target under `IntersectionStateSource`), so later cache probes and the complexity budget
of later checks (`relationCount = (16M - relation.size()) / 8`, TS2859) see the same map. When it gives up, nothing is
recorded and the relater runs; the calls made so far are a prefix of the relater's own calls, repeated as cache hits.

What it saves is the relater's machinery around the comparison (relater pooling, budgets, error-state snapshots,
maybe-key sets, recursion stacks, the dispatch of `structuredTypeRelatedToWorker`) and the nested `isRelatedTo` of each
property pair. It does not skip lookups, so the N x M evaluations stay N x M.

### Exactness evidence

- `TSRS_VERIFY_COND_PREFILTER=<file>` runs the relater after every decision and compares: the result must be false,
  the relation cache must hold exactly the predicted entries (and grow by exactly as many), and the instantiation,
  type and symbol counters must not move (the relater repeats only cache hits). 0 disagreements: big 454,657+
  decisions (also on the error-rich clone below), vscode 24,577+, mui 4,097+, webpack and xstate 64+ each, the whole
  conformance suite.
- Gates against origin/main (7d4f145): `tsrs-test run --suite all --baselines types,symbols` 13,458 error baselines
  (+2 codes, 2 fail) / 12,779 types / 12,779 symbols in the default mode and with `TSRS_LAZY_MEMBERS=0`, result trees
  identical; `--baselines js,jsmap,sourcemap` 13,392 / 149 / 156, trees identical; fourslash 4,066 pass / 63 fail,
  result trees identical; tsctests 374 pass / 32 fail / 0 crash, same lists; default_emit 6/6; `cargo check -D
  warnings`, `tools/lint/ratchet.py` and `tools/lint/source.py` clean.
- Diagnostics byte-identical with base on all five corpora at one and four checkers, and on an error-rich clone of big
  (the endpoint declaration's `describe` field renamed to `describeX`; 10,781 errors, four checkers). Types, symbols
  and instantiations identical in every run.
- `testdata/regressions/cond-prefilter-*` (expected output from tsgo-ref): the corner cases (numeric literal vs numeric
  enum member, string enums, unique symbols, `true`, `null`, `undefined`, `never`, optional on either side, weak
  targets, intersections including a branded primitive, `Exclude`, a primitive and a union-typed discriminant, index
  signatures, a discriminant that is not the first property, conditional chains, hand-written distributions, private
  and protected class members, generic aliases on both sides, a mapped type over the keys) with and without
  `strictNullChecks`; and the instantiation limit: the synthetic mapped type with 1,117 constituents stays at
  5,034,569 instantiations without TS2589, with 1,118 it reports TS2589 at the same node as tsgo-ref
  (`a.ts(1126,14)`); base, A1 and tsgo-ref agree on every count.

### Numbers

Synthetic (`Ev` = N object types discriminated by `kind`; "mapped" = `{ [K in Kind]: Extract<Ev, { kind: K }> }`
assigned to `Record<Kind, object>`; "explicit" = M aliases `Extract<Ev, { kind: 'kJ' }>` with a member access each),
instructions single-threaded, medians of 3; the marginal cost subtracts the same file without the Extract work:

| case | base G | A1 G | marginal base -> A1 | marginal ratio per doubling (base / A1) |
| --- | --- | --- | --- | --- |
| mapped, N = 250 | 1.508 | 1.462 | 0.448 -> 0.402 (-10.3%) | |
| mapped, N = 500 | 2.849 | 2.660 | 1.777 -> 1.588 (-10.6%) | 3.97 / 3.95 |
| mapped, N = 1000 | 8.164 | 7.401 | 7.071 -> 6.308 (-10.8%) | 3.98 / 3.97 |
| explicit, N = 500, M = 300 -> 600 | 1.948 -> 2.899 | 1.828 -> 2.665 | per 300 Extracts 0.952 -> 0.837 (-12.1%) | |

A constant factor on the N x M term, not a change of asymptote.

Corpora (medians of 3 interleaved rounds, load 11-21; paired per-round instruction deltas in the last column, which
alternate in sign by the order within a round, so their median is the figure to read):

| corpus | checkers | instr G base -> A1 | check s base -> A1 | peak GiB base -> A1 | paired instr median |
| --- | --- | --- | --- | --- | --- |
| big | 1 | 275.85 -> 275.63 | 14.97 -> 14.93 | 4.243 -> 4.247 | -0.11% (single-threaded, 4 rounds: -0.10%) |
| big | 4 | 370.82 -> 368.81 | 5.89 -> 5.94 | 5.691 -> 5.702 | -0.49% (-0.38 to -0.54%) |
| vscode | 1 | 116.15 -> 116.13 | 6.21 -> 6.27 | 2.002 -> 2.000 | -0.03% (st: -0.35%) |
| vscode | 4 | 120.74 -> 120.73 | 2.12 -> 2.12 | 2.261 -> 2.250 | +0.02% |
| webpack | 1 | 14.75 -> 14.75 | 0.668 -> 0.665 | 0.321 -> 0.321 | -0.02% (st: -0.56%) |
| webpack | 4 | 16.34 -> 16.34 | 0.228 -> 0.227 | 0.388 -> 0.388 | +0.08% |
| mui | 1 | 55.92 -> 55.38 | 2.30 -> 2.30 | 0.758 -> 0.761 | -0.95% (st: -0.20%) |
| mui | 4 | 76.92 -> 76.96 | 1.18 -> 1.18 | 0.983 -> 0.985 | +0.05% |
| xstate | 1 | 8.21 -> 7.91 | 0.350 -> 0.344 | 0.196 -> 0.197 | -3.65% (st: +0.07%) |
| xstate | 4 | 8.96 -> 8.98 | 0.136 -> 0.134 | 0.234 -> 0.235 | +0.18% |

Neutral on the corpora (every delta within the run-to-run spread, none above +0.2% in the median), -0.5% on big at
four checkers, where the N x M type is evaluated by several checkers.

### Why not the asymptotic version (A2)

Skipping whole constituents (a discriminant index over the union, evaluating only the matching ones) would make the
mapped synthetic linear. It cannot keep the counters exact without simulating every cache the skipped evaluations
touch: per constituent the slow path counts the instantiation of the deferred conditional type (a miss in the active
mapper's cache), of the constituent itself (`couldContainTypeVariables` is true for object literal types), of the
extends type's arguments (a miss for the first constituent only, a hit afterwards), of the check and extends type
parameters under a fresh mapper, the first permissive instantiation of each constituent (cached afterwards), and
whatever the relation resolves the first time; it also inserts one `getConditionalTypeInstantiation` cache entry per
constituent, which later lookups hit or miss. The charge per skipped constituent is therefore not a constant, and the
cache entries would have to be emulated through a side table to keep later hits. The instantiation count feeds TS2589
(5M per statement; the regression tests above sit at that boundary) and the relation-cache size feeds TS2859, so
"approximately" is not good enough. A2 is feasible for the distribution path alone (`getConditionalTypeInstantiation`
calls `getConditionalType` directly there, with no per-constituent cache entry) if the extends type's instantiation
is structurally simple, but that path is 0.6% of the conditional work on big and absent elsewhere.

## Candidate B: `export default <expression>` (known, deferred)

With one checker the census sees 2,495 export-assignment type requests costing 8 ms on big: the expression is checked
when an importer first asks for the default export's type instead of when its own file is checked, so the work moves
and is done once. With several checkers it is done again by every checker that reaches the file without owning it
(notes/perf-checker-scaling.md; the private measurement put one aggregator file at ~2 s cold). Another agent is
removing the per-checker duplication at its root (forked checkers sharing one warm base); if that lands, B's
multi-checker benefit is moot. Not pursued this round.

## Reproduce

```sh
git checkout perf/algo-census && cargo build --release -p tsrs_cli
cd <project> && TSRS_WORK_CENSUS=/tmp/census.md tsrs -p . --noEmit --incremental false --extendedDiagnostics --checkers 1
# A1 decisions cross-checked against the relater:
TSRS_VERIFY_COND_PREFILTER=/tmp/verify.log tsrs -p . --noEmit --incremental false --checkers 1; grep -c MISMATCH /tmp/verify.log
# A1 off (for A/B):
TSRS_COND_PREFILTER=0 tsrs -p . --noEmit --incremental false --extendedDiagnostics
# Regression tests: compare with expected.txt (tsgo-ref output)
cd testdata/regressions/cond-prefilter-limit-at && tsrs -p . --pretty false --singleThreaded | diff expected.txt -
```
