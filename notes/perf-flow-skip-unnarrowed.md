# Skipping flow walks that nothing in the function can narrow

Branch `perf/flow-skip-unnarrowed`, 2026-10-10. The open item of notes/perf-flow-union-inference.md ("skipping
never-assigned / never-narrowed walks: measure key counts first"). Landed: on by default, exact on every gate.

Single-threaded, whole process, median of 5 interleaved runs against origin/main (5a19aafa), macOS arm64:

| project | instructions | peak memory |
| --- | --- | --- |
| vscode | 97.25 -> 96.21 G (-1.07%) | 1594 -> 1605 MB (+0.72%) |
| t3code-server | 46.83 -> 45.79 G (-2.22%; -1.6% against base's lower cluster, see below) | 723 -> 724 MB (+0.13%) |
| drizzle-orm | 14.68 -> 14.58 G (-0.72%) | 378 -> 379 MB (+0.20%) |
| webpack | 13.12 -> 13.04 G (-0.65%) | 297 -> 299 MB (+0.81%) |
| formbricks-web | 49.23 -> 48.93 G (-0.60%) | 1088 -> 1090 MB (+0.20%) |
| cal-diy | 38.74 -> 38.53 G (-0.54%) | 761 -> 761 MB (0%) |
| xstate-main | 7.10 -> 7.09 G (-0.15%) | 166 -> 167 MB (+0.22%) |

t3code-server's base runs fell in two clusters (46.53 and 46.83 G); the new binary repeated to 0.03%. Against the
same binary with `TSRS_FLOW_SKIP=0` it is -1.67%. At the default checker count (3 runs): vscode -0.93% instructions
(+0.80% peak), webpack -1.11% (+1.05%), t3code-server -1.64% (+0.26%).

## Step 1: how many walks could be skipped

Instrumented build (not committed), single-threaded, every walk run. "Steps" are the iterations of
`getTypeAtFlowNode`'s loop that the walk itself makes, not counting walks nested in it. "Ends at declared" is the
oracle: declared and initial type are the same object and the walk's result (after `getFlowTypeOfReference`'s
mapping) is the declared type. "Skipped" is the final rule below.

| project | walks | ends at declared | skipped | steps | steps of walks ending at declared | steps of skipped walks |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 3,726,338 | 2,829,888 (75.9%) | 1,934,771 (51.9%) | 14,198,447 | 10,541,506 (74.2%) | 6,736,856 (47.4%) |
| webpack | 286,400 | 200,336 (69.9%) | 108,466 (37.9%) | 1,836,043 | 1,279,281 (69.7%) | 502,243 (27.4%) |
| xstate-main | 59,691 | 44,967 (75.3%) | 40,492 (67.8%) | 141,145 | 109,656 (77.7%) | 96,595 (68.4%) |
| cal-diy | 303,081 | 243,098 (80.2%) | 192,621 (63.6%) | 1,401,183 | 1,049,902 (74.9%) | 639,991 (45.7%) |
| t3code-server | 948,447 | 752,213 (79.3%) | 596,229 (62.9%) | 5,336,154 | 3,015,448 (56.5%) | 1,992,939 (37.3%) |
| drizzle-orm | 257,549 | 227,049 (88.2%) | 179,814 (69.8%) | 812,351 | 667,592 (82.2%) | 498,809 (61.4%) |
| formbricks-web | 595,104 | 493,109 (82.9%) | 424,861 (71.4%) | 1,730,006 | 1,444,531 (83.5%) | 1,195,659 (69.1%) |

Every skipped walk, run anyway, returned the declared type (0 wrong on all seven).

The memo note's hypothesis ("with the memo, repeated walks of such keys already end at an early hit, so only the first
walk per key would be saved") was half right: on vscode a third of the walks that end at the declared type used a memo
hit, but they still cost per-walk setup, the memo key and the consults, and the first walk of every key costs in full.

How the rule got there (vscode; walks / steps the rule would skip, from earlier instrumented runs that credited steps
slightly differently):

| rule | skipped walks | skipped steps |
| --- | ---: | ---: |
| root names, every assignment and declaration a mention, aliases by name across the file | 543 K | 1.3 M |
| reference paths with prefixes; assignments matter only for union and literal declared types | 793 K | 2.3 M |
| + assertion-candidate calls kept apart, cleared when their effects signature is known to be none | 1,078 K | 3.5 M |
| + positions: only mentions a walk can reach (earlier, or in the same loop or IIFE) | 1,724 K | 5.7 M |
| + only what `narrowType` can reach in an expression; aliases only in scope and only const and untyped | 1,935 K | 6.7 M |

Where the remaining walks that end at the declared type are kept (vscode, walks / own steps): the path is mentioned in
a reachable condition, switch or `for...in` (577 K / 2.2 M); a reachable assertion-candidate call mentions it and its
effects signature is not known yet (212 K / 0.95 M); the reference path has a non-literal element access or is not a
path (54 K / 0.34 M); assigned with a union or literal declared type (40 K / 0.15 M); no index for the flow node
(8 K); more than 500 flow nodes (3.8 K / 0.12 M; t3code-server 55 K / 0.52 M).

## The rule

Not in Go. `Checker::flow_walk_skippable` (crates/tsrs_checker/src/flowskip.rs), asked in `getFlowTypeOfReferenceEx`
after the `flowNode == nil` return and before the walk; a skipped walk still counts `flowInvocationCount` (so
`getTypeOfExpression`'s caching is unchanged) and returns the declared type.

tsgo itself only short-circuits on `flowAnalysisDisabled` and a missing flow node; there is no "declared type not
narrowable" or "never mentioned" check before the walk. Callers decide what to walk (`checkIdentifier`: variables and
aliases not in a definite assignment position; `getFlowTypeOfAccessExpression`: properties, accessors, optional
methods), which this does not change.

The binder (crates/tsrs_binder/src/flownames.rs, built while it makes flow nodes; layout in
crates/tsrs_ast/src/flownames.rs) gives every flow node the index of its flow graph (one per `Start` node: a control
flow container that is not an IIFE; the index lives in `FlowFlags` bits 13 and up, which no flag test reads) and
records per graph:

- **Mentioned paths.** A reference path is an identifier, `this` or `super` followed by property names (string literal
  element accesses count as names; parentheses, `!`, `satisfies`, assignments and comma expressions are looked through,
  as `isMatchingReference` does). For every condition, switch (expression and case expressions) and `for...in`
  expression, the keys of every prefix of every path that `narrowType` can reach: it looks through parentheses, `!`,
  `satisfies`, unary and binary operators and `typeof`, matches paths, and in a call the callee's object and the
  arguments; nothing else (literals, object and array literals, `new`, `await`, conditional expressions, `as`,
  functions, classes, JSX) is matched or narrowed through. `"k" in x` and `x.hasOwnProperty("k")` also add `x.k`: they
  narrow a reference one property longer than the path they name (found by shadow mode on t3code-server, which uses
  `exactOptionalPropertyTypes`). A non-literal element access ends a path with a wildcard.
- **Aliases.** When a recorded name is a const, untyped variable in scope (declared in the graph or an enclosing one)
  whose initializer `narrowType` can inline (`const ok = typeof x === "string"`) or that
  `getCandidateDiscriminantPropertyAccess` takes as a discriminant's object (`const k = x.kind`, `const { kind } = x`,
  `const [tag] = t`), the initializer's paths are added at the alias's position, transitively. Both Go paths require
  `isConstantVariable` and no type annotation, so `let`, `var` and typed consts are not aliases. Variables of global
  scripts (top level or in a namespace) are exported per file; the checker closes over them by name across files, once
  per checker.
