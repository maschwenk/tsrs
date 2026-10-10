# perf-next-heavy-files: what bounds next-packages-next's check phase, and checking one declaration file on several checkers

The README bench (64 vCPU, `bench/results/2026-10-07-056d39463583.json`) has tsrs ahead of `bun check` by 1.7-2.6x on
every Bun benchmark project except `next-packages-next` (vercel/next.js `packages/next`, 2,886 files): 0.382 s against
0.409 s. Its check phase took 0.314 s at 32 checkers against 1.53 s single-threaded, a 4.9x speedup where vscode gets 20x.
This note finds the one file behind that, shows that its work is TypeScript's algorithm (tsgo does the same work and
takes twice as long), and lands a scheduling change: a heavy checked declaration file is checked in statement ranges
on several checkers. On the 64-vCPU runner the check phase goes from 0.399 to 0.114 s at 32 checkers and from 0.399 to
0.192 s at 16, with output byte-identical. Base: main 6b63627 (PR 145, the inference memo, included).

## 1. The tail is one file

`TSRS_FILE_TIMES` at 1, 16 and 32 checkers (Mac, `cargo build --release`; Linux numbers from the probes in section 5):

| file | nodes | 1 checker CPU s | 32 checkers |
| --- | --- | --- | --- |
| `node_modules/@modelcontextprotocol/sdk/dist/esm/types.d.ts` | 248,820 | 0.391 | 0.376 s CPU, alone on checker 0 for the whole pass (Linux: 0.393 s) |
| `@types/babel__traverse/index.d.ts` | 12,623 | 0.090 | 0.089 |
| `@babel/types/lib/index.d.ts` | 72,300 | 0.071 | 0.070 |
| `lib.dom.d.ts` | 113,504 | 0.052 | 0.064 |
| `src/server/config-schema.ts` | 5,857 | 0.038 | 0.049 |

`TSRS_ASSIGNMENT_STATS=times` at 32 checkers on Linux: checker 0 runs that one file in 0.39 s; the other 31 checkers
finish their 1-200 files each in 0.07-0.10 s. So it is not a chain of files on one checker and not per-checker
duplication: it is one file, 23% of a one-checker check (0.39 of 1.73 s of file CPU), and the check phase cannot end
before it does. It is already the first file of its checker (`heavy_files_first`; its static weight, 5.4% of the
program's, is the largest), so ordering cannot help, and stealing moves whole files.

next's `tsconfig.json` does not set `skipLibCheck`, so the declaration files of its dependencies are checked; 1,202 of
the 2,885 checked files are declaration files.

## 2. What the file spends its time on

`types.d.ts` is the MCP SDK's generated declaration of its Zod 3 schemas: 54,851 lines, 2.9 MB, 3,460 `z.ZodObject<...>`
references whose type arguments are type literals spelled out in full, each object shape written three times
(`ZodObject<{...}, "passthrough", ZodTypeAny, objectOutputType<{...}, ...>, objectInputType<{...}, ...>>`).

A cold-checker profile (samply, 4 kHz, the file alone on checker 1 with `--checkerAssignment file:`; inclusive):
`checkTypeReferenceNode` 92%, `checkTypeArgumentConstraints` 74%, `structuredTypeRelatedTo` 72%,
`propertiesRelatedTo` 70%, `signaturesRelatedTo` 52%, `instantiateType` 42%, `getObjectTypeInstantiation` 27%. The work
census (`--features work-census`, the file alone in a program whose other files are its dependencies) names the
relation: 59% of the check is `assignable -> ZodType` (3,026 relations, 191 ms) and 24% `assignable -> ZodRawShape`
(6,854). Every `ZodObject<{...}>` reference checks its shape literal against `T extends ZodRawShape`
(`{ [k: string]: ZodTypeAny }`), which relates each property type (`ZodOptional<...>`, `ZodObject<...>`, `ZodUnion<...>`)
to `ZodTypeAny` = `ZodType<any, any, any>`. These are derived classes against their base, so the relation is
structural: every member of `ZodType` (`refine`, `refinement`, `superRefine`, `or`, `transform`, `pipe`, ... about 40,
each 2,761-2,767 times), generic signatures instantiated and compared.

Nothing in it repeats with the same inputs. Each shape literal is a fresh anonymous type, so every `ZodObject<{...}>`
built from it is a new instantiation, and the relation cache already answers the pairs that do repeat (identical
leaves such as `ZodOptional<ZodString>` are the same type). The 3,026 sources are distinct. The inference memo does
not apply (no inference). Relating a derived generic to its base by variances (`TSRS_DERIVED_VARIANCE`) would cut it,
but it is not exact (docs/DEBUGGING.md) and stays off.

tsgo does the same work and is slower: on the isolated program (the file and its dependencies) tsgo-ref takes
0.670-0.689 s of check time against tsrs's 0.334-0.347 s, with the same counts (Types 411,592 / 409,910,
Instantiations 1,895,810 / 1,889,282; the difference is lazy members). So the cost is the file's own, as t3code's
Effect types were (notes/perf-heavy-files.md). What remains is the scheduling: one checker does all of it.

