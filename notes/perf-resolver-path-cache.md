# Resolver filesystem path cache (2026-10-11)

## Scope and prior work

The owner requested reducing resolver filesystem calls using the cache design in oxc-resolver. The comparison
starts at tsrs `d10178f5f80507336834d293c23406806a319cd2`; the design reference is oxc-resolver
`2e976e6489ff902f315d0179a667f8d0465dcee7` (`cache_impl.rs`, `cached_path.rs`, `cached_meta.rs`).

The PR was subsequently reduced to resolver work only. Both sides of the measurements below include an earlier
bitflag-width refactor that has now been removed from the PR. These tables remain historical screening data;
performance qualification must be repeated against the current base before landing the revised branch.

The earlier lightweight metadata and resolver scratch-string experiments did not clear the performance bar
(`notes/perf-vfs-lightweight-metadata.md`, `notes/perf-resolver-allocations.md`). This experiment changes a different constraint:
file/directory probes, canonicalization, and package reads share one invocation's path records. In particular,
macOS realpath reuses cached physical metadata for shared directory prefixes. It does not change the
resolver's search order, package interpretation, semantic caches, or trace messages.

## Implementation

`crates/tsrs_vfs/src/osvfs/resolvercache.rs` owns a sharded intern table keyed by the complete path. A record stores
its spelling and hash, weak parent and canonical-path links, two atomic metadata bytes (link and followed views),
and lazy full stat, directory listing, and raw package data. Missing results are cached too. The table owns the
records; weak links prevent parent/canonical cycles from retaining a compilation after it ends. Hash collisions
still compare the entire path. As in Oxc, concurrent cold metadata probes may race instead of allocating a lock
per metadata slot. Recursive canonicalization computes outside a `OnceLock` initializer to avoid deadlock.

Metadata and read paths retain `Common::split_path`'s lexical normalization. Realpath uses the raw spelling, so
`symlink/..` follows physical traversal. Failed canonicalization is distinct from a successful identity mapping;
a failed realpath cannot incorrectly prove that a dangling link exists. File existence retains Go's definition
of an existing non-directory, including devices and FIFOs. The initial implementation did not infer absence from
directory listings; the follow-up experiment below uses a separate, guarded snapshot of complete names.
Raw `package.json` text is stored on the directory record; the resolver retains its parsed package cache and
first-writer behavior. Source reads without backslashes or trailing separators bypass the intern table; ordinary
source contents are not retained by this cache.

macOS runs the existing Go symlink walker with cached metadata/readlink callbacks. Linux follows the pinned Go
`nativepath.Realpath` algorithm: `O_PATH | O_CLOEXEC`, `/proc/self/fd` readlink, and RAII close. It retries interrupted
open/readlink calls, grows the readlink buffer when needed, and falls back to the Go walker when the initial
`/proc/self/fd` metadata probe fails.
The Linux backend also replaces the old `std::fs::canonicalize` path outside cached compilations.

The Linux and macOS CLI creates a fresh cache for each nonincremental `--noEmit` compilation with no external code or
testing hook. It bypasses the redundant generic `cachedvfs` layer. Incremental/build, emit, LSP/API, WASM, and custom
filesystems retain their existing caching unless a custom `System` explicitly supplies a compilation filesystem.
Windows also keeps the existing cache. Resolution tracing remains eligible. Inputs must stay stable during the
invocation; there is no persisted cache or invalidation protocol.

Canonical-parent redirection was rejected on correctness grounds: replacing an alias parent with its resolved
path restarts the kernel's symlink traversal budget, so a child probe can succeed where the original lookup fails.
For the same reason, macOS symlink existence uses actual followed metadata. Its Go realpath walker accepts up to
255 links, while the kernel's followed lookup has a lower limit; a successful realpath cannot prove the original
path passes a followed metadata probe. Linux's `O_PATH` lookup retains the kernel limit. A regression covers a
finite 64-link chain with successful macOS realpath but failed file/directory existence, in both call orders.

## Validation before PR scope cleanup

The local validation and measurements below run on macOS arm64. Linux checks use a cross target; they do not
establish Linux runtime or performance results.

- VFS: 103 passing cases, including new coverage for shared positive/negative probes, raw package reads,
  independent listings/stat, collisions, concurrent interning, cache lifetime, symlink traversal/errors/casing,
  permissions, special files, and a fresh cache after filesystem changes.
