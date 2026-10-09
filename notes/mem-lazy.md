# mem-lazy: next lazy-resolution memory wins after #64475 / #64526

> 2026-10-09: the per-candidate switches (`TSRS_LAZY_TUPLES`, `TSRS_LAZY_COND_MAPPER`, `TSRS_LAZY_PROP_CACHE`,
> `TSRS_LAZY_UNMATCHED`, `TSRS_LAZY_EMPTY`, and `TSRS_LAZY_INFERENCE_MAPPERS` of notes/mem-round3.md) were removed:
> the landed candidates now follow the master switch, so `TSRS_LAZY_MEMBERS=0` is still the reference-identical mode
> and there is no untested "master on, candidate off" state. L9 (`TSRS_LAZY_HAS_PROP`, shipped off, -0.1%) was deleted
> with its switch. The counters stay.

Goal: find more checker work of the #64475 / #64526 kind (do less eager work, produce identical results), prototype
each behind its own switch in `tsrs_core::lazymembers`, measure on the private monorepo, and write the winners up so they can be
proposed upstream in Go.

Each candidate switch only takes effect when the master switch (`TSRS_LAZY_MEMBERS`, default on) is on, so
`TSRS_LAZY_MEMBERS=0` stays reference-identical whatever the candidate switches say. Every candidate has
`LazyMemberStats` counters (printed with `--extendedDiagnostics`).

## Summary

| id | change | switch, default | The private monorepo single: symbols / peak | 4 checkers: symbols / peak |
| --- | --- | --- | --- | --- |
| L1 | lazy member tables for tuple references too | `TSRS_LAZY_TUPLES`, **on** (bd93422) | -13.3% / -4.7% | -14.3% / -5.1% |
| L5 | conditional-type instantiation without the composite mapper | `TSRS_LAZY_COND_MAPPER`, **on** | 0 / -1.1% (4.34M mappers) | 0 / -1.4% |
| L6 | union/intersection property caches: no eager copy between the two caches | `TSRS_LAZY_PROP_CACHE`, **on** | 0 / -1.2% (2.37M entries) | 0 / -1.1% |
| L10 | `getUnmatchedProperties` over a lazy target without instantiating it | `TSRS_LAZY_UNMATCHED`, **on** | -1.8% / -0.4% | -2.5% / -0.5% |
| L11 | empty-object tests answered by lazy member tables | `TSRS_LAZY_EMPTY`, **on** | -1.6% / -0.4% | -1.5% / -0.3% |
| L5+L6+L10+L11 | together | | -3.7% / -3.3% | -4.3% / -3.8% |
| L9 | existence-only lookup in `getUnmatchedProperties` (non-lazy targets) | `TSRS_LAZY_HAS_PROP`, off | -0.1% (on top of the rest) | |
| L2, L3, L4, L8 | see "Did not pan out" | removed (code in 8b11836) | ~0 | |

