# mem-compact-ast-sizing: what a more compact syntax tree would save on the 8-checker scoreboard losses (2026-10-09)

Question from the owner: the scoreboard moved to the 16-vCPU runner with tsrs at its default 8 checkers, where the
parsed program, shared by every checker, is a larger share of the peak than at 32. Would a more compact syntax tree
(identifier text as atoms, name-less property nodes, lists as inline ranges, anything else the counts show) put tsrs
below `bun check` on the six projects where it still uses more memory? Answered with counts and bytes of the real
trees, from a counting pass over the parsed and bound program (`--features ast-sizing`, section 3), on the code the
benchmark measured. No design was built; every saving below is counts times layout sizes.

Answer: no. The whole tree alive at the peak is 6-21% of it. No single design reaches 5% of the peak on any of the
six; the realistic package (name-less property, member and specifier names; inline list ranges; literal flags) is
0.9-1.4% on the five application projects and 3.4% on vscode, and the most aggressive layout-only set measured is
1.7-3.6% and 5.7%. Only vscode can flip, by a few MiB on a gap that moves between bench runs (57-73 MiB).

## 1. The scoreboard

Peaks from the README table of tsrs `55c2d9ce5c41` (bench of 2026-10-08 21:05 UTC,
`bench/results/2026-10-08-55c2d9ce5c41.json`, "wide" cells: Depot `depot-ubuntu-24.04-16`, tsrs at its default 8
checkers, bun at 16 threads, median of 10). The crates at the base of this branch (8a95541) are identical to
`55c2d9ce5c41`, so the counts below are of the code that run measured. Cross-checks: the next README run
(`7ff55ce2bde7`, 2026-10-09 05:28 UTC) moves the gaps by -18..+7 MiB (t3code 656, supabase 180, mikro-orm 209, cal-diy
140, formbricks 61, vscode 63); the newest 16-vCPU compare file
(`bench/results/compare/2026-10-08-cccadf7cd14b-16t.md`, mean of 10) has vscode 1,984 vs 1,911 MiB (gap 73), t3code
1,812 vs 1,145 (666), mikro-orm 1,641 vs 1,433 (209); the other projects are not in it.

| project | tsrs peak | bun check peak | gap | gap / tsrs peak |
| --- | ---: | ---: | ---: | ---: |
| t3code-server | 1,806 MiB | 1,149 MiB | 657 MiB | 36.4% |
| supabase-studio | 1,421 MiB | 1,241 MiB | 180 MiB | 12.6% |
| mikro-orm | 1,640 MiB | 1,438 MiB | 202 MiB | 12.3% |
| cal-diy | 1,478 MiB | 1,321 MiB | 157 MiB | 10.6% |
| formbricks-web | 1,716 MiB | 1,637 MiB | 79 MiB | 4.6% |
| vscode | 1,967 MiB | 1,910 MiB | 57 MiB | 2.9% |
| webpack (control) | 451 MiB | 494 MiB | -42 MiB | already below |
| drizzle-orm (control) | 593 MiB | 728 MiB | -136 MiB | already below |

MiB = 2^20 bytes throughout; the bar (AGENTS.md) is 5% of the peak at the default checker count, 70-98 MiB on the six.

## 2. Layout today (base 8a95541)

- `Node` is a 24-byte header (crates/tsrs_ast/src/ast.rs:421-428): one word with the parent handle, kind (9 bits),
  data tag (8) and rare bit (`NodeHeaderWord`, ast.rs:460); `NodeFlags` (4 B); the node id (4 B, assigned lazily);
  `loc` (pos, end: 8 B). The data struct follows in the same allocation (`NodeAlloc<T>`, ast.rs:516), rounded to 8
  bytes; ten structs put almost-never-set fields in a tail allocated only when set (`NodeAllocRare`, ast.rs:531).
  Payload-less nodes (`Token`, `KeywordTypeNode`, ...) are the header alone.
