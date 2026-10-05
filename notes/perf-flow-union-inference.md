# Flow memo, union front cache, generic-call inference memo

Handoff note, 2026-10-05. Branch `perf/flow-memo` (worktree `~/Developer/tsrs-work/wt/flow`), merged with origin/main
at e2916f3.

- **Flow memo:** implemented, on by default, exact on every gate, with a draft PR. The numbers are under "Final
  numbers".
- **Union front cache:** not started. Notes below.
- **Generic-call inference memo:** not started. Notes below.

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
  - `tsrs_cli/src/tsc/emit.rs`: the stats report.
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

These results are for the post-merge build (main e2916f3) against `wt/flow-base`, the same commit without the memo.

- **Suite gates 1-3.** Result trees are identical (`treecmp.py`):
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
  script is the `edit` string in `final.py` below. It appends a function with narrowing errors to every 50th `.ts`
  file under `apps/olympus/src`.
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

\* These cells come from a 5-round rerun with `peak.py`, which rotates the order of base, new and new with
`TSRS_FLOW_MEMO=0`. In the 3-run pass, those cells read +10.8% (check time, 4), +23.9% (check time, 1) and +0.43%
(peak, 4).

The machine was shared with other agents during these runs (1-minute load 7-19), which made the results noisy:

- Wall-clock check time varied by up to ±15% between runs of the same binary.
- Single runs' instruction counts sometimes rose by 1-2%, from kernel work under memory pressure. For example, mui
  "new, 1" read 54.12, 54.26 and 55.46 G.
- So the mui 1-checker delta (-2.1%) is probably too large. The paired check-phase measurement below gave -0.4% to
  -0.5% for mui.

The `peak.py` rerun also shows what the hooks cost when the memo is off (`TSRS_FLOW_MEMO=0`, same binary): vscode at 1
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
    (the `measpatch.py` patch, never committed), CGU=1 builds and paired interleaved runs.
  - Never measure through a bash wrapper: its startup is counted. Pass env vars directly (`VAR=VAL@binary` specs).
  - Always rebuild both sides before measuring.
  - Compare `--extendedDiagnostics` counters with one checker only.
- **Test runner.** A new worktree needs `ln -s ~/Developer/tsrs-work/TypeScript ts-ref` or `tsrs-test` fails with
  "Could not read compiler test files".

### Builds

The base tree is `~/Developer/tsrs-work/wt/flow-base`, detached at e2916f3.

- Build both trees the same way: `CARGO_BUILD_JOBS=8 cargo build --release -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash`.
- Run the corpus gates and the table with `final.py`, then `recheck.py`. They classify the output into diagnostics,
  counters and timings.
- For check-phase instructions:
  1. apply `python3 measpatch.py <tree>`;
  2. build with `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 cargo build --release -p tsrs_cli --target-dir target/meas-cgu1`;
  3. run `checkmeas.py`;
  4. revert with `measpatch.py <tree> --revert`.

## 2. Union front cache (not started)

Census: union construction is the following share of one checker's check time:

| corpus | share |
| --- | --- |
| the 38k-file codebase | 8.79% (self 6.51%) |
| mui | 11.19% |
| webpack | 8.92% |
| xstate | 4.92% |
| vscode | 4.18% |

What the code looks like:

- `checker_12.rs` `get_union_type_ex`:
  - handles the empty and one-type cases;
  - already has Go's 2-input union-of-unions cache (`union_of_union_types`, keyed by the sorted ids, the reduction and
    the alias), used when one input is a union and there is no origin;
  - otherwise calls `checker_13.rs` `get_union_type_worker`, an `#[inline(always)]` census wrapper around
    `get_union_type_worker_inner`.
- `get_union_type_worker_inner` does, in order:
  - `add_types_to_union` (flatten, a stable sort by `compare_types`, dedup);
  - any/unknown absorption;
  - undefined and missing handling;
  - `remove_redundant_literal_types`;
  - template literals;
  - constrained type variables;
  - `remove_subtypes` (`None` returns `error_type`, the TS2590 path);
  - named unions and origin;
  - `get_union_type_from_sorted_list`, which interns by `get_union_key` into `union_types`.
