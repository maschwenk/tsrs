# Instruction-count hill climb, round 4: five territories at once on a large private monorepo (8 checkers)

Goal (owner, 2026-10-11): continue notes/perf-icount-hillclimb.md on a large private monorepo at `--checkers 8`,
this time with six agents working separate parts of the profile in parallel, each change kept only if the count
went down and the output stayed identical. Base 98d8d60e. 59 commits, each droppable on its own.

## Result

Clean `--release` builds, 98d8d60e against this branch, `-p <tsconfig> --noEmit --incremental false --pretty false`.
"VM" is the exact user-space count (DynamoRIO, `tools/perf/dynamorio/icount.c`, Linux AArch64 in an OrbStack VM);
"Mac" is `/usr/bin/time -l` instructions retired on an 18-core Apple Silicon machine (kernel included).

| | base | branch | change |
|---|---|---|---|
| VM, 8 checkers, `--checkerAssignment locality` (repeats to 0.02%) | 322.67-322.74 G | 281.54 G | -12.8% |
| VM, 8 checkers, default mode (stealing; 3 runs each) | 334.8 / 338.0 / 340.5 G | 292.0 / 292.5 / 298.5 G | -13.5% (medians) |
| VM, single-threaded (repeats to 1e-7) | 192.42 G | 170.41 G | -11.4% |
| Mac, 8 checkers, default mode (median of 3) | 366.7 G | 317.2 G | -13.5% |
| Mac, 8 checkers, locality | 350.2-351.5 G | 304.6-304.8 G | -13.2% |
| Mac, single-threaded | 215.1 G | 189.6 G | -11.9% |
| wall, 8 checkers, quiet Mac, interleaved, median of 5 | 3.78 s (user 26.1 s) | 3.39 s (user 23.3 s) | -10% |
| peak RSS, 8 checkers | 6.38-6.46 GB | 6.37-6.40 GB | -1% |
| heap allocations, 8 checkers locality (`MIMALLOC_SHOW_STATS`) | 183.9 M | 69.8 M | -62% |

Public projects, Mac, same binaries, diagnostics identical (vscode 371, cal-diy 136, t3code-server 6,
supabase-studio 9 errors):

| project | 8 checkers, default | single-threaded | wall at 8 checkers (quiet, median of 3) |
|---|---|---|---|
| vscode | 100.5 -> 91.6 G (-8.8%) | 88.84 -> 82.49 G (-7.1%) | 1.04 -> 0.97 s |
| cal-diy | 70.2 -> 61.7 G (-12.1%) | | |
| t3code-server | 120.2 -> 85.3 G (-29%) | 42.71 -> 38.61 G (-9.6%) | 1.25 -> 1.04 s |
| supabase-studio | 76.1 -> 67.4 G (-11.5%) | | |
| mui-docs | | | 1.07 -> 0.92 s |

Conformance: 13,458 passes, pass lists identical to base in check history, in default mode
(`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`) and with every shadow mode on
(`TSRS_DEEP_NESTED`, `TSRS_LAZY_UNAFFECTED`, `TSRS_UNION_CACHE`, `TSRS_IA_MEMO`, `TSRS_INFER_MEMO`).
`tools/regressions.sh` 29/29; `tools/gen-check.sh` 8/8; ratchet 3 findings, none new. Types and Instantiations
counters identical to base in locality mode.

## How it was measured

- The Mac counter includes kernel work and moved by up to 1% between identical runs in default mode (0.35% in
  locality mode, 0.2% single-threaded); it was the screening instrument. Every keep/reject decision used the VM
  count in locality mode, which repeats to 0.02%; default mode varies 1.7% there (stealing), so it is the headline
  only. `bench/count.py` cannot open a counter in the VM (no PMU through the hypervisor).
- First picture: `samply` on a symbolized `--release` build, CPU-time weighted, inline frames resolved with
  `llvm-symbolizer`; the 8-checker profile against the single-threaded one shows what grows with checkers. Exact
  per-function and per-line instruction counts: `tools/perf/dynamorio/bbcount.c` + `bbagg.py` (added here; both
  clients and the recipe are in `tools/perf/dynamorio/README.md`).
- Gates after every kept change: sorted diagnostics identical on four public projects at 8 checkers, the conformance
  pass list, the regression cases, ratchet and source checks, `cargo check --workspace` with `-D warnings`. Every
  reused answer got a shadow mode that recomputes and compares; two are new (`TSRS_LAZY_UNAFFECTED`,
  `TSRS_DEEP_NESTED`, in docs/DEBUGGING.md).

