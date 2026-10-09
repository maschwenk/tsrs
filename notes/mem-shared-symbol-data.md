# Shared symbol data for instantiated symbols (TypeScript #64691)

## Result

[microsoft/TypeScript#64691](https://github.com/microsoft/TypeScript/pull/64691) splits Go's `Symbol` into a
unique header and shareable data. Instantiated symbols then keep their own flags, check flags and ID but share the
source symbol's name, declarations, parent and table. This shrinks an instantiated Go symbol from 96 to 24 bytes.

The same representation was implemented against TypeScript PR head
`56d41e8443ad2311a1cb1c59fb874d7ab7c97f14`, then reverted after measurement. Rust's compressed `Symbol` was
already 32 bytes rather than 96. The implementation kept ordinary symbols at 32 bytes and made instantiated
symbols a 16-byte header pointing at shared symbol data. The memory saving was only 0.15-0.62% at the default
checker count, while the extra branch and indirection on ordinary symbol reads added 1.76-2.31% deterministic
single-threaded instructions.

Rejected: the peak-memory improvement does not clear the 5% landing threshold, and the instruction change is a
regression. Revisit only if instantiated symbol headers account for at least 5% of peak RSS or the representation
can make ordinary symbol reads direct.

## What was built

The Rust implementation mirrored `newSharedDataSymbol` and `SetSymbolData`: symbol data held the name,
declarations, parent and table, while each symbol retained its own flags, check flags and ID. Normal symbols stored
their data in the existing 32-byte allocation; symbols created by `new_instantiated_symbol` used a 16-byte header
that referenced the source symbol's data. A focused unit test checked that instantiated symbols shared data while
their flags, check flags and IDs remained independent.

The workspace compiled, and the full conformance run matched the base at `bef5bc2f` exactly: 13,458 diagnostic
baselines, 12,779 `.types` baselines and 12,779 `.symbols` baselines passed. The base and experiment had identical
failure lists.

## Measurement

Apple arm64, release builds, TypeScript-benchmarking `41f652ab`. Five base/experiment rounds were interleaved after
warm-up. `/usr/bin/time -l` measured retired instructions and maximum resident set size for `Compiler` and
`Compiler-Unions`. Instructions are from the deterministic single-threaded run; RSS is shown both for one checker
and the default checker count. Diagnostics were identical in every run.

| project | single-thread instructions | single-thread peak RSS | default-checker peak RSS |
| --- | ---: | ---: | ---: |
| Compiler | +1.758% | -0.553% | -0.153% |
| Compiler-Unions | +2.305% | -0.478% | -0.619% |

The upstream optimization is valuable for Go because it removes 72 bytes from every instantiated symbol. Here its
maximum per-instance saving was 16 bytes, and all normal symbol reads paid for distinguishing inline from shared
data. The measured working-set reduction was too small to recover that CPU cost.
