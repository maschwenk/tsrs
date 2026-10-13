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

## Later: TS2320 / TS2430 where tsgo's default pool reports them

The per-file rule printed more than tsgo whenever two declaring files share a tsgo checker. The top-400 repository run
(run 38278199589) found two: supabase's root config, where many checked files augment the global `Window` and tsgo
reports TS2430 in `DevToolbar.test.tsx` and `DevToolbarContext.tsx` but tsrs also in `HubSpotFormEmbed.tsx`, and
stagehand's root config, where three vite copies augment rolldown's `MinimalPluginContext` and tsgo reports TS2320 in
one copy, tsrs in two. tsrs `--checkerAssignment go --checkers 4` printed tsgo's output byte for byte on both.

So the default mode now runs the block where it did (the first declaration in each file, which keeps every file's
index-constraint errors) but reports TS2320 and TS2430 only where tsgo at its default four checkers would:
`Checker::reports_interface_check_errors` asks `Program::tsgo_default_check_slot` for the tsgo checker and program
index of each declaring file (`checkerpool::tsgo_default_check_slots`: Go's association at four checkers, the files it
type checks), and reports at a declaration only if no earlier file on the same tsgo checker declares the interface.
Elsewhere the checks run without an error node. That is a function of the program, so the output is still the same at
every tsrs checker count and assignment, and it is now tsgo's default output rather than a superset of it (tsgo at
another `--checkers` count can still differ; `--checkerAssignment go` matches it there). The association is computed
once per compiler-pool program on first use, and only when a checked merged interface with base types is declared in
more than one file (lib files count when they are checked: without `skipLibCheck` / `skipDefaultLibCheck` the lib
interfaces build it). API builds and external project/API pools retain the per-file rule in default mode,
including single-file diagnostics requests. Go-compatible history is unchanged in both kinds of pool.

The lazy association build runs sequentially. It can be entered while a checker is held, so it must not submit work
to the worker pool: a nested wait could execute another file's check that needs the held checker or the same
partly-initialized slot table. The initial 300-program API stress probe completed at 1, 2, 4 and 8 workers without
reproducing a hang; the lock/reentrancy hazard is source-derived. The API regression checks TS2320, TS2430 and TS2411
in all declaring files, whole-program and single-file requests, with a bounded wait.

Incremental builds must also account for this program-wide placement. With `merged-interface-check-site`, adding an
unrelated module first in the root list (100 exported numeric constants) moves `b.ts` onto `a.ts`'s virtual checker.
The cached build used to keep the TS2320 in `b.ts`, while a clean build only reported `a.ts`. Moving the same module
to the end of the root list reproduced the reverse error: a cached build omitted `b.ts`. Default mode now refreshes
semantic diagnostics of files containing interfaces when a program file, reference set, file order or lib-check
option changes. A package export alias can also change the import multigraph from `[a,b,a]` to `[a,b,b]` while all
those inputs stay the same. Reuse additionally requires equal virtual check slots. Default-history build info now
stores `tsrsCanonicalCheckSignature`, a `go4-v1:` prefix and a 128-bit xxh3 hash of the virtual checker indices in
program order (little-endian u32, `u32::MAX` for an unchecked file). The index in that sequence supplies each slot's
visit position. The existing file table supplies the file order, and source/version and lib-check option changes
still invalidate interface caches even if the slots match. This hashes the actual association result, including
the effect of repeated resolved import edges. The serial, lazily initialized slot table is shared between cache
validation and diagnostic reporting. The same compiler program retains
its snapshot fast path; API builds and external API/LS pools retain per-file reporting.

Both build-info writers include the optional field only for default CLI canonical reporting. An absent, malformed or unknown-version
signature causes one conservative refresh of checked interface diagnostics, including cached absence, and a rewrite
with the current proof. Subsequent unchanged loaded programs reuse those caches and neither serialize nor rewrite
build info. Custom writers receive real emissions through the existing callback; no byte-equality write suppression
is needed. Other files' semantic caches and emit caches keep their existing reuse rules. Go-compatible history
never writes the field. As a tsrs-only exception to Go's build-info reader, it rejects any cache containing the
field, including an empty, malformed or unknown-version value, before converting it to a snapshot. Otherwise a
default-to-Go switch at eight checkers can reuse two cached TS2320 sites where a fresh Go build reports four.
The existing cold path rewrites the cache in Go's format; subsequent native Go caches retain their usual reuse.
This checks field presence in the already parsed JSON, with no second parse or virtual-slot work. Default CLI
validation is unchanged. A change to canonical placement semantics must bump the signature version before caches
from that policy can be reused.

API build orchestrators opt out through `ProgramOptions::canonical_diagnostics`, using their existing scoped API
mode rather than a process-global compatibility switch. External project pools also opt out of both canonical
reporting and its signature. The API build-info reader rejects every present CLI tag before the up-to-date check,
so a tagged cache cannot hide an API diagnostic or leak CLI metadata back to the caller. API caches remain in Go's
format and reuse normally. Switching either way in memory invalidates interface diagnostics, including cached
absence; loading an untagged API cache in the default CLI uses the existing one-time signature migration. The
OS-backed regressions check these transitions, full ordered diagnostics, actual cache identity and custom writers.

