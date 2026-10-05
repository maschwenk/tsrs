# fuzz-derived-variance: attacking TSRS_DERIVED_VARIANCE

`TSRS_DERIVED_VARIANCE` (notes/perf-derived-variance.md, `crates/tsrs_checker/src/relater_derived.rs`) relates a derived
generic instance to a reference to its generic base by the base's variances instead of member by member. It is exact
only as far as TypeScript's variance digest agrees with the member-by-member comparison. Before this work: three
guards, zero disagreements on the conformance suite and five projects. This note: an adversarial generator, 15 more
projects, nine new kinds of disagreement (each one a lost error or a wrongly reduced union, confirmed against
tsgo), three new guards that close them, one open cosmetic difference, and what the guards cost.

**Verdict: do not turn it on by default.** The unguarded shortcut is wrong in many simple, ordinary shapes (`keyof T`,
a conditional on `this`, `T & {...}`, a wrong `out`), each giving up an error tsgo reports. The guards that make it
exact on everything found compare exactly the members that held the savings: on the 38k-file codebase the guarded
switch saves about 1% of instantiations and no check time (unguarded: 17% and 11%). See "Judgement" at the end.

## The generator

`tools/fuzz/derived_variance.py` (deterministic from a seed; `gen --seed S` writes one program, `run` runs a range).
Each program declares one generic base (interface, class or abstract class; 1-3 type parameters, sometimes
constrained to `string` or `unknown[]` (tuple arguments), with defaults or `in` / `out` / `in out` annotations) with 16-26 members drawn from 60
member shapes (interfaces) plus 7 class-only ones, named `<feature>_<n>` so a disagreement's culprit list names its
features:

- positions: property, readonly, optional, method parameter / return / both (bivariant), function-typed property
  parameter / return / both (contravariant under strictFunctionTypes), construct signature types, type predicates,
  rest parameters, rest tuples and generic rest parameters (`...a: T`, function-typed and method), `NoInfer`, overloads, generic methods with constraints on the parameters, `this`
  as property type, return (polymorphic `this`), method parameter, function-property parameter, `Box<this>`,
  `keyof this`, `this["x"]`, `this` in a conditional's check type and in a generic method's callback;
- operators: conditional types (distributive, non-distributive `[T] extends [...]`, `infer`, the parameter in the
  extends type, through a conditional alias, in a branch, in a parameter type), mapped types (homomorphic, `-readonly`
  `-?`, `readonly ?`, key remapping with template literals, `Partial`, `Record`, `Pick`), `keyof` (as a type, as a
  method parameter, as a function-property parameter), indexed access (`T[keyof T]`, `X[T & K]`), template literal
  types, `Uppercase`, unions with `undefined | null`, intersections, arrays, readonly arrays, tuples, `Promise`, a
  generic interface `Box<T>`, recursive references to the base (`B<T>` and expanding `B<T[]>`), unique symbols and
  enums in unions;
- class-only: accessors, getter-only, `private`, `protected`, `#private`, abstract properties and methods; static
  members; a numeric index signature; interface merging of the base.

Derived types (3-7 per program): pass the arguments through, fix one, permute them, wrap one (`T[]`, `Box<T>`,
`T | undefined`, `Partial<T>`, `[T]`), add a parameter, a second level (`D1x extends D1`), interface merging of the
derived type, a class merged with an interface that extends the base, a mixin (`class extends mix(Base)<T>`),
compatible and incompatible added members, a `tag` member that steers `this`-conditionals.

Sites (60-120 per program) relate derived instances to base references: assignment (`const t: B<..> = d`), a
conditional type (`D<..> extends B<..> ? "yes" : "no"`), array literals and `pick(d, b)` (subtype reduction, the
subtype relation), `d as B<..>` (comparable relation), nested in an object type, and generic functions
(`function g<G, H extends G>(s: D<H>): B<G>`: type parameters as arguments). Argument pairs come from a lattice of 46
sub/supertype edges (literal / widened, `{}` / `{ a?: string }` / index signatures / `object`, readonly / mutable,
tuples / arrays, enums / members / numbers, unique symbols, template literal types, `string & {}`, functions with
fewer parameters, `void` / `undefined`), identical pairs, `any` / `unknown` / `never` / `{}` on either side, and
unrelated pairs. Option sets: strict (twice as often), strict + exactOptionalPropertyTypes, strict without
strictFunctionTypes, strict without strictNullChecks, and no strict flags.

