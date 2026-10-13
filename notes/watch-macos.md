# watch-macos: `--watch` and `-b --watch` on macOS (FSEvents, kqueue)

The first run of the watch on a Mac (the brief was PR #365's tools/watch/MACOS.md, since removed). Machine: MacBook Pro, Apple M3 Max (14 cores,
96 GB), macOS 26.6.2 (25G83). Comparison: tsgo nightly 7.0.0-dev.20260707.2 (`@typescript/native-preview`), not a
pinned build. Project: vscode at 9cf0128b9822 (9.4k files under `src/`), as in the brief.

## Results (vscode, same Mac for every column)

| Case | tsrs --watch | tsgo --watch | tsrs -b --watch (incremental) | tsgo -b --watch |
|---|---|---|---|---|
| initial build | 2.0 s | 9.8 s | 3.2 s | 8.9 s |
| first comment edit | 48 ms, 1 cycle | 13.5 s, 1 cycle | 3.2 s | 13.4 s |
| repeat comment edit (median of 8; tsgo of 4) | 59 ms | 1.49 s | 264 ms | 1.48 s |
| new export in a hub file | 1.3 s | 11.7 s | 2.8 s | 11.6 s |
| type error in a leaf file | 46 ms | 13.1 s | 3.2 s | 13.1 s |
| detected | 13-16 ms | 14 ms | 12-16 ms | 108-137 ms |
| RSS (footprint) | 2.1-3.3 GB | 10.4-14.8 GB | 2.3-3.4 GB | 10.6-17.3 GB |

tsrs numbers are at the branch tip after rebasing on feat/watch e4c4c8ca (with its signature job); tsgo's are from
the same session. "detected" is FSEvents' own delivery: a minimal C client sees the first event 11-13 ms after the
write whatever the latency or flags (0, 1 ms, 10 ms, 50 ms; with and without NoDefer and FileEvents), and
FSEventStreamFlushSync does not get it sooner (the delay is before the event reaches the stream). The pipeline after
it (debounce, cycle start, status line) adds under 1 ms.

- `watchcheck.py`: ALL SAME, one cycle each, in `--watch`, `-b --watch`, and `--watch` on the kqueue fallback.
- `editduringjob.py`: ALL SAME; comment edits 37-65 ms, declaration changes 1.7 s. The signature job takes 546 ms
  after the initial build (1.15 s on Linux).
- `savepatterns.py` (both modes, vscode and a 300-file project): one cycle and the fresh build's diagnostics for an
  in-place write, truncate / 200 ms / write, atomic rename, rename away + write + delete backup, a 200-file formatter
  rewrite and its restore, a git checkout of 200 files and back, and a case-only rename and back. A writer that
  writes four chunks 100 ms apart starts 2 cycles; the second has the right result (see "Not fixed").
- Idle: 0 interrupt wakeups and 0 ms CPU in 30 s once the signature job is done (proc_pid_rusage; `top`'s IDLEW does
  not count condition-variable timeouts, so it showed 0 even when the watch woke once a second).
- Ctrl-C: exits in 88 ms idle, 23 ms during a cycle.
- tsctests: tscWatch 39 / 42, tsbuildWatch 57 / 65, as on Linux.

## Problems found and fixed

