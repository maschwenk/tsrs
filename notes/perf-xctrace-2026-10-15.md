# xctrace: checker hotspots and memory, 2026-10-15

Base `6822eeed`, Apple M3 Max (14 cores, 36 GiB), macOS 27.0.1, xctrace 27.0. Public inputs match
`bench/projects.json`: vscode, t3code-server and Compiler-Unions. No private code was used.

Result: the remaining cost is mostly checking, especially instantiation on t3code. A small outlining experiment
did not clear the performance bar and was reverted. The existing `--checkers 8` option reduced t3code's peak RSS
25.5% and total instructions 28.8% with unchanged median wall time here; vscode paid 18% wall time for 6.4% less
memory. This supports a per-project choice, not another change to the default.

## Method

Build: `CARGO_PROFILE_RELEASE_STRIP=none cargo build --release --locked -p tsrs_cli`. Fat LTO, default allocator,
compressed pointers, retained symbols, no PGO. Same build settings for both experiment binaries. Flags:
`-p <project> --noEmit --incremental false --pretty false`. Single-threaded measurements additionally use
`--singleThreaded` and `RAYON_NUM_THREADS=1`. The default uses 14 checkers on this machine for vscode and t3code.

Instructions and peak RSS come from **unprofiled** `/usr/bin/time -l` runs. Mac instruction counts include kernel
work, vary across runs, and are screening data, not the deterministic Linux `bench/count.py` gate. Initial baselines
are medians of three runs per cell. The outlining comparison uses three alternating base/candidate pairs; the
checker-count comparison alternates default/8 order across three pairs. Wall time is local screening only, not the
two publishes required by the wall-time landing rule.

Traces recorded with `xcrun xctrace record --template 'CPU Counters'` (vscode single-threaded) and
`--template 'Time Profiler'` (t3code default), `--time-limit 30s`, `--launch -- <binary> <flags>`.
Both ended when the compiler exited normally with diagnostic status 2. Exported tables:

```sh
xcrun xctrace export --input <trace> --toc
xcrun xctrace export --input <trace> \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]' --output samples.xml
xcrun xctrace export --input <trace> \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="MetricTable"]' --output metrics.xml
```

The analysis resolves XML `id`/`ref` references, selects running samples of the target process, sums sample weights,
and demangles symbols with `rustfilt`. Inclusive totals count a function once per stack, including recursive stacks.
Symbols have no source-line information; inlined work belongs to its containing function. These are sampled CPU-time
shares, not exact per-function instruction counts. CPU metric ratios are weighted by the cycles in their intervals.

Local raw evidence, binaries, scripts, exported XML, disassembly and the rejected patch are in
`target/xctrace-20261015/` (ignored): `baseline.json`, `ab.json`, `checker-counts.json`,
`vscode-single.trace`, `t3code-default.trace`, `*-samples.summary.json`, and `*-census.{out,err}`.
The `Allocations` recording failed to attach; it provides no usable allocation evidence. Memory attribution below
uses the existing `TSRS_MEM_SPLIT=1 TSRS_HEAP_CENSUS=1` instrumentation with `--extendedDiagnostics` instead.

## Baselines and hotspots

| project | single instructions | single peak RSS | default instructions | default peak RSS |
| --- | ---: | ---: | ---: | ---: |
| vscode | 83.341 G | 1,618.0 MiB | 95.275 G | 2,104.5 MiB |
| Compiler-Unions | 3.356 G | 59.8 MiB | 4.046 G | 83.6 MiB |
| t3code-server | 39.554 G | 729.3 MiB | 123.202 G | 2,090.1 MiB |

vscode has 9,647 running samples; t3code has 12,531. Stacks containing checker work account for 80.3% and 92.7%
respectively. vscode parsing accounts for 7.4%, binding for 4.5%, and module resolution for 2.1%; these categories
assign nested work to the checker first. t3code has much less front-end work relative to its check.

| function | vscode single self / inclusive | t3code default self / inclusive |
| --- | ---: | ---: |
| `instantiate_type_with_alias_worker` | 0.94% / 5.00% | 5.63% / 41.43% |
| `TypeMapper::map` | — / 0.76% | 2.27% / 7.71% |
| `SymbolMap::search` | 2.32% / 2.32% | 2.01% / 2.01% |
| `get_type_at_flow_node` | 2.27% / 10.65% | 1.36% / 6.62% |
| `get_ready_lazy_member_table_worker` | — / 1.51% | 1.62% / 7.51% |
| `ReferenceInstantiations` rehash | — / 0.31% | 1.32% / 1.36% |

Inclusive rows overlap. The instantiation worker contains inlined dispatch and bookkeeping, so its self samples
are not all cache lookup or hashing. Disassembly places samples in the active-mapper scan, hash-table probe,
instantiation-stack bookkeeping, type-parameter dispatch, cache insertion return, and function return.