- The front cache can sit in `get_union_type_ex` before the worker.
  - Suggested key: the input ids in input order (2-4 of them), the reduction and the alias key.
  - Apply it only when `origin` is `None` and no input is a union. Then the flattened count equals the input count,
    and TS2590 cannot fire: `remove_subtypes`' estimate and the cross-product check need large inputs.
- Hazards to check before trusting it:
  - `UnionReduction::Subtype` runs strict-subtype relations. They can instantiate (instantiation count, TS2589), hit a
    circularity, or read an in-progress resolution, which would make the result depend on state.
  - Start with `Literal`/`None` only, and measure how much `Subtype` would add before covering it. If it is covered,
    skip storing whenever `total_instantiation_count` or the type count changed during the call, like FLAG_EFFECTS in
    the flow memo.
  - `exactOptionalPropertyTypes` (missing vs undefined) is fixed per program, so it is safe.
  - The alias includes its type arguments; key on `get_alias_key`.
- Shadow mode: on a hit, also compute the slow path and assert the same pointer (`P` equality). Run it over the suite
  and the five corpora.

## 3. Generic-call inference memo (not started)

- Census (perf/algo-census build): repeated inferences with identical (signature, argument types):
  - the 38k-file codebase: 309k of 599k, about 7.9% of check time;
  - vscode about 9%, mui about 9.1%, xstate about 4.2%, webpack about 2.6%.
- Those shares include checking the argument expressions, which cannot be skipped. The brief's first step, splitting
  that from `infer_types` + `get_inferred_types`, has not been done. Go no further unless the skippable part is at
  least 1.5% on two corpora.
- Where the code is:
  - `checker_05.rs`: `resolve_call` (300), `choose_overload` (534), `infer_type_arguments` and
    `infer_type_arguments_worker` (963/987);
  - `inference.rs`: `infer_types` (42), `get_inferred_types` (1625);
  - `checker_10.rs`: `get_signature_instantiation` (49).
- Suggested split: time `infer_type_arguments_worker` with the argument checks excluded. Every `check_expression_with_contextual_type` inside it is argument checking. The skippable part is the remainder; use the census timer pattern from `workcensus.rs`.
- Hazards, besides the brief's list (const type parameters, NoInfer, contextual-return-type priorities, intra-expression
  inference sites, first-time resolution):
  - TS2589. Store the instantiation-count delta with the entry. On a hit, take the slow path if
    `instantiation_count + delta` would reach the limit; otherwise add the delta.
  - Inference can create fresh types (object-literal widening, reverse mapped types). Do not store a computation that
    created types, as with FLAG_EFFECTS in the flow memo.

## Appendix: scripts used

They lived in the session scratchpad. Paths in them are the ones on this machine.

### waitload.sh

```
#!/bin/bash
# wait while 1-min load > 40
while true; do
  l=$(sysctl -n vm.loadavg | awk '{print $2}')
  if awk -v l="$l" 'BEGIN{exit !(l<=40)}'; then break; fi
  echo "load $l, waiting" >&2; sleep 20
done
```

### corpora.sh

```
# name dir project
big   /Users/maxschwenk/Developer/tsrs-work/owner-clone/apps/olympus .
vscode /Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/vscode src
webpack /Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/webpack .
mui /Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/mui-docs docs
xstate /Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/xstate-main .
```

### meas.py

