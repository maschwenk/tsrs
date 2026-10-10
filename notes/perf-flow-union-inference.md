# Flow memo, union front cache, generic-call inference memo

Handoff note, 2026-10-05. Branch `perf/flow-memo`, merged with origin/main at e2916f3.

Status (2026-10-10): the flow memo landed (#97) and is on by default (`crates/tsrs_checker/src/flowmemo.rs`). Tasks 2
and 3 of this handoff (union front cache, generic-call inference memo) were done afterwards and are written up in
notes/perf-union-inference.md: the union front cache landed behind `TSRS_UNION_CACHE` (on by default); the generic-call
inference memo was not built (it failed the 1.5% bar), though a narrower memo of `inferTypes` walks later landed, on
by default (`TSRS_INFER_MEMO`, notes/perf-heavy-files-infer-memo.md). This note was condensed to section 1; the task briefs for 2 and 3, the worktree build steps and the appendix of
measurement scripts were removed.

## 1. Flow memo

### What it does

It keeps the result of every recursive `getTypeAtFlowNode` call (a "frame") of a keyed walk across walks, keyed by
(flow node, reference key). The reference key covers the root symbol, the property names, the declared type, the
initial type and the flow container. `upstream/flow-memo.md` has the pattern, the seven exactness conditions and the
results. `docs/DEBUGGING.md` ("The flow memo and its shadow mode") has the switches and how to read a shadow failure.

### Where the code is

- `crates/tsrs_checker/src/flowmemo.rs`:
  - the table: direct-mapped, 2^12 slots of 32 bytes, allocated on first use, one per checker, living as long as the
    checker;
  - the keys: identifiers and `this` packed exactly into a u128; property-access chains hashed with xxh3-128 with bit
    63 set, the full key kept for shadow mode;
  - serials, taint registers, frame snapshots, the shadow checker and the stats line.
- `crates/tsrs_checker/src/flow.rs`:
  - `get_flow_type_of_reference_ex`: computes the key, and redoes the walk without the memo if it hit the depth limit
    after using the memo;
  - `get_type_at_flow_node`: the frame loop; consults at the first node, the iteration end and every fourth node
    (chosen by hash), and fills on exit;
  - the event hooks in the loop label, the reduce label, `narrow_type`'s inline limit, `get_initial_or_assigned_type`
    and `is_constant_reference_of_walk`.
- Small hooks:
  - `flow_types.rs`: taint fields on `SharedFlow`, memo fields on `FlowState`;
  - `checker.rs`: serials on `TypeResolution` and `FlowLoopInfo`;
  - `checker_04.rs`: `flowTypeCache` epoch, resolving-signature taint;
  - `checker_09.rs`: resolution serials, cycle taint;
  - `checker_14.rs`: contextual resolving-signature taint;
  - `checker_15.rs`: `get_narrowable_type_for_flow_reference`;
  - `links.rs`, `tsrs_ast/src/utilities_1.rs`: peeking a node's resolved symbol without assigning an id;
  - `tsrs_execute/src/tsc/emit.rs`: the stats report.
- Tests: `crates/tsrs_cli/tests/flow_memo.rs` runs each case in `testdata/flow-memo/*` with `TSRS_FLOW_MEMO=1`, `0` and
  `shadow`, and compares against `expected.txt`, which was produced by tsgo-ref. There are nine cases:
  - circularity, loops, try-finally, inline-depth, readonly-property, reference-position, fresh-types;
  - depth-limit: TS2563 with and without the memo (it fails without the height check);
  - depth-limit-after-memo: exercises the redo path.

### Switches

| variable | values | effect |
| --- | --- | --- |
| `TSRS_FLOW_MEMO` | `1` (default), `0`/`off`, `shadow` | `shadow` walks again at every hit and panics on a difference, with the full key compared |
| `TSRS_FLOW_MEMO_STATS` | `1` | one line per checker on stderr; needs `--extendedDiagnostics` |
| `TSRS_FLOW_MEMO_BITS` | default `12` | table size |

### How the brief's requirements are met

- **Incomplete or loop-unresolved results are never stored.** Serial-based taint covers in-process loops, cycles and
  in-progress resolutions, re-entered `getResolvedSignature`, reduce labels, the depth and inline limits, and stale
  `sharedFlows` values.
- **Depth limit.** Each entry stores its height; a lookup misses when `depth + height >= 2000`. The `depth-limit` tests
  show it: TS2563 fires as in tsgo-ref with the memo on, off and in shadow mode.
- **Counters.**
  - `flowInvocationCount` is still counted once per walk, at entry, so `checkExpressionCached` caching is unchanged.
  - The memo is consulted only with an empty loop stack, so `flowLoopStart`/`flowLoopCount` see the same loop
    processing.
  - A skipped loop label is one whose result is already in `flowLoopCache`, where Go returns without counting.
  - A hit records into `sharedFlows` only what Go records (the last shared node of an iteration).
- **Side effects.**
  - A frame that instantiated, created a type, symbol or signature, or ran under an active mapper is not stored.
  - An entry that may have seen an instantiation-count reset is used only while the count is 0.
  - Entries that read `flowTypeCache` are tied to its epoch.
  - Diagnostics inside skipped sub-walks were already reported by the first walk.
  - Shadow mode is the check on all of this.
- **Key.** Exact for identifiers and `this`. Hashed for property chains; shadow mode compares the full key.
- **Lifetime and memory.** One table per checker, kept for the program's life: 128 KB per checker, plus small
  `checkpoints` and `shadow` vectors. Peak memory is unchanged within noise ("Final numbers").
- **Skipping the walk for never-assigned, never-narrowed references.** Not done.
  - Assignment is known (`isSymbolAssigned`), but "never narrowed" needs a per-container index of the references that
    flow conditions mention, which neither Go nor tsrs keeps.
  - Hypothesis, not measured: with the memo, repeated walks of such keys already end at an early hit, so only the first
    walk per key would be saved.
  - Measure how many walks have such keys before building the index.

### Verified

These results are for the post-merge build (main e2916f3) against the same commit without the memo.

- **Suite gates 1-3.** Result trees are identical:
  - suite in default mode and with `TSRS_LAZY_MEMBERS=0`: 13,458 pass, 2 codes, 2 fail; 12,779 types; 12,779 symbols;
  - js 13,392, sourcemap 156;
  - fourslash 4,066 pass / 63 fail, same lists.
- **Shadow mode over the suite**, in default mode and with `TSRS_LAZY_MEMBERS=0`: trees identical, 0 panics.
- **Lint and tests.**
  - `cargo test -p tsrs_cli --test '*'` passes, including `flow_memo`. The bin target does not link on macOS, because
    `api::memory_tests` needs `malloc_trim`.
  - `RUSTFLAGS="-D warnings" cargo check --workspace --locked`, `tools/lint/ratchet.py` and `tools/lint/source.py`
    pass.
- **Diagnostics on all five corpora** are byte-identical with default checkers, `--checkers 4` and `--singleThreaded`.
  vscode has 371 errors and webpack 840.
- **Error-rich run** on the 38k-file codebase: 3,221 errors, identical to base, and identical in shadow mode. The edit
  appended a function with narrowing errors to every 50th `.ts` file under `apps/olympus/src`.
- **Shadow mode over the five corpora** is clean: diagnostics identical, no panic.

  | corpus | shadow checks |
  | --- | --- |
  | 38k-file codebase | 5.10M |
  | vscode | 7.59M (clean at 4 checkers too) |
  | webpack | 1.03M |
  | mui | 388K |
  | xstate | 44K |

- **`--extendedDiagnostics` counters.**
  - With one checker they are deterministic, and types, symbols and instantiations are identical.
  - Only work counters differ, because skipped walks do fewer lookups: "Lazy member lookups", "Lazy signature queries",
    "Lazy index info queries", "Lazy every-property queries", "Lazy mapped member lookups", "Mapped signature early
    returns".
  - With several checkers the counters vary from run to run in main itself, so they cannot be compared.

### Final numbers

Release builds, median of 3 interleaved runs per cell. Instructions are whole-process instructions retired. "1" means
`--singleThreaded` with `RAYON_NUM_THREADS=1`.

| corpus | instructions, 1 | instructions, 4 | check time, 1 | check time, 4 | peak, 1 | peak, 4 |
| --- | --- | --- | --- | --- | --- | --- |
| 38k-file codebase | 275.55 → 274.33 G (-0.44%) | 374.60 → 373.60 G (-0.27%) | 16.54 → 16.30 s (-1.5%) | 5.86 → 5.75 s (-1.9%)\* | 4297 → 4298 MB (+0.04%) | 5879 → 5876 MB (-0.05%)\* |
| vscode | 115.84 → 113.25 G (-2.23%) | 121.15 → 118.56 G (-2.14%) | 7.61 → 7.15 s (-6.0%)\* | 1.77 → 1.74 s (-1.4%) | 2018 → 2018 MB (0%) | 2326 → 2325 MB (-0.02%) |
| webpack | 14.59 → 13.85 G (-5.03%) | 16.40 → 15.66 G (-4.48%) | 0.65 → 0.63 s (-3.8%) | 0.21 → 0.20 s (-5.7%) | 311 → 310 MB (-0.06%) | 400 → 399 MB (-0.21%) |
| mui | 55.44 → 54.26 G (-2.12%) | 86.84 → 86.91 G (+0.08%) | 2.31 → 2.31 s (+0.3%) | 1.08 → 1.08 s (-0.3%) | 746 → 746 MB (+0.01%) | 1097 → 1096 MB (-0.12%) |
| xstate | 7.73 → 7.74 G (+0.13%) | 9.04 → 9.05 G (+0.05%) | 0.34 → 0.34 s (+0.9%) | 0.11 → 0.11 s (+0.9%) | 184 → 184 MB (+0.10%) | 243 → 243 MB (+0.14%) |

\* These cells come from a 5-round rerun that rotates the order of base, new and new with
`TSRS_FLOW_MEMO=0`. In the 3-run pass, those cells read +10.8% (check time, 4), +23.9% (check time, 1) and +0.43%
(peak, 4).

The machine was shared with other agents during these runs (1-minute load 7-19), which made the results noisy:

- Wall-clock check time varied by up to ±15% between runs of the same binary.
- Single runs' instruction counts sometimes rose by 1-2%, from kernel work under memory pressure. For example, mui
  "new, 1" read 54.12, 54.26 and 55.46 G.
- So the mui 1-checker delta (-2.1%) is probably too large. The paired check-phase measurement below gave -0.4% to
  -0.5% for mui.

The 5-round rerun also shows what the hooks cost when the memo is off (`TSRS_FLOW_MEMO=0`, same binary): vscode at 1
checker went from 115.75 G to 116.04 G (+0.25%), and the 38k-file codebase at 4 checkers did not change (373.10 G).

Earlier paired measurement, check-phase instructions, one checker, CGU=1 builds, before the merge:

| corpus | before | after | delta |
| --- | --- | --- | --- |
| webpack | 10.93 G | 10.18 G | -7.0% |
| vscode | 89.5 G | 86.7 G | -3.1% |
| 38k-file codebase | 227.7 G | 226.1 G | -0.7% |
| mui | 43.8 G | 43.6 G | -0.4% to -0.5% |
| xstate | 5.35 G | 5.36 G | +0.13% |

xstate is under the 0.2% bar but close. Its walks are short, so the bookkeeping cost is not paid back.

### Things the brief did not say

- **The census overstates what a memo can save.** 64-67% of walk steps re-walk an earlier (key, node) pair, but most
  walks are a few nodes long, so per-walk and per-frame overhead eats much of it.
- **Checkpoints matter.**
  - Without them: webpack -4.4%, xstate +0.50%.
  - At 1 in 4 nodes: webpack -7.0%, xstate +0.13%.
  - Iteration-end consults are about neutral.
  - Table size (2^10 to 2^16) did not measurably matter.
- **Inlining.** `get_type_at_flow_node_step` must be `#[inline(always)]`; out of line it costs +0.17% on xstate. Do
  bookkeeping only for keyed walks.
- **Two shadow findings an analysis would have missed.**
  - Fresh types: an object literal assigned to an auto-typed `let` gets a new type at every evaluation, and union order
    follows type ids. Hence FLAG_EFFECTS.
  - Go records only the last shared node of a frame's iteration, so a hit must not record more.
- **Shadow plumbing.** A shadow hit must be settled on every frame exit, including the `sharedFlows` return path. It
  must also capture its full key at consult time, because the slot can be overwritten before the frame ends.
- **Measurement.**
  - Whole-process instructions move about ±0.3% between builds from code layout alone. Use check-phase instructions
    (a measurement-only patch, never committed), CGU=1 builds and paired interleaved runs.
  - Never measure through a bash wrapper: its startup is counted. Pass env vars directly.
  - Always rebuild both sides before measuring.
  - Compare `--extendedDiagnostics` counters with one checker only.
- **Test runner.** A new worktree needs a `ts-ref` symlink to the TypeScript checkout or `tsrs-test` fails with
  "Could not read compiler test files".