- Module/compiler/execute/CLI: 68 passing cases. The command-line matrix checks cache selection and fallback for
  no-emit, tracing, emit, incremental, external-code, testing-hook, and custom-filesystem configurations.
- Module oracle: 734/734 usable scenarios match the pinned reference, including traces. Three of the 737
  generated scenarios fail in the reference and are excluded.
- Native CLI output and exit status are byte-identical to the baseline on both public projects, separately for
  default checking, single-threaded checking, `--listFiles`, `--explainFiles`, and `--traceResolution`. All regression
  fixtures pass (29 cases).
- Full conformance runs compare all 15,197 variants in both CI modes. Both binaries have 13,458 diagnostic passes,
  two code-only matches, and the same two diagnostic failures. Go-history mode has 12,779 type and symbol passes
  each; canonical mode has 12,778 each and the existing `objectLiteralNormalization` difference. There are no
  crashes or timeouts. Every classification list and saved diagnostic/type/symbol artifact matches byte-for-byte;
  the summary records also match after removing case durations. Results are under
  `/private/tmp/tsrs-resolver-cache-parity.tQbUZJ`.
- Workspace and WASM checks pass. Linux VFS library and tests cross-check and Clippy pass with dead-code warnings
  denied. `tools/lint/ratchet.py` passes (three existing findings, none new); `tools/lint/source.py` passes with the
  existing baseline unchanged.

## Measurement method

Host: Apple M3 Max, 14 physical/logical cores, 36 GiB RAM, macOS 27.0.1 arm64. Both release binaries use
Rust 1.99.0, the same release profile and SDK, and the same public checkouts/dependencies.
Sources were compared byte-for-byte, including executable bits and symlink targets, with archives of the commits
pinned in `bench/projects.json`:

| Project | Commit |
| --- | --- |
| xstate-main | `fbee62e7c1586315ed478c2fedf530d7e0ff5a3e` |
| vscode | `9cf0128b9822dcd8a533f08ef7312956a4390301` |

Xstate's ignored `packages/*/dist` wrappers come from its pinned postinstall (`preconstruct dev`), with no changes
to archived sources. Run each command once to warm it, then seven times per binary, alternating baseline/candidate
order. All builds and correctness checks finish before measurement. Clear ambient `TSRS_*`, `TS_TEST_*`, and
`RAYON_*` overrides; add `RAYON_NUM_THREADS=1` and `--singleThreaded` for the single-threaded rows:

```sh
/usr/bin/time -l /path/to/tsrs -p . --noEmit --incremental false --pretty false --extendedDiagnostics
# In vscode, use -p src instead of -p .
```

Capture retired instructions and peak RSS from macOS `/usr/bin/time -l`, plus the physical filesystem API probe
counters from `--extendedDiagnostics`. Compare diagnostics and exit status on every measured pair. Sum `FS: stat`
and `FS: realpath lstat` in the baseline, and `FS: stat` and `FS: lstat` in the candidate: comparing only `stat` would
hide the move to `lstat`. The baseline does not instrument readlink/open, so their absence is not a zero count.
These counters are not a full syscall trace. DTrace is unavailable on this host (`Operation not permitted`, SIP
enabled). Linux `strace` and deterministic `bench/count.py` measurements require a Linux runtime.

Local raw logs, binary hashes, source verification, commands, and repetitions are under
`/private/tmp/tsrs-resolver-cache-results`; the measurement driver is
`/private/tmp/tsrs-resolver-cache-measure.py`.

## Initial results and landing status

All 64 runs (56 measured plus eight warmups) completed. Diagnostics and exit status match on every pair: xstate
has no diagnostics and exits 0; vscode has the same 371 diagnostic lines and exits 2. Both binary hashes stayed
unchanged throughout measurement. The following are medians of seven repetitions; instruction counts are in
billions and RSS is in MiB.

| Project / mode | Instructions before → after | Change | Peak RSS before → after | Change | stat + lstat before → after | Change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| xstate / single | 6.301349 → 6.253028 | -0.767% | 178.81 → 180.25 | +0.804% | 11,619 → 7,752 | -33.282% |
| xstate / default | 8.317477 → 8.242363 | -0.903% | 256.44 → 257.98 | +0.603% | 11,869 → 7,809 | -34.207% |
| vscode / single | 88.464955 → 88.517709 | +0.060% | 1,607.72 → 1,592.98 | -0.916% | 21,149 → 20,078 | -5.064% |
| vscode / default | 102.206827 → 101.178263 | -1.006% | 2,096.56 → 2,081.41 | -0.723% | 21,152 → 20,086 | -5.040% |

