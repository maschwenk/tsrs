# perf-order-independence: output that does not depend on which checker checks what

With several checkers, tsrs (like tsgo) printed different text depending on how files were split over checkers. On
the 40k-error corpus (the 38k-file codebase in a Linux sandbox, workspace packages unbuilt), `--checkers 1, 2, 4, 8, 12,
16` gave four different outputs. Since 2026-10-04 the default checker count depends on the machine, so two machines
could print different text for the same program. The same dependence rules out dynamic scheduling, where a file's
checker would depend on timing. This note finds the cause, fixes it, makes the old behaviour available for byte-identity
with tsgo, and checks the result with random assignments.

## The diffs

Six thread-mode runs (N = 1, 2, 4, 8, 12, 16) and `random:<seed>` runs on the 40k-error corpus. Every difference is
the position of a filled-in `name?: undefined` property in a widened object-literal union. There are 2-6 lines per pair,
in three files (two router endpoints, one test). For example:

```
N=1:  ... => Promise<{ user?: undefined; status: "expired" | "invalid-otp"; csrfToken?: undefined; ... }
N=12: ... => Promise<{ status: "expired" | "invalid-otp"; csrfToken?: undefined; expiresAt?: undefined; role?: undefined; user?: undefined; ... }
```

The Mac error-rich clone (10,781 errors), webpack, vscode, mui-docs and xstate were already identical for N = 1-16 and
under random assignments. Their programs do not hit the case.

## The cause

`getUndefinedProperty` (checker.go:18837) caches the `x?: undefined` symbol per property **name** for the whole
checker. It copies the declarations of the first property named `x` that the checker ever widened.
`get_named_members` sorts a widened literal's properties by first declaration (`compare_symbols`: file index, then
position). So `x?: undefined` sorts at the declaration of whatever `x` that checker happened to see first. That can be
in another file, or in an earlier statement of the same file. A two-file repro (no import between the files) and the
one-line fix are in `upstream/determinism-undefined-property.md`: tsgo prints `{ user?: undefined; status: string; }`
for `b.ts` when `a.ts` is in the program and `{ status: string; user?: undefined; }` when it is not.

## The fix and the compatibility mode

- **Default (canonical):** the cache is keyed by the sibling property the filled-in property stands for
  (`undefined_properties_by_prop`). The property then always sorts by the declaration it represents, so output is a
  function of the program alone.
- **`--checkerAssignment go` / `TSRS_CHECKER_ASSIGNMENT=go`:** Go's per-name cache, bug for bug. Every such
  two-behaviour place asks `tsrs_core::compat::go_compatible_history()`; this cache is the only one.
- The tsgo-baseline harnesses (`tsrs-test`, `tsrs-fourslash`, tsctests) call `use_go_history_for_tsgo_baselines()`.
  Their baselines are tsgo's output with one checker in program order, so they compare in Go's mode unless
  `TSRS_CHECKER_ASSIGNMENT` names another assignment. `tools/oracle/emit/monorepo.sh` already defaults to `go`.
- `TSRS_CHECKER_ASSIGNMENT=random:<seed>` (debug) assigns each file to a random checker and gives every checker a random
  visit order, both drawn from the seed.

## How far default output now differs from tsgo

