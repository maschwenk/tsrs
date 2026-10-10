# perf-round2-followups: what is left after the 2026-10-04 performance round

One list of the open ideas, decisions and loose ends from the round that landed #37, #40, #38, #41, #42, #49, #50,
#51 and #55. Each stream's own note has the detail and the measurements; this file is the index, ordered by expected
value. Numbers are from the 38k-file codebase on an 18-core Apple Silicon machine unless noted. The sections above
"Measured and rejected" were brought up to date on 2026-10-10; the list itself is the part to read before a new idea.

Where the round ended (2026-10-04): peak memory -15% (4.25 GiB with one checker, 5.68 with four), warm no-edit run
~0.9 s, leaf edit ~1.1 s, hub edit 12.4 s, cold incremental peak 5.94 GiB, emit 1.4-2.3x faster. The cold check itself is flat:
pointer compression costs about +4.9% instructions and took back most of the checker-CPU round's -6.9%
(notes/perf-round2.md).

Since then, by 2026-10-10: the default checker count scales with the cores, BOLT ships in the Linux release binaries,
the union front cache is on by default, the lazy declaration-file member lists are on whenever no declaration file
is type-checked, CI runs on pull requests, and the workspace is at 0.11.0. The items below say so where they apply.

## Decisions from the round (all settled by 2026-10-10)

- **Default checker count** (#48 at the time; notes/perf-checker-scaling.md). The round measured
  `clamp(cores/2, 4, 8)` at -16% wall for +1.6 GiB on the 38k-file codebase (-34% on vscode) and recommended `--checkers` instead. The opposite
  landed: `default_checker_count` (checkerpool.rs) scales with the cores, half the parallelism but every core up to a
  small-machine limit, at least Go's 4, capped by the number of type-checked files (4 on 4 cores, 8 on 8 and 16, 32 on
  64 or more). The `--extendedDiagnostics` Types / Symbols / Instantiations counters depend on the machine unless
  `--checkers` is given.
- **#36** (persisted front-end design): the design is on main as docs/PERSISTED_FRONTEND.md, the record of why not to
  build it.
