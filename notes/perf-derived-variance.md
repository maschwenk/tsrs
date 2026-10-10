# perf-derived-variance: relating a derived generic to its generic base by variances

> Removed on 2026-10-07: the mechanism shipped switched off and, guarded, saved nothing (the judgement in
> notes/fuzz-derived-variance.md), so `relater_derived.rs`, its seven `TSRS_DERIVED_VARIANCE*` switches, the fuzzers under
> tools/fuzz and the `derived_variance` CLI test were deleted. The `testdata/regressions/derived-variance-*` cases stay:
> they are tsgo's output on the shapes the fuzzer found.

Status (2026-10-10): removed in #194 (docs/STATUS.md); condensed to the measured results. The savings below are for guards 1-3 only. An adversarial
generator later found nine more kinds of disagreement; guards 4-6 closed them and removed the savings (guarded:
-1.8% / -0.3% check time at 1 / 8 checkers). notes/fuzz-derived-variance.md has those findings, numbers and the
verdict. The switches and the shadow mode named below no longer exist.

Row 2 of the work census (notes/perf-checker-algorithms.md): on the 38k-file codebase 10-17% of check time is
structural comparison of derived generic instances against references to their generic bases (`ZodObject<...>`,
`ZodString` against `ZodType<any, any, any>`; ORM repositories against `EntityRepository<T>`). TypeScript relates two
references to the same generic by the generic's variances, but never across a derivation, so these are compared
member by member: every method of the base, with generic signatures and `this` types, for every derived instance.
`TSRS_DERIVED_VARIANCE` tried the variance route across the derivation.

## What it did

In `structuredTypeRelatedTo(source, target)` (any relation but identity, no error reporting), with a target that is a
reference to a generic class or interface `G` and a source that is a reference to a different class or interface: find
the reference `R` to `G` in the source's base-type chain (instantiated as `resolveObjectTypeMembers` builds inherited
members), relate `R`'s type arguments to the target's by `G`'s variances, require the guards below, then count each
inherited-unchanged member as related and compare only the members the source declares, plus call, construct and index
signatures. It only ever added True answers. What it trusted is that "R's arguments relate by G's variances" implies
"each inherited member of R relates to the same member of the target", which TypeScript assumes whenever both sides are
references to `G`, but never applies across a derivation.

## The guards, each found by a shadow-mode disagreement

| guard | what went wrong without it | case |
| --- | --- | --- |
| the variance of `this` is measured (like a type parameter: references whose `this` argument is the sub / super marker) and must be covariant, bivariant or independent | `Runtype<A>` declares `constraint: Constraint<this>` and `Constraint` is invariant; `Num extends Runtype<number>` against `Runtype<any>` relates by variances (`number` -> `any`) but not structurally (`Constraint<Num>` vs `Constraint<Runtype<any>>`). The conformance test `invariantGenericErrorElaboration` lost both TS2322 errors with the switch on | `testdata/regressions/derived-variance-this-type` |
| no decisions while a variance computation runs | mui-docs (DataGrid column definitions): comparisons inside a variance measurement, with marker type arguments; the variance answer and the structural one differed for 6 pairs (no diagnostic changed) | covered by the mui-docs shadow run |
| an `any` argument of `R` falls back when its parameter reaches the check type of a conditional type in `G`'s declarations (directly, through a conditional type alias, or through a base type's argument) | playwright's `JSHandle<T>.asElement(): T extends Node ? ElementHandle<T> : null`: `any` takes both branches where the measuring markers kept the conditional deferred. tsgo accepts `JSHandle<any>` -> `JSHandle<unknown>` by variances but rejects `ElementHandle<any>` -> `JSHandle<unknown>` structurally; with the switch on that error was lost | `testdata/regressions/derived-variance-any-conditional` |

## Shadow mode: decisions and disagreements (guards 1-3)

Shadow mode computed both answers for every eligible pair and reported disagreements (exit 7).

| run | decisions | disagreements | normal-comparison time of the decided pairs (outermost) |
| --- | --- | --- | --- |
| conformance suite, default / `TSRS_LAZY_MEMBERS=0` | 661 / 661 | 0 / 0 | 6 / 7 ms |
| 38k-file codebase, one checker | 39,907 over 62 generic bases | 0 | 2.50 s |
| 38k-file codebase, error-rich clone (10,781 errors) | 35,510 | 0 | 2.23 s |
| vscode / webpack / mui-docs / xstate | 2,671 / 470 / 358 / 91 | 0 | 20 / 2 / 1.5 / 0.2 ms |

(Before the guards: 1 disagreement in the suite, 1 on the 38k-file codebase, 6 on mui-docs.) In shadow mode the
result baselines of the suite are identical to the switch being off.

