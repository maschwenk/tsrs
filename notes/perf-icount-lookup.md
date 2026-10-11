# Instruction-count hill climb: table lookups and property/symbol access (8 checkers)

Goal (owner, 2026-10-11): fewer user-space instructions for a cold `tsrs --noEmit` check of a large private monorepo
at `--checkers 8`, one measured change at a time, output identical. This round's territory was the symbol tables
(`SymbolMap`), the relation cache, the link stores, `getPropertyOfType` and its neighbours, and the lazy member
tables. Twelve commits on top of 98d8d60e.

## Result

| | 98d8d60e | branch | change |
|---|---|---|---|
| VM user-space instructions, `--checkerAssignment locality` | 322.67-322.74 G | 308.83 G | -4.30% |
| VM user-space instructions, single-threaded | 192.423 G | 184.994 G | -3.86% |
| Mac instructions, default mode (median of 3) | 362.60 G | 349.25 G | -3.7% |
| Mac instructions, locality (2 runs) | 348.25 / 348.39 G | 334.15 / 334.22 G | -4.1% |
| Mac instructions, single-threaded | 213.17 G | 205.44 G | -3.6% |
| Mac peak RSS, default mode | 6.38-6.42 GB | 6.40-6.50 GB | noise |
| vscode, single-threaded (Mac) | 88.84 G | 85.30 G | -4.0% |
| t3code-server, single-threaded (Mac) | 42.71 G | 40.70 G | -4.7% |

VM: the DynamoRIO counter in the OrbStack Linux VM (arm64, user space only, exact to 0.02% in locality mode and to
1e-7 single-threaded). Mac: `/usr/bin/time -l` (kernel included, ~0.5% noise). Diagnostics identical on the four
identity projects after every commit; single-threaded `--extendedDiagnostics` counters on the private monorepo
(symbols, types, instantiations and every lazy-member counter) identical between base and branch.

## How it was measured, and where the instructions are

Every change was counted in the VM single-threaded and in locality mode against the commit before it, and kept only
if both went down; the commit messages carry the VM and Mac numbers.

