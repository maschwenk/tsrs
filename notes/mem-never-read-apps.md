# mem-never-read-apps: what each checker creates, keeps and never reads on the app projects

Question (2026-10-07): notes/mem-use-census.md found that about 13% of what a checker allocates is reachable and never
read, on the 38k-file codebase, vscode and mui-docs. On t3code-server, cal-diy and formbricks-web every extra checker
rebuilds a different kind of graph (Effect generics with project arguments; zod and @types/react instantiations,
notes/mem-per-checker-duplication.md section 3). Which never-read pools do those projects have, what forces them in
every checker, and can any of them be deferred exactly for 5% of the peak at the default checker count (32 on the
64-vCPU runner)?

Answer: no. Per extra checker (1 -> 4 checkers) 11-14% of what a checker allocates is reachable and never read, the
same share as on the other corpora, and almost none of it is types (2-5% of the types a checker creates are never read).
It is instantiated symbols, their link records, instantiated signatures and name lists, spread over many pools. One pool
is above 2% of the per-checker growth, on t3code only: lazy member tables that instantiate call and construct signatures
only to let `isWeakType` see that there are some. Deferring them exactly (D1 below) gives -1.1% of the peak at 32
checkers on Linux (-1.4% at 16, -1.6% at 4), no instruction change outside the noise, nothing on the other projects.
Below the bar: the code was removed again and is kept in 8be2e48 for reproduction.

Correction to the brief: U1 and U2 (notes/mem-use-census.md) never landed. PR #7 was closed with the census tool, and
`TSRS_LAZY_DISCRIMINANTS` / `TSRS_LAZY_BASES` are not in main. Their pools on these projects are 0.1-0.4% of the
per-checker growth (pools P5 and P9 below), so landing them would not change this note's conclusion.

Base: origin/main 4f43c94. Mac: Apple M5 Max, 18 cores, 16 KiB pages, 1-minute load 10-20 during the runs. Linux:
Depot `depot-ubuntu-24.04-64`, THP `madvise`. MB are 2^20 bytes.

## 1. Method: the use census on current main

The tool of notes/mem-use-census.md (`TSRS_CENSUS=1 TSRS_USE_CENSUS=1`, alloc-profile build) was ported to main on
branch `mem/use-census-apps` (435addd, 74190f7, 0c6e7cb; analysis script `tools/perf/usecensus.py` in b8024ab). Main
has compressed handles now, and the reachability census decodes 48-bit words, so the census build is
`--features alloc-profile,tsrs_core/plain-ptrs`: handles are 8 bytes there and every byte figure below is larger than
in a release build (roughly 1.2-1.4x). Read the shares. What had to change besides the merge:

- New cells of main record reads: `ThinSlice::get` (the elements' block), `ThinSliceCell`, `OptionThinSliceCell`,
  `OwnedTaggedStrCell` (symbol names; its tag bits are mode checks and do not count), `OwnedPSliceCell`, `SlicePair`
  and `StaticSlicePtr` (mapper lists). Setters use `peek` (the symbol tail, the signature rare tail, symbol-table key
  comparison).
- Link-store chunks are `[PSlot<V>]` arrays now; the census splits them into records again.
- Two false uses found with the first-reader table and fixed: the reference instantiation table (A7 of mem-round3)
  reads every new reference's type arguments to hash it, which made 100% of `TypeReference`s look read at creation;
  it now hashes with `peek` (a cache key, as Go's `getTypeListKey`). Lazy member tables keep their state in a
  `OnceCell` and plain words, which the census cannot see, so 65% of them looked never read; they are now marked when
  `get_ready_lazy_member_table` hands one to a caller.
- A new table, the first reader of each block by type. For every large class the first reader is checker work:
  references by lazy-table preparation (`get_type_arguments`, `get_type_with_this_argument`), mappers by `map`,
  signatures by `is_top_signature`, `infer_from_signatures` and `is_signature_applicable`, symbols by
  `get_declaration_modifier_flags_from_symbol` in `properties_related_to` and member lookups.

Runs: `-p <project> --noEmit --incremental false --extendedDiagnostics --checkers {1,4}`,
`TSRS_USE_CENSUS_FRAMES=9` (and 24 for the outermost asker), `TSRS_USE_CENSUS_FIRST_READS=1`, `TSRS_CENSUS_TSV`.
5-10 GB and 20-40 s per run.

## 2. Totals (checker phase: arena blocks and link records allocated after the first checker, freed blocks left out)

| project | checkers | allocated | not read | reachable at exit, never read |
| --- | ---: | ---: | ---: | ---: |
| t3code-server | 1 | 425.4 MB | 50.6 MB (11.9%) | 42.0 MB (9.9%) |
| t3code-server | 4 | 807.4 | 108.3 (13.4%) | 93.9 (11.6%) |
| cal-diy | 1 | 308.2 | 41.3 (13.4%) | 35.8 (11.6%) |
| cal-diy | 4 | 555.5 | 72.7 (13.1%) | 63.4 (11.4%) |
| formbricks-web | 1 | 383.7 | 58.4 (15.2%) | 50.5 (13.2%) |
| formbricks-web | 4 | 605.4 | 89.7 (14.8%) | 78.0 (12.9%) |

Per extra checker (the 4-checker run minus the 1-checker run, over 3): t3code 127.3 MB allocated, 17.3 MB of it never
read (13.6%); cal-diy 82.4 / 9.2 (11.1%); formbricks 73.9 / 9.2 (12.4%).

By object group, per extra checker (never read / allocated, MB):

| group | t3code-server | cal-diy | formbricks-web |
| --- | ---: | ---: | ---: |
| types (`TypeAlloc<*>`) | 0.76 / 26.5 (2.8%) | 0.35 / 20.6 (1.7%) | 1.15 / 20.9 (5.5%) |
| symbols, their link records and tails | 7.60 / 31.4 (24.2%) | 4.70 / 23.9 (19.7%) | 4.58 / 18.2 (25.2%) |
| signatures, parameter and member lists | 3.99 / 14.1 (28.3%) | 0.66 / 10.6 (6.2%) | 0.88 / 7.6 (11.6%) |
| type lists, mappers, inference records | 1.45 / 27.2 (5.3%) | 1.48 / 12.4 (12.0%) | 1.14 / 11.7 (9.7%) |
| rest (name lists, symbol tables, aliases, ...) | 3.49 / 28.1 (12.4%) | 1.99 / 14.9 (13.4%) | 1.44 / 15.5 (9.3%) |

So the duplicated types of mem-per-checker-duplication.md section 3 (Effect generics with project arguments,
zod and @types/react instantiations) are read in every checker that creates them. A checker that rebuilds them does
real work with them; laziness cannot remove them, only sharing could (out of scope).

## 3. Ranked pools (never read and reachable; MB per extra checker, 1 -> 4; census bytes)

Grouped by the code that forced creation (sampled stacks, 9 frames; `usecensus.py pools`). "Share" is of the
per-extra-checker allocation above.

| # | pool | t3code | cal-diy | formbricks | what forces it | deferrable exactly? | Go |
| --- | --- | ---: | ---: | ---: | --- | --- | --- |
| P1 | call/construct signatures instantiated by lazy-table preparation (signatures, parameter symbols, parameter lists; link records not counted) | 4.77 (3.7%) | 0.10 | 0.00 | `isWeakType` -> `getSignaturesOfType` on a reference: preparing its table instantiates every declared signature, `isWeakType` only tests for emptiness, and a same-target relation then goes through variances | **yes: D1** (section 5) | `resolveObjectTypeMembers` instantiates them eagerly too |
| P3 | `unaffected` name lists of lazy tables (sorted `&str`, 16 bytes each) | 2.10 (1.7%) | 0.11 | 0.14 | `prepare_lazy_members` snapshots `isSymbolUnaffectedByInstantiation` of every declared member; read only when a member is looked up | no: the snapshot is what makes lazy members exact. A bitset over declared-member positions would hold the same facts in a word for <= 63 members (layout, not laziness): ~2.4 MB per extra checker on t3code, about 0.5% of the 32-checker peak by D1's ratio of census bytes to measured peak; mem-lazy rejected it at 18 MB on the 38k-file codebase | no such list (tsrs/#64475 only) |
| P7 | mapped type members | 0.97 | 0.78 | 0.30 | `resolveMappedTypeMembers` for `every` / `getPropertiesOfType` | partly: L4 (rejected in mem-lazy) | same |
| P8 | members and signatures of instantiated anonymous types | 0.72 | 0.53 | 0.72 | `resolveAnonymousTypeMembers` for any query | L3 (rejected in mem-lazy) | same |
| P6 | union/intersection properties (`getReducedType`, contextual types) | 0.42 | 0.69 | 0.95 (1.3%) | `createUnionOrIntersectionProperty` computes the type of <= 2-constituent properties eagerly | no: B1 (rejected in mem-round3, changes results) | same |
| P4 | members of a base type resolved in full for a derived `resolveObjectTypeMembers` | 0.63 | 0.31 | 0.35 | `getPropertiesOfType(base)` when a reference is resolved in full before any member lookup (the eager twin of U2) | only by creating a lazy table for the base, which changes which tables exist; 0.4-0.5% | same |
| P2 | both sides of an identity relation resolved in full | 0.40 | 0.31 | 0.39 | `propertiesIdenticalTo` calls `getPropertiesOfObjectType` on both and returns on a count mismatch | the count could come from lazy property orders (L10); 0.3-0.5%, mostly in the first checker (formbricks: 8.4 of 16.4 MB never read at 1 checker) | same |
| P9 | discriminant lookups in inference (U1) | 0.20 | 0.07 | 0.26 | `getUnmatchedProperties(..., matchDiscriminants)` | yes (U1, not landed) | same |
| P5 | bases resolved in full inside `resolveLazyMembers` (U2) | 0.14 | 0.11 | 0.17 | `getPropertiesOfType(base)` for each base | yes (U2, not landed) | n/a |
| | rest (hundreds of stacks below 0.1 MB each) | 2.99 | 2.50 | 3.15 | | | |

Only P1 is above 2% of the per-checker growth, and only on t3code.

## 4. What asks for the per-checker work

All objects (read or not) a checker adds, per extra checker, MB allocated (never read in parentheses), by the
nearest recognisable asker on the allocation stack (9 frames) and by the outermost one (24 frames). The categories
are name matches (`usecensus.py askers`): inference = `inferFromTypes`/`inferTypes`/`getInferredType`; relation =
`structuredTypeRelatedTo`, `isRelatedTo`, `getReducedType`/`getNormalizedType`, `isWeakType`, ...; signature
resolution = `resolveCall`/`chooseOverload`/`isSignatureApplicable`; printing = `typeToString` and the node builder;
getPropertiesOfType = full member walks.

| asker (nearest) | t3code-server | cal-diy | formbricks-web |
| --- | ---: | ---: | ---: |
| relation | 23.4 (7.2) | 11.4 (0.9) | 12.5 (1.4) |
| conditional / mapped type instantiation | 19.6 (1.3) | 19.5 (1.8) | 15.4 (1.0) |
| getPropertiesOfType walks | 15.3 (2.0) | 14.9 (1.7) | 10.3 (2.0) |
| property lookup | 12.7 (0.5) | 5.7 (0.3) | 5.7 (0.5) |
| contextual typing | 8.4 (0.3) | 1.2 (0.0) | 1.6 (0.1) |
| declarations, annotations | 6.7 (0.3) | 5.5 (0.3) | 5.7 (0.2) |
| inference | 6.3 (0.4) | 2.0 (0.2) | 2.8 (0.1) |
| signature resolution | 5.3 (0.4) | 0.8 (0.0) | 0.7 (0.1) |
| printing | 0.0 | 0.1 | 0.3 (0.1) |
| other | 13.4 (0.9) | 7.1 (0.4) | 7.7 (0.9) |

| asker (outermost of 24 frames) | t3code-server | cal-diy | formbricks-web |
| --- | ---: | ---: | ---: |
| signature resolution | 29.7 (1.7) | 9.2 (1.3) | 7.7 (0.8) |
| relation | 15.7 (1.1) | 29.2 (2.4) | 25.2 (2.7) |
| conditional / mapped | 22.7 (6.8) | 11.1 (0.8) | 12.0 (1.0) |
| property lookup | 21.3 (1.5) | 5.9 (0.3) | 2.1 (0.2) |
| inference | 8.6 (0.9) | 5.4 (0.6) | 5.8 (0.7) |
| declarations, annotations | 5.5 (0.5) | 6.6 (0.5) | 5.6 (0.8) |
| contextual typing | 5.4 (0.3) | 3.4 (0.2) | 1.1 (0.0) |
| printing | 0.0 | 0.1 | 0.2 (0.1) |

Types alone (the duplicated classes) come mostly from conditional and mapped type instantiation (7.0-7.5 MB per
extra checker on all three, nearest asker) and from relation checks (2.1-3.6). Read against the declaring-package
split of mem-per-checker-duplication.md: on the zod/React projects the library instantiations are asked for by
assignability checks (relation is the outermost asker of ~40% of the per-checker growth), on t3code by call
resolution and property access through Effect's pipelines (signature resolution and property lookup are the
outermost askers of 46%). Inference allocates a lot but keeps little: its contexts and candidate lists go back to the
free lists, and what survives is what the inferred types instantiate. Printing for diagnostics is negligible. This
split by asker covers all objects; the census cannot class an object as library-only or project-involved (that was
the uncommitted instrumentation of mem-per-checker-duplication.md).

## 5. D1: lazy tables instantiate their call and construct signatures when first read (`TSRS_LAZY_SIGNATURES`, removed)

On t3code the pool is Effect 4's `Rpc<...>` (57,406 of the deferred tables in one checker) and Schema's
`Bottom`/`BottomLazy` (20,515): interfaces with a non-generic construct signature (`new (_: never): ...`), compared
with `isWeakType` before the relation goes through the variances of their type arguments, which never reads the
signatures.

Design (8be2e48, ~180 lines): `prepare_lazy_members` does not instantiate the target's declared signatures when none
has type parameters and they have at most 31 this-parameters and parameters together. It records, for each of them in
instantiation order, whether `isSymbolUnaffectedByInstantiation` holds at that moment (31 bits in the padding after the
table's 4-byte mapper handle, plus a pending bit; the table stays 80 bytes). Inherited signatures are not read from a
base whose own signatures are pending. `isWeakType` and the L11 empty-object test count signatures (declared count plus
the bases' counts; instantiation maps signatures one to one). Every other reader (`getSignaturesOfType` of the table,
`resolveLazyMembers`) instantiates them: each declared signature as `instantiateSignature` would have, with the
snapshot answering `isSymbolUnaffectedByInstantiation`, then each base's `getSignaturesOfType` in base order.

Exactness, clause by clause against `resolveObjectTypeMembers` / `instantiateSignatureEx` /
`newInstantiatedSymbol` (checker.go) and #64475's preparation:

1. Same objects when read. A deferred signature is built from the same declaration, flags, minimum argument count,
   target and mapper; a parameter becomes a new instantiated symbol exactly when eager instantiation would have made
   one, because the snapshot is taken where eager instantiation ran and reads the same link state.
   `newInstantiatedSymbol` copies only binder-time facts of the parameter (flags, name, declarations, parent, value
   declaration, check flags; parameters of declared signatures are not instantiated symbols and have no name type),
   so building it later gives the same symbol. Return types and predicates stay lazy in both. Signature lists are the
   declared ones followed by each base's, in the order `appendInheritedSignaturesAndIndexInfos` used.
2. No type moves. Instantiating a signature without type parameters creates no type (only a signature, symbols and
   links); a table with a generic declared signature, whose instantiation clones type parameters, stays eager. Index
   infos and base types are still instantiated at preparation, in the same order. So type ids, union order and
   `isDeeplyNestedType` see the same sequence.
3. Side effects of base reads stay where they were. For a base that is not pending, `getSignaturesOfType(base)` is
   still called at preparation (resolving an intersection base creates types) and again when the list is built,
   which then only reads resolved members and the cached reduced apparent type.
4. Symbol ids move, as they did for L10 and U1: the binder parameters still get their link records (and ids) at
   preparation, from the snapshot, but the new parameter symbols get theirs later or never, which shifts the ids of
   symbols created after them in that checker. Ids reach output only through `__@name@id` property names; output is
   already independent of the checker assignment, which shifts ids far more. Signature ids (`signature_count`) move
   too; the checker never reads them (Go's `Signature.Id()` is for the API).
5. `TSRS_LAZY_MEMBERS=0` has no lazy tables, so nothing changes there; `--extendedDiagnostics` `Symbols` drops.

Go: the same change applies to #64475's `prepareLazyMembers` (the pinned reference has no lazy tables), plus
`isWeakType` asking for counts. Upstream it saves allocations and GC work rather than retained memory.

Counters (single-threaded): t3code 80,018 of 323,915 lazy tables deferred, 22,929 of them instantiated later (29%),
245,430 counts answered; symbols 2,463,541 -> 2,406,455 (-2.3%), types unchanged. cal-diy 2,034 deferred (symbols
-765), formbricks 931 (-51), vscode 1,642 (-75).

Linux (64 vCPU, one binary built from 8be2e48, `TSRS_LAZY_SIGNATURES=0` vs `=1`: `=0` is main's code path and memory
layout; 6 interleaved reps, medians; Depot run 4c05pdrxxk):

| project | checkers | peak GiB main -> D1 | wall s main -> D1 | check s main -> D1 |
| --- | ---: | --- | --- | --- |
| t3code-server | 4 | 1.251 -> 1.231 (-1.6%) | 1.990 -> 1.967 (-1.1%) | 1.898 -> 1.882 |
| t3code-server | 16 | 2.197 -> 2.166 (-1.4%) | 1.635 -> 1.655 (+1.2%) | 1.537 -> 1.547 |
| t3code-server | 32 | 2.775 -> 2.745 (-1.1%) | 1.527 -> 1.564 (+2.5%; ranges 1.421-1.564, 1.419-1.656) | 1.429 -> 1.460 |
| cal-diy | 4 / 16 / 32 | +0.4% / +0.8% / -0.3% | +0.2% / +0.4% / +0.5% | |
| formbricks-web | 4 / 16 / 32 | +0.1% / +0.2% / +0.1% | +0.3% / +1.7% / -0.5% | |
| vscode | 4 / 16 / 32 | +0.2% / +0.4% / 0.0% | +1.2% / -0.0% / +0.2% | |

Mac (medians of 3 interleaved runs): t3code single-threaded peak 767.6 -> 759.8 MiB (-1.0%), instructions 47.76 ->
47.95 G (single runs range 47.75-48.37 G for main alone: no measurable change); 16 checkers 2,220 -> 2,165 MiB
(-2.5%); 32 checkers 2,734 -> 2,705 MiB (-1.1%; ranges 2,725-2,743 and 2,679-2,724).

Gates run on 8be2e48: suite trees (`--baselines types,symbols`) identical to main's in the default mode and with
`TSRS_LAZY_MEMBERS=0` (13,458 error baselines pass, 2 codes, 2 fail; 12,779 types; 12,779 symbols); diagnostics
byte-identical to main on t3code-server, cal-diy, formbricks-web and vscode at 1, 4, 16 and 32 checkers (Mac), and the
same error counts in every Linux run. Not run, since it does not land: fourslash, `tools/regressions.sh`, the lint
ratchet.

Verdict: exact, but -1.1% of the peak at the default count on one project and 0 elsewhere; the bar is 5%. Removed.

## 6. Why nothing here can reach the bar

The whole never-read reachable share is 11-14% of the per-checker growth, and the growth is 50-70% of the 32-checker
peak on these projects (t3code: (2.79 - 0.87) / 2.79 GiB = 69%; formbricks 53%). Removing every never-read object
exactly would be worth 6-9% of the peak; the largest exact pool is about a tenth of that and the next ones (P3, P4, P2, U1, U2) are 0.3-2% of the
growth each, each a separate mechanism with its own exactness argument. Most of the rest is instantiated member
symbols of full resolutions (anonymous, mapped, references resolved in full) whose deferral was measured and rejected
in mem-lazy (L3, L4) or changes results (B1). The per-checker memory on the app projects is the checker's own type
graph, and it is read; reducing it means sharing it (notes/mem-shared-base.md, out of scope) or running fewer checkers
(notes/mem-per-checker-duplication.md section 5).

## Reproduce

```sh
git worktree add ../census origin/mem/use-census-apps && cd ../census
CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile,tsrs_core/plain-ptrs
cd <bench-cache>/solutions/t3code-server
TSRS_CENSUS=1 TSRS_USE_CENSUS=1 TSRS_USE_CENSUS_FRAMES=9 TSRS_USE_CENSUS_FIRST_READS=1 TSRS_CENSUS_TSV=t3-c1.tsv \
  <census>/target/prof/release/tsrs -p apps/server --noEmit --incremental false --extendedDiagnostics --checkers 1 2> t3-c1.err
# the same with --checkers 4 into t3-c4.tsv / t3-c4.err, then
python3 <census>/tools/perf/usecensus.py pools t3 4
python3 <census>/tools/perf/usecensus.py askers t3-c1.tsv t3-c4.tsv 4
# D1: build 8be2e48; TSRS_LAZY_SIGNATURES=0 restores main's behaviour
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref <branch with 8be2e48> \
  --input projects=t3code-server,cal-diy,formbricks-web,vscode --input script=tools/perf/leafprobe.sh \
  --input probe_args='--checkers 4,16,32 --reps 6 --no-strace --variant main:TSRS_LAZY_SIGNATURES=0 --variant d1:TSRS_LAZY_SIGNATURES=1'
```
