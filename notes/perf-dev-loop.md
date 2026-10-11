# perf-dev-loop: the edit-check loop on the 38k-file codebase

Goal: after `perf-incremental-parallel` (PR #37), account for the remaining time of the runs a developer or an
agent pays on every `typecheck`: a warm `--incremental` run with no edit, and one with a leaf edit. Then take what
can be taken without changing the output: same diagnostics, same tsbuildinfo bytes, same counters.

## Scenarios and method

All runs: `tsrs -p . --noEmit --extendedDiagnostics --pretty false` in the 38k-file codebase (4 checkers), each
binary with its own `--tsBuildInfoFile`; `scratch/run.sh`-style driver, not committed.

- **no edit, steady**: the tsbuildinfo of a previous warm run is copied in first; nothing is rewritten.
- **no edit, writes**: the cold run's tsbuildinfo is copied in first, so the run rewrites it (PR #37's "warm, no
  edit").
- **leaf edit**: append `const __tsrsLeafEdit = N;` to `src/debugwitholympuscontext.ts` (nothing imports it).
- **hub edit**: append `export type __TsrsHubEditN = number;` to `src/orm/db.ts`, which 1,870 files import
  directly; 22k files' declaration signatures are recomputed and their diagnostics refreshed.
- **cold**: `--incremental false`.

Edited files are restored from a copy after every run and checked with `cmp`. Base = `90ef4f8`, the tree of
origin/main `86eaca6` (PR #37 merged); new = this branch; tsgo = `tsgo-ref`. Interleaved base/new/tsgo per round,
median of 3 rounds. Load 3-9 at the start; another agent ran on the machine throughout.

Instruments:
- `--extendedDiagnostics` phase rows (`tsrs_core::phases`), plus temporary rows (per loader round, per change
  compute step, per signature level) that are not committed.
- New committed rows `FS: stat / read file / read dir / realpath lstat`: OS file system calls through `osvfs`.
- samply (`samply record -s --unstable-presymbolicate`) and a small reader of the profile JSON (per-thread CPU,
  inclusive/self, callers, a 50 ms timeline of the dominant stack). Not committed.
- A standalone Rust microbenchmark of the file system operations the loader does, on the corpus's own file list
  (`read`, `open`, `openat` relative to a directory fd, `stat` of the 113k probed paths, `readdir` of their 11k
  parent directories), at 1-18 threads.

## Where the time went (before)

No edit, steady (base, quiet machine): total 1.43-1.67 s.

| phase | s | what it is |
| --- | --- | --- |
| process start | 0.005 | |
| config | 0.10-0.11 | include glob 0.09 (listing prefetch 0.05) |
| buildinfo read | 0.30 | unmarshal 0.17, to snapshot 0.12; ran before program construction |
| parse (program construction) | 0.62-0.92 | root lookups 0.03-0.12, parallel parse + resolve 0.46-0.67 over 14 loader rounds, sequential load 0.06, collect 0.03, verify options 0.03 |
| changes compute | 0.28-0.31 | checker creation 0.06-0.09 (binds all files), file assignment 0.02, per-file loop ~0.2 |
| check | 0.011 | replays the stored diagnostics |
| emit | 0.04 | package.json realpaths 0.036, has-errors 0.005 (nothing is written) |
| statistics | 0.01 | `--extendedDiagnostics` only |
| exit | 0.036 | after `Total time` until the process is gone |

Leaf edit (base): total 3.39 s, the same plus **check 1.25 s** and emit 0.32 s (the tsbuildinfo write: from
snapshot 0.14, marshal 0.13, package.jsons 0.04).

What the leaf edit's check second was, measured: **not** checker creation. Creating the 4 checkers, including
binding all 37,942 files, is 0.06-0.09 s; merging globals and module augmentations is ~7 ms of profile; the edited
file's signature and diagnostics are < 20 ms. 1.40 s of the 1.48 s profile was `ReferenceMap::get_referenced_by`,
which, on its first call, inverts the whole reference map: Go builds `referencedBy` for all 37,942 files (each
references the ~390 files declaring ambient modules, ~15 M entries) to answer "who references this one file".

Exit: `std::process::exit` and `libc::_exit` both leave 36 ms between the exit call and the process being gone
(8 runs alternating): the leak arena runs no destructors, and what remains is the kernel unmapping ~2.5 GB.

Program construction (profile of the steady run, 14 s CPU over 18 workers): `open` 4.4 s, `read` 2.5 s, `stat`
1.4 s, `lstat` 0.7 s (realpath), parsing 1.0 s, binding 0.66 s, mutex waits 1.2 s (resolver caches and checker
locks), hashing/inserting reference sets 1.1 s. The loader's first round parsed the 27.5k root files; rounds 2-13
parsed the 11k files they reach one dependency level per round (0.2 s of mostly idle workers).

## Changes (in order of measured size)

1. **`get_referenced_by` without inverting the map** (`referencemap.rs`). The first query groups the files by
   their (shared) reference set; the first 256 queries test each distinct set for the path; only then is the
   inverted map built, from the same snapshot, keyed to set indices instead of per-file path copies. A leaf edit
   asks twice. Leaf check 1.25 -> 0.02 s.
2. **Read the tsbuildinfo while the program is built** (`execute.rs`). The read only needs the config and the
   file system; it runs on its own thread (`--singleThreaded` keeps Go's order). The read takes longer when it
   shares the CPU (0.30 -> 0.45 s) but it is off the critical path: program construction takes 0.6 s.
3. **Parse ahead of the round barriers** (`filesparser.rs`). The first parallel round follows every reference that
   will certainly be loaded (resolved triple-slash references and type reference directives; imports that
   `resolved_import_sub_task`, the decision now shared with `resolve_imports_and_module_augmentations`, adds and
   does not elide by depth) and parses and resolves it on the spot, recursively, on the same rayon scope. Later
   rounds take those results (by path and file name) instead of parsing again. Only files the program loads are
   touched, so the resolver and package.json caches see the same requests in a different order (the resolution
   caches are order-independent, notes/speed-frontend.md). Off with project references, `libReplacement`,
   `--traceResolution` and `--singleThreaded`; lib files are left to the loader. Rounds 2-14 now parse 0-27 files
   each; 11,334 files are parsed ahead, none unused.
4. **`SyncMap` behind a `RwLock`** (`tsrs_core::collections`). The resolver and package.json caches are read by
   every worker and rarely written. No caller re-enters a map while holding its lock.
5. **The change computation's checker lookups one task per checker** (`programtosnapshot.rs`): 18 workers took
   turns on 4 checker locks once per file. Now `Program::for_each_checker_group` runs the symbol lookups of each
   checker's files on that checker (file order), then the rest runs on the worker pool.
6. **Keep a file's old reference set when nothing changed** (`programtosnapshot.rs`). Instead of building a new
   ~400-path set per file and comparing it with the old one, the old set is checked against the referenced files
   (same size, contains every path) and shared when they match. Changes compute 0.24 -> 0.17 s, steady peak RSS
   2.84 -> 2.53 GiB.
7. **Bind where the loader parses in parallel** (`filesparser.rs`): the parse rounds are bound by file system
   calls; binding right after parsing fills that slack, and the change computation (whose first step binds, via
   `file_affects_global_scope`) and checker creation no longer do it. Binding depends only on the file and runs
   under its `Once`, exactly as when the checkers bind in parallel. Changes compute 0.15 -> 0.10 s.
8. **Affected files' declaration signatures a level at a time** (`affectedfileshandler.rs`, hub edit). Go's
   `forEachFileReferencedBy` walks the referencing files depth first, emitting each file's declaration in turn to
   see whether its signature changed. The visited files are those reachable through files whose signature changed,
   and whether a signature changed does not depend on when it is computed, so the walk now goes level by level and
   computes a level's signatures with one emit, which runs each checker's files on its own thread. Hub edit: 13
   levels (1,870, 1,446, 4,398, 6,804, 3,292, 2,332, 1,542, ... files), Emit time 31 -> 15 s. **Caveat:** each
   checker now emits its files in level order instead of the depth-first order, so the checkers create slightly
   different numbers of types: hub-edit Symbols 16,577,833 -> 16,573,882, Types +36, Instantiations +195. The
   tsbuildinfo (signatures and stored diagnostics) is byte-identical and the reported diagnostics are the same; Go's
   own order here is random (map iteration). This is the one change that moves a counter; it is the last commit
   and can be dropped on its own.

Also committed: the `FS:` call-count rows.

## Results

Seconds; peak RSS in GiB; median of 3 interleaved rounds (range of the total in parentheses).

| scenario | phase | base | new | tsgo |
| --- | --- | --- | --- | --- |
| no edit, steady | **total** | 1.67 (1.34-2.26) | **0.89** (0.87-1.18) | 3.43 (2.75-3.48) |
| | buildinfo read | 0.30 | 0.46 (overlapped) | 0.40 |
| | parse | 0.92 | 0.64 | 0.91 |
| | changes compute | 0.28 | 0.10 | 1.64 |
| | peak RSS | 2.66 | 2.57 | 5.77 |
| no edit, writes | **total** | 2.21 (2.14-2.23) | **1.37** (1.21-1.60) | 5.13 (4.48-6.49) |
| | changes compute | 0.38 | 0.11 | 1.56 |
| | emit (tsbuildinfo write) | 0.41 | 0.31 | 2.11 |
| | peak RSS | 3.39 | 3.09 | 6.49 |
| leaf edit | **total** | 3.39 (2.98-3.40) | **1.30** (1.17-1.33) | 8.65 (8.15-9.92) |
| | check | 1.25 | 0.02 | 3.50 |
| | parse | 1.04 | 0.75 | 0.92 |
| | changes compute | 0.31 | 0.11 | 1.63 |
| | emit (tsbuildinfo write) | 0.32 | 0.32 | 1.74 |
| | peak RSS | 3.88 | 3.08 | 7.60 |
| hub edit (load ~20) | **total** | 52.6 (50.9-57.6) | **25.7** (25.3-26.3) | 97.1 (91.6-106.0) |
| | emit (signatures + write) | 43.2 | 18.4 | 74.9 |
| | check | 7.76 | 6.40 | 19.3 |
| | peak RSS | 9.44 | 8.72 | 27.07 |
| cold, `--incremental false` | **total** | 8.29 (7.70-8.45) | 8.57 (8.37-15.04) | 25.47 (22.80-31.63) |
| | parse | 1.18 | 0.82 | 1.93 |
| | check | 7.00 | 7.53 | 22.90 |
| | peak RSS | 6.71 | 6.72 | 23.41 |

The no-edit, leaf and cold rows were measured with the branch before the last commit (signature levels), which
does not run in those scenarios (no file changed, or a changed file nothing references); the hub row with the
final binary, later and under more load (an earlier quieter pair: base 38.8 s, new 22.9-29.3 s). The cold check
difference is run-to-run spread (one new round at 15.0 s under a load spike; the other two 7.4 and 7.5 s);
the cold run only goes through the parse-ahead and bind changes, which make parse 0.36 s faster.

The tsbuildinfo after every no-edit (writes), leaf and hub run: base and new byte-identical (`cmp` of runs with the
same edit text: writes 3/3, leaf 3/3, hub 6/8; the other two hub pairs differ only in the version and signature of
`src/apiserver.test.ts`, a file another agent on the machine was editing between the base and the new run). `Files/Lines/Identifiers/Symbols/Types/Instantiations` and the
lazy-member counters identical between base and new in the steady, leaf and cold runs.

## After: where the no-edit run goes now

| phase | s |
| --- | --- |
| config | 0.10 |
| buildinfo read (own thread) | 0.45, overlapped |
| parse (critical path) | 0.60-0.64: root lookups 0.035, parallel parse + resolve 0.46, sequential load 0.06, collect 0.03, verify options 0.03 |
| changes compute | 0.10: checker creation 0.003, assignment 0.02, checker lookups ~0.04, per-file ~0.04 |
| check, emit, statistics | 0.012, 0.04, 0.011 |
| exit | 0.036 |

Leaf edit: the same plus the tsbuildinfo write, 0.32 s (from snapshot 0.14, marshal 0.12, package.jsons 0.04).

## Measured and not done

- **Fewer syscalls in program construction.** One warm run makes 113.6k `stat`s (file/directory existence probes,
  96k of them for files; 51k exist), 40.4k file reads, 5.3k directory reads and 150k `lstat`s (realpath walks every
  path component, like Go's `EvalSymlinks`). The microbenchmark on this machine, 18 threads: reading the 37.9k
  files 0.31-0.33 s (best 0.29 s at 6 threads; one thread 0.62 s), of which `open` alone 0.31 s: the kernel's path
  lookup dominates and does not scale past ~4 threads. `openat` relative to a directory fd: 0.35 s, no gain.
  Stat-ing the 113.7k probed paths: 0.30-0.40 s; `readdir` of their 11.2k parent directories (117k entries):
  0.27-0.30 s, so replacing probes by listings is not cheaper here (restricted to the 1,114 directories with >= 20
  probes it would replace 58k probes by 40k entries). These calls already overlap with each other and with parsing.
  The floor for this phase is ~0.3 s of `open`+`read`; it is at 0.46 s.
- **realpath component cache**: the 150k `lstat`s are 0.7 s CPU, ~40 ms wall. Caching the walk per path prefix would remove most of
  them; small and exact, not done.
- **Typed tsbuildinfo decode** (`fileIdsList` 9.5M numbers through `json::Value`): unmarshal 0.17-0.25 s, but the
  read is now overlapped with program construction, so it would save CPU, not wall time.
- **tsbuildinfo write** (leaf edit 0.32 s): not attacked in this round.
- **"Nothing changed" fast path** (written up, not implemented; it is not exact in the strict sense). What a no-edit
  run needs to report the stored diagnostics: the config and file list (include glob), every file's text hash
  compared with `fileInfos.version`, the stored semantic diagnostics (positions need each file's line map, i.e. the
  text, not the AST), and the global and options diagnostics. What it cannot skip: module resolution. The
  tsbuildinfo does not store resolutions, and a new file (e.g. `x.ts` next to `x.d.ts`, a changed `package.json`
  `exports`, a new `@types` package) changes the program without changing any recorded file's text, so the set of
  program files must be recomputed, which needs every file's import specifiers (a scanner pass, not a parse) and
  the resolver. Conditions for skipping the parse and bind of every file: same options (`compilerOptionsAffect*`
  all false and the same `options` value), `errors` false in the tsbuildinfo (no syntactic diagnostics last time;
  the same text gives none again), no pending emit or check, every file's version unchanged, the recomputed file
  list equal (same order) to `fileNames`, and none of `--listFiles`, `--explainFiles`, `--extendedDiagnostics`
  (`Lines`, `Identifiers`, `Symbols` are counted from parsed files). Expected saving: parsing and binding are
  ~1.7 s of CPU but program construction is file-system bound, so ~0.15-0.25 s of wall time; the changes compute
  (0.10 s) would mostly go too. Go does not do this, so the counters of `--extendedDiagnostics` runs would differ.

## Gates

- Conformance, `--baselines types,symbols`, in default, `TSRS_LAZY_MEMBERS=0` and parallel-program
  (`TS_TEST_PROGRAM_SINGLE_THREADED=false`, which is what exercises the parse-ahead) modes: `target/test-results`
  trees identical to base (`diff -r`, `summary.json` excluded). 13,458 pass + 2 codes + 2 fail of 15,197;
  `.types` / `.symbols` 12,779 / 12,779 on both binaries.
- `--baselines js,jsmap,sourcemap`: identical trees; `.js` 13,391 pass + 1 timeout
  (`intersectionConstructorReductionCrash`, which also times out on base, run alone).
- Fourslash: 4,066 pass / 63 fail on both, identical pass/fail/skip lists.
- `cargo test --release -p tsrs_cli` green (with `api::memory_tests` gated to Linux locally, not committed: it
  needs glibc `malloc_trim`). tsctests with a fresh dump (`tools/oracle/tsctests/dump.sh`, run on a private copy of
  the Go module so the shared checkout is untouched): 374 pass / 32 fail / 1 crash, same lists as base; the one
  differing actual is `internal-symbolname-in-tsbuildInfo`, which prints an unsanitized symbol id (as in PR #37).
- `tools/oracle/incremental/fixtures/run-all.sh` plus `cycle` and `graph`: same as base (inc1, inc2, b1, dmap,
  graph identical to tsgo; `b1-outputs` and `cycle` differ from tsgo on both binaries only in macOS's
  `/private/tmp` prefix).
- On the 38k-file codebase: `--listFiles` and `--explainFiles` output byte-identical to base, `--traceResolution`
  output identical (md5).
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked`: clean.

## Reproduce

```sh
cd <38k-file codebase>
B=<worktree>/scratch/n
cp $B/steady.tsbuildinfo $B/t.tsbuildinfo   # a previous warm run's tsbuildinfo
tsrs -p . --noEmit --extendedDiagnostics --pretty false --tsBuildInfoFile $B/t.tsbuildinfo
# leaf edit
cp -p src/debugwitholympuscontext.ts /tmp/leaf.ts; echo 'const __tsrsLeafEdit = 1;' >> src/debugwitholympuscontext.ts
cp $B/steady.tsbuildinfo $B/t.tsbuildinfo; tsrs ... ; cp -p /tmp/leaf.ts src/debugwitholympuscontext.ts
```

`Program:   parsed ahead` counts the files parsed ahead of their round; `FS: ...` the OS file system calls.
