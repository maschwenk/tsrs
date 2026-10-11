# perf-icount-alloc: heap allocations and copies on a large private monorepo at 8 checkers

Goal (2026-10-11): fewer user-space instructions for a cold `--noEmit --incremental false --pretty false --checkers 8`
check of a large private monorepo, by removing heap allocations, reallocations and copies that the checker makes and
drops almost at once, with identical output. Base 98d8d60e. This round covered the allocator (`mi_malloc`,
`mi_free`, `_mi_theap_realloc_zero`, `RawVec` growth) and the `memcpy`/`memmove` under it. Symbol-table lookups were
a separate work item and were left alone here.

## Result

Exact user-space counts from the OrbStack Linux VM (DynamoRIO). Each step is a separate build:

| | 98d8d60e | branch | change |
| --- | ---: | ---: | ---: |
| `--checkers 8 --checkerAssignment locality` | 322.67-322.74G | 313.200G | -2.9% |
| `--singleThreaded`, `RAYON_NUM_THREADS=1` | 192.423G | 187.091G | -2.8% |
| `--checkers 8` (work stealing, 1.7% noise) | 334.8-340.5G | 328.21-329.01G (2 runs) | about -2.9% |

The same commits measured on the Mac with `/usr/bin/time -l` (user + kernel), base and branch run one after the
other:

| | 98d8d60e | branch | change |
| --- | ---: | ---: | ---: |
| `--checkers 8`, median of 3 | 366.20G | 350.38G | -4.3% |
| locality, 2 runs | 348.50-349.66G | 336.76-338.06G | -3.4% |
| single-threaded | 214.67G | 206.45G | -3.8% |
| peak RSS at 8 checkers | 6.39-6.44 GB | 6.40-6.41 GB | unchanged |
| vscode single-threaded | 87.83G | 86.29-86.69G | -1.5% |
| t3code-server single-threaded | 42.08G | 41.02-41.24G | -2.3% |
| heap allocations, single-threaded (`MIMALLOC_SHOW_STATS`) | 113.4M | 47.6M | -58% |
| heap allocations, locality | 183.9M | 69.8M | -62% |

The Mac shows a larger drop because its count also includes kernel instructions and the system's memory paths. Wall
time was not measured, because other agents were building on the machine.

Identity: the diagnostics md5s of vscode, cal-diy, t3code-server and supabase-studio (8 checkers and single-threaded)
matched base after every kept commit. The workload prints nothing, before and after. Conformance (`tsrs-test run
--suite all`): 13,458 passing, and the pass list contains every base entry. `tools/regressions.sh`: 29/29.
`tools/lint/ratchet.py`, `tools/lint/source.py` and `cargo check --workspace` with `-D warnings` are clean.
`cargo test` passes for tsrs_ast, tsrs_checker, tsrs_core, tsrs_module and tsrs_ls.

## Method

- Instructions come from `/tmp/icount/linux` (VM, DynamoRIO): locality mode (0.02% run to run) and single-threaded
  (1e-7). Each step was a snapshot of the worktree built in the VM. VM counts were compared only with other VM
  counts. Mac counts (about 0.35% noise in locality mode) are in the commit messages for reference.
- Allocation counts: the `total` line of `MIMALLOC_SHOW_STATS=1` on the release binary (arena allocations are not
  in it). Printing the stats adds about 2% instructions, so stats runs and instruction runs were always separate.
- Allocation sites: the alloc-profile build (`--features alloc-profile`, `TSRS_HEAP_PROFILE=count`, every 1024th
  allocation), run at 8 checkers in locality mode, with a local patch that was not committed. As built, the
  profiler drops the first three frames, which loses the allocating function after inlining
  (notes/perf-heap-churn.md). The patch keeps every frame (`TSRS_HEAP_PROFILE_SKIP=0`) and writes the raw stacks
  (`TSRS_HEAP_PROFILE_RAW=<file>`: load address, then group, count and return addresses per site). A script resolves
  them with `llvm-symbolizer --inlining` against a `dsymutil` dSYM. It charges each site to the first frame outside
  the standard library, hashbrown, `tsrs_core::ptr` and the allocator shim, keyed by function and file:line, two
  frames deep. At base the top sites were
  conditional-type arguments (12.5M of 163M sampled allocations), intersection type sets (13.2M plus 5.6M
  reallocs), template-literal text (13M), symbol-table growth (10.9M) and instantiation lists (10.4M).
- Allocator time by caller: samply on a line-tables build. Samples whose leaf is a `mi_*` function, or
  `libsystem_platform` under `realloc`, were charged to the first frame outside those libraries. At base
  the allocator took 1.13 s of 43.7 s CPU at 8 checkers; on the branch it takes 0.57 s of 40.0 s.

## What landed

Locality and single-threaded counts are VM counts against the commit before. Allocations are single-threaded
counts.

