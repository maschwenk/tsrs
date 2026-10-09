# mimalloc-safe 0.1.67 (2026-10-09)

## Question and result

Replacing `mimalloc 0.1.52` is also an allocator-version choice: its `libmimalloc-sys 0.1.49` builds mimalloc
v3.3.2 by default, while `mimalloc-safe 0.1.67` / `libmimalloc-sys2 0.1.63` builds v2.5.2 unless the `v3` feature
is enabled. The default-v2 configuration is not used: it raises peak RSS about 4% at the default checker count on
both measured projects, and `notes/perf-build-level.md` already found mimalloc v2 3-14% slower on a large Linux
workload. The dependency therefore enables `v3`, which builds mimalloc v3.5.2.

With v3 enabled, the pinned vscode workload changes retired instructions by +0.15% with one checker and +0.03% at
the default; peak RSS changes by -0.69% and +0.46%. Median paired wall time is -1.13% / -1.44%, but the per-round
ranges are too wide to distinguish that from noise. The two smaller Compiler workloads put instructions within
+0.08-0.19% and peak RSS within -0.84% to +1.07%; the +1.07% case is +0.8 MiB, below the regression gate's
additional 2 MiB threshold. This is not a performance improvement, but it preserves the existing allocator
generation without a material regression.

## Method

Apple M4 Pro (12 cores, 48 GB), macOS 26.6.2, rustc 1.99.0.
Each variant was a clean release build from the same source: `cargo build --release -p tsrs_cli`. The workloads are
vscode `9cf0128b9822` (project `src`, installed with `npm ci --ignore-scripts --no-audit --no-fund`) and `Compiler`
and `Compiler-Unions` from typescript-benchmarking `41f652ab`; these are the revisions pinned by
`bench/projects.json`.

Ten rounds were interleaved after one warm-up. `/usr/bin/time -l` measured instructions retired and maximum
resident set size for `tsrs -p <project> --noEmit --incremental false --pretty false`; single mode adds
`--singleThreaded` and `RAYON_NUM_THREADS=1`, and default mode uses the default checker count. All variants returned
the same expected status 2. The tables give medians and the median paired change. The 0.07-0.30 second Compiler
workloads are too short for a useful wall-time conclusion; retired instructions are the stable CPU measure there.
Diagnostic output from the old and v3 builds was byte-identical on all three projects.

## Before the change vs mimalloc-safe with `v3`: vscode

| mode | wall, old -> new | paired change | instructions, old -> new | paired change | peak RSS, old -> new | paired change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| one checker | 8.715 -> 8.705 s | -1.13% | 100.701 G -> 100.291 G | +0.148% | 1600.88 -> 1589.77 MiB | -0.69% |
| default | 2.135 -> 2.100 s | -1.44% | 111.469 G -> 111.205 G | +0.026% | 1951.23 -> 1960.65 MiB | +0.46% |

Wall-time paired ranges were -25.6% to +3.2% with one checker and -12.5% to +11.0% at the default, so the apparent
wall improvement is not a supported claim. The instruction and peak-RSS changes are below their regression gates.

## Default mimalloc-safe configuration (v2.5.2), rejected

| project | mode | instructions, old -> new | change | peak RSS, old -> new | change |
| --- | --- | ---: | ---: | ---: | ---: |
| Compiler | one checker | 2.204 G -> 2.209 G | +0.220% | 57.30 -> 57.04 MiB | -0.46% |
| Compiler | default | 2.591 G -> 2.600 G | +0.302% | 77.18 -> 80.32 MiB | +3.99% |
| Compiler-Unions | one checker | 4.948 G -> 4.953 G | +0.121% | 59.52 -> 58.52 MiB | -1.68% |
| Compiler-Unions | default | 6.109 G -> 6.117 G | +0.126% | 81.55 -> 84.80 MiB | +3.90% |

## mimalloc-safe with `v3` (v3.5.2), used

| project | mode | instructions, old -> new | change | peak RSS, old -> new | change |
| --- | --- | ---: | ---: | ---: | ---: |
| Compiler | one checker | 2.209 G -> 2.211 G | +0.151% | 57.30 -> 57.22 MiB | -0.14% |
| Compiler | default | 2.594 G -> 2.597 G | +0.084% | 77.07 -> 77.87 MiB | +1.07% |
| Compiler-Unions | one checker | 4.950 G -> 4.955 G | +0.088% | 59.52 -> 59.02 MiB | -0.84% |
| Compiler-Unions | default | 6.107 G -> 6.121 G | +0.194% | 81.74 -> 82.05 MiB | +0.39% |