| what | default (canonical) vs tsgo / Go mode |
| --- | --- |
| conformance suite, Go mode (the harness default) | identical: 13,458 / 12,779 / 12,779; `.js` 13,392, `.js.map` 149, `.sourcemap.txt` 156 |
| conformance suite, canonical (`TSRS_CHECKER_ASSIGNMENT=locality tsrs-test ...`) | 1 test differs by design: `conformance/objectLiteralNormalization` `.types`, `.symbols` and `.js` (its `.d.ts`); error baselines and maps unchanged |
| diagnostics: Mac error-rich clone, vscode, webpack, mui-docs, xstate | 0 lines differ (canonical vs Go mode with one checker, which is tsgo's history) |
| diagnostics: 40k-error corpus | 5 lines vs Go mode with 1 or 8 checkers, 2 lines vs 12 (Go mode prints one of four variants depending on the count) |
| declaration output, monorepo emit oracle (101 packages, `--emitDeclarationOnly`) | 8 of 2,460 files differ from tsgo (3 packages); Go mode 2,460 / 2,460 identical |
| declaration output, vscode / mui-docs / webpack | 8 of 9,399 / 1 of 26,505 / 0 of 884 files differ |

All the declaration differences are the same class: the filled-in optional properties move to source order (for
example `previousLineText?: undefined` and `removeText?: undefined` in vscode's `onEnterRules.d.ts`). The default
locality assignment was never byte-identical to tsgo here either: docs/EMIT.md recorded 2 of the monorepo's 2,325
declaration files differing in property order, which is why the oracle uses `go`. Property order can also decide
which property an error names first ("Types of property 'x' are incompatible", "missing the following properties
...") whenever the relation walks the widened literal's properties in order. None of the corpora above had such a case:
every changed line was a printed type.

## Proof by random assignment (canonical mode)

| corpus | runs | identical |
| --- | --- | --- |
| Mac error-rich clone (10,781 errors, 11,761 lines): N ∈ {1,2,4,8,16} x 20 seeds | 100 | 100 |
| webpack `--declaration` (1,027 lines): N ∈ {1,2,4,8,16} x 20 seeds | 100 | 100 |
| 40k-error corpus (Linux, 43,859 lines): N ∈ {1,2,4,8,16} x 20 seeds | 100 | 100 |
| conformance, `TS_TEST_PROGRAM_SINGLE_THREADED=false` (4 checkers per test), random:1/2/3, types+symbols | 3 | result trees identical to canonical single-threaded |
| conformance, one checker, random visit order (random:4/5) | 2 | identical |
| conformance js/jsmap/sourcemap, 4 checkers, random:7 | 1 | identical |
| monorepo emit oracle, random:3 vs locality | 1 | 2,460 / 2,460 identical |

Before the fix the 40k-error corpus gave four distinct outputs over the same configurations.

## Other ways history can reach output (audit)

- **compare_types' type-id fallback** (utilities.go:414, the last line). An instrumented build counted the ties that
  reach it (TSRS_ORDER_AUDIT, not landed). On the clone, vscode, webpack and mui-docs they are:
  - fresh vs regular literal types of the same value (the regular one always exists first);
  - an object literal's fresh / regular / widened variants (same symbol, no mapper; the variant is made from the
    literal, so later);
  - `Array<...>` references whose arguments tie the same way;
  - type parameters without a symbol (inference and mapped-type clones).
  Every case pairs types that print the same, or whose relative creation order is fixed by construction (a variant is
  created from its source). Unions also drop a fresh literal whose regular type is present. So a tie can change printed
  text only if two such variants that print differently both survive in one union and were created in a
  history-dependent order. No run above found one. Not changed.
- **compare_symbols' symbol-id fallback** (only for symbols without declarations and with equal names): one hit on
  vscode, two module symbols for `@types/ws/index`, merged at checker creation in program order. Not
  history-dependent.
- **Circularity errors** (TS2456, TS7022/7023/7024, TS2502...): the error lands on the declaration whose resolution is
  re-entered, so a cycle entered from different files can report on a different declaration. This can occur. None of
  the error-rich corpora changed under 300+ random assignments.
- **TS2589 / TS2859 budgets** (instantiation depth and count, relation complexity): they count work that is not
  cached yet, so what is already cached can decide whether they fire. This can occur. Not observed under random
  assignments.
- **Diagnostics located in another file:** a checker returns a file's diagnostics right after checking it, so an
  error that checking file B reports inside file A is kept or lost depending on whether A was collected first. This
  can occur (Go behaves the same). Not observed.
- **Alias names of printed types:** unions, intersections and instantiations are interned with their alias in the key,
  so an alias is attached per request, not first-come. No history found.

So the assignment-dependent output observed on these corpora is gone. The remaining channels are theoretical here, and
each would show up under `random:<seed>`.

## What stays deterministic, what does not

- Diagnostics and emitted files: a function of the program alone in the default mode, for any `--checkers` and
  assignment (as tested above). In Go mode they depend on the assignment exactly as tsgo's do.
- `--extendedDiagnostics` Types / Symbols / Instantiations: per checker, so they still depend on the count and the
  assignment.
- Within a checker, files are still visited in program order. That is no longer load-bearing for output (random visit
  orders gave identical results) but keeps the counters stable.

## Reproduce

```sh
tsrs -p . --noEmit --incremental false --pretty false --checkers $N                                  # canonical
TSRS_CHECKER_ASSIGNMENT=random:$SEED tsrs -p . --noEmit --incremental false --pretty false --checkers $N
TSRS_CHECKER_ASSIGNMENT=go tsrs -p . --noEmit --incremental false --pretty false --checkers 1         # tsgo's history
TSRS_CHECKER_ASSIGNMENT=locality tsrs-test run --suite all --baselines types,symbols                  # canonical suite
TSGO=tsgo-ref TSRS_CHECKER_ASSIGNMENT=locality tools/oracle/emit/monorepo.sh <monorepo> -j 4 -- --emitDeclarationOnly --declarationMap false
```