`run` type-checks each program with `TSRS_DERIVED_VARIANCE=shadow` (exit 7 or a report = finding; the decision count
comes from `TSRS_DERIVED_VARIANCE_LOG`), and also runs `off` and `on` and compares them byte for byte, and shadow's
diagnostics against `off`'s. Findings are kept with the program and the report. About half the programs reach a
decision at all (the rest relate pairs the shortcut does not take: unrelated arguments, mixins, failing declared
members); see the tables for the counts.

## Findings

In every case tsgo built from the pinned commit and tsrs with the switch off agree; `on` loses an error (or, for the
`void` cases, reduces a union that TypeScript keeps, which changes printed and emitted types); `shadow` reports the
disagreement and exits 7. Each has a minimal repro under `testdata/regressions/`
(run in all three modes, and without its guard, by `cargo test -p tsrs_cli --test derived_variance`).

| case | cause | guard |
| --- | --- | --- |
| `derived-variance-any-keyof` | `any` argument: `keyof any` is `string \| number \| symbol` and a homomorphic mapped type over `any` is an index signature; the markers kept both deferred, and `any` satisfies every variance (also invariant) | 4 |
| `derived-variance-keyof-optional` | `{}` and `{ a?: string }` are assignable to each other, so they satisfy any variance, but `keyof` tells them apart; no `any` and no conditional type involved | 4 |
| `derived-variance-mutual-conditional` | in a conditional's check type the markers make a parameter bivariant (deferred conditionals relate when their check types relate either way); mutually assignable arguments pick different branches | 4 |
| `derived-variance-this-conditional` | `this` in a conditional's check type measures bivariant, so guard 1 accepts it; the derived type (which adds `tag: 1`) picks the other branch. Identical type arguments; only `this` differs | 4 |
| `derived-variance-any-template` | `any` in a template literal type gives `` `a${any}` ``, not assignable to `"ab"` | 4 |
| `derived-variance-intersection` | assignability is not monotone under intersection: `{}` is assignable to `{ [k: string]: string }` (implicit index signature), but `{} & { z?: 1 }` reduces to `{ z?: 1 }`, which is not | 4 |
| `derived-variance-annotation` | TypeScript takes `in` / `out` as the variances without measuring them; a wrong `out` in a `.d.ts` under skipLibCheck is never reported, and the derived comparison is the only place the error surfaces | 5 |
| `derived-variance-generic-rest` | a rest parameter typed by the type parameter itself (`...a: T`) is compared element by element (`getTypeAtPosition`): `B<never[]>` -> `B<any>` holds by variances (`any` -> `never[]`), but `(...a: never[]) => void` -> `(...a: any) => void` compares `any` with `never`; likewise `never` against a tuple argument | 4 |
| `derived-variance-void-arity` | the strict subtype relation (union subtype reduction: array literals, `pick(a, b)`) treats a trailing parameter whose type contains `void` as optional (`getMinArgumentCount`) and then rejects `(x: void) => void` as a subtype of `(x: unknown) => void` (relater.go `StrictArity`). A plain `m(x: T)` member: `[D<void>, B<unknown>]` stays a union in tsgo, the shortcut reduced it to `B<unknown>[]` | 6 |
| `derived-variance-void-rest` | the same, through a tuple argument spread into a rest parameter (`m(...a: T)`, `T = [void]`) | 6 |

Open, cosmetic: `derived-variance-expanding-elaboration`. With an expanding recursive member (`next?: B<T[]>`),
deciding an earlier pair by variances skips comparisons whose relation-cache entries a later error elaboration
meets, and `on` elaborates that error one or more recursion levels deeper than tsgo (8 programs in the 40,000 of the
first campaign, all with `recursive_wrap`; the original shortcut has it too). The reported errors are the same.
Closing it exactly would mean making the comparisons the shortcut exists to skip; a guard that refuses generics with
expanding self-references would also refuse `ZodType` (`refine(): ... ZodType<R, core.input<this>>`).

