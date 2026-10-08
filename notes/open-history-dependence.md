# open-history-dependence: three programs whose output still depends on the checker assignment (not fixed)

notes/perf-order-independence.md made diagnostics a function of the program on the corpora it tested, and the README
says output does not depend on the checker count. Three bench candidates show that this does not hold in general:
TanStack/router and sequelize (from the bun-check project scouts) and rxjs (found by the excalidraw work, PR #212). All
three are typescript-go's own history dependence: tsgo-ref also prints different output at different checker counts, or
tsrs prints the same output under `--checkerAssignment go` when the files are visited in the same order. None of them is
fixed. For the router case a fix that makes the output canonical exists, but it costs up to 4% of single-threaded
instructions and adds an error that tsgo does not print to another program (below); the cheaper variant leaves part of
the dependence. This note is the evidence and the measurements.

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

None added: `tools/ci/determinism.sh` compares every `testdata/regressions` case at 2 and 4 checkers and under random
assignments with its single-threaded output, so a case that still depends on the assignment would fail it. The router
reduction is in upstream/determinism-base-constraint-depth.md, ready to become a case with whichever fix lands.

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
