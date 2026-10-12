# Shared storage and Rayon batch checking

Implementation baseline: `b8bdd76bf789975cbc4d558d9d42fa78675abfd1`. The owner requested the complete batch
executor/storage separation in an isolated worktree, with performance measured after implementation.

Except for the post-rebase validation section, the validation and measurements below are historical. Final candidate `4260fa6b` was
compared with baseline `b8bdd76b`; the earlier implementation and screening stages are labeled separately.
The branch is now rebased onto `d8b0959e`. That rebased production source has not been measured, so the recorded
performance results and source manifest do not attest it. The architecture described below remains the design
of the rebased implementation.

## Ownership and execution

The ordinary non-incremental CLI `--noEmit` path has a `BatchCheck` executor instead of leasing checkers from the
compiler pool. A dedicated Rayon executor runs persistent `CheckContext`s, with the existing checker-count,
locality, heavy-file, split-file and work-claiming policies. Each context has a permanent scratch arena, entered by
a restoring guard on every job. Its leaked owner guarantees process lifetime, preserving the old thread arena's
destructor-free allocation and recycling rules even when a context moves between workers. Recursive checking
stays synchronous within the context. The program retains its contexts and shared input; escaped diagnostics
retain the old lifetime rules. The CLI backend runs one complete semantic pass. `Checker` remains a
source-compatible alias for the evaluation context used by the checking algorithms.

`SharedCheckData` retains borrowed program input and owns the file-index map, immutable settings snapshot and
payload interner. Types, symbols and signatures retain their local semantic identities. Resolution stacks,
depth/complexity budgets, inference state, relations, lazy resolutions, diagnostics and identity-bearing edges
remain local. Constructors create local identities and charge the same budgets even when immutable storage is
reused.

The interner has 16 mutex-protected shards. A miss copies a complete payload into a shard-owned arena; a hit reuses
it after exact equality, rather than trusting its hash. Ordinary payload reads are direct dereferences, without
locking, refcount increments or a generic lazy-field overlay. Shared records never contain a context-local type,
symbol, signature, mapper or AST node. No evaluator waits for another evaluator's in-progress semantic resolution.

The shared payloads are:

- Literal values, with floating-point bits preserved, including signed zero and individual NaN encodings. Literal
  type objects and their fresh/regular relationships stay local.
- Tuple scalar metadata and unlabeled element-info arrays. Labels, element symbols, type parameters and other
  identity-bearing bindings stay local.
- Template-literal text arrays and completed lazy-member unaffected-name arrays, with their text copied into the
  shared region. The decisions that produced each array are still evaluated locally.
- Completed variance arrays, shared by their final bytes rather than by another context's variance computation.
- Allocated intrinsic, synthetic tuple-index and unique-symbol name strings.

This implements shared immutable storage, not a globally canonical semantic type graph. In particular,
signature parameter/type-parameter/return-type lists and instantiated member tables contain local identities;
interning structurally similar lists would merge identities. Adding an unused shared descriptor beside those
lists would only duplicate metadata, so they remain the local bindings. The measurements below include the
remaining local storage rather than counting it as a sharing win.

## Compatibility and controls

