# perf-incremental-parallel: the warm incremental run

Goal: make the everyday warm `--incremental` run on the 38k-file codebase at least as fast as tsgo's. Before
this branch a warm run with no edits spent 11 ms checking and ~5.4 s in bookkeeping: "Changes compute time"
3.4 s (tsgo 1.1 s) and "Emit time" 2.0 s (tsgo 1.7 s), which is only the 52 MB tsbuildinfo write. Same output:
the tsbuildinfo bytes, the diagnostics and the incremental behaviour must not change.

## Scenarios

All runs: `tsrs -p . --noEmit --extendedDiagnostics --pretty false` in the 38k-file codebase, each binary with its
own `--tsBuildInfoFile`.

- **warm, no edit (writes tsbuildinfo)**: the first run after a cold incremental build. It rewrites the tsbuildinfo
  (the cold run's file differs from the warm one by ~155 bytes, in tsgo too), so it pays both the changes compute
  and the write. This is the run the "reference numbers" in the brief describe. Reproduced by copying the cold
  run's tsbuildinfo (`seed`) over the binary's file before each run.
- **warm, no edit, steady**: the run after that. Nothing changed, so nothing is written (Emit ~0.1 s in both).
- **one leaf edit**: append `const __tsrsLeafEdit = N;` to `src/debugwitholympuscontext.ts` (no file imports it),
  run, restore the file. Rechecks one file and rewrites the tsbuildinfo.
- **cold**: `--incremental false`, to show it is unaffected.

## Changes compute (`programtosnapshot.rs`)

`computeProgramFileChanges` hashes every file's text, computes its referenced files (imports, triple-slash
references, type reference directives, module augmentations and every ambient module's declaring files) and
compares them with the old state. Go runs the per-file function on a `core.WorkGroup`
(`programtosnapshot.go:91`); the port ran a sequential loop.

What was slow, in order (macOS `sample` on the warm run):

1. **Sequential loop.** Now the per-file function runs on the program's rayon pool
   (`tsrs_compiler::worker_pool`, the pool the file loader and the binder use; now `pub`), and
   `--singleThreaded` keeps the sequential loop. `tsrs_core::workgroup` is a sequential stand-in (no thread
   anywhere), so it is not what the file loader uses and not used here. The function only reads shared state and
   returns its results; they are stored into the snapshot afterwards in file order, so the snapshot's maps are
   filled in the same order as before.
2. **Every file's reference set holds ~390 ambient-module files.** `getReferencedFiles` adds the declaring file of
   every ambient module's every declaration; one `.d.ts` that declares many modules was pushed (path clone +
   hash) once per declaration. The declaring files of a checker's ambient modules are now collected once per
   checker (deduplicated by `P<SourceFile>`) and filtered per file (`!= file`). A set ignores duplicates, so the
   set is the same.
3. **The checker lock.** Go holds the file's checker for the whole of `getReferencedFiles`. With 4 checkers and
   18 workers that serialised the loop. Only the symbol lookups need the checker; they now collect declaring
   `P<SourceFile>`s under the lock and the paths are hashed into the set after it is released, in Go's insertion
   order (imports, triple slash, type references, augmentations, ambient modules).
4. **A sort per file.** The deleted-reference check sorted each file's ~390 references before testing them. Its
   outcome ("any reference is missing from the new program but present in the old state") does not depend on the
   order; it is now membership in one precomputed set of deleted files, which is usually empty, so the 15 M
   `get_source_file_by_path` lookups are gone too.
5. The new reference set is moved into the snapshot instead of cloned, and sized up front.

A first attempt reserved the set for all pushed declaring files before deduplicating: with the per-declaration
duplicates that made 38k tables of ~8k buckets, +4.8 GiB peak and slower inserts. Deduplicating first fixed both.

**What the parallel function touches** (docs/PORTING.md "Threading"): `SourceFile` text, imports, module
augmentations and statements (frozen after binding; `bind_source_file` runs under its `Once`, as when the program
binds in parallel); `Program` maps that are built at construction and only read (`source_file_meta_datas`,
`files_by_path`, `lib_files`, type reference resolutions) and the project-reference mapper's sync maps; a checker
only through `get_type_checker_for_file_exclusive`, i.e. under its mutex, as every other parallel checker user
does; the old snapshot's and the new snapshot's `SyncMap`/`SyncSet`s, read only (each is a `Mutex`); the
`DiagnosticsCache` `Arc<Mutex<..>>`s; diagnostics repopulated for unchanged files are allocated in the worker's own
arena (leaked, like every arena object). It writes nothing shared: the `Cell` fields of `Snapshot`
(`build_info_emit_pending`, `options`, ...) are read before the loop or written after it on the calling thread.
`P<T>` is `Send + Sync` by decree, so the compiler does not check this; the list above is what I checked.
Two scheduling details:

