# perf-round2-followups: what is left after the 2026-10-04 performance round

One list of the open ideas, decisions and loose ends from the round that landed #37, #40, #38, #41, #42, #49, #50,
#51 and #55. Each stream's own note has the detail and the measurements; this file is the index, ordered by expected
value. Numbers are from the 38k-file codebase on an 18-core Apple Silicon machine unless noted.

Where the round ended: peak memory -15% (4.25 GiB with one checker, 5.68 with four), warm no-edit run ~0.9 s, leaf
edit ~1.1 s, hub edit 12.4 s, cold incremental peak 5.94 GiB, emit 1.4-2.3x faster. The cold check itself is flat:
pointer compression costs about +4.9% instructions and took back most of the checker-CPU round's -6.9%
(notes/perf-round2.md).

## Decisions waiting on the owner

- **Default checker count** (#48, draft; notes/perf-checker-scaling.md). `clamp(cores/2, 4, 8)` gives -16% wall for
  +1.6 GiB on the 38k-file codebase (-34% on vscode) and makes `--extendedDiagnostics` counters machine-dependent.
  Recommendation: close it and pass `--checkers` where the hardware is known (8 on laptops, 4 or 1 on small boxes).
- **#36** (persisted front-end design, docs only): close, or merge as the record of why not to build it.
- **A release.** main is well ahead of 0.3.0; nothing from this round is published.

## Not verified yet

- **The 32 GiB reservation outside macOS and GitHub runners: verified.** main ran in a Linux x86-64 dev sandbox
  (gVisor-style microVM, `ulimit -v` unlimited, overcommit 1): check, emit and incremental all work. It still needs
  a 47-bit address space and no `ulimit -v` below 32 GiB.
- **x86-64 cost of compression: 0-1.5% wall, once both builds have huge pages** (notes/linux-x86-round.md). The
  earlier figures (`--release` +6.6% wall, `dist` +4.4% with four checkers and +2.3% with one, Intel Xeon 8259CL)
  compared a compressed build whose arena had silently lost its transparent huge pages (the 32 GiB reservation never
  asked for them; mimalloc, which backs `plain-ptrs` chunks, does) with a `plain-ptrs` build that had them. With the
  arena advised again (Xeon 8259CL, 5 rounds): wall +0.6% / +1.4% / -0.9% (1 / 4 / 8 checkers), cycles +1-2.6%,
  instructions still +6.5%, peak -15%; an Ice Lake host agreed within its noise. Shipping `plain-ptrs` on Linux would
  buy about 1% for 15% more memory.
- **Linux emit.** The 38k-file codebase is bound by file creation on macOS (4 writer permits). On Linux file creation
  runs in parallel, so the writer cap and the remaining transform CPU may both matter (notes/perf-emit.md).
- **CI on main: fine.** CI moved to Depot (`.depot/workflows/ci.yml`: `check-and-test`, `lint-ratchet`); results are
  check runs on the commit (`gh api repos/<repo>/commits/<sha>/check-runs`), not `gh run list`. It runs on pushes to
  main only, so a PR that adds a lint finding turns main red after the merge: run `tools/lint/ratchet.py` before merging.
- **`cargo test -p tsrs_cli` does not link on macOS**: `api::memory_tests` calls glibc `malloc_trim`. Every agent
  this round excluded it locally. Needs a `cfg(target_os = "linux")` from the Node API side.

## Ideas, by expected value

1. **The heavy type graphs every checker rebuilds: fixed in the checked codebase**, not in tsrs
   (notes/perf-checker-scaling.md has the measurement). Two causes, both worth knowing for any project:
   `export default new Ctor(...)` makes the checker check the whole constructor call, pulling in every argument's
   type transitively, where `const x = new Ctor(...); export default x` takes the type from the constructor (TS 7
   behaviour); and a mapped type that runs `Extract` over a large union once per key (294 x 590 conditional checks).
   Result there: wall -10% at four checkers, instantiations 76.9M -> 65.6M. `TSRS_FILE_TIMES` (per-file thread CPU,
   compare one checker against eight) is the tool to find such files. Left: a module built from ~200 repository
   getters (~0.3 s cold) that needs a per-service split.
2. **Emit memory over check-only: 0.53 GiB left** (notes/mem-emit-regions.md, "What remains"): raw source maps kept in
   `EmitResult` as Go does (0.15), the print backlog of up to 2,048 files (0.15; a bound of 256 saved 0.14 GiB but
   cost 8% emit time on vscode), declaration-diagnostics leftovers (0.1).
3. **Emit CPU on the checker thread** (notes/perf-emit.md): `get_local_module_specifier`,
   `get_nearest_ancestor_directory_with_package_json` (a memo, ~0.1 s), `get_accessible_symbol_chain_from_symbol_table`,
   each about 3% of emit. Only visible in wall time once writes are not the bound (Linux).
4. **Hub edits**: closed. See "Measured and rejected" and notes/perf-hub-edit-shortcut.md; the exact part
   (one batched emit for global-scope edits, 18.0 -> 10.5 s) landed as #65.
5. **Checker CPU**: round 3 landed (#66, notes/perf-checker-cpu3.md): -4.2% / -5.0% instructions (one / four
   checkers), check time -3.3% / -3.7%. Full name interning was not done: 43% of symbol-table hits already use the
   same string and now skip the comparison; the rest would need the parser to intern every identifier (+9% parse
   time, rejected earlier) for about 0.2%. Handle-to-reference conversions are ~1.2% of samples on arm64; on x86
   zero-based handles, re-measured with huge pages, remove 2.2% of the instructions for -0.4% / -3.2% cycles (one /
   four checkers, notes/linux-x86-round.md part 2). The x86 profile's own list (relation cache = 11% of L3 misses,
   node link stores, instantiation caches) is in that note.