- `Identifier` (crates/tsrs_ast/src/identifier.rs:109-123) is the header plus one 8-byte word: the flow node handle (or,
  before the binder sets one, the file's text index), the text length (17 bits) and a mode. The text is the slice of the
  registered source text that ends at the node's end, so it is neither stored nor copied: 32 B per identifier. An
  identifier whose text is not that slice (synthesized, reparsed JSDoc, escapes) stores a `PackedStr` pointer after the
  word (`IdentifierWithText`, 40 B). Nothing is interned.
- `NodeList` (ast.rs:267-272) is 16 B: `loc` (8) and a one-word `ThinSlice` to its elements, which are 4-byte handles
  in a separate slice. `ModifierList` (ast.rs:359) adds the cached `ModifierFlags`: 24 B. A node holds a list as a
  4-byte `P<NodeList>`.
- `P<T>` is a 32-bit handle into a 32 GiB reservation on unix (Mac and Linux alike), so these counts and bytes are the
  same on the Linux runner.

| node | bytes (data struct) | node | bytes (data struct) |
| --- | --- | --- | --- |
| `Identifier` | 32 (8), 40 with stored text | `Token`, `KeywordTypeNode` | 24 (0) |
| `PropertyAccessExpression` | 40 (12), 40 with `?.` | `StringLiteral`, `NumericLiteral` | 40 (16) |
| `CallExpression` | 40 (12), 48 with tail | `TypeReferenceNode` | 32 (8) |
| `PropertySignatureDeclaration` | 48 (20) | `PropertyAssignment` | 48 (24) |
| `MethodDeclaration` | 80 (56) | `ImportSpecifier` / `ExportSpecifier` | 40 (16) / 48 (20) |
| `ParameterDeclaration` | 48 (20), 56 with tail | `SourceFile` | 736 (704) |

## 3. Method

`crates/tsrs_ast/src/sizing.rs`, compiled only with `cargo build --release -p tsrs_cli --features ast-sizing` (default
builds are unchanged), walks the tree when `TSRS_AST_SIZING=<file>` is set, at two points of the type-check pass
(`Program::get_semantic_diagnostics`): `pre` (after load, bind and the leaf classification, every file) and `post`
(after the pass: every file but the freed check leaves, with the lazy member lists and the JSDoc the checkers forced).
It follows `for_each_child` from each `SourceFile` plus every JSDoc node in the file's cache, counts each node once,
records each list a node visits without forcing lazy ones, and writes per kind, data struct, identifier role, literal
kind and list length: counts, arena bytes (`size_of` of each allocation, rare tails included), bytes copied out of
the source, nodes with an id or nonzero flags, and distinct identifier texts per file and per program.
`tools/sizing/compact_ast.py` turns the rows into the tables here and computes each design from the walked layouts
and the list fields of each data struct (read from `generated.rs`).

Runs: Mac (18 cores), one process at a time, `tsrs -p <project> --noEmit --incremental false --extendedDiagnostics
--pretty false --checkers 8` on the `bench/run.py` checkouts, single run each, plus one `TSRS_MEM_SPLIT=1` run for
the arena and heap at parse end. Checks: error counts equal the README's on all eight (6, 9, 43,651, 136, 0, 371, 840,
10,846); the layouts rebuilt by the tool reproduce the walked node bytes exactly on all eight; the walked tree of the
non-leaf files at `pre` is 70-89% of the thread arenas' used bytes at parse end (the rest is binder output).

Base for every saving: the `post` tree, the tree alive at the end of the pass. The peak is at or near that point
(docs/DEBUGGING.md, the memory split), and freeing leaves cut vscode's 8-checker peak by 13.2% and t3code's by 4.3%
(notes/mem-free-leaf-files.md), about the leaves' whole footprint, so few leaf trees are alive at the peak. Section 5.4
gives what the designs would add if every leaf tree were, the upper bound.

## 4. Counts (measured)

### 4.1 Program and tree

| project | files: source / declaration / freed leaves | source text, MiB | nodes | tree, MiB | tree / peak | headers / tree | identifiers / tree | lists / tree | freed leaf trees, MiB | arena + heap at parse end, MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| t3code | 1,023 / 1,423 / 499 | 41 | 3.34M | 133.4 | 7.4% | 57% | 26% | 12% | 54 | 173 + 91 |
| supabase | 4,621 / 7,226 / 703 | 69 | 4.66M | 194.3 | 13.7% | 55% | 23% | 12% | 23 | 260 + 161 |
| mikro-orm | 1,962 / 949 / 0 | 20 | 2.53M | 101.6 | 6.2% | 57% | 26% | 11% | 0 | 128 + 37 |
| cal-diy | 3,285 / 6,824 / 61 | 59 | 4.35M | 175.6 | 11.9% | 57% | 24% | 11% | 3 | 228 + 118 |
| formbricks | 2,518 / 6,908 / 1,029 | 222 | 5.01M | 211.0 | 12.3% | 54% | 24% | 15% | 49 | 276 + 311 |
| vscode | 7,126 / 1,012 / 2,289 | 95 | 10.65M | 421.5 | 21.4% | 58% | 26% | 10% | 257 | 512 + 237 |
| webpack | 965 / 642 / 0 | 23 | 2.36M | 93.3 | 20.7% | 58% | 25% | 10% | 0 | 117 + 36 |
| drizzle | 796 / 2,500 / 177 | 28 | 2.38M | 96.7 | 16.3% | 56% | 24% | 12% | 20 | 106 + 63 |

