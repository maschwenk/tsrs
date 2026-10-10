# dod-ast-tables: the syntax tree's node rows as columns (data-oriented step 1)

Branch `dod/ast-tables` (from c71ed097). Result: does not clear the bar. Instructions go up 6-12% single-threaded on the
five bench projects, peak memory goes up 1-5% (the bar is 5% of peak, 1% of instructions, 2% of wall). Diagnostics are
byte-identical everywhere; single-threaded counters differ by a few units because a lazy-list decision depends on the
arena's chunk boundaries (section 4). The layout is kept on the branch as the experiment's answer, not for merge.

## 1. What yuku does

yuku (`src/parser/ast.zig`, commit df7b067 of its repo, read only) keeps one `Tree`: nodes in a `MultiArrayList` (struct of
arrays, one column per field), children as `NodeIndex` (a `u32`, `null` = `maxInt`), child lists as an `IndexRange`
window into one flat `extras: ArrayList(NodeIndex)`, strings in a pool, and everything in one arena allocator per tree.
Node data is a fixed-size record per tag, so a node is an index into the columns and nothing else.

## 2. What was built

- `crates/tsrs_core/src/nodetable.rs`: a node is a dense `u32` row index. Kind (`u16`), data tag (`u8`), flags, id
  (lazily assigned, atomic), parent (`PKey`), data handle (`PKey`, the arena handle of the node's data struct, 0 for
  payload-less nodes) and range (`TextRange`) are seven columns. On unix they are one fixed-address reservation, one
  4 GiB stride per column, committed on first touch. Rows are handed out per thread in chunks (1K doubling to 64K),
  and a speculative parse gives its rows back under the arena's rule (`rewind` only when `arena_rewindable`).
- `crates/tsrs_ast/src/ast.rs`: `Node` is a zero-sized view. A `P<Node>` keeps its identity, hashing, ordering and
  `Option` niche because its address encodes the row (`key_of` / `index_of`). `kind()`, `flags()`, `loc()`,
  `parent()`, the setters and `payload()` read the columns. Call sites are unchanged.
- Data structs stay in the arena, referenced from the DATA column. The generator (`tools/gen-ast/gen-ast.ts`) stops
  emitting the unused `NodeRareTail` impls; the `<Node>Rare` structs stay.
- Files: `crates/tsrs_compiler/src/fileregions.rs` takes a segment per file (`begin_segment` / `end_segment`) and
  discards the rows of a freed leaf by the ranges the segment kept.
- Commits on the branch: `4e96aee3` (the layout), `55261d67` (drop the 24-byte size test), `84b28f88` (the segment fix,
  section 3).

Not done: step 2 (lists as `IndexRange`s), step 3 (interned identifier text), and the node-store change in links.rs
(node ids are still lazily assigned and still the key of the id-keyed stores).

## 3. Numbers (base = c71ed097 clean build, new = 84b28f88)

Bench projects per bench/projects.json, this Mac (M5 Max). Single-threaded instructions: median of 3. Peak
footprint: `/usr/bin/time -l`, median of 3. Wall: median of 5 (default checker count), spread in brackets.
Base instruction counts repeat to about 0.1% except for a cold first run, which can be 2% higher (webpack: 13.31G, then
13.01G twice); new repeats to about 0.05% (webpack: 14.455G, 14.453G, 14.454G).

| project | instr single base -> new (G) | delta | peak single base -> new (MiB) | delta | instr default (G) | delta | peak default (MiB) | delta | wall default (s) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| vscode | 96.55 -> 108.32 | +12.2% | 1599 -> 1667 | +4.2% | 112.31 -> 125.30 | +11.6% | 2092 -> 2205 | +5.4% | 0.79 -> 0.90 [0.77-0.80 / 0.83-0.94] |
| t3code-server | 46.27 -> 50.02 | +8.1% | 731 -> 765 | +4.6% | 149.69 -> 157.16 | +5.0% | 2171 -> 2192 | +1.0% | 1.34 -> 1.36 [1.25-1.63 / 1.30-1.52] |
| webpack | 13.00 -> 14.44 | +11.1% | 303 -> 310 | +2.4% | 18.25 -> 19.85 | +8.8% | 497 -> 503 | +1.1% | 0.13 -> 0.14 |
| formbricks-web | 48.95 -> 52.52 | +7.3% | 1098 -> 1146 | +4.4% | 89.02 -> 94.15 | +5.8% | 1993 -> 2034 | +2.1% | 1.05 -> 0.93 [0.72-1.31 / 0.83-1.40] |
| cal-diy | 39.12 -> 41.44 | +5.9% | 761 -> 777 | +2.0% | 96.68 -> 101.62 | +5.1% | 1885 -> 1913 | +1.5% | 1.01 -> 1.11 [0.87-2.16 / 0.96-1.72] |

Wall time is inside the run-to-run spread on every project, so there is no wall verdict. The formbricks and cal-diy
wall medians are noisy (the base spread is 0.7-1.3 s and 0.9-2.2 s).

Memory history, same machine: before the segment fix (`4e96aee3`) vscode peak was +17% single and +13% default, webpack
+2.3%. The gap was the per-file reservation (section 3.1). The residual after the fix is about 3 bytes per node (27 bytes
of columns vs the 24-byte header): vscode has 17.9M nodes, so 54 MB, which is the +4% measured.

### 3.1 The segment gap (fixed in 84b28f88)

A file parsed into its own region first reserved `text/3 + 64` rows and left the unused part as a gap. With one
page-aligned column per field, each file then left a partly used page in all seven columns, where the arena's header
had one page per file. Measured on vscode at 2 s into a single-threaded run (RSS, `ps -o rss`): base 1194 MiB, new with
reservations 1383 MiB, new with no segments at all 1219 MiB. The fix (exact ranges from the thread's chunks, nothing
reserved) brings it to 1222 MiB. Peak on vscode went from 1864 MiB to 1673 MiB (base 1606 in the same sampler).

