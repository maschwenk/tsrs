# Status

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

Project (`tsrs -p apps/project`, release build, single checker thread, placeholder printer), against
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

Project error injection (same 15 errors appended to 3 files in a clone): identical (file, line, col, code) sets and
exit code versus the reference. A mutation-testing campaign is in progress (`tools/mutate`, `notes/fix-project.md`).

In flight: node-builder body wave (branches `body/nb-1..6`) replacing the placeholder type printer.

## 2026-09-30 (later): parallel checking on by default

Checking runs like Go's: 4 checkers by default (`--checkers N`, `--singleThreaded` = 1), Go's deterministic file
assignment, one OS thread per checker; the conformance harness stays single-threaded unless
`TS_TEST_PROGRAM_SINGLE_THREADED=false` (Go testutil). Threading contract and the `TSRS_CHECK_SHARED=1` checker:
PORTING.md "Threading". Conformance with parallel programs is identical to single-threaded (same pass list; every
non-passing output equal modulo placeholder type/symbol ids).

Project (release, 18-core machine shared with other agents, one run each unless noted; peak = `peak memory footprint`):

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

Project (release, 0 errors): counters now equal the reference exactly; peak memory unchanged versus main (19.5 vs
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

Project (`declaration: false`, so the pipeline does not run): 0 errors, counters unchanged (39,704,001 symbols /
16,200,921 types / 89,981,648 instantiations with 4 checkers). Wall times versus main measured back to back on a
loaded machine (load average ~14) were within noise in both directions (4 checkers: 22.3/27.0 s main vs 39.4/34.2 s;
single-threaded: 58.9/56.3 s main vs 43.9/46.7 s).

## 2026-09-30 (later): Project `.types` / `.symbols` walk

`tsrs-test types-dump` and the Go oracle `tools/oracle/project-types` run the conformance `.types`/`.symbols` walk
(printed type / resolved symbol of every expression and declaration) over every non-`node_modules` file of a tsconfig
project, single-threaded, after `tsc`-style checking; `tools/project-types-compare.py` compares them. Project: 28,213
files, 16.5M baseline lines; first run **28,207 / 28,213 files identical** (18 lines in 6 files differ), symbol and
type counters equal. The 6 files print >1 MB `vi.mocked(..., { partial: true, deep: true })` types whose truncation
point depended on **symbol ids**: Go assigns ids on every `valueSymbolLinks` / `symbolNodeLinks` access (those stores
are id-keyed) and binds files in reverse order when single-threaded (LIFO work group); a well-known-symbol property's
internal name embeds the id and counts toward the node builder's length budget. Both fixed (regression
`testdata/regressions/unique-symbol-name-truncation`); conformance unchanged (13,458 / types 12,779 / symbols
12,779). Go's own ids are not deterministic across runs (map iteration), so ids agree as a multiset, not exactly.
`.symbols` on Project: the first 5,558 files identical (run stopped, hours per side). All 51 workspace packages
Project depends on: types and symbols identical (1,526 files), counters equal. Details: `notes/fix-project-types.md`.

## 2026-09-30 (later): peak memory

Project peak footprint is now below the Go reference: 15.35 GB `--singleThreaded` (was 19.5; tsgo 16.7) and
23.35 GB with 4 checkers (was 28.3; tsgo 24.4), counters and conformance unchanged. The largest item was lazy JSDoc
parsing copying the whole file text per node (3 GB). Opt-in arena/heap allocation profile:
`--features alloc-profile` on `tsrs_cli` (+ `TSRS_HEAP_PROFILE=1`). Details: `notes/fix-perf-memory.md`.

## 2026-09-30 (later): lazy member resolution on by default

tsrs now runs ports of microsoft/TypeScript#64475 (lazy member tables of instantiated class/interface references) and
#64526 (lazy members of `keyof` mapped types) by default; `--noLazyMembers` / `TSRS_LAZY_MEMBERS=0` restores the
reference behavior exactly. **tsrs's default is therefore no longer counter-identical to `tsgo-ref`**: it matches tsgo
with both PRs applied (Project symbols/types/instantiations equal to that build exactly). The no-regression check of
DEBUGGING.md applies to the opt-out mode (reference-identical: suite pass lists and artifacts, Project counters) and the
default mode must not lose passes either. Conformance with the flag on: no output change in any variant (errors,
`.types`, `.symbols`). Project, medians of 3 (flag off -> on): single-threaded peak 15.34 -> 11.77 GB, check
26.3 -> 22.9 s; 4 checkers peak 23.32 -> 17.49 GB, check 13.2 -> 11.1 s. Details: `notes/lazy-members.md`.

## 2026-10-01: checker assignment by locality (multi-checker memory)

Files are now assigned to checkers by directory locality (`--checkerAssignment locality`, the default;
`--checkerAssignment go` / `TSRS_CHECKER_ASSIGNMENT=go` restores Go's FENNEL assignment). Checker code is
unchanged and the assignment cannot change any file's diagnostics or their order (per-file results, output in
file order): conformance with parallel test programs is identical under both assignments and both lazy modes
(13,458 pass, `.types` / `.symbols` 12,779 / 12,779, all artifacts byte-identical to single-threaded).

Project (go -> locality, same commit 52ef5a9, medians of 3): 4 checkers peak 14.99 -> 13.37 GB (-10.8%), check
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
stores paged by id. Project, reference mode (isolates these changes; medians of 3, interleaved): single-threaded
peak 13.94 -> 11.08 GB, check 21.2 -> 18.2 s; 4 checkers 21.21 -> 17.06 GB, check 10.07 -> 8.66 s. Default mode
(with lazy tuples and locality assignment landed meanwhile): 10.80 -> 8.39 GB / 20.0 -> 17.1 s single, 16.09 ->
11.33 GB / 9.1 -> 6.7 s on 4 checkers. Details and per-change numbers: `notes/mem-layout.md`.