Tree = node allocations + `NodeList`/`ModifierList` structs + element slices + copied text + JSDoc cache slices
(post). Source text is the text of the files alive at the end (heap, outside the tree). The last column is the Mac
memory split at parse end (thread arenas' used bytes + live heap; leaf regions are not in the arenas' count). At check
end the same split reads 1.3-1.8 GiB on the six: what exists at parse end (tree, binder output, text, loader) is 11-41%
of it (t3code 15%, mikro-orm 11%, vscode 41%) and the checkers hold the rest.

### 4.2 Identifiers by role (post, count / MiB)

| role | t3code | supabase | mikro-orm | cal-diy | formbricks | vscode | webpack | drizzle |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| expression (incl. `a` of `a.b`, shorthand, JSX tag) | 253k / 7.7 | 283k / 8.6 | 229k / 7.0 | 137k / 4.2 | 172k / 5.3 | 1,162k / 35.5 | 180k / 5.5 | 107k / 3.3 |
| declaration name | 176k / 5.4 | 310k / 9.5 | 128k / 3.9 | 231k / 7.1 | 374k / 11.4 | 483k / 14.8 | 129k / 3.9 | 177k / 5.4 |
| property-access name (`b` of `a.b`) | 181k / 5.5 | 78k / 2.4 | 177k / 5.4 | 71k / 2.2 | 64k / 2.0 | 998k / 30.5 | 87k / 2.6 | 74k / 2.2 |
| member declaration name | 94k / 2.9 | 116k / 3.5 | 75k / 2.3 | 228k / 7.0 | 194k / 5.9 | 241k / 7.3 | 68k / 2.1 | 69k / 2.1 |
| object literal / JSX attribute / binding property name | 85k / 2.6 | 126k / 3.8 | 64k / 2.0 | 66k / 2.0 | 69k / 2.1 | 177k / 5.4 | 11k / 0.3 | 43k / 1.3 |
| type-reference name (incl. qualified parts, heritage, `typeof`) | 298k / 9.1 | 333k / 10.2 | 143k / 4.3 | 551k / 16.8 | 627k / 19.1 | 342k / 10.4 | 126k / 4.0 | 232k / 7.1 |
| import/export specifier name | 24k / 0.7 | 131k / 4.0 | 26k / 0.8 | 60k / 1.8 | 92k / 2.8 | 143k / 4.4 | 6k / 0.2 | 31k / 0.9 |
| import binding (default, namespace, `import =`) | 8k / 0.2 | 7k / 0.2 | 1k / 0.0 | 4k / 0.1 | 2k / 0.1 | 3k / 0.1 | 2k / 0.1 | 1k / 0.0 |
| JSDoc | 24k / 0.9 | 75k / 2.7 | 18k / 0.7 | 40k / 1.4 | 43k / 1.5 | 21k / 0.7 | 117k / 4.1 | 12k / 0.4 |
| **all identifiers** | 1,144k / 35.0 | 1,460k / 44.9 | 862k / 26.4 | 1,390k / 42.6 | 1,640k / 50.3 | 3,572k / 109.1 | 725k / 23.0 | 748k / 22.9 |
| text stored in the node (40 B) | 1.4% | 3.4% | 1.9% | 1.7% | 1.6% | 0.3% | 15.8% | 1.0% |
| with a node id | 22% | 17% | 27% | 9% | 10% | 33% | 25% | 15% |
| distinct texts in the program | 59k | 118k | 41k | 108k | 149k | 189k | 46k | 41k |

Text: 84-99.7% of identifiers derive their text from the source (96.6-99.7% on the six; webpack's reparsed JSDoc
types store it); the rest store a pointer that also points into the source text; 2-290 identifiers per project (escapes)
hold a copy, a few KB. Every identifier outside JSDoc has a flow node (the binder sets one on each, binder.rs:760),
packed into the word at no cost. Over the eight projects, node ids (assigned when the checker keys links on a node)
are on 99% of expression identifiers and on 0.0% of property-access names, 0.0% of member declaration names, 0.2% of
specifier names, 0.3% of declaration names and 0.2% of type-reference names: the checker keys almost nothing on those
name nodes. The flow nodes on them are read by none of the seven `flow_node()` / `get_flow_node_of_node` call sites in
tsrs_checker, which take references and expression locations (read, not proven).

