# Union front cache and the generic-call inference memo (2026-10-05)

Tasks 2 and 3 of the flow-memo handoff (notes/perf-flow-union-inference.md; their briefs are not kept), under the round-3
rule (notes/perf-round3.md): caches Go does not have are allowed. Diagnostics, determinism, emit and the conformance
baselines must not change, and anything on by default that departs from Go is off under `--checkerAssignment go`.

- **Union front cache:** landed behind `TSRS_UNION_CACHE`, on by default. -0.5% to -1.1% instructions on every corpus
  with 1, 4 and 8 checkers, check time -1% to -3% with one checker, no memory change. Exact on everything run: the
  suite, fourslash, the regression cases and four corpora, in shadow mode too.
- **Generic-call inference memo:** not built. It failed the 1.5% test. The part a memo can skip is 1.3% of check time
  on the 38k-file codebase, 2.2% on vscode, and 0.3-0.4% on webpack and xstate. The census's ~8% is mostly argument
  checking.

## Method

- Machine: a Linux x86-64 sandbox with 18 vCPUs and 47 GiB of memory. All numbers here come from it, not from the
  Apple Silicon machine in the other round-3 notes.
- Corpora:
  - the 38k-file codebase, with workspace packages unbuilt (~43.9k diagnostics, so error-rich);
  - vscode `src`, webpack and xstate, at the commits and with the install steps in `bench/projects.json`.
  - mui was not set up.
- Instructions: `perf stat -e instructions:u` over the whole process, run without a wrapper.
  - Base is a release build of origin/main 453a12d (`cargo build --release`, without PGO); new is the same tree plus
    the change, built the same way.
  - Five interleaved rounds after a warm-up, medians.
  - Single-threaded runs (`--singleThreaded`, `RAYON_NUM_THREADS=1`) are deterministic, so every paired delta is
    identical.
  - With 4 and 8 checkers, work stealing makes the counts vary from run to run by up to ±2%.
- Check time: from `--extendedDiagnostics`. Max RSS: from `/usr/bin/time -f %M`.
  - With several checkers, max RSS varies by ±3-4% between runs of one binary, because stealing changes which checker
    rebuilds which shared types.
  - So peak memory with several checkers was also measured with `--checkerAssignment go`, the static assignment (see
    "Memory").

## Task A: the union front cache

### Where a small union call spends its time

Flat profile of the 38k-file codebase with one checker (`perf record -F 1999`, cycles, self time). It shows where the
time goes, not instruction counts:

| function (self) | base | with the cache |
| --- | --- | --- |
| `compare_types` + `compare_types_same_flags` (the sort in `addTypesToUnion`) | 0.75% | 0.66% |
| `get_union_type_from_sorted_list` (key, `unionTypes` lookup) | 0.60% | 0.45% |
| `get_union_type_worker_inner` (flags, reductions) | 0.51% | 0.36% |
| `add_types_to_union` + `sort_stable` | 0.53% | 0.38% |
| `get_union_key` | 0.13% | 0.06% |
| `get_union_type_ex` (the 2-input union-of-unions map) | 0.23% | 0.10% |
| `get_union_type_front_cached` | - | 0.50% |

- Before the cache, a call that builds an existing union spends its time on three things:
  - the structural sort (`compareTypes` compares names, symbols and type-argument lists, not ids);
  - building and hashing the `keyBuilder` key (`hash_128` is 0.5% overall, shared with other caches);
  - the hash map probe, often a cache miss in a table with millions of entries.
- The front cache's own cost is mostly one probe into its table: 43% of its samples are on the slot load. Another 15%
  are a store-forwarding stall when the key array is compared (see "Rejected").
- The census's 9.3% for union construction includes `removeSubtypes`, the first construction of every union, and
  every nested call. The calls a cache can answer are the cheap ones: a hit saves about 600 instructions. So the gain
  is 0.5-1.1%, not 9%.

### The pattern

`getUnionType` with two or more inputs and no origin probes a direct-mapped table before doing any other work
(`crates/tsrs_checker/src/unioncache.rs`; the hook is in `checker_12.rs` `get_union_type_ex`).

- The table has 2^10 slots of 40 bytes, one table per checker, allocated on first use.
- The key is:
  - the input handles in input order;
  - the reduction mode;
  - with an alias, its symbol and type arguments, which is what `getAliasKey` hashes.
- A key has at most 8 words. Longer keys, and calls with an origin, take the normal path.
- On a miss the normal path runs, and its answer is stored if the call passes the conditions below.

