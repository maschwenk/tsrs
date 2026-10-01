# mem-small: the two remaining representation items (Symbol, Identifier) and small layout wins

Follow-up to notes/mem-round3.md ("not done": `Symbol` 56 -> 48 bytes, identifier text from the source) on top of
notes/mem-recycle.md. Representation only, no checker semantics.

Gates for every step (same as mem-round3 / mem-recycle): the suite with `--baselines types,symbols`, whole
`target/test-results` trees compared with the base binary, in the default mode, with `TSRS_LAZY_MEMBERS=0` and with
`TS_TEST_PROGRAM_SINGLE_THREADED=false`; the private monorepo's diagnostics and `--extendedDiagnostics` counters
identical before/after in default / opt-out x 1 / 4 checkers (opt-out 4 checkers with `--checkerAssignment go`);
AST oracle unchanged when nodes change. The private monorepo reports 19 errors (16 distinct lines in the comparison)
from stale workspace builds before and after; the gate is identical output.

Measurement: `/usr/bin/time -l` on the private monorepo, base and candidate interleaved, medians of 3, "GiB" = peak
memory footprint / 2^30. The machine was shared (load 5 to 30), so check times are noise and instructions retired
vary by about +-2% between identical runs; the instruction columns give min-max.

Counters at the base (main at 493358a): default 13,212,573 / 9,945,857 / 46,135,905 single and 17,187,182 /
14,365,514 / 79,522,288 on 4 checkers; opt-out 26,900,867 / 9,955,103 / 46,192,430 single and 40,743,802 /
16,630,033 / 90,406,448 on 4 checkers (go assignment).

## 1. `Symbol` 56 -> 48 bytes: the member/export tail shares a word with `parent`

mem-round3 assumed two cuts were needed. The 56 bytes were: flags, check flags, the declarations (4-aligned
pointer + `u32` length) and the `u32` id packed into 24 bytes, then four words: name (`PackedStr`), value
declaration, parent and the pointer to the `SymbolTables` tail (members, exports, export symbol; allocated on the
first non-nil write, mem-layout step 5). Removing one word is enough.

Counted (alloc-profile, the private monorepo single, default mode): 796,407 tails for 13,212,574 symbols (6.0%).
So the tail pointer now shares one word with `parent` (`SymbolParentWord`): the word holds the parent's address
until the symbol gets a tail; the first tail write allocates the tail with the current parent copied into it and
stores the tail's address with bit 63 set. `parent()` reads the word (or the tail's parent when the bit is set),
`set_parent()` writes whichever holds it. Tails grow 24 -> 32 bytes (+6 MB). Provenance is exposed on store and
recovered with `with_exposed_provenance` (like the node header's parent); the address part stays a plain pointer to
the block start, so the census marks see it as before. Same owner-only write contract: the binder writes binder
symbols before they are shared, a checker only its own symbols, and the tail is allocated by that writer.
`symbol.parent.get()` / `.set(..)` became `parent()` / `set_parent(..)` (24 sites, mechanical).

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, base | 5.911-5.928 (5.912) | 303.2-311.7 G |
| single, after | 5.820-5.831 (5.830, -0.082) | 304.0-310.3 G |
| 4 checkers, base | 7.867-7.921 (7.909) | 413.7-419.8 G |
| 4 checkers, after | 7.771-7.803 (7.772, -0.137) | 413.4-416.2 G |
| opt-out single / 4 checkers (go assignment), base -> after (1 run) | 7.944 -> 7.748 / 12.148 -> 11.758 | 332.7 -> 332.2 / 498.8 -> 495.3 G |

Gates: suite trees identical in all three modes; private-monorepo output and counters identical in all four runs;
`cargo test -p tsrs_ast -p tsrs_binder -p tsrs_core`.
