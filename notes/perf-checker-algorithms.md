# perf-checker-algorithms: where the checker does more work than it needs to (round 4)

Rounds 1-3 (notes/cpu-checker.md, perf-checker-cpu2.md, perf-checker-cpu3.md) made the port execute Go's algorithm
faster. This round may change the algorithm, as long as diagnostics, `.types`/`.symbols` baselines and the TS2589 /
TS2859 behaviour stay identical. Step 0 was to find out where the work goes by source-level pattern rather than by
function; step 1 was exact shortcuts for the patterns that matter.

Corpora: the 38k-file codebase ("big"), vscode (`src`, 371 errors), webpack (840 errors), mui-docs and xstate-main
(`bench/projects.json` checkouts), one checker unless noted, 18-core Apple Silicon, shared machine (load 4-25).

## Step 0: the work census

The census (`crates/tsrs_checker/src/workcensus.rs`, compiled in with `--features work-census`, output with
`TSRS_WORK_CENSUS=<out.md>`; docs/DEBUGGING.md) wraps the checker's pattern entry points in spans and keys them by
source identity: conditional types by root alias (with the distribution fan-out and how many
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
  is calibrated at startup and subtracted. With the census on, big retires ~18% more instructions. Compiled in but
  switched off, the hooks cost 0.3-0.5% instructions (single-threaded, paired medians: big +0.48%, vscode +0.43%,
  mui +0.58%), so they are behind the cargo feature; without it the wrappers inline away and the binary is within
  noise of main (paired medians big -0.17%, vscode +0.54%, mui -0.63%, alternating signs; `__text` +3 KB).
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

## A1 (measured and rejected): the definitely-false test without the relater

Draft #88, closed. In a distributive conditional type, `getConditionalType` asks
`isTypeAssignableTo(permissive(check), permissive(extends))` to detect a definitely false extends check. For an object
(or intersection) check type and a non-generic object extends type that is not a type reference, the relater fails in
`propertiesRelatedTo` at the first property whose comparison fails; when that property's two types are not
structured (literal, enum literal, unique symbol, primitive, `null`, `undefined`, `never`), the comparison is
`source == target || isSimpleTypeRelatedTo(source, target)` and records nothing.

The prototype walked the relater's path for that shape and called the same functions in the same order (normalization,
the weak-type check, the relation-cache probes and maybe-key checks, the `someTypeRelatedToType` loop for intersection
sources, `getApparentType`, `getUnmatchedProperty`, the property lookups and type resolutions,
`getEffectiveConstraintOfIntersection`), stopped before anything it did not reproduce (variance probing, excess or
weak-type checks, private members, a structured property type, every property relating), and recorded the relation
cache entries the relater records. A cross-check mode (`TSRS_VERIFY_COND_PREFILTER`) ran the relater after every
decision: 0 disagreements in result, cache entries, or instantiation / type / symbol counters over the conformance
suite and the five corpora (454,657 decisions on big). All gates passed.

The saving was the relater's machinery around the comparison; the lookups stay, so the N x M term only shrank by a
constant: -10.6% of it on the synthetic mapped type with N = 500 (2.849 -> 2.660 G instructions) and -10.8% with
N = 1000 (8.164 -> 7.401 G), quadratic before and after (3.97x / 3.98x per doubling); explicit `Extract`s -12%.
Corpora were neutral (paired medians within +-0.6% on every corpus single-threaded); big -0.49% instructions at four
checkers. **Rejected: exact, but 0.5% on one corpus does not pay for a second implementation of part of the relater
that has to replay its side effects.** Code: `perf/algo-cond-prefilter` (f49df2b).

A timing experiment that also skipped the lookups (deciding from the discriminant property alone; not cache-identical,
40 extra instantiations on big) took -20% of the synthetic N x M term and -1.1% instructions on big single-threaded.
That is roughly what an upstream version could get, since the Go checker does not promise identical relation-cache
contents (`upstream/algo-01-conditional-discriminant-false-test.md`).

## A2 (measured and rejected): skipping non-matching constituents