- The checker pool is created before the parallel loop. Its lazy initialisation (`create_checkers`) itself runs a
  rayon `par_iter`; a worker waiting in that join can steal another file's function, which would call
  `create_checkers` again on the same thread, inside the pool's `OnceLock` initialisation.
- Nothing called while a checker mutex is held enters the rayon pool (the pool's users are the file loader,
  binding, checker creation, diagnostics collection and statistics), so a worker cannot steal a task that waits on
  the mutex it holds.

The per-checker ambient-module cache is keyed by the checker's address for the duration of the loop. It relies on
a checker's ambient modules and their declaration lists being fixed once the checker is initialised (globals are
merged, and module augmentations applied, in `initializeChecker`), which is what makes Go's per-file
`GetAmbientModules` return the same list every time.

`affectedfileshandler.go` (lines ~365, ~383) and `emitfileshandler.go` (~124) also use work groups, and the port
runs them sequentially as well (in sorted path order, see the header of `affectedfileshandler.rs`). They are left
as they are: in the measured scenarios they cost nothing (no edit: no changed files; a leaf edit: one changed file
whose shape does not change; `--noEmit`: no per-file emit), and parallelising them means replacing the handler's
`RefCell`s with locks and giving up the deterministic order for the signature updates. Worth doing with an
emit-enabled benchmark where one change re-emits many files.

## tsbuildinfo write (`snapshottobuildinfo.rs`, `tsrs_core::json`)

New phases rows split the write: building the `BuildInfo` from the snapshot 1.33 s, `marshal` 0.63 s (building the
`json::Value` tree and printing it), writing 0.01 s. The output buffer, pretty printing and writer were not the
problem.

- **Printing numbers.** `fileIdsList` holds ~9.5 M file ids. Every `Value::Number` went through the ES6
  shortest-round-trip formatter: `format!("{:e}")`, a digit `String`, an exponent parse and a new `String`. An
  integral value below 2^53 prints as its digits in every ES6 case (`k <= n <= 21`), so it is now written directly
  with an integer formatter. Strings copy their unescaped runs with one `push_str` instead of a push per byte.
  marshal 0.63 -> ~0.14 s.
- **`toFileIdListId`** sorted every reference set's ~390 `Path`s (string compares) before mapping them to ids, then
  built a `"1,2,3"` key string with one `String` per id. Go maps the set's keys in random order, which only matters
  for a path that gets a new id here; the port sorts so that new ids are deterministic. Now only the paths without
  an id are sorted (the result is the same: paths that have an id keep it), and the id list itself is the map key
  (`"1,2,3"` and `[1,2,3]` identify the same list).
- **The lookups** (15 M path -> id hash lookups) for the sets whose paths all have ids already (every program file
  got one in `setFileInfoAndEmitSignatures`) run on the worker pool against the read-only id map; a set with a path
  that has no id goes through `to_file_id_list_id` in the original order, so new ids are handed out exactly as
  before.
- `setFileInfoAndEmitSignatures` computed each file's relative path twice (once for the name, once for Go's
  sanity check); a file that just got its id under that relative name skips the second computation.
- `ensurePackageJsonsForState` realpaths each package.json entry; these independent lookups now run on the
  worker pool (both lists are sorted afterwards). 0.10 -> 0.04 s; this is in every warm run, including the steady
  one.

Build from snapshot 1.33 -> ~0.14 s. The whole write ("Emit time" of the warm run) 2.0 -> ~0.35 s.

## tsbuildinfo read

0.33 s before: `json::unmarshal` + `unmarshal_json` ~0.2 s, `build_info_to_snapshot` ~0.2 s. Two cheap changes:
`build_info_to_snapshot` gave every file a clone of its reference set; Go shares one `*Set` per id list between the
files that use it, and so does the port now (`Arc<Set<Path>>`, `ReferenceMap::store_shared_references`); and the
JSON parser reads integers of up to 15 digits without the float parser (bit-identical, tested). ~0.33 -> ~0.30 s.
The rest is the generic `Value` tree; not pursued.

## Results

Base `ff92b7f` (origin/main), new = this branch, tsgo-ref; 4 checkers; interleaved base/new/tsgo per round,
median of 3 rounds (range in parentheses); load 10-45 from other agents during the runs. Seconds; peak RSS in GiB.