```
#!/usr/bin/env python3
"""Paired instruction measurement: meas.py <corpus> <rounds> <binA> <binB> [extra args...]
Single-threaded (RAYON_NUM_THREADS=1, --singleThreaded), one discarded warm-up per binary, interleaved rounds
(order flipped every round), median of per-round paired deltas (B vs A)."""
import os, re, statistics, subprocess, sys
CORPORA = {
    'big': ('/Users/maxschwenk/Developer/tsrs-work/owner-clone/apps/olympus', '.'),
    'vscode': ('/Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/vscode', 'src'),
    'webpack': ('/Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/webpack', '.'),
    'mui': ('/Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/mui-docs', 'docs'),
    'xstate': ('/Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/xstate-main', '.'),
}
corpus, rounds, a, b = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
extra = sys.argv[5:]
d, p = CORPORA[corpus]
env = dict(os.environ, RAYON_NUM_THREADS='1')
def run(spec):
    # spec: [VAR=VAL,...@]binary (no wrapper process: its startup would be counted)
    e = dict(env)
    binary = spec
    if '@' in spec:
        vars_, binary = spec.split('@', 1)
        for kv in vars_.split(','):
            k, v = kv.split('=', 1)
            e[k] = v
    out = subprocess.run(['/usr/bin/time', '-l', binary, '-p', p, '--noEmit', '--incremental', 'false', '--pretty', 'false', '--singleThreaded'] + extra,
                         cwd=d, env=e, capture_output=True, text=True)
    m = re.search(r'(\d+)\s+instructions retired', out.stderr)
    pk = re.search(r'(\d+)\s+peak memory footprint', out.stderr)
    return int(m.group(1)), int(pk.group(1))
run(a); run(b)
deltas = []; ia = []; ib = []; pa = []; pb = []
for r in range(rounds):
    order = [(a, ia, pa), (b, ib, pb)] if r % 2 == 0 else [(b, ib, pb), (a, ia, pa)]
    res = {}
    for binary, lst, pl in order:
        i, pk = run(binary); lst.append(i); pl.append(pk); res[binary] = i
    deltas.append(100.0 * (res[b] - res[a]) / res[a])
print(f'{corpus}: min A {min(ia)/1e9:.3f} B {min(ib)/1e9:.3f} ({100*(min(ib)-min(ia))/min(ia):+.2f}%); A median {statistics.median(ia)/1e9:.3f} G, B median {statistics.median(ib)/1e9:.3f} G, paired deltas {" ".join(f"{x:+.2f}" for x in deltas)}%, median {statistics.median(deltas):+.2f}%; peak A {max(pa)/2**30:.3f} B {max(pb)/2**30:.3f} GiB')
```

### checkmeas.py

```
#!/usr/bin/env python3
"""checkmeas.py <rounds> <corpora,...> <specA> <specB>: check-phase instructions (measurement-patch binaries,
TSRS_CHECK_INSTR), single-threaded, interleaved; spec = [VAR=VAL,...@]binary. Prints min/median per side and delta."""
import os, re, statistics, subprocess, sys
CORPORA = {
    'big': ('/Users/maxschwenk/Developer/tsrs-work/owner-clone/apps/olympus', '.'),
    'vscode': ('/Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/vscode', 'src'),
    'webpack': ('/Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/webpack', '.'),
    'mui': ('/Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/mui-docs', 'docs'),
    'xstate': ('/Users/maxschwenk/Developer/tsrs-work/bench-cache/solutions/xstate-main', '.'),
}
rounds = int(sys.argv[1]); corpora = sys.argv[2].split(','); specs = sys.argv[3:5]
checkers = os.environ.get('CHECKERS')
def run(spec, d, p):
    e = dict(os.environ, RAYON_NUM_THREADS='1', TSRS_CHECK_INSTR='1')
    binary = spec
    if '@' in spec:
        vs, binary = spec.split('@', 1)
        for kv in vs.split(','):
            k, v = kv.split('=', 1); e[k] = v
    args = [binary, '-p', p, '--noEmit', '--incremental', 'false', '--pretty', 'false']
    args += ['--checkers', checkers] if checkers else ['--singleThreaded']
    out = subprocess.run(args, cwd=d, env=e, capture_output=True, text=True)
    return sum(int(m) for m in re.findall(r'check instructions: (\d+)', out.stderr))
for c in corpora:
    d, p = CORPORA[c]
    vals = {0: [], 1: []}
    for r in range(rounds):
        order = (0, 1) if r % 2 == 0 else (1, 0)
        for i in order:
            vals[i].append(run(specs[i], d, p))
    a, b = vals[0], vals[1]
    print(f'{c:8s} A min {min(a)/1e9:9.4f} med {statistics.median(a)/1e9:9.4f} | B min {min(b)/1e9:9.4f} med {statistics.median(b)/1e9:9.4f} | delta min {100*(min(b)-min(a))/min(a):+.2f}% med {100*(statistics.median(b)-statistics.median(a))/statistics.median(a):+.2f}%', flush=True)
```

