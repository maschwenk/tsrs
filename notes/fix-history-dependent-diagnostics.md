# fix-history-dependent-diagnostics: TS2320 on a merged interface depended on which files shared a checker

PR 163 (split default library files when they weigh most of a share) ran pr-verify, and on nuxt at 32 checkers one run
of three printed three errors instead of four: the TS2320 at `packages/nuxt/src/app/types/augments.ts(8,13)` (`Interface
'ImportMeta' cannot simultaneously extend types 'NitroImportMeta' and 'NitroStaticBuildFlags'`) was missing. On this Mac
(18 cores, `--checkers 32`) PR 163's binary lost it in 1 of 20 runs and main's in 1 of 60. The cause is not the split
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
  passing run the two files were on two checkers. Stealing moves whole files between checkers, so the two files
  sometimes end up together. PR 163 is not needed for that: nuxt sets `skipLibCheck`, so no library file is checked
  and nothing is split there (`TSRS_SPLIT_FILES=stats` prints nothing), and main's binary (4a00ada) lost the error in
  1 of 60 runs on the Mac too.
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

## A base-type cycle entered at a different member: TS2769 on drizzle-orm

pr-verify of PRs 165 and 171 printed one error more on drizzle-orm at 32 checkers than at 1, 4 and 16, and the README
bench at 8 checkers printed 10845 where tsgo-ref printed 10846. The extra line is

```
drizzle-kit/tests/cli-check.test.ts(75,3): error TS2769: No overload matches this call.
  ...
      Property 'NODE_ENV' is optional in type '{ NODE_ENV?: string; ... }' but required in type 'ProcessEnv'.
```

at `spawnSync(..., { env: { ...process.env, TEST_CONFIG_PATH_PREFIX: '' } })`.

### The dependence

drizzle's program has two bun-types versions, and their declarations make two global interfaces extend each other:

- bun-types 0.6.14 (`types.d.ts`): `declare module "bun" { interface Env extends Dict<string>, NodeJS.ProcessEnv {
  NODE_ENV: string; ... } }`, and the global `process`, whose `env` is that `Env` (its declaration is the first
  one of `process`, so `process.env` is `Env`).
- bun-types 1.2.15 (`overrides.d.ts`): `namespace NodeJS { interface ProcessEnv extends Bun.Env, ImportMetaEnv {} }`,
  and `bun.d.ts` merges `NODE_ENV?: string` into the same `Env`, which makes the merged property optional.
- `@types/node` (three versions) declares `ProcessEnv extends Dict<string>`, and expo-modules-core
  `ProcessEnv extends ExpoProcessEnv`, whose `NODE_ENV: string` is required.

`getBaseTypes` (checker.go:19508) resolves `ProcessEnv -> Env -> ProcessEnv` from whichever member the checker asks
about first. That member's `hasBaseType` check finds the cycle through the other member, which is resolved fully
meanwhile, so the first member drops its base in the cycle and the second keeps its own. Both report TS2310 at all
their declarations, so the TS2310 lines are the same either way (they are in `.d.ts` files, unchecked under
`skipLibCheck`). What changes is what `ProcessEnv` inherits:

- Entered at `Env` (`process.env.X` anywhere): `ProcessEnv` extends `Env`, its first base with `NODE_ENV`, so
  `ProcessEnv.NODE_ENV` is `Env`'s optional property, the one the spread of `process.env` copies, and the call checks.