The unmodified Node API build test reproduced the extra-key failure before this scoped opt-out. On local macOS,
the same pinned client and oracle comparisons then ran all 905 tests: 899 passed, four platform skips, two existing
TODOs, no hard failures, matching the Go oracle's totals. The build-info soft mismatch also disappeared; the known
profiling and transport soft mismatches remain. No goldens or harness comparisons were changed.

The initial unconditional loaded-build fallback had a measured cost. On local macOS arm64 `--release` builds, five unchanged warm runs
per binary and mode, alternating before/after order after one cold run each, matched full ordered diagnostics and
kept build-info bytes stable for each binary. A synthetic 257-file project with 16,384 derived interfaces took 14.3 -> 24.1 ms by
default (0.670 -> 1.643 billion instructions, 53.6 -> 92.9 MiB peak RSS); single-threaded, 32.1 -> 76.7 ms
(0.598 -> 1.540 billion instructions, 45.3 -> 79.5 MiB). Stagehand at f261c38b, 5,787 files, took 136 -> 402 ms
by default (9.14 -> 26.50 billion instructions, 424 -> 879 MiB). Go-compatible four-checker runs were within local
noise: Stagehand 132 -> 133 ms, 8.97 -> 8.98 billion instructions, 410 -> 410 MiB. These are bounded local
measurements with a warm filesystem, not release/CI performance guarantees. Classifying only the new program's
merges cannot establish the old cache's dependencies after a resolution change.

With the persistent slot signature, two local five-sample interleaved comparisons restore useful warm reuse.
The second comparison rebases saved caches lexically, preserving symlinks, so its first load isolates migration.
The pre-fallback binary, the inherited branch after merging current main, and the fix use the same project, flags
and build-info path. Full ordered diagnostics match throughout. The second comparison's warm medians are:

| Project / mode | Pre-fallback ms | Inherited ms | Fixed ms | Pre-fallback / fixed G instructions | Pre-fallback / fixed MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Stagehand default | 146 | 443 | 157 | 9.199 / 9.233 | 423.7 / 423.4 |
| Stagehand single | 2656 | 4289 | 2129 | 10.218 / 10.159 | 379.5 / 380.2 |
| Stagehand Go4 | 205 | 204 | 223 | 9.135 / 9.143 | 410.8 / 410.9 |
| Synthetic default | 16.3 | 31.3 | 18.5 | 0.677 / 0.678 | 53.7 / 53.7 |
| Synthetic single | 40.7 | 94.7 | 40.0 | 0.603 / 0.602 | 45.3 / 45.4 |
| Synthetic Go4 | 23.4 | 24.5 | 23.7 | 0.672 / 0.668 | 52.0 / 51.8 |

This is not an unchanged-performance claim: default wall medians are still +7.3% on Stagehand and +13.7% on the
synthetic project. The first comparison measured 138 -> 142 ms on Stagehand and 16.6 -> 21.0 ms on the synthetic
project. Stagehand single-threaded wall medians varied from 572/588 ms to 2656/2129 ms (before/fixed); Go4 varied
too. Default instruction overhead was 0.37-0.68% on Stagehand and -0.28 to +0.14% on the synthetic project; peak
memory stayed within 0.4% of the pre-fallback baseline in both sets.

Migration remains a one-time conservative check: Stagehand default took 502 ms, 26.82 G instructions and 874 MiB;
the synthetic default took 37 ms, 1.67 G instructions and 93 MiB. All subsequent warm samples kept build-info
bytes and timestamps unchanged. Stagehand warm checking recorded zero instantiations, versus millions during
migration. The incremental regression asserts actual reuse of deserialized interface cache entries, one-time
legacy/invalid-proof migration, and custom-writer behavior. The CLI regressions preserve forward/reverse alias
changes and root reordering; the API control preserves all 12 reporting sites across 48 files at 1/2/4/8 workers.
Native Go and Go-history tsrs build-info bytes match on the control, and the two Go4 benchmark caches remain
byte-identical to both baseline binaries. The 21 targeted tests, four incremental library tests, 36 regression
fixtures and 12 public-project comparisons passed; Nuxt retains four errors.

- supabase (cb4307f1) and stagehand (f261c38b), root `tsconfig.json`, default mode: identical to tsgo.
- nuxt (85b8d54f, the case above): tsgo-ref prints the four errors (both `ImportMeta` TS2320) at its default and at 32
  checkers, three at 1 and 2. tsrs prints the four in `--singleThreaded`, at 1, 2, 4, 8, 16 and 32 checkers (20 more
  runs at 32), under `random:1..8` at 2, 4 and 32 checkers, and in Go mode at 4; Go mode at 1 prints tsgo's three.
- `testdata/regressions/merged-interface-check-site` now expects tsgo-ref's default output (a.ts and b.ts);
  `merged-interface-extends-site` has a TS2430 that tsgo reports in a.ts and b.ts only and TS2411 index-constraint
  errors in b.ts, c.ts and d.ts, which every checker count reports. `merged_interface_check_site.rs` runs both
  single-threaded and at 1-4 and 8 checkers with stealing and 8 random assignments each, and Go mode at 1, 4 and 8.

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
