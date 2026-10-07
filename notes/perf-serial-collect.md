# perf-serial-collect: fewer refcount updates in the collect walk

Goal: shorten "Program: collect files" (`filesParser::get_processed_files`), a serial step of 10 ms on the program
thread between the parse and the check pass of vscode `-p src` on the 64-vCPU runner (`depot-ubuntu-24.04-64`), from
the "what remains" list of notes/perf-front-end-fixed-costs.md. Base: main 4a00ada, the default 32 checkers.

## Where the 10 ms went

The walk visits ~131k task-list entries (one per import edge: 111k of them reach a file already walked) and runs the
post-visit step for 10.4k files. A frame-pointer profile of the program thread (`perf record -F 20000`, three runs,
samples mapped to lines with `llvm-addr2line -i`; run lj0md4k9qh) put 58% of the walk's samples on `Arc<str>`
refcount updates of `Path` and of the task names: 43.5% on clones, 14.6% on drops.

- The post-visit step cloned the task's path once per map (files by path, resolutions, type resolutions, metadata)
  between the map inserts, and dropped its own copy at the end: 36% of the samples.
- The first visit of a file cloned its name into `seen`: 12%.
- The first include reason of a file cloned its path into `reasons_order`: 9%.

A locked increment on x86 waits for the thread's earlier stores to drain, so a clone placed between two inserts into
large maps waits for the first insert's cache misses before the second can start.

## Change

- `seen` keeps the first-walked task and the address of its name instead of a clone. A revisit compares the address
  first and the names' text only when the addresses differ, which is what `Arc<str>`'s `==` does; the address is never
  dereferenced.
- `reasons_order` keeps the task, and the paths are cloned once after the walk, in the same order as before.
- The post-visit step takes its four key clones before the inserts, and takes the metadata (two strings) instead of
  copying it, as it already takes the resolutions: it is the last read of the task's data.

The walk's order, the files list, the include reasons and their order, and every map's contents are unchanged.

## Numbers (runner, run 0btsrwdd7k)

Base and new built in the same directory with `--profile dist`, 11 interleaved runs each.

| row | base median (min-max) | new median (min-max) |
| --- | ---: | ---: |
| Program: collect files | 10 ms (10-11) | 8 ms (8-9) |
| Program: sequential load | 14 ms (14-15) | 14 ms (14-15) |
| Check time | 450 ms (436-471) | 441 ms (433-464) |
| wall minus Total time | 16.6 ms | 16.7 ms |
| peak RSS | 2808 MiB | 2812 MiB |

Samples in the walk (three profiled runs): 527 before, 369 after. Single-threaded user instructions
(`bench/count.py`): 102.5588 G -> 102.5604 G (+0.002%, the list of keys built after the walk). `perf stat` at 32
checkers: 131.2 G vs 131.0 G user instructions, page faults 34.3k vs 34.0k. Output of `--pretty false`, `--listFiles`
and `--explainFiles` identical on vscode on the runner and on the ten older bench projects locally.

## What remains in the walk

After the change the largest items are the first touch of each file's path (the post-visit step's first clone, a cache
miss) and the include-reason pushes (`reasons_by_data[data]`). One more clone and drop per file could go by moving the
path into the last insert; that is below the row's 1 ms resolution and was not measured.