Types and instantiations are unchanged by every landed candidate (only L1 and L10 change which lazy tables exist).
Check time is unchanged within noise for all of them. Suggested upstream order: L1 (one-line condition change on
top of #64475, largest win), L11 (tiny, self-contained), L10 (needs #64475's tables and a small ordering helper),
L6 (independent of the lazy tables, small), L5 (independent; in Go it saves allocations/GC work rather than
retained memory, see below).

## Gates (every landed candidate)

1. Conformance, errors and `--baselines types,symbols`: the whole `target/test-results` tree (pass lists and every
   non-pass `.actual`/`.diff`) byte-identical with the candidate on vs off, both single-threaded programs and
   `TS_TEST_PROGRAM_SINGLE_THREADED=false` (13,458 / 12,779 / 12,779 pass).
2. `TSRS_LAZY_MEMBERS=0`: private-monorepo counters exactly 25,973,354 / 9,639,962 / 44,884,281 single and
   39,704,001 / 16,200,921 / 89,981,648 with 4 checkers. (Since c9c52b2 the default 4-checker run assigns files by
   directory locality; the reference 4-checker numbers are for `--checkerAssignment go`.)
3. The private monorepo 0 errors. Extra check: the quick tier of the private monorepo `.types` comparison (docs/DEBUGGING.md, 2,000
   sampled files against the cached reference dump) is 2000/2000 identical with all landed candidates on.

## Measuring where objects come from: `--features site-counts`

`CARGO_TARGET_DIR=$PWD/target/sc cargo build --release -p tsrs_cli --features site-counts` builds a `tsrs` that
prints creation-site tables at exit (`tsrs_core::sitecount`; `TSRS_SITE_COUNTS_TOP=N` rows per kind). Compiled out
otherwise. The constructors are `#[track_caller]` under the feature, so each row is the code that asked for the
object: `symbol`, `type` (labelled by kind), `signature`, `mapper` (labelled by mapper kind), `links` (new link
slots by link type), `instantiation` (by kind of the instantiated type), and:

- `first-resolve` / `first-resolve-created`: each `resolveStructuredTypeMembers` that actually resolves, attributed
  to the code that asked (through `getPropertiesOfType` / `getPropertiesOfObjectType`), weighted by the symbols and
  signatures created *exclusively* by that resolution; lazy tables resolved in full are labelled `reference:lazy`.
- `inst-symbol-created` / `inst-symbol-type-resolved`: instantiated symbols by creation site (through
  `getPropertyOfType` and the lazy-member lookup chain), and how many of them ever get their type resolved.

The second pair turned out to be the useful predictor. "First resolved by X" over-promises: when X stops resolving
a type, the next consumer usually resolves it anyway (that is what sank L2, L3, L4 and L8). Objects that are created
and never used afterwards are the real wins (L1, L5, L6, and the failing relations in L10).

### The private monorepo, single-threaded, both PRs on (main at c1c1488, before L1)

15,331,397 symbols = 3.78M from the binder + 11.55M created by the checker:

| checker symbols | site |
| --- | --- |
| 3,766,182 (32.6%) | `instantiateSymbolTable` in `resolveObjectTypeMembers` (references without a lazy table, resolved in full; 78% of the inherited members here belonged to tuples' `Array` bases) |
| 2,064,231 (17.9%) | `getLazyDeclaredMember` (#64475: members actually instantiated) |
| 1,613,504 (14.0%) | `checkObjectLiteral` properties |
| 1,309,205 (11.3%) | `instantiateSymbols` = parameters of instantiated signatures |
| 1,064,385 (9.2%) | `newMappedTypeMember` |
| 1,053,891 (9.1%) | `createUnionOrIntersectionProperty` |
| 313,908 (2.7%) | `createSymbolWithType` |

Other kinds: types 9.63M (references 2.11M, instantiated anonymous 1.74M, anonymous 1.27M, intersections 1.13M,
unions 1.02M, conditional 0.63M); type mappers 21.6M (composite 6.6M, of which 4.34M are the
`combineTypeMappers(t.mapper, m)` built before every conditional-type instantiation cache lookup); link slots 22.5M
(value-symbol links 13.6M); signatures 1.57M (965K from `instantiateSignatures`, 98.8% of those in
`resolveAnonymousTypeMembers`).

Where full resolutions came from (`first-resolve-created`, 8.19M symbols+signatures):

| created | resolved type | asked for by |
| --- | --- | --- |
| 2,962,421 | reference | `getPropertiesOfType(baseType)` in `resolveObjectTypeMembers`: 78% for tuples (their `Array<E>` base with `this` = the tuple, ~40 members, a new reference per tuple) -> **L1** |
| 1,278,306 | instantiated anonymous | `getSignaturesOfStructuredType` (function types: signature + parameters) |
| 538,242 | reference | `getUnmatchedProperties` (target enumeration) -> **L10** |
| 361,616 | mapped | member lookup on a mapped type without a lazy table |
| 338,279 / 252,748 | reference / inst. anonymous | `somePropertyReducesToNever` -> L2 (did not pan out) |
| 271,690 | mapped | `everyPropertyOfStructuredType` -> L4 (did not pan out) |
| 170,989 | inst. anonymous | `isEmptyObjectType` -> L3 (did not pan out); for references -> **L11** |

### After L1 (main at 195aacb): instantiated symbols, created vs ever typed

| created | typed | site |
| --- | --- | --- |
| 1,396,522 | 31.6% | `resolveLazyMembers` (lazy table resolved in full) |
| 1,309,205 | 74.5% | signature parameters (`instantiateSymbols`) |
| 1,073,821 | 62.5% | `instantiateSymbolTable` (full resolution without a table) |
| 651,866 | 39.3% | `getUnmatchedProperties` -> `getPropertyOfType(source, name)` |
| 448,184 | 99.9% | `somePropertyReducesToNever` (skipped constituent lookups) |
| 163,142 | 57.3% | `createInstantiatedSymbolTable` (instantiated anonymous types) |

The full resolutions of lazy tables (`reference:lazy`, 132K) were asked for by: base types of tables resolved in
full (47K, 499K symbols), `getUnmatchedProperties` (34K, 453K symbols), `getPropertiesOfUnionOrIntersectionType`
(14K), `isEmptyObjectType` (10K), `propertiesIdenticalTo` (8K), `removeSubtypes`' empty-object test.

## Measurements (main 195aacb + the candidates, medians of 3 interleaved rounds)

The private monorepo = the pristine read-only checkout, 18-core machine shared with other agents (load 6-8), `peak memory
footprint` from `/usr/bin/time -l` in GB (10^9 bytes), check time from `--extendedDiagnostics`. 4 checkers use the
default (locality) file assignment.

| run | symbols | types | instantiations | check s | peak GB |
| --- | --- | --- | --- | --- | --- |
| single, L1 only (L5/L6/L10/L11 off) | 13,297,831 | 9,630,120 | 44,820,708 | 16.92 | 9.04 |
| single, + L5 | 13,297,831 | same | same | 16.83 | 8.94 |
| single, + L6 | 13,297,831 | same | same | 16.64 | 8.93 |
| single, + L10 | 13,053,070 | same | same | 16.84 | 9.00 |
| single, + L11 | 13,082,798 | same | same | 16.71 | 9.00 |
| single, + all four (new default) | 12,811,032 | same | same | 16.68 | 8.74 |
| 4 checkers, L1 only | 17,297,363 | 13,788,912 | 76,888,800 | 6.67 | 12.19 |
| 4 checkers, + L5 | 17,297,363 | same | same | 6.69 | 12.02 |
| 4 checkers, + L6 | 17,297,363 | same | same | 6.67 | 12.06 |
| 4 checkers, + L10 | 16,869,751 | same | same | 6.80 | 12.13 |
| 4 checkers, + L11 | 17,029,740 | same | same | 6.80 | 12.15 |
| 4 checkers, + all four (new default) | 16,549,988 | same | same | 6.78 | 11.73 |

L1 was measured on its own base (main at c1c1488, same protocol, see its section).

Per-path counts with all candidates on (single / 4 checkers): lazy member tables 1,036,922 / 1,489,514 (L1-only
default: 1,013,954 / 1,463,887), resolved in full 128,552 / 189,343 (was 140,112 / 210,175), lazy members
instantiated 2,511,098 / 3,583,748 (was 2,723,026 / 4,074,052); tuple tables 70,353 / 93,180, resolved in full
4,957 / 5,322; conditional composite mappers avoided 4,340,555 / 7,677,745; augmented property-cache copies avoided
2,369,096 / 3,391,408 (augmented lookups served from the other cache 70,673 / 77,947); existence-only property
queries 621,288 / 1,140,871, answered from an uninstantiated member 152,771 / 340,444; lazy property-order lists
requested (incl. recursive and cached) 190,623 / 300,502; empty-object queries answered by tables 33,808 / 39,676.

## Landed candidates

### L1: lazy member tables for tuple references (`TSRS_LAZY_TUPLES`, default on)

#64475 gives instantiated class/interface references a lazy member table but leaves tuple references out. A tuple
reference resolved in full instantiates its element properties and `length` and resolves its base type
`Array<E1 | E2 | ...>` with `this` = the tuple in full: a fresh reference per tuple (the `this` argument differs),
~40 instantiated `Array` members each. Tuples are resolved mostly for signature, index-info and member queries,
which the lazy table answers; their base `Array<..., this>` then gets a lazy table of its own (it is an interface
reference) and only the looked-up members (`map`, `length`, ...) are instantiated.

```go
 func (c *Checker) getReadyLazyMemberTableWorker(t *Type) *lazyMemberTable {
 	source := t.Target()
-	if t.flags&TypeFlagsObject == 0 || source == nil || source == t || source.objectFlags&ObjectFlagsClassOrInterface == 0 ||
-		source.objectFlags&ObjectFlagsTuple != 0 || t.symbol != nil && t.symbol.Flags&ast.SymbolFlagsValueModule != 0 {
+	if t.flags&TypeFlagsObject == 0 || source == nil || source == t || source.objectFlags&(ObjectFlagsClassOrInterface|ObjectFlagsTuple) == 0 ||
+		t.symbol != nil && t.symbol.Flags&ast.SymbolFlagsValueModule != 0 {
```

(`ObjectFlagsTuple` targets are not `ClassOrInterface`, so the old second test was redundant with the first.)
Nothing else is tuple-specific: tuple targets have declared members (`"0"`, `"1"`, ..., `length`) and base types
like interfaces, and every consumer already goes through the lazy-aware accessors of #64475.

| run (main at c1c1488) | symbols | types | instantiations | check s | wall s | peak GB |
| --- | --- | --- | --- | --- | --- | --- |
| single, both PRs | 15,331,397 | 9,630,120 | 44,820,708 | 23.62 | 27.88 | 11.59 |
| single, + L1 | 13,297,831 (-13.3%) | same | same | 23.01 | 27.04 | 11.04 (-4.7%) |
| 4 checkers (go assignment), both PRs | 22,813,093 | 16,186,849 | 89,882,712 | 13.41 | 15.70 | 17.29 |
| 4 checkers (go assignment), + L1 | 19,555,571 (-14.3%) | same | same | 13.58 | 17.29 | 16.41 (-5.1%) |

Per-path (single / 4 checkers): 66,727 / 100,248 tuple tables, 8,402 / 11,824 (13% / 12%) later resolved in full;
declared members in lazy tables +2.82M single, of which +0.66M instantiated: 77% of the new lazy members are never
created.

### L5: conditional-type instantiation without the composite mapper (`TSRS_LAZY_COND_MAPPER`, default on)

`instantiateTypeWorker` builds `combineTypeMappers(t.mapper, m)` for every conditional type it instantiates, but
`getConditionalTypeInstantiation` only uses that mapper to compute the type arguments (`mapper.Map` over the outer
type parameters) for the cache key; on a miss it builds `newTypeMapper(outerTypeParameters, typeArguments)`. The
composite is dropped right away, 4.34M times single-threaded (7.68M on 4 checkers). Applying the two mappers the way
`CompositeTypeMapper.Map` does gives the same type arguments without the allocation.

```go
 	case flags&TypeFlagsConditional != 0:
-		return c.getConditionalTypeInstantiation(t, c.combineTypeMappers(t.AsConditionalType().mapper, m), false /*forConstraint*/, alias)
+		return c.getConditionalTypeInstantiationEx(t, t.AsConditionalType().mapper, m, false /*forConstraint*/, alias)

 func (c *Checker) getConditionalTypeInstantiation(t *Type, mapper *TypeMapper, forConstraint bool, alias *TypeAlias) *Type {
+	return c.getConditionalTypeInstantiationEx(t, nil, mapper, forConstraint, alias)
+}
+
+// The effective mapper is combineTypeMappers(m1, mapper), which is only needed for the type arguments.
+func (c *Checker) getConditionalTypeInstantiationEx(t *Type, m1 *TypeMapper, mapper *TypeMapper, forConstraint bool, alias *TypeAlias) *Type {
 	root := t.AsConditionalType().root
 	if len(root.outerTypeParameters) != 0 {
-		typeArguments := core.Map(root.outerTypeParameters, mapper.Map)
+		typeArguments := core.Map(root.outerTypeParameters, func(t *Type) *Type {
+			// What CompositeTypeMapper{m1, mapper}.Map does.
+			if m1 != nil {
+				if t1 := m1.Map(t); t1 != t {
+					return c.instantiateType(t1, mapper)
+				}
+			}
+			return mapper.Map(t)
+		})
```

(Not `mapTypeWithCompositeMapper`: that one goes through `getMappedType`, which first replaces a distributed type
parameter by its constraint; `CompositeTypeMapper.Map` does not.) In tsrs the mappers live in the arena, so this is
retained memory (-0.10 GB single, -0.17 GB on 4 checkers); in Go the composites are garbage right away, so upstream
the win is ~4.3M fewer allocations of a 40-byte object per private-monorepo run (less GC work), not peak.

### L6: union/intersection property caches without the eager copy (`TSRS_LAZY_PROP_CACHE`, default on)

`getUnionOrIntersectionProperty` keeps two caches per union/intersection type (with and without the
function-property augment). A non-partial property created without the augment is also copied into the augmented
cache. That copy is only ever read by a later augmented lookup of the same name, which can read it from the other
cache instead, with the same result in every order of lookups (the augmented cache is consulted first, so a
property the augmented lookup created itself still wins; a partial property is still recreated). And
`ast.GetSymbolTable` allocates a cache map on lookups that then store nothing; L6 allocates on the first store.
2.37M map entries fewer single-threaded (3.39M on 4 checkers), 70,673 augmented lookups served from the other
cache. Nothing else reads the caches.

```go
 func (c *Checker) getUnionOrIntersectionProperty(t *Type, name string, skipObjectFunctionPropertyAugment bool) *ast.Symbol {
-	var cache ast.SymbolTable
-	if skipObjectFunctionPropertyAugment {
-		cache = ast.GetSymbolTable(&t.AsUnionOrIntersectionType().propertyCacheWithoutFunctionPropertyAugment)
-	} else {
-		cache = ast.GetSymbolTable(&t.AsUnionOrIntersectionType().propertyCache)
-	}
-	if prop := cache[name]; prop != nil {
+	d := t.AsUnionOrIntersectionType()
+	cache := &d.propertyCache
+	if skipObjectFunctionPropertyAugment {
+		cache = &d.propertyCacheWithoutFunctionPropertyAugment
+	}
+	if prop := (*cache)[name]; prop != nil {
 		return prop
 	}
+	// A non-partial property found without the function-property augment is also the augmented result.
+	if !skipObjectFunctionPropertyAugment {
+		if prop := d.propertyCacheWithoutFunctionPropertyAugment[name]; prop != nil && prop.CheckFlags&ast.CheckFlagsPartial == 0 {
+			return prop
+		}
+	}
 	prop := c.createUnionOrIntersectionProperty(t, name, skipObjectFunctionPropertyAugment)
 	if prop != nil {
-		cache[name] = prop
-		// Propagate an entry from the non-augmented cache to the augmented cache unless the property is partial.
-		if skipObjectFunctionPropertyAugment && prop.CheckFlags&ast.CheckFlagsPartial == 0 {
-			augmentedCache := ast.GetSymbolTable(&t.AsUnionOrIntersectionType().propertyCache)
-			if augmentedCache[name] == nil {
-				augmentedCache[name] = prop
-			}
-		}
+		ast.GetSymbolTable(cache)[name] = prop
 	}
 	return prop
 }
```

-0.11 GB single, -0.13 GB on 4 checkers (in Go: Go map entries for the same 2.4M names, plus the empty maps).

### L10: `getUnmatchedProperties` over a lazy target without instantiating it (`TSRS_LAZY_UNMATCHED`, default on)

`getUnmatchedPropertiesWorker(source, target, ...)` walks `getPropertiesOfType(target)` and, for each required
property, asks `getPropertyOfType(source, name) != nil`. Both sides instantiate members they do not need: the
target is resolved in full (34K lazy tables, 453K symbols), and the source's lazy table instantiates every looked-up
member even though only its existence matters (652K members, 61% never typed). This is the relater's hot path for
failing comparisons (union constituents, overload candidates), where the members are dropped right after.

With L10, when the target has a lazy member table:

- the loop runs over the properties `getPropertiesOfType(target)` would return, **in the same order**, but with
  declared members standing in for instantiations the table has not created (same name, flags and declarations);
  this list (`getLazyPropertiesInOrder`) mirrors `resolveLazyMembers` (declared named members, then
  `addInheritedMembers` over each base type's properties) and `getNamedMembers` (members declared in the class or
  interface first, each part sorted with `compareSymbols`, which only looks at declarations and names, and names are
  unique in a member table). It is kept on the table, so base types share it (a first version walked
  `everyLazyProperty` per call: exponential on diamond-shaped mixin hierarchies, `intersectionConstructorReductionCrash`
  went from 6.7 s to 20 s);
- without `matchDiscriminantProperties`, the source is asked with a non-instantiating variant of
  `getPropertyOfType` (`getPropertyOfTypeWorker(..., instantiate=false)`: a declared member that has not been
  instantiated yet is returned as is; only tested for nil);
- the real target property is looked up (`getPropertyOfType(target, name)`) only where it is returned or appended to
  `propsOut`, or its type is compared (discriminants).

So the visited order, the reported properties and the source lookups are those of the reference loop; only the
member instantiations differ. -245K symbols single (-1.8%), -428K on 4 checkers (-2.5%); peak -0.04 / -0.06 GB.

Go sketch:

```go
 func (c *Checker) getUnmatchedPropertiesWorker(source *Type, target *Type, requireOptionalProperties bool, matchDiscriminantProperties bool, propsOut *[]*ast.Symbol) *ast.Symbol {
-	properties := c.getPropertiesOfType(target)
+	properties, lazy := c.getLazyPropertiesInOrder(target) // declared members stand in for uninstantiated ones
+	if !lazy {
+		properties = c.getPropertiesOfType(target)
+	}
 	for _, targetProp := range properties {
 		...
-			sourceProp := c.getPropertyOfType(source, targetProp.Name)
-			if sourceProp == nil {
+			if !matchDiscriminantProperties && lazy {
+				if !c.hasPropertyOfType(source, targetProp.Name) {
+					// report c.getPropertyOfType(target, targetProp.Name)
+				}
+				continue
+			}
+			sourceProp := c.getPropertyOfType(source, targetProp.Name)
+			if lazy {
+				targetProp = c.getPropertyOfType(target, targetProp.Name)
+			}
+			if sourceProp == nil {
```

plus `getLazyPropertiesInOrder` (~35 lines, a `properties []*ast.Symbol` field on `lazyMemberTable`) and an
`instantiate bool` parameter threaded through `getPropertyOfTypeEx` -> `getMemberOfStructuredType` ->
`getMemberOfUnresolvedStructuredType` (only the `getLazyDeclaredMember` call depends on it).

### L11: empty-object tests answered by lazy member tables (`TSRS_LAZY_EMPTY`, default on)

`isEmptyObjectType` and the empty-object test in `removeSubtypes` both do
`isEmptyResolvedType(resolveStructuredTypeMembers(t))`, which resolves a lazy table in full (all members
instantiated) to learn that it has properties. A type with a lazy table is an instantiated reference (never
`anyFunctionType`), and the table already has its signatures and index infos, so:

```go
+// isEmptyResolvedType(resolveStructuredTypeMembers(t)) without resolving a lazy member table.
+func (c *Checker) isEmptyStructuredType(t *Type) bool {
+	if lm := c.getReadyLazyMemberTable(t); lm != nil {
+		return len(lm.callSignatures) == 0 && len(lm.constructSignatures) == 0 && len(lm.indexInfos) == 0 && !c.hasPropertiesOfStructuredType(t)
+	}
+	return c.isEmptyResolvedType(c.resolveStructuredTypeMembers(t))
+}
```

used at both sites. 30,572 queries answered (single), -215K symbols (-1.6%), -268K on 4 checkers; peak -0.04 GB.

## Did not pan out (code for reproduction in 8b11836, removed afterwards)

- **L2 `somePropertyReducesToNever` walks names of every lazy constituent** (instead of #64526's one skipped
  constituent), and of unresolved anonymous instantiations through their target. -19K symbols (-0.15%), +975 types,
  +19.8K instantiations: the final loop stops at the first never-reduced property, and a different name order made
  it create other combined properties before stopping. Net nothing.
- **L3 shape queries on unresolved anonymous instantiations** (`isWeakType`, `isEmptyObjectType`,
  `everyPropertyOfStructuredType` answered from the target, which has the same property names/flags and signature
  and index-info counts). 250K `isWeakType` and 105K `isEmptyObjectType` queries answered, but -19.6K symbols
  (-0.15%): almost every such type (mostly function types, as relation targets) is compared structurally right
  after (`signaturesRelatedTo`, inference) and resolved anyway. It also moves the creation of fresh type parameters
  of instantiated generic signatures later (type ids), which the gates tolerated but is not worth it.
- **L4 `everyPropertyOfStructuredType` on #64526 mapped tables** (members created one at a time, in resolution
  order, stopping when the predicate does). 202K queries answered, -2.2K symbols: `isWeakType` must see every member
  when all are optional, and half of these tables are resolved in full later anyway (31K vs 21K); lookups went from
  494K to 5.1M.
- **L8 no eager base-property resolution in `prepareLazyMembers`** (the #64475 timing mirror for bases that cannot
  have a lazy table): -30 symbols. The bases are needed anyway; the mirror is free.
- **L9 existence-only lookups in `getUnmatchedProperties` for targets without a lazy table**: -48K symbols alone,
  -15K on top of L10 (the members are instantiated by the property comparison that follows). Kept, default off.

Measured and dropped without a prototype:

- **Deferring base types / inherited signatures and index infos in `prepareLazyMembers`.** Profile (top-level
  prepares only): 1.28M types, 2.39M instantiations and 224K symbols+signatures created by preparing tables, of
  which 97.6% are needed later by a signature, index-info, every-property or missed-member query (first need:
  `isWeakType` 32%, then call resolution and `signaturesRelatedTo`). Deferring them would also change type creation
  order, which #64475 deliberately keeps.
- **`unaffected` names in lazy tables as a bitset**: only 1.13M entries in total (18 MB). Not worth it.
- **`valueSymbolLinks.Get` -> `TryGet` for read-only accesses** (e.g. `getContextualTypeForObjectLiteralElement`
  reading `nameType` of declaration symbols, 969K slots): ids are still assigned either way; in Go the paged store
  allocates the slot with its page regardless, so there is no upstream win (in tsrs ~40 MB).
- **Instantiation caches (`instantiations` maps per generic type)**: each entry holds a type that exists anyway; the
  maps cannot shrink without losing type identity.
- **Lazy signature parameters** (1.31M parameter symbols of instantiated signatures, 74.5% typed): needs an accessor
  for `Signature.parameters`, which Go reads directly at hundreds of sites.
- **Lazy keyof mapped tables over intersections and `as` clauses** (38.8K refusals of #64526 tables): would save
  part of 294K symbols for intersection modifiers, but L3/L4 suggest most of these tables end up resolved; not
  prototyped.
