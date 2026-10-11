# Instruction-count hill climb, round 4: a large private monorepo at 8 checkers, five territories at once

Goal (owner, 2026-10-11): continue notes/perf-icount-hillclimb.md on a large private monorepo at `--checkers 8`,
with six agents working separate parts of the profile in parallel (allocation churn, the type-instantiation path,
table lookups, the relater, checker assignment, and a census of what the checkers repeat); a change was kept only if
the exact count went down and the output stayed identical. Base 98d8d60e.

The full round is on branch `perf/icount-round4` (PR #320: 59 code and notes commits, -12.8% in the exact locality
count, with every territory's own note and rejected list). This branch is the trimmed version: only commits that
gain at least 0.3% on their own, plus the stealing rule; without the #272 and #282 commits (they stay their own pull
requests), without the sub-0.3% tail (sixteen commits, about 1.6 points together), and without the duplicates two
agents built independently. 19 code commits.

## Result

Clean `--release` builds, 98d8d60e against this branch, `-p <tsconfig> --noEmit --incremental false --pretty false`.
"VM" is the exact user-space count (a DynamoRIO client that adds each block's length to a per-thread counter,
Linux AArch64 in an OrbStack VM, `-no_enable_reset`; `bench/count.py` cannot open a counter there); "Mac" is
`/usr/bin/time -l` instructions retired on an 18-core Apple Silicon machine (kernel included).

| | base | branch | change |
|---|---|---|---|
| VM, 8 checkers, `--checkerAssignment locality` (repeats to 0.02%) | 322.67-322.74 G | 292.5 G | -9.4% |
| VM, 8 checkers, default mode (stealing; 3 runs each) | 334.8 / 338.0 / 340.5 G | 302.1 / 303.8 / 307.3 G | -10.1% (medians) |
| VM, single-threaded (repeats to 1e-7) | 192.42 G | 177.2 G | -7.9% |
| Mac, 8 checkers, default mode (median of 3) | 366.7 G | 327.7 G | -10.6% |
| Mac, 8 checkers, locality | 350.2-351.5 G | 316.8-317.9 G | -9.7% |
| Mac, single-threaded | 215.1 G | 197.0 G | -8.4% |
| wall, 8 checkers, quiet Mac, interleaved, median of 5 | 3.79 s (user 26.0 s) | 3.53 s (user 24.0 s) | -7% |
| peak RSS, 8 checkers | 6.38-6.46 GB | 6.35 GB | |

Public projects, Mac, same binaries, diagnostics identical (vscode 371, cal-diy 136, t3code-server 6,
supabase-studio 9 errors): vscode 8-checker count 100.5 -> 94.8 G (-5.7%), single-threaded 88.84 -> 85.04 G (-4.3%); cal-diy 70.2 -> 63.4 G (-9.7%); supabase-studio 76.1 -> 70.2 G (-7.7%); t3code-server 120.2 -> 89.9 G (-25%), single-threaded 42.71 -> 40.23 G (-5.8%). Wall at 8 checkers (quiet machine, median of 3): vscode 1.04 -> 1.00 s, t3code-server 1.24 -> 1.09 s, mui-docs 1.19 -> 0.93 s.

Conformance: 13,458 passes, pass lists identical to base in check history, in default mode
(`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`) and with every shadow mode on
(`TSRS_DEEP_NESTED`, `TSRS_LAZY_UNAFFECTED`, `TSRS_UNION_CACHE`, `TSRS_IA_MEMO`, `TSRS_INFER_MEMO`);
`tools/regressions.sh` 29/29; `tools/gen-check.sh` 8/8; ratchet 3 findings, none new.

## How it was measured

- The Mac counter includes kernel work and moved by up to 1% between identical runs in default mode (0.35% in
  locality mode, 0.2% single-threaded); it was the screening instrument. Every keep/reject decision used the VM
  count in locality mode, which repeats to 0.02%; default mode varies 1.7% there (stealing), so it is the headline
  only.
- First picture: `samply` on a symbolized `--release` build, CPU-time weighted, inline frames resolved with
  `llvm-symbolizer`; the 8-checker profile against the single-threaded one shows what grows with checkers. Exact
  per-function and per-line counts: a per-basic-block DynamoRIO client (on the full round's branch under
  `tools/perf/dynamorio/`, with the recipe).
- Gates after every kept change: sorted diagnostics identical on four public projects at 8 checkers, the conformance
  pass list, the regression cases, ratchet and source checks, `cargo check --workspace` with `-D warnings`. Every
  reused answer has a shadow mode that recomputes and compares; two are new (`TSRS_LAZY_UNAFFECTED`,
  `TSRS_DEEP_NESTED`, in docs/DEBUGGING.md).

## Where the 8-checker instructions go on this workload

A work census (the `work-census` feature, with per-thread instruction counters instead of the wall clock) attributed
the 8-checker count before any change. It is 1.6x the single-threaded count; about 38% of it is work every checker
repeats. Of that repeated part, 19% is resolving declared types (library `.d.ts` files only 3%; the rest other
checkers' annotated declarations), 54% is inferring the types of other checkers' **un-annotated exports**, and 27% is
each checker's own instantiation and relation caches for shared generic types. Two exported values of the workload
carried about a third of the repeated work on their own; a declared type on one and a per-key pre-partition of the
other's `Extract` cut the locality count 6.8% with no change in tsrs (a workload-side result, recorded for the
owner). Work stealing adds 4-5%: ten stolen files carry 75% of that premium, each the first file on its thief to
need a large shared type graph, and no static signal (import closure, module overlap, syntax-fitted cost) predicted
them.