### 4.3 Literals, tokens, lists (post, count / MiB)

| | t3code | supabase | mikro-orm | cal-diy | formbricks | vscode | webpack | drizzle |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| string literals (40 B) | 160k / 6.1 | 248k / 9.5 | 96k / 3.7 | 244k / 9.3 | 338k / 12.9 | 352k / 13.4 | 132k / 5.0 | 228k / 8.7 |
| numeric, bigint, regexp literals (40 B) | 16k / 0.6 | 42k / 1.6 | 34k / 1.3 | 13k / 0.5 | 16k / 0.6 | 89k / 3.4 | 38k / 1.5 | 26k / 1.0 |
| template parts | 11k / 0.7 | 20k / 1.3 | 11k / 0.7 | 9k / 0.6 | 7k / 0.5 | 47k / 2.9 | 13k / 0.8 | 14k / 0.9 |
| `Token` nodes (modifiers, `?`, `=>`, operators; 24 B) | 222k / 5.1 | 261k / 6.0 | 118k / 2.7 | 294k / 6.7 | 348k / 8.0 | 686k / 15.7 | 119k / 2.7 | 102k / 2.3 |
| keyword type nodes (24 B) | 113k / 2.6 | 158k / 3.6 | 79k / 1.8 | 178k / 4.1 | 171k / 3.9 | 252k / 5.8 | 104k / 2.4 | 111k / 2.5 |
| lists of 0 (count / struct + elements MiB) | 30k / 0.5 + 0 | 58k / 0.9 + 0 | 66k / 1.0 + 0 | 35k / 0.5 + 0 | 42k / 0.6 + 0 | 182k / 2.8 + 0 | 38k / 0.6 + 0 | 31k / 0.5 + 0 |
| lists of 1 | 382k / 6.4 + 1.5 | 445k / 7.3 + 1.7 | 291k / 4.7 + 1.1 | 330k / 5.5 + 1.3 | 534k / 9.3 + 2.0 | 1,037k / 16.9 + 4.0 | 235k / 3.7 + 0.9 | 217k / 3.6 + 0.8 |
| lists of 2-3 | 172k / 2.7 + 1.4 | 240k / 3.8 + 2.1 | 118k / 1.8 + 1.0 | 227k / 3.6 + 1.9 | 231k / 3.6 + 2.0 | 480k / 7.7 + 4.1 | 112k / 1.7 + 1.0 | 129k / 2.0 + 1.1 |
| lists of 4-8 | 32k / 0.5 + 0.6 | 54k / 0.8 + 1.1 | 22k / 0.3 + 0.4 | 38k / 0.6 + 0.7 | 42k / 0.6 + 0.8 | 101k / 1.5 + 1.9 | 22k / 0.3 + 0.4 | 18k / 0.3 + 0.3 |
| lists of more than 8 | 7k / 0.1 + 0.6 | 13k / 0.2 + 1.3 | 7k / 0.1 + 0.5 | 13k / 0.2 + 1.4 | 13k / 0.2 + 2.0 | 26k / 0.4 + 2.0 | 6k / 0.1 + 0.6 | 6k / 0.1 + 1.1 |
| of which modifier lists (24 B) | 90k | 84k | 44k | 79k | 169k | 194k | 20k | 38k |
| lazy member lists, forced / not forced | 5k / 15k | 8k / 33k | 0 / 0 | 9k / 36k | 11k / 105k | 6k / 7k | 0 / 0 | 6k / 13k |
| nodes with nonzero flags / with a node id | 61% / 16% | 58% / 10% | 72% / 24% | 50% / 7% | 69% / 8% | 28% / 27% | 100% / 18% | 75% / 10% |

Literal text copied out of the source (escapes): 0.04-1.97 MiB. Lists (not counting lazy member lists): 51-62% hold
one element, 5-13% none.

## 5. Designs (savings derived from the counts; interner and side-table costs estimated)

### 5.1 What each design changes, per node after 8-byte rounding

