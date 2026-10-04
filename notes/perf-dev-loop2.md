# perf-dev-loop2: hub edits, the cold incremental peak, the tsbuildinfo write

Follow-up to `perf-dev-loop` (PR #41). Three questions on the 38k-file codebase, with the usual rules: identical
diagnostics, tsbuildinfo bytes and `--extendedDiagnostics` counters.

1. A hub edit cost 25.7 s against 8.6 s cold (tsgo: 97 s against 25 s). Where does it go, what can be cut
   exactly, and what does Go's algorithm make unavoidable?
2. A cold incremental run (no tsbuildinfo yet, i.e. every CI run) peaked 1.2 GiB above `--incremental false`.
3. The tsbuildinfo write was 0.32 s of a 1.3 s leaf-edit run.

Method as in `perf-dev-loop` (scenarios, restoring edited files and checking them with `cmp`, interleaved medians
of 3, samply with the JSON reader, temporary counters that are not committed). The hub edit appends
`export type __TsrsHubEditN = number;` to `src/orm/db.ts` (1,870 direct importers). Base = origin/main 21b95e9
(includes #38 pointer compression and #40); new = this branch.

## 1. Hub edit

### Accounting (base)

Total 19.4-26.7 s depending on load (base, median 19.4 s in the final round):

| piece | s | what it is |
| --- | --- | --- |
| config, tsbuildinfo read (overlapped), program, changes compute | 0.9 | as in a no-edit run |
| **declaration signatures** ("Emit time" minus the write) | 12.5-18.7 | `getFilesAffectedBy`: the d.ts of every file reached through files whose signature changed |
| **semantic check** ("Check time") | 5.6-6.8 | the diagnostics of every file whose cached diagnostics were removed |
| tsbuildinfo write | 0.3 | from snapshot, marshal, write |

Counts (temporary counters): signatures are computed for **22,405 files** over 13 levels (1,870, 1,446, 4,398,
6,804, 3,292, 2,332, 1,542, 464, 103, 32, 13, 8, 2), each **once** (the `updatedSignatures` cache), on the 4
checkers, one emit per level (since #41). The checkers are created once (`Checkers: create` 0.003 s) and the program
is bound once. **All 37,863 non-lib files are re-checked**: `handleDtsMayChangeOfAffectedFile` on the changed file
(whose own signature changed) removes the cached diagnostics of the transitive closure of the files that reference
its referencers, which on this codebase is every file; 37,863 files also get a version-as-signature update.

Where the signature time went (samply, base, 39 CPU-s across the level emits): the declaration transformer's type
serialization (32.5 s inclusive), and in it symbol accessibility (23.4 s), and in that
`getAliasForSymbolInContainer` (19.8 s): **97.3 M calls**, each scanning the container's exports (4.3 entries on
average, 415 M `getSymbolIfSameReference` comparisons, a `Vec` allocation per call). Callers:
`getAlternativeContainingModules` asks every external module of the program (38k files) whether it exports the
symbol whenever no import of the enclosing file does; its per-symbol cache misses for every instantiated (fresh)
symbol, so this 38k-module loop ran ~2,500 times.

### Changes

1. **`getAliasForSymbolInContainer` from an index** (`printer.rs`). The loop over the export table is answered from
   an index of the table by merged resolved target, built the first time in the loop's own resolution order (first
   entry, then the symbol, then the rest). A memo by (container, symbol) was tried first and dropped: the pairs are
   almost all distinct (fresh symbols), and it took +6 GiB. Bypassed (the loop runs) while an alias resolution or a
   module export-table computation is in progress, and until `initializeChecker` has merged the global tables
   (`alias_cache_blockers`); keyed by table identity and size, so a provisional table replaced later is never
   reused. Emit 19.0 -> 10.8 s.
2. **`getAlternativeContainingModules`' all-modules loop from a reverse index** (`printer.rs`). A module whose answer
   can no longer have side effects (final export table cached with no export resolution in progress, indexed, its
   `export=` entry resolved) is "summarized": the answer for it is "`symbol`'s parent is this module, or an entry
   resolves to `symbol`'s target", and its targets go into a reverse index. Modules not summarized yet are asked in
   program order exactly as before (which summarizes them); the rest are answered from the index, and the result
   is in program order. Only used when `symbol`'s own target is already resolved. Emit 10.8 -> 7.4 s.

Final round: hub total **19.4 -> 12.4 s** (emit 12.8 -> 6.0 s); tsbuildinfo and counters identical to base in
3/3 runs. What is left in the signature emit (14 CPU-s on 4 checkers in 13 dependent levels, ~55% parallel
efficiency): type serialization proper, the checking it needs (inferred initializer types), module specifiers for
`import("...")` types (4.3 CPU-s, already cached per module and file).

### Why a hub edit is still slower than cold, and the shortcut Go does not take

Two pieces of Go-mandated work remain, and together they exceed a cold run:

- **The declaration signatures of the 22,405 visited files.** They are stored in the tsbuildinfo
  (`fileInfos[].signature`); tsgo writes the same values, so they cannot be skipped or approximated without changing
  the tsbuildinfo bytes. They need most of these files' declarations type-checked and serialized: ~14 CPU-s, ~5.6 s
  wall (levels are barriers; a checker that finishes a level waits).
- **Re-checking all 37,863 files**, because the removal closure covers everything: about a cold run's check (5.5 s
  here vs 6.0 s cold; slightly less because the signature emit warmed the checkers' caches).

So hub ≈ cold check + signature emit + 0.9 s. The shortcut (written up, not implemented): without `isolatedModules`
and `assumeChangesOnlyAffectDirectDependencies` (which take other branches), the set of files whose diagnostics are
removed does **not** depend on the propagated signatures. It is the referenced-by closure of the
changed files whose own signature changed (one d.ts emit per changed file), plus the visited files, which are inside
that closure. The propagated signatures only decide (a) what is stored as those files' signatures and (b), with
declaration emit on, which files get a pending d.ts emit (`--noEmit` here: none). Proposal: when the closure is all
files (or above N% of them), skip the propagation emits and store each visited file's version as its signature,
which is what Go itself does for every other file in the closure (`updateShapeSignature(file, true)`).

- Reported diagnostics: identical. Every file in the closure is checked either way, against the same program.
- tsbuildinfo: differs from Go's in those files' `signature` (version instead of the d.ts hash).
- Later runs: still correct. A version-as-signature is conservative: the next change to such a file's text changes its
  version, which is then treated as a signature change, so its dependents are re-checked. Go makes the same trade
  for files it does not visit. The cost is possibly more work on the next edit of one of those files (its
  dependents are re-checked even if its declarations did not change).
- Expected hub time: about cold + 0.9 s (one d.ts emit for the changed file, then a full check). With
  `--declaration`/`composite` and emit on, the pending-emit list would be computed from the closure instead (more d.ts
  files re-emitted than Go), so the shortcut belongs to `--noEmit` (or no declaration emit) only.

## 2. Cold incremental peak

Where (experiments that drop one piece at a time, same run otherwise, peak RSS): base cold incremental 6.95 GiB,
`--incremental false` 5.81. Without the reference map: 5.84 (the sets themselves and their share of the
tsbuildinfo). Without the marshal: 6.29. RSS over time: +0.4 GiB right after program construction (the change
computation builds the sets) and +0.74 GiB in the last 0.2 s (the tsbuildinfo write) on top of the checkers' peak.

1. **Reference sets keep the ambient-module part once per checker** (`referencemap.rs`, `programtosnapshot.rs`).
   Every file's set includes the ~390 files declaring its checker's ambient modules (~15 M `Path` entries, 16 bytes
   each plus hash-table slack). A set built from the program (`RefSet`) is now one shared set per checker plus the
   file's own paths (disjoint from the shared part; the file itself is left out of the shared part, as Go filters
   it, but stays in its own part when a triple-slash reference names it). Sets read from a tsbuildinfo stay flat.
   `RefSet` offers set operations only (size, membership, unordered iteration, set equality), which is all the
   reference map's users need. 6.95 -> 6.57 GiB.
