# mem-lazy: next lazy-resolution memory wins after #64475 / #64526

Goal: find more checker work of the #64475 / #64526 kind (do less eager work, identical results), prototype each
behind its own switch in `tsrs_core::lazymembers`, measure on Project, and write them up so they can be proposed
upstream in Go.

Every candidate switch only takes effect when the master switch (`TSRS_LAZY_MEMBERS`, default on) is on, so
`TSRS_LAZY_MEMBERS=0` stays reference-identical regardless of the candidate switches.

## Gates (per candidate)

1. Conformance, errors and `--baselines types,symbols`: the whole `target/test-results` tree (pass lists and every
   non-pass `.actual`/`.diff`) byte-identical with the candidate on vs off, single-threaded programs and
   `TS_TEST_PROGRAM_SINGLE_THREADED=false`.
2. `TSRS_LAZY_MEMBERS=0`: Project counters exactly 25,973,354 / 9,639,962 / 44,884,281 single and
   39,704,001 / 16,200,921 / 89,981,648 with 4 checkers.
3. Project: 0 errors.

## Measuring where objects come from: `--features site-counts`

`CARGO_TARGET_DIR=$PWD/target/sc cargo build --release -p tsrs_cli --features site-counts` builds a `tsrs` that
prints creation-site tables at exit (`tsrs_core::sitecount`; `TSRS_SITE_COUNTS_TOP=N` rows per kind). Compiled out
otherwise. The constructors are `#[track_caller]` under the feature, so each row is the code that asked for the
object: `symbol` (`new_symbol*`, `new_instantiated_symbol`, `instantiate_symbol`, union/intersection properties),
`type` (labelled by kind; `new_type`, `new_object_type`, `new_anonymous_type`, `create_type_reference*`,
`new_union_type`, `new_intersection_type`), `signature`, `mapper` (labelled by mapper kind), `links` (a new link slot,
labelled by link type), `instantiation` (counted instantiations by kind of the instantiated type), and
`first-resolve` / `first-resolve-created`: each `resolveStructuredTypeMembers` that actually resolves, attributed
to the caller that asked (through `getPropertiesOfType` / `getPropertiesOfObjectType`), weighted by the symbols and
signatures created *exclusively* by that resolution (nested resolutions are attributed to their own callers).

### Project, single-threaded, both PRs on (main at c1c1488)

15,331,397 symbols = 3.78M from the binder + 11.55M created by the checker:

