# perf-parse: CPU cost of scanning, parsing and AST construction

Goal: make parsing cheaper without changing the AST. Gates for every change: AST oracle hashes identical to the
base on the 17,319 split corpus units, the 113 bundled libs and the private monorepo's 38,789 files (and equal to
Go's: `tools/oracle/ast/run.py` reports the same 1 non-UTF-8 difference before and after); the suite's
`target/test-results` trees identical (timings in `summary.json` aside) with `--baselines types,symbols` in the
default and the `TSRS_LAZY_MEMBERS=0` mode; the private monorepo's single-threaded output (its 9 current errors)
and counters identical.

## Measuring

- `ast_oracle bench N < list` (crates/tsrs_parser/examples/ast_oracle) parses every listed file N times on one
  thread and prints, per round, the process's instructions retired and cycles (macOS `proc_pid_rusage`; kernel
  included) and the number of heap allocations. It now runs on mimalloc like `tsrs`. One round over the private
  monorepo's files is stable to +-0.3% in instructions on a loaded machine; cycles need several runs.
- The list: `tsrs -p <tsconfig> --noEmit --noCheck --listFiles`, one `path<TAB>-` per line (bundled libs
  mapped to `ts-ref/tsc/internal/bundled/libs`).
- `tsrs --noCheck --singleThreaded --extendedDiagnostics` under `/usr/bin/time -l` for the whole parse phase;
  its instruction count includes file reads and module resolution in the kernel and moves by +-1 G between runs.
- samply (`samply record -s --unstable-presymbolicate`) plus a small reader of the profile JSON (exclusive,
  inclusive, callers, per-address samples; `dsymutil` on the copied binary and `atos -i` map addresses to source
  lines including inlined frames). Not committed.

## What the parse phase is

`--singleThreaded`, private monorepo (38,789 files, 216.7 MB, 6.12 M lines, 22.2 M nodes): Parse time 2.8-3.0 s,
of which parsing proper was ~1.07 s (scan + parse + AST + reference collection), reading the files ~0.86 s
(mostly `open`, ~18 us per call on this machine), module resolution ~1.1 s (mostly `stat`). With rayon on 18
cores the parse phase is ~0.95 s wall but 70% of the busy worker samples are in `read` / `open` / `stat`
(sys 6-8 s against user 3 s), so faster parsing did not shorten it here: on this machine the parallel phase
is bound by the kernel's file system calls. Pure parsing, Go vs tsrs (oracle bench, one thread): tsgo
1.57 s per round (GOMAXPROCS=1, GC included), tsrs 0.86 s before and 0.73 s after.

## Result

| | base (1c02754) | after (0a50271) |
| --- | --- | --- |
| bench: instructions per round | 18.95-19.04 G | 14.54-14.58 G (-23.5%) |
| bench: time per round (best of 3, load ~10) | 0.86-0.91 s | 0.73-0.76 s |
| bench: heap allocations per round | 11.40 M | 1.22 M |
| tsrs --noCheck --singleThreaded: instructions (median of 5) | 58.0 G | 54.2 G |
| tsrs full check --singleThreaded: instructions (3 runs) | 304.6-306.3 G | 299.7-300.8 G |
| tsrs --noCheck, 18 threads: Parse time | 0.91-1.03 s | 0.94-1.15 s (kernel-bound, unchanged) |

Peak memory unchanged (2.42-2.48 GB for `--noCheck`); the text copy removal saves ~50 MB in most runs.

## Changes (each landed separately, numbers in the commit messages)

1. Bench tooling: mimalloc, instructions / cycles / heap allocations per round; heap-profile counts with three
   decimals.
2. Node lists on one parser-owned stack (`Parser::node_stack`), copied once into the arena. Go builds lists in
   `make([]*ast.Node, 0, 16)` slices that stay on the goroutine stack; the port allocated a heap `Vec` per list
   (also per `parseModifiers` call, with or without modifiers), copied it with `to_vec` and then into the
   arena. 18.95 -> 17.77 G, heap allocations 11.4 M -> 1.2 M.
3. `findImportOrRequire` (collectExternalModuleReferences, every file) with two memmem searches instead of a
   stop at every `i` / `r`. 17.77 -> 17.33 G.
4. `parse_source_file_owned`: the compiler host hands the file's `String` over (leaked as the text, as Go shares
   the string) instead of copying 216 MB into the arena. Instructions within noise; peak -50 MB in most runs.
5. Scanner: keywords through a compile-time perfect hash (the 85-arm string `match` compiled to length switches
   and memcmp chains), the ASCII identifier loop through a byte table, single-line whitespace runs skipped in
   one loop. 17.33 -> 16.61 G.
6. `reportScanErrors` runs after every scan; its out-of-line body made every call save registers. Check
   inlined, drain cold. 16.66 -> 15.95 G.