The CPU-time profiles of the base (samply) weight cache misses, and most of this territory is hash probes whose time is
misses, not instructions. So this round used an instruction profile: a small DynamoRIO client (`bbcount`, kept outside
the repository) gives every basic block of the main module a counter incremented on entry; at exit it writes each
block's start, length and count. Single-threaded, the counts are exact. Expanding blocks to instruction addresses
(arm64: 4 bytes each) and symbolizing them with `llvm-symbolizer --inlining` against a `line-tables-only` build gives
exact instructions per function, per inlined frame and per source line, and exact execution counts per line (the
largest count among a line's instructions). Two things it showed that the time profile could not:

- Register save and restore dominate many small hot functions. A function that makes a call on any path saves its
  registers on all of them; the entry and exit lines of `SymbolMap::search` were 1.16 G of its 4.3 G, of
  `get_property_of_type_worker` 0.58 G of 3.2 G. Most commits here move the call off the common path.
- Constant materialization: a 64-bit constant costs four instructions on arm64, and `hash_name` loaded two at every
  hash (over 100M hashes single-threaded).

Base single-threaded instructions in the territory (VM, 189.1 G in the main module): `SymbolMap` search, position and
insert 8.6 G; `get_property_of_type_worker` 3.2 G; symbol sorting (`compare_symbols_worker` with
`source_file_and_index_of_node`) 2.0 G; `TypeData` accessors 3.4 G spread over all callers; lazy member tables
(prepare, lookup, ready-table map) 4.5 G; `get_type_of_symbol` 1.9 G.

## What landed (VM locality / single-threaded, each against the commit before)

- `compareNodes` orders two nodes with one parent by position without looking up either file (-0.39% / -0.38%;
  vscode -1.2%, t3code-server -1.5% on the Mac). Sorting a declaration's members compares nodes that nearly always
  share a parent.
- `SymbolMap::search` without a register frame on its common paths (-0.67% / -0.57%): a fingerprint-only probe, then
  the key returned when it is the looked-up string or equal in an inline compare of at most 16 bytes; longer names and
  fingerprint collisions go on in `search_text`. Single-threaded: 57.8M searches, 19.2M misses, 17.0M hits on the
  same string, 21.7M hits that compare text.
- `SymbolMap::insert` adds a new entry to an index under the hash it already has instead of taking the index out of
  its box and hashing the key again; building the index moves to a cold function (-0.36% / -0.32%).
- A lazy member lookup hashes the name once (`lookup_entry_hashed`, `set_hashed`, `HashedName::with_name`; -0.24% /
  -0.19%).
- The object, union and intersection data structs are `repr(C)` with their parent first (-0.71% / -0.61%; vscode
  -1.5%). Field reordering had put the embedded `ObjectType` at three different offsets, so `as_object_type`,
  `as_structured_type` and friends were switches (`Type::target`: 71M calls). Sizes unchanged (`InterfaceType`
  fields reordered to stay 128 bytes).
- `IdLinkStore::dense_slot` inline again (-0.30% / -0.23%): the out-of-line call (0.3M executions) made
  `get_type_of_symbol`'s cached-type return save five register pairs on 47.8M calls, undoing 0ad4294e's intent.
- `somePropertyReducesToNever` counts names in a reused map (`free_name_counts`; -0.32% / -0.31%): the growing
  `OrderedMap` rehashed 0.36 G instructions.
- Preparing a lazy member table walks the declared members in place, and `is_symbol_unaffected_by_instantiation` is
  inlined into its two callers (-0.14% / -0.14%).
- No set of names for the properties of a lazy table without base types (-0.15% / -0.10%).
- Hash constants without a load: `hash_name` xors with 0x5555... and 0x3333... (logical immediates, one `eor`
  each), and the index hash takes hashbrown's tag from name-hash bits 13-19 with one shifted `orr` instead of a
  multiply by a 64-bit constant (-0.47% / -0.48%; the name hash -0.38%, the index hash -0.10% single-threaded).
- Signatures and index infos of a type with resolved members without a register frame (-0.33% / -0.31%).
- A link-store hit is one `find` without hashbrown's `entry` machinery; the first access goes to an out-of-line
  `create` (-0.31% / -0.28%).

None of these reuses a computed answer; nothing needed a shadow mode. What they rely on: keys of a symbol table are
unique (the search returns the first text match after a fingerprint-only probe); two nodes with the same parent are
in the same file; a type with `MembersResolved` has no lazy member table and no lazy mapped table
(`may_have_lazy_members`, `get_lazy_mapped_table`); the parent struct is the first field of each `repr(C)` data
struct (only a performance property: the accessors stay correct for any layout).

## Measured and rejected

- `search` split with the whole text-compare path, including the second probe, in `search_text`: +0.03% locality,
  +0.05% single-threaded. `search` lost its frame, but `search_text` kept the eight-pair frame and 21.7M of the 57.8M
  searches went there. Kept form: compare up to 16 bytes inline in `search`, so only long names and collisions leave.

## Found, not pursued

- Generated AST accessors are switches over per-kind field offsets, as the `TypeData` accessors were:
  `Node::modifiers` is 42M calls single-threaded with a 10-instruction match (0.42 G), and `Node::name` and
  `Node::expression` are similar (0.6-0.66 G each). Putting the shared bases (`modifiers_base`, ...) first in
  `repr(C)` payloads would make them a tag-mask test and one load; it is a generator change (crates/tsrs_ast).
- `get_declaration_modifier_flags_from_symbol_ex` with `get_combined_modifier_flags` and `Node::modifiers`: ~2.2 G
  single-threaded, 30.9M calls, mostly the relater's `propertyRelatedTo` and union property synthesis on the same
  symbols. A per-symbol memo would need a proof that a symbol's declarations and value declaration no longer change
  (late-bound members, merges).
- The index probe of large symbol tables: 1.54 G for 39.8M searches (hashbrown's group probe plus the fingerprint
  test). A purpose-built open-addressing index might halve it; a filter for indexed tables was rejected in round 3.
- Remaining frames on hot paths (calls x saved pairs x 2): `get_property_of_type_worker` ~0.5 G (an out-of-line
  fast path was rejected in round 1; the slow path would now not repeat the lookup), `insert_hashed`,
  `compare_symbols_worker`, `get_combined_modifier_flags` ~0.2 G each, `Type::target` 0.14 G (its panic paths).
- Lazy member tables: the `lazy_member_tables` map lookup in `get_ready_lazy_member_table_worker` (19.3M calls,
  ~0.75 G) and the per-instantiation scan of a source's declared members in `prepare_lazy_members` (~1.3 G for 1.0M
  tables; the named-member filter could be kept per source type).
- `FxHashSet<&str>` name sets (union property collection, lazy tables with base types): 0.36 G inserts plus 0.18 G
  rehash; the relation table's `reserve_rehash` 0.32 G.

## Gates

`tools/regressions.sh` 29/29; `tsrs-test run --suite all`: 13,458 passes, the base pass list minus ours empty;
`tools/lint/ratchet.py` and `tools/lint/source.py` clean; `RUSTFLAGS="-D warnings" cargo check --workspace` clean;
`cargo test -p tsrs_ast -p tsrs_checker` pass.