- **A, identifier text as a 32-bit atom in the node.** Atom (4 B) + flow handle (4 B) is today's 8-byte word, so a
  compact identifier stays 32 B: 0 B saved on 96.6-99.7% of identifiers. An identifier that stores its text (40 B)
  becomes 32 B. The interner costs ~14 B per distinct text (an 8-byte text pointer plus a hash slot; estimate), and
  the measured global interner cost +9% parse time (mem-round2.md:203-212, section 9: sharded mutex tables, hashing
  7.8M identifiers). A per-file interner avoids the locks but its atoms do not compare across files; a parallel merge
  of per-file tables keeps the hashing and adds a remap pass (estimate: most of the cost stays; not measured). A saves
  nothing to pay for either.
- **B, identifier nodes removed where the checker keys nothing on them, the atom in the parent.** The parent's 4-byte
  name field holds the atom instead of the handle, so the parent does not grow; a "name is inline" bit fits the header
  word's spare bits (with 32-bit handles bits 32-44 of the parent field are always 0, ptr.rs:444-450). 32 B saved per
  name (40 B for stored text). *B core*: property-access names, member declaration names (property and method
  signatures and declarations, accessors, enum members), import/export specifier names; interner over their distinct
  texts. *B ext*: also object literal, JSX attribute and binding-pattern property names. *B max*: every role the
  checker gives no node id (adds declaration names, type-reference names, import bindings, JSDoc); interner over all
  texts. For property-access names alone no interner is needed: their text is the source slice that ends at the
  parent's end, so a 20-bit text index and a 12-bit length fit the 4-byte field, with a real node kept for longer or
  escaped names (2.0-5.5 MiB on the five application projects, 30.5 MiB on vscode).
- **C, a list as an inline (start, len) range into one per-file handle array.** The 16-byte `NodeList` and 24-byte
  `ModifierList` go (modifier flags recomputed from at most three nodes); elements stay 4 bytes each; lazy member lists
  keep their records. A list's position can be derived: the parser makes `pos` the first element's `pos` and `end` the
  last element's `end` or the end of a trailing comma (one bit), and an empty list keeps its `pos` in the unused start
  field (to verify on the AST oracle; synthesized and reparsed lists need a fallback). Node growth is computed per data
  struct with exact rounding, None list fields included: *ranges*: each list field 4 -> 8 B; *shared start*: one 4-byte
  start per node (its lists are consecutive in the array) and a 16-bit length per list; *positions kept*: 16 B per
  list field.