| checker symbols | site |
| --- | --- |
| 3,766,182 (32.6%) | `instantiateSymbolTable` in `resolveObjectTypeMembers` (references without a lazy table, resolved in full) |
| 2,064,231 (17.9%) | `getLazyDeclaredMember` (#64475: members actually instantiated) |
| 1,613,504 (14.0%) | `checkObjectLiteral` properties (inherent) |
| 1,309,205 (11.3%) | `instantiateSymbols` = parameters of instantiated signatures |
| 1,064,385 (9.2%) | `newMappedTypeMember` (mapped types resolved in full) |
| 1,053,891 (9.1%) | `createUnionOrIntersectionProperty` |
| 313,908 (2.7%) | `createSymbolWithType` |

Where full resolutions come from (`first-resolve-created`, 8.19M symbols+signatures created by resolutions):

| created | resolved type | asked for by |
| --- | --- | --- |
| 2,962,421 | reference | `getPropertiesOfType(baseType)` in `resolveObjectTypeMembers`: **78% of these base properties belong to tuples** (`Array<E>` with `this` = the tuple, ~40 members, a new reference per tuple) |
| 1,278,306 | instantiated anonymous | `getSignaturesOfStructuredType` (function types: signature + parameters) |
| 538,242 | reference | `getUnmatchedProperties` (target enumeration, inherent) |
| 361,616 | mapped | member lookup on a mapped type without a lazy table |
| 338,279 / 252,748 | reference / inst. anonymous | `somePropertyReducesToNever` (constituents other than the one #64526 skips) |
| 271,690 | mapped | `everyPropertyOfStructuredType` (`hasPropertiesOfStructuredType`, `isWeakType`, ...) |
| 197,716 | reference | `resolveLazyMembers` base properties |
| 170,989 | inst. anonymous | `isEmptyObjectType` |

Other kinds: types 9.63M (references 2.11M, instantiated anonymous 1.74M, anonymous 1.27M, intersections 1.13M,
unions 1.02M, conditional 0.63M); type mappers 21.6M (composite 6.6M, of which 4.34M are the
`combineTypeMappers(t.mapper, m)` built before every conditional-type instantiation cache lookup); link slots 22.5M
(value-symbol links 13.6M, 7.3M of them for instantiated symbols); signatures 1.57M (965K from
`instantiateSignatures`, 98.8% of those in `resolveAnonymousTypeMembers`).

## Candidates

### L1: lazy member tables for tuple references — landed, default on (`TSRS_LAZY_TUPLES`)

#64475 gives instantiated class/interface references a lazy member table but excludes tuple references. A tuple
reference resolved in full instantiates its element properties and `length` and resolves its base type
`Array<E1 | E2 | ...>` with `this` = the tuple in full: a fresh reference per tuple (the `this` argument differs),
~40 instantiated `Array` members each. Tuples are resolved mostly for signature, index-info and member queries,
which the lazy table answers; their base `Array<..., this>` then gets a lazy table of its own (it is an interface
reference) and only the looked-up members (`map`, `length`, ...) are instantiated.

Go change (in `getReadyLazyMemberTableWorker`):

```go
-	if t.flags&TypeFlagsObject == 0 || source == nil || source == t || source.objectFlags&ObjectFlagsClassOrInterface == 0 ||
-		source.objectFlags&ObjectFlagsTuple != 0 || t.symbol != nil && t.symbol.Flags&ast.SymbolFlagsValueModule != 0 {
+	if t.flags&TypeFlagsObject == 0 || source == nil || source == t || source.objectFlags&(ObjectFlagsClassOrInterface|ObjectFlagsTuple) == 0 ||
+		t.symbol != nil && t.symbol.Flags&ast.SymbolFlagsValueModule != 0 {
```

(`ObjectFlagsTuple` targets are not `ClassOrInterface`, so the old second test was redundant with the first.)
Nothing else is tuple-specific: tuple targets have declared members (`"0"`, `"1"`, ..., `length`) and base types
like interfaces, and every consumer already goes through the lazy-aware accessors of #64475.

Project (medians of 3, interleaved, same binary; machine load ~7, so absolute times are higher than the
headline numbers in the task):

| run | symbols | types | instantiations | check s | wall s | peak GB |
| --- | --- | --- | --- | --- | --- | --- |
| single, both PRs | 15,331,397 | 9,630,120 | 44,820,708 | 23.62 | 27.88 | 11.59 |
| single, + L1 | 13,297,831 (-13.3%) | same | same | 23.01 | 27.04 | 11.04 (-4.7%) |
| 4 checkers, both PRs | 22,813,093 | 16,186,849 | 89,882,712 | 13.41 | 15.70 | 17.29 |
| 4 checkers, + L1 | 19,555,571 (-14.3%) | same | same | 13.58 | 17.29 | 16.41 (-5.1%) |

Per-path (single / 4 checkers): 66,727 / 100,248 tuple tables, of which 8,402 / 11,824 (13% / 12%) were later
resolved in full; all lazy tables 881,767 -> 1,013,954 single (the other new ones are the tuples' `Array` bases);
declared members in lazy tables +2.82M, of which +0.66M instantiated, so 2.16M (77%) of the new lazy members are
never created.

Gates: suite tree identical (single-threaded and multi-threaded programs); reference mode counters exact; 0 errors.