Hit rates with one checker:

| corpus | lookups | hits | not stored: created a type / instantiated / impure / error type | longer keys | origin |
| --- | --- | --- | --- | --- | --- |
| 38k-file codebase | 4.51M | 71.1% | 223K / 4.6K / 5.8K / 55K | 298K | 39K |
| vscode | 1.74M | 78.7% | 21K / 4.1K / 35 / 485 | 31K | 4.3K |
| webpack | 240K | 78.5% | 4.0K / 0.3K / 117 / 1.9K | 4.0K | 1.6K |
| xstate | 187K | 76.1% | 6.5K / 150 / 42 / 12 | 20K | 1.2K |

Most misses are the first call for a key; the 2^10-slot table loses only a few points of hit rate to conflicts.

### Exactness

A call is stored only if all four of these hold:

1. **It created no type, or only the union it returns.** That is, `type_count` is unchanged, or it grew by one and the
   result has the newest id.
   - This rules out storing a call that creates a fresh origin for named unions, or anything that a nested relation or
     resolution creates.
   - So the cache never changes how many types are created or which ids they get. Ids decide union order wherever
     names tie, so this matters.
   - The `Types` counter is unchanged.
2. **It instantiated nothing.** `instantiation_count` and `total_instantiation_count` are both unchanged.
   - So the TS2589 budget sees the same counts with or without the cache.
   - The `Instantiations` counter is unchanged.
3. **It took neither of two reductions whose answer reads state that can still change.**
   - Removing string literals matched by template literals runs relations and template inference.
   - Removing constrained type variables reads base constraints, which return circularity markers while resolving.
   - Both increment `union_front_cache.impure`.
4. **It did not return `errorType`.**
   - TS2590 ("too complex to represent") reports at `currentNode` and caches nothing, so every such call must run again
     and report again.
   - An `any` input that is an error type also returns `errorType`, so it is not stored either.

Why a hit is then exact:

- After such a call, the uncached computation of the same inputs depends only on the inputs and on caches that only
  grow:
  - `unionTypes`;
  - `unionOfUnionTypes`;
  - `subtypeReductionCache`. A successful subtype reduction is stored there, so its second run does no relations.
- So the second run takes the same path through the same entries and returns the same object, creating and
  instantiating nothing.
- This covers every kind of answer:
  - a non-union answer: one distinct member, `never`, `null`/`undefined` widening, `any`/`unknown`/wildcard
    absorption;
  - subtype reduction;
  - the alias case: a pending alias is allocated only when a union is created, and a cached union already has one.
- Types are never freed while their checker lives (only mappers, inference contexts and type lists are recycled), so a
  handle in a key never comes to name another type.
- Under `--checkerAssignment go` the cache is off by default. `TSRS_UNION_CACHE=1` or `shadow` turns it on there too.
  Nothing observable differs, `--extendedDiagnostics` counters included.

### Shadow mode

`TSRS_UNION_CACHE=shadow` computes the uncached answer at every hit and panics if it is a different object
(docs/DEBUGGING.md). It found no differences anywhere:

| run | shadow checks | result |
| --- | --- | --- |
| conformance suite, `.types`/`.symbols`, default and `TSRS_LAZY_MEMBERS=0` | - | result trees identical to base, 0 crashes |
| fourslash | - | pass/fail/skip lists identical (4,066 / 63 / 417) |
| `testdata/regressions` (8 cases, including `conditional-instantiation-limit-*`) | - | identical except the stats line |
| 38k-file codebase, default checkers / 4 / `--checkerAssignment go` | 5.00M / 4.00M / 5.34M | diagnostics byte-identical |
| vscode, the same three | 1.50M / 1.44M / 1.52M | identical |
| webpack | 246K / 215K / 250K | identical |
| xstate | 159K / 152K / 160K | identical (0 errors) |

The table above is from the final build. All of these runs were done twice: once with a 2^12-slot table and once
with the final 2^10. Both were clean. The first version had a 4-input key without aliases and never stored the call
that created the union; it was shadow-checked on webpack only, and was clean.

### Gates, against origin/main 453a12d

- **Suite:** `tsrs-test run --suite all --baselines types,symbols`, in default mode and with `TSRS_LAZY_MEMBERS=0`.
  The result trees are identical: 13,458 pass, 2 codes, 2 fail; 12,779 `.types` and 12,779 `.symbols`.
