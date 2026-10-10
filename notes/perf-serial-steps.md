# perf-serial-steps: the serial steps around the parallel phases (#162, #164, #166, #167)

Goal: shorten the serial steps on the program thread around a CLI run's parallel phases on the 64-vCPU runner
(`depot-ubuntu-24.04-64`), from the "what remains" list of notes/perf-front-end-fixed-costs.md. vscode `-p src`
(10,427 files), the default 32 checkers. This note collects what was measured, what landed, what was reverted and
what did not land.

## Method

- A/B probes as in notes/perf-front-end-fixed-costs.md: base and new built in the same directory with `--profile
  dist` (base = the change reversed with `git apply -R`), 11-15 interleaved runs, every `--extendedDiagnostics` row,
  wall from `wait4`, `perf stat` and a single-threaded `bench/count.py` run per binary, and a byte comparison of
  `--pretty false`, `--listFiles` and `--explainFiles` output. The phase rows print whole milliseconds, so a change
  under 1 ms does not show in them.
- Program-thread profiles: a release build with `-C force-frame-pointers=yes`, `perf record -F 20000 -g --call-graph
  fp` of three runs, samples of the `tsrs` threads attributed to the step on their stack, and the samples whose
  innermost frame is in one function mapped to source lines with `llvm-addr2line -a -i` (offsets from `perf script -F
  sym,symoff` plus the symbol's address from `nm`). The address marker matters: without `-a`, GNU-style output has no
  separator between addresses and the lines cannot be matched to them.
- The probe scripts (an A/B driver, the attribution and the line mapping) are not committed, as before.

## Status (2026-10-10)

#166 and #167 landed and are in the code (filesparser.rs: the collect walk at line 876 and `parseTask::with_path` at
line 77 cite their notes). #162 and #164 were reverted in https://github.com/maschwenk/tsrs/pull/190 (2026-10-07):
each bought 3-4 ms at 32 checkers by adding a thread or an overlap around checker creation, below the bar in AGENTS.md
("A performance change must pay for its complexity"). `checkerPool::create_checkers` again creates the checkers and
then runs `compute_associations` on the program thread, and "Diagnostics: program" runs on the program thread before
binding. Their measurements are kept below; do not re-land the mechanisms for this gain. (Checked in the code; the
local clone is shallow, so the merge and revert commits themselves were not inspected.)

## Results

| PR | step | row before -> after (median, min-max) | run | state |
| --- | --- | --- | --- | --- |
| #162 | file assignment while the checkers are created | Diagnostics: global (first) 15 (14-16) -> 11 (11-11) ms | dkp6ztszxg | reverted in #190 |
| #164 | program diagnostics on a helper beside checker creation | program + global (first) 18 (17-21) -> global (first) 15 (15-17) ms | jc50scgs6q | reverted in #190 |
| #166 | fewer `Arc` refcount updates in the collect walk | Program: collect files 10 (10-11) -> 8 (8-9) ms | 0btsrwdd7k | landed |
| #167 | fewer `Arc` refcount updates per import edge in the sequential load | Program: sequential load 15 (14-15) -> 13 (13-14) ms | p0085wt4dt | landed |

The landed two have their own notes: notes/perf-serial-collect.md, notes/perf-serial-load.md. pr-verify: 102/102
cells identical for #162, #164 and #166 (runs 6b4t7jwl3v, 993rh7tk30, sm03fhlsqm); #167's run is s452czrqsh. Each was
measured against the main of its day. Before #190 the four together were estimated at about 4 + 2-3 + 2 + 2 = 10 ms
less serial time in vscode's run; what remains landed is the two `Arc` changes, about 2 + 2 ms.

### The reverted two (base main 025b496, 11 interleaved `--profile dist` runs each)

- #162: `create_checkers` created the checkers from a scoped helper thread while the program thread computed the
  assignment, which reads only the loaded program. "Checkers: create" 4 ms and "Checkers: assign files" 10 ms were
  unchanged; "Diagnostics: global (first)" went 15 (14-16) -> 11 (11-11) ms. Check time 456 -> 463 ms, peak RSS 2805
  -> 2807 MiB, single-threaded instructions 102.558 G both, output identical.
