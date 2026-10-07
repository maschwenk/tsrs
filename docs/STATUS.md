# Status

## 2026-10-07: 0.6.0 release

A performance and memory release; the TypeScript source pin remains `b85298b6a81f772d080b0455de0ca9d744cd6fd6`
(7.1.0-dev.20260929) and the published package set is unchanged. README bench at commit `313e834a19e0`
(`bench/results/`), default mode on `depot-ubuntu-24.04-64`: vscode 0.60 s -> 0.54 s and 2.87 GiB -> 2.73 GiB against
the 0.5.0 table. Emitted output is unchanged against tsgo at the same pin, and so are diagnostics in
`--checkerAssignment go` mode; the default mode differs in one case (below).

Checker and memory: the checker memoizes repeated inference walks by their inputs (#145, vscode -7.2% and t3code -11.9%
instructions); link values and 4-byte buckets sit inline in four link stores (#136, -72 MiB at 32 checkers on vscode)
and export-star re-exports leave no dead scratch (#135, -150 MiB on formbricks at 32 checkers); the binder keeps flat
per-file label edges (#139, -5.4 MB on vscode); the bundled libs are parsed from the binary's text (#131). Checked leaf
files' trees are freed in `--noEmit` runs, on by default up to 16 checkers (#137, #159: vscode -13 to -16% peak, within
1.6% wall at 1 to 8 checkers on 8 vCPUs). A heavy declaration file is checked in statement ranges on several checkers
(#158, next-packages-next check 0.40 s -> 0.12 s at 32 checkers), and a piece hands back only the diagnostics it added
(#160), except that a default library file is split again when it weighs at least 80% of a share (#169: lib.dom.d.ts had
become the tail at 32 checkers, webpack +13% and next-packages-next +21% wall). The heavy-first threshold is 1/200 of a
checker's share (#124). At larger checker counts files are placed by the modules they share with each checker, and
thieves keep stealing from one queue (#171, instructions -3 to -15% at 16-32 checkers on the application projects);
`TSRS_MODULE_AFFINITY=off` and `TSRS_STEAL_STICKY=0` restore the old behavior (docs/DEBUGGING.md).

Diagnostics: tsgo runs the merged-interface checks (TS2320, TS2430, index constraints) once per checker, so a merged
interface declared in several files reported in files that depended on which files shared a checker. By default tsrs
now runs them at the first declaration of the symbol in each file (#165, notes/fix-history-dependent-diagnostics.md);
`--checkerAssignment go` keeps Go's placement for byte-identity.

Front end and start-up: the config is parsed on the worker pool and a CLI run creates no rayon global pool (#156, -64
threads and -50 MB peak on vscode); checkers are created while files are assigned (#162) and program diagnostics are
computed beside checker creation (#164); the file loader and collect walk make fewer refcount updates (#154, #166,
#167); include-glob listings are matched on the prefetch pool (#141); checker threads survive between work groups
(#150); the CLI ends with `_exit` once its output is written (#143); `--extendedDiagnostics` statistics no longer
spawn `ps` (#140). #143's `_exit` skipped the PGO and BOLT training profiles; #153 and #155 write them again, so this
release is built with both (+1.8-3.5% instructions in the README bench otherwise).

CI and bench: CI gates the `.types` / `.symbols` baselines, the default checker mode and `testdata/regressions`
(#132); `pr-verify` reports before/after identity and numbers for a branch on the 64-vCPU runner (#138); the bench
restores the TypeScript testdata from a cache (#142), adds Bun's benchmark projects and drizzle-orm (#144) and
BOLTs the measured binary as `release.yml` does (#161).

## 2026-10-07: 0.5.0 release

A performance and memory release for machines with many cores; the TypeScript source pin remains
`b85298b6a81f772d080b0455de0ca9d744cd6fd6` (7.1.0-dev.20260929) and the published package set is unchanged. README
bench at commit bd86c6d7a6ba (`bench/results/2026-10-07-bd86c6d7a6ba.md`): on Depot `depot-ubuntu-24.04-8` in the
default mode (4 checkers there) tsrs type-checks 3.6-9.4x faster than tsgo 7.0.2 with 0.13-0.46x its peak memory
(vscode 11.32 s -> 2.78 s, 7.75 GiB -> 2.14 GiB). On `depot-ubuntu-24.04-64` with no thread flags tsrs (32 checkers)
is 3.4-16.1x faster than tsgo (vscode 9.65 s -> 0.60 s) and 1.3-2.4x faster than `bun check` 1.4.3-canary on its 64
threads (vscode 0.84 s -> 0.60 s at the same 2.87 GiB peak; mui-docs left out, where bun takes 40 s and reports 23
errors); on the four application projects tsrs peaks at 1.2-1.9x bun's memory. Diagnostics and emitted output are
unchanged against tsgo at the same pin.

Default checker count: half the available cores clamped to 4..32 (was 4..8), still at most one checker per 32
type-checked files (#120, notes/perf-checker-64.md). It stays 4 on 8 vCPUs and is 32 on 64 or more, which takes
vscode from 1.73 s to 0.82 s on the 64-vCPU machine for +0.9 GiB peak. Large programs on wide machines therefore use
more memory by default than in 0.4.0; `--checkers N` sets the count.

Checker and front end (#120): each checker starts with its heaviest files (above 1% of its share), then program
order, and thieves take from the back (notes/perf-checker-64.md); `is_type_related_to` answers the identity relation
without a relater when the normalized kinds differ; program construction scales past 16 threads (vscode on 64
threads 0.26 s -> 0.11 s, the file loader's lock contention removed; notes/perf-frontend-64.md). Memory: link stores
in 128-id groups in the arena, and reads of reference kinds make no links (#120, notes/mem-64.md); on Linux mimalloc
is built with `no_thp`, so its heap no longer takes a 2 MiB huge page per partly filled thread page, and the parse
pool is capped at 32 threads (#121, notes/mem-no-thp.md): vscode on the 64-vCPU machine peaks at 2.94 GiB instead of
4.13 GiB at the default (-29%) and 25-33% lower at 4 to 64 checkers, for 6-10% more wall time (0.547 s -> 0.602 s at
the default).

Bench: the README bench measures each project on its own machine at the same time (push to results 13 min 51 s, was
26 min 35 s), adds the application projects cal.diy, Formbricks, Supabase Studio and T3 Code (#116), `--checkers 8`
and `--checkers 64` tables, a 64-vCPU default-mode table with `bun check`, and bolds speedups from 5x and memory
ratios at 0.25x or less (#117, #122, #126, #127). bench/compare.py runs the 64-thread head-to-head with tsgo, the
7.1-dev nightly and `bun check` (#119).

## 2026-10-05: 0.4.0 release

A performance and memory release; the TypeScript source pin remains `b85298b6a81f772d080b0455de0ca9d744cd6fd6`
(7.1.0-dev.20260929) and the published package set is unchanged. On the typescript-benchmarking suite against
tsgo 7.0.2 (Depot `depot-ubuntu-24.04-8`, commit 32318a02bd47, README table) tsrs type-checks 3.5-9.4x faster with
0.15-0.86x the peak memory in the default 4-checker mode (vscode 11.75 s -> 2.84 s, 7.66 GiB -> 2.38 GiB; mui-docs
13.60 s -> 1.45 s, 8.20 GiB -> 1.19 GiB), and 2.8-4.6x faster with 0.21-0.43x the memory under `--singleThreaded`.
Diagnostics and emitted output are unchanged against tsgo at the same pin.

Checker: threads steal unstarted files from the busiest checker (#93; -11 to -17% wall at 4-8 checkers on the
38k-file codebase), diagnostics and `.d.ts` no longer depend on the checker count or assignment (#92;
`--checkerAssignment go` keeps tsgo's history-dependent output, which the tsgo-baseline harnesses use), complete
flow-analysis results are remembered across walks (#97; -2 to -5% instructions on vscode and webpack), a front cache
for small union constructions (#100), and checker threads take node and symbol ids in blocks (#94). Relating derived
generics to their generic base by variances landed behind `TSRS_DERIVED_VARIANCE` (#96); differential fuzzing found
nine disagreements and added guards 4-6, and the verdict is to keep it **off** by default (#98,
notes/fuzz-derived-variance.md).

Memory: compressed 32-bit pointers on unix and scoped arenas (perf round 2, #37-#51; cold 1-checker peak -16% on the
38k-file codebase), transparent huge pages for the compressed arena's thread chunks on Linux (#89; sys time
-26 to -36%, page faults 0.9M -> 30K), and the checker-heap stack (#99, #105-#110: lazy member tables in the arena
with one-word slices, CacheHashKey caches in a 20-byte PackedMap, a string literal cache without key copies, flat
export indexes, 16-bit link store slot offsets): peak -3.3% / -6.4% at 1 / 8 checkers on the 38k-file codebase with
instructions flat (-0.004% with PGO). `TSRS_HEAP_CENSUS=1` with `--extendedDiagnostics` prints what each checker
owns; the heap sampler symbolizes on Linux. The emit print backlog is bounded at 2,048 files.

Safety and build: a panic=abort crash in the arena is fixed (#103; a freed `&[T]` parameter let LLVM drop the
free-list link) and guarded by a source check, Miri tests and an arena-safety CI job (#104, leak checking off because
the arena leaks its chunks by design). The Linux release binaries are BOLT-optimized on top of PGO + LTO, with the
gates run on the optimized binaries (#101; -1 to -3% wall; notes/perf-build-level.md records the build-level space:
text huge pages, panic=abort, allocators, opt-level, PGO training, #102). CI and the Linux Node API jobs run on
Depot CI, macOS stays on GitHub; nightly differential parser fuzzing against tsgo (#76); the bench flags instruction
and peak-memory regressions on main (#74); Rust is pinned at 1.99.0 (#78); the clippy ratchet and the unused-code
ratchet were paid down across the workspace (#53, #57-#72, #79-#87) and generated code is checked against its
generators. notes/perf-round3.md indexes what landed, what was rejected and what is open.

## 2026-10-03: 0.3.0 release

The npm package includes default JavaScript/declaration/source-map emit, incremental builds and project references,
plus the synchronous and asynchronous Node API. Type checking without output requires `--noEmit`.
The TypeScript source pin remains `b85298b6a81f772d080b0455de0ca9d744cd6fd6` (7.1.0-dev.20260929).

Validation carried forward from the merged Node API candidate: both CI workflows passed, packed consumers passed
on Node 16/18/20/22/24, and separate Go/Go/Rust captures each passed 902 tests with three known upstream TODOs.
Successful responses matched for 163 of 172 methods; five remain inconclusive, one has only error-path evidence,
and three profiling methods are unsupported. See [Node API limitations](NODE_API.md) and [emit progress](EMIT.md).
The release workflow checks installed packages against the actual PGO binaries on every supported platform before
publishing them.

## 2026-09-30: first end-to-end runs

All checker bodies are merged and the workspace compiles. Type/symbol/signature printing (`printer.rs`) still has
**temporary placeholder bodies** (`type#<id>`, bare symbol names) until the node-builder port lands, so message text
that mentions types does not match yet.

Conformance (`tsrs-test run --suite all`, 15,197 variants, 1,735 skipped as unsupported by the harness, ~22 s):

| class | count | meaning |
| --- | --- | --- |
| pass | 10,479 | baseline byte-identical |
| codes | 2,871 | same (file, line, col, code) set; text differs (placeholder type names) |
| fail | 111 | different diagnostics (mostly declaration-emit TS4xxx/TS2883/TS9xxx and TS5055, not ported) |
| timeout | 1 | `compiler/intersectionConstructorReductionCrash` |
| crash | 0 | |

"The private monorepo" throughout this file is a 38k-file private TypeScript monorepo (5.9M lines) used as the
end-to-end test project; its source is not part of this repository, only the measurements.

The private monorepo (`tsrs -p <project>`, release build, single checker thread, placeholder printer), against
`tsgo-ref --singleThreaded` at the same commit:

| | tsrs | tsgo (single-threaded) |
| --- | --- | --- |
| errors | 0 | 0 |
| files | 37,942 | 37,942 |
| symbols | 25,966,437 | 25,973,354 |
| types | 9,637,422 | 9,639,962 |
| instantiations | 44,879,286 | 44,884,281 |
| check time | 27.1 s | 33.1 s |
| total wall | 31.1 s | 41.0 s |
| peak RSS | 20.5 GB | 18.0 GB |

Counter differences (<0.03%) are expected while the node builder is a placeholder (Go creates a few types while
printing speculative error messages).

## 2026-09-30 (later): compiler-level fixes, node-builder skeleton merged

Conformance on main: **10,519 pass / 2,876 codes / 67 fail / 0 timeout / 0 crash** (suite wall time ~11 s).

Of the 67 fails: 59 are declaration diagnostics (TS2883/TS4xxx/TS9xxx… from the declaration-emit pipeline, not ported:
~7k Go lines in `transformers/declarations`, `emitresolver.go`, isolatedDeclarations); 6 are placeholder-printer effects;
2 (`mutuallyRecursiveInference`, `recursiveMappedTypes`) depend on the Go test harness running JS emit before collecting
diagnostics (the `tsgo` CLI itself reports what we report).

The private monorepo error injection (same 15 errors appended to 3 files in a clone): identical (file, line, col, code) sets and
exit code versus the reference. A mutation-testing campaign is in progress (`tools/mutate`, `notes/fix-mutation.md`).

In flight: node-builder body wave (branches `body/nb-1..6`) replacing the placeholder type printer.

## 2026-09-30 (later): parallel checking on by default

Checking runs like Go's: 4 checkers by default (`--checkers N`, `--singleThreaded` = 1), Go's deterministic file
assignment, one OS thread per checker; the conformance harness stays single-threaded unless
`TS_TEST_PROGRAM_SINGLE_THREADED=false` (Go testutil). Threading contract and the `TSRS_CHECK_SHARED=1` checker:
PORTING.md "Threading". Conformance with parallel programs is identical to single-threaded (same pass list; every
non-passing output equal modulo placeholder type/symbol ids).

The private monorepo (release, 18-core machine shared with other agents, one run each unless noted; peak = `peak memory footprint`):

| | wall | check time | peak memory | symbols | types |
| --- | --- | --- | --- | --- | --- |
| tsgo-ref `--singleThreaded` | 45.7 s | 38.8 s | 16.7 GB | 25,973,354 | 9,639,962 |
| tsgo-ref default (4 checkers) | 26.5 s | 21.8 s | 24.4 GB | 39,704,001 | 16,200,921 |
| tsrs `--checkers 1` | 27.9 s | 25.3 s | 18.2 GB | 25,966,437 | 9,637,422 |
| tsrs `--checkers 2` | 20.3 s | 17.9 s | 21.4 GB | 31,661,779 | 12,357,499 |
| tsrs default (4 checkers) | 14.9 s | 12.6 s | 26.2 GB | 39,696,532 | 16,197,557 |
| tsrs `--checkers 8` | 11.8 s | 9.3 s | 32.7 GB | 51,526,797 | 21,735,474 |

0 errors in every run (5 consecutive default runs, identical counters). With 4 checkers the per-checker check times
are 9.4-12.7 s (Go's assignment); config + parse + bind take ~2 s.

## 2026-09-30 (later): node builder live (real type/symbol/signature printing)

The node-builder wave (`body/nb-1..6`, merged via `nb-integrate`) replaced the placeholder printer: `typeToString` & co.
now run Go's pipeline (printer.go -> NodeBuilder -> nodecopy/pseudo type node builder/symbol accessibility/module
specifiers -> `tsrs_printer`).

Conformance: **13,398 pass / 2 codes / 62 fail / 0 timeout / 0 crash** (was 10,519 / 2,876 / 67; no previous pass
lost). 60 of the 62 fails are declaration-emit diagnostics (TS4xxx/TS9xxx/TS7056/TS2883…, pipeline not ported). The
other 2 fails (`mutuallyRecursiveInference`, `recursiveMappedTypes`) and both codes tests are harness artifacts: the Go
harness runs JS emit, whose const-enum inliner type-checks every property access, before collecting diagnostics, so
errors are first reported from a different current node; the `tsgo` CLI reports what we report. Details and the
clustering tool (`tools/cluster-diffs.py`): `notes/fix-nb-integrate.md`.

The private monorepo (release, 0 errors): counters now equal the reference exactly; peak memory unchanged versus main (19.5 vs
19.4 GB single-threaded, measured back to back).

| | symbols | types | instantiations | check time | wall | peak memory |
| --- | --- | --- | --- | --- | --- | --- |
| tsrs `--singleThreaded` | 25,973,354 | 9,639,962 | 44,884,281 | 26.4 s | 34.0 s | 19.5 GB |
| tsgo-ref `--singleThreaded` | 25,973,354 | 9,639,962 | 44,884,281 | 38.8 s | 45.7 s | 16.7 GB |
| tsrs default (4 checkers) | 39,704,001 | 16,200,921 | 89,981,648 | 13.2 s | 18.1 s | 28.3 GB |
| tsgo-ref default (4 checkers) | 39,704,001 | 16,200,921 | | 21.8 s | 26.5 s | 24.4 GB |

## 2026-09-30 (later): `.types` / `.symbols` baselines

The harness now also generates and compares the reference `.types` (printed type of every expression/declaration)
and `.symbols` (resolved symbol + declaration locations of every identifier) baselines, ported from Go's
`type_symbol_baseline.go`: `tsrs-test run --baselines types,symbols` (lists `types-<class>.txt` /
`symbols-<class>.txt`, artifacts `<name>.{types,symbols}.{actual,diff}`), `tsrs-test show <name> --types`,
`tools/cluster-diffs.py --baseline types`. The default run (errors only) is unchanged.

Of the 12,779 variants that produce these baselines (the rest are harness-skipped or `@noTypesAndSymbols`):
**types 12,778 / symbols 12,779 byte-identical** (first run: 12,776 / 12,779; the fix was the printer's escaping of
the internal-symbol-name prefix). The remaining `.types` mismatch, `declarationEmitObjectAssignedDefaultExport`, follows
from its missing declaration-emit diagnostic (Go's walker prints `any` differently when the test has errors). No
checker divergence surfaced. Details: `notes/fix-types-baseline.md`.

## 2026-09-30 (later): declaration diagnostics

The declaration-emit pipeline that `tsc --noEmit` runs for its diagnostics when `declaration`/`composite` is on is
ported: crate `tsrs_declarations` (transformers/declarations: transform.go, diagnostics.go, tracker.go, util.go,
supplementalreferences.go, plus the `transformers.Transformer` base; ~4.4k Rust lines), the rest of
`checker/emitresolver.go` (`emitresolver.rs`, all 63 functions), `binder/referenceresolver.go`, the EmitContext
environment tracking and visitor hooks in `tsrs_printer`, and the compiler glue (`Program::get_declaration_diagnostics`
with Go's per-file cache, `emithost.rs`, `emitter::get_declaration_diagnostics`). The harness and the CLI already
called it at Go's points (harness: after semantic/global/suggestion diagnostics when `GetEmitDeclarations()`; CLI:
`GetDiagnosticsOfAnyProgram` under `--noEmit`, only when no earlier diagnostics were found). Design notes:
`crates/tsrs_declarations/src/lib.rs`, `resolver.rs` (the checker is lent to the transformer through a `CheckerSlot`;
Go's `checkerMu`-locking resolver methods borrow it per call), `notes/fix-decl-diagnostics.md`.

Conformance: **13,458 pass / 2 codes / 2 fail / 0 timeout / 0 crash** (was 13,398 / 2 / 62; all 60 declaration-
diagnostic fails pass, no previous pass lost; same with `TS_TEST_PROGRAM_SINGLE_THREADED=false`). `.types` now
12,779 / 12,779 (`declarationEmitObjectAssignedDefaultExport` fixed with its diagnostic). The remaining 2 fails and
2 codes are the known Go-harness emit-order artifacts.

The private monorepo (`declaration: false`, so the pipeline does not run): 0 errors, counters unchanged (39,704,001 symbols /
16,200,921 types / 89,981,648 instantiations with 4 checkers). Wall times versus main measured back to back on a
loaded machine (load average ~14) were within noise in both directions (4 checkers: 22.3/27.0 s main vs 39.4/34.2 s;
single-threaded: 58.9/56.3 s main vs 43.9/46.7 s).

## 2026-09-30 (later): the private monorepo `.types` / `.symbols` walk

`tsrs-test types-dump` and the Go oracle `tools/oracle/project-types` run the conformance `.types`/`.symbols` walk
(printed type / resolved symbol of every expression and declaration) over every non-`node_modules` file of a tsconfig
project, single-threaded, after `tsc`-style checking; `tools/project-types-compare.py` compares them. The private monorepo: 28,213
files, 16.5M baseline lines; first run **28,207 / 28,213 files identical** (18 lines in 6 files differ), symbol and
type counters equal. The 6 files print >1 MB `vi.mocked(..., { partial: true, deep: true })` types whose truncation
point depended on **symbol ids**: Go assigns ids on every `valueSymbolLinks` / `symbolNodeLinks` access (those stores
are id-keyed) and binds files in reverse order when single-threaded (LIFO work group); a well-known-symbol property's
internal name embeds the id and counts toward the node builder's length budget. Both fixed (regression
`testdata/regressions/unique-symbol-name-truncation`); conformance unchanged (13,458 / types 12,779 / symbols
12,779). Go's own ids are not deterministic across runs (map iteration), so ids agree as a multiset, not exactly.
`.symbols` on the private monorepo: the first 5,558 files identical (run stopped, hours per side). All 51 workspace packages
the private monorepo depends on: types and symbols identical (1,526 files), counters equal. Details: `notes/fix-project-types.md`.

## 2026-09-30 (later): peak memory

The private monorepo's peak footprint is now below the Go reference: 15.35 GB `--singleThreaded` (was 19.5; tsgo 16.7) and
23.35 GB with 4 checkers (was 28.3; tsgo 24.4), counters and conformance unchanged. The largest item was lazy JSDoc
parsing copying the whole file text per node (3 GB). Opt-in arena/heap allocation profile:
`--features alloc-profile` on `tsrs_cli` (+ `TSRS_HEAP_PROFILE=1`). Details: `notes/fix-perf-memory.md`.

## 2026-09-30 (later): lazy member resolution on by default

tsrs now runs ports of microsoft/TypeScript#64475 (lazy member tables of instantiated class/interface references) and
#64526 (lazy members of `keyof` mapped types) by default; `--noLazyMembers` / `TSRS_LAZY_MEMBERS=0` restores the
reference behavior exactly. **tsrs's default is therefore no longer counter-identical to `tsgo-ref`**: it matches tsgo
with both PRs applied (private-monorepo symbols/types/instantiations equal to that build exactly). The no-regression check of
DEBUGGING.md applies to the opt-out mode (reference-identical: suite pass lists and artifacts, private-monorepo counters) and the
default mode must not lose passes either. Conformance with the flag on: no output change in any variant (errors,
`.types`, `.symbols`). The private monorepo, medians of 3 (flag off -> on): single-threaded peak 15.34 -> 11.77 GB, check
26.3 -> 22.9 s; 4 checkers peak 23.32 -> 17.49 GB, check 13.2 -> 11.1 s. Details: `notes/lazy-members.md`.

## 2026-10-05: checker threads steal work

With output independent of the assignment, the type-check pass schedules dynamically. A checker that has run out of
its locality-assigned files takes unstarted files from the back of the busiest checker. 38k-file codebase, total
time: -10.9% at 4 checkers and -12.3% at 8 on the Mac; -13.8% and -17.0% on Linux. Instructions are +1-5% at those
counts (first touches of stolen files); the gain shrinks at 16. Diagnostics are identical (stealing vs static on the
40k-error corpus, and conformance with 4 checkers per test and stealing active). Counters vary from run to run
unless an assignment is named. Forked checker processes with shared queues were measured and not landed: they tie
threads on the large codebase and lose on small ones (notes/perf-checker-stealing.md, notes/perf-checker-processes.md).

## 2026-10-05: diagnostics no longer depend on the checker assignment

On the 40k-error corpus, tsgo and tsrs printed different text for `--checkers 1, 2, 4, 8, 12, 16`: four distinct
outputs, 2-6 lines each. The cause is one tsgo cache. `getUndefinedProperty` keeps the filled-in `x?: undefined` of
widened object literals per name, with the declaration of the first `x` a checker saw, and properties are sorted by
declaration. By default tsrs keys it by the property it stands for (one line; `upstream/determinism-undefined-property.md`).
Output is then a function of the program alone: identical for N = 1-16 x 20 random assignments
(`TSRS_CHECKER_ASSIGNMENT=random:<seed>`) on the error-rich corpora, and for conformance run with 4 checkers per test
and random assignments. `--checkerAssignment go` keeps tsgo's cache; the conformance, fourslash and tsc harnesses run
that way, so their counts are unchanged (13,458 / 12,779 / 12,779). In the default mode one test differs from tsgo by
design (`objectLiteralNormalization` `.types` / `.symbols` / `.js`), as do 8 of the private monorepo's 2,460
declaration files and 8 of vscode's 9,399. The 2026-10-01 entry below said the assignment cannot change any file's
diagnostics: that was not true for this cache before this change. Details: notes/perf-order-independence.md.

## 2026-10-01: checker assignment by locality (multi-checker memory)

Files are now assigned to checkers by directory locality (`--checkerAssignment locality`, the default;
`--checkerAssignment go` / `TSRS_CHECKER_ASSIGNMENT=go` restores Go's FENNEL assignment). Checker code is
unchanged and the assignment cannot change any file's diagnostics or their order (per-file results, output in
file order): conformance with parallel test programs is identical under both assignments and both lazy modes
(13,458 pass, `.types` / `.symbols` 12,779 / 12,779, all artifacts byte-identical to single-threaded).

The private monorepo (go -> locality, same commit 52ef5a9, medians of 3): 4 checkers peak 14.99 -> 13.37 GB (-10.8%), check
9.5 -> 8.0 s, symbols 19.6M -> 17.3M; reference mode 20.08 -> 17.70 GB. Full curve on 7d96945 (2-8 checkers:
-4% to -13% peak, faster at every count; 3 checkers with locality beat Go's 4 on both axes) in the notes.
Single-threaded unchanged (no assignment with one checker). Of the remaining multi-checker excess, most is library instantiations (zod,
MikroORM, lib) that every checker needs. Measurements, duplication breakdown, instrumentation
(`TSRS_ASSIGNMENT_STATS`, feature `assignment-stats`): `notes/mem-assignment.md`.

## 2026-10-01: data-layout pass (peak memory)

Layout and allocation changes only (counters, suite pass lists and artifacts identical in both lazy modes): one
arena allocation per AST node and per type (32-byte headers), `SymbolTable` as an insertion-ordered `Vec` with an
index only past 8 entries, rarely set `ValueSymbolLinks` and `Symbol` fields in lazily allocated tails, 24-byte
`TypeMapper`, packed slice cells in structured types and signatures, 20-byte relation cache slots, id-keyed link
stores paged by id. The private monorepo, reference mode (isolates these changes; medians of 3, interleaved): single-threaded
peak 13.94 -> 11.08 GB, check 21.2 -> 18.2 s; 4 checkers 21.21 -> 17.06 GB, check 10.07 -> 8.66 s. Default mode
(with lazy tuples and locality assignment landed meanwhile): 10.80 -> 8.39 GB / 20.0 -> 17.1 s single, 16.09 ->
11.33 GB / 9.1 -> 6.7 s on 4 checkers. Details and per-change numbers: `notes/mem-layout.md`.

## 2026-10-01: frontend wall time (program construction, config, statistics)

Everything outside the checkers on the private monorepo (4 checkers, medians of 5): 1.90 -> 1.00 s, wall 9.72 -> 8.63 s,
checker semantics untouched (suite identical in default, opt-out and parallel-program modes; counters,
`--listFiles`, `--explainFiles`, `--traceResolution` unchanged). The file loader's parallel prefetch now also
computes metadata and resolves imports / type references / triple-slash references (was sequential, 0.6 s),
root file lookups, the include glob's directory listings, the assignment's import adjacency and the statistics
line count run in parallel ("Parse time" 1.56 -> 0.80 s). `--extendedDiagnostics` prints the sub-phases
(`tsrs_core::phases`). What remains: ~0.54 s of parallel parse + resolve bound by file-system calls; checker
imbalance (slowest checker consistently ~10.5% above the mean; a syntactic cost model did not transfer, see
the notes). Details: `notes/speed-frontend.md`.

## 2026-10-01: representation pass (peak memory, round 2)

Representation changes only (suite errors and `.types`/`.symbols` byte-identical to the base in both lazy modes;
counters unchanged; AST oracle unchanged): `Symbol` 72 -> 56 bytes (packed name and declarations, 32-bit id),
16-byte `SymbolTable` entries that read the key from the symbol, 16-byte `TypeMapper` (tagged words), identifier /
literal text in one word (`PackedStr`), inference contexts / infos, signatures, emit-context entries and union /
intersection types with rare fields in tails or packed cells, rarely used per-type caches (base constraints of
structured types, object-type instantiation maps, the node builder's abstract-construct cache) in checker maps,
lazy member tables without duplicate lists, 17-byte relation cache slots. The private monorepo, default mode (medians of 3,
interleaved, base e8d4196): single-threaded peak 8.13 -> 7.02 GiB (-13.7%), check 17.6 s both; 4 checkers
10.93 -> 9.38 GiB (-14.2%), check 6.9 -> 7.0 s. Opt-out mode: 11.08 -> 9.47 / 17.11 -> 14.52 GiB. Rejected:
mapper and type-list interning (mapper identity is observable; 5.3M distinct of 10M type lists, the table would
cost more than it saves), recycling inference contexts (they escape through their mappers), a global identifier
interner (parse +9%). Details and per-step numbers: `notes/mem-round2.md`.

## 2026-10-01: checker CPU pass

Implementation changes only (suite errors and `.types`/`.symbols` byte-identical in both lazy modes, counters
unchanged, peak unchanged): pooled inference `visited` maps no longer memset a capacity one deep inference grew,
inlined `keyBuilder` writers, `IdLinkStore` pages in a vector, Go-style reuse of active type-mapper caches, and
stored slices / strings instead of copies where Go returns them (`getTypeArguments`, `getPropertiesOfType`,
`getSignaturesOfType`, `getIndexInfosOfType`, `getStringLiteralValue`, property-name helpers). One port
divergence fixed: Rust's `sort_by` / `binary_search_by` compared different pairs than Go's
`slices.SortStableFunc` / `BinarySearchFunc` (CompareTypes ran 12% more often than in tsgo, counted in both);
`tsrs_core::goslices` ports Go's algorithms. Private-monorepo instructions retired (default mode, medians): single
325 -> 294 G (-9.5%), 4 checkers 434 -> 401 G (-7.6%); wall not confirmed on a quiet machine. Go-vs-tsrs call
counts for ~110 hot functions, cache hit rates, profiles before/after: `notes/cpu-checker.md`.

## 2026-10-01: peak memory, round 3 (representation + lazy creation)

Representation changes (suite errors and `.types`/`.symbols` byte-identical to the base in both lazy modes and
with multi-threaded programs; counters unchanged; AST oracle unchanged): resolved members of structured types in
a record allocated on first write (4.9M of 7.6M are never resolved), relation keys of the simple form as packed
type-id pairs in 8-byte slots (no hashing), a 24-byte AST node header (kind, data tag and parent in one word,
32-bit node ids), 8-byte symbol table entries (symbol address + key fingerprint) and 24-byte table headers,
24-byte value-symbol links (a mode selects target/mapper, synthetic-symbol fields or a full tail), instantiated
type aliases allocated only when a cached constructor creates a type, reference instantiation tables keyed by
the references' own type arguments, 12-byte pointer-keyed link slots, one read-only link access through
`try_get`. Track B (lazy creation, Go-portable): inference context mappers created on first use
(`TSRS_LAZY_INFERENCE_MAPPERS`, default on; -1.13M mappers); deferring two-constituent union/intersection
property types was rejected (-3.7% types but it changes results). The private monorepo, default mode (medians of 3,
interleaved, base dc59d8e): single-threaded peak 7.02 -> 6.02 GiB (-14.2%), 4 checkers 9.38 -> 8.07 GiB
(-14.0%); opt-out 9.47 -> 8.01 / 14.53 -> 12.35 GiB. Details, per-step numbers and rejected candidates:
`notes/mem-round3.md`.

## 2026-10-01: PGO release binaries

Release binaries are built with PGO + fat LTO + one codegen unit (profile `dist`; training on the conformance suite
and two open-source bench projects; `cargo build --release` unchanged). The private monorepo: CPU -6.9% single /
-10.5% on 4 checkers, instructions -13.5% / -13.7%, cycles -8.4% / -13.2%; vscode and mui-docs -11% / -14% CPU;
output and counters identical. Details, x86_64 handling and build cost: `notes/perf-pgo.md`.

## 2026-10-01: repository recreated

The GitHub repository was deleted and recreated from a clean mirror of the rewritten history (no pull-request refs,
no Actions history); all tags were kept. Commit SHAs from before this date no longer exist.

## 2026-10-01: memory census (garbage at exit)

`TSRS_CENSUS=1` (alloc-profile build) marks what is reachable at exit: on the private monorepo with one checker, 661 of 4,572 MB arena and 82 of 1,592 MB live heap are unreachable (~0.7 of 6.3 GiB peak), led by inference contexts (~210 MB), scratch mappers and type lists (~175 MB), and object literal types from overload re-checks (~165 MB). Method, tables and a ranked recycling list: `notes/mem-census.md`.

## 2026-10-01: parse CPU (scanner, parser, AST construction)

Parsing proper, private monorepo (38,789 files, one thread, `ast_oracle bench`): 19.0 -> 14.55 G instructions
per round (-23.5%), 0.86 -> 0.73 s, heap allocations 11.4 M -> 1.2 M (tsgo: 1.57 s). Node lists built on one
parser stack, keyword perfect hash, rare paths of per-token functions moved out of line, a monomorphized child
visitor for the parent pass, `findImportOrRequire` via memmem; AST oracle, suite (both lazy modes) and
private-monorepo output unchanged. The parallel parse phase on macOS stays kernel-bound (`open`/`read`/`stat`).
Details: `notes/perf-parse.md`.

## 2026-10-01: recycling provably dead arena memory

Own bump arena with free lists and checkpoints (`tsrs_core::arena`): rolled-back parses are rewound, dropped flow
labels, `infer` / overload inference contexts and scratch mappers (escape bit set by every store, `MapperCell`)
are reused. Private monorepo peak 6.240 -> 5.892 GiB single (-5.6%), 8.389 -> 7.881 GiB on 4 checkers (-6.1%),
instructions +0.5%; every freed block proven unreachable by the census (precise walk + strong mark, 0 violations
on the monorepo and the corpus). Details: `notes/mem-recycle.md`.

## 2026-10-01: small representation items (Symbol, Identifier)

`Symbol` 56 -> 48 bytes (the member/export tail shares a word with `parent`): private monorepo peak -0.08 GiB single,
-0.14 GiB on 4 checkers. Parser identifiers 40 -> 32 bytes (text derived from the source text and the node's end, flow
node and text in one word): -0.07 / -0.05 GiB, instructions +0.3%. `Symbol` 48 -> 40 bytes (a bit for "the value
declaration is the first declaration", 82% of symbols): -0.09 / -0.11 GiB, instructions +0.2%. Type header 32 ->
24 bytes (the rare alias shares a word with the symbol): -0.07 / -0.10 GiB. Output, counters and AST oracle
identical. Details: `notes/mem-small.md`.

## 2026-10-01: checker balance (opt-in cost cache)

`--checkerCostCache <file>` balances the 4 checkers on per-file CPU times from the previous run (default unchanged; ordering and static cost models gave nothing): wall -3.7% on the private monorepo (slowest checker 10.4% -> 2.1% above the mean), -12.5% vscode, -15% mui-docs; output identical. Details: `notes/perf-balance.md`.

## 2026-10-01: first Linux profile (x86_64 cloud VM)

The private monorepo on an 18-vCPU x86_64 sandbox VM (kernel 7.2, THP `madvise`): release 0.1.5 takes 37-48 s with one checker and 16-20 s with four (TS 7.1 nightly `tsc`: 127 s / 75 s), sys 3.5-7.7 s, ~25 K page faults, because mimalloc already puts ~97% of memory on transparent huge pages (THP off: +11-12% wall); explicit `MADV_HUGEPAGE` on arena chunks changes no wall time (-2% RSS) and pre-faulting adds 16-36% RSS for nothing, so nothing landed. Details: `notes/linux-perf.md`.

## 2026-10-05: huge pages for the compressed arena on Linux

Pointer compression moved the arena out of mimalloc into a 32 GiB reservation that never asked for transparent huge pages, so on Linux hosts with THP `madvise` (Ubuntu's default) the arena ran on 4 KiB pages: ~0.9 M page faults per 4-checker run of the private monorepo instead of ~30 K. Thread-arena chunks from 2 MiB up are now 2 MiB-aligned and advised with `MADV_HUGEPAGE`: 2-6% less wall time, a third less sys time, half the dTLB misses, +0.1-0.7% peak RSS; macOS unchanged. With huge pages on both sides, compressed handles cost about 1% of wall time against `plain-ptrs` on x86-64, not the 4-7% measured before. Details: `notes/linux-x86-round.md`.
