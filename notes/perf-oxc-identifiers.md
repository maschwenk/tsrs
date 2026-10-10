# Oxc identifier techniques (2026-10-10)

Isolated experiment on `codex/oxc-identifier-perf`, based on `d2e80d306f27`. The owner asked to apply the
techniques from [Oxc's identifier.rs](https://github.com/oxc-project/oxc/blob/4b756621b65ab54c19dc66fbdcb3cd902d662c20/crates/oxc_syntax/src/identifier.rs).
The existing parser note (`perf-parse.md`) already measured ASCII identifier scanning through a byte table;
this experiment adds the packed start/continue table and whole-name block checks from Oxc. It does not revisit
the rejected scanner dispatch or whitespace-skipping experiments.

Applied in the experimental worktree:

- One 128-byte ASCII table, aligned to 64 bytes, packs start and continue flags together. The scanner's ASCII
  loop and character predicates share it, replacing the 256-byte continue-only table and repeated range checks.
- Inline character predicates dispatch ASCII to the table and non-ASCII to the pinned Unicode 15.1.0 trie.
  Core's Unicode-property API retains its ASCII semantics and rejects invalid integer runes.
- Whole-name validation processes ASCII as bytes, tests high bits in 8-byte and 4-byte words, then switches
  to `chars()` for a block containing Unicode. Safe `first_chunk`/`from_ne_bytes` loads replace Oxc's unaligned
  pointer reads. `is_valid_identifier` and `is_identifier_text` share the implementation; the JSX variant
  specializes hyphen handling at compile time.

No Unicode upgrade or identifier-property change: the pinned trie already includes U+200C/U+200D and the
Katakana middle dots. Oxc's compatibility patch for older Unicode versions would change TypeScript behavior
and is not applied. There is no new allocation, cache, thread, or unsafe code.

## Whole-compiler measurements

Apple M3 Max, 14 logical CPUs, 36 GiB RAM; Rust 1.99.0, `cargo build --release` (fat LTO, one codegen unit,
no PGO). Both projects come from `bench/projects.json`'s suite commit `41f652ab2df5`, restored without local
edits. Commands pass `--noEmit --incremental false --pretty false`; single-threaded runs also pass
`--singleThreaded` and set `RAYON_NUM_THREADS=1`. One warm-up pair followed by five interleaved pairs,
alternating order; medians below. Every run had identical diagnostics and exit status between binaries.

Counters are macOS `/usr/bin/time -l` instructions retired and maximum resident set size. These are local
measurements, not the Linux `bench/count.py` deterministic gate or two published headline benchmarks.

| project | mode | instructions before | instructions after | change | peak RSS before (bytes) | peak RSS after (bytes) | change |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Compiler | single | 2,147,622,396 | 2,146,715,302 | -0.042% | 59,539,456 | 59,490,304 | -0.083% |
| Compiler-Unions | single | 4,823,591,122 | 4,822,436,305 | -0.024% | 62,095,360 | 61,440,000 | -1.055% |
| Compiler | default | 2,537,256,572 | 2,538,572,296 | +0.052% | 81,575,936 | 81,559,552 | -0.020% |
| Compiler-Unions | default | 5,978,822,467 | 5,969,075,351 | -0.163% | 86,097,920 | 86,327,296 | +0.266% |

Result: essentially neutral on these whole-compiler workloads, below the 1% instruction / 5% default-memory
landing bar. Retain the implementation on the isolated experimental branch at the owner's request; it is not a
proposed performance change to land. Other benchmark projects and the Linux instruction gate have not been measured.
Revisit for landing only with evidence of a qualifying gain on a pinned benchmark project or headline workload.

## Validation

- Core and scanner unit checks pass: 108 core checks (one existing ignored check) and nine scanner checks.
  New coverage compares all Unicode code points with the pinned library, all identifier characters with Go's
  predicate structure, and valid/invalid ASCII and Unicode at every position around the 4-/8-byte boundaries
  for standard and JSX names. Mixed Unicode tails and escaped identifiers still scan correctly.
- Full pinned conformance corpus, before and after: 15,197 variants, 13,458 error-baseline passes, 12,779
  `.types` passes and 12,779 `.symbols` passes; no crashes or timeouts. All result lists and retained actual/diff
  files are byte-identical. Existing mismatches are unchanged.
- `cargo check --workspace`, the `wasm32-wasip1` check, `cargo test -p tsrs_cli`, `tools/lint/ratchet.py`
  (three findings, none new), `tools/lint/source.py`, and all 29 regression fixtures pass.

The capability counts and supported behavior do not change, so the README capability table needs no update.