- **D, from the counts.** *D1*: the node id out of the header (24 -> 20 B, 4-aligned data at offset 20) saves 8 B on
  nodes whose data struct is 4 mod 8 bytes; ids must stay dense because the id-keyed link stores page by consecutive
  ids (notes/mem-layout.md "Id-keyed link stores paged by id", crates/tsrs_checker/src/links.rs:285) and the API
  codec orders nodes by id (crates/tsrs_api_codec/src/encoder.rs:55-75), so the 7-27% of nodes that get an id keep it
  in a side table (~12 B each, estimate). *D2*: `TokenFlags` out of string, numeric, bigint and regexp literals (40 ->
  32 B). *D3*: every payload-less `Token` node (24 B) replaced by its kind in the parent. Measured and not pursued: a
  16-byte header (flags out too) would need a side table for the 28-72% of nodes with nonzero flags; headers are
  54-58% of the tree, but the design that removes them (bun's header-less rows) was sized at months
  (bun-check-memory.md section 2).

### 5.2 Savings (MiB at the peak, % of the 8-checker peak; **bold**: alone enough to go below bun)

| project | gap | A | B core | B ext | B max | C ranges | C shared start | C positions kept |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| t3code | 657 | -0.7 | 8.8 (0.5%) | 10.9 (0.6%) | 26.3 (1.5%) | 5.9 (0.3%) | 7.0 (0.4%) | -3.6 |
| supabase | 180 | -1.2 | 9.1 (0.6%) | 12.2 (0.9%) | 34.4 (2.4%) | 6.4 (0.5%) | 8.2 (0.6%) | -7.5 |
| mikro-orm | 202 | -0.4 | 8.2 (0.5%) | 9.9 (0.6%) | 18.8 (1.1%) | 4.8 (0.3%) | 5.8 (0.4%) | -2.7 |
| cal-diy | 157 | -1.3 | 10.3 (0.7%) | 11.5 (0.8%) | 36.7 (2.5%) | 4.2 (0.3%) | 5.8 (0.4%) | -8.5 |
| formbricks | 79 | -1.8 | 9.9 (0.6%) | 10.8 (0.6%) | 42.7 (2.5%) | 6.1 (0.4%) | 9.2 (0.5%) | -10.5 |
| vscode | 57 | -2.5 | 40.7 (2.1%) | 45.1 (2.3%) | **70.8 (3.6%)** | 19.0 (1.0%) | 22.0 (1.1%) | -7.7 |
| webpack | -42 | 0.3 | 4.7 (1.0%) | 4.7 (1.0%) | 16.8 (3.7%) | 3.1 (0.7%) | 4.2 (0.9%) | -4.0 |
| drizzle | -136 | -0.5 | 5.0 (0.8%) | 6.1 (1.0%) | 18.8 (3.2%) | 2.6 (0.4%) | 3.8 (0.6%) | -4.8 |

| project | D1 20-byte header | D2 literal flags | D3 no token nodes | B core + C shared + D2 | B max + C shared + D2 + D3 | ceiling: the whole tree |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| t3code | 1.8 (0.1%) | 1.3 (0.1%) | 5.1 (0.3%) | 17.1 (0.9%) | 39.8 (2.2%) | 133.4 (7.4%) |
| supabase | 5.1 (0.4%) | 2.2 (0.2%) | 6.0 (0.4%) | 19.5 (1.4%) | 50.8 (3.6%) | **194.3 (13.7%)** |
| mikro-orm | -0.6 | 1.0 (0.1%) | 2.7 (0.2%) | 14.9 (0.9%) | 28.3 (1.7%) | 101.6 (6.2%) |
| cal-diy | 6.5 (0.4%) | 2.0 (0.1%) | 6.7 (0.5%) | 18.0 (1.2%) | 51.2 (3.5%) | **175.6 (11.9%)** |
| formbricks | 6.2 (0.4%) | 2.7 (0.2%) | 8.0 (0.5%) | 21.8 (1.3%) | 62.6 (3.6%) | **211.0 (12.3%)** |
| vscode | -4.2 | 3.4 (0.2%) | 15.7 (0.8%) | **66.1 (3.4%)** | **111.9 (5.7%)** | **421.5 (21.4%)** |
| webpack | 0.6 (0.1%) | 1.3 (0.3%) | 2.7 (0.6%) | 10.2 (2.3%) | 25.0 (5.5%) | 93.3 (20.7%) |
| drizzle | 2.2 (0.4%) | 1.9 (0.3%) | 2.3 (0.4%) | 10.7 (1.8%) | 26.9 (4.5%) | 96.7 (16.3%) |

Nothing alone or combined puts tsrs below bun on t3code, supabase, mikro-orm, cal-diy or formbricks; on t3code even
the whole tree is a fifth of the gap. vscode flips with B max or the realistic package by 9-14 MiB over the README
run's 57 MiB gap, by 3-8 MiB over the next run's 63, and not against the compare file's 73 (tsrs 1,984 vs bun 1,911
MiB, mean of 10); only the set with every design clears all three.

### 5.3 Gross and costs (MiB)

| project | A: gross - interner | B core: gross - interner (names removed) | C: list structs - node growth (ranges / shared start / positions kept) | D1: gross - id side table |
| --- | ---: | ---: | ---: | ---: |
| t3code | 0.1 - 0.8 | 9.1 - 0.3 (298k) | 10.2 - 4.3 / 3.2 / 13.7 | 8.0 - 6.2 |
| supabase | 0.4 - 1.6 | 9.9 - 0.9 (325k) | 13.0 - 6.6 / 4.8 / 20.5 | 10.4 - 5.3 |
| mikro-orm | 0.1 - 0.5 | 8.5 - 0.3 (278k) | 8.0 - 3.3 / 2.2 / 10.8 | 6.3 - 6.9 |
| cal-diy | 0.2 - 1.4 | 11.0 - 0.6 (359k) | 10.4 - 6.2 / 4.7 / 18.9 | 10.2 - 3.6 |
| formbricks | 0.2 - 2.0 | 10.7 - 0.7 (350k) | 14.5 - 8.3 / 5.3 / 24.9 | 10.6 - 4.4 |
| vscode | 0.1 - 2.5 | 42.2 - 1.5 (1,382k) | 29.3 - 10.3 / 7.3 / 37.1 | 28.4 - 32.7 |

B ext and B max are charged an interner over every distinct text in the program (an upper bound).

### 5.4 Freed leaf trees

At the peak most leaf trees are already freed (section 3). If every one were still alive, the designs would save this
much more (MiB, `pre` leaf trees): t3code 54 MiB of tree, +7.9 for B core + C shared + D2; supabase 23, +3.4;
formbricks 49, +7.4; vscode 257, +38.0; cal-diy 3, +0.4; mikro-orm 0. Even then the five application projects stay
below 2% for that package; vscode would reach 5.3% (104 MiB), in a case the leaf-freeing measurement rules out.

