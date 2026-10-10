# mem-use-census: what the checker builds, keeps reachable, and never reads

The reachability census (notes/mem-census.md) asks what is garbage at exit. This pass asks a different question
for the memory the checker adds: which objects are **created, stay reachable, and are never read after
creation**. Those are the candidates for laziness of the #64475 kind (build on first use instead of eagerly),
which free nothing but avoid creating.

Status (2026-10-10): nothing from this note is on main. The tool (`TSRS_USE_CENSUS`, `tsrs_core::ucell`, `usebits`)
and the two deferrals U1 (`TSRS_LAZY_DISCRIMINANTS`) and U2 (`TSRS_LAZY_BASES`) lived on branch `mem/use-census`; PR #7
was closed with them (notes/mem-never-read-apps.md). The tool was later ported to branch `mem/use-census-apps` for
that note. The findings below stand as measurements.

## The tool (branch only)

An alloc-profile build kept a bitmap over the arena address range, one bit per 4 bytes, set at the address of a field
that was read through the checker's cells (`get` / `borrow` on `Cell`, `RefCell`, the packed slice and string cells,
and the getters of hand-packed words). At exit a block counted as *used* if any bit inside it was set. Writes, mode
checks inside packed setters (a new `peek`), flag read-modify-writes, symbol-table key comparison, ids and identity
comparisons did not count as reads. Reads inside *builder scopes* (`getNamedMembers`, `addInheritedMembers`,
`instantiateSymbolTable`, `instantiateSymbol`, which only assemble structures) were recorded separately as
"builder-only". The first version counted every member as used because `getNamedMembers`' sort reads names and
declarations, which is why builder scopes exist; the first-reader table confirmed that for every large class the
first reader is real checker work. Cost: the private monorepo, 1 checker: ~3 min, ~17 GB peak.

## What it finds (checker phase, 1 checker, main de3beaf + tool)

| run | checker-phase arena + link records | not used | of which reachable at exit | builder-only |
| --- | --- | --- | --- | --- |
| private monorepo, default | 2,430 MB | 383 MB (15.8%) | 325 MB | 59 MB |
| private monorepo, `TSRS_LAZY_MEMBERS=0` | 3,738 MB | 1,278 MB (34.2%) | 1,214 MB | 536 MB |
| vscode (`src`), default | 770 MB | 109 MB (14.1%) | 85 MB | 15 MB |
| mui-docs (`docs`), default | 364 MB | 77 MB (21.2%) | 62 MB | 9 MB |

The opt-out row is the control: ~0.9 GB more unused-but-reachable memory in the eager member tables, the waste
#64475/#64526 removed. With them on, **about 13% of what the checker allocates is reachable and never read; there is
no second pool of that size**. The heap outside the arena is not in these numbers.

Ranked pools (unused and reachable, MB: private monorepo / vscode / mui-docs):

1. Union/intersection properties synthesized by `getReducedType` -> `somePropertyReducesToNever`: 61.6 / 1.6 / 10.6.
   Not deferrable exactly: this is B1, rejected in notes/mem-round3.md (changes results).
2. Expression re-checks under contextual types (overload resolution and inference): 42.8 / 23.7 / 14.1. Escapes
   through value-keyed caches.
3. Object-type instantiation of instantiated types whose members are never resolved: 29.9 / 8.6 / 1.5. The mapper is
   the type.
4. Instantiated signatures of anonymous types (`resolveAnonymousTypeMembers` instantiates every signature): 28.9 / 2.9
   / 0.5. Only with a lazy table for anonymous instantiations or lazy `Signature.parameters` (rejected in mem-lazy).
5. Mapped type members (full resolutions): 24.8 / 1.5 / 16.2. Partly L4 (rejected).
6. Line maps of every file, only for the `Lines:` counter of `--diagnostics`: 21.6 / 10.6 / 2.4. Not a checker item.
7. Union/intersection properties, other askers: 21.3 / 2.9 / 10.8. As 1.
8. Lazy tables resolved in full: 14.7 / 3.1 / 0.8. Callers iterate all members.
9. Base types resolved in full inside `resolveLazyMembers`: 14.7 / 1.9 / 1.0. U2.
10. Lazy members instantiated by discriminant matching in `getUnmatchedProperties`: 10.2 / 3.8 / 0.1. U1.

Smaller rows: type alias records never printed (2.4 MB), `SymbolNodeLinks` records never written (363K records,
2.8 MB; tsrs-only, a `try_get` like mem-round3 A5), string literal texts copied by `getStringLiteralType` (12.8 MB,
all read; Go shares the caller's string, tsrs copies: a representation item).

Pools 1-5 and 7 are 210 MB of the 325 MB. None of them is a laziness candidate that keeps type creation order: the
unread objects are types (or hang off types) whose creation order assigns the ids that union ordering and printing
depend on, or they are created by resolutions whose other members are read. After #64475/#64526 and L1-L11, the
remaining never-read memory is mostly *types*, and deferring types is what B1 showed to be unsafe.

## U1 and U2 (built on the branch, not landed)

- **U1**, discriminant matching looks the source property up only when its type is needed: in
  `getUnmatchedPropertiesWorker(..., matchDiscriminantProperties=true)` the source is asked `hasPropertyOfType` first
  and `getPropertyOfType` only for unit target types. 2,855,500 source lookups avoided; symbols -2.7% single, -2.1% on
  4 checkers; types, instantiations and diagnostics unchanged.
- **U2**, a table resolved in full inherits from lazy bases without resolving them (walks the base's L10 list instead
  of `getPropertiesOfType(base)`): lazy tables resolved in full 133,092 -> 86,506; symbols (on top of U1) 12,903,804 -> 12,885,417 single,
  16,947,825 -> 16,920,105 on 4 checkers. Smaller than pool 9 suggested: most base members are inherited (kept) by
  the derived table, so they are created either way; U2 saves the overridden members and the bases' own member tables.

Measured together (private monorepo, 3 interleaved rounds): peak 5.729 -> 5.700 GiB (-0.5%) at 1 checker and 7.574 ->
7.531 GiB (-0.6%) at 4, instructions within the +-2% run-to-run spread. On the app projects their pools are 0.1-0.4%
of the per-checker growth (notes/mem-never-read-apps.md, P5 and P9).
