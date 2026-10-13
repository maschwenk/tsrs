# Testing and optimizing tsrs `--watch` on macOS

You are taking over tsrs's watch mode (`--watch` and `-b --watch`) on macOS. It was built and tuned on Linux and
has never run on a Mac, so the macOS file-watching path is unverified. Your job, in this order:

1. Make it correct on macOS.
2. Measure it against the targets below.
3. Optimize until it meets them, without regressing Linux.

Speed and efficiency are the goal: edit-to-report latency, idle cost, and memory. Correctness comes first, because a
fast watch that misses an edit or runs two cycles per save is worse than a slow one.

## Background

- Branch: `feat/watch` in the tsrs repo. It stacks on `perf/parallel-incremental-emit`. Start a branch from
  `feat/watch` for your work.
- File events come from `crates/tsrs_fswatch`, a port of tsgo's `fswatch` package. On macOS, `default()` in
  `watcher.rs` picks FSEvents (`fsevents_darwin.rs`, FFI in `fsevents_darwin_ffi.rs`) and falls back to kqueue
  (`kqueue.rs`).
  - FSEvents runs one shared stream per process, split into 512-path chunks. The stream is rebuilt whenever the set
    of watched directories changes.
  - The stream is created with latency 0.001 s and the flags UseCFTypes | FileEvents.
  - Callbacks run on a GCD serial queue.
- `debounce.rs` batches events the way Go does: a leading edge, then 50 ms quiet / 500 ms max. "File change
  detected" prints on the first event, so the "detected" column in `watchlat.py` measures event delivery latency.
- Write settling is tsrs-only and Linux-only. inotify reports IN_CLOSE_WRITE, so the debounce waits until a file
  being written is closed (at most 1 s idle / 2 s total). FSEvents and kqueue have no close event, so on macOS a
  save that arrives in several writes can start a cycle on a half-written file and then run a second cycle. That
  is the main known macOS gap; see "What to investigate" below.
- Watch programs free the previous program on a background thread and check in per-checker scratch regions. On
  Linux this keeps RSS flat across cycles. Memory behavior on macOS (mimalloc returning pages to the OS, the memory
  compressor) is unmeasured.
- `TS_WATCH_DEBUG=1` makes the watch print what it watches and why each cycle started.

## Setup

```sh
cargo build --release -p tsrs_cli                    # binary: target/release/tsrs
cargo test --release -p tsrs_fswatch                 # the FSEvents/kqueue unit tests have never run on a Mac
cargo test --release -p tsrs_execute tsctests -- --nocapture
```

The tsctests line runs the watch scenario baselines against a mock file watcher, so it is platform-independent.
Linux results are tscWatch 39 / 42 and tsbuildWatch 57 / 65, or 65 / 65 once the build-info signature diagnostic
fix branch is merged. They must match on macOS.

Benchmark project: vscode (about 9.4k files under `src/`). The Linux numbers below come from commit
`9cf0128b9822dcd8a533f08ef7312956a4390301`.

```sh
git clone https://github.com/microsoft/vscode && cd vscode && git checkout 9cf0128b9822dcd8a533f08ef7312956a4390301
npm ci --ignore-scripts          # only the type packages matter
cd src                           # run the scripts from here; they edit vs/base/common/*.ts and restore them
```

For `-b --watch` with an incremental config, create `src/tsconfig.watchinc.json`:

```json
{ "extends": "./tsconfig.json", "compilerOptions": { "incremental": true, "tsBuildInfoFile": "/tmp/watchinc.tsbuildinfo" } }
```

Comparison binary: tsgo nightly (`npm i -g @typescript/native-preview`, then the `tsgo` binary in the
`@typescript/native-preview-darwin-arm64` package). The Linux comparison also used the nightly, not a pinned
commit, so note which version you used.

Both runs report 371 errors on this tree, because some native modules lack types. Only agreement with a fresh build
matters.

## The two scripts in `tools/watch`