6. **Smaller layouts that handles now allow** (notes/mem-pointer-compression.md, "Not done"): `Type` header 24 -> 20
   bytes (at most ~38 MB), `TypeMapper` below 16 bytes (needs a home for its kind and escape bits).
7. **The "nothing changed" fast path** (notes/perf-dev-loop.md): exact conditions written up; it still has to
   re-resolve every import and would change `--extendedDiagnostics` counters, so the 0.9 s no-edit run is close to
   its floor (program construction 0.6 s, of which `open()` is ~0.3 s).
8. **Parallel `affectedfileshandler` / `emitfileshandler`** (notes/perf-incremental-parallel.md): still sequential in
   the port; costs nothing under `--noEmit`. Worth doing with an emit-on incremental benchmark.

## Landed in round 3, union and inference work (notes/perf-union-inference.md)

- **Union front cache** (`TSRS_UNION_CACHE`, on by default and off under `--checkerAssignment go`; its shadow mode is
  in docs/DEBUGGING.md). It is a direct-mapped table in front of `getUnionType` for calls without an origin. It stores
  a call only if that call created nothing but the union it returns, instantiated nothing, took no state-dependent
  reduction and did not return `errorType`.
  - Instructions: -0.5% to -1.1% on four corpora, with 1, 4 and 8 checkers.
  - Check time with one checker: -1.1% to -3.3%.
  - No change in memory or `--extendedDiagnostics` counters.
  - Diagnostics byte-identical, and shadow mode clean on the suite, fourslash and four corpora.

## Measured and rejected (do not redo)

- A1, deciding `getConditionalType`'s definitely-false test for discriminated unions without the relater (draft #88,
  notes/perf-checker-algorithms.md): exact (it replays the relater's side effects; cross-checked on the suite and five
  corpora), but -0.5% instructions on one corpus at four checkers and neutral elsewhere, not worth a second
  implementation of part of the relater.
- A2, skipping the non-matching constituents of such a conditional through a key index: asymptotically better, but the
  skipped evaluations' instantiation counts depend on cache states, so it moves the instantiation-budget (TS2589)
  boundary that `testdata/regressions/conditional-instantiation-limit-*` pins.
- A memo for generic calls inferred again with the same inputs (notes/perf-union-inference.md, task B). The part
  such a memo could skip is 1.3% of one checker's check time on the 38k-file codebase, 2.2% on vscode and 0.3-0.4% on
  webpack and xstate, which fails the 1.5%-on-two-corpora bar. Those figures are upper bounds that ignore the memo's
  own cost. The census's ~8% for repeated inferences is mostly checking the argument expressions, which has to run
  at every call site.
- A larger union front cache (2^12 slots or more; notes/perf-union-inference.md): 0.03% fewer instructions, but about
  2 MiB of RSS per checker.
- A full unit-property index for relations to union targets: would skip at most 0.2% (big) / 0.1% (vscode) of failed
  constituent checks; the rest is inherent (notes/perf-checker-algorithms.md, "Row 1").

