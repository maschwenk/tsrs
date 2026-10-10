# Unicode identifier scalar validation

Measured 2026-10-10 on macOS arm64 with Rust 1.99.0. Base: `0925af8b` (PR #279 already using
`unicode-id-start = "=1.1.2"`). Candidate: preserve decoded `char` values through identifier classification.
This follows the dependency replacement: it introduced the integer-to-scalar check being investigated here.
The prior scanner work in `perf-parse.md` and the round-two follow-up list did not measure this conversion.

`AsRune::as_char` validates integer runes and returns `Some(self)` for a `char`. Scanner identifier predicates
accept `impl AsRune`, and ordinary identifier scanning keeps the decoder's `char`. Integer and escape callers
retain validation. The UTF-8 decoder's existing byte bounds prove scalar validity; one documented unchecked
construction at that boundary, with a debug assertion, preserves it in the return type. Legacy rune callers
use a wrapper returning `ch as i32`.

## Whole compiler

Both binaries use the repository's release profile (fat LTO, one codegen unit, no PGO). Three interleaved pairs
per project/mode; medians below. The checkout is the exact `bench/projects.json` suite commit
`41f652ab2df5077b1115f73eaeefc2fe9f674132`. Run each binary with:

```sh
/usr/bin/time -l ./tsrs -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false --singleThreaded
```

Use `Compiler-Unions` for the other project and omit `--singleThreaded` for default mode. `time -l` reports
instructions including kernel work on macOS; this is not the deterministic Linux `bench/count.py` gate.
Peak RSS is in bytes. Both old compiler fixtures report existing diagnostics (exit 2); output and exit codes
were identical between binaries in every pair.

| Project | Mode | Instructions before | Instructions after | Change | Peak RSS before | Peak RSS after |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Compiler | single | 2,149,745,721 | 2,147,038,962 | -0.126% | 60,686,336 | 60,686,336 |
| Compiler | default | 2,531,904,260 | 2,526,434,344 | -0.216% | 81,330,176 | 81,870,848 |
| Compiler-Unions | single | 4,828,032,435 | 4,827,159,009 | -0.018% | 62,914,560 | 62,881,792 |
| Compiler-Unions | default | 5,964,637,438 | 5,961,888,443 | -0.046% | 85,983,232 | 86,048,768 |

## Scanner probe

A scratch driver reuses one `Scanner`, sets its text, and calls `scan()` until EOF, five million times. It sums
token kinds and end offsets to retain the work. Both versions produced identical sums. The two input strings:

```typescript
const alpha = beta + gamma; let delta = epsilon.alpha;
const αλφα = βητα + γαμμα; let δελτα = έψιλον.αλφα;
```

Each input includes one trailing space. Three interleaved before/after pairs, same optimized build settings:

| Input | Instructions before | Instructions after | Change | Peak RSS before | Peak RSS after |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ascii | 12,777,690,084 | 12,776,634,670 | -0.008% | 2,031,616 | 2,015,232 |
| unicode | 20,817,076,611 | 19,867,200,048 | -4.563% | 2,031,616 | 2,015,232 |

The RSS difference is one 16 KiB page, not an allocation reduction. Optimized ARM64 assembly has no scalar
validation guard on the `char` predicate; the integer predicate keeps it.

## Verification and decision

Existing core/scanner checks: 106 passed, one intentional Miri negative-control case ignored. Source checks
and the lint ratchet pass. Scratch comparisons (no new checked-in unit cases) found exact identifier-table
parity for U+0000..U+10FFFF and invalid boundaries; exact decoder parity for every Unicode scalar, every one-
and two-byte input, malformed three/four-byte boundaries, a million deterministic four-byte samples, and EOF;
and identical token values, flags, offsets and diagnostics for raw identifiers, escapes, surrogates, invalid
code points and JSX variants. Full conformance and generator checks still need the absent `ts-ref` checkout.

The Unicode-heavy scanner probe improves, but these whole-project measurements do not establish the repository's
performance landing threshold. The requested typed-path change remains in draft #279 for review; this note is not
a claim of a qualifying whole-compiler speedup. Before landing it as a performance change, demonstrate the gate
on a representative project with the prescribed measurement. Do not repeat the same ASCII-dominated probes
expecting the scanner-only result to transfer to the entire compiler.