## 4. Fidelity

- Diagnostics: the `error TS` lines are byte-identical on all five projects, single-threaded, at `--checkers 4`, at
  `--checkers 4 --checkerAssignment go`, and at the default count. Apart from timing, memory and counter lines, the
  single-threaded output is identical on all five projects; default-mode output differs only in counter lines (for
  example "Empty-object queries answered by lazy tables"), which base also varies between runs.
- Counters, single-threaded (deterministic on base): Identifiers, Types, Instantiations match on all five projects.
  Symbols differs by -172 on vscode, -11 on t3code-server, -10 on formbricks-web, +8 on cal-diy, and matches on webpack.
  Lazy-member counters move by a few units (for example 2,682,952 vs 2,682,955 lookups on vscode).
- Root cause, isolated: `parse_member_list_lazily` (crates/tsrs_parser/src/parser_1.rs) decides whether a member list is
  lazy by whether the arena can be rewound (`arena_rewindable`: no pin, no free, same chunk). A list in vscode's
  monaco.d.ts whose parse crosses an arena chunk boundary is kept eager in base and lazy in new, because the new layout
  allocates fewer arena bytes so the boundary falls elsewhere. Checked: with debug builds of both trees the decision logs
  match except for that one list, and the per-file binder counts differ only for monaco.d.ts (2165 vs 1993). A temporary
  build that allocates a 24-byte shadow in the arena per node (the base header's size, nothing else) makes the new tree's
  Symbols count equal base's exactly (2271526 with `--noCheck`). That experiment was reverted and is not on the branch.
- Multi-checker counters are not usable as a fidelity check: base itself gives different Symbols on two runs of the same
  binary (`--checkers 4 --checkerAssignment go` on vscode: 5231156 and 5231123), so the new deltas there (+15 to +42 on
  lazy counters and t3code Symbols) are within base's own noise.
- Conformance: `pass.txt` is identical to base (13458 entries; `.types` and `.symbols` lists identical). The
  conformance gate script passes with its committed minimums.
- Fourslash: 4066 passed, 63 failed, 417 skipped on both trees, with identical failure lists.
- Regressions: 29/29. Determinism: 594 runs, all identical to the single-threaded output.

## 5. Gates

- `cargo check --workspace`: 0 warnings. `cargo check -p tsrs_wasm --target wasm32-wasip1`: ok. `cargo check -p tsrs_cli`
  with `tsrs_core/shared-graph` and with `tsrs_core/plain-ptrs`: ok.
- `tools/lint/ratchet.py`: ok (3 existing findings, none new). `tools/lint/source.py`: ok.
- `tools/gen-check.sh`: ok on the committed tree (regeneration reproduces the committed generated files).
- `cargo test -p tsrs_core -p tsrs_ast -p tsrs_parser -p tsrs_printer -p tsrs_compiler -p tsrs_binder`: all pass.
  One test that asserted `size_of::<Node>() == 24` was removed; the const assert in ast.rs covers the new size.
- Not run: the `ast-sizing` feature (crates/tsrs_ast/src/sizing.rs) does not compile on the new layout; it reads the old
  header. The sizing tool is therefore unavailable on this branch.

## 6. Why the instructions go up

Instructions are not explained by the lazy-list decision: the shadow build above (with its 24-byte arena writes per node)
measured 14.78G on webpack single-threaded, against 14.45G for the final layout (14.53G on the first measurement of
it) and 13.0G for base. What remains
is the per-access cost of the layout. Every field read goes through the zero-sized view (`P` to address to row index)
and then a column address, where base read one header word. Data reads go through the DATA column as well (base's data
struct sat at a constant offset from the header). Node creation writes seven columns and reads the thread-local chunk
twice. Node creation counts match base (vscode 17,520,725 at parse end on both; 17,874,611 vs 17,874,613 at check end),
so creation is not the difference. Section 7 has the open question.

No profiler could attribute instructions to call sites here: `xctrace` CPU Counters needs kperf, which needs sudo, and
samply gives time-based samples only. The self-time comparison (samply, webpack, single-threaded, /tmp/dod-ast/self-*.txt)
shows the growth in accessor-bound functions: compare_nodes 41 to 81 samples, Node::text 43 to 61, get_combined_modifier_flags
10 to 25, is_statement 14 to 24, plus a new non-inlined for_each_child specialisation.

## 7. Open problems

1. Instruction overhead of 6-12% is unexplained at call-site level. A follow-up would pass the row index (`u32`) through
   the hot accessors instead of the zero-sized view, keep the DATA handle and tag in one column, and measure again.
2. Lazy-list decisions depend on the arena's chunk boundaries (section 4). Any layout change moves them, so counters
   cannot match base exactly unless the decision is made independent of chunk boundaries in base too. That is a
   change to base behaviour and belongs in its own PR.
3. Per-node layout costs 27 bytes against 24 (+3 bytes, +4% on vscode peak). The data-only arena savings match the
   header exactly on webpack (checked with the shadow build: 24 bytes per node).
4. `ast-sizing` must be ported to the row layout before anyone counts bytes again.
5. Step 2 (lists as ranges) is not started. The sizing note (notes/mem-compact-ast-sizing.md) predicts 0.9-1.4% of peak on
   the application projects for the realistic layout package, so it is unlikely to clear the bar alone.

## 8. Verdict

Step 1 does not clear the bar: instructions +6-12%, peak memory +1-5%, wall inside its noise. The memory result is close
to what the layout predicts (+3 bytes per node). The instruction cost is the open question, and the data-oriented
layout does not pay for itself on this codebase without the call-site changes in open problem 1.