- **A release**: done; releases have been cut since (the workspace is at 0.11.0).
- **Checker-count-dependent output on TanStack/router** (notes/open-history-dependence.md): left open. The README's
  capability table lists it as a known exception. The exact fix (branch `exp/canonical-base-constraints`) costs +1.5%
  instructions on router, +4.1% on sequelize, +0.4% on type-fest and reports an error tsgo does not on a ten-level
  indexed-access chain (see "Measured and rejected"). Not tried: redesigning `getResolvedBaseConstraint` to resolve
  each type from the top (the note's option 3).

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
  main and on pull requests into main, drafts included (ci.yml's `on:`), so a lint finding shows on the PR.
- **`cargo test -p tsrs_cli` does not link on macOS** (still so on 2026-10-10): `api::memory_tests` in
  tsrs_cli/src/api.rs declares glibc `malloc_trim` under `#[cfg(test)]` only. CI runs the tsrs_cli tests on Linux
  (#226). Needs a `cfg(target_os = "linux")` on the module.
- **`cargo test -p tsrs_api` does not link on macOS either** (still so on 2026-10-10):
  crates/tsrs_api/tests/memory_test.rs declares glibc `malloc_trim` with no cfg.
- **`pinned_node_clients_round_trip` failed on macOS on main** (crates/tsrs_api_transport/tests/node_roundtrip.rs,
  seen in the tsrs_api lint paydown's gates): it expects `/var/folders/...` and gets `/private/var/folders/...`
  (macOS `/var` is a symlink). Not re-checked since (Linux box).

- **`panic = "abort"` builds segfaulted: fixed** (notes/fix-arena-recycle-uaf.md). `recycle_mapper_with_targets`
  freed a type list it had received as a `&[P<Type>]` parameter, a protected borrow for the call, so LLVM could and
  (with `panic = "abort"`) did delete the free-list link write. Undefined behaviour that the unwind build survived by
  luck of code generation; no output was affected. Now a raw slice, with a source check, Miri on the arena tests and
  a `panic = "abort"` conformance job as guards.

## Ideas, by expected value

0. **BOLT for the Linux release binaries: landed** (notes/perf-build-level.md): -2.7% / -3.2% / -1.0% wall at 1 / 4 /
   8 checkers on the 38k-file codebase, -3.4% to -4.0% on vscode, identical output and gates. `release.yml` and
   `.depot/workflows/bench.yml` both run `.github/scripts/bolt.sh`, so the bench measures what ships.
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
9. **type-fest's tuple targets** (notes/perf-excalidraw-typefest.md): the element type parameters and index names
   are now shared by all targets of a checker (notes/perf-shared-tuple-elements.md: type-fest -29% peak at one
   checker, -31% at 8). The element symbols cannot be shared exactly (`create_union_or_intersection_property`
   compares target symbols) and are most of what is left against bun. The same note ranks lazy formatting of
   relation error arguments (excalidraw's remaining ~120 MiB) next.

## Round 3, union and inference work (#100, landed; notes/perf-union-inference.md)

- **Union front cache** (`TSRS_UNION_CACHE`, crates/tsrs_checker/src/unioncache.rs; on by default and off under
  Go-compatible history, `--checkerAssignment go`; its shadow mode is in docs/DEBUGGING.md). It is a direct-mapped table in front of `getUnionType` for calls without an origin. It stores
  a call only if that call created nothing but the union it returns, instantiated nothing, took no state-dependent
  reduction and did not return `errorType`.
  - Instructions: -0.5% to -1.1% on four corpora, with 1, 4 and 8 checkers.
  - Check time with one checker: -1.1% to -3.3%.
  - No change in memory or `--extendedDiagnostics` counters.
  - Diagnostics byte-identical, and shadow mode clean on the suite, fourslash and four corpora.

## Measured and rejected (do not redo)

- A tracing collector for checker data instead of retiring checkers (notes/mem-checker-gc.md): weak identity caches
  free 3% more than garbage at mid-run; even an upper bound with member tables and value-symbol links weak frees
  ~37% of the program's memory, about where `--maxMemory` already gets, for weeks of data-model changes.

- Three ideas from notes/mem-recycle-checkers.md (the 38k-file codebase, 8 checkers; `--maxMemory` itself
  landed, opt-in): mapping source files of at least 16 KiB instead of reading them (-3.4% footprint, RSS unchanged,
  +4% wall from page faults); retiring a checker only where its queue changes directory (no instruction change: the
  rebuild cost is the shared base); staggering the checkers' first retirements (-2.2% peak for +2.5% instructions).

- `mimalloc-safe 0.1.67` without its `v3` feature (notes/perf-mimalloc-safe.md): the crate defaults to mimalloc
  v2.5.2, unlike the old crate's v3.3.2 default. On macOS arm64 it adds about 4% peak RSS at the default checker
  count on both Compiler workloads; the prior large Linux measurement found v2 3-14% slower. The migration enables
  `v3` (v3.5.2): on pinned vscode it is within +0.03-0.15% instructions and -0.69% to +0.46% peak RSS of the old
  build; the smaller Compiler workloads agree.
- A type graph shared by the checker threads (frozen seed + forks; notes/spike-shared-graph.md, branch
  `spike/shared-graph`, PR 213 closed): exact in every run, but on the 16-vCPU Linux runner peak -5..-10% at the
  default 8 checkers for +6..+18% wall (the serial seed). Revisit only with a seed that costs no wall time.
- Checking a relation without reporting first and elaborating only on failure, as bun does
  (notes/perf-excalidraw-typefest.md): on excalidraw at 9 checkers, 501 -> 379 MiB after the deferred constraint
  check. Not exact against tsgo: the reporting run spends more of the relation complexity budget, so a
  non-reporting first pass can succeed where tsgo reports TS2859. The exact form is lazy formatting of error
  arguments.
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
- A per-checker Bloom filter in front of `merged_symbols` (notes/perf-merged-symbols-filter.md): 99% of the 32M
  `get_merged_symbol` calls on vscode miss, but the filter saves 0.13-0.62% of single-threaded instructions on the
  17 bench projects, and even a free "never merged" test would stay under 1% (about 0.8-0.9% on vscode and
  t3code-server). Branch `probe/merged-symbols-filter-inline` has the code.

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
- Build-level options on top of PGO + fat LTO (notes/perf-build-level.md, Linux x86-64, Ice Lake): BOLT `-hugify`
  (text on 2 MiB pages: no gain over BOLT alone), mimalloc v2 / jemalloc / glibc malloc instead of mimalloc v3 (3-14%
  slower for 3-7% less peak), `opt-level = "s"` for the cold crates (`.text` -6%, speed unchanged), vscode at eight
  checkers added to the PGO training (-3% instructions, cycles unchanged). `panic = "abort"` was not measured: its
  builds crashed until notes/fix-arena-recycle-uaf.md.
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
- Per-checker garbage, free-list high-water and hash-table slack (notes/mem-checker-scratch.md). Garbage is 3.3-5.5
  MB per extra checker, spread over about ten causes. The one exact fix of size (`export *` tables as values) is -1.9%
  peak at 32 checkers on Linux (formbricks-web, cal-diy). The free lists peak at 6 KB per checker. hashbrown tables
  are already minimal; a non-power-of-two table would save 1.7-2.2% of peak at 32.
- Directory listings instead of existence probes, `openat`, a typed tsbuildinfo decode, skip-if-identical
  tsbuildinfo writes (notes/perf-dev-loop.md, perf-dev-loop2.md).
- Pointer compression below +2% instructions on arm64; `PSlice`/`PStr` for memory (slices are already one word)
  (notes/mem-pointer-compression.md).
- A per-file `type_to_string` builder, lazy `ErrorSymbolName`, dropping source maps from `EmitResult`
  (notes/mem-emit-regions.md).
- Deferring never-read checker objects on the app projects (notes/mem-never-read-apps.md): 11-14% of each extra
  checker's allocation is never read, nearly all symbols, links and signatures (types are read), in many small pools.
  The one pool above 2% (t3code: lazy tables instantiating signatures only for `isWeakType`) was deferred exactly
  (D1, code in 8be2e48): -1.1% peak at 32 checkers on Linux, nothing elsewhere.
- Residency slack at 32 checkers on the 64-vCPU runner (notes/mem-linux-residency-32.md): the unused part of each
  thread arena's huge-page block was 57-71 MiB at the peak (2.1-2.7%, 5.8% on drizzle-orm) and is gone with #199 (a
  finished thread hands its arena to the next one, a finished checker trims its block: -1.4% to -2.1% peak on five
  projects, -6.4% on drizzle-orm, no wall change). Rejected: trimming the parse workers' blocks (the handoff saves the
  same by reuse), `mi_collect` on the parse workers or on finished checkers (0-1%: mimalloc's 4.7-8.2% of retained
  memory is free blocks in pages that still hold live blocks; a forced purge at the peak finds 4-5 MiB), smaller
  stacks (4-11 MiB resident in all), switching a running checker into a finished checker's arena.
- Lazily parsed and bound member lists of unchecked declaration files (notes/mem-lazy-dts-members.md): exact, landed
  (on when `skipLibCheck` or `noCheck` is set; `TSRS_LAZY_DTS=0` turns it off): -7.1% peak on formbricks-web at 32 checkers on Linux, -1.5% to -2.2% on
  cal-diy, supabase-studio, t3code-server, drizzle-orm and xstate-main, wall within +-1% on eight projects once the
  global libraries' lists are forced before the checkers start (without that, drizzle-orm's checkers waited on each
  other: +8% wall). Left: namespace bodies (+43 MB on formbricks-web, needs
  the binder's per-body state at parse time), lists with import types (30-60 MB never asked for on t3code-server and
  cal-diy), eager `@see`/`@link` JSDoc (4-17 MB), per-member laziness (about 1% of peak).
- Canonical base constraints for TanStack/router's checker-count-dependent TS2536 (notes/open-history-dependence.md):
  exact by recomputation (+0.4% to +4.1% single-threaded instructions on type-heavy projects, 1.4x-6x the base
  constraint computations, and an error tsgo does not print on a deep indexed-access chain); with per-result summaries
  to reuse more (6-12% fewer computations); and not caching cut results below the top (+1.2% to +2.3% on sequelize
  and cal-diy, still history-dependent).
- A more compact syntax tree for the 8-checker scoreboard losses (notes/mem-compact-ast-sizing.md; counts of the real
  trees on t3code-server, supabase-studio, mikro-orm, cal-diy, formbricks-web and vscode, nothing built): the whole
  tree alive at the peak is 6-21% of it. Identifier text as an atom in the node saves nothing (identifiers are already
  32 B; -0.4 to -2.5 MiB after the interner); name-less property-access, member and specifier names 0.5-0.7% of peak
  (vscode 2.1%); lists as inline ranges 0.3-0.6% (1.1%); the node id out of the header about 0 once ids stay dense;
  everything measured together, token nodes included, 1.7-3.6% (vscode 5.7%). Nothing flips the five application
  projects. Revisit only if the checkers' share of the peak shrinks until the tree is the gap.
- The read path of an immutable shared type layer, on main with nothing shared (R1, notes/design-shared-type-layer.md
  section 5.4, branch `spike/r1-read-path`): +2.6% single-threaded instructions on xstate-main and t3code-server
  (Mac), about 2 points of it the cells of the 40 lazy fields a fork would overlay (mostly the unset test on every
  read, not the window comparison); the rest alone is +0.41..+0.65% on Linux. Closes the shared layer for the default
  binary.

- A contract-preserving form of the one-active-group checker-thread fix (`run_work_group_for`, 600723a;
  notes/emit-checker-thread-fix.md): it was output-neutral and created fewer threads (1.3 K vs 4.9 K at 4 builders)
  but saved no memory on `TSRS_EMIT=1 tsrs -b .` over
  8 x 150-file composite projects (1,691 / 1,717 / 1,746 MB at 1 / 4 / 8 builders against 1,694 / 1,727 / 1,749 MB
  without a fix; the landed six-line hunk: 354 / 393 / 409 MB).
- Three items of the Bun study that its adversarial checks refuted (notes/bun-check-memory.md section 3, vscode,
  estimates): instantiation caches keyed by result arguments (0 MB; values do not determine keys), dropping the
  default-library text as Bun does (net 0-1 MB, medium risk; the zero-copy half landed instead), numeric literals in a
  table (0 MB).
- Frozen binder symbol tables with Bun's per-file entry ranges (PR 147, closed; notes/mem-flat-symbol-tables.md):
  -9.4 MB front end on vscode, -8.0 MB on formbricks-web, for +0.4% to +2.5% single-threaded instructions and ~340
  lines. Untried: a wider header filter, a bulk freeze without per-table headers.
- Flat flow-label edges and one shared Start per file for bodyless signatures (reverted in #191;
  notes/mem-flow-compaction.md): -5.4 MB arena on vscode (0.2% of peak), for 377 lines of binder representation.
- A global identifier interner (notes/mem-round2.md, step 9): -0.075 GiB single-threaded on the private monorepo, but
  parse time +9% (2.6 -> 2.8 s) and +1% instructions from hashing and locking 7.8M identifiers; `PackedStr` took -0.10
  GiB with no table.
- Mapper interning (identity is observable: `find_active_mapper`, `compare_type_mappers`) and type-list interning
  (10.0M lists, 5.35M distinct: a hash-consing table costs more than the 80 MB it saves) (notes/mem-round2.md).
- Deferring the type of two-constituent union/intersection properties (B1, notes/mem-round3.md): -3.7% types but only
  6.289 -> 6.275 GiB, and it changed results (a TS2578 on the private monorepo).
- Lazy member tables in the arena instead of `Rc` (notes/mem-round3.md): peak +0.01 / +0.03 GiB (the `Rc` blocks fit
  a 176-byte size class exactly); packed inside the `Rc`, -0.01 / -0.02 GiB, not worth the churn.
- U1 / U2, discriminant matching and base resolution without instantiating lazy members (notes/mem-use-census.md, PR
  #7 closed): exact, -0.5% / -0.6% peak at 1 / 4 checkers on the private monorepo; 0.1-0.4% of the per-checker growth
  on the app projects (notes/mem-never-read-apps.md).
- Rolling back the speculative work of overload resolution (notes/mem-overload-rollback.md): 175-201 MB upper bound
  for the argument checks in `isSignatureApplicable`, below the 300 MB threshold; all of `chooseOverload` 351-409 MB at
  exit only; any of it needs a store barrier on every checker write.
- Bun-style dense link tables over lazily committed pages, per-kind node numbering, symbol-id or type-id groups for
  the hashed link stores (notes/mem-dense-link-tables.md): pages cost 23-632 MB more than the landed 32/128-id groups
  at 4-32 checkers on vscode; node numbering saves at most 4-5 MB at one checker, for a parser and AST change.
- Sparse id pages by default (notes/mem-64.md, notes/mem-checker-heap.md): -0.15 GiB at 64 checkers for +2%
  instructions; the 128-id groups save the same at 64 and more at 4-16 for no instructions, and the sparse form is
  removed.
- A symbol-table position index in 8/16 bits (change 6 of notes/mem-checker-heap.md): -1.4% to -1.5% peak for +0.39%
  instructions, above the 0.3% bar.
- Pre-faulting whole arena chunks on Linux (notes/linux-perf.md): peak RSS +1.2-2.2 GiB (+16% to +36%) and no wall
  gain (the faults it removes were ~25 K 2 MiB faults); with THP off it removes two thirds of the faults, but sys time
  still goes up.
- A thin generic shim over `&mut dyn FnMut` bodies for the port's `impl FnMut` callbacks
  (notes/monomorphization-audit.md): the port's own closures are 3.6% of tsrs_checker's LLVM IR and the largest
  generic tsrs function 1.1%; nothing to gain.

- Relating derived generics to their generic base by variances (`TSRS_DERIVED_VARIANCE`, #96, removed in #194;
  notes/perf-derived-variance.md, notes/fuzz-derived-variance.md): with guards 1-3, -8.4% / -16.8% instructions at 1 /
  8 checkers on the 38k-file codebase, but not exact: the fuzzer found disagreements in 3,670 of 8,000 programs in
  ordinary shapes (`keyof T`, conditionals on `this`, `T & {...}`). Guards 4-6 make it exact (471,465 decisions, 0
  disagreements) and leave about 1% of instantiations and -1.8% / -0.3% check time at 1 / 8 checkers (noise).
  `testdata/regressions/derived-variance-*` stay.
- A variance table shared by the checkers of a program (notes/perf-heavy-files.md, branch
  `perf/heavy-files-variance-share`): -33% checker CPU on mui-docs at 16 checkers, -8% on formbricks-web, -6% on
  cal-diy, but not exact: TypeScript's unreliable / unmeasurable marks depend on what the checker related before the
  measurement, so variances differ between checkers (cal-diy at 4 checkers printed 40 more lines).
- Go's pdqsort (`goslices::sort_func`) for every symbol sort, not only where `compareSymbols` is not total
  (notes/lsp-memfix.md): +1.4% instructions on webpack (16.56 vs 16.33 G).
- Flow memo variants (notes/perf-flow-union-inference.md): no checkpoints (webpack -4.4% instead of -7.0%, xstate
  +0.50% instead of +0.13%), table sizes 2^10 to 2^16 (no measurable difference), the per-node step out of line
  (+0.17% on xstate).
- Checker CPU micro-changes (notes/perf-checker-cpu2.md, notes/perf-checker-cpu3.md): a direct-mapped cache in front
  of `lazy_member_tables` (94% hits, no instruction change), inline fast paths III (-0.08%).
- `ReferenceInstantiations` hashing argument handles instead of type ids (notes/perf-checker-cpu3.md: paired median
  -0.07%, no change in instructions or cycles; notes/perf-memory-traffic-32.md A1: within noise at 16 and 32
  checkers).

- Static cost models for the checker assignment (notes/perf-balance.md): per-file cost per weight unit differs 40x
  between kinds of files; import-closure weights are worse on all three projects (mui-docs CPU imbalance 36% -> 77-98%);
  dropping the fanout term helps vscode and mui-docs but slows mui-docs' slowest checker. In-checker order is not free
  either: reversing it changed a printed TS2345 message.
- A per-file check-cost model fitted from syntax (notes/speed-frontend.md): in-sample within 0.5 points, but used for
  the assignment it made the slowest checker 21.5% above the mean (check 8.2 -> 9.5 s), because the assignment moves
  the first-touch costs.
- Longest-processing-time-first queues and thieves taking from the front (notes/perf-checker-64.md): LPT +3.6% CPU at
  16 checkers and 5-8% more CPU per checker with the static assignment (program order keeps caches warm), README bench
  -2%; front-stealing +4-6% CPU at every count.
- Memory-traffic work at 32 checkers (notes/perf-memory-traffic-32.md): IPC is equal or higher at 32 checkers than at
  1 (vscode 1.92 -> 1.96) and miss rates are low, so padding, pinning, prefetching and hot/cold splits have nothing to
  win; `ReferenceInstantiations` slot hashes (A2) were within noise (+11 MiB peak).
- Overlapping checker creation with the file assignment (#162) and computing the program diagnostics on a helper
  beside checker creation (#164) (notes/perf-serial-steps.md): 3-4 ms of serial time each at 32 checkers on the
  64-vCPU runner (Diagnostics: global (first) 15 -> 11 ms; program + global 18 -> 15 ms), reverted in #190 because a
  new thread or overlap needs more than that. `common_source_directory`'s file list on the worker pool: "verify
  options" stayed at 2 ms in 22 runs.
- Largest files first in the parallel parse (notes/perf-frontend-64.md): 0.06 -> 0.24 s at 64 threads (rayon put the
  150 largest files in one leaf); neutral with a leaf cap; the phase is not tail-bound.
- A 64-shard map for the resolver's module-resolution cache (notes/speed-frontend.md): the parse + resolve phase is
  bound by file-system calls; 0.51 s either way on the private monorepo.
- Releasing memory before exit and a smaller rayon pool (notes/perf-front-end-fixed-costs.md): `munmap` of the arena
  3.0 ms with the teardown unchanged, `madvise` from 16 threads 4.2 ms; 8-, 32- and 64-thread pools changed wall within
  noise.
- mimalloc options on macOS (`PURGE_DELAY=-1`, `ARENA_EAGER_COMMIT=1`, `ARENA_RESERVE=4GiB`, `ALLOW_LARGE_OS_PAGES=1`;
  notes/speed-frontend.md): within noise, peak within 0.05 GB.
- Unlimited concurrent emit writes (notes/perf-emit.md): emit on the 38k-file codebase 6.4 / 4.3 / 4.5 / 7.0 s with 1
  / 2 / 4 / unlimited writers; the cap is 4.
- Lower edge cut in the locality assignment (notes/mem-assignment.md): label-propagation refinement halves the cut
  (0.36 -> 0.22) and gains nothing in peak (15.97 -> 16.02 GB, 4 checkers, private monorepo); keeping directory
  subtrees together is what helps.
- A resident daemon / watch mode for the CLI (notes/perf-round3.md, not attempted): it holds several GiB per checkout,
  while the cold-process incremental path is ~1 s. Also reasoned out there without measurement: per-file summaries as
  declaration text (declaration emit is not total, import cycles, not identity-preserving), per-file check regions
  (types made in a body can enter long-lived caches), mmap of source files (SIGBUS on truncation).

## The lint ratchet

`tools/lint/baseline.tsv` went from 1,373 findings to 312 (#57-#59, #61, #63; notes/lint-paydown-compiler.md) and is
now 3 (status 2026-10-10: 2 `clippy::needless_pass_by_value` in tsrs_modulespecifiers/src/specifiers.rs, 1 `dead_code`
in tsrs_parser/src/parser_1.rs). Measured on the way: none of the 43 `#[inline(always)]` were needed; the unchecked
string conversions and link-store indexing are (+1.6% to +5%, +0.6%); removing ~170 clones did not change speed.

## Housekeeping

- Emitting benchmarks must pass `--rootDir <repo root>` and run under a write-deny `sandbox-exec` profile
  (notes/perf-emit.md, "How to reproduce"): a run with `--outDir` and no `--rootDir` once wrote ~13k stray outputs into
  a local bench checkout.
