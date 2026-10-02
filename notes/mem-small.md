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

## 2. Identifier text from the source: `NodeAlloc<Identifier>` 40 -> 32 bytes

mem-round3 thought the text pointer and length were 12-16 of the 40 bytes; since mem-round2 step 9 the text is a
`PackedStr` (8 bytes) and the other 8 are the flow node, so a 32-byte identifier needs both in one word. Counted once
(throwaway counters, the private monorepo single, default mode):

- 8,145,484 identifiers allocated; 7,862,185 by the parser's `createIdentifier` path (including those rolled back
  with speculative parses), **all** with a token value that is the slice of the file's text ending at the token
  end (no unicode-escaped identifier in the project); the other 283,299 are missing identifiers, clones, JSDoc
  and synthesized names.
- The binder sets a flow node on every identifier it visits (7,164,357 of 7,164,357 with `currentFlow` non-nil), so
  the flow node cannot move out of the identifier.
- `Identifier::text()` is called 50.8M times per check.

Design (`crates/tsrs_ast/src/identifier.rs`, hand-written; `tools/gen-ast/gen-ast.ts` skips `Identifier`'s
struct, factory and clone): each parsed file's text is registered once in a global table (`register_source_text`,
1M zero-initialized slots; the index goes into `SourceFile.text_index`). A parser identifier is one word: the text
length (17 bits), a 2-bit mode, and 45 bits that hold the text index until the binder sets a flow node, then the
flow node's address / 8. The binder stamps every flow node it makes with its file's text index
(`FlowNode.text_index`, in the struct's former padding; the checker's three flow nodes get `NO_SOURCE_TEXT`), so the
text is `source_text(index)[end - len..end]` with the index from the word or from the flow node and `end` from the
node's range. A flow node from another text (never seen) goes to a mutex-guarded side map. Identifiers that are not
such a slice (the factory's `new_identifier`) store the `PackedStr` after the word (40 bytes, as before).
`new_source_identifier` checks the slice by pointer equality before choosing the compact form and sets the range;
`Node::set_loc` panics if a compact identifier's end would change (`Node.loc` is now crate-private, so every write
goes through it or through the factory on a fresh node). `text()` returns the same slice as before (same pointer and
length). The flow node of every kind is now read and written through `Node::flow_node()` / `set_flow_node()` /
`has_flow_node_data()` (21 call sites; the generated `flow_node_data()` became the crate-private `flow_node_base()`,
so no caller can miss an identifier's flow node).

Arena (alloc-profile, single): `NodeAlloc<Identifier>` 310.7 MB (8.15M x 40) -> 239.9 MB (7.86M x 32) + 10.8 MB
`IdentifierWithText` (283K x 40).

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, step 1 | 5.819-5.839 (5.834) | 303.6-305.2 G |
| single, after | 5.764-5.772 (5.765, -0.069) | 304.4-306.2 G (+0.3%: +0.8, +1.7, +0.8 G pairwise) |
| 4 checkers, step 1 | 7.780-7.787 (7.782) | 415.4-415.6 G |
| 4 checkers, after | 7.733-7.737 (7.737, -0.045) | 416.5-417.3 G (+0.3%) |
| opt-out single / 4 checkers (go assignment), step 1 -> after (1 run) | 7.750 -> 7.692 / 11.912 -> 11.843 | 328.9 -> 330.4 / 493.1 -> 493.1 G |

(An earlier round of the same comparison, with the bounds check still in `text()`: single 5.819 -> 5.774, 4 checkers
7.795 -> 7.732, instructions +0.3% / +0.4%. Dropping the check, which `set_loc` and the creation check make
redundant, is the landed version.) The extra instructions are the text index lookup in `text()` (the flow node's
`text_index`, then the table) and the identifier tests in `set_loc` and the flow accessors, within the 0.5% budget.

Gates: suite trees identical to step 1 in all three modes, also with a dev build (debug assertions) and
`TSRS_CHECK_SHARED=1`; AST oracle 113/113 libs, 17,318/17,319 test units (the known non-UTF-8 file) and 38,911/38,911
private-monorepo files identical to Go; private-monorepo output and counters identical in all four runs (and with
the dev build, 4 checkers). Census (`TSRS_CENSUS_VERIFY=1`, both lazy modes): precise walk 0 references to freed
blocks; the strong mark reported one "violation" in each mode, a heap word of `new_checker` whose two `u32` halves
(0x7c00, 0x7c01 / 0x7c02) read as the address of a freed `[P<InferenceInfo>]` block in the census arena region
(0x7c00_0000_0000+). It is a coincidence of the census's address choice: with the census arena moved to
0x7d40_0000_0000 (local patch, not landed) the same binary reports 0 violations, and step 1 reports 0 at the usual
base. (The census strong mark decodes x8-encoded words only in heap blocks, so it does not follow identifiers' flow
nodes; the precise walk does, through `flow_node()`.)

## 3. Next arena rows: `Symbol` 48 -> 40 bytes (value declaration as a bit)

Profile after step 2 (alloc-profile, single): Symbol 605 MB (13.2M x 48), TypeMapper 255, NodeAlloc<Identifier> 240,
[ValueSymbolLinks] 238, [P<Type>] 217, ObjectType / TypeReference 167 / 165, Signature 136, StructuredMembers 125.
`Symbol` is still the largest row, so it went first.

Counted once (every symbol registered at creation, inspected at exit; the private monorepo single):

| symbols | value declaration nil | = first declaration | another node |
| --- | --- | --- | --- |
| binder (3.92M) | 689,627 | 3,228,501 | 2,672 |
| transient, default mode (9.29M) | 1,621,384 | 7,650,022 | 20,367 |
| transient, opt-out (22.98M) | 3,454,097 | 19,505,610 | 20,360 |

So the value declaration is either nil or `declarations[0]` for 99.8% of the symbols. A bit of the parent/tail word
(bit 62 of `SymbolParentWord`) now says "the value declaration is the first declaration"; another node is kept in the
symbol's tail (`SymbolTables.value_declaration`, the tail grows 32 -> 40 bytes). `set_value_declaration` picks the
form, `value_declaration()` reads `declarations[0]` when the bit is set, and every write of the declarations
(`set_declarations`, the new `set_declarations_static` for Go's slice sharing, `append_declarations`) moves the value
declaration into the tail first when the new slice's first element differs. The bit is only set while the
declarations are non-empty. `declarations` is private now, `value_declaration` an accessor pair (37 + 23 sites,
mechanical). Same owner-only write contract as the other `Symbol` words.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, step 2 | 5.764-5.781 (5.773) | 305.5-306.0 G |
| single, after | 5.668-5.684 (5.681, -0.092) | 306.2 G (+0.2%) |
| 4 checkers, step 2 | 7.726-7.742 (7.736) | 416.1-417.1 G |
| 4 checkers, after | 7.613-7.631 (7.628, -0.108) | 416.9-417.6 G (+0.2%) |
| opt-out single (3 runs) | 7.691-7.698 -> 7.498-7.500 (-0.19) | 329.0-331.9 -> 331.0-333.7 G (noise: +-1.5%) |
| opt-out 4 checkers, go assignment (1 run) | 11.828 -> 11.541 (-0.29) | 496.3 -> 495.6 G |

(A first version indexed `declarations[0]` with a bounds check: single +0.7% instructions in two of three pairs;
`get_unchecked` under the non-empty invariant, which a `debug_assert` checks, is the landed one.)

Gates: suite trees identical to step 2 in all three modes, also with a dev build (debug assertions) and
`TSRS_CHECK_SHARED=1`; private-monorepo output and counters identical in all four runs (and the dev build, 4
checkers, without panics).

## 3b. Type header 32 -> 24 bytes: the alias shares a word with the symbol

Every type (9.9M single, 14.4M on 4 checkers) carried Go's `alias` in its header, but only the 0.6M types created
with an alias have one (`TypeAlias` records: 604,835, mem-round3 A6 allocates one only for a type created with
it). Like step 1, `alias` now shares one word with `symbol` (`TypeSymbolWord`): the word holds the symbol until the
first non-nil alias write, which allocates a `TypeSymbolAlias` record (symbol + alias, 16 bytes) and stores its
address with bit 63 set. `symbol()` reads the word (or the record), `alias()` the record (nil without one);
`set_symbol()` / `set_alias()` replace the 30 direct field accesses. Header: flags, object flags, id and data tag
in 16 bytes plus that word; `TypeAlloc<T>` data now starts at offset 24. Types are checker-owned, plain `Cell`s as
before.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, step 3 | 5.685-5.687 (5.686) | 306.7-307.5 G |
| single, after | 5.618-5.622 (5.620, -0.066) | 307.2-307.8 G (+0.1%) |
| 4 checkers, step 3 | 7.592-7.629 (7.628) | 417.0-419.2 G |
| 4 checkers, after | 7.506-7.532 (7.526, -0.102) | 417.5-420.4 G (pairwise +3.4, -0.2, +0.3 G) |
| opt-out single / 4 checkers, go assignment (1 run) | 7.498 -> 7.438 / 11.491 -> 11.428 | 333.4 -> 332.4 / 498.6 -> 497.5 G |

Gates: suite trees identical to step 3 in all three modes, also with a dev build and `TSRS_CHECK_SHARED=1`;
private-monorepo output and counters identical in all four runs.
