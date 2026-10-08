# mem-linux-residency-32: the resident slack of a 32-checker run on Linux

Question (2026-10-07): on the 64-vCPU scoreboard machine (Linux x86-64, THP `madvise`, tsrs's default of 32 checkers),
how much of the peak RSS is memory that is resident but holds nothing: the unused part of each thread arena's partly
used 2 MiB huge-page block, mimalloc's per-thread pages and freed memory it has not given back, the touched part of the
512 MiB (virtual) thread stacks. And which of it can be given back or avoided with no output change and no wall-time
cost. On the Mac (16 KiB pages, no THP) that "rest" is 4-7 MiB per checker (notes/mem-per-checker-duplication.md 2a);
on Linux nobody had split it since `no_thp` (#121) and the 2 MiB arena chunks (#89).

Result in one paragraph: at the peak (the end of the type-check pass, every checker thread still alive) the slack is
57-71 MiB of unused arena blocks (2.1-2.7% of the peak on the large projects, 5.8% on drizzle-orm), 137-177 MiB that
mimalloc keeps resident beyond its live blocks (4.7-6.5%; 100 MiB, 8.2%, on drizzle-orm), and 4-11 MiB of stacks (0.2-0.4%).
The arena part is all recoverable and is gone with #199: a thread that is done allocating hands its arena to the next
thread (97 thread arenas become 33) and a checker that is done trims its block, for -1.4% to -2.1% peak on five
projects and **-6.4% on drizzle-orm** at 32 checkers with no measurable wall-time change at 4, 16 and 32 checkers. The
heap part is mostly free blocks inside pages that still hold live blocks and resident pages of reused, unpurged slices:
a forced purge at the peak finds 4-5 MiB, and every exact give-back tried (collecting the parse workers' or the finished
checkers' heaps) moved the peak by 0-1%. Stacks are too small to matter.

## Method

`TSRS_MEM_SPLIT=1` (`crates/tsrs_core/src/memsplit.rs`, docs/DEBUGGING.md "the memory split") prints, inside tsrs, at
`parse end`, at `check end` and at `exit`:

- thread arenas (a registry of every thread arena, kept only when the stat is on): capacity, used bytes (above each
  chunk's finger), and the resident bytes below the finger, i.e. never handed out, from `mincore` over the current
  chunk's and each retired chunk's unused range;
- the mimalloc heap from `mi_heap_visit_blocks` over every page of every thread: live blocks (`used x block size`),
  initialized blocks (`capacity x block size`) and the resident bytes of the page areas (`mincore`);
- each checker thread's own resident stack (`pthread_getattr_np` + `mincore`), summed;
- `/proc/self/status` and `/proc/self/smaps`, summed into four buckets by address and name: the arena reservation
  (`reserve::BASE_ADDR` + 32 GiB), thread stacks (`[stack]` and the 256 / 512 MiB anonymous mappings), the heap and
  other anonymous memory, and file-backed pages.

`check end` is taken behind a barrier in the type-check pass: every checker has finished its files and no thread has
exited yet. `tools/perf/slackprobe.py` (via `slackprobe.sh` on `.depot/workflows/perf-probe.yml`) runs one split run
per cell with a sampler reading `/proc/<pid>/status` and `smaps_rollup` every 10 ms, then interleaved timed reps
(wall from `wait4`, peak RSS, phases). The sampled RSS peak and the `check end` RSS agree within 0-3 MiB on every
project (t3code-server: peak sampled at 1.48 s of a 1.49 s run; formbricks-web 0.64 of 0.67 s; vscode 0.59 of 0.61 s),
so `check end` is the moment of the peak. On drizzle-orm the peak sample came after the pass, but the RSS there is
within 1 MiB of `check end`.

Runner: Depot `depot-ubuntu-24.04-64` (AMD EPYC 9R45, kernel 6.12, THP `enabled`/`defrag` = `madvise`),
`cargo build --release`. Projects as in the bench (`bench/projects.json`).

## 1. The split at 32 checkers

Probe mt2fbzqgr3, base binary b5d5d87 (main 4f43c94 plus this note's inert stat), `check end`, MiB. "Heap retained"
is the heap bucket's RSS minus the live blocks, split into what lies inside the live pages' areas and outside them.

| | t3code-server | formbricks-web | vscode | drizzle-orm | supabase-studio | cal-diy |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| RSS at the peak | 2,893 | 2,981 | 2,717 | 1,216 | 2,358 | 2,721 |
| arena used | 1,960 (67.8%) | 1,616 (54.2%) | 1,778 (65.4%) | 739 (60.8%) | 1,312 (55.6%) | 1,456 (53.5%) |
| arena unused, resident | **66.8 (2.3%)** | **71.3 (2.4%)** | **63.7 (2.3%)** | **71.0 (5.8%)** | **64.1 (2.7%)** | **57.5 (2.1%)** |
| heap live | 712 (24.6%) | 1,094 (36.7%) | 706 (26.0%) | 287 (23.6%) | 821 (34.8%) | 1,007 (37.0%) |
| heap retained | **137.3 (4.7%)** | **171.8 (5.8%)** | **146.2 (5.4%)** | **100.0 (8.2%)** | **140.1 (5.9%)** | **177.3 (6.5%)** |
| - free blocks in initialized pages | 62.5 | 92.8 | 72.4 | 42.4 | 61.1 | 86.5 |
| - resident beyond initialized blocks (headers, reused slices) | 50.3 | 41.4 | 45.3 | 39.4 | 39.7 | 44.4 |
| - outside live pages | 24.5 | 37.6 | 28.5 | 18.2 | 39.3 | 46.5 |
| stacks | 4.4 (0.15%) | 10.9 (0.37%) | 5.9 (0.22%) | 4.0 (0.33%) | 4.8 (0.20%) | 5.9 (0.22%) |
| file-backed (binary, libraries) | 14.4 | 16.7 | 16.6 | 14.6 | 16.9 | 16.8 |
| AnonHugePages | 1,958 | 1,604 | 1,768 | 716 | 1,292 | 1,440 |
| thread arenas | 97 | 97 | 97 | 97 | 97 | 97 |

The rows add up to the RSS within 0.3 MiB (the arena reservation's RSS minus used and unused-resident bytes is
0.2-0.3 MiB). At 4 and 16 checkers (probe cfszpm2rks, t3code-server / formbricks-web / vscode): arena unused resident
31.6 / 36.8 / 39.7 and 47.6 / 50.9 / 45.7 MiB, heap retained 71.9 / 76.7 / 90.0 and 99.6 / 110.6 / 128.4 MiB
(vscode at 4 and 16 checkers has file regions: leaf freeing is on up to 16).

### 1a. The arena blocks

Every thread arena's chunks after its 1 MiB first chunk are whole 2 MiB-aligned blocks advised `MADV_HUGEPAGE`
(`reserve::alloc_chunk`), and the bump finger moves down through them, so the block under the finger is one huge page,
resident in full, and the part of it below the finger holds nothing. A 32-checker run creates 97 thread arenas: the
main thread, 32 parse workers (`PARSE_THREAD_CAP`), 32 checker-creation threads and 32 type-check threads
(`run_work_group` spawns new threads per pass; #190 reverted the overlaps, so creation and checking are two threads per
checker). By owner (probe cfszpm2rks, t3code-server / formbricks-web / vscode; drizzle-orm from g8z8xklqhb):

| arenas | t3code-server | formbricks-web | vscode | drizzle-orm |
| --- | ---: | ---: | ---: | ---: |
| 32 parse workers (idle after the load, alive) | 33.3 | 30.9 | 36.1 | 29.9 |
| 32 checker-creation threads (exited) | 0.8 | 0.0 | 0.4 | 14.4 |
| 32 type-check threads | 27.8 | 34.3 | 35.5 | 35.5 |
| retired chunks | 0.2 | 0.5 | 0.6 | 0.3 |

About 1 MiB per arena whose current chunk is a huge-page chunk, as expected for a finger uniformly placed in its
block. Creation threads stay in their 4 KiB-page first chunk (< 1 MiB used) except on drizzle-orm, where creating a
checker allocates 4.5 MiB and reaches a huge-page chunk.

### 1b. The heap

mimalloc v3 with `no_thp` faults its heap in 4 KiB pages, so a partly used 64 KiB page costs only its touched part. What
it keeps beyond the live blocks:

- **Free blocks in initialized pages** (62-93 MiB): each allocating thread has a partly filled page per size class
  plus the holes freed blocks leave in pages that still hold live blocks. 7.4k-12k pages. A collect frees only pages
  with no live block.
- **Resident beyond the initialized blocks** (39-50 MiB): page headers and, mostly, pages carved from slices that an
  earlier page freed and mimalloc did not purge yet: v3's purge delay is 1,000 ms (`options.c`), longer than these runs,
  so a reused slice keeps its old resident pages past the new page's capacity.
- **Outside live pages** (18-47 MiB): freed slices not purged, allocator metadata, glibc's own allocations. A forced
  purge at the peak (`TSRS_MEM_SPLIT=purge`, `mi_collect(true)` once every checker is done; probe g8z8xklqhb) gave back
  4.7 MiB on t3code-server and 3.8 MiB on formbricks-web: the freed but unpurged memory at the peak is small, because
  the heap grows through the whole check (67-347 MiB live at parse end, 287-1,094 at the peak) and reuses what is
  freed.

At `parse end` the heap already retains 32-77 MiB, 27-43 MiB of it inside the pages' areas.

### 1c. Stacks

Checker threads touch 0.1-0.3 MiB of their 512 MiB stacks (3.1 / 9.8 / 4.6 MiB for 32 threads on t3code-server /
formbricks-web / vscode, measured by each thread), the 32 parse workers 1.0-1.3 MiB in all. The virtual size costs
nothing; a smaller stack would save at most 0.1-0.4% of the peak. (notes/bun-check-memory.md listed Bun's 3 MB task
stack with a reader's guess of "tens of MB" for tsrs; it is 4-11 MiB.)

## 2. Prototypes

Every give-back was exact: nothing that an object can reach was freed or moved. Throwaway switches (`TSRS_SLACK`) in
one binary, interleaved; peak and paired wall change against the same binary without a switch, at 32 checkers.

- `parse-arena`: each parse worker, once the program is loaded (`worker_pool().broadcast` before checker creation),
  `madvise(MADV_DONTNEED)` on the never-used rest of its block.
- `parse-heap`: each parse worker `mi_collect(true)` at the same point (frees its empty pages and purges every arena's
  freed memory).
- `check-arena`: a type-check thread whose queue ran dry trims its block the same way (#199's trim).
- `check-heap`: the same thread `mi_collect(true)`.
- `handoff`: threads that are done allocating hand their arena to the next thread (#199's handoff).

Probe g8z8xklqhb (b5d5d87, 5 reps):

| variant | t3code-server peak / wall | formbricks-web | vscode | drizzle-orm |
| --- | --- | --- | --- | --- |
| parse-arena | -0.8% / +1.5% | -1.2% / +1.1% | -0.8% / +0.1% | -4.5% / -0.8% |
| parse-heap | -0.2% / -2.6% | -0.9% / -2.6% | -0.6% / -0.1% | -1.0% / -1.7% |
| check-arena | -1.1% / +0.4% | -1.0% / -0.2% | +0.1% / +0.5% | -3.3% / +0.2% |
| check-heap | 0.0% / -3.0% | -0.5% / -1.6% | +0.2% / -1.0% | -0.7% / -0.4% |
| all four | -2.1% / +0.2% | -3.1% / -3.2% | -1.8% / -0.8% | -5.9% / +5.5% |

Probe ztsnkhbqll (bb7ed2a, 10 reps):

| variant | t3code-server peak / wall | formbricks-web | vscode | drizzle-orm |
| --- | --- | --- | --- | --- |
| handoff | -0.9% / +0.4% | -1.0% / +0.1% | -1.2% / +0.5% | -4.8% / -0.7% |
| handoff + check-arena | -2.0% / +0.7% | -1.9% / +0.5% | -1.2% / +1.9% | -7.7% / -1.7% |
| handoff + check-arena + parse-heap | -2.7% / -0.2% | -2.0% / -0.0% | -2.4% / +2.2% | -6.9% / +0.6% |
| all four trims (no handoff) | -2.2% / -0.3% | -2.4% / -0.9% | -2.2% / +2.0% | -5.5% / -0.8% |

The 5-rep wall cells move by 2-3% either way on the same binary (the drizzle-orm runs take 0.2 s); the 10-rep
vscode +1.9% / +2.2% did not repeat in the 10-rep run of the final code below (+0.3%).

## 3. Landed: #199

Threads that are done allocating give their arena back (`tsrs_core::ptr::release_own_arena`: the end of every
`run_work_group` thread, and each parse worker at the start of checker creation), and the next thread that needs an
arena takes it and continues below its finger. A type-check thread whose queue ran dry trims the rest of its block
(`tsrs_core::arena::trim_own_arena_tail`). 97 thread arenas become 33, and at the peak the unused-resident row of
section 1 is 0.0-2.2 MiB.

Probe mt2fbzqgr3 (cd57aa0 = #199 plus the stat, against b5d5d87), 10 reps at 32 checkers, 5 at 4 and 16:

| project | 32: peak | 32: wall (paired) | 16: peak | 16: wall | 4: peak | 4: wall |
| --- | --- | --- | --- | --- | --- | --- |
| drizzle-orm | 1.170 -> 1.095 GiB (**-6.4%**) | 0.183 -> 0.180 s (-1.4%) | -3.5% | -0.4% | -0.4% | -1.2% |
| t3code-server | 2.785 -> 2.728 (-2.0%) | 1.450 -> 1.447 (-0.4%) | +0.1% | +1.2% | -0.4% | +0.9% |
| formbricks-web | 2.873 -> 2.812 (-2.1%) | 0.655 -> 0.652 (-0.3%) | -0.8% | +0.5% | -0.3% | -1.1% |
| cal-diy | 2.622 -> 2.570 (-2.0%) | 0.590 -> 0.600 (+0.1%) | -1.3% | -0.1% | -0.3% | -0.8% |
| supabase-studio | 2.284 -> 2.240 (-1.9%) | 0.580 -> 0.579 (+0.1%) | -1.3% | -0.9% | -0.5% | -0.6% |
| vscode | 2.650 -> 2.612 (-1.4%) | 0.599 -> 0.598 (+0.3%) | -0.8% | -1.3% | -0.3% | -1.1% |

Against `bun check` at its default (README table of 2026-10-07): drizzle-orm 1.15 vs 1.01 GiB becomes about 1.08.

Caveat of the trim: `MADV_DONTNEED` on part of a huge page splits its mapping. RSS (and `ru_maxrss`, the scoreboard's
number) drops at once, but the kernel frees the physical subpages only when it splits the huge page itself, under
memory pressure (Documentation/admin-guide/mm/transhuge.rst, "deferred split"). The handoff is plain reuse. Of the
-6.4% on drizzle-orm, the handoff is about -4.8 points and the trim the rest (ztsnkhbqll); without the trim no project
reaches 5%.

## 4. Rejected, with the reason

- **Trimming the parse workers' blocks** (`parse-arena`): the handoff saves the same 31-36 MiB by filling those blocks,
  with no system call and no huge page split.
- **Collecting the parse workers' heaps at parse end** (`parse-heap`): -0.2% to -1.0%, inside the run-to-run spread on
  two of four projects, and it needs a hook into the allocator from the compiler crate. Below the bar.
- **Collecting a finished checker's heap** (`check-heap`): 0 to -0.5%. The heap's slack at the peak is free blocks in
  pages that still hold live blocks, which a collect does not touch; the forced purge found 4-5 MiB.
- **Purging the heap on a schedule**: worth at most those 4-5 MiB at the peak; `MIMALLOC_PURGE_DELAY=0` costs 19% check
  time (notes/mem-no-thp.md), and runtime knobs are out of scope.
- **Smaller thread stacks**: 4-11 MiB in all (section 1c).
- **One arena for the thread that creates a checker and the thread that checks with it**: part of the handoff; alone it
  saves what the creation threads' arenas hold, 0.0-0.8 MiB on the large projects and 14.4 MiB on drizzle-orm.
- **4 KiB pages for short-lived arenas' first chunks**: they already have them (the 1 MiB first chunk is not advised);
  only drizzle-orm's creation threads grow past it, and the handoff covers those.
- **Switching a running checker to a finished checker's arena when its chunk is full** (instead of the trim): would fill
  the finished checkers' blocks without `madvise`, but a block freed after the switch goes onto another arena's free
  list (the debug check in `arena::free_block` forbids it) and the scope stack must not name the old arena; worth at
  most the trim's 0-2.9 points. Not built.

## 5. What is left

- The heap's 4.7-6.5% (8.2% on drizzle-orm): allocator fragmentation per thread and unpurged reused slices. Allocator policy is out of scope;
  fewer heap-allocating threads is the other lever (the parse pool is already capped at 32, notes/mem-no-thp.md).
- The arena's used bytes (54-68% of the peak) are the per-checker type graphs (notes/mem-per-checker-duplication.md).

## Reproduce

```sh
cargo build --release --locked -p tsrs_cli
cd <project> && TSRS_MEM_SPLIT=1 tsrs -p <tsconfig> --noEmit --incremental false --pretty false --checkers 32 2>&1 >/dev/null
TSRS_MEM_SPLIT=purge tsrs ...   # also purge mimalloc's freed memory after `check end` and print again
# On the 64-vCPU runner, a branch against a base commit, splits and interleaved reps:
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref <branch> \
  --input projects=t3code-server,formbricks-web,vscode,drizzle-orm --input script=tools/perf/slackprobe.sh \
  --input probe_args="--base-ref <commit> --split --checkers 4,16,32 --rep-checkers 4:5,16:5,32:10 --variant 'base:BIN=\$BASE_BIN' --variant 'fix:'"
```

Probes: cfszpm2rks (the split at 4, 16, 32), g8z8xklqhb and ztsnkhbqll (the prototypes, on branch
mem/linux-residency-32 at b5d5d87 and bb7ed2a), mt2fbzqgr3 (#199 against its base).
