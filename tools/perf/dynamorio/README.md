# Exact instruction counts without a hardware counter (DynamoRIO)

`bench/count.py` needs `perf_event_open`, which virtual machines (OrbStack, most hypervisors) do not expose. These two
DynamoRIO clients count executed instructions in software instead: exact, user space only, repeatable to 0.02% at
8 checkers in `--checkerAssignment locality` and to ~1e-7 single-threaded (notes/perf-icount-round4.md).

- `icount.c`: the whole process, summed over all threads (one inlined add per basic block into a per-thread
  counter). Options: `-out <path>`, `-per_thread` (one line per thread, i.e. per checker).
- `bbcount.c`: execution counts per basic block of the main module, for an instruction profile. The counter update
  is not atomic, so use single-threaded runs for exact numbers. `bbagg.py <bbcount.txt> <binary> <prefix>` turns the
  output into `.self.tsv` (innermost inlined function), `.outer.tsv` (physical function), `.line.tsv` and
  `.incl.tsv`; it needs `llvm-symbolizer` (or `LLVM_SYMBOLIZER=<path>`) and a binary built with
  `CARGO_PROFILE_RELEASE_STRIP=none` and `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`.

Build (Linux, x86-64 or AArch64; a DynamoRIO release or weekly build unpacked at `$DR`):

```sh
cmake -S tools/perf/dynamorio -B target/dr-clients -DDynamoRIO_DIR=$DR/cmake && cmake --build target/dr-clients
```

Run (`-no_enable_reset`: cache resets re-instrument and add noise; keep drrun's default `-max_elide_jmp 0`, or
elided branches go uncounted):

```sh
$DR/bin64/drrun -no_enable_reset -c target/dr-clients/libicount.so -out /tmp/icount.txt -per_thread -- \
  target/release/tsrs -p <project> --noEmit --incremental false --pretty false --checkers 8 --checkerAssignment locality
$DR/bin64/drrun -no_enable_reset -c target/dr-clients/libbbcount.so -out /tmp/bb.txt -- \
  target/release/tsrs -p <project> --noEmit --incremental false --pretty false --singleThreaded
python3 tools/perf/dynamorio/bbagg.py /tmp/bb.txt target/release/tsrs /tmp/prof
```

A counted 8-checker run takes about 7x the native time, a single-threaded one 3.5-4x. Default mode (work stealing)
varies by about 1.7% between runs; compare locality-mode or single-threaded counts, and read default mode as the
headline only.
