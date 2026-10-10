# Likely review pushback, and what we have

Evidence that applies to every commit (README.md has the commands):

- **Same results.** The checker tests and the testrunner (13,485 compiler test variants with errors, `.types` and
  `.symbols` baselines) pass at every commit with no `testdata/baselines/local` output; on the top commit also `go
  test ./...`, `TS_TEST_PROGRAM_SINGLE_THREADED=false` and `-race`. On the 38k-file program: 0 errors at every commit;
  types and instantiations never change; symbols only drop where a lazy table answers.
- **Same work as the prototype, in two implementations.** Built on the tsrs reference commit (b85298b6), each Go
  commit's symbol/type/instantiation counters on the 38k-file program equal the Rust port's with the matching
  `TSRS_LAZY_*` switches on, single-threaded in every run, and with 4 checkers in most runs (some 4-checker Go runs
  are 6 symbols higher, an ordering difference that main already has, see below); also on the two tests from
  #64475/#64526 and the three new tests.
- **The new tests pin output from before any of this.** Their baselines were generated on unmodified main (no
  #64475/#64526 either).

## General

- **"This is a stack on two unmerged PRs."** L1, L11 and L10 need #64475's lazy member tables (L1 only needs #64475;
  L11 and L10 were measured on #64475 + #64526 + the earlier commits). L6 and L5 don't depend on them at all: they
  apply cleanly to main, pass the suite there, and have their own main-only numbers (pr-04, pr-05). L6 and L5 were opened
  against main first (#64600, #64601).
- **CONTRIBUTING.md bars "bulk, agent-driven contributions".** It requires that a human picked the change and
  shepherds it. Open these one at a time, as Max's own follow-ups to #64475/#64526, and keep the disclosure line.
- **"Check time?"** Within noise everywhere. The machine was shared (load 8-18 during the runs); single-threaded check
  time moved by up to +-17% between builds with identical counters. Not claimed as a win.
- **"Peak RSS went up with 4 checkers on some commits."** Go's peak footprint spreads by ~1.5 GB between runs of the
  same build with 4 checkers (GC timing). Heap after check (`Memory used`) spreads by < 0.02 GB and goes down at
  every commit except L5 (which only removes garbage).
- **"Are the counts deterministic?"** Single-threaded: yes, every run of every build gave the same symbols, types and
  instantiations (and mallocs within a few hundred); 1-checker runs gave constant symbols too. With 4 checkers, some
  runs come out exactly 6 symbols higher (19,241,827 vs 19,241,821 on L11, 18,691,695 vs 18,691,689 on L10..L5, and
  now also 19,553,619 vs 19,553,613 on L1). Types and instantiations never vary. It is not caused by L11, not by the
  checkers running concurrently, and not by global symbol ids; it is an existing run-to-run ordering difference inside
  a checker, which main also has, and which the lazy tables let reach the symbol count. Details in "The 6-symbol
  variation" below. The suite also passes with `-race` and concurrent test programs on the top commit (no race
  reports).

### The 6-symbol variation (investigated 2026-10-01, main 82f0546163 and the old base edf7da4e93)

The old hypothesis (global symbol ids -> `__@name@<id>` property names -> property order -> early-exit paths) is
refuted. Measured with throwaway instrumentation on the private monorepo, 4 checkers. `tools/variance-debug.patch`
(applies to the edf7da4e93-based L1/L11 commits; `TSGO_SYMDUMP=<dir>` with `--extendedDiagnostics` writes per-checker
stack histograms for those six member names, `TSGO_SEQ4=1` plus `--singleThreaded` runs 4 checkers one after
another) is the last of three variants; the first two (a full `newSymbol` histogram and the `compareSymbolsWorker`
counters) were the same idea at other sites:

- **Which symbols.** A per-checker histogram of every `newSymbol` (name, flags, check flags; names that embed ids,
  `__@x@<id>` and private `\xfe#<id>@#x`, normalized) differs between a normal and a +6 run of L11 in exactly 6
  entries, all in checker 3: the optional properties `id`, `name`, `scope`, `service`, `type`, `version` of one
  lib.es5 `Partial<X>`. A stack capture at `newMappedTypeMember` shows they come from one full resolution of that
  mapped type: `resolveMappedTypeMembers` <- `everyPropertyOfStructuredType` <- `hasPropertiesOfStructuredType` <-
  `isWeakType` <- `isRelatedToEx` <- `typeRelatedToSomeType` (union target) <- type-argument relation of a signature,
  during overload resolution (`chooseOverload` / `isSignatureApplicable`). In normal runs that `Partial<X>` is never
  resolved in full.
- **Not the symbol ids.** Counting the comparisons in `compareSymbolsWorker` (which `sortSymbols` and `CompareTypes`
  use) that are decided by the `@<id>` suffix of a unique-symbol name or by the final symbol-id fallback: zero, on both
  bases. No sort in the checker depends on a global id in these runs.
- **Not concurrency.** With 4 checkers forced under `--singleThreaded` (same file assignment, parse, bind and the
  four checkers run one after another, so ids are handed out in a fixed order) the +6 still shows up: L11 1 of 15
  runs, L1 1 of 3 runs (the same `Partial<X>`, same stack). Parallel 4-checker L11 on the old base: 4 of 53 runs.
- **Present on main.** In those sequential runs the type ids at a fixed creation site (members of type-fest's
  `Simplify` mapped type, via `getUnmatchedPropertiesWorker` in `propertiesRelatedTo`) differ between runs by 51 ids
  in checkers 0, 1 and 3, on unmodified main edf7da4e93 too (5 of 5 run pairs differ), while main's counts stay
  constant (6 of 6). So something inside a checker creates types in a per-process order (with goroutines and global ids
  ruled out, presumably a Go map iteration; not located). On main every order does the same total work. With the lazy
  tables, whether this one `Partial<X>` gets resolved in full depends on that order. A plausible link, not verified:
  `CompareTypes` orders some union constituents by type id as its last resort, and `typeRelatedToSomeType` stops at
  the first related constituent, so a different id order relates (and `isWeakType`-checks) a different constituent
  first.
- **Why it looked like "from L11 on".** Before L11, `isEmptyObjectType` / `removeSubtypes` resolved such types in every
  order anyway; L11 answers those from the lazy tables, so more types stay unresolved and the order matters more
  often. L1 shows it too, only less often (0 of 33 parallel runs before; 1 of 3 sequential runs now). On current main
  (37,943 files, one more lib file, so a different file assignment) 27 4-checker runs of L11..L5 gave constant counts.
- **Implications.** Diagnostics are the same in every run; the difference is which members of one mapped type get
  created. It is not something L11 introduces, so it doesn't block L11, but a reviewer may see different counts
  between runs with #64475 itself. Making it go away means finding the order-dependent loop on main (a follow-up, not
  part of these PRs).

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
  malloc delta on the stack on main 82f0546163 (4,340,793, the number in the PR) agree to within 250; the earlier
  edf7da4e93 stack gave 4,340,242 (and an earlier run 4,340,505).
- **"Why not `mapTypeWithCompositeMapper`?"** It goes through `getMappedType`, which substitutes a distributed type
  parameter's constraint; `CompositeTypeMapper.Map` doesn't, so it would change type arguments in some cases.