vscode CPU Counters classify sustainable instruction bandwidth as 35.28% instruction delivery bottleneck,
13.87% discarded work, 21.45% instruction processing bottleneck and 29.40% useful. These are Apple's bandwidth
categories, **not fractions of elapsed time or a guaranteed speedup ceiling**. Instruction delivery is the largest
category; the processing category alone does not establish a data-memory bottleneck.

## Rejected micro-change: outline `instantiate_type_worker`

Fresh evidence: the hottest t3code wrapper contains 11,616 bytes of machine code, including its inlined worker;
vscode has substantial instruction-delivery pressure. Adding `#[inline(never)]` only to `instantiate_type_worker`
reduced the wrapper to 3,132 bytes and put 7,508 bytes in the worker. No algorithm or data representation changed.

Three-pair medians, baseline -> candidate:

| project | single instructions | change | default peak RSS | change |
| --- | ---: | ---: | ---: | ---: |
| vscode | 83.414 -> 83.492 G | +0.09% | 2,118.0 -> 2,111.5 MiB | -0.31% |
| Compiler-Unions | 3.362 -> 3.359 G | -0.10% | 83.4 -> 84.0 MiB | +0.64% |
| t3code-server | 39.304 -> 39.477 G | +0.44% | 2,110.2 -> 2,097.6 MiB | -0.60% |

Default wall medians are unchanged on all three projects. Single-threaded peak RSS changes by less than 0.2%.
Every compared diagnostic byte and exit status matches (36 runs, six project/mode cells). One base vscode run
was an instruction-count outlier; the medians above include it. No qualifying win: the source change was removed.
Do not retry whole-worker outlining solely because the function is large; a narrower measured change is needed.

## Memory and an existing useful knob

t3code's memory census at check end, single/default: arenas hold 569.0/1,508.2 MiB of used allocations; the allocator
reports 193.8/564.3 MiB of live heap blocks. At parse end the corresponding arena usage is almost equal,
241.2/241.6 MiB. Direct checker-container census totals are 69.2/289.7 MiB, including some arena-backed link tables;
these totals overlap the arena/heap figures and must not be added to them. Arena used bytes are not a reachability
census, and none of these snapshot figures is interchangeable with process peak RSS.

The growth is overwhelmingly during checking. This agrees with [memory round 4](mem-round4.md) and
[the earlier instantiation profile](perf-t3code-instantiation.md): each checker builds its own type graph and caches.
Allocator tweaks and smaller front-end layouts do not address that duplication.

Same baseline binary, three alternating pairs, default (14 checkers) -> `--checkers 8`:

| project | total instructions | peak RSS | wall median |
| --- | ---: | ---: | ---: |
| t3code-server | 123.286 -> 87.766 G (-28.8%) | 2,083.6 -> 1,553.1 MiB (-25.5%) | 1.36 -> 1.36 s |
| vscode | 95.400 -> 91.634 G (-3.9%) | 2,134.7 -> 1,997.3 MiB (-6.4%) | 1.06 -> 1.25 s (+17.9%) |

Diagnostics and exit statuses match across all 12 runs. The single-project option is immediately usable; these
measurements do not justify changing the global default. Multithreaded counts also vary with stealing.

## Next micro-optimization experiments, ranked

1. **AST accessor dispatch.** `Node::expression`, `name`, `declaration_data`, `modifiers` and `initializer` together
   account for 2.46% of vscode self samples and 1.60% on t3code. This opportunity was also identified in
   [instruction round 4](perf-icount-round4.md#found-on-the-way-not-pursued).
   [Follow-up: AST accessor dispatch](perf-ast-accessor-dispatch.md) implements tag dispatch and generated constant
   offset tables: warmed Mac screening shows -1.99% single-threaded instructions on vscode and -1.16% on t3code;
   the Linux landing gate is still pending.
   The candidate preserves checked casts, optional values and mutable-cell reads.
2. **Narrow instantiation bookkeeping.** Use instruction-PC sampling to separate active-mapper scanning, cache
   probing and stack maintenance before changing them. The old five-byte-key hashing improvement already landed;
   it is not what the remaining 5.63% self samples mean. Whole-worker outlining failed above. Existing mapper-cache
   experiments in [the rejected list](perf-round2-followups.md#measured-and-rejected-do-not-redo) remain exclusions.
3. **Symbol lookup, only with new evidence.** It is still about 2% self on both traces, but prior hash changes,
   caches and lookup/insert combinations were flat or worse. Measure linear/indexed hit distributions before
   changing `SymbolMap`; the profile by itself does not establish another viable optimization.

No production code change is retained. No conformance count or supported capability changes. The required
workspace Clippy command, `tools/lint/source.py`, and `git diff --check` pass. Conformance was not rerun for the
reverted attribute-only experiment; diagnostic equality above covers the measured public workloads.