- **fourslash:** 4,066 pass, 63 fail, 417 skip, with identical lists.
- **`tools/regressions.sh`:** all 8 cases pass.
- **Lints:** `RUSTFLAGS="-D warnings" cargo check --workspace --locked`, `tools/lint/ratchet.py` (no new findings) and
  `tools/lint/source.py` all pass.
- **Diagnostics:** byte-identical to base on all four corpora, with default checkers, with `--checkers 1` and with
  `--checkerAssignment go`. The same exit codes.
- **The checkout:** the 38k-file codebase's checkout is untouched (`git status` empty before and after).

### Before and after

Instructions are medians of 5 interleaved rounds. One checker means `--singleThreaded`. Check time is
`--extendedDiagnostics` "Check time" (wall).

| corpus | instructions, 1 checker | 4 checkers | 8 checkers | check time, 1 checker | max RSS, 1 checker |
| --- | --- | --- | --- | --- | --- |
| 38k-file codebase | 257.51 G -> 255.62 G (-0.73%) | 354.16 -> 351.50 G (-0.75%) | 476.23 -> 473.11 G (-0.66%) | 50.24 -> 49.36 s (-1.7%) | 4305 -> 4305 MiB (0.00%) |
| vscode | 118.76 -> 118.14 G (-0.52%) | 125.06 -> 124.43 G (-0.50%) | 130.54 -> 129.88 G (-0.51%) | 21.43 -> 20.75 s (-3.2%) | 2049 -> 2048 MiB (-0.05%) |
| webpack | 15.002 -> 14.924 G (-0.52%) | 16.83 -> 16.74 G (-0.51%) | 19.26 -> 18.98 G (-1.41%, stealing noise) | 2.078 -> 2.055 s (-1.1%) | 346 -> 346 MiB (+0.02%) |
| xstate | 7.550 -> 7.471 G (-1.04%) | 8.797 -> 8.704 G (-1.06%) | 9.834 -> 9.750 G (-0.85%) | 1.148 -> 1.110 s (-3.3%) | 223 -> 223 MiB (+0.02%) |

Check time with 4 checkers: the 38k-file codebase 16.09 -> 16.02 s, vscode 5.92 -> 5.96 s, webpack 0.680 -> 0.654 s,
xstate 0.363 -> 0.362 s. With 8: 11.30 -> 11.12 s, 2.94 -> 2.89 s, 0.356 -> 0.351 s, 0.235 -> 0.233 s. All of these
are within run-to-run noise except the one-checker column.

### Memory

- The table itself is 40 KiB per checker.
- **The first table size cost real memory.** 2^12 slots (160 KiB) cost about 2 MiB of RSS per checker: xstate with 8
  checkers and `--checkerAssignment go` went from 575 to 592 MiB (+2.9%). At 2^11 (80 KiB) it was +1.3%; at 2^10 it
  was +0.02%. An allocation above mimalloc's medium-object size seems to commit far more than it uses; this was not
  investigated further.
  - The larger table saves little: 2^12 gives 0.03% fewer instructions on the 38k-file codebase, 2^16 changes webpack's
    hit rate from 80.3% to 81.7%.
  - So the default is 2^10 (`TSRS_UNION_CACHE_BITS`).
- **Medians with work stealing look like a regression, but are noise.** With 8 checkers, the medians showed +0.4% to
  +1.7%, which is inside the ±3-4% noise. Three controls:
  - Same binary, cache off against on, 21 rounds, xstate with 8 checkers: 601.8 -> 597.4 MiB (-0.7%).
  - Static assignment, 7 rounds, webpack with 8 checkers: -0.5%.
  - Single-threaded max RSS changes by -0.05% to +0.02%.

### Rejected

- **Not storing the call that creates the union.** This was the first version, keyed on up to 4 inputs without
  aliases. It is correct but misses once more per distinct union: -0.62% on the 38k-file codebase, against -0.79% for
  the final version.
- **A 6-word, 32-byte slot compared word by word,** to avoid the store-forwarding stall and slots that straddle two
  cache lines. It cost +0.11% to +0.17% instructions (keys longer than 6 words bypass the cache, and the word loop
  costs more than the vector compare), and the cycle counts were within noise.
- **Larger tables:** see "Memory".

## Task B: the generic-call inference memo — not built

### The split

The census counts repeats of `inferTypeArguments` by (signature, argument type ids, mode, contextual type): 309K of
599K calls on the 38k-file codebase, about 8% of check time. That share includes checking the argument expressions,
which no memo can skip: each call site has its own nodes, and their types are the memo key.

A measurement build split it (patch in the appendix, not committed). It times every
`infer_type_arguments_worker` and subtracts from each call:

- **argument checking:** `check_expression_with_contextual_type` for each argument, `get_this_argument_type` and
  `get_spread_argument_type`;
- **nested inferences that run outside argument checking:** each of these is counted in its own row instead.

The rest (contextual-return-type inference, `inferTypes`, `getInferredTypes`) is the part a memo could skip.

- **The key:** the signature, the check mode, the inference-context flags (`AnyDefault` for JS, `NoConstraintChecks`),
  the argument count, the contextual type, the type of `this` and each argument type as checked for inference, and the
  spread type.
- **A call is eligible** under the brief's conditions:
  - it is not JSX;
  - no argument is context-sensitive;
  - no outer inference context exists;
  - the inference context is fresh (no candidates and no return mapper, so this is not a second pass);
  - it has no explicit type arguments (calls with explicit type arguments never infer).
- **A call counts as skippable** if it is eligible and its key was seen before.

One checker, `--singleThreaded`. The figures for the 38k-file codebase are the median of three runs: 646, 634 and 655
ms skippable.

| corpus | check time | `inferTypeArguments` calls | repeats (any) | repeats eligible for a memo | their argument checking | **skippable** | share of check time |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 38k-file codebase | 48.1 s | 635K | 317K | 286K | 2.28 s | **0.65 s** | **1.3%** |
| vscode | 21.6 s | 273K | 153K | 121K | 1.06 s | **0.48 s** | **2.2%** |
| webpack | 2.15 s | 7.6K | 2.6K | 1.6K | 16 ms | **7 ms** | **0.3%** |
| xstate | 1.18 s | 16.4K | 5.3K | 4.7K | 27 ms | **5 ms** | **0.4%** |

Even the ineligible repeats would not pass the test: counting every repeat, the non-argument part is 1.9%, 2.7%, 1.0%
and 1.0%. Of the ineligible repeats on the 38k-file codebase, 17.6K had an outer inference context (204 ms) and 12.8K
a context-sensitive argument (69 ms).

### Decision

The brief's threshold is 1.5% on two corpora. Only vscode reaches it.

- These figures are an upper bound on what a memo could save:
  - the instrumentation's own lock and clock calls between the argument checks are counted as skippable;
  - a memo would add its own costs: building the key, looking it up, recording the inferred types, the inference
    flags and the return mapper, and adding the stored instantiation count on a hit.
- So task B stops here, as the brief says. The repeats the census sees are mostly argument checking:
  - on the 38k-file codebase the repeated calls cost 7.0% of check time: 1.9% inference and 5.1% argument checking (of
    which 4.7% is in eligible calls);
  - that argument checking is per call site.

The hazards were not checked against the Go reference, since nothing was built: `const` type parameters, `NoInfer`,
return-type priorities, intra-expression inference sites and the TS2589 counter.

## Appendix: the split measurement patch (not committed)

It applies to origin/main 453a12d. Run with `TSRS_INFER_SPLIT=1 tsrs -p . --noEmit --singleThreaded`; it prints four
`infer split:` lines on stderr. It is single-threaded only (a global mutex), and the timer includes a little of its
own overhead.

- `crates/tsrs_checker/src/infersplit.rs`, a new module (`pub mod infersplit;` in `lib.rs`).
  - It keeps a stack of frames: start time, excluded time, a flag for being inside an argument check, the key words,
    eligibility and the reason when ineligible.
  - `begin` and `end` wrap `infer_type_arguments_worker`. At the end of a frame:
    - if its parent is not inside an argument check, the frame's total is added to the parent's excluded time;
    - the frame's own `total - excluded` is added to the totals;
    - it is added to the repeat totals when its xxh3-128 key has been seen before.
  - `arg_begin` and `arg_end` wrap the three argument-checking calls and add their time to the frame's excluded time.
  - `word` appends a type id to the key.
- `checker_05.rs` `infer_type_arguments`, when enabled:
  - checks eligibility: JSX, `is_context_sensitive` on any argument, `get_inference_context(node)`, and whether the
    context has candidates or a return mapper;
  - starts the key with the signature, check mode, context flags and argument count;
  - wraps the worker in `begin`/`end`.
- `checker_05.rs` `infer_type_arguments_worker`:
  - pushes the contextual type id where the census pushes it;
  - wraps `get_this_argument_type`, each `check_expression_with_contextual_type` and `get_spread_argument_type` in
    `arg_begin`/`arg_end`, then pushes the resulting type id.
- `tsrs_cli/src/main.rs` calls `Checker::infer_split_report()` at exit.
