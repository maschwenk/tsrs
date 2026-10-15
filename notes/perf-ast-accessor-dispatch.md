# AST accessor dispatch, 2026-10-15

Base `6822eeed`, Apple M3 Max (14 cores, 36 GiB), macOS 27.0.1, xctrace 27.0. Follow-up to
[the xctrace investigation](perf-xctrace-2026-10-15.md), using only public projects pinned in
`bench/projects.json`.

The candidate reduces single-threaded instructions by 1.99% on vscode and 1.16% on t3code-server in warmed Mac
screening runs. Peak memory is essentially unchanged. The source change is retained for review; **the Linux
`bench/count.py` landing gate remains unverified**. Mac counts include kernel work and are not deterministic,
and these local wall times are not the two benchmark publishes required by the wall-time gate.

## Evidence and change

The original CPU Counters recording attributed 2.46% of vscode's running self samples to `Node::expression`,
`name`, `declaration_data`, `modifiers` and `initializer`. `notes/perf-icount-round4.md` also identified AST switches
as an untried opportunity. No previously rejected accessor experiment was repeated.

Two sources of dispatch overhead were visible in the release disassembly:

- Hand-written accessors switched on syntax kind, then their `as_*` calls checked the payload tag again.
  `expression`, `initializer`, `type_node`, `question_token` and `postfix_token` now switch on the payload tag.
  The checked casts remain in the source; LLVM can prove their checks redundant. `expression` still rejects a
  default clause, and both loop accessors still check the for-in/for-of kind aliases.
  Malformed nodes with mismatched kinds and payloads can return `None` where an accessor previously panicked
  (for example, `new_token(Kind::Parameter).question_token()`); valid AST behavior is unchanged in the suite.
- Generated `name`, `modifiers` and embedded-base accessors used indirect jump tables to select field loads or
  addresses. The generator now emits explicit constant offset arrays indexed by payload tag. Rust's
  `offset_of!` computes each concrete allocation's field location. Zero denotes a missing field. Mutable fields
  are accessed through their actual `OwnedCell` type; required and optional names retain separate typed reads.
  Generator checks reject unexpected name/modifier field types or storage. Pointer reconstruction uses the
  existing `arena_neighbor` provenance helper, as `Node::payload` does. No AST layout or allocation policy changes.

A first attempt expressed the offsets as matches. LLVM converted them back into indirect jump tables, so that
form was discarded. The explicit arrays retain data lookups and allow the generated accessors to inline.

| standalone function | baseline machine code | final candidate |
| --- | ---: | ---: |
| `expression` | 2,240 bytes | 192 bytes |
| `initializer` | 852 bytes | 252 bytes |
| `name` | 116 bytes | inlined; no standalone symbol |
| `modifiers` | 112 bytes | inlined; no standalone symbol |
| `declaration_data` | 140 bytes | inlined; no standalone symbol |
| `locals_container_data` | 108 bytes | inlined; no standalone symbol |

An inlined accessor still costs instructions inside its callers; disappearance of its symbol is not elimination
of all its work. The final CPU Counters recording has 7,854 running target-process samples; the generated
standalone accessor frames are absent, and their inlined work belongs to the callers. The process-level counts
below measure the net effect.

## Measurements

Both binaries use `CARGO_PROFILE_RELEASE_STRIP=none cargo build --release --locked -p tsrs_cli`, fat LTO, default
allocator and compressed pointers, without PGO. The baseline is the saved original binary. Commands use
`-p <project> --noEmit --incremental false --pretty false`. Single-threaded runs additionally set
`RAYON_NUM_THREADS=1` and pass `--singleThreaded`; default runs use the usual checker count (14 on vscode/t3code).

The final comparison warms both binaries on each project/mode, then runs five alternating baseline/candidate
pairs. The tables contain medians from unprofiled `/usr/bin/time -l` runs. All 60 measured runs have matching
diagnostic output hashes and exit status within each project. The earlier three-pair tag-only screen had large
timing outliers; its apparent wall-time improvement is not evidence of a speedup.

| project | single instructions, before → after | change | default peak RSS, before → after | change |
| --- | ---: | ---: | ---: | ---: |
| vscode | 83.246 → 81.588 G | -1.99% | 2,128.3 → 2,112.2 MiB | -0.76% |
| t3code-server | 39.290 → 38.835 G | -1.16% | 2,096.4 → 2,093.6 MiB | -0.13% |
| Compiler-Unions | 3.358 → 3.331 G | -0.81% | 83.8 → 83.8 MiB | -0.06% |

| project | default instructions, before → after | change | default wall, before → after | single wall, before → after |
| --- | ---: | ---: | ---: | ---: |
| vscode | 95.315 → 93.364 G | -2.05% | 1.02 → 1.01 s | 7.83 → 7.67 s |
| t3code-server | 123.429 → 121.889 G | -1.25% | 1.36 → 1.34 s | 3.29 → 3.26 s |
| Compiler-Unions | 4.040 → 4.011 G | -0.72% | 0.10 → 0.10 s | 0.22 → 0.22 s |

The largest default RSS difference is below 1%; there is no qualifying memory win. The stable instruction
reduction on vscode warrants Linux validation. Its baseline single-threaded instruction range was
83.217–83.587 G, versus 81.579–82.590 G for the candidate; the ranges do not overlap.

## Validation and reproduction

The pinned TypeScript checkout at `target/typescript-port-base` supplies the schema and conformance fixtures:
the default `ts-ref` fixture links were broken. The unmodified generator reproduces checked-in Rust byte for byte
with that schema. The modified generator is also checked for reproducible output.

- All 25 AST unit checks pass with both compressed and plain pointers. They cover optional names, absent fields,
  mutable expressions/types/modifiers, rare initializers, shared loop payloads and the default-clause panic.
- The full suite preserves all baseline pass sets: 13,458 diagnostics, 12,779 types and 12,779 symbols, with no
  crashes or timeouts. Canonical/default checking preserves diagnostic passes and has the documented
  `objectLiteralNormalization` type/symbol difference (12,778 each), matching CI's thresholds.
- Workspace Clippy and source lint pass with the commands in `AGENTS.md`; the local regression cases also pass.
  Miri has not been run.

Raw local evidence is under `target/ast-accessors-20261015/` (ignored): `table-pairs.json`, `table-comparison.log`,
`table-tsrs`, the `*-base.asm`/`*-table.asm` disassembly, conformance result sets, and the CPU Counters recordings
`vscode-offset.trace` (discarded match-based form) and `vscode-table.trace` (final form). Record with:

```sh
xcrun xctrace record --template 'CPU Counters' --output <output.trace> --time-limit 30s \
  --env RAYON_NUM_THREADS=1 --launch -- <binary> -p <vscode/src> \
  --noEmit --incremental false --pretty false --singleThreaded
xcrun xctrace export --input <output.trace> \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]' --output samples.xml
```

The Linux follow-up must build both revisions with identical settings and use `bench/count.py` on the pinned
projects, measuring default-count RSS as well as single-threaded instructions.
