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
(`--checkerAssignment locality`, notes/mem-assignment.md); Go uses FENNEL over single files. Per-file diagnostics
do not depend on it, but multi-checker symbol/type/instantiation counters and peak memory do: compare them
against `tsgo-ref` with `--checkerAssignment go` (or `TSRS_CHECKER_ASSIGNMENT=go`). Single-threaded runs are
unaffected. `--checkerCostCache <file>` (opt-in, locality only) balances the checkers on the previous run's
per-file CPU times (notes/perf-balance.md); `TSRS_ASSIGNMENT_STATS=times` prints per-checker wall and CPU seconds.
Inside a checker, files must stay in program order: reordering them can change printed types in messages.

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
status 3 on a violation; `TSRS_CENSUS_CHAINS=N` prints N referrer chains (default 20).

Each `violation class` line names the freed block's site and the referrer's type and field offset. Look the offset
up in the referrer's current layout (`offset_of!`) before believing it: the strong mark reads every 4-byte step as a
48-bit pointer unless the type registered its padding, scalar, tagged and x8-encoded words with
`tsrs_core::census_layout` (`tsrs_checker::types::census_layouts`, `tsrs_ast::census_layouts`). **A layout change
must update those registrations** (they are computed with `offset_of!`, so reordering fields is covered, but a new
packed word, flag bits above an address or a new enum payload is not). Stale registrations both invent violations
(padding read as pointers) and hide real ones (pointer fields skipped); notes/census-regress.md is what happened
when they were hard-coded offsets. A violation that survives that check is a real bug: fix the escape tracking or
stop freeing that class.

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