Single-threaded file reads are unchanged: 1,791 for xstate and 10,511 for vscode. At the default thread count they
fall from 1,804 to 1,791 and 10,520 to 10,511 respectively. Directory reads stay at 75 and 2,693. Default-thread
cold metadata races account for small run-to-run differences; the combined stat/lstat column is the median of
each run's sum, not the sum of component medians.

Median wall times are 0.55 → 0.55 s for xstate single and 0.12 → 0.12 s default; vscode is 9.32 → 9.65 s single and
1.37 → 1.50 s default. Vscode wall times varied substantially with host noise. These are not the two controlled
README publishes required for a wall-time claim, and they do not demonstrate a wall-time improvement.

Binary SHA-256:

- Baseline: `09f6c266c5ab8d2b72ce77967eacff2743ec8aa0dd3c462cd263981134df5236`
- Candidate: `ba742a3ca7a612b00020aa6c5ea638cc3370188c8b489c79a0e43099f6a8c99e`

**The filesystem-call reduction is demonstrated; the performance landing bar is not.** Single-threaded macOS
instruction changes are below 1%, default RSS changes are below 5%, and there is no demonstrated wall-time win.
The default-thread instruction result is not the required single-threaded metric, and these macOS counters are
not Linux `bench/count.py`. An added cache needs a clear margin, so these numbers alone do not justify landing it.
The requested implementation remains a draft experiment for review, with Linux runtime and performance
qualification outstanding; it is not qualified to merge or release.

Next validation requires a Linux runtime: run the VFS tests (including procfs fallback and long readlink paths),
the CLI parity checks, and before/after `bench/count.py` on the pinned projects with `RAYON_NUM_THREADS=1` and
`--singleThreaded`; also measure peak RSS at the default checker count. Collect a separate `strace -f -c` run for
metadata, open, readlink, and close calls to verify the Linux backend's effect. No Linux runtime or container
runner was available in this session. If it does not establish the required margin, retain this finding as a note
and drop the implementation rather than treating fewer filesystem calls as sufficient evidence of overall speed.

## Follow-up: instrument first, then reduce cold misses

The owner requested instrumentation of all resolver filesystem calls before further optimization. The optional
`tsrs_vfs/resolver-fs-stats` feature records every resolver FS method at its call site, including indirect directory
walking and case-sensitivity queries. The native layer records filesystem API operations, their arguments and
outcomes under the enclosing resolver call; other compiler/configuration work is labeled `nonresolver`. Separate
cache events describe interning, metadata, canonicalization, reads, and listings. The feature is disabled by
default and its stubs inline away. See `docs/DEBUGGING.md` for invocation and schema.

These are API-operation counts, not complete kernel syscall counts. Directory iterator and entry-type methods
can issue no syscall; a source read hides several. Rustix rows include explicit wrapper retries. Cache-row paths
are the enclosing logical request, not the actual shared prefix/key; physical-row paths are the OS arguments.
Profiled binaries are used only to analyze filesystem traffic, never for CPU/RSS comparisons.

The initial cache's single-threaded profiles identify distinct cold missing candidates as the remaining resolver
cost, rather than repeated queries to the same cache entry:

| Project | stat + lstat | Resolver share | Missing probes | Main resolver missing-probe sites |
| --- | ---: | ---: | ---: | --- |
| xstate | 7,752 | 7,346 | 4,406 | candidate extension 3,222; package parent 562 |
| vscode | 20,078 | 10,403 | 6,236 | package.json 2,541; candidate extension 2,179; node_modules directory 1,296 |

The one-time case-sensitivity metadata probe is excluded from this table, matching extended diagnostics.
Single-threaded duplicate metadata calls are only four on xstate and zero on vscode; adding a lock to every
metadata slot therefore has little headroom. In xstate, missing `.ts` and `.tsx` alternatives account for 1,211
and 1,187 calls. In vscode, 9,675 metadata probes and all 2,693 initial directory reads are outside the resolver.
Configuration already enumerates parents of 4,088 later missing paths, but those scans precede the per-compilation
cache. Sharing them would require a different cache lifetime and is not implemented here.

