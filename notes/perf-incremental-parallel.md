# perf-incremental-parallel: the warm incremental run

Goal: make the everyday warm `--incremental` run on the 38k-file codebase at least as fast as tsgo's. Before
this branch a warm run with no edits spent 11 ms checking and ~5.4 s in bookkeeping: "Changes compute time"
3.4 s (tsgo 1.1 s) and "Emit time" 2.0 s (tsgo 1.7 s), which is only the 52 MB tsbuildinfo write. Same output:
the tsbuildinfo bytes, the diagnostics and the incremental behaviour must not change.

Status (2026-10-10): landed as #37. The timings here are superseded by notes/perf-dev-loop.md and
notes/perf-dev-loop2.md (the ambient-module reference sets became shared, the tsbuildinfo marshal was rewritten again).
Condensed to the design and invariants that still hold; the scenario definitions, gate log and reproduce steps were
removed.

## Changes compute (`programtosnapshot.rs`)

`computeProgramFileChanges` hashes every file's text, computes its referenced files (imports, triple-slash
references, type reference directives, module augmentations and every ambient module's declaring files) and
compares them with the old state. Go runs the per-file function on a `core.WorkGroup`
(`programtosnapshot.go:91`); the port ran a sequential loop.

What was slow, and what changed:

1. **Sequential loop.** The per-file function runs on the program's rayon pool (`tsrs_compiler::worker_pool`, the
   pool the file loader and the binder use), and `--singleThreaded` keeps the sequential loop.
   `tsrs_core::workgroup` is a sequential stand-in, so it is not used here. The results are stored into the
   snapshot afterwards in file order, so the snapshot's maps are filled in the same order as before.
2. **Every file's reference set holds ~390 ambient-module files.** `getReferencedFiles` adds the declaring file of
   every ambient module's every declaration; one `.d.ts` that declares many modules was pushed once per
   declaration. The declaring files of a checker's ambient modules are collected once per checker (deduplicated)
   and filtered per file. A first attempt reserved the set for all pushed declaring files before deduplicating:
   38k tables of ~8k buckets, +4.8 GiB peak and slower inserts. Deduplicate first.
3. **The checker lock.** Go holds the file's checker for the whole of `getReferencedFiles`; with 4 checkers and
   18 workers that serialised the loop. #37 took only the symbol lookups under the lock. (Status 2026-10-10: the
   current code goes further: the checker lookups run first through `for_each_checker_group`, one task per checker
   holding its checker for all of its files, then the rest of the per-file work runs on the worker pool.)
4. **A sort per file.** The deleted-reference check sorted each file's ~390 references. Its outcome does not depend
   on the order; it is now membership in one precomputed set of deleted files (usually empty), which also removed
   15 M `get_source_file_by_path` lookups.

### What the parallel function touches

(docs/PORTING.md "Threading" has the general rules.) `SourceFile` text, imports, module augmentations and
statements (frozen after binding; `bind_source_file` runs under its `Once`, as when the program binds in parallel);
`Program` maps that are built at construction and only read (`source_file_meta_datas`, `files_by_path`,
`lib_files`, type reference resolutions) and the project-reference mapper's sync maps; a checker only under its
mutex, as every other parallel checker user does; the old snapshot's and the new snapshot's `SyncMap`/`SyncSet`s,
read only (each is a `Mutex`); the `DiagnosticsCache` `Arc<Mutex<..>>`s; diagnostics repopulated for unchanged files
are allocated in the worker's own arena (leaked, like every arena object). It writes nothing shared: the `Cell`
fields of `Snapshot` (`build_info_emit_pending`, `options`, ...) are read before the loop or written after it on the
calling thread. `P<T>` is `Send + Sync` by decree, so the compiler does not check this; the list above was checked
by hand.

Two deadlock hazards (invariants for this loop):

- The checker pool must exist before the parallel loop. Its lazy initialisation (`create_checkers`) itself runs a
  rayon `par_iter`; a worker waiting in that join can steal another file's function, which would call
  `create_checkers` again on the same thread, inside the pool's `OnceLock` initialisation. (The non-batched path in
  `programtosnapshot.rs` still creates it first.)
