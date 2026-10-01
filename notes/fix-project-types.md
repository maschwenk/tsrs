# fix-project-types: every expression's type on Project

Goal: compare the printed type of every expression/declaration (the conformance `.types` walk) on the real
Project project, not only its diagnostics and counters.

## Tooling

- Go oracle `tools/oracle/project-types/main.go` (binary `bin/tsrs-oracle-project-types`) and
  `tsrs-test types-dump -p <tsconfig> --out <dir> [--mode types|symbols|both] [--text all|none|<list>]`.
  Both load the project like `tsc -p` (cached-FS compiler host, single-threaded program), collect diagnostics with
  `GetDiagnosticsOfAnyProgram` (so every file is checked first, as in `tsc`), then walk every file that is not under
  `node_modules` and not a default lib, in program order, with `type_symbol_baseline.go`'s walker
  (`hasErrorBaseline` = any diagnostics). `both` = type walk over all files, then the symbol walk (test order).
- Output: `<out>/manifest.<kind>` (`fnv1a64 \t lines \t relpath` per file, then `#counts types symbols
  instantiations` after the walk) and, per `--text`, `<out>/<kind>/<relpath>.<kind>`. With a list and one mode, the
  walk stops after the last listed file (earlier files still run, so checker state is the same).
- `tools/project-types-compare.py <go-out> <rs-out> [--list diff.txt] [--show N]`: matching files, counters,
  differing lines.
- Cost on Project (28,213 walked files, 16.5M baseline lines): ~42 min (tsrs) / ~71 min (Go) when run together;
  peak footprint ~125 GB (tsrs) / ~132 GB (Go): the walk prints >1 MB types (NoTruncation) and neither side frees
  much of what the node builder builds. Run the two sides one after the other.

## Results

| run | files identical | differing lines | counters (types / symbols / instantiations) |
| --- | --- | --- | --- |
| types, main 7f683f4 | 28,207 / 28,213 | 18 (6 files) | equal / equal / tsrs +20 |

Debug technique that located it (kept out of the tree): a private copy of the Go module under
`target/scratch/project-types/tsc-dbg` with a trace in `visitAndTransformType` (`V/X/C typeId approximateLength`)
and in `GetSymbolId` (`id name decl`), the same prints in a throwaway Rust build, both enabled only while walking the
one differing file (`TRACE_FILE`); `diff` of the two traces gives the first node-builder step whose length differs.

## Divergences

| cluster | files / lines | root cause | fix |
| --- | --- | --- | --- |
| `vi.mocked(spy, { partial: true, deep: true })` types (>1 MB with NoTruncation) cut at a slightly different point (Go prints `callbackfn: any` one signature earlier) | 6 router tests / 18 lines; +20 instantiations in tsrs | Symbol ids. Go's `valueSymbolLinks` (`symbolArenaLinkStore`) and `symbolNodeLinks` (`nodeLinkStore`) are keyed by `ast.GetSymbolId` / `ast.GetNodeId`, so every access assigns an id; the port keyed them by pointer and assigned far fewer ids. A well-known-symbol-keyed property's internal name embeds its symbol id (`\xFE@toPrimitive@<id>`), and the node builder adds `len(name)+1` to `approximateLength`, so a shorter id moved the truncation point (found with an `approximateLength` trace diffed between both sides: the first difference was 1 unit at `Date[Symbol.toPrimitive]`). Second slip found while diffing id-assignment logs: Go's single-threaded `WorkGroup` runs queued work last-in-first-out, so `BindSourceFiles` binds files in reverse order (the binder assigns ids for private names); the port bound forward. | `links.rs` `SymbolArenaLinkStore` / `NodeLinkStore` assign the id on get/try_get/has; `bind_source_files` single-threaded iterates in reverse. Regression `testdata/regressions/unique-symbol-name-truncation` (error-message truncation at 160 chars moves by one property with the old build). |

Notes:
- Go's symbol ids are **not deterministic across runs**: code that iterates a Go map (symbol tables) touches
  `valueSymbolLinks` in random order (two Go runs on the same project differ from the ~125th id on). After the fix the
  multiset of (symbol, id-assignment count) is identical to Go's and the order differs only inside such map-iteration
  windows (<= 64 entries on the vimock repro), so digit counts, the only observable, agree except right at a power of
  ten. An exact comparison of ids is impossible; compare the sorted `name decl` multiset instead.
- Node ids are now assigned at the same accesses as in Go too (`symbolNodeLinks`); they only feed cache keys.
