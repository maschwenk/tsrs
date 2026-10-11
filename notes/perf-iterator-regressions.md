# Iterator refactor regressions (PR 317)

The [original PR comparison](https://github.com/maschwenk/tsrs/pull/317#issuecomment-6106599399)
reported 0.036–0.339% more single-threaded instructions on 16 of 17 public projects, with unchanged diagnostics.
This investigation removes regressions from the refactor; it does not introduce a new performance optimization.

## Method

Release builds with the pinned Rust toolchain and unchanged optimization settings. Public Compiler and
Compiler-Unions inputs come from `bench/projects.json` (typescript-benchmarking at `41f652ab2df5`). On Linux,
`bench/count.py` measures `--noEmit --incremental false --pretty false --singleThreaded` with
`RAYON_NUM_THREADS=1`, so parsing and checking both use one thread. Compare ordered diagnostics and exit codes too.

The local macOS counters varied too much to resolve all changes this small; use Linux counts to select the fix.
The probe applies the measured patch explicitly and records source hashes alongside its results. Retain the
previously documented loop in `maybe_constituent_of_kind` as well.

## Cause and correction

Callgrind profiles on the public Compiler workload identified extra flow-cache scan instructions and additional
hash-table growth in union construction. Flattening `named_unions` into one iterator loses the constituent
slices' length hints: the original code reserved for each slice before inserting it. Call `extend` for each slice, preserving that allocation behavior and retaining iterator traversal of the outer list.

The flow-cache scan retains its indexed loop. Selected hot ancestor searches, contextual-node lookup, signature
collection and scanner cursor walks retain their original loops as well. These are exceptions to the stylistic
refactor, not changes to the checker's algorithms or compiler optimization settings.

## Initial compiler measurements

Three single-threaded runs per build on Depot's 32-vCPU AMD EPYC 9R45 Linux runners, median instructions.
The baseline is `4f9eda48`, before the refactor; the original PR is `0f359f85`, and the first correction is
`6786879e`. These profiling builds retain symbols
(`CARGO_PROFILE_RELEASE_STRIP=none`) with the same release optimizations. The original PR/baseline probe is
[here](https://depot.dev/orgs/qw3zz3l8cs/workflows/v78zdn64h4); the explicitly applied correction is
[here](https://depot.dev/orgs/qw3zz3l8cs/workflows/1n7btftcj0).

| Public project | Pre-refactor | Original PR | Corrected | Corrected vs pre-refactor |
| --- | ---: | ---: | ---: | ---: |
| Compiler | 1,965,496,560 | 1,970,108,773 | 1,964,955,117 | -0.028% |
| Compiler-Unions | 3,750,297,612 | 3,759,686,089 | 3,751,323,553 | +0.027% |

Peak RSS at the default checker count, medians of three separate runs (bytes):

| Public project | Pre-refactor | Original PR | Corrected |
| --- | ---: | ---: | ---: |
| Compiler | 110,497,792 | 109,158,400 | 108,486,656 |
| Compiler-Unions | 115,429,376 | 115,605,504 | 115,372,032 |

These small RSS differences are not a memory improvement claim. On macOS arm64, `--features alloc-profile`
with one parser/checker reports heap allocation counts of 405,225 → 404,445 for Compiler and
439,453 → 436,681 for Compiler-Unions (original PR → correction). Arena bytes and peak live heap bytes are
unchanged. Restoring the outer union loop together with the type-list comparator, or separately restoring the
union-worker body, union-capacity folds or union-cache hash fold did not recover the remaining Compiler-Unions increase, so those reversions
are not retained. The full project comparison below uses current main after resolving the unsigned-position refactor conflicts.

Workspace all-feature checks, required lint scripts, 29 regression cases, formatter cases and the dependency
traversal case pass. The full conformance gate reports 13,458 error-baseline passes and 12,779 each for types and
symbols, with no crashes or timeouts; all 18 result lists match the original PR.

## Remaining hot paths after merging main

The first full run against `f6365f22` retained small instruction increases, including +0.053% on VS Code,
+0.061% on Supabase and +0.021% on Compiler-Unions. All 102 diagnostic/exit comparisons matched. Further
isolated measurements use `7a1d78e7` as the control, with the same single-threaded command and three runs:

| Reversion from that control | Compiler instructions | Compiler-Unions instructions |
| --- | ---: | ---: |
| Control | 1,963,176,510 | 3,747,386,884 |
| Four ancestor walks | 1,962,656,553 | 3,746,902,279 |
| Template-literal collector | 1,963,058,322 | 3,745,819,049 |
| Six other union helpers | 1,963,044,832 | 3,747,321,848 |

The ancestor comparison is
[here](https://depot.dev/orgs/qw3zz3l8cs/workflows/sj1brs4zbh), and the collector isolation is
[here](https://depot.dev/orgs/qw3zz3l8cs/workflows/8h062mp9z5).

Retain loops in the four ancestor walks: `successors` computes the next parent before yielding the node, including
when the search is about to return. Retain the template-literal collector and the measured group of six other
union helpers. Reverting the union subset search had no effect, and reverting the property-literal collector
slightly increased instructions, so both retain their iterator forms.

The next probe also restores the template-literal comparison loop in `structured_type_related_to_worker`:
its recursive `try_fold` increased the hot relation worker's cost. Restoring the mapper or type-list comparator
alone did not help; keep their iterator forms. The three candidates are recorded
[here](https://depot.dev/orgs/qw3zz3l8cs/workflows/jd6nzfjj1s).

The last ancestor/collector corrections (`7a1d78e7` → `8f70f6dc`) leave heap allocation counts unchanged:
401,601 for Compiler and 433,694 for Compiler-Unions, with unchanged arena bytes and peak live heap bytes
(`--features alloc-profile`, one parser/checker, macOS arm64).

## Formatter and dependency traversal

Restore the formatter's diagnostic cursor across calls. With N diagnostics at `[4i, 4i + 1)` and N increasing
lookups at `[4i + 2, 4i + 3)`, the iterator version restarts at zero on every lookup. The original loop advances
past diagnostics that cannot overlap later ranges, retaining an overlapping diagnostic for subsequent calls.

A standalone release harness extracts the actual helper from each revision, creates its closure before timing,
and calls it through `std::hint::black_box` for every lookup. Median of nine runs on macOS arm64:

| Diagnostics / lookups | PR iterator | Restored cursor |
| --- | ---: | ---: |
| 5,000 | 17.652 ms | 12.583 µs |
| 10,000 | 70.036 ms | 24.500 µs |
| Doubling ratio | 3.97× | 1.95× |

Also honor `false` from `range_dependencies` across dependency groups. A regression case covers every stopping
position and complete traversal, including sorted names within a group. This preserves the original and Go
callback contract.