By feature, before the new guards (campaign B below, the generator's culprit lists): conditional types in every form
(`cond_*`, `this_cond`), `keyof` in every position, mapped types in every form including `Partial`, indexed access,
template literals, `Uppercase`, intersections, and, in programs whose base has a wrong annotation (TS2636), any
member at all. Nothing else: no disagreement had only "structural" culprits (properties, methods, function types,
unions, arrays, tuples, `Promise`, `Box<T>`, recursive references, overloads, generic methods, accessors,
private / protected / `#private`, abstract, `NoInfer`, predicates, rest) outside TS2636 programs, apart from the
intersection case.

A side finding: in a program with a wrong annotation, measuring the `this` variance of that base (guard 1) before the
declaration check ran changed which member the TS2636 elaboration names (`on` and `shadow` printed a different
message from `off` and tsgo, 36 programs in 3000). Guard 5 removes it (such a base never reaches the measurement; a
base whose annotations are being verified takes no decisions). The original code had it too, hidden behind the
disagreements in the same programs.

## The guards

**Guard 4, monotone members** (`member_sensitive`, `sensitive_type`). The variance digest is a statement about
marker arguments. For a member whose type uses the type parameters and `this` only in monotone positions (property
types, function parameters and returns, unions, arrays, tuples, references to generic classes and interfaces, which
both routes relate by the same variances) the markers' answer carries over to real arguments. The seven cases are
all operators where it does not: they evaluate eagerly for real arguments and stay deferred for markers, or are not
monotone in assignability. So an inherited member counts as related only if its declarations put no class /
interface / signature type parameter and no `this` under keyof / unique, a conditional type, a mapped type, indexed
access, a template literal type, an intersection, a rest parameter's type, `infer`, a type query (`typeof this.x` always), or an intrinsic
alias (`Uppercase`, `NoInfer`); type aliases are followed into their bodies with their parameters bound to the
arguments that mention a variable. Declarations without a type annotation count as sensitive. Sensitive members are
compared structurally like the members the derived type declares. Exception: a sensitive slot that is `any` or
`unknown` in the target (a property's type, a method's return or parameter type; methods with one declaration, no
type parameters, no `this` parameter) relates whatever the source is, so it stays covered. Guard 4 subsumes guard 3.

Guard 4 also covers `typeof this.x` (a type query starting with `this`).

**Guard 5, verified annotations** (`annotations_hold`). Before trusting variances that come from `in` / `out`, check
them as `checkTypeParameterDeferred` does (marker instantiations related in the annotated direction), silently, with
markers of its own (sharing the `*_for_check` markers left relation-cache entries that changed TS2636 elaborations).

**Guard 6, `void` arity under strict subtype** (`reaches_void`). No decision under the strict subtype relation when
an argument of the base reference or the target has `void` as a constituent, or as a constituent of a tuple or array
element (spread into a rest parameter). Only the strict subtype relation checks arity by `getMinArgumentCount` (in
assignability a smaller minimum only makes the source more assignable).

Also: member comparisons inside a decision now combine like the relater's (`result &= related`) and accept Maybe
(recursion through `this` back to the pair being decided); requiring True made most decisions with structural members
fall back.

## Campaigns

All with the final binary (guards 1-6), rebased on main; seeds are disjoint ranges, so each campaign is reproducible
with `tools/fuzz/derived_variance.py run --start S --count N [--min-members 0] [--env TSRS_DERIVED_VARIANCE_NO_GUARD=4,5,6]`.

| campaign | setting | programs | programs with a decision | decisions | disagreements | on vs off compared / differing | shadow vs off differing |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A | default threshold (16 properties), seeds 500000-539999 | 40,000 | 18,593 | 334,909 | 0 | 40,000 / 4 | 0 |
| C | every target size (`--min-members 0`), seeds 600000-615999 | 16,000 | 7,584 | 136,556 | 0 | 16,000 / 2 | 0 |
| B | guards 4-6 off, seeds 500000-507999 (on/off every 4th) | 8,000 | 4,240 | 106,174 | 29,408 | 2,000 / 891 | 0 |

Guarded: **471,465 decisions over 56,000 programs, 0 disagreements**. The 6 on/off differences in A and C all have the
expanding recursive member (`recursive_wrap`) and the same error set as `off`: the open cosmetic case above. With
guards 4-6 off (B) the same generator gives 29,408 disagreements in 3,670 of 8,000 programs, and every on/off
difference there is explained by a reported disagreement.

Earlier campaigns while the guards were being built (each finding above was found by one of them, then closed and
re-run): 3,000 programs with guards 1-3 (6,115 disagreements); 40,000 programs with guards 1-5 (340,000 decisions, 0
disagreements; 7 cosmetic on/off differences) and 8,000 at every target size (67,736 decisions, 1 disagreement: the
`void` arity case); 40,000 with guards 1-6 before rest parameters were sensitive (337,372 decisions, 18 disagreements:
the generic rest case, after the generator learned `...a: T`).

Decisions per feature: a decision is credited to every member feature of its program's base, so these count how
often a feature was present when the shortcut decided, not that it decided because of it. The last column counts
disagreements whose culprit list names the feature (B; in parentheses those in programs where the base has a wrong
variance annotation, TS2636, where any member can be a culprit).

| feature | decisions credited, guarded (A) | decisions credited, guarded, all sizes (C) | decisions credited, guards 4-6 off (B) | disagreements naming it, guards 4-6 off (B; in TS2636 programs) |
| --- | --- | --- | --- | --- |
| `this_cond` | 67,316 | 26,384 | 30,560 | 9,736 (1,864) |
| `cond_ext` | 81,790 | 33,869 | 28,897 | 3,448 (1,212) |
| `cond_nondist` | 92,234 | 36,107 | 30,870 | 2,454 (672) |
| `keyof_fnparam` | 90,807 | 37,775 | 29,175 | 2,120 (681) |
| `mapped_opt` | 90,942 | 38,785 | 29,005 | 1,446 (579) |
| `mapped_homo` | 94,166 | 37,431 | 30,131 | 1,004 (926) |
| `mapped_remap` | 92,155 | 36,894 | 29,289 | 1,045 (746) |
| `mapped_minus` | 81,968 | 34,086 | 24,763 | 848 (878) |
| `partial` | 93,313 | 36,600 | 29,847 | 1,012 (612) |
| `template` | 92,167 | 35,928 | 29,345 | 710 (659) |
| `keyof` | 86,762 | 34,830 | 28,336 | 699 (649) |
| `uppercase` | 93,746 | 37,311 | 28,842 | 809 (533) |
| `ctor_type` | 88,882 | 36,841 | 28,815 | 1 (1,240) |
| `fnprop_both` | 89,237 | 35,412 | 29,193 | 0 (1,191) |
| `indexed` | 85,854 | 34,174 | 28,732 | 521 (524) |
| `indexed_key` | 81,379 | 32,705 | 27,237 | 550 (378) |
| `cond_fn` | 86,198 | 35,510 | 28,164 | 555 (249) |
| `pred` | 91,867 | 37,862 | 30,029 | 0 (752) |
| `cond_branch` | 94,186 | 37,160 | 30,116 | 1 (722) |
| `uniq` | 94,745 | 37,517 | 28,614 | 0 (718) |
| `fnprop_param` | 88,889 | 36,977 | 29,300 | 0 (697) |
| `overload` | 95,608 | 39,966 | 29,714 | 0 (696) |
| `union_undef` | 94,217 | 39,575 | 29,197 | 0 (683) |
| `intersection` | 93,984 | 38,306 | 29,021 | 117 (557) |
| `prop` | 94,985 | 38,462 | 30,067 | 0 (661) |
| `record` | 95,655 | 39,269 | 31,242 | 0 (656) |
| `box` | 95,346 | 38,537 | 30,485 | 0 (650) |
| `tuple_opt` | 93,219 | 39,564 | 29,529 | 0 (631) |
| `enum_union` | 95,754 | 38,039 | 28,844 | 0 (627) |
| `readonly_array` | 95,341 | 38,949 | 28,680 | 0 (612) |
| `fnprop_ret` | 94,381 | 38,324 | 28,920 | 0 (599) |
| `cond_dist` | 94,225 | 38,673 | 30,129 | 340 (251) |
| `readonly` | 95,807 | 38,409 | 30,771 | 0 (584) |
| `cond_alias` | 92,187 | 38,929 | 29,887 | 317 (265) |
| `method_both` | 93,894 | 38,558 | 29,466 | 0 (560) |
| `method_ret` | 92,994 | 38,275 | 29,657 | 0 (542) |
| `pick_keys` | 94,798 | 40,833 | 28,905 | 0 (522) |
| `cond_infer` | 92,378 | 37,720 | 29,676 | 382 (137) |
| `generic_method_this` | 92,007 | 36,815 | 29,079 | 0 (503) |
| `optional` | 94,513 | 37,557 | 28,966 | 0 (493) |
| `promise` | 93,167 | 38,292 | 27,503 | 0 (488) |
| `unknown_fn` | 93,889 | 38,180 | 29,309 | 0 (437) |
| `tuple` | 95,356 | 37,919 | 29,654 | 0 (428) |
| `array` | 96,582 | 38,879 | 29,019 | 0 (422) |
| `getter_only` | 56,623 | 22,163 | 16,817 | 0 (357) |
| `private` | 54,540 | 21,741 | 16,302 | 0 (312) |
| `protected` | 54,729 | 22,597 | 17,857 | 0 (298) |
| `accessor` | 53,664 | 22,187 | 17,134 | 0 (260) |
| `abstract` | 27,910 | 12,428 | 8,492 | 0 (175) |
| `rest_generic_fn` | 12,516 | 5,262 | 3,769 | 6 (120) |
| `abstract_method` | 29,423 | 11,772 | 7,686 | 0 (99) |
| `this_indexed` | 35,672 | 13,783 | 10,830 | 0 (64) |
| `rest_generic` | 13,980 | 6,054 | 4,417 | 4 (7) |
| `noinfer` | 93,838 | 37,874 | 29,287 | 1 (0) |
| `keyof_param` | 93,251 | 39,807 | 29,631 | 0 (1) |
| `rest_param` | 194,920 | 77,951 | 61,148 | 0 (0) |
| `this_ret` | 137,728 | 56,166 | 45,077 | 0 (0) |
| `recursive` | 95,376 | 38,966 | 29,249 | 0 (0) |
| `this_prop` | 95,053 | 39,205 | 31,004 | 0 (0) |
| `pad` | 94,940 | 38,507 | 30,953 | 0 (0) |
| `this_param` | 94,466 | 38,619 | 30,641 | 0 (0) |
| `generic_method` | 94,395 | 38,840 | 28,928 | 0 (0) |
| `rest_tuple` | 94,081 | 36,802 | 30,196 | 0 (0) |
| `recursive_wrap` | 93,588 | 39,718 | 29,483 | 0 (0) |
| `this_box` | 93,507 | 38,126 | 29,928 | 0 (0) |
| `method_param` | 91,569 | 39,614 | 30,743 | 0 (0) |
| `static` | 59,097 | 24,757 | 18,564 | 0 (0) |
| `hash_private` | 53,955 | 22,041 | 16,031 | 0 (0) |
| `index` | 51,467 | 21,107 | 16,825 | 0 (0) |
| `merged_base` | 22,473 | 8,283 | 6,584 | 0 (0) |
| `this_fnprop` | 21,825 | 9,701 | 6,475 | 0 (0) |

## Real code

`tools/fuzz/derived_variance_corpus.sh` (clone / install / run) pins 15 projects and type-checks 16 tsconfigs in
shadow mode, with overrides for options TypeScript 7 removed (node10 resolution, ES5, baseUrl) and composite projects:

| project (pinned commit in the script) | decisions | generic bases | disagreements | on vs off | tsrs errors (incomplete installs, removed options) |
| --- | --- | --- | --- | --- | --- |
| zod/packages/zod | 672 | 18 | 0 | same | 4 |
| typeorm/packages/typeorm | 42 | 7 | 0 | same | 31 |
| mikro-orm | 0 | 0 | 0 | same | 10 |
| rxjs/packages/rxjs | 13 | 3 | 0 | same | 64 |
| effect/packages/effect | 142 | 10 | 0 | same | 3 |
| mobx-state-tree | 1,199 | 9 | 0 | same | 175 |
| fp-ts | 0 | 0 | 0 | same | 3 |
| three-types/types/three | 200 | 20 | 0 | same | 0 |
| sequelize/packages/core | 87 | 14 | 0 | same | 160 |
| kysely | 13 | 2 | 0 | same | 0 |
| drizzle-orm/drizzle-orm | 748 | 37 | 0 | same | 32 |
| trpc/packages/server | 15 | 4 | 0 | same | 0 |
| trpc/packages/client | 2 | 1 | 0 | same | 0 |
| nest | 402 | 50 | 0 | same | 23 |
| vue-core | 4 | 2 | 0 | same | 5 |
| io-ts | 0 | 0 | 0 | same | 0 |

3,539 decisions, 0 disagreements, `on` byte-identical to `off` everywhere. These projects have few large derived
generic families (the 16-property threshold leaves little), so they confirm absence on ordinary code more than they
attack the shortcut. mikro-orm took 449 decisions (0 disagreements) in an earlier run and 0 in the final one with the
same configuration, also with main's binary; not explained. zod with its tests (vitest unresolved) took 3,015 decisions,
0 disagreements, with guards 1-3. The conformance harness (`TSRS_CHECKER_ASSIGNMENT=locality`, all suites): the same
13,458 passing tests with the switch off, on and in shadow mode; 194 decisions, 0 disagreements.

## Cost on the 38k-file codebase

Medians of 3 interleaved rounds (off, guarded on, unguarded on = `TSRS_DERIVED_VARIANCE_NO_GUARD=4,5,6`), a 2-core sandbox, `--extendedDiagnostics`; peak from the child's max RSS. Instantiations are exact at one checker and vary slightly with work distribution at 8.

| variant | 1 checker: check s | instantiations | peak GiB | 8 checkers: check s | instantiations | peak GiB |
| --- | --- | --- | --- | --- | --- | --- |
| off | 42.23 | 41.32 M | 4.23 | 9.99 | 110.86 M | 7.56 |
| on, guards 1-6 | 41.48 (-1.8%) | 41.05 M (-0.7%) | 4.20 (-0.8%) | 9.96 (-0.3%) | 109.26 M (-1.4%) | 7.37 (-2.6%) |
| on, guards 1-3 (unsafe) | 37.33 (-11.6%) | 34.28 M (-17.0%) | 3.83 (-9.4%) | 8.05 (-19.4%) | 83.45 M (-24.7%) | 6.28 (-16.9%) |

Switch off: counters identical to main (single-checker types and instantiations equal). The guarded switch decides
about as many pairs as before (10,549 in shadow mode at one checker, 0 disagreements), but `ZodType`'s costly members
(`parse(): core.output<this>`, `safeParse(): ZodSafeParseResult<core.output<this>>`, `check(...checks:
CheckFn<core.output<this>>[])`, `def: Internals["def"]`) put `this` or a parameter under a conditional or an indexed
access, so guard 4 compares them structurally (31 of `ZodType`'s 55 members per decision on average); the
top-type exception only helps where the target is `ZodType<any, any, any>` and the slot is a bare return or
property. The members that carried the savings are exactly the ones where the digest is not proven.

## Judgement

**Do not turn it on by default; the bench table confirms the drafted verdict.**

- Unguarded (the state before this work) the shortcut saves 17-25% of instantiations and 12-19% of check time on the
  38k-file codebase, but it is not exact: with guards 4-6 off the generator finds disagreements in 46% of its programs (3,670 of 8,000), in
  shapes that are ordinary TypeScript (`keyof T`, `T & {...}`, any conditional type, `this extends ...`, a mapped
  type, `...a: T`, a `void` argument meeting union reduction, a wrong `out` in a `.d.ts`). zod's own `core.output<this>`
  is the `this`-conditional shape; it is harmless on the 38k-file codebase only because no derived schema changes that
  conditional's branch. Zero disagreements on 20 real projects says little: they rarely compare such pairs with
  arguments where markers and real types differ.
- Guarded (1-6), every disagreement found is closed (471,465 decisions, 0 disagreements; real code and the suite
  unchanged), but the savings are gone: -1.8% / -0.3% check time (noise), -0.7% / -1.4% instantiations, -0.8% / -2.6%
  peak at 1 / 8 checkers. Keeping it for that is not worth the risk that remains: the guards are empirical, built from
  what the generator found; a proof that "monotone member + variance digest" implies the structural answer does not
  exist (accessor variance, generic methods with constrained type parameters, `Maybe` results through recursion and
  cache-order effects are covered only by testing), and the open cosmetic case shows the relation cache can still
  change messages.
- If the speed is wanted anyway, it can only come with accepted inexactness: `TSRS_DERIVED_VARIANCE=on
  TSRS_DERIVED_VARIANCE_NO_GUARD=4,5,6` gives the old behaviour, with the failure list above as its documented cost.
  A different route to the same savings would have to avoid instantiating the derived types' inherited members
  without trusting the digest (for example by sharing member instantiations across derived types whose base
  reference has the same arguments, for members that do not mention `this`); not tried.