The follow-up experiment counts actual `NotFound` child probes. After eight misses in a known directory, it reads
a complete name snapshot once and can reject absent ASCII names without another metadata call. It never infers
positive metadata: listed entries, symlinks, special files, and unreadable paths still take their ordinary probes.
The counter saturates; repeated hits on a cached missing entry do not advance it. Raw realpath spellings and
empty/`.`/`..` names cannot use negative inference. Matching is conservatively ASCII case-insensitive; a
non-ASCII or non-UTF-8 directory entry disables the snapshot, and non-ASCII queries always probe normally.

Directory enumeration is not proof on every filesystem: Linux procfs exposes thread-ID paths absent from its
listing. Each attempted snapshot first performs one instrumented `statfs` and accepts only APFS/HFS on macOS or
EXT/XFS/BTRFS/TMPFS on Linux. Unknown types, including overlay and network filesystems, fall back to ordinary
metadata. Expanding this list requires evidence that lookup-visible names are enumerated; reducing a syscall
count is not sufficient. A Linux regression uses a nonleader thread's `/proc/<tid>` path to exercise this fallback.
The separate `FS: statfs` counter makes the extra eligibility cost visible.

Final single-threaded profiles compare the initial cache with the adaptive experiment, including the macOS
symlink-limit correction. The startup case probe is excluded; `statfs` is counted separately.

| Project | stat + lstat before → after | Missing probes before → after | Added statfs | Added directory reads | Added iterator calls |
| --- | ---: | ---: | ---: | ---: | ---: |
| xstate | 7,752 → 5,818 (-24.95%) | 4,406 → 2,283 | 78 | 78 | 3,346 |
| vscode | 20,078 → 18,862 (-6.06%) | 6,236 → 5,020 | 63 | 63 | 2,926 |

Name snapshots suppress 2,123 and 1,216 missing probes respectively. Only two snapshots per project suppress no
later probe. Positive metadata and source reads remain live where required; ordinary reads stay at 1,791 and
10,511. Xstate's symlink correction changes stat calls 35 → 273 and readlink calls 238 → 204. The added iterator
calls are not equivalent to added syscalls, but their CPU/allocation cost must be included in the uninstrumented
measurement. All four project/mode profiles retain identical logical operation/path/result sets and identical
diagnostics/exit status. Single-threaded logical counts are identical too; default counts vary with cold races.
Raw reports and analyzer comparisons are under `/private/tmp/tsrs-resolver-profile-{before,final}`.

Follow-up validation passes 109 VFS cases without instrumentation and 112 with it, including threshold behavior,
case/Unicode fallbacks, special nodes, failed listings, raw paths, and symlink limits. The Linux-specific invalid
UTF-8 and hidden-procfs cases cross-compile but still need a Linux runtime. Another 69 module/compiler/execute/CLI
cases pass; the feature-enabled module oracle matches all 734 usable scenarios. The analyzer passes all four
before/after comparisons, omits paths by default, and reports changed logical tuples as a failure.

Both final full conformance modes again match the original baseline's complete classification lists, saved
artifacts, and summaries after removing durations. All ten public native CLI comparisons and all 29 regressions
pass. The workspace and WASM compile, the lint ratchet and source checks pass without raising baselines, and
both macOS/Linux VFS Clippy checks pass with profiling enabled and disabled. Final parity artifacts are under
`/private/tmp/tsrs-resolver-parity-round2.8k6iy8r9`.

### Follow-up CPU and memory screening

After all builds and validation finished, measure three uninstrumented binaries in the same session: historical
baseline `d10178f5`, the initial cache, and the refined cache. Use the same pinned projects and command/environment as above,
one warmup each and seven measured repetitions per project/mode. Rotate through all six binary orders. All 84
measured runs and 12 warmups complete with identical diagnostics and exit status. Binary hashes stay unchanged;
the refined binary is `391b03388f446de57e6a13b1ddd193ca9eb3181728a3240116e9338270069792`.

Medians below are original → initial cache → refined cache; instructions are billions and RSS is MiB.