## Where the 8-checker instructions go on this workload

A work census (the `work-census` feature, with per-thread instruction counters instead of the wall clock) attributed
the 8-checker count before any change. It is 1.6x the single-threaded count; about 38% of it is work every checker
repeats. Of that repeated part, 19% is resolving declared types (library `.d.ts` files only 3%; the rest other
checkers' annotated declarations), 54% is inferring the types of other checkers' **un-annotated exports**, and 27% is
each checker's own instantiation and relation caches for shared generic types. Two exported values of the workload
carried about a third of the repeated work on their own; a declared type on one and a per-key pre-partition of the
other's `Extract` cut the locality count 6.8% with no code change in tsrs (a workload-side result, recorded for the
owner, not part of this branch). Work stealing adds 4-5%: ten stolen files carry 75% of that premium, each the
first file on its thief to need a large shared type graph, and no static signal (import closure, module overlap,
syntax-fitted cost) predicted them (notes/perf-icount-assign.md).

## What landed, by territory (each note has the per-commit numbers and its rejected list)

- Allocation churn (notes/perf-icount-alloc.md, -2.9% VM locality): the open #272 heap-churn commits, then pooled
  scratch lists, sets and strings, borrowed lists where the port copied, template-literal text in a pooled buffer.
- Type instantiation (notes/perf-icount-inst.md, -3.1%): short type-argument lists packed into the 128-bit cache
  keys instead of hashed; lazy member tables reuse their target's settled unaffected-member answers; active mapper
  caches by depth; `TypeMapper::map` cold kinds out of line; `object_target()`.
- Lookups (notes/perf-icount-lookup.md, -4.3%): symbol-table search and insert without a register frame on the
  common paths and with the hash reused; `repr(C)` object/union/intersection data with the parent first; hash
  constants without 64-bit loads on arm64; a link-store hit as one probe; `compareNodes` by position under one parent.
- Relater (notes/perf-icount-relater.md, -2.9%): a string literal's ends tested before the full template-literal
  match in union reduction; recursion identities kept on the relater's and inference's stack entries; the
  `somePropertyReducesToNever` counts in a pooled table.
- Checker assignment (notes/perf-icount-assign.md): at 8 to 15 checkers at most a quarter of the checkers steal from
  one queue at a time (`TSRS_STEAL_MAX_THIEVES`): -0.3% here (noise), t3code-server -20%, mui-docs -4%, wall lower or
  unchanged everywhere measured. The affinity knobs, sticky stealing, run stealing and victim choice by module
  overlap were measured and rejected.
- Flow walks skipped for references nothing in the container can narrow (#282, rebased): -0.4% locality, -0.6%
  single-threaded here.

Stacked, the branch is -12.8% in the VM where the parts sum to 14.0%: three agents pooled the same map, two split
the same signature accessor, and two walked the same member table; the duplicates were dropped during integration
(the memoized version of the member walk kept, the pooled count table kept, lookup's accessor split kept).

## Measured and rejected

Each territory's note lists its own (also added to notes/perf-round2-followups.md). Besides those:
notes/perf-icount-mapped.md: a key-remapped mapped type indexed by a template-literal key looked like a tsrs slow path
(+9% at 8 checkers) but tsgo pays the same in `isGenericMappedType` and `getTemplateLiteralType`; a tsrs-only memo of
that test would be about -0.4% on the unmodified workload and was left as a lead.

## Found on the way, not pursued

- Cross-checker reuse of a handful of resolved exports: bound 72-94 G of the 323 G locality count on this workload
  (the census's (b) plus other checkers' declared types); the shared-graph seed does this under `--maxMemory` but
  without a memory target every checker still waits for the serial seed (notes/spike-shared-graph.md).
- A prefilter for `Extract`-style distribution over a large union of objects with literal discriminants: an
  estimated 3-5% here if it can be proven to give identical types.
- The parallel front end on macOS: 18 threads doing `stat`/`open`/`read` spend about 4.4 s more CPU in the kernel
  than 8 threads, for 0.2 s less wall; it does not change the instruction count.
- The generated AST accessors (`Node::modifiers`, `name`, `expression`) are per-kind switches like the type accessors
  were before the `repr(C)` change (0.4-0.7 G each single-threaded); that needs a generator change in tsrs_ast.