| commit | locality | single-threaded | allocations |
| --- | ---: | ---: | ---: |
| #272 (perf/heap-churn, 9 commits plus 2 notes, cherry-picked with `-x`) | 322.67-322.74 -> 321.310G (-0.43%) | 192.423 -> 191.133G (-0.67%) | 113.4 -> 93.2M |
| conditional-type arguments and intersection type sets in pooled lists | -0.33% | -0.31% | -> 76.5M |
| template literal text in a pooled String | -0.72% | -0.41% | -> 71.2M |
| borrows: constituents, inference matches, base-constraint stack, lazy member walk | -0.38% | -0.38% | -> 63.5M |
| pooled scratch sets and lists in property synthesis and walks (with the property-count pool) | -0.78% | -0.69% | -> 57.2M |
| union construction in a pooled list | -0.04% | -0.04% | -> 54.1M |
| no copies of variances, export names, ambient-module keys, template texts | -0.16% | -0.18% | -> 50.0M |
| pooled lists for inference contexts, indexed access over unions, tail recursion | -0.07% | -0.04% | -> 47.6M |
| union front-cache key compare without `bcmp` | -0.05% | -0.07% | |
| link-store chunks written in place in the arena | -0.02% | -0.005% | |

- #272 applied on 98d8d60e with three conflicts. The previous round had already taken the property-access half of
  `get_accessed_property_name` and the `exclude_properties` borrow, and 98d8d60e had moved number formatting to
  `dragonbox_ecma`. `Number::string_equals` therefore compares non-integers against the formatted buffer without a
  `String`, and `Checker::fork` needed the new pool field. On this workload #272 is -0.43%
  locality and -0.67% single-threaded on its own.
- The pools are per-checker free lists of empty containers: `free_type_lists` (existing), new `free_strings`,
  `free_name_sets`, `free_property_counts`, `free_index_lists`, `free_inference_lists` and `free_meaning_maps`. A
  user pops a container, or takes a new one if the list is empty, and pushes it back cleared. A recursive call
  takes a container of its own, so nesting needs no reasoning. Containers above 256 entries (lists) or 1024
  (sets, maps) are dropped instead of kept. Functions with many early returns became a wrapper that pops the
  container, calls a worker with `&mut` access to it, and pushes it back.
- Borrows replace copies only where nothing can change the borrowed data while it is read: constituents of unions
  and intersections (never changed after creation), stored variance lists (set once, `&'static`), and declared
  members walked by position. `SymbolTable::entry` already documents that a walk by position sees what a snapshot
  would, as long as nothing is deleted, and the tests in that walk add and replace nothing either.
- `getResolvedBaseConstraint`'s stack of recursion identities is now a list threaded through the callers' frames
  (`BaseConstraintStack`), which removes one copy per level. It is at most 50 deep, as before, and
  `contains` is only asked at depth 10 to 49.

## Measured and rejected

- Alias type arguments of `AliasArg::Pending` held as `Cow`: the alias's own list when unchanged, else a pooled
  buffer handed back after the type is made. Template-literal instantiation also used `instantiate_types_changed`.
  This was +0.20% locality on top of the property-synthesis commit (314.295 -> 314.910G), and +0.26% (315.118G) with
  the new helpers `#[inline(never)]`, although it removes allocations. I did not find out why.
  The evaluation order and the instantiation calls are the same, so the cost is not in the type work.
- Symbol-list and signature-list pools for `instantiateSignature`'s parameters and
  `resolveAnonymousTypeMembers`' signatures, `getNamedMembers` into a pooled buffer, `alloc_slice_concat` for a
  structured type's signatures, and the object-literal all-properties table presized to the literal's property
  count. Together: -0.012% locality (314.295 -> 314.257G), inside the VM's spread, for about 5M fewer
  single-threaded allocations. Each of these lists is allocated once and copied into the arena right away, so a
  pool's push and pop cost about as much as mimalloc's fast path.
- What paid in this round: hash tables (an `IndexSet` per synthesized union property, a names-seen `FxHashSet`
  per lazy walk, an `OrderedMap` per `somePropertyReducesToNever`, a `String` per export name in a hash set), chains
  of growth (`realloc` and copy: intersection sets, template text, symbol tables), and copies of lists that were
  only read. A single small Vec that is filled once and dropped costs little to allocate.

## Found, not pursued

- Symbol-table growth is now the largest allocator item: `EntryVec::set_capacity` takes 0.14 s of 40 s CPU at 8
  checkers, plus 0.026 s of `memcpy`. At 8 checkers that is 4.8M first allocations and 4.7M reallocations. The
  tables that grow are union property caches, lazy declared-member caches, mapped-type members, binder tables and
  inherited members. Their final size is unknown when they are created, and the 1, 2, 4, 8, then +50% growth is
  tuned for memory (symbol.rs), so changing it trades peak RSS for instructions. Not tried.
- The front end builds a `DefaultResolver` (three 64-shard `SyncMap`s) for every file in `source_file_meta_data`
  to ask one package-scope question. That is about 0.027 s of allocator CPU at 8 checkers.
- Inference copies its candidate lists (`LazyVec::to_vec`, about 1M at 8 checkers) as a snapshot against pushes
  during the walk. Dropping the copy needs a proof that nothing pushes.
- `memcpy` / `bcmp` callers left at 8 checkers: `same_text` in symbol-table key matching (0.13 s, part of the
  symbol-table lookup work, and an inline compare was rejected in round 2), string-literal cache key compares
  (0.05 s), `setStructuredTypeMembers`' copies into the arena (0.04 s) and `compare_type_names` (0.03 s).
