# Fix-wave workflow

All function bodies are ported; from here the conformance suite and the Go reference drive the work.
Several agents fix different failure clusters at the same time. This is how they avoid stepping on each other.

## Your worktree

`$TSRS_WORK` below is the directory that holds the main checkout (`$TSRS_WORK/tsrs`), the agent worktrees
(`$TSRS_WORK/wt/`) and the reference binaries (`$TSRS_WORK/bin/`). `$PRIVATE_PROJECT` is the tsconfig directory of
the large project used for the end-to-end checks: a 38k-file private TypeScript monorepo (5.9M lines), not part of
this repository.

- You get a private git worktree `$TSRS_WORK/wt/<agent-name>` on branch `fix/<agent-name>`,
  created from `origin/main`. Work only there (never in the main checkout `$TSRS_WORK/tsrs`
  or another agent's worktree). `ts-ref` inside it is a symlink to the Go reference checkout.
- Build with your own target dir: `CARGO_TARGET_DIR=$PWD/target cargo build --release -p tsrs_testrunner -p tsrs_cli`
  (release: the suite runs in ~25 s; a debug build is much slower to run).

## Landing changes (trunk-based, through GitHub)

Land small, often — every coherent fix, not one big drop at the end:

```sh
git add -A . && git commit -m "<area>: <what and why>"
git fetch -q origin main && git rebase origin/main        # resolve conflicts if any, keep both sides' intent
CARGO_TARGET_DIR=$PWD/target cargo check --workspace      # must be 0 errors, 0 warnings after the rebase
tools/lint/ratchet.py                                     # no new clippy findings (docs/RUST.md)
# re-run the tests you touched, and before every push a full-suite run to check for regressions (see below)
git push -q origin HEAD:main                              # if rejected: fetch, rebase, check, push again
```

Never force-push, never rewrite `origin/main`, never commit build output. If a rebase conflict is in code you do not
understand, read both versions and the Go source; do not blindly take one side.

## Running tests

- Full suite: `./target/release/tsrs-test run --suite all` -> summary table; per-class name lists in
  `target/test-results/{pass,codes,fail,crash,timeout}.txt`; `summary.json` has the first difference per test.
- One test / a cluster: `tsrs-test run --filter <substring|regex>`, `tsrs-test show <suite/name>` (expected vs actual),
  `tsrs-test crashes --top 20` (panics grouped by location).
- Classes: `pass` = baseline byte-identical; `codes` = same (file, line, col, code) set but message text differs;
  `fail` = different diagnostics; `crash`; `timeout`.
- **No regressions**: before each push, compare `pass.txt` against the list from before your change
  (`comm -23 <(sort before/pass.txt) <(sort target/test-results/pass.txt)` must be empty, or every lost test must be
  explained by an upstream change you just rebased onto). Record the new totals in your commit message.
- Reference behavior for anything outside the suite: `$TSRS_WORK/bin/tsgo-ref` (same commit
  as the port). Oracle tools for scanner/parser/binder/module resolution/printer live in `tools/oracle/` with binaries in
  `$TSRS_WORK/bin/`.

## Lazy member resolution (default on) and the opt-out mode

tsrs ports microsoft/TypeScript#64475 + #64526 (lazy member tables, `tsrs_core::lazymembers`, notes/lazy-members.md)
and runs them **by default**. `--noLazyMembers` / `TSRS_LAZY_MEMBERS=0` turns them off and restores the reference
behavior exactly (same counters, same baselines). The no-regression check above is therefore run twice before a push:
`TSRS_LAZY_MEMBERS=0 tsrs-test run --suite all --baselines types,symbols` must match the previous opt-out pass lists
exactly (that is the reference-equivalence gate), and the default run must not lose passes either. Comparisons
against `tsgo-ref` counters (symbols/types/instantiations) use the opt-out mode; with the default mode tsrs creates
fewer symbols, types and instantiations than `tsgo-ref` (it equals tsgo with both PRs applied).

## Checker assignment (multi-checker counters)

With more than one checker, tsrs assigns files to checkers by directory locality by default
(`--checkerAssignment locality`, notes/mem-assignment.md); Go uses FENNEL over single files. Diagnostics and emitted
files do not depend on the assignment (notes/perf-order-independence.md), but multi-checker
symbol/type/instantiation counters and peak memory do: compare them against `tsgo-ref` with `--checkerAssignment go`
(or `TSRS_CHECKER_ASSIGNMENT=go`). `go` also restores Go's check history in the one cache where tsgo's output depends on
what a checker saw first (`tsrs_core::compat`), so it is the mode for byte-identity with tsgo. The conformance,
fourslash and tsc harnesses run in it; set `TSRS_CHECKER_ASSIGNMENT=locality` to run them in the default mode.
`TSRS_CHECKER_ASSIGNMENT=random:<seed>` puts each file on a random checker with a random visit order in each checker:
the default mode must print the same text for every seed and checker count. Single-threaded runs have no assignment.
Without a named assignment the type-check pass also steals work: a checker that has run out of files takes unstarted
files from the back of the busiest checker (notes/perf-checker-stealing.md). Output is unchanged by it, but the
counters vary from run to run; naming an assignment (`--checkerAssignment locality`) turns stealing off for fully
reproducible counters. `TSRS_HISTORY=canonical` runs the tsgo-baseline harnesses in the default mode, stealing
included, without naming an assignment.
Each checker begins the type-check pass with the files of its queue whose static weight exceeds 1/200 of an average
checker's share, heaviest first, then the rest in program order (notes/perf-checker-64.md,
notes/perf-heavy-first-threshold.md); `TSRS_HEAVY_SHARE_DIVISOR=<n>` changes the divisor for experiments.
`--checkerCostCache <file>` (opt-in, locality only) balances the checkers on the previous run's per-file CPU times
(notes/perf-balance.md); `TSRS_ASSIGNMENT_STATS=times` prints per-checker wall and CPU seconds.
From 16 checkers on, the locality placement also counts the modules (files and their imports, library declaration
files included) a group shares with each checker, then moves groups while that lowers the modules held per checker
(notes/perf-clustered-assignment.md); `TSRS_MODULE_AFFINITY=off` restores the placement by import edges alone, and
`TSRS_MODULE_AFFINITY=<mu>`, `TSRS_MODULE_AFFINITY_GAMMA=<g>` and `TSRS_MODULE_AFFINITY_PASSES=<n>` are for experiments.
From 16 checkers on, a thief also keeps stealing from the checker it stole from last while that checker has at least
half the busiest one's work left; `TSRS_STEAL_STICKY=0` picks the busiest checker for every file.

