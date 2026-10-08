# perf-excalidraw-typefest: why tsrs loses to bun check on excalidraw and type-fest

Two projects from the bun check set (#206) are where tsrs loses to `bun check` by far the most. On excalidraw, tsrs was
2x slower than bun and used more memory than tsgo. On type-fest, tsrs used 1.8x bun's memory. This note covers where
the time and memory go on each, what comes from tsrs and what comes from the Go design, and the fix candidates. One
fix is in this branch: on excalidraw it cuts peak memory by 66% at 9 checkers and by 80% at 22 checkers, the
Linux scoreboard's count.

All numbers are from the Mac (Apple M5 Max, 18 cores, 16 KiB pages, shared with other agents at a 1-minute load of
15-40). Depot was not used, so there are no Linux numbers. The binaries:

- base: `origin/main` 990d32d5
- fix: this branch
- bun: the canary 1.4.3-canary.1+620b50f6a
- tsgo: `tsgo-ref`

Defaults on this Mac are 9 checkers for tsrs, 18 threads for bun, and 4 checkers for tsgo. Peak is the maximum RSS
from `/usr/bin/time -l`. Instructions are instructions retired.

| project | base: wall / peak / instructions | fix | bun | tsgo-ref |
| --- | --- | --- | --- | --- |
| excalidraw | 0.43 s / 1,460 MiB / 39.5 G | 0.30 s / 498 MiB / 21.4 G | 0.20 s / 439 MiB / 15.9 G | 1.57 s / 1,763 MiB / 78.6 G |
| type-fest | 0.89 s / 1,752 MiB / 61.9 G | unchanged | 1.07 s / 967 MiB / 50.2 G | 3.62 s / 5,589 MiB / 225 G |

These are medians of 5 interleaved runs (tsgo: 1 run). Wall times are noisy at this load. Peaks repeat within 1% on
excalidraw and within 4% on type-fest.

## excalidraw: one type-argument constraint check, repeated in every checker

### What the hypotheses gave

- **The front end is small.** `--noCheck` takes 0.07 s and 140 MiB.
- **The switches change nothing.** `TSRS_LAZY_DTS=0`, `TSRS_LAZY_MEMBERS=0` and `TSRS_FREE_LEAVES=0/1` stay within 2%
  of the default at 1 and 9 checkers.
- **The config is not the cause.** jsx, paths and allowJs do not matter. The overlay only drops `baseUrl`.
- **The cost grows with the checker count.** At one checker tsrs (353 MiB) is close to bun with one thread
  (285 MiB). From there, base costs about 140 MiB per extra checker:

  | checkers | 1 | 2 | 4 | 9 | 16 | 23 | 32 |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
  | base peak (MiB) | 353 | 505 | 786 | 1,472 | 2,409 | 3,304 | 4,463 |
  | base instructions (G) | 14.9 | 19.2 | 25.1 | 39.5 | 58.3 | 76.9 | 99.5 |
  | fix peak (MiB) | 354 | 383 | 427 | 503 | 573 | 630 | 703 |
  | fix instructions (G) | 15.0 | 16.5 | 18.3 | 21.3 | 24.3 | 26.5 | 29.4 |

  Bun grows from 285 to 436 MiB between 1 and 18 threads, and its instructions stay at 15.9 G.

### Where the memory went

At 9 checkers, the alloc-profile build (`TSRS_ALLOC_PROFILE_TOP=40`) attributes most of the arena to node builder
output:

- 221.8 MB of `NodeBuilderContext` (559K of them; 62K at one checker)
- 125 MB of identifier nodes
- 99 MB of property signature nodes
- 56 MB of `Node`
- 40 MB of strings

The heap profile shows more of the same: 66 MB from `enter_context`, about 100 MB from `add_emit_flags`, 55 MB of
message strings formatted in `report_error`, and 50 MB of node builder hash maps. Every one of these is
`type_to_string` / `symbol_to_string` output, made by the relater's error reporting (`report_relation_error`,
`report_unmatched_property`).

### Which check produces it

A backtrace was taken at every 500th `NodeBuilder::enter_context`. 124 of the 125 backtraces pass through the same
chain: `check_type_argument_constraints` ← `check_type_reference_or_import` ← `check_signature_declaration` ←
`contextually_check_function_expression_or_object_literal_method`. Counting node builder calls per call of
`check_type_argument_constraints` points to one site, `packages/excalidraw/tests/helpers/api.ts:7105`, the return
type of `API.createElement`:

