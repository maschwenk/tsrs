# Non-checker traversal experiments (2026-10-11)

Neither candidate established a qualifying whole-project gain. Both were measured separately against the same
original compiler and then reverted; only these findings are retained. This follows the hotspots in
[the non-checker profile](perf-nonchecker-xctrace.md). Prior art was checked in `notes/perf-round2-followups.md`,
`notes/perf-parse.md`, `docs/RUST.md` Techniques and `notes/mem-round4.md` before the experiments.

## Binder child visitor specialization (2026-10-11, rejected)

The non-checker investigation (`notes/perf-nonchecker-xctrace.md`) found binding in 25.3% of VS Code's
single-threaded frontend stack observations. The candidate changed only `Binder::bind_each_child` from
`node.for_each_child(&mut |child| self.bind(child))` to `node.for_each_child_static` with the same closure.
The dynamic entry point already forwards to that generic implementation, so this specializes the existing
visitor without changing child order, short-circuit behavior or the pinned Go algorithm. The parser's existing
static parent visitor had helped previously (`notes/perf-parse.md`); this experiment measured the binder call site.

Base: `63ac5a23a4b6253e85378e6b4bb56e2fcec91731`; candidate: that compiler plus the one-line binder patch.
The existing profiling build uses release optimization, fat LTO, one codegen unit, line tables, no stripping and
no PGO, on the same 14-core arm64 Mac running macOS 27.0.1. The build uses
`DEVELOPER_DIR=/Library/Developer/CommandLineTools` because the selected older Xcode linker cannot read the
installed SDK. Public projects and checkout provenance are the same as the non-checker investigation:
Formbricks `ec45c28e4d20`, t3code `9bd1d8009a6b`, and the previously validated archive-backed VS Code checkout.

Each project/mode/binary had one warm-up and five retained runs. Base/candidate order alternated by repetition,
and mode order rotated; measurements ran sequentially. Full checks used `--noEmit --incremental false
--extendedDiagnostics --pretty false`. Single-threaded checks added `--singleThreaded` and
`RAYON_NUM_THREADS=1`; default checks cleared inherited thread/profiling overrides and used the default checker
count. Counters came from `/usr/bin/time -l`, not Instruments.

The before/after columns below are independent medians of the five runs. A paired delta is calculated for each
adjacent base/candidate repetition as `(candidate / base - 1) * 100`; the delta column reports the median of
those five percentages. These two summaries need not agree. Pairing helps describe changing machine conditions,
but does not eliminate noise or make the macOS instruction counter deterministic. Negative deltas mean less work
or memory. Instructions are billions; peak RSS is MiB.

| project | mode | instructions before → after | paired instruction delta, median | paired instruction delta, range | peak RSS before → after | paired RSS delta, median |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| vscode | single | 88.291 → 87.149 | -0.053% | -1.406% to +0.124% | 1605.188 → 1605.219 | +0.002% |
| vscode | default | 101.875 → 101.021 | +0.243% | -1.114% to +1.130% | 2089.125 → 2109.734 | +0.040% |
| formbricks-web | single | 44.278 → 45.370 | -0.202% | -0.610% to +2.972% | 1100.031 → 1100.109 | +0.007% |
| formbricks-web | default | 78.321 → 78.001 | -0.452% | -1.905% to +1.390% | 1973.297 → 1968.062 | +0.008% |
| t3code-server | single | 42.138 → 41.693 | -0.283% | -1.058% to -0.119% | 733.297 → 733.344 | +0.006% |
| t3code-server | default | 133.503 → 133.806 | +0.011% | -0.479% to +0.791% | 2134.703 → 2133.828 | -0.041% |

In particular, dividing VS Code's independent single-threaded medians suggests -1.293%, but the median paired
change is only -0.053%, and individual pairs span both directions. t3code's independent medians similarly suggest
-1.057%, versus -0.283% paired. Formbricks reverses direction between the summaries (+2.468% versus -0.202%).
These are noisy screening results, not evidence that the 1% single-threaded instruction gate was met.
macOS instruction counts include kernel work; the required deterministic Linux `bench/count.py` gate was not run.