## Why a TS2590 comes and goes: `TSRS_TRACE_UNION_REDUCTION`

By default a TS2590 no longer depends on the checker assignment: a result computed while one was reported is not
cached, so every file that evaluates the type reports it, at the innermost of its nested sites
(`Checker::too_complex_since`, `flush_too_complex_reports`, notes/open-history-dependence.md section 4). The trace below is for `--checkerAssignment go`, which keeps Go's caches and so Go's first-evaluation-only
reports, and for finding which union a TS2590 is about.

TS2590 ("Expression produces a union type that is too complex to represent") from `removeSubtypes` is decided after
100,000 comparisons: the number of comparisons per constituent so far, times the union's length, must stay at most
1,000,000. So only a union of more than 1,000 types can get it. `TSRS_TRACE_UNION_REDUCTION=1` prints one line on
stderr for every subtype reduction of such a union (crates/tsrs_checker/src/uniontrace.rs):

```
tsrs union reduction: at b.ts(4,20) while checking b.ts | 1101 -> 1101 types, too complex (TS2590) | 100000 comparisons, at 100000: 91 sources begun, estimate 1208898 | set 2d3aac47bef09b0c order bffdf1259fb1b3bf removed d1fba762150c532c | first [...] last [...]
```

`at` is the node the error is reported at and `while checking` the file the checker was checking; when they differ,
the error is filed against another file, and it is printed only if this checker checks that file afterwards: it is
lost if this checker already handed that file's diagnostics over or another checker checks it, and never collected for
a file that is not type-checked (a JSON file, a declaration file under `skipLibCheck`).
`-> n types` is what was kept (or left when it gave up), `from the cache` an answer reused from an earlier reduction of
the same list on that checker. The three fingerprints are built from the
types' symbols, declaration files (by base name) and positions, type arguments and literal values, not from type ids,
so they are comparable between runs and checker counts (the example is testdata/regressions/union-too-complex-canonical-order): if two runs disagree on a TS2590, a different `set` means the union itself differed, a
different `order` with the same `set` means its constituents were ordered differently, and the same `set` and `order`
with a different `removed` (or a different count at the 100,000 checkpoint) means some comparisons answered
differently. Tracing reads only what the types already hold and resolves nothing. notes/open-history-dependence.md
section 4 has what is known.