| scenario | phase | base | new | tsgo |
| --- | --- | --- | --- | --- |
| warm, no edit (writes) | **total** | 8.60 (6.93-13.53) | **1.75** (1.74-1.89) | 5.13 (4.84-5.25) |
| | changes compute | 3.61 | **0.32** | 1.54 |
| | emit (tsbuildinfo write) | 2.17 | **0.33** | 1.99 |
| | buildinfo read | 0.39 | 0.31 | 0.42 |
| | parse | 0.86 | 0.67 | 0.95 |
| | peak RSS | 3.52 | 3.40 | 6.61 |
| warm, no edit, steady | **total** | 5.07 (4.82-6.11) | **1.56** (1.41-1.60) | 3.28 (3.03-3.34) |
| | changes compute | 3.78 | 0.32 | 1.33 |
| | emit | 0.11 | 0.04 | 0.15 |
| one leaf edit | **total** | 8.99 (8.25-9.60) | **3.34** (3.33-3.58) | 7.87 (7.81-11.96) |
| | changes compute | 4.44 | 0.32 | 1.35 |
| | check | 1.36 | 1.48 | 3.28 |
| | emit | 2.10 | 0.39 | 1.72 |
| | peak RSS | 3.98 | 3.85 | 7.63 |
| leaf edit reverted | **total** | 6.95 (6.79-7.25) | **1.85** (1.80-1.92) | 5.62 (4.60-5.98) |
| cold, `--incremental false` | **total** | 8.35 (7.37-9.19) | 7.68 (7.09-9.21) | 20.93 (20.75-23.10) |
| | check | 7.33 | 6.62 | 19.63 |
| | peak RSS | 6.71 | 6.70 | 25.19 |

The cold run does not go through any changed code path; its difference is within the run-to-run spread (its
ranges overlap). Single-threaded (`--singleThreaded`), steady warm run: changes compute 4.8 s -> 1.3 s.

The tsbuildinfo after every warm and leaf-edit run above: base and new byte-identical (`cmp`), and new identical
to tsgo's apart from `version` (6/6). `Files`/`Identifiers`/`Symbols`/`Types`/`Instantiations` identical between
base and new in the steady warm run and the cold run.

## Gates

- Conformance, `--baselines types,symbols`, default and `TSRS_LAZY_MEMBERS=0`: `target/test-results` trees
  identical to base (`diff -r`, `summary.json` differs only in `ms`). 13,458 pass + 2 codes + 2 fail of 15,197;
  `.types` / `.symbols` 12,779 / 12,779, both binaries.
- `--baselines js,jsmap,sourcemap`: identical trees; 13,462 error baselines, `.js` 13,392 pass / 0 fail.
- Fourslash: 4,066 pass / 63 fail, same pass list.
- `cargo test -p tsrs_cli`: green (on macOS the bin's `api::memory_tests` needs glibc `malloc_trim` and does not
  link, on main too; I gated it to Linux locally, not committed). tsctests with a fresh dump
  (`tools/oracle/tsctests/dump.sh`): tsc 187/216, tsbuild 187/190, same pass/fail lists as base; the one differing
  failing actual is `internal-symbolname-in-tsbuildInfo` printing a symbol id that is not sanitized and varies
  between runs.
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked`: clean.
- `tools/oracle/incremental/fixtures/run-all.sh` plus the `cycle` and `graph` fixtures: identical output before
  and after (inc1, inc2, b1, dmap, graph identical to tsgo; `b1-outputs` and `cycle` differ from tsgo on both
  binaries only in macOS's `/private/tmp` prefix in printed paths). Run with GNU sed first on `PATH` on macOS.

## Reproduce

```sh
cd <38k-file codebase>
B=<worktree>/scratch                       # one tsbuildinfo per binary
tsrs -p . --noEmit --extendedDiagnostics --pretty false --tsBuildInfoFile $B/seed/t.tsbuildinfo   # cold, makes the seed
cp $B/seed/t.tsbuildinfo $B/n/t.tsbuildinfo
tsrs -p . --noEmit --extendedDiagnostics --pretty false --tsBuildInfoFile $B/n/t.tsbuildinfo      # warm, no edit (writes)
tsrs -p . --noEmit --extendedDiagnostics --pretty false --tsBuildInfoFile $B/n/t.tsbuildinfo      # steady
```

The `BuildInfo read: ...` and `BuildInfo: ...` rows under `--extendedDiagnostics` split the read and the write.
Byte comparison: the tsbuildinfo of base and new after the same scenario with `cmp`; against tsgo after
`sed 's/"version":"[^"]*"/"version":"V"/'` (docs/EMIT.md), with both files at the same depth so the relative
`tsBuildInfoFile` option matches.