```ts
static createElement = <T extends ...>({...}): NonDeleted<
    T extends "arrow" | "line" ? ExcalidrawLinearElement
    : T extends "freedraw" ? ExcalidrawFreeDrawElement
    : ... /* 8 levels */ : ExcalidrawGenericElement> => { ... }
```

`NonDeleted<TElement extends ExcalidrawElement>` makes the checker relate the 8-level conditional type to the 11-member
`ExcalidrawElement` union. The check passes and reports nothing. The relater first tries the conditional's default
constraint, with error reporting on (relater_2.rs, the `Conditional` source branch). That attempt fails, and every
failing member is elaborated with `type_to_string`. Then the distributive constraint succeeds, and
`restore_error_state` throws the elaboration away.

The cost depends on how warm the checker is:

| checker | node builder calls | time |
| --- | ---: | ---: |
| the only checker, at 1 checker | 62K | 215 ms |
| checker 0 of 9, which checked 910 files first | 80K | 309 ms |
| the other 8 checkers, each reaching the site cold | 380K-457K | 340-400 ms |

**Every checker reaches the site.** `createElement` is a static property whose initializer is an arrow function. Any
test file that calls `API.createElement` needs the property's type. Computing that type checks the arrow function
(`check_function_expression_or_object_literal_method`), and that checks its signature once per checker, including the
type-argument constraints of the return type. With 9 checkers, all 9 ran the check. Only the checker that checks
`api.ts` keeps the diagnostics; the other 8 discard theirs.

**What removing the site gives:**

- Skipping that one call (instrumented build): 1,480 → 379 MiB and 40.8 → 19.3 G at 9 checkers, check time
  0.375 → 0.161 s. At one checker: 354 → 234 MiB.
- Dropping the constraint from the source (`NonDeleted<TElement>`): tsrs uses 383 MiB and 0.26 s at 9 checkers, bun
  375 MiB.

**Printing, not the relation, is the cost.** Two experiments, both env-gated and not committed, measured at 9
checkers:

- `type_to_string` returns the type id instead of printing: 400 MiB, 21.2 G, check 0.160 s. So printing is about 95%
  of the site's cost.
- The relation is checked without reporting first, and reporting runs only on failure: 379 MiB, 19.3 G, 0.148 s.

### Inherent or tsrs-specific

The cost comes from the Go design, and tsgo pays more of it:

| | tsgo-ref single | 4 checkers | 9 checkers |
| --- | --- | --- | --- |
| original source | 903 MiB / 40.4 G | 1,763 MiB / 78.6 G | 3,274 MiB / 134.5 G |
| constraint dropped | 651 MiB / 34.6 G | 900 MiB / 51.7 G | 1,190 MiB / 61.3 G |

Two Go design choices cause it:

- typescript-go runs this check at once in every checker. TypeScript (JS) defers it with `addLazyDiagnostic` in
  `checkTypeReferenceOrImport`.
- typescript-go elaborates eagerly: `checkTypeRelatedTo` relates with `reportErrors` on from the start.

Bun avoids both:

- Its relater runs without reporting and elaborates only a pair that fails (`src/sema/check/explain_relation.rs`: "A
  pair that fails the non-reporting run is compared again with reporting").
- All threads share one type graph.

Bun uses 16.10 G instructions on excalidraw and 16.04 G with the constraint dropped.

## The fix in this branch: run the check in the checker that checks the file

In the default (canonical) history mode, `check_type_reference_or_import` no longer runs the type-argument constraint
check when its node lies in a file that this checker has neither started nor finished checking. It queues the node
under that file (`Checker::deferred_type_argument_checks`). `check_source_file` runs the queued checks for its file
first, each as checking the type reference itself would run it: `current_node` set to the node, `instantiation_count`
reset. A checker that never checks the file drops the queue.

This is TypeScript's `addLazyDiagnostic` for this one call site. In Go-compatible history mode (`--checkerAssignment
go`, the tsgo-baseline harnesses), the check still runs at once, as in Go.

**Why output does not change.** The check only reports diagnostics, and its result is not used. A checker returns a
file's diagnostics right after it checks that file (notes/perf-order-independence.md). So:

- In a checker that never checks the file, the diagnostics were already discarded.
- In the checker that does check it, they were produced before the file was collected, and now they are produced at
  the start of its check.
- If the file is being checked or was already checked, the check still runs at once.

Work stealing is covered, because ownership is decided when `check_source_file` runs, not ahead of time. Pieces of a
split declaration file are checked with `checking_file` set, so their checks also run at once.

What does move is when the relation cache is filled. That is the history channel (complexity budgets, circularity
entry points) that the order-independence note already treats as theoretical and tests with random assignments.

### Numbers (Mac, interleaved medians)

| project | mode | peak base / fix (MiB) | wall base / fix (s) | check base / fix (s) | instructions base / fix (G) | n |
| --- | --- | --- | --- | --- | --- | ---: |
| excalidraw | default (9) | 1,472 / 503 (-65.8%) | 0.51 / 0.36 | 0.410 / 0.271 | 39.54 / 21.34 (-46.0%) | 5 |
| excalidraw | 4 checkers | 786 / 427 (-45.6%) | 0.54 / 0.48 | 0.454 / 0.394 | 25.06 / 18.25 (-27.2%) | 5 |
| excalidraw | 1 checker | 353 / 354 | 1.02 / 1.12 | 0.935 / 1.041 | 14.93 / 14.98 | 3 |
| excalidraw | single-threaded | 344 / 344 | 1.03 / 1.02 | 0.804 / 0.824 | 14.64 / 14.61 | 4 |

Twelve other projects at the default count (3 reps each) are neutral:

- Peak memory moved by -0.4% to +0.6%: type-fest, nest, rxjs, zod, t3code-server, formbricks-web, cal-diy,
  supabase-studio, vscode, webpack, xstate, mui-docs.
- Instructions moved by -0.7% to +0.5%.
- Walls were within the noise of the load.

The Linux scoreboard runs excalidraw at 22 checkers: 712 checked files / 32. At 22 checkers on the Mac, the fix takes
peak memory from 3,187 to 627 MiB (-80%), instructions from 74.5 to 26.7 G, and check time from 0.64-0.82 to
0.25-0.27 s (two runs each). The Linux numbers still need a run on the 64-vCPU runner, once #206's projects are on main.

### Gates

- **Conformance**, `tsrs-test run --suite all --baselines types,symbols`: 13,458 pass / 2 codes / 2 fail;
  12,779 types; 12,779 symbols. The result trees are identical to base in five modes:
  - default
  - `TSRS_LAZY_MEMBERS=0`
  - `TSRS_HISTORY=canonical`
  - `TSRS_HISTORY=canonical TS_TEST_PROGRAM_SINGLE_THREADED=false` (4 checkers per test)
  - `TSRS_CHECKER_ASSIGNMENT=random:1 TS_TEST_PROGRAM_SINGLE_THREADED=false`
- **The suite barely exercises the change.** A traced build counted 8 deferred checks over the whole suite in the
  random 4-checker mode and 1 run at a file's check, with no diagnostic. The new case
  `testdata/regressions/deferred-type-argument-check` covers it. It is a two-file program in which checking `a.ts`
  first computes the type of `b.ts`'s arrow function, whose `Box<number>` breaks its constraint. It is checked by
  `tools/regressions.sh` (single-threaded) and by `crates/tsrs_cli/tests/deferred_type_argument_check.rs` (1-3
  checkers, 8 random assignments each, Go mode). With the call in `check_source_file` removed, the case loses its
  TS2344 and fails.
- **Fourslash**: 4,066 pass, 63 fail; the result tree is identical to base.
- **Other checks**:
  - `cargo test -p tsrs_cli` is green, with `api::memory_tests` cfg'd out locally because it does not link on macOS.
  - `RUSTFLAGS="-D warnings" cargo check --workspace --locked` is clean.
  - `tools/lint/ratchet.py` is ok (11 findings, none new). `tools/lint/source.py` is ok.
  - `tools/regressions.sh` passes every case.
- **Diagnostics are byte-identical to base at 1, 4, 16 and 32 checkers** on excalidraw, type-fest, nest (262 errors),
  zod (31), rxjs (6,158), t3code-server, formbricks-web, cal-diy (136), supabase-studio, vscode (371), webpack (840),
  xstate and mui-docs. rxjs was compared under `--checkerAssignment locality`, and under `random:1-3` at 8 checkers,
  because base is not deterministic there (next section).

### Found on the way: rxjs output depends on the assignment, and run to run (not fixed)

On base, rxjs prints the same 6,158 errors in every configuration, but 5 of them have different elaboration text
depending on which checker checks what:

- `group-by.spec.ts(24,41)`
- `multicast.spec.ts(33,19)`
- `multicast.spec.ts(184,77)`
- `concat-all.spec.ts(36,48)`
- `merge-all.spec.ts(45,48)`

For example, one run prints "The types returned by '[bufferTime](...)[groupBy]'" and another prints
"'[bufferTime](...)[bufferTime](...)[groupBy]'". The output differs:

- between 1 and 16 checkers
- between `random:1`, `random:2` and `random:3`
- from run to run at the default scheduling with 16 checkers, because stealing depends on timing

So rxjs breaks the "output is a function of the program" property in notes/perf-order-independence.md. Which
elaboration path the relater reports depends on what the relation cache already holds. This needs its own
investigation; the fix in this branch leaves rxjs's output unchanged under every fixed assignment.

## type-fest: tuple targets with a type parameter and a symbol per element

**Time is even with bun.**

- One checker: tsrs takes 2.52 s and 47.6 G instructions; bun with one thread takes 2.62 s and 50.0 G.
- At the defaults: tsrs takes 0.89 s; bun takes 1.07 s.

Six test files take 72% of the check CPU (`--checkerCostCache`):

| file | share of check CPU |
| --- | ---: |
| `int-range.ts` | 22% |
| `union-to-tuple.ts` | 13% |
| `exclude-rest-element.ts` | 10% |
| `extract-rest-element.ts` | 10% |
| `array-reverse.ts` | 10% |
| `split-on-rest-element.ts` | 7% |

`int-range.ts` alone (0.71 s) bounds the wall at any checker count. These files build tuples of every length up to
about 1,000 at the type level.

**Memory is the gap, and it is there at one checker.**

| | 1 | 2 | 4 | 9 | 18 |
| --- | ---: | ---: | ---: | ---: | ---: |
| tsrs peak (MiB), by checkers | 983 | 1,088 | 1,457 | 1,683 | |
| bun peak (MiB), by threads | 468 | | 617 | 904 | 987 |

tsrs's peak grows by about 88 MiB per extra checker. tsgo-ref takes 4,131 MiB single-threaded (175.5 G instructions)
and 5,589 MiB at 4 checkers, so tsrs is already 4x below tsgo, and bun is 2x below tsrs.

**Where the arena goes at one checker** (alloc profile, 768 MB requested):

| allocation | size |
| --- | ---: |
| `TypeParameter` (4.63M) | 247 MB |
| `Symbol` (5.57M) | 170 MB |
| value-symbol link pages | 85 MB |
| `TupleElementInfo` | 35 MB |
| union and type-argument lists | 33 MB + 29 MB + 24 MB |
| `all_type_parameters` lists | 17.5 MB |
| index-name strings (3.8M of them, `"0"`, `"1"`, ...) | 10.9 MB |

The cause is `create_tuple_target_type`. As in Go, each tuple target (one per arity, flags, readonliness and labels)
gets:

- a fresh type parameter for every element
- a property symbol named after the index for every element before the first rest element
- a value link holding that symbol's type
- an element-info entry

An instrumented build counted the targets and their elements:

- One checker built 7,543 tuple targets with 4.58M elements.
- 9 checkers built 17,956 targets with 8.89M elements: each checker builds the targets its own files need.
- The type parameters of 99.4% of the targets are read later, by mappers for the targets' references. So creating
  them lazily would save nothing.

Per-element state is about 480 MB of the 768 MB arena at one checker. It is inherent to the Go design: tsgo makes the
same objects.

**Bun shares the element type parameters across all targets.** In `src/sema/types.rs`, `Marker::TupleElement(i)` says:
"`createTupleTargetType`: `typeParameters` and `thisType`. All targets share them: only a mapper ever observes them."
Bun also caches the member shape per list of element flags.

## Fix candidates, ranked

1. **Defer the type-argument constraint check to the checker that checks the file (this branch).** On excalidraw:

   - -66% peak and -46% instructions at 9 checkers, wall 0.51 → 0.36 s
   - -80% peak at the Linux count of 22 (Mac)
   - neutral on 12 other projects

   It is exact in Go mode by construction. In canonical mode, output is unchanged by the order-independence
   argument, and the gates confirm it.

2. **type-fest: share tuple-element type parameters across targets within a checker, as bun does.** The same idea
   extends to the per-element property symbols, keyed by (index, optional, readonly), and to the index-name strings.
   - **Expected gain:** most of the ~480 MB of per-element state at one checker (983 → ~550 MiB, about -44%), and
     about twice that at 9 checkers (1,683 → ~900 MiB). There are also fewer instructions spent creating 4.6M
     types and 3.8M symbols.
   - **Not exact by construction.** It needs an audit of every reader of a target's own type parameters: the target's
     `Array<T0 | ... | Tn>` base type, the target used directly as a type, printing, and `compare_types`' type-id
     fallback for type parameters without a symbol.
   - This is the largest open item for type-fest.

3. **Format relation error arguments lazily.** Keep the types and print them only when the error chain becomes a
   diagnostic.
   - **Expected gain:** what the fix leaves on excalidraw is the checker that owns `api.ts` elaborating once. That is
     501 → ~382 MiB and check time 0.26 → 0.16 s at 9 checkers, as measured with printing stubbed out. The same
     saving applies anywhere a relation elaborates and then restores its error state: the conditional
     default-then-distributive constraint, `someTypeRelatedToType` on intersections, and so on.
   - **Risk:** printing has side effects, which the comment on `type_to_string_ex` names: lazy member resolution that
     can report diagnostics, and creating types. Deferring or dropping them is a history change, so Go mode would
     need to keep eager printing.
   - **Required change:** `report_relation_error` picks its message by comparing printed strings
     (`source_type == target_type`), so that choice must move to render time.
   - **Also removes:** 55 MB of message strings that `report_error` formats eagerly.

4. **Check the relation without reporting first, as bun does.** It gains a little more than candidate 3 (379 MiB,
   check 0.148 s) and is simpler. It is **not exact against tsgo**: a reporting run recomputes cached failures, so it
   spends more of the relation complexity budget (`(16M - cache size) / 8`, about 2M relations). Where tsgo reports
   TS2859 "Excessive complexity", a non-reporting first pass can succeed and print nothing. The excalidraw site
   already elaborates hundreds of thousands of failures, within an order of magnitude of that budget. Rejected as a
   direct port. The exact form is candidate 3.

5. **Use a scratch region for checker-side `type_to_string`.** This saves memory only, with no CPU gain. The cached
   `type_to_string` builder's `id_to_symbol` map and `serializedTypes` cache keep its nodes, which is why
   notes/mem-emit-regions.md left them in the checker arena. After the fix, only the owning checker's ~120 MiB on
   excalidraw would be reclaimable.

6. **Intern tuple index names.** It is exact but small: 10.9 MB at one checker on type-fest (1.1%), below the bar.

Not causes:

- the front end
- the jsx, paths and allowJs settings, and the excalidraw overlay
- `TSRS_LAZY_DTS`, `TSRS_FREE_LEAVES` and `TSRS_LAZY_MEMBERS` (lazy members already help type-fest:
  `TSRS_LAZY_MEMBERS=0` raises its one-checker peak from 983 to 1,367 MiB)

## Reproduce

```sh
# projects: from #206's branch, in a worktree (do not commit the overlays)
git checkout origin/bench/bun-pr-projects -- bench/projects.json bench/run.py bench/overlays/excalidraw bench/overlays/type-fest
python3 bench/run.py --setup-only --projects excalidraw,type-fest
cd bench/.work/solutions/excalidraw
F="--noEmit --incremental false --pretty false --extendedDiagnostics"
/usr/bin/time -l tsrs -p . $F --checkers 9                                 # peak, instructions
TSRS_ALLOC_PROFILE_TOP=40 target/prof/release/tsrs -p . $F --checkers 9    # --features alloc-profile build
TSRS_ASSIGNMENT_STATS=times tsrs -p . $F --checkers 9                      # per-checker seconds
BUN_INSTALL=~/Developer/tsrs-work/bun-canary bun check -p $PWD --no-pretty --all --threads=9   # from an empty cwd
# type-fest per-file costs (locality, 2+ checkers)
tsrs -p . --noEmit --checkers 4 --checkerAssignment locality --checkerCostCache /tmp/tf-cost.txt
```

The node builder backtraces, the per-site node builder counts, the printing and two-pass switches, and the
tuple-target census were local instrumentation, not committed. Each is a few lines in
`NodeBuilder::enter_context`, `check_type_argument_constraints`, `type_to_string_ex` / `symbol_to_string_ex`,
`check_type_related_to_ex` and `create_tuple_target_type`.