- **Assigned paths.** Every assignment target and declared name, exactly. An assignment to the reference itself gives
  the declared type unless it is a union (assignment narrowing) or a literal, template, string-mapping or enum type
  (its base type in compound-like assignments), so these block only for such declared types. An assignment to a
  prefix of the reference gives the declared type (`containsMatchingReference`) and is harmless.
- **Assertion-candidate calls** (expression statements calling a dotted name, the binder's `FlowCall` nodes): the same
  paths, closed over aliases, kept with the call.
- **Positions and spans.** The earliest position of a node that mentions each key, and the outermost loops and IIFEs.
  A walk from a node reaches only nodes the binder made before it, which start before it except across a loop's back
  edge or into an IIFE's arguments (bound before its body). So a mention counts only if it starts before the walk's
  start, or before the end of the loop or IIFE the start is in.

The checker skips a walk when declared and initial type are the same object, the declared type is not auto, not
auto-array and, if any or unknown, exactly `any`, `error` or `unknown` (unions with the unreachable never type keep
only those), the reference is an identifier, `this`, `super` or a path of at most 15 names, and in each graph the walk
can pass through (the reference's, then the containing function's while its `Start` node continues: the same
condition as `getTypeAtFlowNode`'s, per `flowContainer`, reference kind and arrow functions):

- no reachable mentioned key is the reference's path or a wildcard below one of its proper prefixes;
- for a union or literal declared type, no reachable assigned key is;
- every reachable call mentioning it has its effects signature computed already and it is none (a peek at
  `SignatureLinks::effects_signature`; computing it there is not safe, see below);
- no reachable mention of a global alias that may narrow the root;
- fewer than 500 flow nodes in the graphs passed. Along one recursion path a walk enters each flow node at most twice
  (a loop label is analyzed again only on a restart), so a skipped walk could never have reached the 2000-frame limit
  that reports TS2563 and disables flow analysis (testdata/flow-memo/depth-limit is such a case and stays exact).

Then every path ends at the initial type, at an unreachable node (the declared type), or at a never-returning call
(the unreachable never type, which `getFlowTypeOfReference` maps to the declared type), and the walk returns the
declared type. Declaration files get no index (no code to walk); flow nodes the checker makes (type predicate
inference) have no graph, so their walks always run.

### Where the index lives

The binder allocates a file's index where it binds the file: in the thread arena, or in the file's region for a
predicted leaf (tests, specs; notes/mem-free-leaf-files.md). A registry indexed by the file's text index maps flow
nodes to it (`FlowNode::text_index`, like compact identifiers' text). A leaf's region is freed while other checkers
run, so `fileregions::free` unregisters the index first, and the checker reads global aliases only from scripts (a
leaf is always a module). `recycle_checkers` (`--maxMemory` with poisoned retired regions) caught the first version,
which built the global alias map from every file's index in a checker created after a leaf was freed.

### What a skipped walk does not do

A walk has effects besides its result: it resolves the names in the conditions it passes, computes the effects
signatures of the calls it passes, marks assignments, fills `flowLoopCache` and the flow memo. A skipped walk does
none of that; another walk or the statement's own check does it later, or nothing needs it. tsrs's output already does
not depend on which checker resolves what first (notes/perf-order-independence.md), and this moves far less than a
different checker assignment does. The `--extendedDiagnostics` type count moves (vscode creates 5 fewer types), like
the memo's work counters. Every gate below compares output bytes.

## Exactness

- Shadow mode (`TSRS_FLOW_SKIP=shadow`: every skippable walk runs and must return the declared type, else panic):
  conformance suite with `.types` / `.symbols` clean (0 crashes, result tree identical to base), and seven corpora
  clean.
- Conformance suite, base against new: result trees identical (`treecmp.py`) for errors + `.types` + `.symbols` in
  tsgo's history (13,458 pass, 2 codes, 2 fail; 12,779 / 12,779), for js / jsmap / sourcemap (13,392 / 149 / 156),
  in the canonical mode at four checkers (`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`), and with
  `TSRS_LAZY_MEMBERS=0`.
- `--pretty false` output byte-identical to base on vscode (371 errors), webpack (840), xstate-main, cal-diy (136),
  t3code-server (6), drizzle-orm (10,846) and formbricks-web, single-threaded and at the default checker count.
- `tools/regressions.sh`: 28 / 28. `tools/ci/determinism.sh` (default set): 576 runs identical.
- `cargo test -p tsrs_cli --test flow_skip` (CI's "cargo test (fast crates)" step): eight cases in testdata/flow-skip,
  each tsgo-ref's output, run with the skip on, off and in shadow mode. They narrow through condition aliases (same
  scope, enclosing scope, module level after use, destructured, transitive, global script in another file, also as an
  assertion argument), `in` / `hasOwnProperty`, assertion calls, an IIFE's arguments, loops, `for...in`, assignments
  and walks that continue into enclosing functions. Disabling each part of the index in turn (alias closure, `in` keys,
  call mentions, effects peek, spans, `for...in`, assignment targets, the continuation, global aliases in calls) makes
  at least one case fail.

## Cost

Phase instructions (the never-committed measurement patch printing `proc_pid_rusage` instructions around bind and
check; same binary, `TSRS_FLOW_SKIP=0` against on, single-threaded, min of 3; these repeat to 0.01%):

| project | bind | check | bind + check |
| --- | --- | --- | --- |
| vscode | 6.350 -> 7.638 G (+1.288) | 73.653 -> 71.168 G (-2.485) | -1.197 G |
| t3code-server | 1.704 -> 1.864 (+0.160) | 38.781 -> 37.841 (-0.940) | -0.780 |
| formbricks-web | 2.343 -> 2.445 (+0.102) | 31.056 -> 30.625 (-0.431) | -0.329 |
| cal-diy | 1.664 -> 1.735 (+0.071) | 27.473 -> 27.180 (-0.293) | -0.222 |
| drizzle-orm | 1.053 -> 1.099 (+0.046) | 8.770 -> 8.605 (-0.165) | -0.119 |
| webpack | 0.804 -> 0.909 (+0.105) | 9.645 -> 9.442 (-0.203) | -0.098 |
| xstate-main | 0.350 -> 0.366 (+0.016) | 4.950 -> 4.923 (-0.027) | -0.011 |

The binder's share (vscode, samply): closing over aliases, collecting paths and sorting each graph's keys. The checker's
share: the binary searches and the call peeks. `TSRS_FLOW_SKIP=0` (no index, no check) is within 0.13% of main
(t3code-server aside, whose base runs were bimodal).

Memory: the index is arena data kept for the program (per graph 24 bytes; per key 8 with its position; per call
mention 8). vscode: 321 K graphs, 840 K keys, 876 K call mentions, 21.4 MB; webpack 1.5 MB; t3code-server 2.8 MB;
formbricks-web 2.1 MB; cal-diy 1.2 MB; drizzle-orm 0.9 MB; xstate-main 0.3 MB. Peak footprint +0.7% on vscode and
+0.8% on webpack, under 0.25% elsewhere.

## Measured and rejected on the way

- **Root names instead of paths.** One `this.foo()` call statement blocked every `this.*` reference of the method, and
  every declaration with an initializer counted as a mention: 543 K skippable walks on vscode, about 0.5% at best.
- **Aliases by name across the whole file.** A name like `options` declared in a hundred functions merged a hundred
  initializers into every graph that tests it; on vscode the binder phase cost +2.6 G instructions (+41%). Scoping
  aliases to the enclosing graphs and to const, untyped declarations (all Go accepts) fixed it.
- **Computing a call's effects signature in the skip check** instead of peeking: the call-blocked walks are the most
  expensive kept walks (212 K walks, 0.95 M steps on vscode, much of it first-time effects signatures), but computing
  them from the check overflowed the 512 MB stack on vscode: `getEffectsSignature` can resolve the call's signature,
  check its arguments and start nested walks, whose checks compute more, in an order no walk produces.
- **Ignoring calls** (unsound, measured only): 1.71 M skippable walks on vscode, 2,629 of them wrong (`assert*`).
- **Binder pitfalls.** `FxHashMap::clear` costs the map's capacity, so one large graph made every later small graph in
  the file pay; an `std::env::var` read per closure call; walking whole call arguments (object literals in
  `registerConfiguration(...)`); one file-wide sort of interleaved per-graph entries (now sorted per graph when its
  container ends).

## Follow-ups (not done)

- Calls whose effects signature is unknown when the check runs. A walk computes it for the calls on its path, but a
  call in a sibling branch before the reference stays unknown for every later reference. Knowing which calls are on the
  path (dominance in the binder's graph) would clear most of these exactly.
- The remaining binder cost on vscode (+1.29 G) is mostly the alias closure; a per-graph Bloom filter could also cut
  the checker's binary searches for references a graph never mentions (most skipped walks), for 8 bytes a graph.
- The node limit (500) is conservative; a tighter recursion-depth bound would release t3code-server's 55 K walks.

## Reproduce

```sh
cargo build --release -p tsrs_cli
TSRS_FLOW_SKIP=shadow target/release/tsrs -p <project> --noEmit --singleThreaded   # panics on a wrong skip
TSRS_FLOW_SKIP=shadow target/release/tsrs-test run --suite all --baselines types,symbols --panic-summary
cargo test -p tsrs_cli --test flow_skip
# A/B in one binary: TSRS_FLOW_SKIP=0 builds no index and skips nothing.
```