### measpatch.py

```
#!/usr/bin/env python3
# Applies (or reverts with --revert) a measurement-only patch: tsrs prints the check phase's instructions retired
# (proc_pid_rusage, user+kernel) to stderr when TSRS_CHECK_INSTR is set. Never committed.
import sys
p = sys.argv[1] + '/crates/tsrs_cli/src/tsc/emit.rs'
s = open(p).read()
old = '''        &mut |ctx, file| {
            let check_start = input.sys.now();
            let diags = program.get_semantic_diagnostics(ctx, file);
            check_time.set(input.sys.now() - check_start);
            diags
        },'''
new = '''        &mut |ctx, file| {
            let check_start = input.sys.now();
            let i0 = meas_instr();
            let diags = program.get_semantic_diagnostics(ctx, file);
            if std::env::var("TSRS_CHECK_INSTR").is_ok() { eprintln!("check instructions: {}", meas_instr() - i0); }
            check_time.set(input.sys.now() - check_start);
            diags
        },'''
fn = '''
fn meas_instr() -> u64 {
    extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut u64) -> i32;
    }
    let mut buf = [0u64; 64];
    // SAFETY: measurement patch, never committed; the buffer is larger than rusage_info_v4.
    let rc = unsafe { proc_pid_rusage(std::process::id() as i32, 4, buf.as_mut_ptr()) };
    if rc == 0 { buf[31] } else { 0 }
}
'''
if '--revert' in sys.argv:
    s = s.replace(new, old).replace(fn, '')
else:
    assert old in s
    s = s.replace(old, new) + fn
open(p, 'w').write(s)
```

### treecmp.py

```
#!/usr/bin/env python3
"""treecmp.py A B: compare two tsrs-test results trees; summary.json compared without per-test 'ms'."""
import json, os, sys
a, b = sys.argv[1], sys.argv[2]
def files(root):
    out = set()
    for d, _, fs in os.walk(root):
        for f in fs:
            out.add(os.path.relpath(os.path.join(d, f), root))
    return out
fa, fb = files(a), files(b)
diffs = []
for f in sorted(fa ^ fb):
    diffs.append(f'only in one: {f}')
def strip(x):
    if isinstance(x, dict):
        return {k: strip(v) for k, v in x.items() if k != 'ms'}
    if isinstance(x, list):
        return [strip(v) for v in x]
    return x
for f in sorted(fa & fb):
    pa, pb = os.path.join(a, f), os.path.join(b, f)
    if f == 'summary.json':
        if strip(json.load(open(pa))) != strip(json.load(open(pb))):
            diffs.append('summary.json differs (beyond ms)')
    elif open(pa, 'rb').read() != open(pb, 'rb').read():
        diffs.append(f'differs: {f}')
print(f'{len(fa)} files vs {len(fb)}; {len(diffs)} differences')
for d in diffs[:20]:
    print('  ' + d)
```

### gates.sh