## `.types` / `.symbols` equivalence on the private monorepo against the cached reference

Running the Go oracle (`tools/oracle/project-types`) takes hours per side. Cache its equivalent instead (not
populated yet; the first full opt-out run creates it):
`$TSRS_WORK/project-ref-dump/<types|symbols>/manifest.<kind>` (hash, line count, path
per file, plus the `#counts` line), produced by `tsrs-test types-dump --mode <kind> --text none` in the opt-out
mode (`TSRS_LAZY_MEMBERS=0`; ~30 min and ~122 GB peak for `types`). That mode's `.types` walk was verified identical to the Go oracle on all 28,213 files
(notes/fix-project-types.md); `.symbols` only on the first 5,558 files (the Go run was stopped). Reference commit b85298b6 (nightly
7.1.0-dev.20260929); project: the pristine private-monorepo checkout
`$PRIVATE_PROJECT`
(read-only; never write there — run `tsgo-ref` on it only with `--incremental false`, or it rewrites
`dist/tsconfig.tsbuildinfo` and later runs skip checking). If either changes, the cache is stale.

- Quick tier (minutes): `tsrs-test types-dump -p tsconfig.json --out <dir> --mode types --text none --sample
  <repo>/target/project-types-sample.txt` (run from `$PRIVATE_PROJECT`), then `tools/project-types-compare.py
  $TSRS_WORK/project-ref-dump/types <dir> --subset`. The sample is ~2,000 files
  weighted toward zod schemas, ORM entities, workflows and router endpoints (`tools/project-types-sample.py`
  writes it; the list names private files, so it is not committed). The checker has fully checked the program before the walk either way, but the walk itself can
  create types and assign symbol ids, so a file that differs only in a sample run should be confirmed with a full run.
- Full tier (pre-landing for checker changes that can affect printing; ~40 min, up to ~125 GB peak, one at a time):
  the same without `--sample` and without `--subset`.

## The pristine private-monorepo checkout is read-only