- Nothing called while a checker mutex is held may enter the rayon pool (the pool's users are the file loader,
  binding, checker creation, diagnostics collection and statistics), so a worker cannot steal a task that waits on
  the mutex it holds.

The per-checker ambient-module cache relies on a checker's ambient modules and their declaration lists being fixed
once the checker is initialised (globals are merged, and module augmentations applied, in `initializeChecker`),
which is what makes Go's per-file `GetAmbientModules` return the same list every time.

### Left sequential: `affectedfileshandler` and `emitfileshandler`

`affectedfileshandler.go` (lines ~365, ~383) and `emitfileshandler.go` (~124) also use work groups; the port runs
them sequentially (in sorted path order, see the header of `affectedfileshandler.rs`). In the measured scenarios
they cost nothing (no edit: no changed files; a leaf edit: one changed file whose shape does not change;
`--noEmit`: no per-file emit), and parallelising them means replacing the handler's `RefCell`s with locks and giving
up the deterministic order for the signature updates. Worth doing with an emit-enabled benchmark where one change
re-emits many files.

## tsbuildinfo write and read

- **Printing numbers** (`tsrs_core::json`). `fileIdsList` holds ~9.5 M file ids. An integral value below 2^53 prints
  as its digits in every ES6 case (`k <= n <= 21`), so it is written with an integer formatter instead of the ES6
  shortest-round-trip formatter. marshal 0.63 -> ~0.14 s.
- **`toFileIdListId`**: Go maps the set's keys in random order, which only matters for a path that gets a new id;
  the port sorts so that new ids are deterministic, but only the paths without an id (paths with an id keep it, so
  the result is the same). The id list itself is the map key.
- The 15 M path -> id lookups for sets whose paths all have ids run on the worker pool against the read-only id map;
  a set with a path that has no id goes through `to_file_id_list_id` in the original order, so new ids are handed
  out exactly as before.
- `ensurePackageJsonsForState` realpaths run on the worker pool (both lists are sorted afterwards). 0.10 -> 0.04 s;
  this is in every warm run, including the steady one.
- Build from snapshot 1.33 -> ~0.14 s. The whole write ("Emit time" of the warm run) 2.0 -> ~0.35 s.
- Read: `build_info_to_snapshot` shares one reference set per id list between the files that use it, as Go does
  (`Arc<Set<Path>>`, `ReferenceMap::store_shared_references`); the JSON parser reads integers of up to 15 digits
  without the float parser (bit-identical). ~0.33 -> ~0.30 s. The rest is the generic `Value` tree; not pursued.

## Results (at #37)

Base `ff92b7f` (origin/main), new = this branch, tsgo-ref; 4 checkers; interleaved base/new/tsgo per round,
median of 3 rounds (range in parentheses); load 10-45 from other agents during the runs. Seconds; peak RSS in GiB.

| scenario | phase | base | new | tsgo |
| --- | --- | --- | --- | --- |
| warm, no edit (writes) | **total** | 8.60 (6.93-13.53) | **1.75** (1.74-1.89) | 5.13 (4.84-5.25) |
| | changes compute | 3.61 | **0.32** | 1.54 |
| | emit (tsbuildinfo write) | 2.17 | **0.33** | 1.99 |
| warm, no edit, steady | **total** | 5.07 (4.82-6.11) | **1.56** (1.41-1.60) | 3.28 (3.03-3.34) |
| one leaf edit | **total** | 8.99 (8.25-9.60) | **3.34** (3.33-3.58) | 7.87 (7.81-11.96) |
| cold, `--incremental false` | **total** | 8.35 (7.37-9.19) | 7.68 (7.09-9.21) | 20.93 (20.75-23.10) |

Single-threaded (`--singleThreaded`), steady warm run: changes compute 4.8 s -> 1.3 s. The tsbuildinfo after every
warm and leaf-edit run: base and new byte-identical, new identical to tsgo's apart from `version` (6/6). Conformance,
fourslash, tsctests and the `tools/oracle/incremental` fixtures were unchanged against base.

Byte comparison against tsgo: `sed 's/"version":"[^"]*"/"version":"V"/'` (docs/EMIT.md), with both files at the same
depth so the relative `tsBuildInfoFile` option matches.
