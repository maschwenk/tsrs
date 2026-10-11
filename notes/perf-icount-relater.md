# Instruction-count hill climb, relater and union/intersection construction (2026-10-11)

Goal (2026-10-11): fewer user-space instructions for a cold `--noEmit` check of a large private monorepo at
`--checkers 8`, one measured change at a time, output identical. This round covered the relation checker
(`relater_*.rs`) and union/intersection construction. Base: 98d8d60e (the three rounds of
notes/perf-icount-hillclimb.md already on main). Eleven commits, two rejected ideas and one rejected variant.

## Result

Base 98d8d60e against this branch, `-p <tsconfig> --noEmit --incremental false --pretty false`, diagnostics identical
(the workload has none; the public projects below are the identity gate).

| | base | branch | change |
|---|---|---|---|
| Linux VM user-space instructions, 8 checkers locality | 322.67-322.74G | 313.40G | -2.9% |
| Linux VM user-space instructions, single-threaded | 192.423G | 187.542G | -2.5% |
| Linux VM user-space instructions, 8 checkers default (one run; base 4 runs) | 334.8-340.5G | 331.5G | about -2% (stealing noise 1.7%) |
| Mac instructions (user+kernel), 8 checkers default, median of 3 | 362.85G | 354.37G | -2.3% |
| Mac, 8 checkers locality (2 runs) | 348.41-348.43G | 339.16-339.19G | -2.7% |
| Mac, single-threaded | 212.99G | 208.01G | -2.3% |
| peak RSS, 8 checkers default | 6.38-6.43 GB | 6.39-6.43 GB | none |
| vscode single-threaded (VM) | 81.145G | 80.781G | -0.45% |
| t3code-server single-threaded (VM) | 39.339G | 38.892G | -1.1% |

The Mac rows were measured back to back in one session (`/usr/bin/time -l`); Mac counts include kernel instructions and
moved by up to 1% between sessions of the same binary, so every A/B below uses the VM counts.

Gates on the final tree: diagnostics md5 unchanged on vscode, cal-diy, t3code-server and supabase-studio at 8
checkers (and vscode, t3code-server single-threaded); conformance pass list identical to base (13,458), also with
`TSRS_DEEP_NESTED`, `TSRS_UNION_CACHE`, `TSRS_INFER_MEMO` and `TSRS_IA_MEMO` all in shadow mode (no crashes);
`tools/regressions.sh` 29/29; `tools/lint/ratchet.py` and `tools/lint/source.py` clean; `RUSTFLAGS="-D warnings"
cargo check --workspace` clean; `cargo test -p tsrs_checker` passes.

## How it was measured

- Counts: DynamoRIO in a Linux arm64 VM, user space, summed over threads; `--checkerAssignment locality` repeats to
  0.02% and single-threaded to 1e-7, so changes of 0.05% are visible single-threaded. Each kept change was counted in
  both modes against the previous commit's binary.
- Where the instructions go: a DynamoRIO client that counts executions per basic block (single-threaded), attributed
  to physical functions, inline frames and lines with `llvm-symbolizer`. Entry counts per function (executions of
  its first block) give calls and instructions per call. The CPU-time profiles of the Mac were a poor guide here:
  `is_deeply_nested_type` and the relation cache lookups rank high by time (memory stalls) but the instruction
  profile ranks other things first, and a change that saved a third of the walk's `has_matching_recursion_identity`
  calls (below, rejected) added instructions.
- Temporary counters (an atomic array printed at exit, not committed) for distributions: stack lengths of the
  deeply-nested walks, union/intersection property sizes, template-literal pairs, union front cache stores.

## What landed

Per commit, previous commit -> this (VM locality / VM single-threaded):

1. `some_property_reduces_to_never` counts property names in a pooled open-addressing table (`NameCounts`, the
   32-bit symbol-table name hash) instead of an `IndexMap<&str, i32>` per call: -0.59% / -0.56%. 330K calls
   single-threaded over 9.2M names; the map was 1.23G instructions (3,700 a call).
2. Union reduction tests a string literal against a template literal's first and last texts before calling
   `isTypeMatchedByTemplateLiteralType` (the test `inferFromLiteralPartsToTemplateLiteral` starts with; nothing else
   happens before it), and `compare_types_assignable_comparer` returns a function item instead of allocating a
   closure in the arena per call: -0.88% / -0.74%. 12.5M (literal, template) pairs, 1.9G instructions; 2,230 pairs
   pass the text test.
3. Such a reduction counts as impure for the union front cache only when a literal reaches the full match:
   -0.13% / -0.14% (impure misses 14,292 -> 4,677). Shadow-checked with `TSRS_UNION_CACHE=shadow` (with the
   inference and indexed-access memo shadows) on the workload at 1 and 8 checkers and the conformance suite.
4. `getPropertiesOfUnionOrIntersectionType` deduplicates names in a pooled `NameSet` instead of an
   `FxHashSet<&str>`: -0.11% / -0.11%.
