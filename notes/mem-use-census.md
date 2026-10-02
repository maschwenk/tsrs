# mem-use-census: what the checker builds, keeps reachable, and never reads

The reachability census (notes/mem-census.md) asks what is garbage at exit. This pass asks a different question
for the memory the checker adds: which objects are **created, stay reachable, and are never read after
creation**. Those are the candidates for laziness of the #64475 kind (build on first use instead of eagerly),
which free nothing but avoid creating.

Branch `mem/use-census`. Tool first (commit d428def + ea2021d), then two deferrals it pointed at (U1, U2).

## The tool (`TSRS_CENSUS=1 TSRS_USE_CENSUS=1`, alloc-profile build only)

Build: `CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile`.
Normal builds are unchanged: `tsrs_core::ucell::{Cell, RefCell}` are `std::cell::{Cell, RefCell}` there, `peek` is
`get`, and every `usebits::mark*` is an empty inline function (suite trees byte-identical in all four modes with the
tool compiled in).

**Recording.** A bitmap over the profile build's arena address range (chunks are mapped from 0x7c00_0000_0000 up,
mem-recycle), one bit per 4 bytes, set at the address of the field that was read. At exit the census counts a
block as *used* if any bit inside it is set. Three bitmaps: uses, reads inside *builder scopes*, and writes.

**What counts as a read (a use):**

- `get` / `replace` / `take` / `update` on the cells that hold arena fields: the checker's `Cell` / `RefCell`
  (`crate::Cell` is now `tsrs_core::ucell::Cell`, a transparent wrapper in the profile build), `borrow` /
  `try_borrow` on `RefCell` and `FrozenCell`, and `get` on `SliceCell`, `OptionSliceCell`, `StrCell`, `OwnedCell`,
  `OwnedSliceCell`, `OwnedStrCell`, `MapperCell`;
- a slice or string handed out by such a getter: the block that holds the elements (16-byte values whose second
  word is a small length; 24-byte values with a slice at either word, e.g. `LiteralValue::String`);
- the getters of hand-packed words: `ValueSymbolLinks::{target, mapper, containing_type, name_type, write_type,
  function_or_constructor_checked}` (the word they answer from), `Type::{symbol, alias}`, `Symbol::{parent,
  members, exports, export_symbol, value_declaration}`;
- `TypeMapper::data()` (the mapper and its source/target lists), lazy member tables' signature / index-info /
  base-type slices when handed out, the subtype-reduction cache, line maps, stored identifier text.