```
#!/bin/bash
# gates.sh <out dir>: run gates 1-3 (+ shadow suite) for base (wt/flow-base) and new (wt/flow); compare trees.
OUT=$1; mkdir -p $OUT
S=$S
B=/Users/maxschwenk/Developer/tsrs-work/wt/flow-base
N=/Users/maxschwenk/Developer/tsrs-work/wt/flow
run() { # name dir env... -- args
  local name=$1 dir=$2; shift 2
  $S/waitload.sh
  (cd $dir && env "$@") > $OUT/$name.log 2>&1
  echo "$name: $(tail -n 4 $OUT/$name.log | tr '\n' ' ' | cut -c1-300)"
}
for side in base new; do
  if [ $side = base ]; then D=$B; else D=$N; fi
  run suite-$side $D TSRS_TEST_RESULTS=$OUT/tr-$side ./target/release/tsrs-test run --suite all --baselines types,symbols --jobs 12
  run suite-nolazy-$side $D TSRS_LAZY_MEMBERS=0 TSRS_TEST_RESULTS=$OUT/tr-nolazy-$side ./target/release/tsrs-test run --suite all --baselines types,symbols --jobs 12
  run js-$side $D TSRS_TEST_RESULTS=$OUT/tr-js-$side ./target/release/tsrs-test run --suite all --baselines js,jsmap,sourcemap --jobs 12
  run fourslash-$side $D TSRS_FOURSLASH_RESULTS=$OUT/fs-$side ./target/release/tsrs-fourslash run -j 12
done
run suite-shadow $N TSRS_FLOW_MEMO=shadow TSRS_TEST_RESULTS=$OUT/tr-shadow ./target/release/tsrs-test run --suite all --baselines types,symbols --jobs 12 --panic-summary
run suite-nolazy-shadow $N TSRS_FLOW_MEMO=shadow TSRS_LAZY_MEMBERS=0 TSRS_TEST_RESULTS=$OUT/tr-nolazy-shadow ./target/release/tsrs-test run --suite all --baselines types,symbols --jobs 12 --panic-summary
echo "== tree comparisons"
python3 $S/treecmp.py $OUT/tr-base $OUT/tr-new
python3 $S/treecmp.py $OUT/tr-nolazy-base $OUT/tr-nolazy-new
python3 $S/treecmp.py $OUT/tr-js-base $OUT/tr-js-new
python3 $S/treecmp.py $OUT/tr-base $OUT/tr-shadow
python3 $S/treecmp.py $OUT/tr-nolazy-base $OUT/tr-nolazy-shadow
for f in pass.txt fail.txt skip.txt; do cmp -s $OUT/fs-base/$f $OUT/fs-new/$f && echo "fourslash $f identical" || echo "fourslash $f DIFFERS"; done
echo GATES-DONE
```

### final.py

