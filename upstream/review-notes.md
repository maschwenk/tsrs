# Likely review pushback, and what we have

Evidence that applies to every commit (README.md has the commands):

- **Same results.** The checker tests and the testrunner (13,485 compiler test variants with errors, `.types` and
  `.symbols` baselines) pass at every commit with no `testdata/baselines/local` output; on the top commit also `go
  test ./...`, `TS_TEST_PROGRAM_SINGLE_THREADED=false` and `-race`. On the 38k-file program: 0 errors at every commit;
  types and instantiations never change; symbols only drop where a lazy table answers.
- **Same work as the prototype, in two implementations.** Built on the tsrs reference commit (b85298b6), each Go
  commit's symbol/type/instantiation counters on the 38k-file program equal the Rust port's with the matching
  `TSRS_LAZY_*` switches on, single-threaded in every run, and with 4 checkers in most runs (some 4-checker Go runs
  from L11 on are 6 symbols higher, see below); also on the two tests from #64475/#64526 and the three new tests.
- **The new tests pin output from before any of this.** Their baselines were generated on unmodified main (no
  #64475/#64526 either).

## General

- **"This is a stack on two unmerged PRs."** L1, L11 and L10 need #64475's lazy member tables (L1 only needs #64475;
  L11 and L10 were measured on #64475 + #64526 + the earlier commits). L6 and L5 don't depend on them at all: they
  apply cleanly to main, pass the suite there, and have their own main-only numbers (pr-04, pr-05). Opening L6 and L5
  against main first is possible and probably easier to review.
- **CONTRIBUTING.md bars "bulk, agent-driven contributions".** It requires that a human picked the change and
  shepherds it. Open these one at a time, as Max's own follow-ups to #64475/#64526, and keep the disclosure line.
- **"Check time?"** Within noise everywhere. The machine was shared (load 8-18 during the runs); single-threaded check
  time moved by up to +-17% between builds with identical counters. Not claimed as a win.
- **"Peak RSS went up with 4 checkers on some commits."** Go's peak footprint spreads by ~1.5 GB between runs of the
  same build with 4 checkers (GC timing). Heap after check (`Memory used`) spreads by < 0.02 GB and goes down at
  every commit except L5 (which only removes garbage).
- **"Are the counts deterministic?"** Single-threaded: yes, every run of every build gave the same symbols, types and
  instantiations (and mallocs within a few hundred); 1-checker runs gave constant symbols too. With 4 checkers, Go
  builds from L11 on came out 6 symbols higher in 9 of 52 runs (19,241,827 vs 19,241,821, 18,691,695 vs 18,691,689,
  ...); builds before L11 (main, PRs, L1, and main + L6/L5) gave constant counts in all 33 runs (main alone: 9), and
  the Rust port with 4 checkers is constant and equals the usual Go value. Types and instantiations never varied.
  Cause not established. A hypothesis (not verified): symbol ids are global across checkers (`ast.GetSymbolId`), and
  unique-symbol property names embed them (`__@name@<id>`), so with several checkers the name order of some synthetic
  members can change between runs; a path that visits properties in name order and stops early (as
  `somePropertyReducesToNever` does) would then create a few more or fewer combined properties. Worth understanding
  before opening L11. The suite also passes with `-race` and concurrent test programs on the top commit (no race
  reports).

## L1 (tuple lazy tables)

- **"Do tuples behave differently from interfaces here?"** No tuple-specific code: tuple targets have declared
  members (`"0"`, `"1"`, ..., `length`) and base types, `getReferenceMemberTypeArguments` already handles the padded
  `this` argument that tuples use (it is the same code `resolveTypeReferenceMembers` runs for them), and every
  consumer goes through #64475's lazy-aware accessors. `tupleLazyMembers.ts` covers element access, `length`, array
  methods through the `this`-typed `Array` base, optional/rest/named/readonly tuples, weak-type checks.
- **"The old `ObjectFlagsTuple != 0` exclusion looked deliberate."** It was redundant with the `ClassOrInterface`
  test (tuple targets are `Tuple|Reference`, never `Class`/`Interface`), so it never excluded anything that wasn't
  already excluded; it reads like a reminder that tuples were left out on purpose in the first PR.
- **"Only 13% of tuple tables are resolved in full later?"** Counted in the port: 8,402 of 66,727 single-threaded.

## L11 (empty object checks)

- **"`isEmptyStructuredType` skips the `anyFunctionType` test on the lazy path."** A type with a lazy table is an
  instantiated reference (`mayHaveLazyMembers` requires `ObjectFlagsReference`), and `anyFunctionType` is an anonymous
  type, so it can never take that path. The non-lazy path is the old expression.
- **"Signatures are checked before properties, the old code checked properties first."** Pure predicates, no side
  effects differ: the table's signatures/index infos were computed when it was prepared.

## L10 (getUnmatchedProperties on a lazy target)

- **"This duplicates the resolution order logic."** `getPropertiesOfTypeLazily` builds the same member table
  `resolveLazyMembers` builds, with the same helpers (`addInheritedMembers` over the same base types, then
  `getNamedMembers`), substituting the declared member wherever the table hasn't instantiated it yet. A declared member
  has the instantiation's name, flags and declarations, and `getNamedMembers` orders by declarations then name, so the
  order is the same. `lazyMembersUnmatchedProperties.ts` pins the order through TS2739/TS2740 messages
  ("missing the following properties ...: d1, shared, b1, b2", "... and 2 more") with baselines from unmodified main.
- **"Why cache the list on the table?"** Base types share it. Without the cache a first prototype walked bases per
  call and went exponential on diamond-shaped mixin hierarchies (`intersectionConstructorReductionCrash` 6.7 s -> 20 s
  in the port).
- **"It allocates more."** A little: +0.25M mallocs single-threaded (+0.17%), +0.5M with 4 checkers (+0.2%), for
  -272K / -550K retained symbols and -0.04 / -0.10 GB heap. That's the property list per walked table (a member
  table and a slice). The first translation cost +1.69M; the difference was apparent-type recomputation for
  type-parameter targets (README, "What did not transfer one to one"). The only commit that adds allocations; it can be
  dropped without affecting the others (L6 and L5 don't touch the same code).
- **"`hasPropertyOfType` returns an uninstantiated declared member."** It returns a bool; the only consumer of the
  symbol inside is `symbolIsValueEx`, which reads flags, and a declared member has its instantiation's flags. The
  `instantiate` parameter only changes the `getLazyDeclaredMember` call; mapped-type tables still create the member
  (same as before).
- **"The discriminant path."** With `matchDiscriminantProperties` the source lookup is the regular instantiating one,
  and the real target property is looked up before its type is read, so types are compared exactly as before.

## L6 (union/intersection property cache)

- **"Is reading the other cache really equivalent in every order?"** The three cases are in the PR description:
  augmented-first (augmented cache wins, as before), non-augmented-first (the copy and the other-cache read return the
  same symbol), partial (never copied, never shared). Nothing else reads either cache (`grep propertyCache`).

## L5 (conditional instantiation mapper)

- **"No heap change, why bother?"** -4.34M allocations single-threaded (-2.9%), -8.8M with 4 checkers (-3.5%), all
  of them 40-byte objects that were dropped immediately. The port's count of avoided composites (4,340,555) and the Go
  malloc delta (4,340,505) agree to within 50.
- **"Why not `mapTypeWithCompositeMapper`?"** It goes through `getMappedType`, which substitutes a distributed type
  parameter's constraint; `CompositeTypeMapper.Map` doesn't, so it would change type arguments in some cases.
