# watch-memory: freeing parsed files in watch mode, and what still grows

Measured on vscode (bench commit 9cf0128b, `npm ci`, `-p src`, default checkers, release build, 8-core Linux box)
with a script that drives `tsrs --watch` and compares each cycle's diagnostics with a fresh `tsrs -p src --noEmit`.
"Round trip" is `git checkout` between 9cf0128b and eb84d814 (4,037 files under `src/` differ) and back.

## The leak

Every cycle reparses the changed files into the thread arenas, which are never freed, so each version of every
changed file stayed for the rest of the session. A branch switch that rewrites thousands of files costs about
0.9 GB each time. With compressed pointers every arena shares one 32 GiB address range, so a session that switches
branches long enough aborts:

    tsrs: the arena address range is exhausted (compressed pointers: one 32 GiB address range for every arena; 32403 MiB of it in use).

Before the fix this happened after 16 round trips (about 10 minutes of switching), at 29 GB RSS.

## The fix (crates/tsrs_execute/src/watchmem.rs)

- Each parsed file version gets a registered region of its own, as in the language server (tsrs_project
  parsecache.rs). The text, tree, binder data (`fileregions::bind`, opt-in through `bind_in_file_regions`) and
  lazily filled data (`arena::enter_owner`) live there.
- After each `install_program`, regions whose file is not referred to by the source file cache, the current
  program, the last full build (its processed-file data), or the diagnostics the snapshot keeps
  (`Program::for_each_diagnostic_file`) are freed. `-b --watch` does the same after each cycle over the host's
  cache and the tasks' programs.
- The frees run on one thread in queue order (`Freer`), so a file region goes only after the programs that
  referred to it. This also orders `SharedProgramData::free` after the frees of the programs that shared it, which
  separate free threads did not guarantee.
- `arena::enter_owner` / `enter_table_owner`: once any registered region exists, an object of the unregistered
  scratch region entered on this thread (a watch checker's) keeps its lazily filled data in that region, as when no
  region is registered. Before, it went to the thread arena: +0.45 GB over 80 soak cycles.

## Numbers (RSS after the round trip settles)

| | PR head | with the fix |
| --- | --- | --- |
| `--watch`, 8 round trips | 2.5 -> 16.6 GB, +1.85 GB/round | 2.5 -> 6.2 GB (peak 7.4), ~0.5 GB/round |
| `--watch`, until the 32 GiB range is exhausted | 16 round trips | not reached in 12 (9.2 GB) |
| `-b --watch`, 4 round trips | 2.3 -> 6.9 GB, ~1 GB/round | 2.4 -> 3.8 GB, ~0.16 GB/round |
| initial build | 4.9 s, 2.48 GB | 4.9 s, 2.53 GB (+2.7%: per-file regions) |
| hub-file export edit | 4.6-5.1 s | 4.6-4.7 s |
| comment edit (fast path) | 0.12-0.13 s | 0.12-0.13 s |

Diagnostics matched a fresh build after every checked cycle, and the same scenarios ran clean under
`TSRS_ARENA_POISON=1` until the address range ran out, which happens early in that mode because poisoned memory is
never reused. tscWatch / tsbuildWatch pass lists are unchanged.

## Second round: what grew beside the file regions

Measured with the now-removed allocation profiler (per-site arena counts, the arena address range in use, live regions, slabs and
thread arenas, dumped after each cycle) and the sampling heap profiler with allocation epochs (bytes still live that
were allocated more than five cycles earlier).

- **Thread exit kept slabs** (arena.rs `CurrentSlab`, ptr.rs `ReleaseAtExit`). The bulk of the remaining
  round-trip growth was the address range, not live regions: slabs with one reference left and no live region in
  them. A thread holds a reference to its current slab and owns its arena until it releases them, and threads that
  ended never did; a watch session starts checker and worker threads every build (about 225 slabs of 1 MiB per full
  build). Both now go back when the thread exits.
- **File loader data** (tsrs_compiler programregions.rs): resolutions, include reasons and processed-file data
  went to the thread arenas (about 25 MB per `-b` task build). They go to per-thread regions freed with the data the
  programs since that full build share (`--watch`), or with the task's program (`-b --watch`).
- **Checker diagnostics** (arena.rs `Region::route_diagnostics`, checkerpool.rs `SlotChecker`): diagnostics escape
  the checkers' regions, a few thousand per edit (most are never reported: speculative elaboration, duplicate
  declarations found while merging globals). Each checker's go to a region its program frees; the snapshot copies
  those it keeps into a region of its own at each install (`Program::copy_diagnostics`). The route applies only while
  the checker's own region is the target, so lazily filled file data and program caches keep their old targets. Off
  under the test harness, which reads the previous snapshot (it shares diagnostics with the new one).
- **Per-edit leftovers**: a reused program's file list and resolution data, with its heap copy of the package.json
  cache table (a region the program owns); the replaced incremental program and snapshot (their arena blocks go to
  the free list; `-b --watch` now drops them once the task is built); the two status messages of each cycle.

| vscode, RSS | branch head (8733e222) | with these fixes |
| --- | --- | --- |
| `--watch`, 8 round trips | 3.3 -> 5.8 GB (peak 7.6) | 2.7 -> 2.9 GB (peak 4.8) |
| `--watch`, 100 edit/revert pairs | 2.5 -> 8.4 GB (peak 9.1) | 2.5 -> 1.9-3.2 GB, flat (peak 4.1) |
| `-b --watch`, 4 round trips | 2.87 -> 3.43 GB | 2.82 -> 2.91 GB |
| initial build | 5.2 s | 5.3 s |
| hub-file export edit | 3.7 s | 3.7-3.8 s |
| comment edit | 0.08-0.11 s | 0.09-0.12 s |

Over 160 edit cycles the thread arenas stay at 0.4 MB used (they grew 184 MB before) and the address range in use is
flat. Every checked cycle matched a fresh build, the scenarios ran clean under `TSRS_ARENA_POISON=1` until the range
ran out (poisoned memory is never reused), and the tsctests pass lists match the branch head.

## What still grows

- **The id space.** Node and symbol ids come from process-wide counters that only grow: about 730k symbol ids per
  edit and 5M per full build on vscode. A checker's id-keyed link stores index the id space densely
  (links.rs `IdLinkStore::index`, one entry per 128 ids up to the highest id the checker touches, and every checker
  touches the newest ids), so each new checker's index is a little bigger: the heap grows about 180 KB per edit.
  Ids are 32-bit (`assign_symbol_id` panics past `u32::MAX`), so a session that lasts long enough (about 5,900 edits
  or 850 full builds at these rates) panics. Fixing it means paging the index (memory only) or recycling or widening
  the ids; not done.
- **package.json cache entries** (resolver.rs `get_package_json_info`): `enter_table_owner` puts them in the thread
  arena unless the cache's registered region is current, about 0.7 MB per full build.

Tried and dropped: carving file regions from slabs of their own (a third thread-local slab pool), on the theory
that file regions sharing slabs with checker and emit regions pin them. No measurable change: what pinned slabs was
the threads' own references (above).