```
#!/usr/bin/env python3
"""final.py: corpus gates and the before/after table for the flow memo, base (wt/flow-base) vs new (wt/flow).
Per corpus: diagnostics + counters identical (default checkers, 4, 1); medians of REPS interleaved runs for
instructions / check time / peak with 1 checker (single-threaded) and 4; shadow run (1 checker; vscode also 4).
Then the error-rich run in an APFS clone. Writes summary.md."""
import os, re, shutil, statistics, subprocess, sys
S = os.path.dirname(os.path.abspath(__file__))
OUT = S + '/final'; os.makedirs(OUT, exist_ok=True)
W = '/Users/maxschwenk/Developer/tsrs-work'
BASE = W + '/wt/flow-base/target/release/tsrs'
NEW = W + '/wt/flow/target/release/tsrs'
CORPORA = [('big', W + '/owner-clone/apps/olympus', '.'), ('vscode', W + '/bench-cache/solutions/vscode', 'src'),
           ('webpack', W + '/bench-cache/solutions/webpack', '.'), ('mui', W + '/bench-cache/solutions/mui-docs', 'docs'),
           ('xstate', W + '/bench-cache/solutions/xstate-main', '.')]
REPS = int(os.environ.get('REPS', '3'))
only = sys.argv[1:]
summary = open(OUT + '/summary.md', 'a')
def say(s):
    print(s, flush=True); summary.write(s + '\n'); summary.flush()
STAT = re.compile(r'^[A-Za-z][A-Za-z0-9 ()/.,_-]*:\s+[\d.]+[A-Za-z%]*$')
def split(out):
    diags, counters = [], []
    for l in out.splitlines():
        if STAT.match(l):
            if 'time' not in l.lower() and 'memory' not in l.lower():
                counters.append(l)
        else:
            diags.append(l)
    return '\n'.join(diags), '\n'.join(counters)
def run(tag, binary, d, p, checkers, env=None, ext=True):
    subprocess.run([S + '/waitload.sh'])
    e = dict(os.environ)
    args = ['/usr/bin/time', '-l', binary, '-p', p, '--noEmit', '--incremental', 'false', '--pretty', 'false']
    if ext:
        args.append('--extendedDiagnostics')
    if checkers == 1:
        args.append('--singleThreaded'); e['RAYON_NUM_THREADS'] = '1'
    elif checkers:
        args += ['--checkers', str(checkers)]
    e.update(env or {})
    r = subprocess.run(args, cwd=d, env=e, capture_output=True, text=True)
    open(f'{OUT}/{tag}.out', 'w').write(r.stdout); open(f'{OUT}/{tag}.err', 'w').write(r.stderr)
    m = re.search(r'Check time:\s+([\d.]+)s', r.stdout)
    return dict(rc=r.returncode, instr=int(re.search(r'(\d+)\s+instructions retired', r.stderr).group(1)),
                peak=int(re.search(r'(\d+)\s+peak memory footprint', r.stderr).group(1)), check=float(m.group(1)) if m else 0.0,
                out=r.stdout, err=r.stderr)
def pct(a, b):
    return f'{100.0 * (b - a) / a:+.2f}%'
rows = []
for name, d, p in CORPORA:
    if only and name not in only:
        continue
    res = {}
    ok = True
    for mode in (None, 4, 1):
        diag = {}
        for rep in range(REPS if mode else 1):
            for side, binary in (('base', BASE), ('new', NEW)) if rep % 2 == 0 else (('new', NEW), ('base', BASE)):
                r = run(f'{name}-{side}-{mode}-{rep}', binary, d, p, mode)
                res.setdefault((side, mode), []).append(r)
                dg = split(r['out'])
                if (side, mode) in diag and diag[(side, mode)] != dg:
                    say(f'{name}: NONDETERMINISTIC {side} checkers={mode}')
                diag[(side, mode)] = dg
        dd, cc = diag[('base', mode)], diag[('new', mode)]
        if dd[0] != cc[0]:
            ok = False; say(f'{name}: DIAGNOSTICS DIFFER checkers={mode}')
        if dd[1] != cc[1]:
            ok = False; say(f'{name}: COUNTERS DIFFER checkers={mode}')
            for a, b in zip(dd[1].splitlines(), cc[1].splitlines()):
                if a != b:
                    say(f'    {a} | {b}')
    ndiag = sum(1 for l in split(res[('base', 1)][0]['out'])[0].splitlines() if ': error TS' in l)
    sh = run(f'{name}-shadow-1', NEW, d, p, 1, env={'TSRS_FLOW_MEMO': 'shadow', 'TSRS_FLOW_MEMO_STATS': '1'})
    shadow_ok = sh['rc'] == res[('base', 1)][0]['rc'] and split(sh['out'])[0] == split(res[('base', 1)][0]['out'])[0] and 'panicked' not in sh['err']
    checks = re.findall(r'shadow[^\n]*', sh['err'])
    say(f'{name}: diags+counters identical={ok} ({ndiag} errors); shadow 1 checker clean={shadow_ok} [{"; ".join(checks)[:300]}]')
    if name == 'vscode':
        sh4 = run(f'{name}-shadow-4', NEW, d, p, 4, env={'TSRS_FLOW_MEMO': 'shadow'})
        say(f'{name}: shadow 4 checkers clean={sh4["rc"] == res[("base", 4)][0]["rc"] and split(sh4["out"])[0] == split(res[("base", 4)][0]["out"])[0] and "panicked" not in sh4["err"]}')
    med = lambda side, mode, k: statistics.median(r[k] for r in res[(side, mode)])
    row = [name]
    for mode in (1, 4):
        a, b = med('base', mode, 'instr'), med('new', mode, 'instr')
        row.append(f'{a / 1e9:.2f} → {b / 1e9:.2f} G ({pct(a, b)})')
    for mode in (1, 4):
        a, b = med('base', mode, 'check'), med('new', mode, 'check')
        row.append(f'{a:.2f} → {b:.2f} s ({pct(a, b)})')
    for mode in (1, 4):
        a, b = med('base', mode, 'peak'), med('new', mode, 'peak')
        row.append(f'{a / 2**20:.0f} → {b / 2**20:.0f} MB ({pct(a, b)})')
    rows.append('| ' + ' | '.join(row) + ' |')
    say(rows[-1])
say('| corpus | instructions, 1 checker | instructions, 4 checkers | check time, 1 | check time, 4 | peak, 1 | peak, 4 |')
say('| --- | --- | --- | --- | --- | --- | --- |')
for r in rows:
    say(r)
if not only or 'errors' in only:
    clone = W + '/owner-clone-flow'
    if not os.path.exists(clone):
        subprocess.run(['cp', '-c', '-R', W + '/owner-clone', clone], check=True)
    edit = r'''find src -name '*.ts' ! -name '*.d.ts' | LC_ALL=C sort | awk 'NR % 50 == 0' | while read -r f; do
  printf '\nexport function __flowErr(x: string | number | undefined, o: { v?: string | number }) {\n  if (x === undefined) return;\n  if (typeof x === "string") { const a: boolean = x; } else { const b: boolean = x; }\n  const c: boolean = x;\n  if (o.v !== undefined && typeof o.v !== "number") { const d: number = o.v; }\n  let e = Math.random() > 0.5 ? "s" : 1; while (typeof e === "string") { e = 2; } const f: string = e;\n}\nconst __flowErr2: number = "x";\n' >> "$f"; done'''
    subprocess.run(['bash', '-c', edit], cwd=clone + '/apps/olympus', check=True)
    d = clone + '/apps/olympus'
    a = run('errors-base', BASE, d, '.', None, ext=False); b = run('errors-new', NEW, d, '.', None, ext=False)
    s1 = run('errors-shadow-1', NEW, d, '.', 1, env={'TSRS_FLOW_MEMO': 'shadow'}, ext=False)
    n = sum(1 for l in a['out'].splitlines() if ': error TS' in l)
    say(f'error-rich (38k-file codebase, every 50th file edited): {n} errors; new identical={a["out"] == b["out"] and a["rc"] == b["rc"]}; '
        f'shadow 1 checker identical={a["out"] == s1["out"] and "panicked" not in s1["err"]}')
    shutil.rmtree(clone)
    say('clone removed')
say('FINAL-DONE')
```

