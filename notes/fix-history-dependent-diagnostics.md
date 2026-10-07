# fix-history-dependent-diagnostics: TS2320 on a merged interface depended on which files shared a checker

PR 163 (split default library files when they weigh most of a share) ran pr-verify, and on nuxt at 32 checkers one
run of three printed three errors instead of four: the TS2320 at `packages/nuxt/src/app/types/augments.ts(8,13)`
(`Interface 'ImportMeta' cannot simultaneously extend types 'NitroImportMeta' and 'NitroStaticBuildFlags'`) was
missing. On this Mac (18 cores, `--checkers 32`) PR 163's binary lost it in 1 of 20 runs. The cause is not the split
check and not the identity relation: TypeScript runs the merged-interface checks once per checker, at whichever
declaration that checker visits first. This note shows it, fixes it in the default mode, and keeps Go's behaviour under
`--checkerAssignment go`.

## The dependence

`checkInterfaceDeclaration` (checker.go:5097 at the pinned commit) guards three checks with a per-checker link:

```go
if links := c.declaredTypeLinks.Get(symbol); !links.interfaceChecked {
    links.interfaceChecked = true
    ... checkInheritedPropertiesAreIdentical(type, node.Name())        // TS2320 at node.Name()
    ... checkTypeAssignableTo(typeWithThis, baseWithThis, node.Name(), // TS2430 at node.Name()
    ... checkIndexConstraints(type, symbol, false)
```

TS2320 and TS2430 are reported at the declaration being checked. For an interface declared in several files, each
checker reports them at the first declaration it visits and never at the others. So which files report depends on
which files share a checker and in what order the checker visits them. Strada used `node === firstInterfaceDecl`
(reporting at the first declaration of the program); tsgo changed it to the link and with it made the output depend on
the assignment.

nuxt declares the global `ImportMeta` in two checked files, `packages/nuxt/src/app/types/augments.ts` and
`packages/schema/src/builder-env.ts` (plus lib files and `.d.ts` files, unchecked under `skipLibCheck`). The merged
interface extends `NitroImportMeta` and `NitroStaticBuildFlags` (nitro's augmentations), whose `preset` properties
differ.

Evidence:

- tsgo itself (npm 7.1.0-dev.20260930.4) prints one TS2320 (augments.ts) at `--checkers 1` and `2`, two at `4` and `32`.
- A debug print in `check_interface_declaration` (not landed) on PR 163's binary, in the failing run (run 51 of 60):
  one checker checked `builder-env.ts` and then `augments.ts`, and skipped the second because the link was set. In a
  passing run the two files were on two checkers. Stealing moves whole files between checkers, and the split pieces of
  PR 163 changed how much each checker had left, so the two files sometimes ended up together.
- Whole-file assignment shows it without stealing or splitting: `TSRS_SPLIT_FILES=0 --checkerAssignment random:<seed>`
  at 2 checkers lost the builder-env.ts error for seed 2 and the augments.ts error for seed 8 (8 seeds). At 32 checkers
  the 8 seeds tried kept both.
- The split check has the same problem inside one declaration file. `testdata/split-check/merged-interface-pieces`
  declares `Merged` twice in `types.d.ts`, extending `Left` and `Right`. Unsplit, the file's checker reports once, at
  the first declaration (tsgo too). With `TSRS_SPLIT_FILES=force:2` the second declaration's piece runs on a checker
  that has not checked `Merged`, which reports a second TS2320 at `types.d.ts(10,11)`; under `shadow` the owner panics.

The identity relation, variances, lazy members and the split merge were the other suspects; none is involved. The
failing and passing runs print the same message whenever the check runs.

## The fix

`Checker::is_interface_check_site` (checker_03.rs) decides whether a declaration runs the guarded checks:

- Default (canonical): the declaration is the first interface declaration of the symbol in its own source file. Every
  file that declares the interface runs the checks once, whatever was checked before and wherever the file's
  statements are checked.
- `go_compatible_history()` (`--checkerAssignment go`, the tsgo-baseline harnesses): Go's `interface_checked` link,
  unchanged.

The default output is tsgo's output whenever every file that declares the interface is the first such file on its
checker, which is what tsgo prints for nuxt at its default 4 checkers and at 32.

## Why no other diagnostic can change

The guarded block reports three kinds of diagnostics:

- TS2320 and TS2430 at the name of the declaration being checked. The fix only changes which declarations run the
  block: before, the first one a checker visited; now the first one in each file. A file's TS2320/TS2430 lines are
  therefore the ones the old code printed when that file was the first declaring file on its checker; the fix adds
  them in the other assignments and never adds a second one in a file.
- Index-constraint errors (`checkIndexConstraints`) at a member or index signature declared in the interface, or at the
  interface's first declaration. Each of those nodes is in a file that declares the interface, and that file now runs
  the block itself, so the error is present whether or not another file's check reported it first (diagnostics are
  deduplicated per file).
- Nothing else: the block reads types and relations, which other checks resolve the same way whatever the history
  (notes/perf-order-independence.md).

So the only lines that change are the ones that used to depend on the assignment.

## Results

- nuxt, the fixed binary: `--singleThreaded`, and `--checkerAssignment random:1..8` at 1, 2 and 3 checkers (24 runs)
  print the same four errors; Go mode at one checker prints tsgo's three.
- `testdata/regressions/merged-interface-check-site` (a.ts, b.ts, c.ts and d.d.ts declare `Merged`; expected.txt is
  tsgo-ref at 8 checkers, where every file is the first on its checker): over the single-threaded run and 24 random
  assignments the old code printed 47 of the 75 expected TS2320 lines, the fix all 75.
  `crates/tsrs_cli/tests/merged_interface_check_site.rs` runs it single-threaded, at 1-4 checkers with stealing and
  under 8 random assignments each, and checks Go mode against tsgo-ref at 1 and 8 checkers.
- `testdata/split-check/merged-interface-pieces` runs in `split_check.rs` (split off, default, `force:2/3/5/9`, shadow).
- Conformance suite: identical pass lists before and after, Go mode (13,458 / 12,779 / 12,779 errors / `.types` /
  `.symbols`) and canonical with 4 checkers (13,458 / 12,778 / 12,778).

## Other per-symbol links (audited, not changed)

`type_parameters_checked` (TS2428), `enum_checked` (TS2473/TS2432) and `index_signatures_checked` (TS2374) are also
set once per checker, but they report at every declaration of the symbol, so a file's checker always reports the
file's own lines, whichever declaration it reaches first.