## 3. Checking a declaration file in statement ranges

`crates/tsrs_compiler/src/splitcheck.rs`. In the type-check pass with stealing, a checked declaration file whose static
weight is at least 40% of an average checker's share is cut into contiguous statement ranges of about equal text,
each about 1/8 of a share (7 ranges for the MCP file at 16 checkers, 14 at 32). The checker that owns the file checks
the first range. Each other range is a queue item at the front of the least loaded other checker
(`plan_splits`), which checks only those statements and the nodes they deferred
(`Checker::check_source_file_piece`) and hands back its diagnostics for the file. A range nobody has started when the
owner has finished its own is run by the owner. The owner waits for the ranges in flight (they never wait on anything,
so this cannot deadlock), adds their diagnostics to its collection (`add_piece_diagnostics`) and then runs the normal
per-file step, whose `checkSourceFile` does the file-level checks (grammar, deferred nodes, module exports) and skips
the statements. Later passes over the file stay with the owner.

Why the output is the same. A file's diagnostics are its checker's collection for the file, deduplicated
(`equal_diagnostics`) and sorted. Output already does not depend on which checker checks a file or in which order a
checker visits files (notes/perf-order-independence.md, checked with `random:<seed>` assignments): what is reported
for a node is a function of the program, not of what the checker resolved before. A range is the same thing at
statement granularity: a checker visiting some statements of a file without the others. What is new is state shared
between the statements of one file, which a whole-file check always sees in source order. For top-level statements
the only node they share is the source file. Its node links carry emit flags (unused in ambient code), a parameter
flag, and `hasReportedStatementInAmbientContext`, which reports TS1036 "Statements are not allowed in ambient
contexts" on the first executable statement of a block only. Two ranges would each report their first one. The forced
mode below found exactly that (four `nodeModulesDeclarationEmitWithPackageExports` variants gained a second TS1036),
so a file is split only if every top-level statement is a declaration; only executable statements run that check.
The per-symbol "check once" guards a declaration's check runs either pick the first declaration in source order
(overloads, merged exports: the same declaration whichever checker gets there) or report the same diagnostics
whichever checker runs them (identical type parameters of merged interfaces reports on every declaration), which the
deduplication absorbs. Only declaration files are split: no function bodies, so no control flow, unreachable-code or
unused-identifier checks, and nothing later passes need from the checker that checked them.

It is not a cache, but it has a shadow mode in the same spirit (`TSRS_SPLIT_FILES=shadow`): the ranges run as usual,
then the owner also checks the other checkers' ranges itself and reports the file as one checker finds it, and panics
if a range's checker found a diagnostic the owner did not, or the owner found one in a range that no other checker
did. Together those mean the split output equals the owner's. `force:<k>` splits every checked declaration file with at
least two statements into k ranges whatever its weight, so the conformance suite exercises it.

## 4. Verification

- Conformance suite in the default checker mode (`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`, 4
  checkers per test, stealing), errors plus `.types` and `.symbols`: `TSRS_SPLIT_FILES=force:2,shadow`,
  `force:3,shadow`, `force:5`, `force:5,shadow`, `force:7`, `force:9,shadow` all give pass, codes, fail, crash,
  `types-*` and `symbols-*` lists identical to `TSRS_SPLIT_FILES=0` (13,458 / 12,778 / 12,778), no panics. With
  `stats`: 331-345 test programs split 626-653 declaration files into 1,306-1,796 ranges, of which 365-679 ran on
  another checker.
- `testdata/split-check` (`cargo test -p tsrs_cli --test split_check`): a declaration file with errors in most
  statements (constraint violations, merged interfaces with different type parameters, redeclared variables,
  overloads, an unresolved name) and one with two top-level executable statements; tsgo-ref's output in every mode
  (off, default, forced into 2, 3, 5, 9 ranges, with and without shadow) at 4 checkers. With the declaration-only rule
  removed the second case prints TS1036 twice and the test fails.