**Not reads:** writes (`set`, `borrow_mut`, `set_*`); the mode checks inside packed setters, which use the new
`peek` (`ValueSymbolLinks`, `Symbol`, `Type` symbol word, `StructuredType::resolved_for_write`, `Signature` rare
tail, `GoMap::set`); read-modify-write of flag words (`x.flags.set(x.flags.peek() | F)`, 66 sites); symbol-table
key comparison (the table reads its entries' names with `peek`); `TypeMapper::data_peek` in the escape barrier and
recycling; object ids (`Type.id`, symbol and node ids: what cache keys are built from); identity comparison and
hashing of `P`; counters; the census itself (it reads memory directly after `usebits::freeze`).

**Builder scopes** (`usebits::builder()`): reads inside `getNamedMembers`, `addInheritedMembers`,
`instantiateSymbolTable` and `instantiateSymbol` are recorded separately. These only assemble a structure from
objects (sort and filter a member list, copy inherited members, copy a symbol's declarations into its
instantiation); an object read only there is in some table but never consulted. Reported as "bonly".

**Reporting.** Only blocks allocated after the first `new_checker` ("checker phase"; the front end is one summary
line), minus blocks the arena freed or rewound (recycled in normal builds). Link-store chunks are split into
records (`usebits::note_slot` from every link store), each attributed to the code that asked for it (sampled
stacks), plus a table of link records never written (all zero: a non-allocating lookup would answer the same).
Tables: by type, by call site, by type and allocating function with N callers (`TSRS_USE_CENSUS_FRAMES`, sampled
1/16 like the census), symbols by depth of use (value links used / symbol used without its links / builder-only /
never), per-offset read shares for the top types (`TSRS_USE_CENSUS_FIELDS=N`), first readers
(`TSRS_USE_CENSUS_FIRST_READS=1`). `TSRS_CENSUS_TSV` gets every row (`use:` prefix).

**Validation by hand** (tiny programs with `--noLib`, `TSRS_CENSUS_ARENA_SAMPLE=1`): `interface Box<T> { a: T; b: T;
c(): T }; declare const x: Box<string>; x.a`. Default mode: of the instantiated `Box<string>` only the looked-up
member `a` is created (lazy table), and it is used; opt-out mode: three members instantiated by the full
resolution, the property list of the instantiated reference reported never read (nothing iterates it). The first
version counted every member as used: `getNamedMembers`' sort reads names and declarations, which is why builder
scopes exist. Other false uses found and fixed this way: packed setters reading their mode word, flag
read-modify-writes, symbol tables comparing keys. False *non*-uses found and fixed: slices held outside cells
(lazy member tables, subtype-reduction cache), identifier text stored out of line, `LiteralValue::String` in a
24-byte cell (string literal texts first showed up as 12.7 MB never read). The `TSRS_USE_CENSUS_FIRST_READS`
table (who reads each type first) is the check for the remaining definition: for every large class the first
reader is real checker work (relations, property lookups, `getReducedType`, inference), not plumbing.

Remaining known limits: plain (non-cell) fields read directly are invisible (by the port's uniform-mutability
rule there are almost none in arena structs besides `Type.id` / `data_tag`, which are intentionally not reads);
`Deref` of `ucell::Cell` to `std::cell::Cell` (for helpers written against `&Cell`) reads without recording (no
such helper reads checker fields); a 16-byte `Cell<(P<A>, u32)>` would mark `A` (none in the hot types). A "use"
is any read: a symbol whose flags are read by a lookup that only tests existence counts as used, so the symbol
depth table and the link records (whose reads mean the symbol's type or target was consulted) are the sharper
signal for symbols.

Cost: the private monorepo, 1 checker: ~3 min, ~17 GB peak (the reachability census plus three bitmaps).

## What it finds (checker phase, 1 checker, main de3beaf + tool; U1/U2 off)

| run | checker-phase arena + link records | not used | of which reachable at exit | builder-only |
| --- | --- | --- | --- | --- |
| private monorepo, default | 2,430 MB | 383 MB (15.8%) | 325 MB | 59 MB |
| private monorepo, `TSRS_LAZY_MEMBERS=0` | 3,738 MB | 1,278 MB (34.2%) | 1,214 MB | 536 MB |
| vscode (`src`), default | 770 MB | 109 MB (14.1%) | 85 MB | 15 MB |
| mui-docs (`docs`), default | 364 MB | 77 MB (21.2%) | 62 MB | 9 MB |

The opt-out row is the control: the tool attributes ~0.9 GB more unused-but-reachable memory to the eager member
tables (mostly builder-only symbols and their link records), the waste #64475/#64526 removed. With them on,
**about 13% of what the checker allocates is reachable and never read; there is no second pool of that size**.
The heap outside the arena (symbol-table entry buffers, instantiation maps, relation caches, lazy tables) is not in
these numbers.

### Ranked pools (unused and reachable, MB; sampled stacks, 9 frames, grouped by the code that forces creation)

| # | pool | private monorepo | vscode | mui-docs | what forces it | deferrable exactly? | Go |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | union/intersection properties synthesized by `getReducedType` -> `somePropertyReducesToNever` (relations' `getNormalizedType`) | 61.6 (intersection types 23.1, value links 14.9, link tails with write types 10.3, type lists 6.1, mappers 3.7) | 1.6 | 10.6 | `createUnionOrIntersectionProperty` computes the property type (and write type) eagerly for <= 2 constituents; `isNeverReducedProperty` only needs check flags unless `NonUniformAndLiteral` | no: this is B1 (rejected in mem-round3: type creation order / ids, and deferred-type paths read `resolvedType` differently) | same in Go |
| 2 | expression re-checks under contextual types (object literal types, members, property symbols, spread types, optional unions) | 42.8 | 23.7 | 14.1 | `checkExpressionWithContextualType` from overload resolution and inference; the results stay reachable through the type caches | no (mem-census item 3: escapes through value-keyed caches) | garbage-ish in Go too |
| 3 | object-type instantiation (mappers, type-argument lists, cloned type parameters) of instantiated types whose members are never resolved | 29.9 | 8.6 | 1.5 | `getObjectTypeInstantiation` on a cache miss (e.g. `getTypeOfSymbol` of union-property constituents, pool 1) | no: the mapper is the type; only not creating the type would help | same |
| 4 | instantiated signatures of anonymous types (parameter symbols 11.0, signatures 7.4, links 6.7, type parameters 1.9) | 28.9 | 2.9 | 0.5 | `resolveAnonymousTypeMembers` instantiates every call/construct signature when the type is resolved for any query | only with a #64475-style lazy table for anonymous instantiations (L3 showed most get resolved anyway) or lazy `Signature.parameters` (rejected in mem-lazy: hundreds of direct reads) | same |
| 5 | mapped type members (full resolutions) | 24.8 | 1.5 | 16.2 | `resolveMappedTypeMembers` for `every`/`getPropertiesOfType` queries; #64526 covers `keyof` lookups | partly: L4 (rejected) | same |
| 6 | line maps of every file | 21.6 | 10.6 | 2.4 | `Program::line_count` for the `Lines:` counter of `--diagnostics` / `--extendedDiagnostics` only | yes (count newlines without storing), not a checker item | same |
| 7 | union/intersection properties, other askers (contextual types, discriminants) | 21.3 | 2.9 | 10.8 | as pool 1 | no (as 1) | same |
| 8 | lazy tables resolved in full (not via a derived type) | 14.7 | 3.1 | 0.8 | `getPropertiesOfObjectType` in `propertiesRelatedTo`, inference, reverse mapped types | no simple one: callers iterate all members | same |
| 9 | **base types resolved in full inside `resolveLazyMembers`** | 14.7 (symbols 7.2, links 6.1, tables 1.1) | 1.9 | 1.0 | `getPropertiesOfType(base)` for each base of a table resolved in full | **yes: U2** | yes |
| 10 | **lazy members instantiated by discriminant matching in `getUnmatchedProperties`** | 10.2 links (+ ~25 MB of symbols whose only read is the existence test's flag check) | 3.8 | 0.1 | `inferFromTypes` -> `isTypeCloselyMatchedBy`-style `getUnmatchedProperty(source, target, false, true)` looks up every source property | **yes: U1** | yes |

Smaller rows: type alias records never printed (2.4 MB), `SymbolNodeLinks` records never written (363K records,
2.8 MB; tsrs-only, a `try_get` like mem-round3 A5), string literal texts copied by `getStringLiteralType` (12.8 MB,
all read; Go shares the caller's string, tsrs copies: a representation item).

Pools 1-5 and 7 are 210 MB of the 325 MB. None of them is a laziness candidate that keeps type creation order: the
unread objects are types (or hang off types) whose creation order assigns the ids that union ordering and printing
depend on, or they are created by resolutions whose other members are read. That is the main finding: after
#64475/#64526 and L1-L11, the remaining never-read memory is mostly *types*, and deferring types is what B1 showed to
be unsafe.

## U1: discriminant matching looks the source property up only when its type is needed (`TSRS_LAZY_DISCRIMINANTS`, default on)

`getUnmatchedPropertiesWorker(source, target, false, matchDiscriminantProperties=true)` (inference) calls
`getPropertyOfType(source, name)` for every required target property, which instantiates the source's lazy member,
then reads the source property's type only when the target property's type is a unit type. Now the source is
asked `hasPropertyOfType` (as L9/L10 do), and the real source property is looked up after the target type, only for
unit targets. Same results; the instantiation moves after the target type's resolution, which can only change the
order of symbol creation (as L10 did for target properties). Go: the same two-line change on top of L10's
`hasPropertyOfType`.

```go
-			sourceProp := c.getPropertyOfType(source, targetProp.Name)
-			if sourceProp == nil {
+			if !c.hasPropertyOfType(source, targetProp.Name) {
 				...
 			} else if matchDiscriminantProperties {
 				targetType := c.getTypeOfSymbol(targetProp)
 				if targetType.flags&TypeFlagsUnit != 0 {
+					sourceProp := c.getPropertyOfType(source, targetProp.Name)
 					sourceType := c.getTypeOfSymbol(sourceProp)
```

2,855,500 source lookups avoided (private monorepo, 1 checker). Symbols 13,264,462 -> 12,903,804 (-2.7%) single,
17,314,530 -> 16,947,825 (-2.1%) on 4 checkers; types, instantiations and diagnostics unchanged.

## U2: a table resolved in full inherits from lazy bases without resolving them (`TSRS_LAZY_BASES`, default on)

`resolveLazyMembers` called `getPropertiesOfType(base)` for each base type, which resolves a base that has a lazy
table of its own in full (every member instantiated, a member table and a property list built) only to copy into
the derived table the members it does not declare. U2 walks the base's L10 list (`getLazyPropertiesInOrder`: the
`getPropertiesOfType(base)` order with declared members standing in, same names and flags) and runs
`addInheritedMembers`' test on the stand-ins; a kept member is looked up with `getPropertyOfType(base, name)`, which
returns the symbol the full resolution would have stored (the lazy table caches instantiated members). Same
insertion sequence into the derived table, same symbols; the base stays lazy. Go: ~12 lines in `resolveLazyMembers`
on top of L10's helper.

Lazy tables resolved in full 133,092 -> 86,506 (31,482 base walks). Symbols (on top of U1) 12,903,804 -> 12,885,417
single, 16,947,825 -> 16,920,105 on 4 checkers; types, instantiations, diagnostics unchanged. Smaller than pool 9
suggested: most base members are inherited (kept) by the derived table, so they are created either way; U2 saves
the overridden members and the bases' own member tables.

## Measurements (private monorepo, 3 interleaved rounds, medians; machine load 20-40)

| run | base (tool only) | U1 | U1 + U2 |
| --- | --- | --- | --- |
| 1 checker, peak | 5.729 GiB | 5.710 (-0.019) | 5.700 (-0.030, -0.5%) |
| 4 checkers, peak | 7.574 GiB | 7.558 (-0.016) | 7.531 (-0.043, -0.6%) |
| 1 checker, instructions | 309.6 G | 308.7 G | 311.1 G (+0.5%) |
| 4 checkers, instructions | 424.3 G | 419.9 G | 421.9 G (-0.6%) |

Instructions vary by +-2% between identical runs here (range 307-315 G single); no measurable CPU cost.

## Gates

- Suite (`--baselines types,symbols`), whole `target/test-results` trees vs the base binary: identical in default,
  `TSRS_LAZY_MEMBERS=0`, single-threaded and `TS_TEST_PROGRAM_SINGLE_THREADED=false` (13,458 / 12,779 / 12,779), for
  the tool commit, U1 and U2 (only `summary.json` differs, in its `ms` timings).
- Fourslash: 4,066 pass / 63 fail (the known unported features).
- Private monorepo: diagnostics identical (19 errors from stale workspace builds) with 1 and 4 checkers in both
  modes; `TSRS_LAZY_MEMBERS=0` counters identical (the switches act only under the master switch); default counters
  change as stated above (symbols only).
- No memory is freed or reused by these changes, so the census free-gate does not apply.
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked` clean, also with `--features alloc-profile`.