On the 38k-file codebase the decided time is concentrated: `ZodType` (zod v4 classic) 1.94 s, `EntityRepository`
0.46 s, everything else (60 bases) about 0.1 s. Variances: `ZodType`'s type parameters are declared `out` (no
measurement, reliable), its `this` measures Covariant|Unreliable; `EntityRepository`'s parameter measures
Invariant|Unreliable.

## Savings with the switch on

The 38k-file codebase, paired medians of 3 interleaved rounds:

| variant | instructions, 1 checker | check s, 1 checker | peak, 1 checker | instructions, 4 checkers | check s, 4 checkers | peak, 4 checkers |
| --- | --- | --- | --- | --- | --- | --- |
| off | 274.66 G | 14.32 | 4.199 GiB | 371.41 G | 5.89 | 5.658 GiB |
| on, all bases | -7.97% | -8.3% | -8.7% | -12.9% | -14.3% | -15.0% |
| on, type-parameter variances reliable (`=params`) | -6.12% | -10.2% | -7.8% | -8.4% | -10.8% | -11.8% |
| on, also the `this` variance reliable (`=1`) | +1.0% | +0.3% | +0.7% | +0.5% | | |

Types 9.63M -> 7.85M and instantiations 44.8M -> 37.4M with one checker (on, all bases): the skipped comparisons
instantiate the members of every derived schema. The `=1` variant loses because it excludes `ZodType` (its `this` is
Unreliable) and still pays for measuring `this` variances.

With the default threshold (targets with 16 or more properties; 11,774 decisions instead of 39,907, the same 0
disagreements), the 38k-file codebase, paired medians of 3 rounds, load 13-45 (check times noisy, instructions not):

| variant | default (8 checkers) instr / check / peak | 4 checkers instr / check / peak | single-threaded instr / check / peak |
| --- | --- | --- | --- |
| switch off vs main | -0.09% / +0.1% / +0.1%, counters identical | +0.35% / -2.4% / +0.1% | -0.10% / +3.0% / 0, counters identical |
| on, all bases | -16.8% / -18.5% / -18.3% (7.24 -> 5.91 GiB) | -12.9% / -14.0% / -14.9% | -8.4% / -11.0% / -9.2% |
| on, type-parameter variances reliable | -9.6% / -12.0% / -13.9% | -8.6% / -9.0% / -12.0% | -7.0% / -0.4% / -8.3% |
| shadow | +1.3% / +5.9% / +1.0% | +0.8% / +0.8% / +0.5% | +0.4% / +1.2% / +0.2% |

Split by base (on, one base at a time, before the threshold; instructions): `ZodType` alone -6.7% single-threaded /
-10.0% at 8 checkers; `EntityRepository` alone -0.7% / -6.4%; the other 60 bases together +0.9% / +1.1% (cheap
comparisons where measuring variances costs more than it saves, which is what the 16-property threshold removes:
it keeps 2.40 of the 2.50 s of decided relater time). So it applies to codebases with large derived generic
families (schema libraries built from classes, ORM repositories), not to ordinary class hierarchies.

The other four corpora have no such hierarchies: with the switch on, instructions move by -1.4% to +0.8% (paired
medians, inside their run-to-run spread), and their diagnostics are byte-identical.

The `=params` restriction bought no measured safety: for a True answer TypeScript itself accepts the variance result
for two references to the same generic whatever the reliability flags (the flags only allow a structural fallback
after a False), so "type-parameter variances reliable" is not a trust boundary TypeScript draws for True answers; it
gave up `EntityRepository` (Invariant|Unreliable) and half the 8-checker win, and both variants had 0 disagreements.

## What it changed beyond answers (applies to any relation shortcut)

- **Counters and budgets.** Fewer relation checks run, so fewer types and instantiations are created and fewer
  relation-cache entries are recorded. The instantiation count of a statement (TS2589 at 5M) and the relation-cache
  size (the TS2859 budget, `relationCount = (16M - size) / 8`) are therefore lower than in Go: for a program at
  either limit, the error can fire later than in Go, or not at all.
- **Type ids.** Types are created in a different order (some not at all), so ids shift; printed union order could in
  principle change. No printed type changed: the suite with the switch on (canonical history, both lazy modes) has
  identical baselines, and the five corpora and the error-rich clone print byte-identical diagnostics at one and four
  checkers.
- **Answers.** Budget exhaustion is not the only possible difference: the switch extends TypeScript's trust in its
  variance digest across a derivation, and the digest is an approximation (markers instead of real arguments). The
  guards close every disagreement found, on 81,000 decisions (the fuzzer later found nine more kinds; see
  notes/fuzz-derived-variance.md); a digest error not covered by them would answer True
  where the member-by-member comparison answers False, so tsrs would miss an error that Go reports. It would never
  report an error that Go does not.