- `tools/watch/watchcheck.py <compiler> <projdir>` makes 13 edits under `--watch`:
  - an error added and then fixed;
  - a new file added, fixed, and deleted;
  - a hub export added and then removed;
  - an import broken and then restored;
  - an exported signature changed and then restored.

  After each cycle it compares the printed diagnostics with a fresh `--noEmit` run. It must print `ALL SAME`, with
  `cycles 1` on every line. `EMITCHECK=<outDir>` also compares the emitted files with a fresh emit at the end.
- `tools/watch/watchlat.py <compiler> <projdir>` measures the initial build and four edits. For each edit it prints:
  - detected: how long after the save "File change detected" printed;
  - done: how long until "Watching for file changes" printed;
  - the number of cycles;
  - RSS.

  `REPEAT=5` repeats the edits (use it for memory). `BUILD=tsconfig.watchinc.json` runs `-b <config> --watch`
  instead. `LOG=<file>` saves the output.

Both scripts restore every file they touch, even on failure. Check `git status` in vscode afterwards anyway.

## Reference numbers (Linux, 9.4k files, same machine for both columns)

| Case | tsrs --watch | tsgo --watch | tsrs -b --watch (incremental) | tsgo -b --watch |
|---|---|---|---|---|
| initial build | 4.8 s | 16.6 s | 6.6 s | 16.9 s |
| first comment edit | 5.0 s, 1 cycle | 23.5 s, 2 cycles | 7.4 s | 26.5 s |
| repeat comment edit | 80 ms | 554 ms | 385 ms | 2.76 s |
| new export in a hub file | 3.6 s | 17.7 s | 6.0 s | 20.3 s |
| type error in a leaf file | 5.0 s | 20.8 s | 7.3 s | 25.0 s |
| detected | 0.5-3 ms | | | |
| RSS | 1.7-2.9 GB, flat | 10-15 GB | 2-3.3 GB | |

The first edit after startup is slower than a repeat because it builds the incremental state. A repeat comment edit
reuses the program.

## Targets on macOS

Hard requirements:

- `watchcheck.py` prints ALL SAME, in both `--watch` and `-b --watch`, with `cycles 1` everywhere.
- Exactly one cycle per save for every save pattern listed under "What to investigate". A second cycle doubles the
  wait, and a cycle on a half-written file reports false errors.
- No missed edits, including in projects under `/tmp` (a symlink to `/private/tmp`), on case-insensitive APFS, and
  for edits made through a symlinked directory.
- Idle cost of zero: no CPU and no wakeups while nothing changes. Laptops run on battery, so a watch that wakes up 100
  times a second is a bug even if it is fast.
- RSS flat across `REPEAT=5`, meaning no growth from one round to the next.
- Ctrl-C exits promptly and cleanly.

Performance targets (Apple Silicon; scale if the Mac is much slower than the Linux box):

- detected: under 20 ms. FSEvents latency is 1 ms, so anything much higher points at the stream setup, the GCD
  queue, or the debounce.
- repeat comment edit, done: under 100 ms for `--watch`, under 400 ms for `-b --watch`.
- Every row in the table: at least the same ratio over tsgo that Linux shows (3-7x).
- Rebuilding the FSEvents stream when the watch set changes (a file added or a new directory) adds under 10 ms to that
  cycle and loses no events.

## What to investigate (most likely problems first)

1. Save patterns without close events. Write a small script for each pattern and check for `cycles 1` and correct
   diagnostics:
   - an in-place write (`open(path, "w").write(...)`, what the scripts do);
   - truncate, sleep 100-300 ms, then write the rest (a slow writer);
   - several chunked writes over 300 ms;
   - an atomic save (write `x.ts.tmp`, then rename over `x.ts`; most editors);
   - `git checkout` of a branch touching hundreds of files;
   - a formatter rewriting 200 files.

   If saves split into two cycles, design a macOS equivalent of write settling. Options to evaluate and measure,
   not assumptions:
   - Hold the debounce while a changed file's size or mtime is still moving (a stat on fire, re-arm if it changed
     within the last N ms).
   - FSEvents flag combinations (ItemModified, ItemInodeMetaMod, ItemXattrMod).
   - Whether kqueue on macOS offers any close-after-write notification. FreeBSD has NOTE_CLOSE_WRITE; check whether
     macOS does before relying on it.

   Whatever you choose must not add a fixed delay to every save. The Linux design fires as soon as the last writer
   closes.