| Project / mode | Instructions | Refined vs original | Peak RSS | Refined vs original | stat + lstat |
| --- | ---: | ---: | ---: | ---: | ---: |
| xstate / single | 6.301260 → 6.250963 → 6.278809 | -0.356% | 178.81 → 180.27 → 180.80 | +1.110% | 11,619 → 7,752 → 5,818 |
| xstate / default | 8.304441 → 8.257306 → 8.274625 | -0.359% | 257.08 → 258.58 → 258.67 | +0.620% | 11,813 → 7,850 → 5,942 |
| vscode / single | 88.002986 → 88.411638 → 88.283731 | +0.319% | 1,607.72 → 1,593.00 → 1,593.00 | -0.916% | 21,149 → 20,078 → 18,862 |
| vscode / default | 101.735626 → 102.300724 → 101.242261 | -0.485% | 2,086.92 → 2,090.14 → 2,089.00 | +0.100% | 21,158 → 20,088 → 18,870 |

Versus the initial cache, refined single-threaded instructions change +0.445% on xstate and -0.145% on vscode;
RSS changes +0.295% and 0.000%. This comparison includes the necessary macOS symlink-limit correction, so it does
not isolate the directory snapshot's CPU cost. Whole-program instruction savings are not established. Default
metadata totals vary with scheduling; each table cell is the median of per-run sums. Final uninstrumented
directory reads are 153 / 2,756 and `statfs` calls 78 / 63 for xstate / vscode in both modes.

Wall medians are 0.55 → 0.55 → 0.54 s (xstate single), 0.12 s for all xstate default binaries,
8.94 → 9.05 → 8.83 s (vscode single), and 1.40 → 1.41 → 1.34 s (vscode default). The last row is promising but is
not two controlled README benchmark publishes; it cannot establish the wall-time landing gate. These macOS
screens do not replace deterministic Linux instruction counts or establish a default-memory win.

The refined cache reduces single-threaded metadata calls by **49.93% on xstate and 10.81% on vscode versus the
original**, but **still does not establish the required performance margin**. Instrumentation and analysis are
complete; the requested cache implementation remains a draft experiment, not qualified to land. Do not
treat syscall/API-call reduction alone as a speedup. The next qualification step remains the Linux runtime and
benchmark procedure above, including syscall tracing; if that fails the bar, keep the findings and profiler
and drop the optimization. Raw measurements are in `/private/tmp/tsrs-resolver-round2-final-results/measure`;
the driver is `/private/tmp/tsrs-resolver-cache-round2.py`.

The profiles also estimate higher miss thresholds without changing code. At 32 misses, xstate would retain about
59% of the avoided metadata probes and 54% of the directory-iteration work; vscode would retain 45% and 63%.
At 64, retained avoided probes drop to 38% and 25%. Large scans survive the higher thresholds, so raising the
threshold is not an obvious CPU improvement and sacrifices the requested call reduction. These estimates assume
unchanged candidate order, parent knowledge and eligibility; they are not runtime measurements. No threshold
change is made. Details are in `/private/tmp/tsrs-resolver-round2-final-results/threshold-estimates.json`.

## Resolver-only PR validation before the main rebase

The scope cleanup replayed only the resolver commit on `cd6bac5b`; all 27 bitflag-only files were absent from its diff,
and the resolver implementation is unchanged. The rebuilt branch passes workspace/WASM checks, both lint gates,
and 163 focused cases with profiling enabled (112 VFS, 28 module, three execute, 20 CLI), including all 734 usable
module oracle scenarios. All ten public native
comparisons, 29 regressions, and both 15,197-case conformance modes preserve the historical baseline's outputs,
classification lists, and saved artifacts exactly. There are no new failures, crashes, or timeouts.

These comparisons establish semantic parity after the scope cleanup, not current-head performance. Fresh CPU,
RSS, and Linux runtime qualification remains outstanding. Evidence is under
`/private/tmp/tsrs-resolver-parity-resolver-only.m999pemv` and `/private/tmp/tsrs-nobitflags-*.log`.

## Linux CI evidence and why the call reduction is not a CPU win

