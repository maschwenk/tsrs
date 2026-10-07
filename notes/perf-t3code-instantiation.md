# perf-t3code-instantiation: type instantiation and template-literal inference on t3code-server

Goal: exact cuts of at least 1% of single-threaded user-space instructions on t3code-server (or 1% of its check time
at 32 checkers on the 64-vCPU runner) in instantiation, inference and relation checking, with no project worse by more
than 0.3%. Exact means the same types, symbols and signatures created in the same order, identical
`--extendedDiagnostics` counters, byte-identical `--pretty false` output, also under `--checkerAssignment go`. Base:
origin/main 6baca34. One change kept (below); the rest were measured and dropped.

## Profile

`perf record -e instructions:u -c 200000 --call-graph fp` of t3code-server's check, single-threaded, and `-e cycles:u`
at 32 checkers, on the 64-vCPU runner (`--release` with frame pointers; `tools/perf/srclines.py` for lines; run
s0kz37f052). 47.0 G instructions sampled single-threaded, 87.1 G cycles at 32 checkers.

| symbol (self) | instructions, 1 checker | cycles, 32 checkers |
| --- | ---: | ---: |
| `instantiate_type_with_alias_worker` | 5.08% | 7.11% |
| `SymbolMap::search` (not this lane) | 2.67% | 2.65% |
| `get_property_of_type_worker` | 2.41% | 1.88% |
| libc `0x1986a5` (glibc string routine) | 0.55% | 2.18% |
| `CacheHashKey::hash_128` (out-of-line key hashing) | 0.86% | |
| `ReferenceInstantiations::get` / its rehash | 0.83% / | 1.09% / 1.31% |
| `Type::target` (out of line) | 0.74% | 0.91% |

Lines of `instantiate_type_with_alias_worker` (11,933 samples): **74.5% on one inlined line, `xxh3_128_4to8`**, the
xxh3-128 of the 5-byte key (`write_type` + `write_alias(None)`) of the active mapper cache; then pushing the active
mapper (1.7%), the cache's hashbrown probe (~5%), `find_active_mapper`'s scan (~1%). Instruction samples skid onto
the instructions after a multiply chain, so 74% overstates the hash's share of instructions, but it was the one site
in the function worth taking out.

Site counts (`--features site-counts`) of the same check: 11.0 M calls reach the worker; 3.24 M (29.5%) come with a
mapper that is not active, 6.52 M miss an active mapper's cache, 1.25 M hit it. In the first group the worker pushes
the mapper with a cleared cache, hashes the key, looks it up in that empty table, and never stores into it (Go stores
the result only when the mapper was already active).

The libc routine: its callers at 32 checkers (DWARF-free frame-pointer chains, so the leaf's direct caller is
skipped) are `infer_types_from_template_literal_type` <- `is_type_matched_by_template_literal_type` <-
`get_union_type_worker_inner`: union reduction removing string literals matched by a template literal
(`remove_string_literals_matched_by_template_literals`), done again in every checker. At one checker it is 0.55% of
instruction samples; at 32 it is 2.2% of cycles because the compared texts are cold in the checker that repeats it.

## Kept: the active mapper cache key (checker_11.rs)

1. When the mapper is not active, `push_active_mapper` gives it a cleared cache (a `debug_assert` checks it is
   empty), and this call stores nothing in it, so a lookup there can only miss. No key is built and no lookup made.
2. Without an alias, the key is the type id (`u32`) multiplied by an odd 64-bit constant (a bijection: two ids never
   share a key) in the low word, under a fixed high word, instead of the xxh3-128 of the id's 4 bytes and a zero byte.
   These caches are private to `instantiate_type_with_alias_worker` and only compare keys for equality (nothing
   iterates them). An aliased instantiation still uses the xxh3 key; it can equal an id key with the same 2^-128 odds
   as two xxh3 keys can equal each other. `PackedMap` hashes by the low word, which the multiply spreads into the top
   bits hashbrown uses for its control bytes.

Nothing created or cached changes; the same instantiations run in the same order.

## Result

`bench/count.py`, user-space instructions, single-threaded, 64-vCPU runner, base 6baca34, `--release`, base and
change built in the same job (runs n6crh8t9w0 and 42kqs90c3k, both counts identical across the two runs):

