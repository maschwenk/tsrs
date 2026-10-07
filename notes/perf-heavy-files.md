# perf-heavy-files: what bounds the check phase on vscode, t3code-server, mui-docs and formbricks-web

At 16-32 checkers on a 64-vCPU machine the check phase of these four projects ends when one of a few files ends
(brief of 2026-10-07: vscode 0.711 s with one 0.344 s file; t3code-server 1.431 s with three files of 0.82-0.95 s;
mui-docs 0.870 s with three of ~0.5 s; formbricks-web 0.547 s with 0.245 / 0.202 s). This note says what each of those
files spends its time on and whether an exact change can remove it. One could:
notes/perf-heavy-files-infer-memo.md (https://github.com/maschwenk/tsrs/pull/145). The rest is the TypeScript algorithm,
which tsgo runs too, or work that every checker repeats and that cannot be shared without changing diagnostics.

Method: the heavy file alone in a program (a tsconfig under a scratch directory that `extends` the project's, with
`"files": [<the file>]`, `"include"` cleared and `typeRoots` pointed back at the checkout), and the heavy file on a
checker of its own inside the full program (`--checkers 2 --checkerAssignment file:<list>` with only that file on
checker 1, the list in `--listFilesOnly` order), profiled with samply per thread (4-10 kHz), plus the work census
(`--features work-census`) and per-file CPU (`TSRS_FILE_TIMES`). Mac (18 cores, shared, load 12-30) unless stated;
the Linux numbers are from the 64-vCPU runner via `.depot/workflows/perf-probe.yml`.

## Which kind of heavy each file is

| file | in its own program, deps checked first | on a cold checker in the full program | kind |
| --- | --- | --- | --- |
| vscode `mapSessionEvents.test.ts` | 0.30 s | 0.35 s | its own inference: fixed by the inference memo (below 0.1 s after) |
| t3code `apps/server/src/server.ts` | 0.013 s | 1.04 s | first use of imported Effect declarations |
| t3code `apps/server/src/binCli.ts`, `cli/theme.test.ts` | 0.002 s, 0.002 s | ~1 s each (Linux, 16 checkers) | the same |
| mui-docs `AppTheme.tsx`, `dashboard-template-theme.tsx`, `Locales.tsx` | 0.15-0.42 s | ~0.5 s each | variance measurement of MUI's theme generics |
| formbricks `lib/survey/service.test.ts` | 0.15 s | 0.25 s | one structural relation of a Prisma mock, plus Zod variances |

"First use" means: the file itself is cheap, but it is the first file in its checker that needs the types of
declarations in other files (exported `const`s whose types are inferred from Effect pipelines and generators, so their
initializers are checked), and that checker pays for them. Every checker that touches the same declarations pays again;
in a one-checker run the work is charged to the files that declare them. The file's place in the queue does not
change the cost, only when it is paid.

## t3code-server: `HandlersServices`, 3 x 178 x 178 conditional types

A cold checker spends 1.04 s on `server.ts`; 59% of it is `getConditionalType` under conditional distribution, and the
census of the isolated program puts 21% of the whole check under one alias, Effect's

```ts
type HandlersServices<Rpcs, Handlers> = keyof Handlers extends infer K ? K extends keyof Handlers & string
  ? HandlerServices<Rpcs, K, Handlers[K]> : never : never
type HandlerServices<Rpcs, K, Handler> = true extends Rpc.IsStream<Rpcs, K> ? ... : Handler extends (...) => Effect<infer _A, infer _E, infer _R> | ...
  ? Exclude<Rpc.ExcludeProvides<_R, Rpcs, K>, Scope> | Rpc.ExtractRequires<Rpcs, K> : never
```

with 178 RPC handlers. `IsStream`, `ExtractProvides` and `ExtractRequires` each distribute over the 178-member RPC union
for each of the 178 keys: 31,684 evaluations each (census: 31,657 / 31,684 / 31,684 of them `never`). Each evaluation
infers the six `infer` parameters of `Rpc<Tag, infer ...>`, instantiates `Rpc<K, P_j, S_j, E_j, M_j, R_j>` (a new type
per pair), and relates the RPC to it; `Rpc`'s variances allow a structural fallback, so the failing relation resolves
members of the new instantiation. Nothing repeats: every (RPC, key) pair is distinct, and the conditional
instantiation cache already holds each pair once.

- tsgo does the same: on the isolated program tsgo's check takes 7.9-12.0 s against tsrs's 5.1-5.8 s (same load,
  same counts: Types 1,019,545 / 1,021,479, Instantiations 8,141,994 / 8,146,658).
- An exact shortcut would have to decide `R_j extends Rpc<K, infer ...>` for mismatched tags without creating the
  instantiated extends type. Skipping a type creation changes type ids, and ids are `compareTypes`' last tie-break
  (union order). The caches in this repo keep the set of created types unchanged (union front cache, flow memo,
  inference memo), and this one cannot.
- Deciding the mismatched pairs from the `_tag` discriminant alone is notes/perf-checker-algorithms.md's A2, rejected
  because it changes the instantiation count that drives TS2589 (pinned by
  `testdata/regressions/conditional-instantiation-limit-*`); its exact form, A1, replays the relater and saved about
  0.5%, and does not apply here anyway (A1 skips type-reference extends types, and `Rpc<...>` is one).
- In the program: a map type keyed by tag (`{ [R in Rpcs as R['_tag']]: R }`) turns each lookup into one indexed
  access.

The inference memo (PR above) takes 11.9% of t3code's single-checker instructions off (repeated walks elsewhere in the
program, not this shape); `server.ts` on a 16-checker run goes from 1.147 to 1.115 s on Linux.