- #164: a scoped helper thread computed "Diagnostics: program" (2 ms: the processing diagnostics and a walk over the
  ~110k resolutions in `includeProcessor::get_diagnostics`) while the program thread bound and created the checkers;
  the result kept Go's order. Serial time 18 (17-21) -> 15 (15-17) ms. Check time 454 -> 456 ms, peak RSS 2809 ->
  2808 MiB, single-threaded instructions 102.558 G -> 102.557 G, output identical.

The two `Arc` changes have one cause in common: a locked refcount update on x86 waits for the thread's earlier stores
to drain, so a clone or drop between stores that miss the cache (inserts into large maps, pushes into a growing task
vector) serializes them. The placeholder paths and lookup clones paid for it on every import edge. Other per-edge or per-file
loops on the program thread may have the same pattern.

## The steps, as measured

Program-thread timeline before the changes (a perf-slowed run, run hff3trj7fc):

| step | ms on the runner (rows) | what the profile shows |
| --- | ---: | --- |
| sequential load | 14-15 | `run_queued` and `add_prepared_sub_task`: one new task per import edge (110k); placeholder paths and name clones (#167) |
| collect files | 10 | 58% `Arc<str>` clones and drops of paths and names (#166) |
| verify options | 2 | most of it `common_source_directory` (vscode has `outDir`): a serial filter over the files and a `to_string` per file, then a parallel `contains_path` |
| Diagnostics: program | 2 | the include processor's diagnostics: a walk over 110k resolutions (#164 hid it; reverted) |
| checker creation, 4-5 ms, then file assignment, 10 ms | 15 | the row "Diagnostics: global (first)": the sweep itself is under 1 ms (#162 overlapped the two; reverted) |
| setup of the check pass | ~2.5 (profile) | 1.2 ms in `plan_splits` (below), 0.2 ms mapping `files` to program indices |
| Diagnostics: report | 2 | |
| kernel exit | ~15 | not re-measured |

## Not landed

- `common_source_directory`'s file list built on the worker pool (`par_iter().filter_map`). "Program: verify
  options" stayed at 2 ms in all 22 runs (run fsl05xm5r3). The profile put about 2 ms of a perf-slowed run in the
  serial part, so the real gain is under the row's resolution. Not proposed.
- The check pass's `index_of` (a hash lookup per file to map `files` to program indices) when `files` is the
  program's own list: 0.2 ms of the setup in the profile. Not tried.
- The sequential-load change in halves: `run_queued`'s lookup without a clone (with the address-first casing
  comparison) and `parseTask::with_path` each moved the row by under 1 ms in 11 runs (p5wvff6z8x, 1jsxcsw4rv);
  together they are #167. With `with_path` alone, the samples of `add_prepared_sub_task` halved and most of what was
  left moved to the push of the new task: the stores into fresh task memory are the rest of that step's cost.
- One more clone and drop per file in the collect walk (moving the path into the last insert): below the row's
  resolution, not measured.

## For the assignment and splitting work

- `plan_splits` calls `program.is_source_file_default_library(file.path())` for every file of every active checker
  before it tests `!file.is_declaration_file()` (still so on 2026-10-10, checkerpool.rs:1113, where the result now
  picks `LIB_MIN_SHARE_PERCENT`). That is a hash of each file's ~100-byte path into the lib-files map, 10.4k times, on
  the program thread at the start of the check pass: 1.2 ms of the setup in the profile (the symbol shows up as a
  `HashMap<Path, P<jsxRuntimeImportSpecifier>>` lookup, folded with the identical lib-files one). Testing the
  declaration flag and the weight first and the default library last would skip nearly all of the hashes. Not changed.
- With the checkers created before the assignment (as on main since #190), "Diagnostics: global (first)" is checker
  creation (4-5 ms) plus the assignment (10 ms, `compute_associations`). Overlapping the two was #162: 3-4 ms, below
  the bar. Shortening `compute_associations` itself shortens the serial time one for one, with no new thread.

## The two global sweeps

"Diagnostics: global (after)" is 0 ms on vscode: the deferred diagnostics run inside the check pass, and the second
sweep only reads each checker's collection. The first sweep's cost was the checker creation and assignment it
triggers (above), not the sweep, so merging the two sweeps would save nothing measurable.

## Kernel exit

Nothing new: no probe here looked at the teardown beyond what notes/perf-front-end-fixed-costs.md measured.