7. Same split for `nextToken` (keyword-escape diagnostic), `parseExpected` (diagnostic) and
   `tryParseTypeArgumentsInExpression` (the speculative parse). 15.91 -> 15.24 G.
8. Same split for `withJSDoc`, `checkJSSyntax`, `tryReparseOptionalChain`. 15.24 -> 15.07 G.
9. `Node::for_each_child_static`: the generated per-struct `for_each_child` methods and visit helpers are generic
   over the visitor (`V: FnMut + ?Sized`; the `&mut dyn FnMut` dispatcher instantiates them as before), and the
   parser's parent pass (`overrideParentInImmediateChildren`, every finished node) uses a dispatcher compiled for
   its closure: no indirect call per child. 15.05 -> 14.64 G.
10. `parseAnyContextualModifier` returns false before marking when the token is not a modifier (Go marks and
    rewinds an unchanged state: 2.8 M no-op rewinds); `createIdentifierWithDiagnostic`'s error path cold.
    14.65 -> 14.55 G.

The pattern behind 6-8 and 10: a function called once per token or node that returns early in the common case
but has a heavy rare path pays the rare path's register saves on every call (no shrink-wrapping across a big
body). Splitting the rare path into a `#[cold]` / out-of-line function is invisible to the port's structure.

## Profile (bench, exclusive samples within parsing)

Before: `Scanner::scan` 19.3%, `scan_identifier` 12.8%, `get_identifier_token` 3.3%, `find_import_or_require`
3.3%, `new_identifier` 3.0%, `report_scan_errors` 2.9%, `for_each_child` 2.8%, `parse_member_expression_rest`
2.6%, memmove 2.2%, `scan_string` 1.9%, malloc + free 2.0%, TLS / arena access ~2%.

After: `Scanner::scan` 26.8% (the keyword lookup is inlined into it), `scan_identifier` 13.4%, memmove 4.3% (half
of it is the bench copying each file's text into the arena), `new_identifier` 3.4%,
`parse_member_expression_rest` 2.9%, `scan_string` 2.2%, `for_each_child_static` 1.8%, then a flat tail of
grammar functions (< 1.6% each). Inside `scan`, the samples sit on the jump-table dispatch (mispredicted
indirect branch, mostly at identifier starts), the return, indentation skipping after line breaks (~12% of
`scan`) and multi-line comment bodies (~12% of `scan`). IPC is ~4.5: the remaining scanner cost is branch
mispredictions and short loops, not instruction count.

## Rescans

The parser calls `scan` 28.7 M times. Speculation that rewinds (mark/rewind, lookAhead) accounts for 3.3 M of
those scans; the busiest sites are `isStartOfDeclaration` (1.17 M rewinds), `isParenthesizedArrowFunctionExpression`
(0.25 M rewinds, 0.55 M scans), list-element lookaheads for type and class members, and `tryParse` of
parenthesized arrow functions (23 K rewinds, 0.28 M scans). Same structure as Go; not changed.

## Tried and rejected

- memchr3 for multi-line / single-line comment bodies: -0.56 G instructions but +5% cycles (short comment lines;
  the byte loop predicts well). SWAR skipping of eight spaces at a time after line breaks: -0.1 G instructions,
  +1-3% cycles. Both reverted.
- Copying 1-4 element slices into the arena element by element instead of memcpy (`alloc_slice`): +0.1 G.
- Testing for an ASCII identifier start before `scan`'s `match` (a conditional branch instead of the jump
  table): no change in instructions or cycles.
- `#[inline]` on `Scanner::re_scan_greater_than_token`: no change.
- Thread-local arena access (`ARENA.with`, ~1% of samples through `_tlv_get_addr`): a const-initialized
  thread-local would still go through the TLV thunk on macOS; not tried.

## What remains

- Parallel parse phase on macOS: bound by `open` / `read` / `stat` (70% of busy worker samples with 18 threads;
  `open` ~18 us each even single-threaded on this machine). Fewer syscalls (no EOF probe read per file, `stat`
  answers from directory listings) would be the lever; not done (the first changes read-until-EOF semantics, the
  second is not exactly equivalent on a case-insensitive file system, see notes/speed-frontend.md). Not measured
  on Linux, where these calls are much cheaper and parse CPU is a larger share.
- `scan`: dispatch mispredictions and the per-call prologue/epilogue (12 callee-saved registers) over 28.7 M calls;
  a restructured scanner (hot single-character tokens without the big frame) would deviate from Go's shape.
- The recursive-descent expression chain (assignment -> binary -> unary -> update -> left-hand side -> member ->
  primary), ~0.3-0.4% of samples per level in prologues.
- `new_identifier` and the other node constructors: ~45 instructions each (two shared counters through `Rc`,
  TLS arena lookup, bump allocation).
