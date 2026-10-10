# Oxc-style scanner byte dispatch (2026-10-10)

Isolated experiment on `codex/oxc-scanner-perf`, based on `d2e80d306f278ebf67ba3808e0244100d3db9af2`.
The candidate is maintained on this experimental branch, with a worktree at `/private/tmp/tsrs-oxc-scanner-perf`. The owner will measure performance;
instruction counts and peak RSS are pending. This is an experiment, not a performance change proposed to land.

## Why this experiment

Oxc at `4b756621b65ab54c19dc66fbdcb3cd902d662c20` dispatches incoming bytes through a 256-entry function table
([byte_handlers.rs](https://github.com/oxc-project/oxc/blob/4b756621b65ab54c19dc66fbdcb3cd902d662c20/crates/oxc_parser/src/lexer/byte_handlers.rs)).
Its token loop calls handlers with separate frames for punctuation, identifiers, numbers, comments and Unicode.

`notes/perf-parse.md` identified the large frame and dispatch cost of tsrs's `Scanner::scan`. Testing an ASCII
identifier start before the existing match was neutral; this experiment instead moves the match bodies into
separate functions, so every token no longer enters the frame needed by the most complicated branch. It does
not repeat the rejected comment `memchr3` or SWAR whitespace experiments, or the separate packed identifier-table
experiment.

## Change

- A compile-time-built `[fn(&mut Scanner) -> ScanAction; 256]` dispatches each byte to the corresponding handler.
  The table is 2 KiB on this arm64 host. ASCII letters, `_`, `$`, and backslash share the existing identifier logic;
  unsupported ASCII and all non-ASCII bytes use the existing default logic.
- `scan` resets full-start position and flags once, sets token-start on each iteration, checks EOF before reading
  and indexing, and repeats when a handler returns `Continue`. Handlers keep the previous state assignments and
  return `Return` when the old loop returned a token.
- All existing token algorithms, keyword hashing, trivia predicates, caches, numeric normalization, error order,
  directives, rescans and mark/rewind behavior remain in place. Public APIs and token kinds are unchanged. No new
  unsafe code, allocation, thread, cache, keyword matching strategy or byte-search algorithm is introduced.

## Code generation

Rust 1.99.0, macOS arm64, `cargo rustc --release --locked -p tsrs_scanner --lib -- --emit=asm` on both sources:

| scanner-crate assembly | baseline | candidate |
| --- | ---: | ---: |
| `Scanner::scan` frame | 208 bytes | 32 bytes |
| saved registers, including frame pointer and link register | 12 | 4 |

The candidate loop loads the handler from the table and calls it with `blr`; simple punctuation handlers are leaf
functions. This verifies the intended frame split, not a speedup. These figures come from optimized scanner-crate
assembly, before final CLI linking and PGO/BOLT. Handler calls and table loads must still earn their cost in the
owner's measurements.

## Correctness

Pinned Go source: `b85298b6a81f772d080b0455de0ca9d744cd6fd6`. Fresh baseline binaries were built from the agreed
source before editing, with the same release profile as the candidate (fat LTO, one codegen unit, no PGO).

- All four scanner-oracle modes: 17,956 valid-UTF-8 files have identical baseline, candidate and Go hashes. Inputs
  comprise 17,319 split compiler/conformance units, 113 bundled libraries and 525 focused inputs, excluding one
  non-UTF-8 unit. Focused inputs include every ASCII byte at EOF and with lookahead, Unicode, escapes, compound
  operators, trivia, CRLF, conflict markers, shebangs, JSX/JSDoc and templates.
- AST hashes: all 17,957 inputs are unchanged between baseline and candidate. The one existing difference against
  Go (`regexInvalidUtf8WithUnicodeFlag`) is unchanged; Rust reads that input lossily.
- Full conformance with `types,symbols,js,jsmap,sourcemap`: all 36 retained result files are byte-identical,
  excluding the timing-bearing summary. No crashes or timeouts. The emit harness has 13,462 error passes, 12,779
  type passes, 12,779 symbol passes, 13,392 JavaScript passes, 149 map passes and 156 sourcemap-text passes on both
  builds. These are emit-harness totals, not an increase over the README's type-check-only totals.
- Canonical parallel-program mode and the `TSRS_LAZY_MEMBERS=0` opt-out, with `types,symbols`: all retained results
  are also byte-identical. The existing canonical `.types`/`.symbols` difference remains unchanged.
- Core/scanner/CLI checks pass, including three added scanner cases for EOF dispatch, trivia retries and retained
  token state. All 29 regression fixtures pass. Workspace and `wasm32-wasip1` compilation pass.
- Both lint checks pass: the ratchet has three existing findings, none new; source inventory is unchanged. The
  local clippy invocation used `SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk` to avoid the host
  linker's SDK 27 stub-file incompatibility.

Behavior and pass counts are unchanged, so the README capability table needs no update.

## Ready for the owner's measurement

```sh
cd /private/tmp/tsrs-oxc-scanner-perf
cargo build --release --locked -p tsrs_cli --target-dir /private/tmp/tsrs-oxc-scanner-target
```

Matching binaries already built:

- baseline: `/private/tmp/tsrs-oxc-scanner-baseline-bin/tsrs`
- candidate: `/private/tmp/tsrs-oxc-scanner-target/release/tsrs`

Detailed comparison logs and hashes are under `/private/tmp/tsrs-oxc-scanner-validation`, with the suite and check
logs named `/private/tmp/tsrs-oxc-scanner-*.log`. Add before/after instruction and peak-RSS numbers here when
provided. A landing decision remains pending those numbers and the project's performance bar.