2. **The tsbuildinfo is written without a `json::Value` tree** (`buildinfo.rs`). `BuildInfo::marshal` streams
   `marshal_json`'s document: same fields, order and omission rules; list elements other than the id lists still go
   through their `marshal_json`, one at a time. A unit test checks `marshal() == json::marshal(marshal_json())` on a
   build info with every field set, with empty file infos and with nothing. The output buffer is sized up front.
   6.57 -> **5.96 GiB** (non-incremental 5.80; target "within ~0.2 GiB" met), marshal 0.12 -> 0.06 s.

The sorted key vectors and signature strings were not significant (the two experiments above account for the whole
difference within 0.03 GiB).

## 3. The tsbuildinfo write on a leaf edit

Go writes the tsbuildinfo whenever `buildInfoEmitPending` is set and never compares with what is on disk
(`emitBuildInfo`, program.go:304). A leaf edit changes the edited file's version, so the content always differs and
skip-if-identical would never apply; a no-edit run already writes nothing (`buildInfoEmitPending` stays false). So
the write was made cheaper instead:

- the streaming marshal above (0.12 -> 0.06 s);
- `setFileInfoAndEmitSignatures` names each program file relative to the tsbuildinfo one at a time; the names are
  a pure function of the path and are now computed on the worker pool up front, ids handed out in the same order
  (from snapshot 0.14 -> 0.10 s).