5. Each relater stack entry keeps its recursion identity (`RecursionStackEntry`): the first deeply-nested walk to
   reach an entry computes it as `hasMatchingRecursionIdentity` would (so whatever that resolves is resolved at the
   same point as before), later walks compare one word; the tested type, when it is the top entry, gets the identity
   the walk computes anyway; the length test is inline: -0.41% / -0.29%. 18.8M tests and 45.6M entry visits
   single-threaded, 2.1G instructions before.
6. The same for inference's stacks (`invokeOnce`, depth 2): -0.12% / -0.13%.
7. `add_type_to_intersection` forced inline into the loop of `add_types_to_intersection`, its only caller (26.4M calls,
   about 16 of 55 instructions a call were prologue and epilogue): -0.15% / -0.15%.
8. An intersection entry keeps its constituents' identity keys (an arena slice, the same one the per-intersection map
   holds) instead of looking up the map on each visit: -0.21% / -0.20%.
9. `is_simple_type_related_to` answers two structured or instantiable types in an inline wrapper; the worker's frame
   (18 of 59 instructions a call, 27.7M calls) is set up only for the other cases: -0.20% / -0.11%.
10. One stack borrow per push in `recursive_type_related_to`, the relation read once: -0.08% / -0.07%.
11. `signatures_of_structured_type` answers a type with resolved members (not mapped) inline: -0.03% / -0.07%.

Items 5, 6 and 8 are reuses of computed answers. What they rely on: an entry's recursion identity follows from the
type's structure (flags, symbol, deferred reference node, tuple target, conditional root) and, for an instantiated
mapped type, from its modifiers type, which is set once (`get_modifiers_type_from_mapped_type`). `TSRS_DEEP_NESTED=shadow`
(docs/DEBUGGING.md) makes every test again with a fresh walk and recomputes every kept identity, panicking on a
difference; it ran clean on the workload single-threaded and at 8 checkers and on the conformance suite.

The previous round rejected "recursion identities cached per relater stack entry (+0.13%)" on another workload. The
kept version differs in three ways: identities are filled lazily by the walks (nothing is computed at push), the
pushed type's identity comes from the walk's own computation, and intersection entries use the per-intersection
identities that round added later.

## Measured and rejected

- An exact early exit from the deeply-nested walk (stop once the entries left cannot bring the count to the depth;
  the entries after that point visited only when their identity lookup can resolve something): VM locality
  322.78G against 322.67-322.74G, single-threaded 192.499G against 192.423G (+0.04%). It removed a quarter of the
  `has_matching_recursion_identity` instructions but the extra test and tail loop cost more; the kept identities
  (item 5) were the better answer to the same profile line.
- `Relation::set` through hashbrown's `entry` (one probe instead of `find_mut` then `insert_unique`):
  single-threaded -0.005%, locality -0.03% (within 1.5 spreads). A miss's `insert_unique` does not compare keys, so
  there was nothing to save.
- A 256-bit Bloom filter of each list's names in `NameCounts`, counting only names whose bit another list sets:
  +0.03% in both modes. The filters of long property lists are dense, so most names pass and pay the extra pass.

## Found, not pursued

- Signature relations are 3.4x the single-threaded CPU at 8 checkers (the brief's question). Under
  `compare_signatures_related` the growth is the relations of parameter and return types (2.15 -> 7.57 s) and
  `get_min_argument_count` / `get_non_circular_return_type_of_signature` resolving parameter and return types of
  declarations in other files (0.05 -> 0.47 s, 0.09 -> 0.30 s): work every checker repeats for its own copy of the
  types, not repeated work inside one checker. Per call, the comparison does nothing an earlier result could answer
  that Go does not also do.
- Repeated misses of union/intersection property synthesis: 0.85M of 1.4M misses single-threaded repeat an earlier
  miss for the same (type, name), at about 2.2 constituents each. An upper bound of ~0.2% before the cost of a
  filter and its validity conditions (no member or type resolution in progress, members reset); not tried.
- `get_declaration_modifier_flags_from_symbol_ex(prop, true)` equals the read flags for a non-synthetic symbol with
  no set accessor among its declarations (the symbol's flags are not enough: a mapped property can carry a set
  accessor's declaration without `SetAccessor`); `create_union_or_intersection_property` computes both for 11.8M
  constituents. Estimated 0.1-0.15%.
- `Relater::get_error_state` clones `related_info` on every structured comparison (12.85M calls, 0.35G); the
  vector only grows between a save and its restore, so a length would do, but restore order would then matter.
- The relation table's growth rehashes (0.32G single-threaded per checker), the prologue of `is_related_to_ex`
  (0.31G over 20M calls), the remaining `FxHashSet<&str>` users (`HashMap<&str, ()>::insert`, 0.28G).
- What is left of `some_property_reduces_to_never` after item 1 is hashing and probing (0.65G for 9.2M names); for
  two constituents only the names of both matter, but a lookup structure for the other list costs about as much.