Default peak RSS shows no qualifying reduction: the paired median changes are +0.040%, +0.008% and -0.041% for
VS Code, Formbricks and t3code. Their paired ranges are -0.090% to +1.771%, -0.907% to +0.239%, and -0.383% to
+0.910%, respectively. All are far from a repeatable 5% reduction. Wall time varied substantially; default paired
ranges were -6.897% to +8.850%, -15.888% to +12.941%, and -13.372% to +6.294%. This was not two README benchmark
publishes, so these timings do not establish the separate 2% wall-time gate.

Single-threaded frontend-only runs (`--listFilesOnly`, without extended diagnostics) also failed to establish a
consistent gain. Paired instruction medians were +0.092% / -1.005% / -0.080% for VS Code / Formbricks / t3code,
with ranges -4.584% to +5.368% / -4.228% to +0.906% / -5.163% to +0.053%. These runs exclude checking and are
not the whole-project instruction gate.

Correctness screening: all 12,760 compiler/conformance input files produced identical baseline/candidate binder
oracle hashes, with zero panics. The oracle covers binder-set flags, symbols, locals, exports, flow graphs,
diagnostics and symbol counts, including discovery order. Full-check diagnostics and exit codes matched per
project across all modes/repetitions; frontend file-list output and exit codes matched separately. This was a
baseline/candidate comparison, not a new claim of complete equivalence to the Go oracle or a full conformance run.

The change was rejected and the original binder source restored: no qualifying whole-project improvement was
established. Reconsider this call-site specialization only with new workload or code-generation evidence and
deterministic Linux instruction measurements; the existing sampled hotspot alone is insufficient.

Artifacts: `/tmp/tsrs-binder-static/` contains `candidate.patch`, `provenance.json`, `build.log`, `measure.py`,
`binder-measure.json`, `binder-summary.json`, raw run outputs/counters, and `oracle-validation.json`.

## Identifier leaf finishing (rejected)

`create_identifier_with_diagnostic` normally finishes a newly created `Kind::Identifier` through `finish_node`,
which sets its range and flags and dispatches a child traversal to install parent links. Identifiers have no
children. This candidate replaced that call on the successful identifier path with the same range, context/error
flag and `has_parse_error` handling, and the same final `current_parent = None`, without traversing children.
It left missing-identifier recovery unchanged. The range still ended at `self.node_pos()` after token advancement,
not at the preceding token's `token_end`; that distinction matters for source-backed identifiers.

This was independent of the binder candidate: no binder change was in the identifier binary. The baseline,
build settings, workloads, warm-up, five alternating pairs per cell and counters were as above. Before/after
columns are independent medians; delta columns are medians of the five adjacent paired percentages.
Instructions are billions; peak RSS is MiB.

| project | mode | instructions before → after | paired instruction delta, median | paired instruction delta, range | peak RSS before → after | paired RSS delta, median |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| vscode | single | 87.054 → 86.944 | -0.126% | -1.457% to +1.187% | 1605.188 → 1605.203 | +0.001% |
| vscode | default | 100.964 → 100.649 | -0.354% | -0.434% to +0.081% | 2103.531 → 2094.734 | -0.592% |
| formbricks-web | single | 44.135 → 43.928 | -0.467% | -0.773% to +0.395% | 1100.031 → 1100.047 | +0.001% |
| formbricks-web | default | 78.041 → 78.151 | -0.180% | -1.680% to +0.499% | 1970.812 → 1967.750 | -0.059% |
| t3code-server | single | 41.745 → 41.702 | -0.106% | -0.111% to -0.065% | 733.297 → 733.312 | +0.002% |
| t3code-server | default | 132.967 → 133.271 | -0.168% | -0.559% to +1.336% | 2121.219 → 2131.594 | -0.168% |

Single-threaded frontend-only paired instruction deltas were -0.286% / -0.646% / -0.500% for VS Code /
Formbricks / t3code (ranges -5.649% to +4.793% / -1.472% to +0.366% / -0.583% to +4.798%). Whole-project
single-threaded paired instruction medians were -0.126% / -0.467% / -0.106%. Default-mode paired peak RSS deltas were
-0.592% / -0.059% / -0.168%, with ranges -1.329% to +0.494% / -0.694% to -0.037% / -0.478% to +0.837%.
There is no demonstrated 5% memory gain. Default wall-time paired medians were +0.980% / -2.299% / -2.797%,
but every range spanned both directions and these were not two README benchmark publishes; those values do
not establish the wall-time gate either.

