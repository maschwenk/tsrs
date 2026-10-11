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
`TSRS_LAZY_MEMBERS=0`. `tools/regressions.sh`: 29/29.

Public projects (#299's pr-verify run of the bench suite): diagnostics identical in 102 of 102 cells; single-threaded
instructions -6.5% (drizzle-orm) to -22.0% (Compiler-Unions) on all 17 projects; median wall -1.5% to -17.8% per
project; peak RSS -2.9% to +3.5% (the largest increase Compiler at 16 checkers, not investigated).

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

## Round 2 (after #299)

Five more commits on a07bd4b7. Clean `--release` builds, a07bd4b7 (main) against the branch, same command and
machine as above, diagnostics identical:

| | main | branch | change |
|---|---|---|---|
| instructions, default mode | 1.4687T | 1.4074T | -4.2% |
| instructions, `--checkerAssignment locality` | 1.4460T | 1.3869T | -4.1% |
| wall, native, 3 runs each | 31.73-32.44 s | 30.59-31.31 s | -4.1% |
| peak RSS | 16.87-16.99 GB | 17.00 GB | +0.5% |

Conformance pass lists, `tools/regressions.sh` and `--maxMemory 6G` diagnostics as in the first round: unchanged.

- A contextual union's constituents that cannot have the property are skipped (-1.6%). On the 38k-file codebase the
  contextual unions of object literals average over 100 constituents, an object literal asks about each property three
  or four times, and 84% of the (union, name) pairs repeat; the positions of the constituents that can answer are kept
  per (union, name).
- `checkExpressionForMutableLocation` takes the contextual type `isConstContext` already computed (-1.3%).
- The maybe stack of `recursiveTypeRelatedTo` is scanned while it holds at most 32 keys instead of going through its
  hash set (-0.8%).
- Discrimination keeps a large union's discriminant property types that are known for good per name (-0.3%).
- `propertiesRelatedTo` reuses the source lookups `getUnmatchedProperty` made (-0.1%).

The two caches (contextual union positions, discriminant property types) only record answers that hold for good: a
constituent counts only if it is a plain object type with resolved members, no member resolution is in progress when
they are computed, and they are dropped whenever a type's members are reset. Each of the three reuse changes was
shadow-checked (the reused answer recomputed and compared) on the 38k-file codebase and the conformance suite in both
histories, with no difference.

Measured and rejected in this round (also in notes/perf-round2-followups.md): a per-constituent guard instead of "no
member resolution in progress" for the contextual cache (no gain), an absent-property set for large unions before
`createUnionOrIntersectionProperty` (+0.05%), an inline 16-byte compare instead of `bcmp` in symbol-table key
matching (+0.36%), and a per-union memo that the apparent-type mapping of a contextual type is the identity (no
gain).

## Round 3 (after #301)

Nine more commits on cb07284f. Clean `--release` builds, cb07284f (main) against the branch, same command and
machine as above, diagnostics identical:

| | main | branch | change |
|---|---|---|---|
| instructions, default mode (2 runs each) | 1.4051-1.4057T | 1.3580-1.3587T | -3.3% |
| instructions, `--checkerAssignment locality` | 1.3870T | 1.3357T | -3.7% |
| wall, native, 3 runs each | 30.28-31.07 s | 30.10-30.20 s | -2.4% (medians) |
| peak RSS | 16.90-16.95 GB | 16.99-17.08 GB | +0.6% |
| `--maxMemory 6G`: wall / peak RSS (one run) | 73.3 s / 7.65 GB | 73.3 s / 7.68 GB | 0% / +0.3% |

Conformance pass lists (three modes and the gate), `tools/regressions.sh` and `--maxMemory 6G` diagnostics as in the
first round: unchanged. Two runs of the same binary in static assignment differed by up to 0.04% in this round, so
changes below that are noise.

- Property lookups without a call: `propertiesRelatedTo` and `getUnmatchedProperty` look up every target property
  name in the source; for a plain object source with resolved members (no module symbol) the member table answers
  `getPropertyOfType` alone (a value member, or a miss for a name no augmenting global type has), with the type tests
  done once per walk (-1.0%). `createUnionOrIntersectionProperty` does the same per constituent with the name hashed
  once (-0.3%).
- Large unions and intersections: `maybeTypeOfKind` keeps the OR of a large union's constituent flags, nested ones
  included (-0.5%); the deeply-nested test keeps the recursion identities of each intersection on the relater's
  stack (-0.7%) and computes the others inline (-0.45%: the two helpers are forced inline, a plain `#[inline]` was
  ignored; without it on top of the cache the count is 0.17% higher).
- `removeSubtypes` keeps each target's key property type per name for the call: the sources of a discriminated
  union share the name (-0.5%).
- Smaller: a target property's flags tested before its declaration in `getUnmatchedProperty` (-0.2%); an
  identifier's length before its text in `isThisIdentifier` (-0.16%); no candidate list for the contextual property
  types of intersections (-0.09%).

What the reused answers rely on: the member table of a type whose members are resolved (any member reset ends the
walk's use of it, through `members_reset_epoch`), constituents of unions and intersections (never changed after
creation), and property types of a target (dropped with the name or a member reset). Each reuse was shadow-checked
(recomputed and compared, panicking on a difference) on the 38k-file codebase and the conformance suite in three
modes, with no difference. The peak RSS increase is about the size of the two per-checker maps (constituent flags,
intersection identities); not broken down further.

## Round 4 (after #304)

Twenty-one commits on c21a36ef (the last a lint fix), plus this note. Clean `--release` builds, c21a36ef (main)
against the branch, same command and machine as above, diagnostics identical:

| | main | branch | change |
|---|---|---|---|
| instructions, default mode (2 runs each) | 1.3552-1.3591T | 1.3041-1.3087T | -3.7% |
| instructions, `--checkerAssignment locality` | 1.3352T | 1.2859T | -3.7% |
| wall, native, 3 runs each | 30.12-30.81 s | 29.54-29.80 s | -1.9% (medians) |
| user CPU | 220.7-221.9 s | 215.4-217.1 s | -2.2% |
| peak RSS | 16.99-17.08 GB | 16.79-16.90 GB | -1.3% (medians) |
| `--maxMemory 6G`: wall / peak RSS (one run) | 73.3 s / 7.69 GB | 71.8 s / 7.69 GB | -2.0% / 0% |

After merging main at a7eb62be (with #327, a separate round on another workload that made the same pooled-list and
borrowed-list changes and its own symbol-table search; the merge keeps main's versions of those, so this branch's
linear-probing symbol-table index is not in it), the same comparison:

| | main | branch | change |
|---|---|---|---|
| instructions, default mode (2 runs each) | 1.2746-1.2780T | 1.2409-1.2438T | -2.7% |
| instructions, `--checkerAssignment locality` | 1.2596T | 1.2262T | -2.6% |
| wall, native, 3 runs each | 29.05-29.91 s | 28.87-29.13 s | -1.1% (medians) |
| user CPU | 212.7-213.4 s | 209.5-209.9 s | -1.5% |
| peak RSS | 17.00-17.05 GB | 17.00-17.03 GB | -0.2% (medians) |
| `--maxMemory 6G`: wall / peak RSS (one run) | 70.3 s / 7.74 GB | 70.9 s / 7.73 GB | +0.9% / -0.2% |

Conformance pass lists (three modes and the gate), `tools/regressions.sh` and `--maxMemory 6G` diagnostics as in the
first round: unchanged. Per-commit numbers (static assignment, profiling builds) are in the commit messages; the round
went from 1.38165T to 1.33103T (-3.7%) there, before the rebase onto c21a36ef.

- Symbol tables: the index of a table with more than 16 entries is linear probing over 16-bit (up to 255 entries) or
  32-bit slots holding the entry's position and 8 bits of the key's hash, instead of a `hashbrown` table (-0.50%, peak
  RSS lower). 383M of 539M lookups go through the index, 40% in tables of 17 to 64 entries; most of the gain is in
  inserts and growth. An entry's key fingerprint is compared as one word (-0.18%).
- Modifier flags of properties: `propertyRelatedTo` reads accessibility only for class members and synthetic
  properties (a declared symbol whose parent is not a class has no accessibility bits; -0.37%);
  `createUnionOrIntersectionProperty` reuses the read modifiers for writes when there is no set accessor (-0.25%) and
  passes them to `isReadonlySymbol` (-0.07%).
- Name resolution: `resolveName` steps through ancestors that cannot hold locals (calls, property accesses,
  statements, JSX, type literals, ...) without the per-kind tests (-0.36%); `getIsDeferredContext` tests the kind first
  (-0.22%).
- Sorting: `sortSymbols` sorts by a key computed once per symbol when all files are known (-0.30%, shadow-checked
  against the comparator); `compareTypes` compares two string literal types inline and keeps the rest out of line
  (-0.47% over two commits).
- Paths: patterns of `paths` indexed by prefix (-0.34%); a URL scheme separator found by its colons (-0.19%).
- Fewer temporary lists: pooled type lists in `getIntersectionType` and `getConditionalTypeInstantiation` (-0.18%
  together), in `createUnionOrIntersectionProperty` (-0.12%); `inferFromTypes` borrows constituent lists it does not
  change (-0.12%).
- Smaller: one shared empty excluded-properties set (-0.07%); `getUnmatchedProperty` records only looked-up positions
  (-0.08%); the constituent-flags cache only for unions of 8 or more and a pooled store of intersection recursion
  identities (peak RSS -0.26%, instructions unchanged). After the merge with #327 the pooled store gave way to #320's
  arena slices of identity keys: 0.12% fewer instructions, the same peak RSS.

What the new equivalences rely on, each shadow-checked (the original computation run alongside, panicking on a
difference) on the 38k-file codebase and the conformance suite in three modes: accessibility is dropped for
non-class parents in `getDeclarationModifierFlagsFromSymbol` itself; a declared symbol's write declaration differs
from its read declaration only through a set accessor; the symbol table index finds what a linear scan finds;
`sortSymbols`' key order is the comparator's order. (In the shadow build's default-mode run one heavy test,
`intersectionConstructorReductionCrash`, timed out under the extra linear scans; alone it passes with no difference.)

## Found on the way, not pursued

- One checker (`--checkers 1`) on the 38k-file codebase aborted after 2 h 7 min with "the arena address range is
  exhausted (32751 MiB in use)" at 51 GB RSS (measured on this branch before the rebase; main not compared).
- 16M of 34M union/intersection property syntheses repeat an earlier miss for the same type and name; caching the
  misses was a net loss with the current table layout (above), but a cheaper negative filter might not be.
