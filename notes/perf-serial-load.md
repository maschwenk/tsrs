# perf-serial-load: fewer refcount updates per import edge in the sequential load

Goal: shorten "Program: sequential load", the serial part of the loader on the program thread (14-15 ms for vscode
`-p src` on the 64-vCPU runner, `depot-ubuntu-24.04-64`, the default 32 checkers), from the "what remains" list of
notes/perf-front-end-fixed-costs.md. Base: main 6a0cecc.

## Where the time went

The sequential load runs `run_queued` for every queued task and, for each loaded file, appends one sub task per import
edge (`add_prepared_sub_task`; vscode: ~110k edges). A frame-pointer profile of the program thread with samples mapped
to source lines (run p5wvff6z8x, three runs at 20 kHz) showed:

- 77% of `add_prepared_sub_task`'s samples on `parseTask::new`'s placeholder `path: Path::default()`, which clones the
  one static empty `Arc<str>` (a locked increment), and 13% on its drop when the real path is assigned on the next
  line.
- `run_queued` cloned each task's name to look it up among its file's casings and dropped the clone when the name was
  already there, which is nearly always. The lookup compared the strings' text even when both are the same shared
  string.

A locked refcount update waits for the thread's earlier stores, which here are the writes of the new tasks into the
growing task vector.

## Change

- `parseTask::with_path` creates a task with its path; the two sub-task constructors use it, so a sub task no longer
  clones and drops the empty placeholder.
- `run_queued` looks the name up by reference and clones it only to insert a new casing.
- `TasksByCasing::get_key_value` compares the address and length first (the tasks of a file name share its string)
  and the text only when they differ, which is the same result.

## Numbers (runner)

Base and new built in the same directory with `--profile dist`, interleaved runs.

| change | runs | sequential load, base (min-max) | new (min-max) | run |
| --- | ---: | --- | --- | --- |
| `run_queued` and the casing lookup alone | 11 | 14 ms (14-15) | 14 ms (13-14) | p5wvff6z8x |
| `with_path` alone | 11 | 14 ms (14-15) | 14 ms (13-14) | 1jsxcsw4rv |
| both (this change) | 15 | 15 ms (14-15): seven 14, eight 15 | 13 ms (13-14): thirteen 13, two 14 | p0085wt4dt |

Either half alone moved the row by under its 1 ms resolution; together they move it by 1-2 ms. With `with_path` alone,
`add_prepared_sub_task`'s samples halved (398 -> 200 in three profiled runs) and most of the rest moved to the push
of the new task (`new_task`), the stores into fresh task memory that the refcount updates had been waiting for.

Other rows of the combined run: collect files 10 -> 10 ms, parallel parse + resolve 67 -> 67 ms, Parse time
105 -> 103 ms. Single-threaded user instructions (`bench/count.py`): 102.5577 G -> 102.5567 G. `perf stat` at 32
checkers: page faults 34.1k both. Output of `--pretty false`, `--listFiles` and `--explainFiles` identical on vscode on
the runner and on the ten older bench projects locally.