2. Self-triggered cycles. The watch writes outputs (`outDir`, `.tsbuildinfo`). Check that those writes never cause
   another cycle, including when `outDir` is inside a watched directory. FSEvents reports with directory granularity
   when coalescing, and its flags accumulate across events for the same path, so a stale "created" flag can turn a
   modify into a structure change.
3. Path identity. FSEvents reports real paths (`/private/tmp/...`, case as stored on disk). The watch manager
   compares paths with what the program resolved. Test a project under `/tmp`, a project opened through a
   symlinked directory, and a file renamed only in case.
4. Stream rebuild cost. Measure how long a watch-set change takes (stop the old stream, create the new one) with
   `TS_WATCH_DEBUG=1` and timestamps. Also check whether events between stop and start are lost: tsgo passes
   `sinceWhen`/event IDs for this, so verify the port does the same.
5. The kqueue fallback. It needs one file descriptor per watched file or directory, and the default macOS
   `ulimit -n` is 256. Force the fallback locally (temporarily make `fsevents().available()` return false) and run
   both scripts. It must either work or fail with a clear error, never silently stop watching.
6. Memory on macOS. Check RSS with `ps`, but also `footprint <pid>` and `vmmap --summary <pid>`, because RSS
   excludes compressed pages. Check that the background free actually returns memory to the OS on macOS.
7. Idle cost. Run `top -pid <pid> -stats pid,cpu,idlew,power` for a minute with no edits. Use
   `sudo powermetrics --samplers tasks --show-process-energy -i 5000 -n 6` to count wakeups.

## Tools

- `samply record -- target/release/tsrs --watch -p .` for CPU profiles (works on macOS, opens in Firefox Profiler).
  Instruments Time Profiler or `xctrace record --template 'Time Profiler'` also work. `sample <pid> 5` gives a quick
  look at a busy process.
- `sudo fs_usage -w -f filesys <pid>` shows the watch's own file syscalls. Use it to find redundant stats or reads
  per cycle.
- For symbols in profiles, build with:
  `CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_TARGET_DIR=target/dbgrel`.
- `TS_WATCH_DEBUG=1` prints watch decisions.

## Rules for changes

- Every fix needs a test that fails without it. FSEvents behavior can be tested in `crates/tsrs_fswatch` with real
  temp directories, as `tests/write_settling.rs` does for inotify.
- Don't change Linux behavior. If you touch shared code (`debounce.rs`, `watcher.rs`, the watch manager), re-run the
  Linux checks or ask for them to be run. Gate macOS-only code with `#[cfg(target_os = "macos")]` so Linux and
  Windows builds have no dead code.
- Keep Go parity unless you measure a clear win. Mark deviations as tsrs-only in a comment that says why.
- Before each commit:
  - `cargo clippy --workspace --locked --exclude tsrs_fourslash --config 'build.warnings="deny"' -- -D dead-code`
  - `python3 tools/lint/source.py`

  Test targets in `tsrs_fswatch` already show five clippy warnings on macOS (`testutil_test.rs`, `watcher_test.rs`).
  They come from main, not from the watch work.
- Don't weaken a test or a baseline to get a pass. If something can't be fixed, report it with the evidence.
- Report numbers only for public projects such as vscode.

## Report back

- A table like the reference one, for macOS, with tsrs and tsgo on the same Mac. Include the Mac model, the macOS
  version, and the tsgo version.
- `watchcheck.py` results for both modes, plus the save-pattern results (pattern → cycles, correct or not).
- Idle CPU and wakeups, and RSS/footprint across `REPEAT=5`.
- Each problem found: the cause, the fix, its commit, and its test. Also list what you couldn't fix and why.