- next at 1, 4, 16 and 32 checkers in every mode (off, on, shadow, `force:2`, `force:5`, `force:2,shadow`,
  `force:7,shadow`): one output. Linux probes: one output over 86 next runs and 40 webpack runs.
- vscode, xstate-main, webpack, mui-docs, cal-diy, formbricks-web, supabase-studio, t3code-server at 16 and 32
  checkers, split on against off: identical output. Only webpack splits anything (lib.dom.d.ts at 32 checkers); the
  others set `skipLibCheck`.

## 5. Measurements

Depot `depot-ubuntu-24.04-64` (64 vCPU), `cargo build --release`, `--noEmit --incremental false --extendedDiagnostics
--pretty false`. Each probe runs one binary with the split off (`TSRS_SPLIT_FILES=0`, which is main's behaviour) and on,
interleaved, under `perf stat -e instructions:u` and `/usr/bin/time`; medians of 10 (3 for one checker). Probe 3 is
the final rule (40%); `TSRS_SPLIT_MIN_SHARE=75` is the first rule, kept as a column.

Probe 3 (run wkjml1s3tg), next-packages-next:

| checkers | check s, off | check s, on (40%) | at 75% | wall s, off / on | instructions G, off / on | peak MiB, off / on |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 1.801 | 1.815 (no split) | | 1.86 / 1.88 | 23.88 / 23.89 | 535 / 535 |
| 8 | 0.398 | **0.294** | 0.398 | 0.47 / 0.36 | 28.05 / 28.82 (+2.7%) | 659 / 677 |
| 9 | 0.399 | **0.261** | 0.406 | 0.47 / 0.33 | 28.64 / 29.11 (+1.6%) | 678 / 695 |
| 16 | 0.405 | **0.173** | 0.187 | 0.48 / 0.24 | 30.69 / 31.12 (+1.4%) | 752 / 762 |
| 32 | 0.401 | **0.120** | 0.112 | 0.47 / 0.19 | 33.11 / 33.85 (+2.2%) | 862 / 881 |
| 64 | 0.402 | **0.110** | 0.107 | 0.48 / 0.19 | 35.72 / 37.34 (+4.5%) | 1,033 / 1,079 |

webpack, the one other project that splits (it checks declaration files; `lib.dom.d.ts` and `types.d.ts` reach the
threshold), same probe:

| checkers | check s, off / on (40%) / 75% | instructions G, off / on | peak MiB, off / on |
| --- | --- | --- | --- |
| 16 | 0.123 / 0.125 / 0.125 | 20.15 / 20.66 (+2.5%) | 612 / 625 |
| 32 | 0.094 / 0.094 / 0.097 | 24.47 / 24.74 (+1.1%) | 773 / 778 |
| 64 | 0.081 / 0.081 / 0.080 | 30.05 / 31.30 (+4.2%) | 1,022 / 1,062 |

One diagnostic output over all 156 next runs and all 90 webpack runs.

- The check phase now ends with the other checkers, not with the MCP file. At 32 checkers (`TSRS_FILE_TIMES` and
  `TSRS_ASSIGNMENT_STATS=times`, probe 3) the ranges other checkers ran took 0.02-0.11 s each, the owner's item ends at
  0.12 s (0.07 s of it checking, the rest waiting for the last range), and the 32 checkers finish at 0.08-0.12 s,
  against 0.39 s for the whole file on one checker before. At 8 checkers the file is cut into 4 ranges: two ran on other
  checkers (0.07 and 0.20 s), the owner checked its own and the fourth (0.16 s), and the 8 checkers finish at
  0.28-0.30 s: balanced.
- The extra instructions (1.4-4.5% on next, 1-4% on webpack) are the work each range's checker repeats: the types
  its statements need that another checker also resolved. They run on checkers that would otherwise be idle; check
  time is the same or lower in every cell measured. Single-
  threaded runs never split (no stealing), so the regression counter's numbers do not move (23.88 / 23.89 G).
- Peak RSS grows by 10-46 MiB on next (the ranges' checkers hold their own instantiations of the Zod types), 5-40 MiB
  on webpack.
- At 4 checkers (the 8-vCPU bench) nothing splits: the MCP file is 22% of a share and is not a tail there.

Probe 2 (run vgtr5ln1zv), the 75% rule, the same shape: next 0.399 -> 0.192 s at 16 checkers, 0.399 -> 0.114 at 32,
0.407 -> 0.117 at 64, unchanged at 8 (0.393 / 0.396); webpack 0.121 / 0.122 at 16 and 0.094 / 0.096 at 32. Probe 1
(run b02x8ft2fc) was the first version (pieces of 1/4 of a share, no threshold): 0.401 -> 0.201 s at 16 and
0.400 -> 0.141 at 32; it showed the ranges were too coarse at 16 (one range took 0.21 s while the owner's took 0.13).

Divisor of the range size, Mac, next, one run each (the largest MCP range's CPU): 1/4 of a share 0.17 s at 16
checkers, 1/8 0.086 s, 1/16 0.088 s; at 32 checkers 0.085 s for all three. 1/16 splits 25 files at 32 checkers
(1/8: 9) for no gain.

## 6. Gates

`cargo check --workspace` clean, `tools/lint/ratchet.py` ok (none new), `tools/lint/source.py` ok,
`cargo test -p tsrs_checker -p tsrs_compiler` ok, `cargo test -p tsrs_cli --test split_check` ok.

pr-verify (`depot ci run --workflow .depot/workflows/pr-verify.yml`, run n1187b8sgs, main 0257e3e against this branch,
all 17 bench projects at 1, 4, 16 and 32 checkers plus a poisoned-arena run): identical diagnostics in 102 of 102 cells.
next-packages-next wall 0.46 -> 0.24 s at 16 checkers and 0.47 -> 0.19 s at 32 (check 0.40 -> 0.17 / 0.12),
unchanged at 1 and 4; single-threaded instructions +0.000-0.005% on every project. The other projects' wall medians
are within -1.1% to +1.8% (webpack and next-root +3-4% at 16-32 checkers, 0.01 s, where they split lib.dom.d.ts; mikro-orm
+6% at 32 and -2.4% at 16, which does not split anything). Peak RSS at most +3.8% (webpack, 16 checkers).

Status (2026-10-10): a later rule raised the bar for default library files. `plan_splits` splits one only when it
weighs `splitcheck::LIB_MIN_SHARE_PERCENT` = 80% of a share (splitcheck.rs, checkerpool.rs `plan_splits`), because
splitting lib.dom.d.ts on webpack at 16 checkers, where it is half a share, cost 3-4% of wall. At 32 checkers on
webpack and next-packages-next it is about a whole share and is still split. Other declaration files keep the 40%
threshold above.

## 7. Not done

- The split is static: which files and how many ranges come from the static weight, which misjudges declaration files
  both ways (the MCP file costs four times its weight, lib.dom.d.ts about its weight). Stealing statement ranges from a
  file in progress, the way files are stolen now, would need no threshold; it is more machinery than one file needs.
- Source files are never split: their statements share flow, unreachable-code and unused-identifier state, and later
  passes (declaration emit) expect the file's checker to have checked all of it.
- The work itself: 3,026 structural relations of Zod classes to `ZodTypeAny`. A shortcut would have to decide those
  without comparing members, which is the inexact derived-variance relation.

## Reproduce

```sh
python3 bench/run.py --setup-only --projects next-packages-next
cd bench/.work/solutions/next-packages-next
TSRS_FILE_TIMES=ft.tsv TSRS_ASSIGNMENT_STATS=times tsrs -p packages/next --noEmit --incremental false --checkers 32
TSRS_SPLIT_FILES=0 tsrs -p packages/next --noEmit --incremental false --extendedDiagnostics --checkers 32   # before
TSRS_SPLIT_FILES=stats tsrs -p packages/next --noEmit --incremental false --extendedDiagnostics --checkers 32
# the heavy file on a checker of its own, profiled
tsrs -p packages/next --listFilesOnly | awk '{print ($0 ~ /sdk\/dist\/esm\/types\.d\.ts$/) ? 1 : 0}' > assign.txt
samply record -r 4000 --unstable-presymbolicate -s -o p.json.gz \
  tsrs -p packages/next --noEmit --incremental false --checkers 2 --checkerAssignment file:assign.txt
```

The Linux numbers come from `.depot/workflows/perf-probe.yml` runs with a probe script that ran the same binary with
`TSRS_SPLIT_FILES=0` and on, interleaved (not committed; the runs and their artifacts are named in section 5).