- Zero-based handles on Linux (reserve 4-32 GiB so a dereference needs no base; #62, notes/mem-pointer-compression.md
  section 6 on that branch): removes 2.7 of the 7 points of extra x86 instructions, but wall and cycles move by 0.7-2%,
  inside the host's drift, and it adds low-address-space failure modes. The rest of the cost is the 32-bit handle
  itself. Re-measured once the compressed arena had huge pages again (notes/linux-x86-round.md part 2): -2.2%
  instructions, wall -0.4% / -2.2% and cycles -0.4% / -3.2% with one / four checkers; still not worth the failure
  modes.
- `-C target-cpu=x86-64-v2` / `-v3` for the Linux x64 release (notes/linux-x86-round.md part 2): v3 retires 0.6% fewer
  instructions and takes 1-2% more cycles, v2 changes nothing. Nothing to ship.
- mimalloc purge delay (never, 10 s) and eager arena commit on Linux (same note): page faults -54..-77% but no
  consistent change in cycles; eager commit +3.7% cycles.
- Faster file reading for the front end (`io_uring`, `readahead`, fewer syscalls): not tried, the front end is 5.6-6.6%
  of a 4- or 8-checker run on Linux x86 (same note), below the bar where it could pay.

- Hub-edit shortcut, storing file versions as signatures when an edit re-checks most of the program (#64): hub edit
  10.7 -> 7.9 s, global `.d.ts` 18.0 -> 7.2 s, but the first later body-only edit of each skipped file re-checks the
  whole program (1.0 -> 7.6 s), and the stored signatures differ from tsgo's (notes/perf-hub-edit-shortcut.md).
- Deferred signatures, checking first and computing Go's signatures after the check: ~1 s on the hub edit
  (11.1 -> 10.0 s), and 7-11 of ~22k signatures differ from base because printed declarations depend on type
  creation order (tsgo itself is nondeterministic on those files). The global-scope part of it, batching the per-file
  signature emits, is exact and landed (global `.d.ts` edit 18.0 -> 10.5 s).
- Persisted, mmap-able front end: 26% of nodes cacheable, 0.02-0.03 s at 18 threads, at most 0.15 GiB
  (docs/PERSISTED_FRONTEND.md on #36).
- Scope regions for inference contexts: 25-50% of scopes keep a live block (notes/mem-scoped-arenas.md).
- Work stealing between checkers: landed after all (notes/perf-checker-stealing.md), once output stopped depending on
  the assignment (notes/perf-order-independence.md); only the counters vary between runs, and naming an assignment
  keeps them fixed. Partition changes: +-4% with no consistent winner (notes/perf-checker-scaling.md).
- Forked checker processes sharing one warm checker copy-on-write (notes/perf-checker-processes.md): 17-47% fewer
  instructions and 0.2-1.9 GiB less memory at 8-16 workers, but no faster than threads, static or with stealing; worse
  on small programs. Children's private state is mostly their own caches and copied hash-table pages.
- Sharing types across checkers: cannot be exact (notes/mem-shared-base.md).
- Directory listings instead of existence probes, `openat`, a typed tsbuildinfo decode, skip-if-identical
  tsbuildinfo writes (notes/perf-dev-loop.md, perf-dev-loop2.md).
- Pointer compression below +2% instructions on arm64; `PSlice`/`PStr` for memory (slices are already one word)
  (notes/mem-pointer-compression.md).
- A per-file `type_to_string` builder, lazy `ErrorSymbolName`, dropping source maps from `EmitResult`
  (notes/mem-emit-regions.md).

## The lint ratchet

`tools/lint/baseline.tsv` went from 1,373 findings to 312 (#57-#59, #61, #63; notes/lint-paydown-compiler.md,
notes/lint-paydown-project.md). Left: 195 in `tsrs_api*` (the Node API crates, not touched), ~112 in the
project / language-service crates that need a redesign rather than a cleanup (by-value handler arguments fixed by
fn-pointer types, one large JSON error type, 31 hash-iteration loops whose order is observable), and 5 compiler-side
findings blocked on callers in those crates. Measured on the way: none of the 43 `#[inline(always)]` were needed;
the unchecked string conversions and link-store indexing are (+1.6% to +5%, +0.6%); removing ~170 clones did not
change speed.

## Housekeeping

- `bench-cache/solutions/mui-docs` (a local bench checkout) holds ~13k stray emit outputs from a benchmark that used
  `--outDir` without `--rootDir`; mui-docs numbers are off until it is restored. The file list is in the perf-emit
  worktree's `scratch/mui-polluted-files.txt`. Emitting benchmarks must pass `--rootDir <repo root>` and run under a
  write-deny `sandbox-exec` profile (notes/perf-emit.md, "Reproducing").
- Finished agent worktrees under `wt/` hold ~80 GiB of build output and scratch; all their branches are merged.