Emit time on a leaf edit 0.31 -> 0.21 s; the leaf run 1.26 -> 1.14 s.

## Results

Base = origin/main 21b95e9, new = this branch, tsgo-ref; 4 checkers; seconds, peak RSS GiB; interleaved, median of 3
(range of the total); load 8-12.

| scenario | | base | new | tsgo |
| --- | --- | --- | --- | --- |
| hub edit | **total** | 19.42 (18.09-20.53) | **12.42** (12.23-13.30) | 81.1 (74.9-81.7) |
| | emit (signatures + write) | 12.79 | 5.96 | 62.8 |
| | check | 5.56 | 5.50 | 15.2 |
| | peak RSS | 7.67 | 7.17 | 27.06 |
| cold incremental (no tsbuildinfo) | **total** | 7.26 (7.23-7.47) | 7.13 (7.04-7.62) | 21.8 (21.5-23.1) |
| | **peak RSS** | **6.90** | **5.96** | 26.13 |
| | emit (tsbuildinfo write) | 0.31 | 0.21 | 1.73 |
| cold, `--incremental false` | total | 6.86 (6.68-6.88) | 6.80 (6.75-6.83) | 18.99 (18.81-19.16) |
| | peak RSS | 5.77 | 5.80 | 23.66 |
| leaf edit | **total** | 1.26 (1.21-1.33) | **1.14** (1.10-1.14) | 8.18 |
| | emit (tsbuildinfo write) | 0.31 | 0.21 | 2.01 |
| | peak RSS | 2.86 | 2.37 | 7.60 |
| no edit, rewrites tsbuildinfo | total | 1.32 (1.31-1.37) | 1.15 (1.07-1.18) | 4.66 |
| | peak RSS | 2.88 | 2.37 | 6.60 |
| no edit, steady | total | 0.97 | 0.98 | 3.04 |
| | peak RSS | 2.34 | 2.34 | 5.94 |

tsbuildinfo after every rewrite, leaf, hub and cold incremental run: base and new byte-identical (12/12, `cmp`);
`Files/Lines/Identifiers/Symbols/Types/Instantiations` identical in all of them and in the cold runs. The peak RSS of
the warm runs drops by 0.5 GiB with the shared ambient-module part of the reference sets.

## Gates

- Conformance `--baselines types,symbols`, default / `TSRS_LAZY_MEMBERS=0` / `TS_TEST_PROGRAM_SINGLE_THREADED=false`:
  result trees identical to base; 13,458 pass + 2 codes + 2 fail, `.types` / `.symbols` 12,779 / 12,779.
- `--baselines js,jsmap,sourcemap`: identical trees; `.js` 13,392 pass / 0 fail.
- Fourslash: 4,066 pass / 63 fail / 417 skip, identical lists.
- `cargo test --release -p tsrs_cli` green (`api::memory_tests` gated to Linux locally, not committed);
  `tsrs_incremental`'s new marshal test green. tsctests (fresh dump): 374 / 32 / 1, same lists as base; the one
  differing actual is `internal-symbolname-in-tsbuildInfo` (an unsanitized symbol id, as in #37 and #41).
- Incremental fixtures (`run-all.sh`, `cycle`, `graph`): same as base and #41.
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked`: clean.

## Reproduce

As in `perf-dev-loop`; the hub edit is
`printf '\nexport type __TsrsHubEditN = number;\n' >> src/orm/db.ts` (restore from a copy afterwards). The cold
incremental run is a run whose `--tsBuildInfoFile` does not exist yet. Peak RSS: `/usr/bin/time -l`, or sampling `ps`
every 50 ms for the timeline.