## mui-docs, formbricks-web, cal-diy: variance measurement, once per checker

On a cold checker, 95% of `AppTheme.tsx` is `getVariancesWorker`: MUI's `Components<Theme>` (an interface with ~300
optional entries, each built from `ComponentsOverrides<Theme>[...]`) takes 0.38 s and creates 53,231 types, its
`OverridesStyleRules` 0.28 s, `AutocompleteProps` 0.16 s. Variances are checker state, so every checker that relates
two instantiations of these generics measures them again. Thread CPU inside top-level variance measurements (a trace
build, not committed):

| project | 1 checker: measurement CPU / checker CPU | 16 checkers | most expensive at 16 checkers |
| --- | --- | --- | --- |
| formbricks-web | 0.39 / 2.90 s (13%) | 4.69 / 6.97 s (67%) | Zod 4's `ZodOptional` (15 checkers, 0.67 s), `ZodNullable`, `ZodDefault`, `ZodPrefault`, `ZodArray`, `ZodUnion` (16 each, 0.42-0.61 s) |
| mui-docs | 0.74 / 3.44 s (21%) | 4.47 / 9.22 s (48%) | `Components` (4 checkers, 2.03 s), `OverridesStyleRules` (8, 1.96 s), `AutocompleteProps` (1, 0.18 s) |
| cal-diy | 0.44 / 3.06 s (15%) | 2.79 / 7.92 s (35%) | react-select's `ClearIndicatorProps`, `ContainerProps`, `ControlProps`, react-hook-form's `UseFormResetField` (7-8 checkers, 0.22-0.28 s each) |
| vscode | 0.11 / 7.02 s (2%) | 0.57 / 10.16 s (6%) | |
| supabase-studio | 0.06 / 4.40 s (1%) | 0.37 / 8.68 s (4%) | |
| t3code-server | 0.01 / 3.28 s (0%) | 0.13 / 15.71 s (1%) | |

(At 16 checkers the CPU per measurement is higher than at 1: 16 threads on 18 shared cores.)

