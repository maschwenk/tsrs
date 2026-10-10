# Instruction-count hill climb on the 38k-file codebase (8 checkers)

Goal (owner, 2026-10-09): make the total user-space instructions of a cold `--noEmit` check of the 38k-file codebase
at `--checkers 8` significantly smaller, one measured change at a time. 39 commits on top of 3f8d6faa; every one
kept only if the count went down and the diagnostics stayed identical.

## Result

Clean `--release` builds, 3f8d6faa (main) against this branch, `-p tsconfig.typecheck.json --noEmit --incremental
false --pretty false --checkers 8`, Linux x86-64, 8 cores. Diagnostics identical in every row.

| | main | branch | change |
|---|---|---|---|
| instructions, default mode (work stealing) | 2.3913T | 1.4655T | -38.7% |
| instructions, `--checkerAssignment locality` | 2.3786T | 1.4457T | -39.2% |
| wall, native, 3 runs each | 42.44-42.50 s | 31.78-32.49 s | -24% |
| user CPU | 318.0-318.5 s | 232.8-233.9 s | -27% |
| peak RSS | 17.46-17.50 GB | 16.85-16.96 GB | -3.3% |
| `--maxMemory 6G`: wall / peak RSS (one run) | 95.4 s / 7.54 GB | 75.2 s / 7.68 GB | -21% / +1.8% |

Conformance: identical pass lists to main for errors, `.types` and `.symbols`, in check history and in default mode
(`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`), and for the error baselines with
`TSRS_LAZY_MEMBERS=0`. `tools/regressions.sh`: 29/29. Not run: `bench/run.py` (the public projects) and a
single-threaded count, so the 1%-of-`bench/count.py` bar is unmeasured; the evidence here is the 38k-file codebase.

## How it was measured

There is no `perf_event_open` in the sandbox this ran in, so `bench/count.py` cannot open a counter. Counts come from a
small DynamoRIO client that counts executed basic-block instructions per thread (user space only; run with
`-no_enable_reset`, or DynamoRIO's cache resets add noise). The default mode varies by about 0.3% run to run (stealing
changes which checker does what); `--checkerAssignment locality` is deterministic to about 0.02%, so every A/B used it,
with the default mode checked at the end. Per-commit numbers in the commit messages were measured before the rebase onto
3f8d6faa, on builds with frame pointers: default-mode counts for the first eight commits, static-assignment counts
from "inline fast path of getNormalizedType" on (absolute values differ from the table).

Where the instructions went: the same client dumping per-block counts, attributed to functions and source lines with
`nm`/`addr2line`, and exact direct-call counts per call site from a run with `-max_elide_jmp 0 -max_elide_call 0`
(a block ends at each call, so the call runs as often as its block). An `LD_PRELOAD` frame-pointer sampler gave
inclusive profiles.

## What landed (largest first; full list in the commit log)

- Unions: named-union membership through one pointer set (-11.3%); long keys in the union front cache (-7.0%); fewer
  `compare_types` searches under a total order (-3.2%); galloping merge of a short sorted run into a long one.
- Checker assignment: module affinity and sticky thieves from 8 checkers when at least 24,000 files are checked
  (-3.3%, wall -4%, peak RSS -3.6%; smaller projects keep the 16-checker limit).
- Memo bits in spare `ObjectFlags`: `shouldNormalizeIntersection` (-1.9%) and `isWeakType` for object types with
  resolved members (-1.4%). Both are lazily computed families: read with `object_flags_lazy()` so a fork of the
  shared-graph seed sees its own bits; every reset of a type's members clears the `isWeakType` bits with them.
- Memoized indexed accesses through a union index without an access node (-1.3%, under the union front cache's store
  rule; `TSRS_IA_MEMO=shadow` verifies).
- Index signatures: look up a name's literal type only when the type has index infos (-1.0%, -0.6%).
- Relater: one test for two structured types in `isSimpleTypeRelatedTo` (-0.4%); two different regular literals of
  one kind are unrelated (-1.5%), also in flow narrowing and in T & P constraint reduction; relation keys for
  non-references without the key builder (-0.35%).
- Inline fast paths for functions with many calls whose common answer is trivial: `getApparentType`,
  `getReducedApparentType` for a plain object type, `getNormalizedType`, six predicates, `Node::text` for identifiers.
- No allocation where a borrow or an in-place append does: paths patterns, discriminator names, distributed
  constituents, contextual property types, accessed property names, excluded properties, member-table walks.
- Presized member tables for spreads and object literals; `vfsmatch` tests a directory's entries only against the
  excludes that can match below it (-1.2%); a one-entry cache of the emit module format (-0.2%); a direct-mapped cache
  of each node's file for `compareNodes` (-0.3%).
- Symbol names hashed with one widening multiply up to 16 bytes (-0.3%); mimalloc's plain entry points for the global
  allocator when the alignment needs nothing more (-0.5%, peak RSS unchanged).

Several of these are below the 1% bar on their own, and a few add a cache (emit module format, `compareNodes` file
cache, indexed-access memo) or a memo invariant (the `ObjectFlags` memo bits). They are separate commits so each can be
dropped on its own.

## Measured and rejected

Added to notes/perf-round2-followups.md "Measured and rejected". In short: inline or split fast paths in
`getPropertyOfType` (+2.6% to +2.9%: most calls miss the resolved members, and extra inline code at many call sites
costs more than it saves), union fast paths in `getNormalizedType` and `getReducedApparentType`, cached recursion
identities per relater stack entry, and negative entries in union/intersection property caches (+0.05%).

## Found on the way, not pursued

- One checker (`--checkers 1`) on the 38k-file codebase aborted after 2 h 7 min with "the arena address range is
  exhausted (32751 MiB in use)" at 51 GB RSS (measured on this branch before the rebase; main not compared).
- 16M of 34M union/intersection property syntheses repeat an earlier miss for the same type and name; caching the
  misses was a net loss with the current table layout (above), but a cheaper negative filter might not be.