### recheck.py

```
#!/usr/bin/env python3
"""recheck.py: re-classify final/*.out (diagnostics vs counters vs timings) and compare base/new and reps."""
import glob, os, re, sys
OUT = os.path.dirname(os.path.abspath(__file__)) + '/final'
TIMING = re.compile(r'(time|memory|Memory)|[\d.]+s$')
def split(path):
    diags, counters = [], []
    for l in open(path).read().splitlines():
        if 'error TS' in l or l.startswith(' ') or not l.strip():
            diags.append(l)
        elif not TIMING.search(l):
            counters.append(re.sub(r'\s+', ' ', l))
    return diags, counters
for name in sys.argv[1:] or ['big', 'vscode', 'webpack', 'mui', 'xstate']:
    for mode in ('None', '4', '1'):
        reps = {s: sorted(glob.glob(f'{OUT}/{name}-{s}-{mode}-*.out')) for s in ('base', 'new')}
        if not reps['base'] or not reps['new']:
            continue
        res = {s: [split(p) for p in ps] for s, ps in reps.items()}
        det = {s: (all(r[0] == v[0][0] for r in v), all(r[1] == v[0][1] for r in v)) for s, v in res.items()}
        same_d = res['base'][0][0] == res['new'][0][0]
        same_c = any(b[1] == n[1] for b in res['base'] for n in res['new'])
        nerr = sum(1 for l in res['base'][0][0] if 'error TS' in l)
        print(f'{name} checkers={mode}: diags identical={same_d} ({nerr} errors); counters identical={same_c}; '
              f'deterministic base diags/counters={det["base"]} new={det["new"]} (reps {len(reps["base"])})')
        if not same_c and mode == '1':
            for a, b in zip(res['base'][0][1], res['new'][0][1]):
                if a != b:
                    print(f'    {a} | {b}')
for sh in sorted(glob.glob(f'{OUT}/*-shadow-*.out')):
    name, mode = os.path.basename(sh).split('-shadow-')[0], os.path.basename(sh).split('-shadow-')[1][:-4]
    base = f'{OUT}/{name}-base-{mode}-0.out' if name != 'errors' else f'{OUT}/errors-base.out'
    err = open(sh[:-4] + '.err').read()
    same = split(sh)[0] == split(base)[0] if os.path.exists(base) else None
    print(f'shadow {name} checkers={mode}: diags identical to base={same}; panicked={"panicked" in err}; {" ".join(re.findall(r"shadow checks \d+", err))}')
```