## What landed (commit order; the number in each subject is the commit's own gain on the full branch, exact,
8 checkers locality)

- Allocation churn: conditional-type arguments and intersection type sets in pooled lists (-0.33%); template-literal
  text built in a pooled `String` (-0.72%); borrowed lists where the port copied (constituents, inference matches,
  base-constraint stack) (-0.38%); per-call scratch sets and lists from pools in property synthesis and walks (-0.78%).
- Type instantiation: short type-argument lists packed into the 128-bit instantiation and conditional cache keys
  instead of hashed (-1.0%); lazy member tables reuse their target's settled unaffected-member answers (-0.96%,
  `TSRS_LAZY_UNAFFECTED=shadow`); `object_target()` where `Type::target` is called on a known object type (-0.31%).
- Lookups: `compareNodes` orders two nodes with one parent by position (-0.39%); symbol-table search without a
  register frame on its common paths (-0.67%); symbol-table insert under its known hash (-0.36%); object, union and
  intersection data structs `repr(C)` with the parent first (-0.71%); the dense id-link lookup inline again (-0.30%);
  symbol-name and index hashes without 64-bit constants (-0.47%, arm64); signatures and index infos of a resolved
  type without a register frame (-0.33%); a link-store hit as one probe (-0.31%).
- Relater: `somePropertyReducesToNever` counts names in a pooled table instead of an ordered map (-0.6%); a string
  literal's ends tested before the full template-literal match in union reduction (-0.9%); each relater stack entry
  keeps its recursion identity for the deeply-nested walks (-0.4%, `TSRS_DEEP_NESTED=shadow`).
- Checker assignment: from 8 to 15 checkers, at most a quarter of the checkers steal from one queue at a time
  (`TSRS_STEAL_MAX_THIEVES` overrides): -0.3% here (noise), t3code-server -20% and mui-docs -4% at 8 checkers, wall
  lower or unchanged everywhere measured.

What the reused answers rely on: a value symbol's resolved type and write type are set once and a target's declared
member table does not change (the unaffected-member memo checks the table and its length); a stack entry's
recursion identity never changes once computed (a mapped type's modifiers type is set once). Type ids fit the packed
keys' 32 bits; symbol ids are checked and fall back to hashing.

## Left out of this branch, with numbers

- #272 (heap churn, -0.43% / -0.67% here) and #282 (flow walks skipped, -0.40% / -0.61%): their own pull requests.
- Sixteen commits under 0.3% each (pooled union construction -0.04%, no-copy variances and export names -0.16%,
  `TypeMapper::map` fast paths -0.14% and -0.11%, store without a second search -0.08%, packed aliased keys -0.07%,
  packed union/intersection/indexed-access keys -0.26%, active mapper caches by depth -0.20%, a lazy lookup hashing
  once -0.24%, no name set without base types -0.15%, union-cache impurity only on a full match -0.13%, pooled name
  set for union property lists -0.11%, recursion identities on inference's stacks -0.12%, intersection identity keys
  -0.2%, `isSimpleTypeRelatedTo` inline -0.2%, one stack borrow per push -0.08%): about 1.6 points together, all on
  `perf/icount-round4` with their notes.
- Per-checker instruction stats in `TSRS_ASSIGNMENT_STATS=times` and the DynamoRIO clients: tooling, same branch.

## Measured and rejected

The territory notes on `perf/icount-round4` list each agent's rejected ideas with numbers; the entries are also in
notes/perf-round2-followups.md "Measured and rejected". The largest negative results: the affinity knobs, sticky
stealing, run stealing and victim choice by module overlap (+0.1% to +2.2% or noise); a short linear list in front of
the active-mapper cache (+0.18%); pools for lists copied straight into the arena (noise); an early exit from the
deeply-nested walk (noise); a key-remapped mapped type indexed by a template-literal key is not a tsrs slow path
(tsgo pays the same in `isGenericMappedType` and `getTemplateLiteralType`).

## Found on the way, not pursued

- Cross-checker reuse of a handful of resolved exports: bound 72-94 G of the 323 G locality count on this workload;
  the shared-graph seed does this under `--maxMemory`, but without a memory target every checker still waits for the
  serial seed (notes/spike-shared-graph.md).
- A prefilter for `Extract`-style distribution over a large union of objects with literal discriminants: an
  estimated 3-5% here if it can be proven to give identical types.
- The parallel front end on macOS: 18 threads doing `stat`/`open`/`read` spend about 4.4 s more CPU in the kernel
  than 8 threads, for 0.2 s less wall; no instruction change.
- The generated AST accessors (`Node::modifiers`, `name`, `expression`) are per-kind switches like the type accessors
  were before the `repr(C)` change (0.4-0.7 G each single-threaded); that needs a generator change in tsrs_ast.