| project | base | 1 alone | 1 + 2 (kept) |
| --- | ---: | ---: | ---: |
| t3code-server | 50.386 G | -0.611% | **-1.002%** |
| vscode | 103.290 G | -0.161% | -0.207% |
| formbricks-web | 51.550 G | -0.526% | -0.710% |
| supabase-studio | 62.433 G | -0.571% | -0.930% |
| cal-diy | 40.136 G | -0.626% | -0.878% |

`--profile dist` (fat LTO), `tools/perf/abprobe.py`, 11 interleaved runs per cell at 32 checkers (medians,
min-max), run 42kqs90c3k:

| | t3code-server base | new | vscode base | new |
| --- | ---: | ---: | ---: | ---: |
| single-threaded instructions (count.py) | 49.492 G | 48.916 G (-1.16%) | 100.646 G | 100.410 G (-0.23%) |
| 32 checkers, instructions:u (perf stat, one run) | 191.52 G | 186.44 G (-2.65%) | 124.12 G | 123.57 G (-0.44%) |
| 32 checkers, Check time | 1362 ms (1328-1419) | 1349 ms (1302-1394) | 429 ms (414-441) | 432 ms (412-451) |
| 32 checkers, summed user CPU | 18.95 s | 18.58 s (-2.0%) | 15.10 s | 14.98 s |
| peak RSS | 2867 MiB | 2861 MiB | 2716 MiB | 2709 MiB |

At 32 checkers t3code-server's instructions fall more than single-threaded (-2.65% against -1.16%): every checker
re-instantiates the shared Effect service graph (notes/perf-memory-traffic-32.md), so the per-instantiation saving is
paid 32 times. The check-time medians move by -1.0% (t3code-server) and +0.7% (vscode), both inside the run-to-run
spread. Peak RSS (count.py) within 2 MiB.

## Dropped

| candidate | exact | result (count.py, single-threaded, t3code-server unless stated) | why dropped |
| --- | --- | --- | --- |
| B: template-literal inference tests the length and the first (last) byte before `starts_with` (`ends_with`) | yes | -0.019% (cal-diy -0.090%, others -0.01%) | the mismatching prefix and suffix tests are not where those instructions go; which comparison on the matching path the libc routine runs was not resolved (frame-pointer chains skip a leaf's direct caller), and at 32 checkers its cost is cold data more than instructions |
| key builder without zeroing its 192-byte buffer | yes (needs `MaybeUninit` and `unsafe`) | not built: 5.06 M key builders per check (site count), ~15 instructions of stores each, at most 0.15% | below the bar for new `unsafe` |
| `ReferenceInstantiations` rehash | | not tried | the two exact changes to it are in notes/perf-memory-traffic-32.md; nothing new to try |
| C: `#[inline]` on `Type::target` (0.74% self, out of line) | yes | +0.064% (vscode +0.334%, formbricks-web +0.208%, supabase-studio +0.242%, cal-diy +0.303%; run dmxcsjp8sd) | worse: inlining its flag dispatch into every caller costs more than the call |

Not tried (ruled out by earlier notes or by the profile): skipping the creation of a failing conditional's instantiated extends
type (notes/perf-heavy-files.md), sharing variances or types between checkers (same note, notes/mem-shared-base.md),
`SymbolMap` / name resolution (another lane). `is_deeply_nested_type` (0.4% of samples) and `get_object_type_instantiation`
(0.6%, mostly its two hash probes) have no redundant step: their lines are the walk and the probes themselves.

## Gates

Local (Mac), the kept change's release binary against a 6baca34 binary built here: `--pretty false` output
byte-identical on vscode, t3code-server, formbricks-web, supabase-studio, cal-diy, mui-docs, webpack and xstate-main,
single-threaded and at 4 and 32 checkers and under `--checkerAssignment go` (32 of 32 cells); `--extendedDiagnostics`
counters identical single-threaded (timing rows excluded). Conformance (`.github/scripts/conformance-gate.sh`,
reference checkout at b85298b6a): 13,458 error baselines, 12,779 `.types`, 12,779 `.symbols`, no crash or timeout.
A `dev` build (debug assertions on) checks t3code-server with output identical to the base. `cargo check
--workspace` without warnings, `tools/lint/ratchet.py`, `tools/lint/source.py`, `cargo test -p tsrs_checker -p
tsrs_compiler`. On the runner: diagnostics, `--listFiles` and `--explainFiles` identical (abprobe, t3code-server and
vscode).
