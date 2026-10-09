# open-history-dependence: three cases whose output still depends on the checker assignment (not fixed), and one fixed

notes/perf-order-independence.md made diagnostics a function of the program on the corpora it tested, and the README
says output does not depend on the checker count. Three bench candidates show that this does not hold in general:
TanStack/router and sequelize (from the bun-check project scouts) and rxjs (found by the excalidraw work, PR #212). All
three are typescript-go's own history dependence: tsgo-ref also prints different output at different checker counts, or
tsrs prints the same output under `--checkerAssignment go` when the files are visited in the same order. None of them is
fixed. For the router case a fix that makes the output canonical exists, but it costs up to 4% of single-threaded
instructions and adds an error that tsgo does not print to another program (below); the cheaper variant leaves part of
the dependence. A fourth case, a TS2590 reported on a private monorepo (issue #218), was reproduced by the reporter and
is fixed: section 4 keeps the mechanism and the measurements. This note is the evidence and the measurements.

## Summary

| project @ commit (`-p`) | tsrs 1 / 4 / 8 / 16 checkers | tsrs `random:1..5` at 4 | tsrs Go mode at 4 | tsgo-ref 1 / 4 / 8 / 16 |
| --- | --- | --- | --- | --- |
| TanStack/router `663282b0ebbc` (`packages/react-router/tsconfig.bench.json`, overlay) | 5 / 7 / 7 / 7 errors | 7 each | 5 | 5 / 5 / 5 / 5 |
| sequelize `d191d5be03ad` (`tsconfig.bench.json`, overlay) | 42 / 43 / 43 / 43 | 43 each | 41 | 42 / 41 / 41 / 42 |
| rxjs `54796b38a57e` (`packages/rxjs/tsconfig.bench.json`, overlay) | 6,158 each, 5 with different elaboration text | `random:1..3` at 2 and 4 differ | | text differs between 1 and 4 |

tsrs is main at 990d32d5 on the Mac; tsgo-ref is `typescript@7.1.0-dev.20260930.4`. The overlays are the scouts'
`tsconfig.bench.json` files (bench PR #206 for rxjs). Single-threaded tsrs equals tsgo-ref at one checker on all three.
`TSRS_LAZY_DTS=0`, `TSRS_FLOW_MEMO=0`, `TSRS_UNION_CACHE=0`, `TSRS_INFER_MEMO=0` and `TSRS_LAZY_MEMBERS=0` change none
of the router and sequelize counts at 4 checkers, so no tsrs-only cache is involved.

## 1. TanStack/router: a base constraint cut short in one file is reused in another

At 4, 8 and 16 checkers tsrs adds two errors that single-threaded tsrs and tsgo-ref do not print:

```
packages/router-core/src/link.ts(519,13): error TS2536: Type '"types"' cannot be used to index type 'ResolveRoute<TRouter, TFrom, TTo, ResolveRelativePath<TFrom, TTo>>'.
packages/router-core/src/link.ts(519,13): error TS2536: Type 'ResolveToParamType<TParamVariant>' cannot be used to index type 'ResolveRoute<TRouter, TFrom, TTo, ResolveRelativePath<TFrom, TTo>>["types"]'.
```

Some random assignments also add `useMatch.tsx(151,29)` and `(153,26)`: "Property 'stores' / 'isServer' does not exist
on type 'TRouter'" (9 errors). A debug build that visits chosen files first (not landed) reduced the trigger to one
file: the two TS2536 appear exactly when `packages/react-router/tests/useNavigate.test.tsx` is checked before
`packages/router-core/src/link.ts` on the same checker, in the default mode and under `--checkerAssignment go` alike
(single-threaded, that order prints 7 errors in both). tsgo-ref prints 5 at every count only because its FENNEL
assignment never puts the test file first on link.ts's checker.

The cause is `getResolvedBaseConstraint` (checker.go:27917; `get_resolved_base_constraint`, checker_13.rs). It explores
ten levels of nested constraints, then stops at a type whose recursion identity is already on the stack, and caches the
result for the type whatever the stack was, `noConstraintType` included. `getRecursionIdentityTarget` gives an indexed
access the identity of its object type and a type parameter's identity is its symbol, so a chain like
`TRouter["routeTree"]["types"]...` carries `TRouter`'s identity. While inferring a call in the test file, the checker
resolves such a chain from the top (`getConstraintOfIndexedAccess` -> `hasNonCircularBaseConstraint`); ten levels down
it reaches `TRouter` itself, finds its identity on the stack and stops. `TRouter`'s base constraint is cached as
`noConstraintType`, and so are the types computed above it. A trace of cache hits (not landed) shows link.ts reading 34
such entries when the test file came first, among them `TRouter`, `InferFileRouteTypes<TRouter["routeTree"]>`,
`RoutesByPath<TRouter["routeTree"]>` and `ResolveRoute<TRouter, TFrom, TTo, ...>`, 89 of the reads returning
`noConstraintType`. The index check at link.ts(519,13) relates `"types"` to `keyof ResolveRoute<...>` through that
constraint, and fails.

A two-file reduction and the write-up for typescript-go are in upstream/determinism-base-constraint-depth.md: tsgo-ref
prints a TS2339 that the program does not have at 1, 2 and 4 checkers and not at 3, 8 and 16; tsrs prints it
single-threaded and not at 4 checkers. TypeScript 5.9.3 prints nothing: its `getRecursionIdentity` gives an indexed
access the identity of its object type, not of the type parameter's symbol, so a chain never cuts the type parameter
itself (it caches cut results the same way, though).

### Fixes measured and rejected

Instructions are the minimum of 5 interleaved single-threaded runs on the Mac (3 for type-fest, mui-docs and
t3code-server), main 990d32d5 against the experiment; "computations" counts the base constraints computed (not taken
from the cache), single-threaded. Single-threaded output was byte-identical to main on every project in the table.

1. **Exact** (branch `exp/canonical-base-constraints`, commit 5f62f788, not for merge). A cached base constraint is
   reused only where computing it again could not cut anything: a result computed at the top of a frame (an empty stack)
   is reused at the top, and a result in which nothing was cut is reused while its nested levels stay within the ten the
   guard always explores. Everything else is computed again; cut results below the top of a frame are kept per (frame,
   type, depth) for the rest of that frame. `--checkerAssignment go` keeps Go's cache. Router prints its 5 errors
   single-threaded, at 4, 8 and 16 checkers and under `random:1..8` at 1, 2 and 4 checkers; the reduction prints nothing
   at any count.

   | project | instructions | base constraint computations |
   | --- | ---: | ---: |
   | router | +1.48% | 40k -> 160k |
   | sequelize | +4.08% | 80k -> 450k |
   | type-fest | +0.43% | 80k -> 300k |
   | xstate | +0.36% | 25k -> 35k |
   | webpack | -0.04% | 25k -> 40k |
   | mui-docs | +0.17% | 25k -> 25k |
   | t3code-server | +0.10% | 135k -> 135k |
   | cal-diy | not measured cleanly | 190k -> 780k |
   | formbricks-web | | 140k -> 160k |
   | supabase-studio | | 90k -> 100k |
   | vscode | | 85k -> 85k |

   Peak memory single-threaded: +8 to +9 MiB on sequelize, type-fest and cal-diy (the per-type heights). Rejected for
   the cost, and because its answers are the "cold" ones: a chain requested from the top is always cut. This program
   alone reports TS2339 on `y.b`, which tsgo and main do not print (checking the parameter's type node resolves
   `U["a"]`, then `U["a"]["a"]`, and so on, each from the top, so no request is ever ten levels deep):

   ```ts
   interface Box { a: Box; b: string }
   function ten<U extends Box>(y: U["a"]["a"]["a"]["a"]["a"]["a"]["a"]["a"]["a"]["a"]) { return y.b; }
   ```

   About 90% of the requests it computes again are uncut results requested deeper than their height allows (sequelize
   241k of 266k, cal-diy 188k of 208k), and 87-89% of those compute the same result again without a cut. Keeping a
   summary per result (height, deepest repeated identity, a 64-bit Bloom filter of the identities it visits) to reuse
   those when the caller's stack cannot cut them removed only 6-12% of the computations (sequelize 450k -> 395k, cal-diy
   780k -> 705k): most of them involve an identity that does repeat.

2. **Cut results below the top not cached** (results reused at any depth, cut results computed below the top of a frame
   kept only for that frame). Router and the reduction are fixed and the chain example prints nothing. It costs less but
   not little: sequelize +1.2 to +1.6%, cal-diy +1.4 to +2.3% (noisy), type-fest +0.2 to +0.4%, router +0.1%. And it is
   not canonical: a cut result computed at the top of a frame is still cached for every caller. With `return x.b + y.b;`
   in the reduction's `b.ts`, `y.b` on the ten-level chain errors when `a.ts` was checked first (`a.ts` resolved the
   chain from the top and the cut was cached) and not when the files are on different checkers.

3. **Not tried: each type's own top-level constraint.** A nested request for a type that has no cached constraint would
   compute it from the top (an empty stack) first and use that. This is the answer checking in program order usually
   gives, because declarations resolve their type nodes inner levels first, and it computes about what Go computes. But
   the depth guard could then no longer stop constraints that instantiate new types at every level: it stops at a
   repeated identity, and a finite chain like the one above repeats identities too. It needs a different limit (for
   example on nested top-level computations), and the results for such expanding constraints would change. That is a
   redesign of `getResolvedBaseConstraint`, not a cache fix; it is the direction to take if this is fixed.

## 2. sequelize: a circular property type entered from different places

Single-threaded, tsrs and tsgo-ref print 42 errors, including

```
packages/core/src/sequelize-typescript.ts(335,12): error TS7022: 'models' implicitly has type 'any' because it does not have a type annotation and is referenced directly or indirectly in its own initializer.
```

tsgo-ref drops it at 4 and 8 checkers (41) and keeps it at 16 (42). tsrs keeps it at every count, and at 4, 8 and 16
checkers and under every random assignment tried (`random:1..6` at 1, 2 and 4 checkers) it also prints

```
packages/core/src/associations/belongs-to-many.ts(942,13): error TS2347: Untyped function calls may not accept type arguments.
```

The property is `readonly models = new ModelSetView<Dialect>(this, this.#models);` in `SequelizeTypeScript`. Resolving
its type resolves the `new` call, which checks the type argument against `ModelSetView`'s constraint. That relates
instantiations of `AbstractDialect`, whose variances are computed by comparing its members, among them the `sequelize`
property of type `Sequelize<this>`, which needs `Sequelize`'s variances, which read `models` again. `getVariancesWorker`
(relater.go:1334) opens a new resolution window (`resolutionStart`) when no variance computation is in progress, and on
a variance circularity it restarts from the smallest symbol with an empty variance stack (relater.go:1411);
`getResolvedSignature` (checker.go:8581) opens one too. Each window lets `models` be pushed once more: a trace (not
landed) of the program-order run shows it pushed four times while checking belongs-to-many.ts, with the variance stack
`[AbstractDialect, Sequelize, SequelizeCoreOptions, PoolOptions, Connection, AbstractConnectionManager]` at the second
push and `[PoolOptions, AbstractConnectionManager, AbstractDialect, Sequelize]` at the fourth, which is circular. The
circularity reports TS7022 and caches `any` as the symbol's type, while the outermost resolution still returns
`ModelSetView<Dialect>` to its caller.

Two things depend on history:

- Whether the cycle is met at all: if the variances it needs were already computed when `models` is first resolved, the
  relation uses them and `models` resolves without the cycle. With belongs-to-many.ts alone on one checker and every
  other file in program order on another, neither error is printed (41).
- Which reader gets the `any`. In program order belongs-to-many.ts(941) (`sequelize.models.hasByName(...)`) is the
  outermost resolution: it gets the real type, and the instantiated `models` symbol it caches is the one line 942 reads.
  With `test/unit/decorators/attribute-validator.test.ts` (`sequelize.addModels([User])`) checked first on the same
  checker, the cycle is entered there, line 942's symbol is resolved after the cache holds `any`, and
  `getOrThrow<M>(...)` on `any` is TS2347. Single-threaded with that test file visited first, tsrs prints the TS2347 in
  the default mode and under `--checkerAssignment go` (43 errors).

Not fixed: making it canonical needs every variance a property type depends on to be computed before that property type
(no such order exists in general), or the circularity to be reported and typed the same wherever it is entered. Both
change how `getVariancesWorker` and the resolution stack break cycles.

## 3. rxjs: which elaboration the relater reports

From notes/perf-excalidraw-typefest.md (PR #212): rxjs prints its 6,158 errors in every configuration, but five of them
(`group-by.spec.ts(24,41)`, `multicast.spec.ts(33,19)`, `multicast.spec.ts(184,77)`, `concat-all.spec.ts(36,48)`,
`merge-all.spec.ts(45,48)`) elaborate differently, for example "The types returned by '[bufferTime](...)[groupBy]'"
against "'[bufferTime](...)[bufferTime](...)[groupBy]'". On main, `random:1`, `random:2` and `random:3` at 2 and 4
checkers each differ from the single-threaded output; at one checker in a random visit order they do not, so it depends
on which files share a checker. tsgo-ref also prints different text at 1 and 4 checkers. #212 traces it to the relation
cache: which elaboration path the relater reports depends on what the cache already holds. Not investigated further
here; the base constraint experiments above leave its output byte-identical to main in all of these runs.

## Regression cases

`union-too-complex-cross-product` (section 4, the fixed case, from the reporter's PR #228) and
`union-too-complex-canonical-order` (section 4, what the case is not). `tools/ci/determinism.sh` compares every
`testdata/regressions` case at 2 and 4 checkers and under random assignments with its single-threaded output, so a case
that still depends on the assignment would fail it. The router reduction is in
upstream/determinism-base-constraint-depth.md, ready to become a case with whichever fix lands.

## Reproduce

```sh
# router: overlay packages/react-router/tsconfig.bench.json from the scout, pnpm install --frozen-lockfile --ignore-scripts
tsrs -p packages/react-router/tsconfig.bench.json --noEmit --incremental false --pretty false --singleThreaded   # 5
tsrs -p packages/react-router/tsconfig.bench.json --noEmit --incremental false --pretty false --checkers 4        # 7
tsrs ... --checkers 4 --checkerAssignment go                                                                     # 5
# sequelize: overlay tsconfig.bench.json, yarn 4 install --mode=skip-build
tsrs -p tsconfig.bench.json --noEmit --incremental false --pretty false --singleThreaded                         # 42
tsrs -p tsconfig.bench.json --noEmit --incremental false --pretty false --checkers 4                             # 43
# rxjs: overlay from bench PR #206
tsrs -p packages/rxjs/tsconfig.bench.json --noEmit --incremental false --pretty false --checkers 2 --checkerAssignment random:1
```

## 4. TS2590 reported only by the first evaluation on a checker (issue #218, fixed)

Reported on a private monorepo (Linux x64, 0.7.0 and 0.9.0): at 8 checkers in the default mode a TS2590 ("Expression
produces a union type that is too complex to represent") was printed in some runs and not in others on the same tree;
on another commit the default mode never printed a TS2590 that tsgo printed at 4 and 8 checkers but not at 1, and tsrs
under `--checkerAssignment go` at 4 checkers printed exactly tsgo's output. The reporter traced it with
`TSRS_TRACE_UNION_REDUCTION` (the discussion on PR #219) and reduced it to a generated project (PR #228), whose
three-file core is now `testdata/regressions/union-too-complex-cross-product`.

What it is: the write constraint of `ModelMap[T]`, needed when an arrow function is related to a generic callable, is
the intersection of three unions of 50 object types, a cross product of 125,000 constituents. `check_cross_product_union`
(checker_13.rs; Go `checkCrossProductUnion`) reports TS2590 at `current_node` when the product reaches 100,000, and
`get_intersection_type_ex` returns the error type. Neither TS2590 site caches its own failure (`remove_subtypes` inserts
into `subtype_reduction_cache` only on success), but Go caches what it builds from the error type, one level up: when
an intersection of three or more constituents is split in halves, the split's caller stores the error type under the
whole intersection's key (checker_13.rs `intersection_types`; Go checker.go:26658 and :26690); a union of two unions
stores it in `union_of_union_types`; alias, object and conditional type instantiations store it in their instantiation
maps; and a comparison through a constraint that became the error type succeeds and is recorded in the relation cache
(relater_2.rs `reset_maybe_stack`, or `Failed` for a failure). So a checker reports the error only the first time it
evaluates the type. In the reproduction `x/a.ts` evaluates it first under `// @ts-ignore` and `x/zz.ts` evaluates it
again: a checker that checked `a.ts` prints nothing at `zz.ts`, a checker that did not prints TS2590 there. With work
stealing the two files share a checker in some runs and not in others (15 of 30 identical runs at 2 checkers reported
it on the generated 1,600-file project, 0 of 30 with `--checkerAssignment locality`), and under `random:<seed>`
assignments seeds 1, 3, 4 and 6 printed it and 2 and 5 did not. On the three-file case the reporter found that
disabling the intersection cache alone still leaves `zz.ts` silent, through the relation cache. tsgo has the same
history dependence for a fixed assignment: tsgo-ref prints nothing at 1 and 2 checkers and `x/zz.ts(5,14)` at 3.

What it is not: the creation order of the union's constituents (`compareTypes` orders them canonically; the regression
case `union-too-complex-canonical-order` creates the same 1,101 classes in opposite orders in two files and prints the
same TS2590 under every assignment, as tsgo-ref does, where TypeScript 5.9.3 prints it only when a.ts is checked
first), and not a diagnostic filed against another file (that channel exists and Go behaves the same: the trace shows
it on webpack, a TS2590 filed `at test/fixtures/acorn-corpus.json(3,9) while checking tooling/compare-js-tools.js` in
every run and printed in none, because JSON files are not type-checked and their diagnostics are never collected;
tsgo-ref does not print it either).

The fix: `Checker::too_complex_reports` counts TS2590 reports, and `too_complex_since(mark)` says whether a computation
that began at `mark` reported one. In the default mode such a result is not cached: `get_intersection_type_ex`,
`union_of_union_types`, the three instantiation caches and the relation cache skip their insert, so every evaluation
computes the type again. The reports are collected while a file is checked and emitted when it is done, and a checker
remembers which (file, evaluation) pairs it has reported (`too_complex_reported`), so each file reports a too-complex
type once, at the first site in it that evaluated the type, whichever file's check that evaluation happened in
(`report_too_complex`, `flush_too_complex_reports`). The evaluation's identity (`too_complex_key`) is the caller's
cache key where it has one, the alias-qualified intersection key, so that an aliased and an alias-free evaluation of
the same intersection report separately, as Go's alias-qualified cache makes them; for a union reduction it is the
union's constituents. That is the site Go's caches give within one file, since Go reports the first evaluation per
checker: `compiler/normalizedIntersectionTooComplex` reports once, at the arrow parameter, not also at the call and
its argument; a call whose parameter type is resolved before its argument reports at the call, as tsgo does; two
different too-complex types in one expression both report. (Two earlier revisions, the innermost of nested sites and
a first-site memory emptied at each flush, were refuted by the adversarial review of the fix: the first dropped a
second type and moved the call case, the second reported a file's second site when its first had been reached from
another file's check.) The output no longer depends on the assignment:
`x/zz.ts(5,14)`
single-threaded, at 1-4 checkers and under `random:1..8` (crates/tsrs_cli/tests/union_too_complex_cross_product.rs;
`tools/ci/determinism.sh` sweeps the case too), and 30 of 30 identical runs on the generated project. It differs from
tsgo where tsgo's own output depends on the checker count: tsgo prints the error at a site only when that site's
checker evaluates the type first. `--checkerAssignment go` keeps Go's caches and output (nothing single-threaded on the
case, as tsgo-ref). The four conformance tests that contain a TS2590 (templateLiteralTypes1,
unionSubtypeReductionErrors, normalizedIntersectionTooComplex, templateLiteralTypeTooComplex) are unchanged in both
modes.

Still open, unchanged by the fix: a type cached on a declaration rather than keyed by type (a symbol's or node's
resolved type, a type parameter's constraint, a lazily resolved instantiated member) carries the error type to every
later use without re-evaluation, so a use evaluates the type, and reports, only if its checker has not resolved the
declaration before. When the declaring file is checked, the report at the declaration is deterministic, but a use in
another file reports in addition only under some assignments (the review's cases c1, c2, c9 and c11: under
`random:1..4` at 2 checkers the use site reports or not, or the using file's one report moves between two of its
sites). When the declaring file is an unchecked declaration file (`skipLibCheck`), the report at the declaration is
never printed either: a `declare const v: K0 & K1 & K2`, a `Cond<X>` alias whose true branch is the intersection, a
`T extends K0 & K1 & K2` constraint and a lazily resolved instantiated member, each in a `.d.ts`, are each silent at
the use when the using file shares a checker with an earlier user, identical before and after the fix. Closing these
would mean keeping poisoned results out of the symbol and node links as well (resolved and declared types, constraints,
instantiated members), which is a larger change. Also unchanged: a report filed against another file while checking
this one is kept only if this checker checks that file later (as every diagnostic), and a checked declaration file
split into pieces (`TSRS_SPLIT_FILES`) can report one site per piece. Cost: a program that reports a TS2590 recomputes the type at every site; for the cross product this is the size
check only, for a union of 1,101 classes each re-evaluation pays the 100,000-comparison estimate, about 4 ms per site
(100 sites in one file: 0.46 s against 0.05 s). `TSRS_TRACE_UNION_REDUCTION=1` (docs/DEBUGGING.md) remains, for the
`remove_subtypes` site, for Go mode, and to find which union a TS2590 is about.
