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

## Full-suite comparison after the hot-loop corrections

The [full PR verification](https://depot.dev/orgs/qw3zz3l8cs/workflows/28275503cj) compares main `e99e1816`
with PR head `37942867` (merge `2e750efb`) on one 32-vCPU AMD EPYC 9R45 Linux runner. All 102 diagnostic/exit
comparisons match, including 1/4/16/32 checkers and poisoned arenas. Instructions fall on 16 of 17 projects;
Supabase retains a +0.008% increase, down from the original +0.246%. This residual is not a zero-regression claim.
The command uses `--noEmit --incremental false --extendedDiagnostics --pretty false --singleThreaded`
and `RAYON_NUM_THREADS=1`. RSS below is the same single-threaded run's peak in MiB.

| Public project | Main instructions | Corrected instructions | Change | Peak RSS, main → corrected |
| --- | ---: | ---: | ---: | ---: |
| vscode | 88,966,334,694 | 88,957,167,385 | -0.010% | 1605.21 → 1605.51 |
| xstate-main | 6,026,576,306 | 6,018,120,609 | -0.140% | 183.15 → 183.26 |
| webpack | 11,971,388,279 | 11,969,476,259 | -0.016% | 312.75 → 312.83 |
| mui-docs | 37,857,547,385 | 37,850,731,897 | -0.018% | 741.84 → 742.14 |
| Compiler | 1,963,286,014 | 1,962,016,817 | -0.065% | 59.62 → 59.59 |
| Compiler-Unions | 3,746,426,958 | 3,742,783,306 | -0.097% | 63.32 → 63.35 |
| cal-diy | 32,320,892,647 | 32,284,306,660 | -0.113% | 775.14 → 774.12 |
| formbricks-web | 41,991,948,907 | 41,973,451,932 | -0.044% | 1102.25 → 1104.92 |
| supabase-studio | 44,023,263,304 | 44,026,961,943 | +0.008% | 876.70 → 876.82 |
| t3code-server | 43,357,515,301 | 43,334,056,686 | -0.054% | 738.51 → 738.51 |
| mikro-orm | 65,877,582,885 | 65,658,647,983 | -0.332% | 1136.85 → 1137.47 |
| next-packages-next | 18,527,734,376 | 18,518,590,925 | -0.049% | 446.23 → 445.95 |
| next-root | 13,629,879,620 | 13,598,903,227 | -0.227% | 316.40 → 316.84 |
| storybook | 10,247,272,063 | 10,242,487,599 | -0.047% | 281.52 → 282.45 |
| nuxt | 9,308,440,221 | 9,306,787,899 | -0.018% | 280.39 → 280.44 |
| playwright | 8,374,438,797 | 8,372,581,325 | -0.022% | 244.36 → 244.37 |
| drizzle-orm | 12,612,432,085 | 12,605,794,512 | -0.053% | 394.71 → 394.85 |

A [separate paired run](https://depot.dev/orgs/qw3zz3l8cs/workflows/sx403mhbqx) builds the same two revisions
on one runner and takes medians of three runs, both single-threaded and with no checker-count flags (the program's
default). This command omits `--extendedDiagnostics`, so its absolute instruction counts differ slightly from
the full-suite harness above.

| Public project | Instructions, main → corrected | Single-threaded peak RSS, bytes | Default peak RSS, bytes |
| --- | ---: | ---: | ---: |
| Compiler | 1,963,185,582 → 1,961,919,010 (-0.065%) | 62,189,568 → 62,803,968 | 109,531,136 → 106,409,984 |
| Compiler-Unions | 3,746,367,030 → 3,742,726,222 (-0.097%) | 66,433,024 → 66,580,480 | 115,322,880 → 116,281,344 |

At the default checker count, median instructions are 2,320,926,902 → 2,319,783,623 for Compiler and
4,546,958,065 → 4,541,810,595 for Compiler-Unions. Parallel scheduling makes these less stable than the
single-threaded counts. The small RSS differences vary between runs; they do not establish a memory improvement.
The final relation-loop
correction also leaves the allocation-profile counts above unchanged. Workspace CI, required lints, Node API,
Wasm, arena safety and the full conformance gate pass at this head.

## Supabase residual isolation

The remaining Supabase increase is reproducible with a same-runner, three-repetition
[comparison](https://depot.dev/orgs/qw3zz3l8cs/workflows/s76z7j6q7v). Reverting all of `relater_1.rs`'s
remaining conversions helps; reverting `relater_2.rs` or `inference.rs` does not. These runs omit
`--extendedDiagnostics`, as in the paired compiler probe.

| Supabase variant | Median single-threaded instructions | Single-threaded peak RSS, bytes | Default peak RSS, bytes |
| --- | ---: | ---: | ---: |
| Main `e99e1816` | 44,024,711,621 | 919,613,440 | 1,850,331,136 |
| PR `37942867` | 44,028,392,876 | 919,416,832 | 1,858,830,336 |
| Restore `relater_1.rs` | 44,023,759,729 | 919,126,016 | 1,861,996,544 |
| Restore `relater_2.rs` | 44,035,006,280 | 919,158,784 | 1,859,497,984 |
| Restore `inference.rs` | 44,028,457,956 | 919,302,144 | 1,863,286,784 |

[Narrower probes](https://depot.dev/orgs/qw3zz3l8cs/workflows/gljlb8d7z6), each paired with `37942867`, find:

| Restored helpers | Instructions, control → candidate | Single-threaded RSS, bytes | Default RSS, bytes |
| --- | ---: | ---: | ---: |
| Signature comparison | 44,028,393,915 → 44,028,181,181 | 919,429,120 → 919,367,680 | 1,857,044,480 → 1,861,914,624 |
| Recursive predicates | 44,028,394,662 → 44,025,671,266 | 919,121,920 → 919,375,872 | 1,855,524,864 → 1,853,059,072 |
| Template-literal helpers | 44,028,409,453 → 44,030,588,839 | 919,293,952 → 919,113,728 | 1,858,965,504 → 1,870,573,664 |

The recursive predicates are `is_deeply_nested_type`, `is_known_property` and `has_common_properties`.
The template group is `infer_types_from_template_literal_type`, `is_type_matched_by_template_literal_type`
and `infer_from_literal_parts_to_template_literal`. Keep that group's iterators and restore the other relation
loops together, then check the combined correction against all 17 projects. The signature result alone is too
small to claim an improvement. All probe outputs and exit codes match exactly.

The combined correction leaves heap allocation counts unchanged at 401,551 for Compiler and 433,650 for
Compiler-Unions, comparing `37942867` with the follow-up on macOS arm64 (`--features alloc-profile`, one
parser/checker). Arena requests remain 33.0/31.9 MB and peak live heap remains 10.0/12.8 MB. The full conformance
gate and 29 regressions pass, with all 18 conformance result lists unchanged; workspace checks and both required
lint scripts pass.

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
