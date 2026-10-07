# perf-serial-steps: the serial steps around the parallel phases (#162, #164, #166, #167)

Goal: shorten the serial steps on the program thread around a CLI run's parallel phases on the 64-vCPU runner
(`depot-ubuntu-24.04-64`), from the "what remains" list of notes/perf-front-end-fixed-costs.md. vscode `-p src`
(10,427 files), the default 32 checkers. This note collects what was measured, what landed and what did not.

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

## What landed

| PR | step | row before -> after (median, min-max) | run |
| --- | --- | --- | --- |
| #162 | file assignment while the checkers are created | Diagnostics: global (first) 15 (14-16) -> 11 (11-11) ms | dkp6ztszxg |
| #164 | program diagnostics on a helper beside checker creation | program + global (first) 18 (17-21) -> global (first) 15 (15-17) ms | jc50scgs6q |
| #166 | fewer `Arc` refcount updates in the collect walk | Program: collect files 10 (10-11) -> 8 (8-9) ms | 0btsrwdd7k |
| #167 (open) | fewer `Arc` refcount updates per import edge in the sequential load | Program: sequential load 15 (14-15) -> 13 (13-14) ms | p0085wt4dt |

Each has its own note: notes/perf-serial-assign-overlap.md, notes/perf-serial-program-diagnostics.md,
notes/perf-serial-collect.md, notes/perf-serial-load.md (in #167). pr-verify: 102/102 cells identical for #162, #164
and #166 (runs 6b4t7jwl3v, 993rh7tk30, sm03fhlsqm); #167's run is s452czrqsh. Each was measured against the main of
its day, so the sum is an estimate: about 4 + 2-3 + 2 + 2 = 10 ms less serial time in vscode's run.

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
| Diagnostics: program | 2 | the include processor's diagnostics: a walk over 110k resolutions (#164 hides it) |
| checker creation, 4-5 ms, then file assignment, 10 ms | 15 | the row "Diagnostics: global (first)": the sweep itself is under 1 ms (#162 overlaps the two) |
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

## For the assignment and splitting work (not changed here)

- `plan_splits` calls `program.is_source_file_default_library(file.path())` for every file of every active checker
  before it tests `!file.is_declaration_file()`. That is a hash of each file's ~100-byte path into the lib-files map,
  10.4k times, on the program thread at the start of the check pass: 1.2 ms of the setup in the profile (the symbol
  shows up as a `HashMap<Path, P<jsxRuntimeImportSpecifier>>` lookup, folded with the identical lib-files one). Testing
  the declaration flag and the weight first and the default library last would skip nearly all of the hashes. Not
  changed here because `plan_splits` belongs to the other agent's work.
- With #162 the file assignment (10 ms, `compute_associations`) is the critical path before the check pass; checker
  creation (4-5 ms) finishes inside it. Anything that shortens the assignment now shortens the serial time one for
  one, up to the creation's 4-5 ms. The assignment reads only the loaded program, so it could also start earlier, for
  example beside "Diagnostics: program" or the end of program construction, if the pool were created there.

## The two global sweeps

"Diagnostics: global (after)" is 0 ms on vscode: the deferred diagnostics run inside the check pass, and the second
sweep only reads each checker's collection. The first sweep's cost was the checker creation and assignment it
triggers (above), not the sweep, so merging the two sweeps would save nothing measurable.

## Kernel exit

Nothing new: no probe here looked at the teardown beyond what notes/perf-front-end-fixed-costs.md measured.