`TSRS_CHECK_ENGINE=shared` selects the new backend where supported (also the worktree's default), and `legacy`
selects the old backend. External pools always take precedence. Go-compatible history, explicit assignments,
declaration diagnostics, emit, incremental/build, LSP/API, memory-target retirement, the frozen-seed experiment,
explain-files and assignment/cost tracing retain the existing backend. The test harness remains on the public
compatibility API; standalone CLI differential cases exercise the new path separately.

The compatibility backend retains its allocation ownership, but literal values and tuple metadata also use the
new representation there. Before/after measurements therefore use the original baseline binary, rather than
treating `TSRS_CHECK_ENGINE=legacy` in the candidate as an unchanged baseline.

`--extendedDiagnostics` reports `Check contexts`, retained context arena bytes, and shared payload request/storage,
arena, index-table and hit/miss counts. Avoided payload bytes are an allocation-volume counter, not peak RSS saved.

## Why this differs from earlier experiments

The historical frozen-seed design warms one complete checker and overlays its lazy fields in other checkers
(`notes/spike-shared-graph.md`, `notes/design-shared-type-layer.md`). This implementation has no seed, no serial
warm-up and no overlay lookup on every lazy-field read. It publishes only completed immutable payloads and retains
local semantic history. Its costs are constructor-time hashing/publication, extra payload indirections, shared
index tables and the remaining local bindings. The implementation was requested before measuring these costs;
it does not establish that they pay for themselves.

## Historical validation and measurements

The baseline and candidate use the same pinned TypeScript reference, `b85298b6a81f`. Full conformance result lists
match exactly: 13,458 passing diagnostic baselines, two code-only matches and two failures; all 12,779 type and
symbol baselines pass. Forced multithreading under canonical history also matches its baseline lists, with 12,778
type and symbol passes. Neither mode has crashes or timeouts. These harness runs use the compatibility API.

The standalone CLI corpus materializes 1,542 recursive, generic, contextual, union, tuple, circular, instantiation
and inference cases. Exact stdout, stderr and status match the original binary in all three configurations:
four contexts, `--singleThreaded`, and four contexts with `TSRS_LAZY_MEMBERS=0` (4,626 comparisons). Declaration
diagnostic cases intentionally cover the compatibility route; the remaining eligible cases exercise `BatchCheck`.

Seven payload tests cover concurrent interning, full-hash collisions, signed-zero/NaN bits, local type identity and
fresh links, creator-region destruction, and labeled-tuple fallback. Compiler tests cover persistent contexts,
shared input ownership and external-pool precedence. The CLI regression matrix compares both engines at one, two
and four contexts, including arena poisoning, shared-object write checks and eager members. The core, scanner and CLI suites pass (one existing Miri negative-control test remains
ignored), including leaf freeing, memory-target retirement, split/shadow checking, emit, incremental builds and LSP
edit coverage. The split matrix explicitly pins the shared engine.

Workspace and WebAssembly (`wasm32-wasip1`) checks pass, as does compilation with `shared-graph,alloc-profile`.
The lint ratchet and source checks pass with no new findings. All eight generators complete without changing any
of the 1,196 crate files; the generator script's final clean-diff check sees the uncommitted implementation, so
regeneration was verified separately with matching before/after file hashes.

## Initial measurements after implementation (2026-10-11)

Both binaries use the repository's standard release profile (fat LTO, one codegen unit), built with Rust 1.99.0
and `DEVELOPER_DIR=/Library/Developer/CommandLineTools`. The baseline is the pinned commit above; the candidate
includes the shared-variance lifetime fix, before the allocator follow-up below. SHA-256: baseline
`d4a22a4e6b79d7f7ab92f62051169fdc314ba594337d0c35a68e6513228a5cc8`; candidate
`29832a9a038f3d5dab6f602644886a108eb9525861410b5494eee0c7c90523db`.

Host: macOS arm64, Apple M3 Max, 14 logical cores, 36 GiB RAM. Two separate passes each use three repetitions,
interleaving the original and candidate binaries and reversing their order every other repetition, after one
warm-up per binary/project. Tables show medians, original → candidate. There are 144 measured invocations across
default and single-threaded modes. This is a local comparison, not a README benchmark publish.

Other sessions were compiling throughout: each invocation records a process-activity sample, with 5–31
`cargo`/`rustc`/`clippy-driver` processes and aggregate compiler CPU usage ranging from 109.7% to 821.8%.
Wall time is therefore exploratory, not evidence for the wall-time landing criterion. Instructions come from
macOS `/usr/bin/time -l`, not the deterministic Linux user-space counter in `bench/count.py`.

Available public workloads: Compiler and Compiler-Unions from the benchmark suite at `41f652ab2df5`, and
xstate-main at `fbee62e7c158` have verified commits and unchanged tracked source. The installed vscode snapshot
has no git metadata, so its source pin was not independently verified. No private corpus was used.

The standard Compiler configurations request declaration diagnostics and retain the legacy backend. Their
explicit `--declaration false` rows are separate workload variants; no tsconfig was edited. `Check contexts:`
in the candidate's extended diagnostics confirms the backend and context count shown below. All measured runs
have identical diagnostic lines and exit status between binaries (Compiler 43 errors, Compiler-Unions 41,
xstate-main 0, vscode 371).

### Default peak RSS (MiB)

| Workload/backend | Pass 1 | Pass 2 |
| --- | ---: | ---: |
| Compiler (legacy) | 78.0 → 78.2 (+0.1%) | 76.2 → 78.2 (+2.5%) |
| Compiler-Unions (legacy) | 83.7 → 83.5 (-0.2%) | 82.7 → 83.8 (+1.3%) |
| xstate-main (shared, 7 contexts) | 256.7 → 268.5 (+4.6%) | 256.8 → 269.4 (+4.9%) |
| vscode snapshot (shared, 14 contexts) | 2090.7 → 2130.2 (+1.9%) | 2096.6 → 2119.8 (+1.1%) |
| Compiler, `--declaration false` (shared, 4) | 77.7 → 78.7 (+1.3%) | 77.3 → 78.9 (+2.1%) |
| Compiler-Unions, `--declaration false` (shared, 4) | 81.8 → 84.4 (+3.1%) | 82.6 → 85.1 (+2.9%) |

### Single-threaded instructions retired (billions)

| Workload/backend | Pass 1 | Pass 2 |
| --- | ---: | ---: |
| Compiler (legacy) | 1.905 → 1.917 (+0.6%) | 1.905 → 1.912 (+0.4%) |
| Compiler-Unions (legacy) | 3.564 → 3.589 (+0.7%) | 3.559 → 3.588 (+0.8%) |
| xstate-main (shared) | 6.296 → 6.555 (+4.1%) | 6.487 → 6.328 (-2.4%) |
| vscode snapshot (shared) | 88.331 → 89.030 (+0.8%) | 88.354 → 90.257 (+2.2%) |
| Compiler, `--declaration false` (shared) | 1.915 → 1.933 (+0.9%) | 1.918 → 1.930 (+0.6%) |
| Compiler-Unions, `--declaration false` (shared) | 3.569 → 3.615 (+1.3%) | 3.570 → 3.612 (+1.2%) |

### Default wall time (seconds; competing builds present)

| Workload/backend | Pass 1 | Pass 2 |
| --- | ---: | ---: |
| Compiler (legacy) | 0.121 → 0.140 (+15.7%) | 0.117 → 0.110 (-6.3%) |
| Compiler-Unions (legacy) | 0.179 → 0.169 (-5.8%) | 0.193 → 0.181 (-6.2%) |
| xstate-main (shared, 7 contexts) | 0.227 → 0.280 (+23.3%) | 0.228 → 0.253 (+11.1%) |
| vscode snapshot (shared, 14 contexts) | 2.432 → 3.163 (+30.0%) | 2.624 → 2.889 (+10.1%) |
| Compiler, `--declaration false` (shared, 4) | 0.159 → 0.248 (+55.7%) | 0.154 → 0.194 (+26.0%) |
| Compiler-Unions, `--declaration false` (shared, 4) | 0.256 → 0.319 (+24.8%) | 0.238 → 0.223 (-6.2%) |

### Retained storage accounting

Candidate default-mode medians across both passes, in MiB. Context figures include local identities, bindings,
caches and evaluation storage; they are not just the immutable payloads. Shared arena figures include payload
record/alignment overhead. Index-table bytes are retained hash-table allocation capacity, reported separately.
Arena capacity is not resident memory. Requested minus stored bytes measures avoided payload allocation volume,
not RSS saved. Some string bytes were borrowed in the original representation, so these request counters are not
an estimate of how many bytes the original binary allocated.

| Shared workload | Payload requested / stored | Shared arena used / capacity | Index tables | Context arenas used / capacity |
| --- | ---: | ---: | ---: | ---: |
| xstate-main (shared, 7 contexts) | 2.681 / 0.367 | 0.390 / 4.000 | 0.063 | 90.856 / 102.591 |
| vscode snapshot (shared, 14 contexts) | 52.413 / 16.207 | 16.987 / 19.027 | 2.328 | 764.888 / 853.488 |
| Compiler, `--declaration false` (shared, 4) | 1.929 / 0.283 | 0.295 / 4.000 | 0.049 | 16.945 / 19.073 |
| Compiler-Unions, `--declaration false` (shared, 4) | 1.810 / 0.270 | 0.282 / 4.000 | 0.047 | 18.117 / 19.789 |

The initial implementation did not demonstrate a qualifying improvement. Default peak RSS increased on every workload
that enters the shared backend. Single-threaded instructions do not improve consistently across both passes;
xstate's sign changes between passes. No wall-time claim is justified under the measured external load. These
initial results did not support landing that version as a performance improvement. In particular, sharing tens of
MiB of payload requests on vscode did not offset its remaining local graph, representation, arena and interner
costs. The allocator and context-lifetime follow-up below revisits the measured overhead.

### Reproduction

Run both binaries in the same public project directory:

```sh
/usr/bin/time -l "$binary" -p "$project" --noEmit --incremental false --pretty false --extendedDiagnostics
/usr/bin/time -l "$binary" -p "$project" --noEmit --incremental false --pretty false --extendedDiagnostics --singleThreaded
# Explicit Compiler/Compiler-Unions variant: append --declaration false to both binaries' commands.
```

The `-p` arguments are `.` for Compiler/xstate-main, `tsconfig.json` for Compiler-Unions, and `src` for vscode.
Use the standard release profile for both commits, repeat three times per mode with alternating binary order,
and repeat that whole pass. Keep ordinary commands distinct from declaration-disabled variants and inspect
`Check contexts:` before attributing a result to the shared backend.

Local raw stdout/stderr, per-run JSON, binary hashes, backend probes, the materialized differential corpus and
harness scripts are retained under `target/shared-checker-validation/` in the isolated worktree. The four
measurement directories are `measure-one`, `measure-two`, `measure-no-declarations-one` and
`measure-no-declarations-two`; each includes `runs-enriched.json` with retained-capacity counters and sampled
competing-build activity. Those generated artifacts are not repository files.


## RSS follow-up (2026-10-12)

The owner requested continuing until RSS was lower. The initial candidate's compatibility backend measured near
the original binary (xstate -0.02%, vscode +0.36%), isolating the regression to the new batch path. The important
cost was freeable-region bookkeeping, not simply unused virtual chunk capacity:

- Each `needs_drop` arena allocation added a 24-byte destructor entry. The retained lists alone occupied 9 MiB
  on xstate and 48 MiB on vscode (236,831 and 1,635,941 entries).
- A freeable region also refused to recycle those blocks because the destructor list still referred to them.
  The CLI's original thread arenas neither recorded destructors nor imposed that restriction.

`Region::new_permanent_scratch` now leaks its owning handle explicitly, guaranteeing process lifetime even during
unwinding. Permanent context arenas retain the old thread-arena destructor/recycling rules and downward chunk
growth, including Linux huge-page advice and unused-tail trimming. Ordinary freeable regions keep their existing
destructor and slab behavior. Regression coverage verifies destruction for ordinary regions, no destruction for
permanent regions, reuse across thread migration, and valid pointers across growth and repeated trimming. Linux
and wasm checks compile the new paths; Linux huge-page behavior was not runtime-tested on this macOS host.

Fresh literal creation also reuses the regular literal's existing immutable payload pointer. The fresh Type is
still allocated with its own ID and fresh/regular links; it avoids a redundant hash/lock/lookup (and a duplicate
payload record when sharing is disabled).

### Screening and rejected tuning

Intermediate comparisons used matching release settings with only LTO disabled in both binaries, for fast
iteration. These are screening results, separate from the normal-profile measurements:

- Freeable → permanent context arenas in the same candidate: xstate 270.77 → 256.69 MiB (-5.20%), vscode
  2123.16 → 2063.55 MiB (-2.81%). Destructor entries became zero. Against the original binary, the permanent-only
  stage was near flat on xstate and about 1.1% lower on vscode.
- Reducing each interner shard's initial allocation from 64 KiB to 4 KiB cut xstate's virtual shared capacity from
  4 MiB to about 0.52 MiB but did not clearly lower RSS: isolated medians were +0.14% on xstate and -0.24% on vscode.
  Rejected; the store keeps 64 KiB chunks. Untouched virtual capacity must not be counted as resident savings.
- Matching permanent contexts to the old thread-arena chunk policy did not establish an independent macOS RSS
  gain (isolated xstate +0.24%, vscode +0.58%, with scheduling variation). It preserves the original permanent-arena
  allocation behavior, including Linux huge pages; no separate performance benefit is claimed.

A separate 64 → 16 shard comparison used the same permanent-context/fresh-payload implementation on both sides,
five repetitions and no competing compiler processes. Compiler and Compiler-Unions (declarations disabled)
medians fell 1.32% and 1.28%; xstate was flat (+0.08%) and vscode -0.16%. Instructions changed +0.10/-0.21/+0.04/-0.10%
and wall time -0.03/-0.11/-0.27/-0.48% for xstate/vscode/Compiler/Compiler-Unions. This reduces fixed per-shard costs
without showing a contention regression in these workloads. Keep 16 shards; every shard still starts at 64 KiB.

A completed-context cleanup experiment saved final globals/counters, released five relation tables and dropped
checker-owned maps once that job exhausted its work. It passed all 1,542 CLI comparisons and retained exact
single-context counters and lazy-member statistics. Live heap at check end fell by about 22 MiB on xstate and
204 MiB on vscode, but this was after peak usage: n=5 isolated peak medians were xstate -0.04%, vscode +0.19%.
Rejected and reverted; the final implementation retains contexts. Releasing memory after the peak does not meet
the peak-RSS criterion, and the extra finalization/snapshot contract did not justify itself.

All screening comparisons preserved exact diagnostic output and exit status. Raw stage results and preserved
binaries live under `target/shared-checker-validation/`; none of the temporary tuning environment switches remain.

### Final pre-rebase release measurements

The final candidate retains permanent context arenas with thread-style chunk allocation, reuses immutable
literal payload pointers, and uses 16 interner shards. It does not contain completed-context finalization or
the 4 KiB shared-arena experiment.

These are two local interleaved comparisons on the same Apple M3 Max (14 cores, 36 GiB RAM), with one warm-up
per binary/project followed by three measured repetitions per binary and mode. Both binaries use the workspace
release profile with fat LTO, built with `DEVELOPER_DIR=/Library/Developer/CommandLineTools`. The earlier
LTO-disabled runs above were screening experiments; the numbers below use the normal release profile.

- Baseline: `b8bdd76bf789975cbc4d558d9d42fa78675abfd1`, SHA256 `d4a22a4e6b79d7f7ab92f62051169fdc314ba594337d0c35a68e6513228a5cc8`.
- Final pre-rebase candidate: `4260fa6b`, SHA256 `c26aef2b004b54155d153a2ff0cf69875054f6a2cde7e6f2c78353fd38779759`.

Command for each timed invocation (from the project directory):

```sh
/usr/bin/time -l <binary> -p <project> --noEmit --incremental false --pretty false --extendedDiagnostics
```

The single mode adds `--singleThreaded`. The explicitly labeled Compiler variants add `--declaration false`;
the standard configs remain untouched. Default checker counts are unchanged. The harness alternates
baseline/candidate order on successive repetitions.

Public inputs: Compiler and Compiler-Unions from suite commit `41f652ab2df5077b1115f73eaeefc2fe9f674132`;
xstate from `fbee62e7c1586315ed478c2fedf530d7e0ff5a3e`; the same installed public VS Code snapshot used above,
whose checkout has no Git metadata and therefore no independently verified revision. Project arguments are
`.`, `tsconfig.json`, `.`, and `src`, respectively.

Each cell reports baseline → final median, followed by the relative change. Retired instructions are macOS
`/usr/bin/time -l` hardware counters, not the deterministic Linux `bench/count.py` result.

**Pass one**

| Project / mode | Peak RSS (MiB) | Instructions (billions) | Wall time (s) |
|---|---:|---:|---:|
| Compiler / default | 78.23 → 78.23 (+0.00%) | 2.246 → 2.250 (+0.18%) | 0.071 → 0.073 (+2.13%) |
| Compiler / single | 58.53 → 58.66 (+0.21%) | 1.902 → 1.902 (-0.01%) | 0.158 → 0.162 (+2.76%) |
| Compiler-Unions / default | 83.61 → 83.45 (-0.19%) | 4.324 → 4.343 (+0.44%) | 0.114 → 0.114 (+0.06%) |
| Compiler-Unions / single | 60.94 → 61.08 (+0.23%) | 3.557 → 3.582 (+0.70%) | 0.250 → 0.250 (+0.08%) |
| xstate-main / default | 257.98 → 256.55 (-0.56%) | 8.279 → 8.275 (-0.05%) | 0.129 → 0.130 (+0.34%) |
| xstate-main / single | 183.38 → 182.88 (-0.27%) | 6.279 → 6.292 (+0.20%) | 0.552 → 0.555 (+0.58%) |
| vscode / default | 2107.86 → 2062.66 (-2.14%) | 101.135 → 101.227 (+0.09%) | 1.599 → 1.783 (+11.46%) |
| vscode / single | 1599.00 → 1596.62 (-0.15%) | 87.019 → 88.344 (+1.52%) | 8.309 → 11.200 (+34.79%) |
| Compiler (`--declaration false`) / default | 78.36 → 77.89 (-0.60%) | 2.244 → 2.259 (+0.66%) | 0.071 → 0.073 (+2.05%) |
| Compiler (`--declaration false`) / single | 58.52 → 58.73 (+0.37%) | 1.900 → 1.909 (+0.50%) | 0.159 → 0.157 (-1.31%) |
| Compiler-Unions (`--declaration false`) / default | 83.67 → 83.77 (+0.11%) | 4.325 → 4.351 (+0.61%) | 0.113 → 0.114 (+1.20%) |
| Compiler-Unions (`--declaration false`) / single | 60.95 → 61.14 (+0.31%) | 3.556 → 3.586 (+0.85%) | 0.246 → 0.253 (+2.85%) |

**Pass two**

| Project / mode | Peak RSS (MiB) | Instructions (billions) | Wall time (s) |
|---|---:|---:|---:|
| Compiler / default | 78.28 → 77.92 (-0.46%) | 2.247 → 2.249 (+0.11%) | 0.073 → 0.071 (-2.98%) |
| Compiler / single | 58.53 → 58.66 (+0.21%) | 1.899 → 1.901 (+0.11%) | 0.156 → 0.160 (+2.52%) |
| Compiler-Unions / default | 83.56 → 84.14 (+0.69%) | 4.324 → 4.340 (+0.37%) | 0.114 → 0.115 (+1.04%) |
| Compiler-Unions / single | 60.97 → 61.08 (+0.18%) | 3.556 → 3.579 (+0.66%) | 0.244 → 0.247 (+1.34%) |
| xstate-main / default | 258.16 → 256.70 (-0.56%) | 8.245 → 8.269 (+0.29%) | 0.125 → 0.128 (+2.28%) |
| xstate-main / single | 183.41 → 183.28 (-0.07%) | 6.262 → 6.278 (+0.26%) | 0.531 → 0.536 (+0.93%) |
| vscode / default | 2113.52 → 2064.62 (-2.31%) | 100.574 → 100.826 (+0.25%) | 1.217 → 1.230 (+1.08%) |
| vscode / single | 1597.52 → 1597.41 (-0.01%) | 86.851 → 87.117 (+0.31%) | 8.117 → 8.223 (+1.30%) |
| Compiler (`--declaration false`) / default | 78.50 → 77.97 (-0.68%) | 2.245 → 2.262 (+0.75%) | 0.072 → 0.074 (+3.12%) |
| Compiler (`--declaration false`) / single | 58.52 → 58.70 (+0.32%) | 1.899 → 1.909 (+0.51%) | 0.158 → 0.158 (-0.18%) |
| Compiler-Unions (`--declaration false`) / default | 83.58 → 83.11 (-0.56%) | 4.323 → 4.357 (+0.77%) | 0.112 → 0.114 (+2.13%) |
| Compiler-Unions (`--declaration false`) / single | 60.94 → 61.14 (+0.33%) | 3.556 → 3.586 (+0.84%) | 0.245 → 0.256 (+4.43%) |

All 144 measured invocations preserve each project’s diagnostic hash and exit status. Candidate backend probes
show shared checking for xstate (7 default contexts), VS Code (14), and declaration-disabled Compiler variants
(4); single-threaded shared runs use one context. The standard Compiler configs keep declaration diagnostics
enabled and use the compatibility backend even with `--noEmit`, so their rows do not measure the new batch
path.

No task-owned builds or verification ran during measurement. External compiler activity was sampled before
every invocation: the first standard pass observed 0–3 cargo/rustc/clippy processes and 0.0–100.0% aggregate
CPU; the second standard pass and both declaration-disabled passes observed zero compiler processes in every
sample. These samples cannot prove the host stayed quiet between samples; wall-time and small instruction
changes need that qualification.

Raw output, per-run metrics, process samples, and summaries are preserved under
`target/shared-checker-validation/measure-final-{one,two}` and `measure-final-no-declarations-{one,two}`. The
exact pre-rebase production-source file hashes and release binary hashes are in `final-release-source.json`.

**Retained allocation accounting**

Pass-two default-mode medians from `--extendedDiagnostics` follow. These distinguish bytes stored in the
immutable payload store, retained interning tables, arena use, and arena capacity; none is a direct estimate
of RSS saved. Requested bytes count requests, including payloads the baseline could sometimes borrow from
existing program text. Arena capacity is virtual allocation capacity, not resident memory.

| Project | Payload requested / stored (MiB) | Shared arena used / capacity (MiB) | Interner tables (MiB) | Context arenas used / capacity (MiB) |
|---|---:|---:|---:|---:|
| xstate-main | 2.345 / 0.367 | 0.390 / 1.000 | 0.059 | 89.474 / 105.000 |
| vscode | 34.280 / 16.205 | 16.986 / 19.973 | 2.406 | 748.975 / 882.000 |
| Compiler (`--declaration false`) | 1.002 / 0.283 | 0.295 / 1.000 | 0.048 | 16.609 / 28.000 |
| Compiler-Unions (`--declaration false`) | 0.944 / 0.270 | 0.282 / 1.000 | 0.045 | 17.878 / 28.000 |

Context drop-entry counts and drop-list capacity bytes are zero on all final shared-backend measurements. The
permanent-arena change removes the initial candidate’s destructor-bookkeeping regression; reduced interner
shards improve the controlled small-project screening comparison. Neither mechanism should be described as
turning all payload request deduplication into RSS savings.

Across the two normal-release passes, default RSS is lower for VS Code by 2.14% and 2.31%, for xstate by 0.56%
in both, and for the explicit Compiler variant by 0.60% and 0.68%. Compiler-Unions is effectively flat (+0.11%
and −0.56%). The first pass’s VS Code wall-time increase (+11.46% default, +34.79% single) does not reproduce
in pass two (+1.08%, +1.30%). One first-pass single-thread pair records essentially identical cycles (33.723
billion versus 33.710 billion), but elapsed time changes from 8.309 to 11.232 seconds despite zero sampled
compiler processes. CPU placement or frequency differences are a plausible explanation for part of the timing
variation; the measurements do not prove the cause or demonstrate a speedup.

These results do not establish the repository’s performance landing bar: default peak-RSS improvements remain
below 5%, there is no repeatable 2% wall-time gain, and there is no 1% single-threaded instruction reduction.
The macOS instruction proxy also does not replace deterministic Linux `bench/count.py` qualification.

The final fat-LTO binaries were separately checked against the original baseline: all 4,626 materialized CLI
comparisons match exactly across four contexts, single-threaded execution, and eager members; all 29
regression fixtures pass; and every normal/canonical conformance classification list is byte-identical to its
corresponding baseline. No crashes or timeouts were introduced.

## Post-rebase validation

Before opening the draft, the implementation was rebased onto `d8b0959e`. The resolution preserves upstream's
lazy-unaffected-member cache and thief admission limits: completed name lists enter shared storage when the
cache creates them, and each Rayon context keeps its thief slot across queue claims.

On the rebased source, `cargo check --workspace`, the core/scanner/checker/compiler/CLI suites (178 passing,
one existing ignored case), both lint gates, and `cargo check -p tsrs_cli --features shared-graph,alloc-profile`
pass. All 29 regression fixtures pass with `TSRS_LAZY_UNAFFECTED=shadow`; the shared-checking and split-check
CLI suites also pass with both that shadow check and `TSRS_STEAL_MAX_THIEVES=1` enabled. Full conformance and
performance were not rerun after the rebase; their results above apply only to the earlier source.