### peak.py

```
#!/usr/bin/env python3
"""peak.py <corpus> <rounds> <checkers>: peak / check time / instructions for base, new, new with TSRS_FLOW_MEMO=0,
rotating order each round."""
import os, re, statistics, subprocess, sys
S = os.path.dirname(os.path.abspath(__file__))
W = '/Users/maxschwenk/Developer/tsrs-work'
C = {'big': (W + '/owner-clone/apps/olympus', '.'), 'vscode': (W + '/bench-cache/solutions/vscode', 'src'),
     'webpack': (W + '/bench-cache/solutions/webpack', '.'), 'mui': (W + '/bench-cache/solutions/mui-docs', 'docs'),
     'xstate': (W + '/bench-cache/solutions/xstate-main', '.')}
corpus, rounds, checkers = sys.argv[1], int(sys.argv[2]), sys.argv[3]
d, p = C[corpus]
sides = [('base', W + '/wt/flow-base/target/release/tsrs', {}), ('new', W + '/wt/flow/target/release/tsrs', {}),
         ('new-off', W + '/wt/flow/target/release/tsrs', {'TSRS_FLOW_MEMO': '0'})]
res = {s[0]: [] for s in sides}
for r in range(rounds):
    for name, binary, env in sides[r % 3:] + sides[:r % 3]:
        subprocess.run([S + '/waitload.sh'])
        e = dict(os.environ, **env)
        args = ['/usr/bin/time', '-l', binary, '-p', p, '--noEmit', '--incremental', 'false', '--pretty', 'false', '--extendedDiagnostics']
        if checkers == '1':
            args.append('--singleThreaded'); e['RAYON_NUM_THREADS'] = '1'
        else:
            args += ['--checkers', checkers]
        o = subprocess.run(args, cwd=d, env=e, capture_output=True, text=True)
        res[name].append((int(re.search(r'(\d+)\s+peak memory footprint', o.stderr).group(1)) / 2**20,
                          float(re.search(r'Check time:\s+([\d.]+)s', o.stdout).group(1)),
                          int(re.search(r'(\d+)\s+instructions retired', o.stderr).group(1)) / 1e9,
                          float(os.getloadavg()[0])))
for name, v in res.items():
    print(f'{corpus} c={checkers} {name:8s} peak MB med {statistics.median(x[0] for x in v):.0f} [{" ".join(f"{x[0]:.0f}" for x in v)}] '
          f'check s med {statistics.median(x[1] for x in v):.2f} [{" ".join(f"{x[1]:.2f}" for x in v)}] '
          f'instr G med {statistics.median(x[2] for x in v):.2f} load [{" ".join(f"{x[3]:.0f}" for x in v)}]', flush=True)
```