A key index over the union, evaluating only the matching constituents, would make the `Extract`-per-key mapped type
linear. **Rejected: it moves the instantiation-budget boundary.** Per skipped constituent the slow path counts the
instantiation of the deferred conditional type (a miss in the active mapper's cache), of the constituent itself
(`couldContainTypeVariables` is true for object literal types), of the extends type's arguments (a miss for the first
constituent only, a hit afterwards), of the check and extends type parameters under a fresh mapper, the first
permissive instantiation of each constituent (cached afterwards), and whatever the relation resolves the first time;
it also inserts one `getConditionalTypeInstantiation` cache entry per constituent that later lookups hit. The charge
per skipped constituent is not a constant, so the counter that drives TS2589 (5M per statement) cannot be charged
exactly. The boundary regression tests (`testdata/regressions/conditional-instantiation-limit-*`: 1,117 constituents
stay at 5,034,569 instantiations without TS2589, 1,118 report it at the same node as tsgo-ref) pin it. A2 would be
chargeable on the distribution path alone (no per-constituent cache entry there) for structurally simple extends
types, but that path is 0.6% of the conditional work on big and absent elsewhere.

## Row 1 (inherent): relations to union targets

`typeRelatedToSomeType` (a source against a union target: the key-property match first, then every constituent in
order) is 8.9% of big, 16.2% of mui, 1.5% of vscode. The census split each call by how the loop ended and timed the
failed constituent checks:

| corpus | no constituent relates: calls, failed checks, time | relates later, no single-property index skips the earlier ones | relates later, a property index would skip every earlier one |
| --- | --- | --- | --- |
| big | 131,622 calls, 774K checks, 637 ms (4.0%; 2-member unions 343 ms) | 129,148 calls, 665K checks, 194 ms (1.2%) | 6,655 calls, 56K checks, 31 ms (0.2%) |
| mui | 35,887, 152K, 126 ms (5.3%) | 20,746, 165K, 20 ms (0.8%) | 19, 19, 0.1 ms |
| vscode | 22,757, 127K, 24 ms (0.3%) | 43,153, 117K, 24 ms (0.3%) | 5,251, 18K, 7 ms (0.1%) |
| webpack | 5,925, 28K, 27 ms (3.1%) | 3,371, 9K, 3 ms (0.3%) | 23, 50, 0.1 ms |
| xstate | 2,803, 10K, 5 ms (1.1%) | 9,289, 20K, 5 ms (1.1%) | 103, 182, 0 ms |

Two suspected causes hold in the Go source: `getKeyPropertyCandidateName` only tries the first unit-typed
property of the first object constituent, and `mapTypesByKeyProperty` gives up unless every object constituent has a
literal of that name and at least 10 and half of them are unique. But an index over every unit-typed property would
skip at most the last column (0.2% on big, 0.1% on vscode, nothing elsewhere). Most of the time is failures, where every constituent has
to be related anyway, and late successes whose earlier constituents are not excluded by any unit-typed property of the
source; 5-23% of the failed checks in failing calls and 1-5% in late successes are already relation-cache hits. Reordering would also not be exact: the skipped
failing checks record relation-cache entries (the TS2859 budget) and can resolve types the first time
(instantiation counts, type ids). Not pursued.

## Row 5 (inherent): mapped types over large key sets

mui spends 13.7% of its check samples in `resolveMappedTypeMembers`; the two resolutions of a `Partial<...>` over a
1,276-key intersection take 75-86 ms each (~60 us per key: each key's modifiers property is a synthetic property of an
intersection that contains a ~100-member union, built by scanning the constituents), the other 1,196 resolutions have
257-512 keys. A function profile of mui shows who asks: 8.7% of all samples resolve the mapped type for `isWeakType`
(`getIndexInfosOfStructuredType` 7.4%, `everyPropertyOfStructuredType` 1.3%), 3.8% for `getReducedType` of an
intersection containing it, the rest is scattered. `Partial` is all-optional, so the relation's weak-type check needs
every member and the index infos; after it, a relation that succeeds (mui has no errors) goes on to
`propertiesRelatedTo`, which lists every target property, and `getReducedType` lists every property of the
intersection. Answering `isWeakType` lazily (notes/mem-lazy.md, L4) therefore only moves the full resolution to the
next consumer: L4 answered 202K such queries for -2.2K symbols on big, half of the tables were resolved in full later
anyway (31K vs 21K), and property lookups went from 494K to 5.1M. The work is keys x constituents in the type itself;
not pursued.