- Entered at `ProcessEnv` (cli-check.test.ts's object literal is contextually typed by `NodeJS.ProcessEnv`): it
  loses `Env` and inherits expo's required `NODE_ENV`, while the spread still has `Env`'s optional one: TS2769.

So the error appears exactly when cli-check.test.ts is checked before any file that reads `process.env` on its
checker. Evidence:

- A debug print in `get_base_types` (not landed): in `--checkers 4 --checkerAssignment random:4` (the only one of seeds
  1-12 at 2 and 4 checkers that printed the error) one checker resolved `Env: [Dict<string>, ProcessEnv]` and
  `ProcessEnv` without `Env`, and the property the relation compared was expo's `NODE_ENV` (required) against bun's
  (optional); in the other checkers and seeds `Env: [Dict<string>]` and `ProcessEnv` with `Env`. A print of the
  checked node at the cycle's entry: single-threaded, `drizzle-kit/src/cli/commands/utils.ts`
  (`process.env.X`) enters at `Env`; in the failing checker, cli-check.test.ts enters at `ProcessEnv`.
- tsrs's `get_base_types` / `resolve_base_types_of_interface` / `has_base_type` are line-for-line ports, and Go has
  the same dependence: tsgo 7.0.2 prints the error at `--checkers 32` but not at 1 or 4; tsgo-ref (the pinned commit)
  at 4 but not at 1 or 32. On main, 1 of 3 runs at 32 checkers printed it.
- With the declaring files checked (`--skipLibCheck false`), tsgo-ref at one checker reaches `ProcessEnv`'s first
  declaration (`@types/node@18`'s `process.d.ts`, file 164 of 3473) before any user file and prints the TS2769.

### The fix

`Checker::canonicalize_base_type_cycles` (checker_09.rs), default mode only:

- `get_base_types` records the types whose base types it resolves within one outermost call and the ones whose
  resolution was circular. When the outermost call ends with a cycle, the cycle's entry is the circular type that
  finished last.
- If that entry is not the cycle's member declared first in program order (the earliest of its declarations by
  `compareNodes`; the symbol's first declaration is not always it, global augmentations are merged after the files'
  globals), the base types resolved in the call are reset and resolved again from that member, then from the type
  asked for. Each pass resolves the chosen member for good, so the loop ends.
- Cycles through a class are left alone: they also go through the base constructor type, which this does not reset.
- `go_compatible_history()` keeps Go's result.

So the default output is what tsgo prints when the cycle is entered at its first declaration, as it is when the
declaring files are checked in program order. On drizzle-orm that is `ProcessEnv`, so tsrs now prints the TS2769 in
every run and at every checker count: 10846 errors, tsgo-ref's count at 4 and 8 checkers and tsgo 7.0.2's at 32, one
more than tsgo prints at 1 checker (and 7.0.2 at its default 4), where a user file happens to reach `Env` first.
Choosing `Env` instead would need a rule that knows which checked file reaches the cycle first in program order, which
a checker that checks only some of the files cannot know.

### Why no other diagnostic can change

Programs without a base-type cycle reset nothing and run as before. When a cycle is met, only the base types resolved
within that outermost `get_base_types` call are reset (with `MembersResolved`, which `get_base_types` clears anyway).
Within the call only the heritage clauses' types are resolved; a clause that read the members of a type being reset
(say `extends Pick<A, "x">` inside the cycle) would keep what it read from the first pass. That shape has not been
seen; drizzle's cycle and the regression case extend plain interfaces. Each cycle member reports TS2310 at all its
declarations whichever member is entered, so the TS2310 lines do not change either.

### Results

- drizzle-orm, the fixed binary: `--singleThreaded`, `--checkers` 1, 4, 8 and 16, 20 runs at 32, and
  `--checkerAssignment random:1..12` at 2 and 4 checkers all print the same 10846 errors (main's single-threaded
  output plus the TS2769). Go mode at one checker prints main's 10845. With `--skipLibCheck false` Go mode at one
  checker prints tsgo-ref's output byte for byte, and the default mode differs from it only in PR 165's TS2320/TS2430
  lines.
- `testdata/regressions/base-type-cycle-entry` (env-a.d.ts, env-b.d.ts and env-c.d.ts declare the cycle, read-env.ts
  enters at `Env`, spawn.ts at `ProcessEnv`; expected.txt is tsgo-ref at 8 checkers, which prints the error, as it
  does at 16; at 1-5 it does not): main's binary printed the wrong output for 9 of 32 random assignments (1-4 checkers,
  seeds 1-8); the fix prints expected.txt for all of them, single-threaded and at 1-4 checkers with stealing. Go mode
  at one checker prints tsgo-ref's empty output, and with `--skipLibCheck false` tsrs prints tsgo-ref's output in both
  modes. `crates/tsrs_cli/tests/base_type_cycle_entry.rs` runs it.
- Conformance suite: identical pass lists before and after, Go mode (13,458 / 12,779 / 12,779 errors / `.types` /
  `.symbols`) and default mode with several checkers (13,458 / 12,778 / 12,778).
- vscode, t3code-server, formbricks-web, supabase-studio, cal-diy, mui-docs, webpack and xstate-main print the same
  bytes as main single-threaded and at 4 and 32 checkers.
- pr-verify (PR 177): 97 of 102 cells identical; the five drizzle-orm cells differ by the TS2769, which new prints in
  every run (10846) and base printed in some (at 16 checkers in reps 1 and 2 but not rep 0). Single-threaded
  instructions on drizzle-orm +0.05% (each checker that enters the cycle at `Env` resolves it twice).

### Sweep for other history-dependent diagnostics

`--checkerAssignment random:1..8` at 2 and 4 checkers on vscode, t3code-server, formbricks-web, supabase-studio,
cal-diy, mui-docs, webpack and xstate-main (128 runs), and `random:1..12` at 2 and 4 on drizzle-orm, against each
project's single-threaded output with the fixed binary: every run byte-identical. Not covered locally: nuxt, next,
storybook, playwright, mikro-orm and the Compiler projects (pr-verify runs them at fixed checker counts only).
Base-type cycles through a class (`class A extends B` with `B` reaching `A`) are not canonicalized and were not
seen; they also resolve the base constructor type, which would need resetting too.