| Problem | Cause | Fix | Test |
|---|---|---|---|
| 2-3 cycles per save on vscode | FSEvents queue had no QoS; with the cores busy (the cycle's checkers, or Spotlight on the efficiency cores) fseventsd dropped events for the client (UserDropped + MustScanSubDirs), which the watch turns into a full rebuild | `QOS_CLASS_USER_INITIATED` queue | `fsevents_load.rs` (10k writes, all cores busy; 3/3 fail without) |
| Truncate-then-write save: 2 cycles, the first on an empty file | no close event on FSEvents/kqueue | an empty file holds the callbacks like an unclosed one on inotify | `write_settling_fsevents.rs`, `kqueue_macos.rs` |
| Formatter / codegen burst: 2 cycles | leading edge fired on the first ~12 ms of the burst | a batch that changed two existing files waits for 50 ms of quiet | `write_settling_fsevents.rs` ([2, 18] -> [20]) |
| Case-only rename: stale program, missing TS1149 | the path is case-folded, so the one-file fast path reused the old spelling | an event spelled unlike the program's file rebuilds; `-b`: the file names reload | `watch_macos.rs`, `savepatterns.py` |
| Edits through a symlinked directory never seen (tsgo too) | a recursive watch does not follow a symlink below its root | such directories get their own watch | `watch_macos.rs` (both modes) |
| Edit during the first build missed (tsgo too) | watches start after the build | files changed since they were read are queued | `watch_macos.rs` (both modes) |
| Own outputs started a second full rebuild per save (tsgo too) | outDir inside `include: ["**/*"]` | events for the config's outputs are dropped | `watch_macos.rs` |
| chmod / setup writes started cycles | FSEvents delivers ~12 ms late, metadata changes report the file | events for files unchanged since parsed are dropped | `watch_macos.rs` |
| 1 wakeup a second while idle | `run_loop`'s 1 s wait timeout | no timeout; `stop` notifies under the lock | `watch_macos.rs` (proc_pid_rusage) |
| A new directory beside the project ran a cycle | any directory under any watch counted | only wildcard matches, lookup paths and node_modules count | `watch_macos.rs` (both modes) |
| kqueue: past 256 files, edits silently missed | Rust keeps the shell's 256-descriptor soft limit (Go raises it) | raise it like Go; out of descriptors is an error naming `ulimit -n` | `kqueue_macos.rs` |
| A watch error printed nothing at all | the output is buffered and only the status line flushed it | the report goes out after the error | `watch_macos.rs` |
| `-b --watch` repeat edit 578 ms, detected 121 ms | wildcard reload every cycle, 10k realpaths per cycle, every file reread | skip the reload for edits, cache realpaths, reuse files by mtime | `watchcheck.py`, `savepatterns.py`, tsctests |
| A batch to 284 callbacks took 133-301 ms under the locks (stalled new watches 49 ms) | per-callback snapshot and case-folded rebase of every event | index by parent directory, outside the locks | `watcher_test.rs` (differential) |

## Evaluated and rejected (do not redo)

- `F_SETLEASE` read lease as a "someone has it open for writing" probe: EPERM for an unprivileged process.
- libproc `proc_listpidspath` to find writers: 182 ms per path.
- kqueue `NOTE_CLOSE_WRITE`: does not exist on macOS (only NOTE_FUNLOCK, NOTE_LEASE_*).
- FSEventStreamFlushSync to see whether a burst continues: returns in 0.03 ms but cannot deliver events younger
  than fseventsd's ~11 ms.
- NoDefer + 50 ms latency (bun's fs.watch): same first delivery, half the callbacks in a burst, but the first batch
  then holds one file, which defeats the burst detection above.
- A fixed settle window (watchman's 20 ms): would add to every save; the brief rules it out.
- Event IDs (`sinceWhen`) for stream rebuilds: Go uses `SinceNow` too, and the port starts the new stream before it
  stops the old one, so a rebuild loses nothing. The measured add-a-watch cost (49 ms) was lock contention with the
  callback delivery above, not the stream.

## Not fixed

- A writer that writes one file in pieces with pauses (4 x 100 ms): 2 cycles. Its first event shows partial content,
  which nothing tells apart from a finished save without waiting.
- Memory: see "Memory" below.
- An overflow (dropped events) still rebuilds every callback of a shared watch, whatever path it names. With the QoS
  fix none occurred in these runs; parallel tests that churn `$TMPDIR` can still trigger one.
- Emitted outputs under a consolidated stream root still reach the process (now cheap); FSEventStreamSetExclusionPaths
  for outDir is untried.

## Shared code (Linux runs needed)

debounce.rs (cfg only), watcher.rs `trigger_callbacks` (the index), event.rs, watchmanager `run_loop`/`stop`,
watcher.rs and orchestrator.rs cycle logic (case rename branch is dead on Linux; the others apply there too and fix
the same tsgo-shared bugs). The Linux reference numbers and `write_settling.rs` should be rerun.

## Memory: what the remaining growth over full rebuilds is

After notes/watch-memory.md's fixes, comment edits (the one-file cycle) are flat: footprint 1.51-1.57 GB over 100
edit/revert pairs, RSS +45 MB (0.2 MB per edit, the id space). Hub-export round trips (full rebuilds) were not:
`footprint` put the thread arenas (untagged VM_ALLOCATE) flat at ~2.05 GB from the first round trip, and mimalloc's
heap (VM tag 100, which footprint names "IOAccelerator") at 822 -> 1,926 MB over 6 round trips, then ~30 MB more per
round trip.

With the per-cycle heap report (docs/DEBUGGING.md), 8-10 cycles of hub round trips on vscode:

- The live Rust heap is flat: 542 MB after the first build, 617 after the first cycle, then +4 MB per cycle, and
  the sampled survivors of each cycle are 2-3 MB. Leaving out the profiler's own bookkeeping (its per-site tables and
  sample map, which the counting allocator counts too), 3.7 MB of bytes from 7 cycles survive, 3.1 MB of them the
  current config's `file_names_by_path` map, built once in the first cycle and still in use (the same 3.1 MB after 5
  or 7 cycles, and with the config kept across rebuilds): no product leak shows in the live heap.
- mimalloc's committed memory levels at ~2.46 GB after cycle 3 (the heap's peak during a full rebuild); what grows is
  the resident part of it. mimalloc counted 860 -> 1,012 MB of blocks in use while the program held 617 -> 632 MB:
  blocks the program had freed but mimalloc still counts. The free thread frees what the worker-pool threads
  allocated in the cycle (parsing, emit, the program's data); a cross-thread free only puts the block on its page's
  thread-free list, and the page (and its memory) is not reusable or purgeable until the owning thread collects. The
  pool threads are idle after a cycle, so their pages kept those blocks. `mi_collect(true)` on the free thread alone
  (faafb52d) only collected its own heap.
- Broadcasting `mi_collect(true)` to the pool threads too (after each cycle, ~10 ms on the free thread): mimalloc's
  footprint after 6 round trips 1,415 -> 1,195 MB, and over 12 round trips it levels at ~1.28 GB (rt 4 -> 12:
  +16 MB per round trip and flat at the end, against ~25-30 MB per round trip continuing before). Repeat comment edit
  47-51 vs 44-64 ms over interleaved runs (no difference beyond noise). What remains is mimalloc settling into the
  working set of a full rebuild (committed stays at its ~2.5 GB peak; how much of it is resident levels off).
- Not it: the ~300 threads each cycle starts and ends (checker threads; mimalloc abandons their pages): abandoned pages
  stay at ~6k; `MIMALLOC_PAGE_RECLAIM_ON_FREE=1` (reclaim them on a free) made the heap larger (1,698 MB at 6 round
  trips). Purging on every free (MIMALLOC_PURGE_DELAY=0, MIMALLOC_PURGE_DECOMMITS=1) gives the same footprint as the
  post-cycle collect but slows the cycles (hub edit 1.3 -> 2.0 s).