## Row 2: derived generics related to their generic base by variances (handoff)

Status (2026-10-10): removed on 2026-10-07 (#194, docs/STATUS.md). `crates/tsrs_checker/src/relater_derived.rs`,
the `TSRS_DERIVED_VARIANCE*` switches and the `derived_variance` CLI test no longer exist; only the
`testdata/regressions/derived-variance-*` cases remain. Differential fuzzing found nine more kinds of disagreement; the guards that close them (4-6) remove the
savings below (guarded: -1.8% / -0.3% check time at 1 / 8 checkers), so the verdict was not to enable it
(notes/fuzz-derived-variance.md). The section below is the state at handoff: the code paths, switch and test it names
are gone, and its savings are for guards 1-3 only, which are not exact.

Full write-up: notes/perf-derived-variance.md. Code: `crates/tsrs_checker/src/relater_derived.rs`, switch
`TSRS_DERIVED_VARIANCE=off|shadow|on` (default off; forced off under `--checkerAssignment go`), docs/DEBUGGING.md.

- Design: when the target is a reference to a generic `G` and the source's base chain contains a reference `R` to `G`
  (with the source as `this`), relate `R` to the target by `G`'s variances; if True, members the source inherits
  unchanged count as related, the rest (and signatures, index signatures) are compared structurally. True-only; else
  the normal comparison runs. Only targets with 16+ properties.
- Guards (each from a shadow disagreement): `this` variance measured and covariant/bivariant/independent
  (`testdata/regressions/derived-variance-this-type`); no decisions inside a variance computation (mui-docs); no `any`
  argument at a parameter reaching a conditional check type (`testdata/regressions/derived-variance-any-conditional`).
  `cargo test -p tsrs_cli --test derived_variance` runs both cases off/on/shadow against tsgo-ref.
- Shadow (with guards): suite 661 decisions (both lazy modes), big 39,907, error-rich clone 35,510, vscode 2,671,
  webpack 470, mui 358, xstate 91; 0 disagreements. Failure mode if a digest error slips through: a missed error,
  never a spurious one; counters and the TS2589/TS2859 budgets drop (errors at the limit can fire later than in Go).
- Savings on big (on, all bases): -16.8% instructions / -18.5% check / -18.3% peak at 8 checkers, -12.9% at 4,
  -8.4% single-threaded; type-parameter-reliable variant -9.6% / -8.6% / -7.0%. Other corpora: no such hierarchies,
  within noise. Shadow costs +0.4-1.3%.
- Recommendation: all bases with the threshold and guards (TypeScript accepts True variance answers regardless of
  reliability flags). Default on/off is the owner's decision.

## Candidate B: `export default <expression>` (known, deferred)

With one checker the census sees 2,495 export-assignment type requests costing 8 ms on big: the expression is checked
when an importer first asks for the default export's type instead of when its own file is checked, so the work moves
and is done once. With several checkers it is done again by every checker that reaches the file without owning it
(notes/perf-checker-scaling.md; the private measurement put one aggregator file at ~2 s cold). Another agent is
removing the per-checker duplication at its root (forked checkers sharing one warm base); if that lands, B's
multi-checker benefit is moot. Not pursued this round.

## Reproduce

```sh
CARGO_TARGET_DIR=target/census cargo build --release -p tsrs_cli --features work-census
cd <project> && TSRS_WORK_CENSUS=/tmp/census.md target/census/release/tsrs -p . --noEmit --incremental false --extendedDiagnostics --checkers 1
TSRS_WORK_CENSUS_SLOW=5 ...   # also print mapped-type resolutions slower than 5 ms, with the type
# the A1 prototype and its cross-check: branch perf/algo-cond-prefilter, TSRS_VERIFY_COND_PREFILTER=<file>
```