### 5.5 Shared across checkers

All of it. Every design changes only the parsed program, which the parser builds once and every checker reads; the
lazy member lists and JSDoc the checkers force are cached in the file and shared too. Each saving counts once at any
checker count: at 32 checkers the same MiB are a smaller share of a larger peak, single-threaded a larger one (t3code
single-threaded peaks near 871 MiB, mem-round4.md section 1, where B core would be 1.0%).

## 6. Effort, risk and blast radius

Blast radius: hand-written lines (generated lines in parentheses) at 8a95541 in the crates that consume the tree,
one regular expression per pattern, comment lines and tests excluded (`tools/sizing/blast_radius.py`). Weeks are
estimates.

| design | saving on the six | effort | risk | blast radius |
| --- | --- | --- | --- | --- |
| A | -0.4..-2.5 MiB (a loss) | 1-2 weeks | low; +9% parse measured for the global form | identifier.rs, the parser's identifier creation, `new_identifier` |
| B core | 8.2-10.3 MiB (0.5-0.7%); vscode 40.7 (2.1%) | 6-10 weeks | medium: name nodes lose their identity | 1,317 lines (+85) test for or cast to the 10 parent kinds or the element predicates that cover them: property-access parents 269, member declarations 885, specifiers 163 (checker 427, LS 308, ast 213, transformers 158); 1,391 generic `.name()` reads (+245) to audit (checker 549, LS 266, transformers 251), 102 `property_name()` reads, 138 calls of `get_name_of_declaration` / `declaration_name_to_string` / property-name text helpers |
| B max | 18.8-42.7 MiB (1.1-2.5%); vscode 70.8 (3.6%) | months | high | B core plus every declaration and type-reference name: binder symbol names, checker diagnostics at names, all of the LS |
| C | 4.2-9.2 MiB (0.3-0.6%); vscode 22.0 (1.1%) | 4-8 weeks | medium: derived positions must be exact for the printer's comments, the API's list `pos`/`end` and trailing commas | 354 lines name `NodeList`/`ModifierList` (+440), 274 list-accessor calls, 493 list-factory calls (228 in transformers), 30 list-position reads, 176 list-visitor calls (+127); every list field in tools/gen-ast/gen-ast.ts |
| D1 | -4.2..+6.5 MiB | 1-2 weeks | low | the header word split in two (ast.rs:421-512), `get_node_id` (utilities_1.rs:53) |
| D2 | 1.0-2.7 MiB | days | low | literal factories and token-flag readers |
| D3 | 2.7-8.0 MiB; vscode 15.7 | months | high: a checker API rewrite (bun-check-memory.md section 3) | every operator, modifier and punctuation token field |

## 7. Fidelity

All of these are representation changes: the parser makes the same syntax with the same texts and positions, the
binder and checker read the same values through the accessors, so diagnostics cannot change as long as every accessor
returns what it returns today. A and C keep every node. A changes only where the text lives (`text()` would return
the interned slice, pointer-equal across occurrences; the symbol-table pointer fast path, perf-round2-followups.md
item 5, would hit more often). C removes the `NodeList` objects: whatever hands one out must build one or take a
range. B removes nodes: whatever hands out a name node must make one on demand and cache it per parent to keep its
identity and parent pointer.

Outputs that touch the moved fields:

- API codec (crates/tsrs_api_codec): encodes every node, the name identifiers included, and each node list as a
  record with `pos`/`end` (the TypeScript API's `NodeArray`); handles are ordered by node id (encoder.rs:55-75). B and
  C change what it walks; D1 must keep the id order.
- Language service and LSP: `astnav` finds the token under the cursor, and hover, rename, references, completions and
  go-to-definition start from name nodes; signature help uses argument-list spans. B needs synthesized name nodes
  there; C needs list spans.
- Printer and declaration emit: emit names from identifier nodes, use list positions for comments and trailing
  commas, and record name positions in source maps.
- fourslash: runs all of the above.

## 8. What was measured before, and what is different now

- notes/bun-check-memory.md sections 2-3 (2026-10-07, read from Bun's code against vscode at 64 vCPU, estimates):
  names as inline atoms 75-110 MB, "months, not layout-only"; header-less rows 60-100 MB, lazy parents net negative,
  id-as-handle 0 B after rounding; lists as inline ranges 10-44 MB, "lenses disagree"; flat symbol tables 12-15 MB
  (built as #147, closed below the bar); literal `TokenFlags` 0-7.7 MiB; inline single declaration at most 9.7 MB;
  numeric literals into a table 0 MB.
- notes/mem-round2.md:203-212: a global identifier interner, -0.075 GiB single-threaded for +9% parse time, rejected
  for `PackedStr`; notes/perf-round2-followups.md item 5: full name interning for about 0.2% of instructions.
- notes/mem-small.md step 2: identifiers became 32 B with their text derived from the source; notes/mem-frontend.md:
  the tree is headers, payloads, identifiers and lists, nothing node_modules-specific.
- notes/mem-round4.md sections 1 and 6: the gap is what each extra checker holds; a type graph shared by the checkers
  is the only change that closes t3code (and notes/spike-shared-graph.md measured it at -5..-10% peak for +6..+18%
  wall at 8 checkers). AGENTS.md cites a section 7 of mem-round4.md; it has six sections at 8a95541 and on main
  4e5c85a.

Different now: (1) the default is 8 checkers on 16 vCPUs, peaks 1.4-2.0 GiB instead of 2.3-2.9, so the shared tree is
a larger share, and it is still 6-21%; (2) leaf trees are freed at up to 16 checkers (notes/mem-free-leaf-files.md),
257 MiB of vscode's tree and 54 MiB of t3code's, before the peak, so a compact tree saves less than the 64-vCPU sizing
assumed exactly where leaves are large; (3) lazy member lists of unchecked declaration files (#202) already dropped the
largest never-read part of the tree (formbricks' 254 MB ceiling); (4) these are counts of the real trees on the
losing projects, not inferences: the atom figure corresponds to B max here (70.8 MiB on vscode against 75-110 MB
inferred), lists to C (19-22 MiB, inside 10-44), literal flags to D2 (3.4 MiB, inside 0-7.7). An atom stored in the
node itself (A) saves nothing since identifiers are 32 B.

## 9. Recommendation

1. No design clears the 5% bar on any of the six at the 8-checker default. The largest single layout change measured,
   B max (months), is 1.1-2.5% on the five application projects and 3.6% on vscode; the realistic package (B core + C
   with a shared start + D2, 10-18 weeks by estimate) is 0.9-1.4% and 3.4%.
2. None flips t3code, supabase, mikro-orm or cal-diy; formbricks would need 79 MiB (37% of its tree); vscode flips
   with B max or the package by 3-14 MiB on two bench runs and misses on the third (gap 57, 63, 73 MiB).
3. The gap is checker state: on the Mac split, arena + heap grows from 0.16-0.73 GiB at parse end to 1.3-1.8 GiB at
   check end on the six; t3code's gap is 4.9 times its tree.
4. Do not start a compact-tree project for the scoreboard; this change adds A, B, C and D1-D3 to "Measured and
   rejected" in notes/perf-round2-followups.md. First week of work: none on the tree. If a smaller tree is wanted for
   another reason (the language server, single-threaded runs), start with B for property-access names, text from the
   parent's end and no interner (269 lines name the parent kind; 30.5 MiB on vscode, 2.0-5.5 MiB elsewhere).

## 10. Reproduce

```sh
# build the opt-in tool (default builds do not contain it)
cargo build --release -p tsrs_cli --features ast-sizing
python3 bench/run.py --setup-only            # checkouts in bench/.work/solutions/<name> (bench/projects.json)
# per project, from its checkout root; -p: t3code-server apps/server, supabase-studio apps/studio, cal-diy apps/web,
# formbricks-web apps/web/tsconfig.typecheck.json, vscode src, mikro-orm / webpack / drizzle-orm .
TSRS_AST_SIZING=/tmp/sizing/<name>.tsv target/release/tsrs -p <project> --noEmit --incremental false \
    --extendedDiagnostics --pretty false --checkers 8
TSRS_MEM_SPLIT=1 target/release/tsrs -p <project> --noEmit --incremental false --extendedDiagnostics --pretty false \
    --checkers 8 2> /tmp/sizing/<name>.split.err
python3 tools/sizing/compact_ast.py --dir /tmp/sizing --bench bench/results/2026-10-08-55c2d9ce5c41.json
```

Blast radius: `python3 tools/sizing/blast_radius.py <tree>` on a clean export of the base (`git archive 8a95541 crates
| tar -x -C <tree>`), so the tool's own hooks are not counted.
