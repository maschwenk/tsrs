# Instruction-count hill climb: the type instantiation path

Goal: fewer user-space instructions for a cold `--noEmit` check of a large private monorepo at `--checkers 8`, in the
type instantiation path (`instantiateTypeWithAlias`, conditional-type instantiation, the mappers, the instantiation
caches and their keys, lazy member tables of references). One measured change at a time on top of 98d8d60e, kept only
if the count went down and the output stayed identical. Ten commits; method and rules as in
notes/perf-icount-hillclimb.md.

## Result

98d8d60e against this branch, `-p <tsconfig> --noEmit --incremental false --pretty false`, same machine, diagnostics
identical (the workload prints none; the identity projects below are the output check).

| | 98d8d60e | branch | change |
|---|---|---|---|
| user-space instructions, Linux VM (DynamoRIO), 8 checkers, locality | 322.677 G | 312.576 G | -3.13% |
| user-space instructions, Linux VM, single-threaded | 192.423 G | 187.569 G | -2.52% |
| user-space instructions, Linux VM, 8 checkers default (stealing; 4 / 3 runs, medians) | 338.03 G | 325.19 G | -3.8% |
| macOS `time -l` (user + kernel), 8 checkers default, median of 3 | 362.40 G | 353.90 G | -2.3% |
| macOS, 8 checkers locality (2 runs) | 348.90-348.96 G | 338.59-338.99 G | -2.9% |
| macOS, single-threaded | 213.87 G | 209.58 G | -2.0% |
| peak RSS, 8 checkers default | 6.39-6.47 GB | 6.36-6.42 GB | about -0.5% |

`--extendedDiagnostics` at 8 checkers with the locality assignment: Types and Instantiations identical; the Symbols
count and the lazy-table query counts differ between two runs of the same binary as much as between the binaries.

## How it was measured

- Decisions: the Linux VM's DynamoRIO count (/tmp/icount/linux), user space only, `--checkerAssignment locality`
  (0.02% spread) and single-threaded (1e-7). The macOS counts include kernel instructions and moved by up to 1.7%
  between two runs of one binary while other agents shared the machine, so they only confirm the larger changes.
- Where the instructions are: a second DynamoRIO client (per-basic-block execution counts; not committed) plus
  `llvm-symbolizer` on a `line-tables-only` build. Because AArch64 instructions are four bytes, a block's last
  instruction is at `start + 4 * (n - 1)`; decoding it gives exact call counts per call site, and block starts at
  function symbols give exact entry counts. That replaced the sampling profiles for every decision below (sampled CPU
  time weights cache misses, which the count does not see).
- Single-threaded profile of the base (192.4 G): `instantiateTypeWithAlias` was entered 51.2M times (44.6M
  instantiations; the active-mapper cache answered 6.6M of 31.3M lookups and stored 24.7M results), `TypeMapper::map`
  69M calls, `hash_128` 18.7M calls (12.3M from the instantiation and conditional keys), `Type::target` 71M calls (not
  inlined), `isSymbolUnaffectedByInstantiation` 18.3M calls (15.1M from lazy member tables).

## What landed (in commit order; 8-checker locality / single-threaded, against the commit before)

| commit | locality | single-threaded |
|---|---|---|
| pack short instantiation and conditional cache keys | -1.04% | -0.72% |
| lazy member tables reuse the settled unaffected-member answers of their target | -0.96% | -0.88% |
| `TypeMapper::map`: kinds that call back out of line | -0.14% | -0.09% |
| active mapper caches stay in place (no free list) | -0.20% | -0.20% |
| `object_target()` where the type is known to be an object | -0.31% | -0.21% |
| pack short union, intersection and indexed-access keys | -0.26% | -0.24% |
| store an active mapper's result without a second search | -0.08% | -0.08% |
| pack short aliased instantiation keys | -0.07% | -0.06% |
| `TypeMapper::map` without a stack frame for simple and array mappers | -0.11% | -0.07% |

- Packed keys. Go keys these caches with xxh3-128 over a byte encoding and only tests keys for equality. A list of at
  most three type ids (plus a flag, or two ids and the access flags, or arguments + alias symbol + alias arguments of
  at most three ids) is now encoded injectively into the 128 bits: family tag, small metadata and the third id in the
  high word, a bijective mix of the first two ids and the high word in the low word (which `PackedMap` uses as the
  hash). Packed keys are equal exactly when their inputs are; a packed key matches an xxh3 key of a longer input with
  the same 2^-128 odds as two xxh3 keys. Each family has its own tag, so the never-hit `getTypeListKey(typeParameters)`
  entry that Go puts in a type alias's instantiation map stays unreachable. This was the largest win at 8 checkers
  (conditional instantiations grow 2.2x with the checkers).
- Lazy member tables. `prepare_lazy_members` asked about every named declared member of the target for every table
  (2M tables, 28M members at 8 checkers). The answer settles once the member's type is resolved; each target keeps
  the names answered true for good (one sorted arena list shared by its tables) and the members still unresolved.
  `TSRS_LAZY_UNAFFECTED=shadow` recomputes every list and panics on a difference; it ran clean on the private
  monorepo (8 checkers and single-threaded, macOS; 8 checkers locality, Linux VM) and on the conformance suite in the
  default and the canonical history.
- The rest are code-shape changes on the hottest calls: `map` saving fewer registers (then none for the common kinds),
  the active-mapper caches kept in place by depth, a direct object target read where `Type::target`'s flag dispatch
  was an out-of-line call, and a store into an active mapper's cache without searching again when the cache is
  unchanged since the lookup (its length and a count of `clear_active_mapper_caches` calls).

Invariants the reuse relies on: links set a value symbol's resolved type and write type once (`get_type_of_...`
store only when unset), `could_contain_type_variables` of a type never changes, a target's declared-member table is
set once (the memo also checks the table and its length), and `maps_this_only` of a lazy table's mapper depends only
on the target's type parameters (checked anyway).

## Measured and rejected

- A pooled list instead of a fresh `Vec` for the conditional instantiation's type arguments: -0.01% single-threaded,
  0.00% at 8 checkers (mimalloc's fast path costs about what the pool does).
- The active-mapper cache as a linear list of up to 8 entries before a hash table: +0.18% at 8 checkers. Most
  lookups miss, and a miss scans the whole list, then the store scans it again.
- Keeping cleared active-mapper tables up to 2^16 slots instead of dropping those over 64 slots that are mostly empty:
  +0.42%. Dropping from 16 slots: +0.04%. Clearing a large table on every pop costs more than regrowing it.
- `is_tuple_type` through `object_target()`: neutral (+0.004% / 0.000%), and it made `is_generic_tuple_type` an
  out-of-line call at 49M sites; left out of the `object_target()` commit.

## Found, not pursued

- At 8 checkers the remaining growth in this territory is the number of instantiations itself
  (`instantiate_type_with_alias_worker` 8.7 G single-threaded, about 17 G at 8 checkers): the same library generics
  instantiated by every checker. Only sharing across checkers would remove it (rejected for the default build in
  notes/perf-round2-followups.md).
- `get_type_list_key` (signature instantiations, 0.7M calls) and template-literal keys still hash; too few calls.
- `ReferenceInstantiations` growth (0.2 G single-threaded in rehashing) was measured before
  (notes/perf-checker-cpu3.md, notes/perf-memory-traffic-32.md); not redone.
- The packed-key commits touch union and intersection keys, which other work in the same round may also change.
