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

## What still grows

About 40 MB per full rebuild with no reparsed files (add then delete a root file), and about 25 MB per cycle that
rechecks many files (hub edits): program construction and checker data that escape to the thread arenas (resolution
data, snapshots, diagnostics). The PR head grows the same. Freeing it needs the language server's region ownership for
what `new_program` allocates (docs/LSP.md, "Memory plan"), including what the parse workers allocate.

Tried and dropped: carving file regions from slabs of their own (a third thread-local slab pool), on the theory
that file regions sharing slabs with checker and emit regions pin them. No measurable change in RSS growth.
