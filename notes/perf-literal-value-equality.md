# Numeric literal equality in simple relations (2026-10-15)

Candidate on `79b58f47`, branch `perf/literal-value-equality`, isolated worktree
`/private/tmp/tsrs-literal-value-equality`. Retained for review; not landed. macOS screening shows
1.95% fewer single-threaded instructions on Compiler-Unions. The deterministic Linux `bench/count.py`
landing gate remains unverified: this machine is macOS and that script requires `perf_event_open`.
There is no material peak-memory improvement or wall-time claim.

## Evidence and change

The preceding xctrace investigation found `is_simple_type_related_to` at 9.35% inclusive sampled CPU
on Compiler-Unions, with 1.18% leaf samples in derived literal-value equality. Its assembly copied both
24-byte literal enums onto the stack before comparing them. The function reserved 208 bytes of stack.

The prior rejected inline/outlined number-literal experiment in `compareTypes` targets a different
function. This candidate changes the four value comparisons in simple relations, using the new
`LiteralType::value_equals` helper. It compares numeric pairs through the existing floating-point
equality and falls back to the original `Option<LiteralValue>` equality for other variants. It preserves
NaN inequality, signed-zero equality, `None`, cross-variant inequality, and string content equality.
The flag predicates, branch ordering, enum-resolution calls, and diagnostic side effects stay in place.
It adds no cache, allocation, unsafe code, layout change, or validity assumption about literal values.

The complete release binary confirms the intended code generation. LLVM outlines `value_equals`
despite its inline hint. Its numeric path loads the tags and doubles directly and uses `fcmp`, without
copying the enums to the stack. The nonnumeric fallback also avoids the enum stack copies in this build.
The relation function's stack reservation falls from 208 to 128 bytes; the helper itself reserves 16 bytes.
This is a stack/code-generation observation, not evidence of a peak-RSS reduction.

## Build and measurement

Apple M3 Max, 14 cores, 36 GiB, macOS 27.0.1, Instruments 27.0, Rust 1.99.0. Only the public pinned
Compiler and Compiler-Unions projects were used, suite commit `41f652ab2df5077b1115f73eaeefc2fe9f674132`.
Both executables were built in the same worktree, first the unchanged baseline, then the candidate:

```sh
CARGO_PROFILE_RELEASE_DEBUG=2 \
CARGO_PROFILE_RELEASE_STRIP=none \
CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed \
cargo build --release --locked -p tsrs_cli --target-dir target/profiling
```

Full release optimization, fat LTO, one codegen unit, full debug information, no strip, no PGO.
The saved binary/dSYM pairs have matching UUIDs:

- Baseline: `068D5FD6-B89F-3166-847A-98F5FAF18AE2`.
- Candidate: `D6AFDD8F-5F5F-37B9-A28C-19259992D84D`.

Ten interleaved repetitions after one warmup per project/mode/variant, reversing variant order each
round. Uninstrumented `/usr/bin/time -l` supplies instructions retired and maximum RSS. Single-threaded
runs use both `--singleThreaded` and `RAYON_NUM_THREADS=1`; default runs use neither override. All runs
use `--noEmit --incremental false --pretty false`. Builds and validation completed before measurement.
Every diagnostic output is byte-identical within each project/mode, with the same expected exit 2.

| Project / mode | Baseline instructions | Candidate instructions | Change | Baseline / candidate peak RSS |
| --- | ---: | ---: | ---: | ---: |
| Compiler, single | 1.815749 G | 1.807840 G | -0.436% | 57.219 / 57.094 MiB |
| Compiler-Unions, single | 3.357809 G | 3.292363 G | **-1.949%** | 59.867 / 59.727 MiB |
| Compiler, default | 2.131160 G | 2.125256 G | -0.277% | 78.414 / 78.414 MiB |
| Compiler-Unions, default | 4.038967 G | 3.972288 G | -1.651% | 83.586 / 83.484 MiB |

These are medians. macOS instruction counts include kernel work; they screen candidates but do not
replace the repository's deterministic Linux user-space instruction gate. Default RSS changes by
0.00% and -0.12%, far below the memory threshold. These two small projects do not establish results
for larger applications, other architectures, or PGO builds.

## xctrace follow-up

Three interleaved Time Profiler launches per binary on Compiler-Unions, single-threaded, with
100-microsecond sampling and the complete recording-options JSON. Exported XML references are resolved
before aggregation; samples are weighted, with each function counted once per stack for inclusive CPU.

| Sampled CPU | Baseline | Candidate |
| --- | ---: | ---: |
| Total samples | 6,807 | 6,770 |
| `is_simple_type_related_to`, inclusive | 9.86% | 9.35% |
| Derived `LiteralValue` equality, leaf | 1.23% | 0 samples |
| `LiteralType::value_equals`, inclusive | absent | 0.55% |

The equality samples move into the new helper, consistent with the assembly. The short sampling runs
are attribution evidence, not a wall-time speedup measurement. The performance claim above comes from
the separate uninstrumented instruction counts.

## Validation and artifacts

- Focused equality test: all 324 pairs of 18 values, including NaN, infinities, signed zero, strings in
  distinct storage, booleans, bigints, missing values, and mixed variants, agree with original equality.
- Full dev-profile conformance in Go history: 13,458 diagnostic passes and 12,779 type/symbol passes.
- Full canonical-history conformance: 13,458 diagnostic passes and 12,778 type/symbol passes, with the
  existing `objectLiteralNormalization` type/symbol failure unchanged.
- Every result list matches its baseline exactly in both histories, with zero crashes or timeouts.
  The initial candidate run hit the default 20-second limit on `intersectionConstructorReductionCrash`
  while release compilation and Clippy were running. Rerunning with the CI gate's 120-second allowance
  completed normally and produced the matching lists above.
- All 36 standalone regression cases pass.
- Workspace Clippy with locked dependencies, all targets, warnings denied and dead code denied passes;
  `tools/lint/source.py` and `git diff --check` pass.

Local ignored artifacts are under `target/literal-equality/`: saved baseline/candidate executables and
dSYMs, UUIDs, disassembly, `paired-measurements.json`, measurement and profile summaries, six `.trace`
recordings and exported XML, conformance result lists/logs, Clippy/regression logs, and the measurement,
recording, analysis, and validation scripts. `target/profiling/release/tsrs` is the candidate release
binary, with its matching `tsrs.dSYM` beside it.

Before landing, run paired Linux `bench/count.py` measurements on the pinned public projects with the
same release settings and retain only if the repository's instruction threshold is confirmed without
regressions. Keep the candidate isolated until then.