The AST oracle produced identical valid hashes for all **17,961** inputs, with no missing results, panics or
crashes: 17,319 previously split compiler/conformance units, 525 scanner edge cases, 113 current bundled libraries,
and four original simple cases covering two splitter output-name collisions. The corpus's reference commit matches
the current TypeScript pin; existing parse flags were preserved. The AST dump covers source ranges, flags,
parent links, text, diagnostics and counts. This compares the prototype with the original Rust compiler, not a
new Go-equivalence claim. Across all 108 project invocations, diagnostics and exit codes matched per project/mode;
frontend file lists matched, single-threaded full-check counters were identical, and stderr was empty.

The original parser source was restored. The change adds finishing logic that would need to stay synchronized
with `finish_node`, without evidence of a qualifying gain. Revisit only with materially different workload or
code-generation evidence and the deterministic Linux gate. The sampled cost of identifier construction alone
does not establish that this traversal is expensive enough to specialize.

## Reproduce and interpret

The experiments were screens on macOS, not qualifying Linux measurements. `time -l` includes kernel work.
In the binder run, identical single-threaded program counters accompanied distinct instruction clusters;
independent medians sometimes selected opposite clusters. Instructions correlated with system CPU time, but
the cause of the clusters was not established. Paired summaries describe the observations without proving the
true effect is below 1%. The decision is that no qualifying gain was established, not that Linux could never
show one. Neither prototype went to the Linux `bench/count.py` gate or a full conformance run because neither
is being retained. No speedup is claimed from xctrace: it still failed before recording, as documented in the
original investigation.

Build the original compiler, save its binary, apply exactly one candidate, then rebuild with identical settings:

```sh
DEVELOPER_DIR=/Library/Developer/CommandLineTools \
CARGO_PROFILE_RELEASE_STRIP=none CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
CARGO_TARGET_DIR=target/xctrace cargo build --release -p tsrs_cli --locked
```

From each public project root, full single-threaded counters use:

```sh
RAYON_NUM_THREADS=1 /usr/bin/time -l /absolute/path/to/saved-tsrs -p "$TSRS_PROJECT" \
  --singleThreaded --noEmit --incremental false --pretty false --extendedDiagnostics
```

For defaults, unset `RAYON_NUM_THREADS` and omit `--singleThreaded`. For the isolated single-threaded frontend,
replace `--extendedDiagnostics` with `--listFilesOnly`. Clear inherited `TSRS_`, `TS_TEST_` and `MIMALLOC_`
overrides too. Do not run builds or other measurements concurrently. Alternate base/candidate order and rotate
mode order; discard one warm-up per project/mode/binary. Use the public revisions and project arguments in
`bench/projects.json` (VS Code `src`, Formbricks `apps/web/tsconfig.typecheck.json`, t3code `apps/server`).

Local artifacts are in `/tmp/tsrs-binder-static/`: both patches, all three CLI binaries, `measure.py`,
`binder-{measure,summary}.json`, `identifier-{measure,summary}.json`, raw outputs/counters,
`oracle-validation.json`, `identifier-output-validation.json`, `compare-ast.py`, `identifier-ast.list`,
`identifier-ast-comparison.json`, both oracle hash sets and `experiment-binaries.json`. The AST oracle is built
with the same environment and `cargo build --release -p tsrs_parser --example ast_oracle --locked`; the binder
oracle uses `-p tsrs_binder --example binder_oracle`. Their `hash` modes read corpus lists on stdin. Saved
baseline CLI SHA-256: `83f2b434779a2c21e3295f29eec35ce0ff5939abd9c0c043b9b236a5863f5d7b`; binder candidate:
`194860a5cfb50e77e195a9d29ffe52e83f8e79f5d02418f282835243e4edf16c`; identifier candidate:
`e75b70d384a342295a8eb1b8cce5ac25cde7f853aa1e6eeb5f524ecc312f7a41`.

Final source checks: both `tools/lint/ratchet.py` and `tools/lint/source.py` pass. The ratchet reports three
existing findings, none new. Compiler source, lint baselines and README capability status are unchanged.
The pre-existing merge conflict in `notes/perf-round2-followups.md` was preserved; the new rejection entry is
outside its conflict markers.