`$PRIVATE_PROJECT`
belongs to a working checkout of the private monorepo. Two agents have already written into it by accident (a `tsbuildinfo` from a `tsgo-ref`
run without `--incremental false`; a bench script's results file). Rules: tsrs emits by default like tsc, so run both `tsrs` and
`tsgo-ref` there only with `--noEmit --incremental false` (cwd elsewhere), or with every output path (`--outDir`,
`--declarationDir`, `--tsBuildInfoFile`) redirected into your worktree's `target/` or `/tmp`;
anything that writes (mutation testing, scripts that create files) uses the disposable clone at
`$TSRS_WORK/project-clone`. Before you finish, run `git -C <pristine root> status --short`
(read-only) and confirm it prints nothing.

## The census free-gate (run it after any memory or layout change)

tsrs gives some arena memory back (free lists for dead mappers, inference contexts, type lists, flow labels;
parser rewinds; language-server regions; notes/mem-recycle.md, notes/lsp-mem.md). A freed block that something
still points to is a use-after-free. The alloc-profile build checks that at exit: frees are recorded, never reused,
and every freed block must be unreachable.

The census decodes 48-bit words as pointers, so it needs plain pointers (compressed handles are 32-bit offsets,
notes/mem-pointer-compression.md); a compressed build refuses `TSRS_CENSUS=1`.

```sh
CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile,tsrs_core/plain-ptrs
cd $PWD/target && TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1 TSRS_CENSUS_TOP=5 \
  ./prof/release/tsrs -p $PRIVATE_PROJECT/tsconfig.json --noEmit --checkers 1 2> census.err
grep -E "census verify \(recycling\)|strongly reachable freed blocks|violation class" census.err
```

Both numbers must be 0: the precise walk (`census verify (recycling): ... 0 to freed or rewound blocks`) and the
strong mark (`strongly reachable freed blocks (violations): 0`). Also run `--checkers 4` and `TSRS_LAZY_MEMBERS=0`.
A run takes 2-3 minutes and ~15 GB (more with 4 checkers): run one at a time. `TSRS_CENSUS_ASSERT=1` exits with
status 3 on a violation; `TSRS_CENSUS_CHAINS=N` prints N referrer chains (default 20); `TSRS_CENSUS_FRAMES=N` sets the
frames of the "<- caller" tables (default 2 for heap blocks, 3 for arena blocks). Without the census, the
alloc-profile build prints the free lists of each arena ("arena free lists": bytes on them at exit and at their peak,
reissued, and bumped by recycling sites that found their list empty).

Each `violation class` line names the freed block's site and the referrer's type and field offset. Look the offset
up in the referrer's current layout (`offset_of!`) before believing it: the strong mark reads every 4-byte step as a
48-bit pointer unless the type registered its padding, scalar, tagged and x8-encoded words with
`tsrs_core::census_layout` (`tsrs_checker::types::census_layouts`, `tsrs_ast::census_layouts`). **A layout change
must update those registrations** (they are computed with `offset_of!`, so reordering fields is covered, but a new
packed word, flag bits above an address or a new enum payload is not). Stale registrations both invent violations
(padding read as pointers) and hide real ones (pointer fields skipped); notes/census-regress.md is what happened
when they were hard-coded offsets. A violation that survives that check is a real bug: fix the escape tracking or
stop freeing that class.

## What each checker holds on the heap: the heap census

`TSRS_HEAP_CENSUS=1` with `--extendedDiagnostics` prints, per checker on stderr, every hash table and vector the
checker owns directly (cache maps, relation caches, link stores, lazy member tables, ...) with entries, capacity,
bytes per slot, load factor and heap bytes (`crates/tsrs_checker/src/heapcensus.rs`, notes/mem-checker-heap.md).
`TSRS_HEAP_CENSUS_MIN=<bytes>` sets the smallest row (default 1 MiB). It works in any build. Containers owned by
arena objects are attributed by the heap sampler of the alloc-profile build instead:

```sh
CARGO_TARGET_DIR=target/prof cargo build --release -p tsrs_cli --features alloc-profile
TSRS_HEAP_PROFILE=1 TSRS_HEAP_PROFILE_RATE=65536 TSRS_HEAP_PROFILE_TSV=/tmp/heap.tsv target/prof/release/tsrs -p . --noEmit --checkers 8
```

The TSV has one row per sampled stack and thread group (`main`, `checker-N`, `other`) with live, at-peak and
cumulative bytes. Symbols come from `atos` on macOS and `addr2line` on Linux.

## What the resident memory is: the memory split

`TSRS_MEM_SPLIT=1` (any build) prints on stderr, at `parse end`, at `check end` (every checker of the type-check pass
is done and its thread still alive, the moment memory peaks with many checkers) and at `exit`: the thread arenas'
capacity, used bytes and the resident bytes below each arena's finger that were never handed out (`mincore`); the
mimalloc heap's live blocks and the resident bytes of its pages (`mi_heap_visit_blocks`); the checker threads' resident
stacks; and on Linux `/proc/self/status` and `/proc/self/smaps` summed into the arena reservation, thread stacks, the
heap and other anonymous memory, and file-backed pages, with a one-line split `arena used + arena unused resident |
heap live + heap retained (in pages, outside pages) | stacks | file` (`crates/tsrs_core/src/memsplit.rs`,
notes/mem-linux-residency-32.md). `TSRS_MEM_SPLIT=purge` also has mimalloc purge its freed memory after `check end`
(`mi_collect(true)`) and prints the split again. `tools/perf/slackprobe.sh` runs it on the selected Depot runner
(`.depot/workflows/perf-probe.yml`, 32 vCPU by default) with interleaved timed reps.

## Where checker time goes by source pattern: the work census

A function profile cannot tell which type alias or call site the time belongs to. The work census
(`crates/tsrs_checker/src/workcensus.rs`, notes/perf-checker-algorithms.md) attributes the checker's work to
source-level identities: conditional types by root alias (with distribution fan-out and `never` results), mapped types
by declaration and key count, generic calls by callee (and repeats of the same signature + argument types), relations
by target symbol and relation kind (cache hit rates, pairs compared under several relations, how the constituent loop
for union targets exits), flow walks by function (steps per walk, sampled re-walks), union construction /
`removeSubtypes` / indexed access / `keyof` by size, export-assignment and initializer types.

It is compiled in only with `--features work-census` (the runtime test alone costs 0.3-0.5% instructions):

```sh
CARGO_TARGET_DIR=target/census cargo build --release -p tsrs_cli --features work-census
cd <project> && TSRS_WORK_CENSUS=/tmp/census.md target/census/release/tsrs -p . --noEmit --incremental false --extendedDiagnostics --checkers 1
```

The report (Markdown, written after `--extendedDiagnostics`) starts with per-category totals, then the top keys per
category. Read "key incl" as the time spent under the outermost span of that key (recursion counted once) and "self"
as the time minus every nested span; categories overlap, so shares do not add up. `TSRS_WORK_CENSUS_TOP=<n>` sets the
rows per table (default 60); `TSRS_WORK_CENSUS_SLOW=<ms>` prints every mapped-type resolution slower than that, with
the type. Use one checker: with several, each checker's spans are merged and wall time is shared. The census adds
about 18% instructions; its span cost is calibrated and subtracted, but patterns made of many short spans (unions,
relations, indexed access) still read a little high.

## The flow memo and its shadow mode

Control flow analysis keeps the result of a sub-walk (the type at a flow node for one reference) across walks
(`crates/tsrs_checker/src/flowmemo.rs`, notes/perf-flow-union-inference.md). It is on by default; it must never change
output. Environment switches, read once per process:

- `TSRS_FLOW_MEMO=0`: off. Every walk is Go's walk. Use it to tell whether a difference comes from the memo.
- `TSRS_FLOW_MEMO=shadow`: every memo answer is also walked for real, and the walked result is the one used; a
  different type, an incomplete result or a different full reference key (the stored key bytes against the walk's)
  panics with the reference, the flow node, and where the stored answer came from. Output equals `TSRS_FLOW_MEMO=0`.
  Run it over the suite (`TSRS_FLOW_MEMO=shadow tsrs-test run --suite all --baselines types,symbols --panic-summary`)
  and the corpora after any change to the checker that could affect what a flow walk reads.
- `TSRS_FLOW_MEMO_STATS=1` with `--extendedDiagnostics`: one line per checker on stderr: walks, consults, answers
  found / blocked (by an active loop analysis, the instantiation counters, another `flowTypeCache`, transient
  `sharedFlows` values) / used, height misses (answers not used near the depth limit), stores, frames not stored and
  why, walks redone after the depth limit (`aborts`), shadow checks.
- `TSRS_FLOW_MEMO_BITS=<8..24>`: log2 of the table's slots (default 12, 32 bytes each).

testdata/flow-memo holds the hazard cases (each with tsgo's output); `cargo test -p tsrs_cli --test flow_memo` runs
them with the memo on, off and in shadow mode.

## The union front cache and its shadow mode: `TSRS_UNION_CACHE`

`getUnionType` calls with two or more inputs and no origin first look in a small direct-mapped table per checker
(`crates/tsrs_checker/src/unioncache.rs`; 2^10 slots of 40 bytes, allocated on first use), keyed by the input handles
in input order, the reduction mode and the alias (symbol and type arguments), at most 8 words. A call is stored only if
it created no type other than the union it returns, instantiated nothing, took neither the template-literal nor the
constrained-type-variable reduction, and did not return `errorType` (the TS2590 path); the module comment has the
argument for why a hit then returns exactly what the uncached call would. On by default, off under
`--checkerAssignment go`. Diagnostics, `.types`/`.symbols` output and the `Types`/`Instantiations` counters are the
same either way.

| variable | values | effect |
| --- | --- | --- |
| `TSRS_UNION_CACHE` | unset (on; off under `--checkerAssignment go`), `0`/`off`, `1`/`on` (on in every mode), `shadow` | `shadow` also computes the uncached answer at every hit and panics if it is a different type (the panic names the input type ids, the reduction and both answers) |
| `TSRS_UNION_CACHE_STATS` | `1` | at exit, one line on stderr: lookups, hits, stores, why misses were not stored, calls that bypass the cache (shadow mode prints it too) |
| `TSRS_UNION_CACHE_BITS` | default `10` | log2 of the table size. Larger tables hit slightly more often (2^12: 0.03% fewer instructions) but cost up to 2 MiB of RSS per checker (an allocation above mimalloc's medium size) |

To audit a change near union construction, run the suite and a corpus with `TSRS_UNION_CACHE=shadow`: the result trees
and the diagnostics must equal a `TSRS_UNION_CACHE=0` run, with no panics (`tsrs-test run --panic-summary`). A shadow
failure means a stored call was not a function of its inputs: find which branch of `get_union_type_worker_inner` it
took and either keep it out of the cache (count it in `union_front_cache.impure`) or fix the state it read.

## The inference walk memo and its shadow mode: `TSRS_INFER_MEMO`

`inferTypes` looks up its walk in a per-checker map (`crates/tsrs_checker/src/infermemo.rs`) keyed by the source, the
target, the priority, the contravariance and what the walk reads from each inference info (type parameter, candidate
lists, priority, `topLevel`, `isFixed`, implied arity; at most 8 infos). The value is what the walk wrote: each info's
candidate lists, priority and `topLevel`, and whether it cleared the cached inferred types. A walk is stored only if it
took at least 16 `inferFromTypes` steps (and an earlier walk to the same target did too), created no type, symbol or signature, instantiated nothing, added no
diagnostic, took no impure union reduction, read no transient state (the flow memo's taint frame around it), did not
start inside an instantiation and, if it started with the instantiation count at 0, checked no expression; the
module comment has the argument. On by default, off under `--checkerAssignment go`. It pays where one generic call is
inferred many times from the same contextual type (vscode's `mapSessionEvents.test.ts`:
notes/perf-heavy-files-infer-memo.md).

| variable | values | effect |
| --- | --- | --- |
| `TSRS_INFER_MEMO` | unset (on; off under `--checkerAssignment go`), `0`/`off`, `1`/`on` (on in every mode), `shadow` | `shadow` also walks every hit from the same starting state and panics unless the walk again creates nothing but lazy member symbols, has no other effects and ends in the stored outcome (the panic names the type ids and both outcomes) |
| `TSRS_INFER_MEMO_STATS` | `1` | at exit, one line on stderr: lookups, hits, the `inferFromTypes` steps the hits skipped, stores, and why long walks were not stored (shadow mode prints it too) |
| `TSRS_INFER_MEMO_MIN_STEPS` | default `16` | the shortest walk (in `inferFromTypes` steps) that is stored; lower stores more walks (more memory, more lookups that pay) |

## Freeing checked leaf files: `TSRS_FREE_LEAVES`

A CLI `--noEmit` check with at most 16 checkers (`MAX_DEFAULT_CHECKERS`: above that it costs vscode 2.4-3.9% wall time,
so it is off unless `TSRS_FREE_LEAVES=1`; notes/mem-leaf-regions-cost.md) parses each TypeScript root file whose path predicts a leaf (tests, specs,
stories, mocks: `PREDICTED_LEAF_PATTERNS`) into a region of its own and frees the tree and binder output of each leaf
among them as soon as its diagnostics are collected (`crates/tsrs_compiler/src/fileregions.rs`,
notes/mem-free-leaf-files.md). A leaf is a checked TypeScript module that no other file refers to and that exports
only its own declarations; on vscode 2,289 of the 2,337 leaves are predicted (99.2% of the leaves' nodes). Other files
are parsed into the thread arenas as before. The text, the `SourceFile` and the diagnostics are kept, so the report is
the same. It is off with declaration emit (`declaration`, `composite`), `--explainFiles`, `--incremental`, `--build`,
`--checkerAssignment go`, in the language server, the API and the test harnesses, under the debug modes that walk
files or checker tables after the check (`TSRS_FILE_TIMES`, `TSRS_ASSIGNMENT_STATS`, the work and heap censuses,
`TSRS_CENSUS=1`), and in builds without compressed pointers (`plain-ptrs`, non-unix): there a freed region's memory
stays mapped and the heap its values owned is reused, so a stale read would see a live object, and nothing would be
saved. Asking a program for one freed leaf's diagnostics after the pass panics (the tree is gone); the CLI never does.

| variable | values | effect |
| --- | --- | --- |
| `TSRS_FREE_LEAVES` | a comma list of: unset (free with at most 16 checkers), `1` (free at any count), `0`, `keep`, `stats`, `all` | `0`: no file regions (the layout before). `keep` (any count): file regions and the leaf marks, nothing freed (what the regions alone cost). `all` (any count): every TypeScript root file gets a region, not only the predicted ones (frees every leaf, but moves every tree out of the huge-page thread arenas: +6-9% wall time on vscode at 32 checkers on Linux against +2.5-5% predicted). `stats`: one line on stderr: the leaves freed and the ones the prediction missed (with their share of the leaves' nodes), the bytes their regions used, all file regions' bytes, the pages given back and in how many system calls, and the arena address space the run used. Example: `TSRS_FREE_LEAVES=all,stats` |

After a change that reads a file after the check pass (a new report, a new whole-program loop in the checker, such as
`getAlternativeContainingModules`'s), run a corpus with `TSRS_ARENA_POISON=1`: a freed region is then filled with
`0xA5` and kept, so a read of a freed leaf crashes. The output must equal a `TSRS_FREE_LEAVES=0` run. `cargo test
-p tsrs_cli --test free_leaf_files` covers the checker's loop over every module
(testdata/regressions/leaf-alternative-containers) and a structurally identical instantiation after a freed leaf
(leaf-structural-instantiation).

## Declaration-file member lists parsed on first use: `TSRS_LAZY_DTS`

A CLI compile (not `--incremental`, `--build` or watch) in which no declaration file is type-checked (`skipLibCheck`
or `noCheck`) parses the member list of each interface, class and type literal of a declaration file, and keeps it
only as a record (position, parser context, binder state) when nothing in it reaches outside the list: the first
reader of the list's nodes or of the owner symbol's members parses and binds it again (`crates/tsrs_ast/src/lazylist.rs`,
`parse_member_list_lazily` in the parser, `bind_lazy_member_list` in the binder; notes/mem-lazy-dts-members.md). Before the checkers of a multi-checker pass start, the lists of the global libraries (default and `lib` files, type
reference directives and what they reference) and of interfaces merged into the global scope are forced in parallel,
so that the checkers do not wait for one another on them (`force_shared_lists`; `TSRS_LAZY_DTS_SHARED=0` skips the
global libraries). Output is the same; `--extendedDiagnostics` `Symbols` counts only the lists that were bound. Off in the language server, the
API and the test harnesses, and under `TSRS_CENSUS=1`, `TSRS_LAZY_DTS_CENSUS=1` and `TSRS_CHECK_SHARED=1`.

| variable | values | effect |
| --- | --- | --- |
| `TSRS_LAZY_DTS` | unset (on when it applies), `0`/`off`, `stats`, `force` (`tsrs-test` only) | `stats`: one line on stderr: lists made lazy, deferred by the binder, never reached by it, parsed again (and how many while their file was bound). `force` in `tsrs-test`: every declaration file's lists are lazy, checked files included, so the checker and the `.types` / `.symbols` walks force them all: the result trees must equal a run without it (`TSRS_LAZY_DTS_STATS_FILE=<file>` collects the workers' counts) |
| `TSRS_LAZY_DTS_CENSUS` | `1` (alloc-profile build) | the ceiling census: bytes of every member list of unchecked declaration files, and which lists and member symbols any reader asked for, by phase; `TSRS_LAZY_DTS_CENSUS_TSV=<file>` adds a per-file table |

After a change to what the binder does for members of interfaces, classes or type literals, or to what a member list
can contain, run the suite with `TSRS_LAZY_DTS=force` (also with `TS_TEST_PROGRAM_SINGLE_THREADED=false`) and a corpus
with `TSRS_ARENA_POISON=1` (rewound first parses are filled with `0xA5` and never reused); both must equal main.
testdata/regressions/lazy-dts-members holds the cases where a lazy list could differ.

## Checking a heavy declaration file on several checkers: `TSRS_SPLIT_FILES`

In the type-check pass with stealing (the default mode with more than one checker), a checked declaration file that
weighs at least 40% of an average checker's share is cut into contiguous statement ranges of about 1/8 of a share
(`crates/tsrs_compiler/src/splitcheck.rs`, notes/perf-next-heavy-files.md). The checker that owns the file checks the
first range; each other range is a queue item at the front of another checker's queue, which checks only those
statements (`Checker::check_source_file_piece`) and hands its diagnostics for the file to the owner. The owner adds
them to its collection, which deduplicates and sorts, and then runs the file-level steps as usual. A file is split only
if all its top-level statements are declarations (`TS1036` is reported on the first executable statement of a block
only, so two ranges would each report one). It is off with a named assignment (`--checkerAssignment`, `go`,
`random:<seed>`), single-threaded, and in every other pass (declaration diagnostics, emit, incremental, the language
server).

| variable | values | effect |
| --- | --- | --- |
| `TSRS_SPLIT_FILES` | a comma list of: unset (on), `0`/`off`, `shadow`, `force:<k>`, `stats`, `stats:<file>` | `shadow`: the owner also checks the other checkers' ranges itself, reports the file as one checker finds it, and panics if the split would have reported anything else (a diagnostic another checker found that it did not, or one it found in another checker's range that no other checker did). `force:<k>`: split every checked declaration file with at least two statements into up to k ranges, whatever its weight (the test mode). `stats`: one line per pass on stderr (files split, ranges, ranges run by another checker); `stats:<file>` appends it to a file (the test harnesses keep stderr) |
| `TSRS_SPLIT_MIN_SHARE` | percent, default `40` | which files are split (weight relative to an average checker's share) |
| `TSRS_SPLIT_PIECE_DIVISOR` | default `8` | range size: 1/n of an average checker's share |

After a change to what checking a declaration statement reads or writes outside the statement, run the conformance
suite in the default checker mode with forced splits and compare its lists with an unsplit run:
`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical TSRS_SPLIT_FILES=force:3,shadow tsrs-test run --suite all
--baselines types,symbols --panic-summary` against the same with `TSRS_SPLIT_FILES=0` (about 630 declaration files in
330 test programs are split). `cargo test -p tsrs_cli --test split_check` runs testdata/split-check in every mode
against tsgo-ref's output.

## Profiling

Profile the `dist` profile (fat LTO, one codegen unit; release builds also add PGO, `.github/workflows/release.yml`),
not `--release`. For reliable stacks on Linux x86-64 build it with frame pointers, in its own target directory (Apple
arm64 always keeps frame pointers):

```sh
RUSTFLAGS="-C force-frame-pointers=yes" CARGO_TARGET_DIR=target/profiling cargo build --profile dist -p tsrs_cli
samply record ./target/profiling/dist/tsrs -p <project> --noEmit --incremental false
```

For before/after numbers, compare instruction counts, not wall time: `python3 bench/count.py out.json -- <tsrs command>`
on Linux (single-threaded, with `--singleThreaded` and `RAYON_NUM_THREADS=1`, counts repeat to about 0.001%), or
`/usr/bin/time -l` on macOS. Per-function counts: `valgrind --tool=callgrind --callgrind-out-file=cg.out <tsrs command>`
on a `--release` build, then `callgrind_annotate --inclusive=no --tree=none cg.out` (`--tree=none`, or call-graph lines
are counted twice). The bench flags regressions on main by itself (`bench/README.md`, "Regression flag").

## How to fix

- The Go source (`ts-ref/tsc/internal/…`) is the specification. Find the Go function behind the wrong behavior, read it
  next to the Rust port (`// file.go:LINE` origin markers; `docs/sigs/checker.txt` maps names), and make the Rust do
  what the Go does. Most bugs are small porting slips: a dropped branch, a swapped argument, `intersects` vs
  `contains`, nil-vs-empty, an `unwrap()` where Go tolerates nil, evaluation order, a wrong flag constant.
- Never special-case a test, never paper over a symptom, never "improve" on Go. If Go has a quirk, port the quirk.
- Fix root causes in shared helpers rather than at one call site; then look for the same slip elsewhere
  (grep for the pattern) — sibling bugs are common because the same agent ported neighboring functions.
- When a signature changes, run `python3 tools/sigs-from-rust.py` to refresh `docs/sigs/checker.txt`.
- Keep a short log in `notes/fix-<agent-name>.md` (root causes found, patterns worth grepping for, things you could not
  resolve). It is committed with your work and read by the other agents.
- Do not stop at the first fix. Work through your cluster until it is done or what remains is blocked on something
  outside your scope — then say exactly what.
