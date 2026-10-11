# VFS realpath prefix cache (2026-10-11)

## Decision

Rejected. Caching canonical directory prefixes in `cachedvfs` removed 63-80% of macOS realpath `lstat` calls,
but did not move whole-program instructions on the pinned vscode benchmark and did not reduce peak memory. The
prototype added another concurrent path map and a second VFS realpath entry point, so it is not retained for the
small-project-only win.

This tests the realpath component-cache idea left in `notes/perf-dev-loop.md`. That note measured 150,000 component
`lstat`s at about 0.7 CPU-seconds and 40 ms wall on a large incremental workload. The result here says when to revisit
it: a workload must have that scale of repeated realpaths, and the cache should be composed with another filesystem
change or avoid a separate map. Repeating the same standalone cache on ordinary configured projects is not useful.

## Prototype

On macOS and the other platforms where `osvfs` ports Go's component-by-component `filepath.EvalSymlinks`, the
prototype recorded each directory prefix after `symlink_metadata` proved it was not a symlink. A later realpath
started at the longest recorded prefix and walked only the remaining components. The prefix map lived beside
`cachedvfs`'s whole-path realpath cache, used the same concurrent map implementation and was cleared or disabled in
the same cache generation. Linux and Windows kept their native canonicalization paths.

The fast path did not assume that lexical and physical paths were interchangeable. Tests primed the cache and then
compared cached and uncached results across a relative directory symlink, a physical `..` after that symlink, a
missing path and ordinary sibling files.

## Method

Apple M3 Max, 14 cores, 36 GiB RAM, macOS 27.0.1, rustc 1.99.0. Base was `f970b59a`; both binaries were non-PGO
release builds. Each measurement used one warm-up and alternating base/candidate order. `/usr/bin/time -l` supplied
instructions retired and maximum RSS.

The configured benchmark was vscode at `9cf0128b9822dcd8a533f08ef7312956a4390301`, `-p src --noEmit --incremental
false --pretty false`. Single-threaded runs also used `--singleThreaded` and `RAYON_NUM_THREADS=1`. Five paired runs
were made in each mode. The table reports independent medians and the median paired percentage change. macOS's
instruction counter alternated between two stable bands; the paired median is robust to one pair that crossed bands,
but this remains screening rather than Linux `bench/count.py` evidence.

The public Oxc website checkout was also screened because it concentrates repeated package realpaths. It used the
same single-threaded flags, 15 `/usr/bin/time -l` pairs and a separate 30-run Hyperfine wall-time comparison.

## Results

| Workload and mode | Measure | Base | Prefix cache | Paired change |
| --- | --- | ---: | ---: | ---: |
| vscode, single | instructions retired | 88.190 G | 88.217 G | -0.003% |
| vscode, single | peak RSS | 1,599.4 MiB | 1,609.0 MiB | +0.606% |
| vscode, default | peak RSS | 2,074.5 MiB | 2,067.5 MiB | -0.047% |
| Oxc website, single | instructions retired | 0.992 G | 0.978 G | -1.48% |
| Oxc website, single | peak RSS | 52.39 MiB | 52.53 MiB | +0.27% |
| Oxc website, single | Hyperfine wall, 30 runs | 71.3 +/- 1.0 ms | 69.3 +/- 0.6 ms | -2.8% |

The independent vscode default-RSS medians moved -0.34%, while the paired median was -0.047%; both are noise and far
below the 5% memory bar. Default-mode wall time was scheduling-noisy and was not used. No two-publish headline
comparison was made.

The mechanism itself worked:

| Workload | realpath `lstat`s: base -> cache | Reduction |
| --- | ---: | ---: |
| vscode | 1,609 -> 324 | -79.9% |
| Oxc website | 2,071 -> 763 | -63.2% |

Vscode is the landing decision: its whole-program instruction change is zero, against the required 1% on a configured
project. The Oxc website clears that percentage but is not in `bench/projects.json`, is only a 419-file workload and
does not justify a permanent concurrent map whose benefit disappears on Linux and Windows.

## Correctness and validation

- VFS units: 90 passed, including cached/uncached relative-symlink and physical-`..` parity.
- Vscode and the Oxc website: diagnostic bytes, exit status and all files/lines/identifiers/symbols/types/
  instantiations counters were identical. Vscode also had identical filesystem counters other than realpath
  `lstat`s.
- `cargo check --workspace --offline`, Linux-targeted `tsrs_vfs`, and `tsrs_wasm` for `wasm32-wasip1` passed.
- `tools/lint/ratchet.py` and `tools/lint/source.py` passed; the source inventory was below, not above, its cross-OS
  baseline on macOS.

The implementation was removed after measurement. Only this result and the rejected-experiment index remain.