Tried: a table of variances shared by the checkers of a program (branch `perf/heavy-files-variance-share`, not
opened). A checker publishes what it measured from a clean start (empty variance stack, no transient taint, no
diagnostic); another checker uses it instead of measuring, or waits while it is being measured. On the Mac at 16
checkers it cut checker CPU by 33% on mui-docs (9.2 -> 6.1 s, check 0.88 -> 0.72 s), 8% on formbricks-web and 6% on
cal-diy. It is not exact:

- `TSRS_SHARED_VARIANCE=shadow` (measure locally anyway, compare) found declarations whose variances differ between
  checkers: react-hook-form's `UseFormReturn` (`[Unreliable, Unreliable]` in one checker, `[0, Unreliable]` in
  another), xstate's `TransitionConfigOrTarget` (nine parameters, `Covariant | Unreliable` vs `Independent` and more),
  MUI X's `GridColDef` (`0` vs `Contravariant`).
- With the table on, cal-diy at 4 checkers printed 40 more lines than main (a TS2322 elaboration through react-select's
  `ClearIndicatorProps`).
- Cause: TypeScript records the unreliable / unmeasurable marks in a relation cache entry only when the relation is
  computed during a variance measurement. A relation cached earlier, outside one, carries no marks, and a measurement
  that hits it does not see them. So a measurement's result depends on what its checker related before the first
  request, that is, on the context of that request. main is stable under `TSRS_CHECKER_ASSIGNMENT=random:<1..6>` on
  cal-diy (identical output). Within one checker the first request evidently comes from the same place, but a result
  carried to another checker does not.
- An exact version would need variance results that do not depend on the relation cache, a change to TypeScript's
  algorithm (worth reporting upstream: the same program can get different variances depending on check order).

The conformance suite does not see the difference (identical result trees with the table on, in shadow mode and under
three random assignments): its programs are too small to have the cached-before-measured pattern.

## formbricks-web: one relation of a Prisma mock

`lib/survey/service.test.ts` calls `prisma.$transaction.mockImplementation(...)` on `vitest-mock-extended`'s
`DeepMockProxy<PrismaClient>`: one `assignable` relation of the mock to `Omit<PrismaClient, ...>` is 192 ms of the
file's ~0.25 s (census: `_DeepMockProxy` 1,150 mapped-symbol resolutions, 204 ms). It compares every Prisma delegate's
methods through the conditional `_DeepMockProxy` template, once per checker that checks such a test. Same algorithm in
tsgo. The rest of the per-checker cost of the formbricks floor is the Zod variance measurement above.

## Not targets after all

- t3code's generated `schema.gen.ts` (57,042 lines, 0.34 s alone in tsrs): module-level flow walks of 1,000-2,000
  steps (`Schema.X` property accesses walked back through thousands of `const` declarations). tsgo takes 3.5 s on it,
  ten times tsrs (the flow memo), so it is not a port problem and not on the 16-checker floor.
- vscode's next heaviest files after the memo (`copilotAgentSession.test.ts` 0.137 s at 16 checkers on Linux,
  `agentService.test.ts`, `agentHostChatContribution.test.ts` ~0.1 s): flat profiles, no repeated shape.
- mui-docs `useMemo(() => createTheme(...))`: `removeSubtypes` over the return union compares against `Partial<...>`
  of a 1,025-2,048-key type (two resolutions of 81 ms each); TypeScript's algorithm, as in notes/perf-checker-algorithms.md
  row 5.

## Reproduce

```sh
# a heavy file on a checker of its own
tsrs -p <project> --listFilesOnly > files.txt
awk '{print ($0 ~ /apps\/server\/src\/server\.ts$/) ? 1 : 0}' files.txt > assign.txt
samply record -r 4000 --unstable-presymbolicate -s -o p.json.gz \
  tsrs -p <project> --noEmit --incremental false --checkers 2 --checkerAssignment file:assign.txt
# per-file CPU at 16 checkers
TSRS_FILE_TIMES=ft.tsv tsrs -p <project> --noEmit --incremental false --checkers 16
```