Linux evidence subsequently became available for resolver-only head `dec28370650d53ce5dc0b17607890c7636eaecfa`.
The [PR verification report](https://github.com/maschwenk/tsrs/pull/324#issuecomment-6107682263) measured merge
`50fbd5afe2a9d955a7d89540db2c01471150e02e`, whose parents are that head and base
`b8bdd76bf789975cbc4d558d9d42fa78675abfd1`. This comparison contains no bitflag changes. All 102 diagnostic cells
across 17 projects match. Linux unit suites including VFS/module/compiler/execute/CLI, both conformance modes,
and regressions pass, superseding the earlier lack of Linux runtime evidence.

Single-threaded user instructions change **+0.015% on vscode** (88.801 → 88.815 G) and **+0.029% on xstate**
(5.947 → 5.948 G). Across all 17 projects, changes range from -0.017% to +0.134%: none clears the instruction gate.
Single-threaded peak RSS changes +0.2% / +1.0%; at 32 checkers it changes -0.1% / +0.5%, respectively. Median wall
deltas across checker counts are +0.1% for both projects, within the report's documented identical-run noise.
This does not demonstrate a qualifying wall or memory improvement either.

Three effects explain why fewer probes do not imply a comparable speedup:

- **The counters omit the target work.** `bench/count.py` sets `EXCLUDE_KERNEL | EXCLUDE_HV`, measuring only
  user-space retired instructions. The [CodSpeed simulation report](https://github.com/maschwenk/tsrs/pull/324#issuecomment-6107698411)
  explicitly warns that system calls are excluded for xstate and drizzle-orm. The cache is enabled in both
  workflows (`--noEmit --incremental false`); this is not a missing-flag problem. Avoiding kernel path lookup
  cannot directly reduce a user-space-only count, while cache hashing and synchronization remain in that count.
- **The changed work is a small part of full checking.** Historical single-threaded phase medians put checking
  at about 71% of xstate runtime and 80% of vscode. Parse+resolve together account for 23% / 13%; filesystem work
  is only part of that. Their combined medians change 133 → 125 ms and 1,173 → 1,162 ms, with overlapping ranges.
  A possible small frontend benefit is diluted by the unchanged checker and parsing work.
- **The cache trades work rather than removing it all.** Followed-metadata hits already serve 66.9% / 90.8% of
  logical requests in the initial cache. Remaining missing candidates are mainly distinct paths. Directory
  snapshots avoid 2,123 / 1,216 probes but add 78 / 63 scans and `statfs` checks, plus 3,346 / 2,926 iterator calls.
  Source/package reads remain 1,791 / 10,511. Hashing, allocations, locks and scans may offset cheap warm metadata
  lookups; their individual CPU costs have not been isolated, so this is an explanation to test, not attribution
  proved by the operation counters.

Historical vscode default-mode medians illustrate the distinction: system CPU falls 1.82 → 1.34 s, while user
CPU is essentially flat at 12.77 → 12.76 s. Wall time falls 1.40 → 1.34 s in that Mac screen, but this was not the
two controlled publishes required for qualification. The Linux reports do not contain raw FS counters, so
directory-snapshot activation under the filesystem whitelist remains unverified there. Local CI evidence is
saved in `/private/tmp/resolver-ci-evidence.json`.

## Rebase onto current main

Rebase the resolver-only change onto `1e62e1443269a15af204b11de7851bc810a857db`, including main's checker
optimizations. Preserve both entries in the follow-up list when resolving the only conflict. The resolver
implementation is unchanged from `dec2837`, and bitflag changes remain absent. Workspace/WASM checks and both
lint gates pass. Ten native comparisons match, all 29 regressions pass, and both 15,197-case conformance modes
match the stored historical baselines' complete classifications and artifacts. Validation evidence is in
`/private/tmp/tsrs-resolver-parity-rebase-main.gx4ejtxh` and `/private/tmp/tsrs-mainrebase-*.log`.

Build plain release binaries for this main and the rebased PR with the same SDK and no Rust compiler/profile
overrides. On the Apple M3 Max, run the same pinned xstate/vscode fixtures, with profiling disabled and compiler,
thread, and profiler environment overrides cleared. Use one warmup per binary/project/workload/mode, then seven
paired repetitions alternating main/PR order: 112 measured runs and 16 warmups. All builds and other validation
are idle during measurement. Each invocation uses `/usr/bin/time -l` and:

```sh
tsrs -p . --noEmit --incremental false --pretty false --extendedDiagnostics
# Use -p src for vscode; add --singleThreaded for single mode, --noCheck for that workload.
```

Both binaries retain identical hashes throughout: main `cce9c4f1789d74d1daf933605cd1dba5d362052a4b56c3b2532d30cde8e22bf0`,
PR `6b503747af1f8be13a98bf4bd7add7928412aec48a99372aae362f90fc392139`. Every paired diagnostic, exit-code,
and file-count comparison matches. Medians below are main → PR; instructions are billions, peak RSS is MiB,
and wall time is seconds. These are Mac screening measurements, not deterministic Linux user-instruction counts.

| Project / workload / mode | Instructions | Change | Peak RSS | Change | Wall |
| --- | ---: | ---: | ---: | ---: | ---: |
| xstate / full / single | 5.914163 → 5.890322 | -0.403% | 178.61 → 180.11 | +0.840% | 0.54 → 0.52 |
| xstate / full / default | 7.753011 → 7.714010 | -0.503% | 255.77 → 257.69 | +0.751% | 0.12 → 0.12 |
| vscode / full / single | 83.730784 → 83.664499 | -0.079% | 1,607.73 → 1,594.28 | -0.837% | 8.84 → 8.46 |
| vscode / full / default | 96.307946 → 96.563995 | +0.266% | 2,096.67 → 2,097.50 | +0.039% | 1.15 → 1.25 |
| xstate / noCheck / single | 2.009624 → 1.915309 | -4.693% | 93.81 → 95.55 | +1.849% | 0.17 → 0.16 |
| xstate / noCheck / default | 2.187413 → 2.162585 | -1.135% | 114.95 → 116.72 | +1.536% | 0.04 → 0.04 |
| vscode / noCheck / single | 21.980613 → 22.373339 | +1.787% | 1,142.23 → 1,146.72 | +0.393% | 1.72 → 1.75 |
| vscode / noCheck / default | 22.845474 → 23.872452 | +4.495% | 1,193.20 → 1,192.61 | -0.050% | 0.47 → 0.49 |

The full-check instruction changes remain small. Single-threaded ranges are 5.908–6.006 G → 5.885–5.894 G
for xstate (sample standard deviations 0.035 / 0.004 G), and 83.615–85.016 G → 83.632–84.863 G for vscode
(0.666 / 0.589 G). Default-mode ranges are 7.737–7.765 G → 7.705–7.741 G and 95.673–97.260 G →
95.905–97.376 G, respectively. Wall results are also noisy: vscode single ranges 8.18–11.21 s → 8.32–8.78 s,
whereas default ranges 1.06–1.41 s → 1.07–1.42 s. The apparent single-thread wall improvement is not a
repeatable default-mode win or the two controlled README publishes required for qualification.

`noCheck` does **not** reveal a consistent hidden frontend improvement. Xstate's single-thread instruction
ranges overlap at 1.931–2.161 G → 1.910–2.153 G (standard deviations 0.085 / 0.090 G); default ranges
2.178–2.407 G → 2.150–2.386 G (0.084 / 0.085 G). Vscode median instructions regress in both modes, also with overlapping ranges:
21.910–22.880 G → 21.973–23.152 G (0.430 / 0.429 G) single, 22.549–23.868 G → 22.611–23.944 G
(0.643 / 0.600 G) default. These noisy screens cannot establish the sign of a small frontend effect.

Single-threaded parse+resolve medians change 123 → 118 ms for full xstate and 1,190 → 1,105 ms for full vscode;
with `noCheck`, they change 134 → 122 ms and 1,129 → 1,150 ms. In default full checking they change
27 → 27 ms / 203 → 211 ms. Thus checking dilution alone does not explain the absence of a speedup.
`noCheck` can also enable lazy declaration parsing and omits some checker-driven xstate probes, so subtracting
it from full checking would not isolate resolver cost. Both workloads load the same 1,536 / 10,427 files;
single-threaded reads are unchanged at 1,791 / 10,511. Single-threaded vscode has identical filesystem counts
between the two workloads within each binary. Xstate full metadata probes change 11,619 → 5,818, versus 11,380 → 5,707 with
`noCheck`; vscode changes 21,149 → 18,862 in both. Metadata counts exclude `statfs`; main lacks some of the
new physical-operation counters, so missing counters are unavailable, not zero.

The measured conclusion remains: fewer metadata calls, **no established overall performance margin**. Kernel
work is outside the Linux instruction metric; user-space cache costs may offset savings, but proving their
contribution needs an ablation or CPU profile. Before further cache tuning, capture Linux physical-call counts
and system/user CPU with and without the cache under the actual benchmark filesystem, then isolate directory
snapshots from the shared metadata cache. Do not attribute the result to filesystem-whitelist fallback without
that evidence. Keep the PR a draft; if controlled measurements still miss the bar, retain the profiler/findings
and remove the optimization.

Raw distributions, phases, and workload comparisons are in `/private/tmp/tsrs-resolver-rebased-results`;
the driver is `/private/tmp/tsrs-resolver-rebased-measure.py`.
