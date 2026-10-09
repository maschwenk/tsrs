# design-shared-type-layer: an immutable type layer shared by the checker threads, tsgo-identical and wall-neutral (design, no code, 2026-10-08)

The owner asked for a design, without code, for an immutable type layer that every checker thread reads, under two hard
constraints that the earlier prototypes relaxed:

1. output byte-identical to tsgo's for the same checker assignment (diagnostics, union constituent order, recursion
   cutoffs, circularity reports, and the `--checkerAssignment go` mode);
2. no wall cost at the scoreboard's default (8 checkers on Depot's 16-vCPU runner) or single-threaded.

The note decides whether such a design exists, what it would cost, and which experiment would falsify its speed
assumption. Nothing was built or run for it. Every number comes from the notes, branches and code cited. Estimates
are labelled as such.

**Verdict: no design meets both constraints.**

- **Exactness is reachable, at main's level and no higher.** The layer is off in Go mode, so Go-mode output is
  exact by construction. In the default mode every fork is a sequential continuation of one pool checker's prefix, so
  each checker's history is one that main could also have had (section 2). Its output then equals main's wherever
  main's own output does not depend on the assignment. No design can be stronger, because type ids and
  first-filled fields reach the output in two places.
- **Speed is the obstacle.** The spike measured two costs, and both stand:
  - The read paths alone, compiled in and switched off, cost +2.3..+4.8% wall at 8 checkers and +1.5..+4.3%
    single-threaded instructions (spike sections 10.1 and 4.2). The read path proposed here makes one comparison
    against a constant and loads nothing. It has never been measured; section 5.3 is the 2-3-day experiment that
    decides it.
  - The seed costs wall time even with that floor subtracted: the best strategy (FA) measured +7.8% on cal-diy and
    +11.6% on t3code-server above the empty-seed floor (section 10.1). Folding the seed into a pool checker
    (section 3.1) removes an estimated 21-46 ms of that: 1-2 points on t3code-server, 2-4.5 on cal-diy. The budget
    is +2%, or 21-49 ms.
- **What memory is left under both constraints at the default: none that can be shown.** If the read path cost
  nothing, the layer would keep -5.7..-14.4% peak at 8 checkers on formbricks-web, supabase-studio, mikro-orm and
  vscode. It would cost cal-diy and t3code-server 3-11% wall. No rule can pick the projects before the check starts.

## 1. What was tried before, and what is different

| note | design | result | why it was declined |
| --- | --- | --- | --- |
| mem-shared-base.md section 2b (2026-10-02) | share lib + node_modules types | 351 MB of 971 MB duplicates (36%) are library-only at 4 checkers on the 38k-file codebase, ceiling 4.4% of peak | judged not exact: lazy fields filled by whoever asks first, per-checker ids in two output paths, resolution state, not closed under use |
| perf-checker-processes.md | `fork()` N processes from one warm checker | byte-identical; -17..-47% instructions, -0.2..-1.9 GiB; wall at parity on the Mac, +4.5..+8.6% on Linux | the warm-up is serial: building the shared work S costs what it costs |
| perf-shared-checker.md sections 3-4 | a read-mostly subsystem | library-only work is 3.6% of S (6.5% with library-only instantiations) on the 38k-file codebase; 297 mutable fields, 860 writes | below the bar; the full shared checker is a rewrite |
| spike-shared-graph.md sections 1-9 | frozen seed + forks | diagnostics byte-identical in every run; Linux, 8 checkers: peak -2.1..-9.9%, wall +7.2..+12.8% (section 9) | the seed is built serially |
| spike-shared-graph.md section 10 (branch `spike/shared-graph-seed`, PR #223) | hiding the seed | best (FA): peak -5.7..-14.4%, wall +6.0..+16.2%; floors: `foff` +2.3..+4.8%, `on0` +4.3..+8.0%; throwaways 24-49% productive | both floors |

What is different this time: the two later rounds optimised memory and allowed inexactness (the spike's brief said
"don't worry about exactness") or a wall cost. This design makes exactness and speed-neutrality the constraints and
asks what memory is left. It adds four things the spike did not have:

1. Go mode turns the layer off. The spike never consulted `go_compatible_history()`, so with the switch on it would
   have changed the histories that Go mode must reproduce.
2. A read path whose cost on a private object is one comparison against a constant, with no load. The spike's tests
   loaded two globals.
3. The seed is folded into a pool checker, so no ninth thread runs. This combines the spike's strategies B and F;
   neither was built in that form.
4. A stated exactness contract, measured against tsgo's checker pool (section 2).

## 2. What "identical to tsgo's output" can mean

tsgo numbers types per checker: `c.TypeCount++; t.id = TypeId(c.TypeCount)` (checker.go:25473-25485). tsrs does the
same at checker_12.rs:1904-1916. tsgo's `createCheckers` (compiler/checkerpool.go:367-422) assigns every file to a
checker by FENNEL over the import graph, for the given N. `forEachCheckerGroupDo` (476-494) has every checker walk
the files in program order and check its own. So tsgo's output is the union of N sequential histories that
(program, N) fixes. That output itself changes with N: on sequelize tsgo prints 42 / 41 / 41 / 42 errors at
1 / 4 / 8 / 16 checkers (open-history-dependence.md).

tsrs keeps three levels of identity today:

- **E-go.** `--checkerAssignment go` at N prints tsgo's output at N, bug for bug. Stealing is off
  (checkerpool.rs:838-841), files are assigned by FENNEL (`go_associations`, 1087), and Go's history-dependent caches
  are kept through `compat::go_compatible_history()` (compat.rs:20-31).
- **E-default.** The default mode prints the same bytes at every checker count and assignment, equal to the
  single-threaded output (perf-order-independence.md). `tools/ci/determinism.sh` gates it with 504 runs. Three
  programs are exceptions: TanStack/router, sequelize and rxjs, where main's own output depends on the assignment
  (open-history-dependence.md).
- **E-tsgo-1.** The single-threaded default output equals tsgo's at one checker, except for the documented canonical
  differences: 1 conformance test, and 8 of 2,460 declaration files in the monorepo emit oracle.

Internal state was never part of the contract. Since #217, tsrs creates 0.52M types instead of 5.1M on type-fest at
one checker, in Go mode too (perf-shared-tuple-elements.md). That change was accepted on an audit build and
differential runs. The `--extendedDiagnostics` counters already vary from run to run with stealing.

**The contract for this design:** E-go and E-tsgo-1 exactly as main, and E-default byte-identical to main wherever
main's own output does not depend on the assignment.

**Why nothing stronger exists.** A shared object is created outside each reader's id sequence, and its lazy fields
are filled during one history. Ids reach output in two places:

- the last line of `compare_types` (utilities.rs:768-769; 41 call sites), which orders union constituents;
- the increasing-id rule of `is_deeply_nested_type` (relater_1.rs:1083-1091). It is called at depth 2 in inference
  (inference.rs:363-366, 1208-1214) and at depths 3 and 10 in the relater (relater_2.rs:820-826, 1308, 1516).

"Whoever asks first fills the field" reaches output through the resolution stack (checker_09.rs:2023-2061) and
through every cache whose answer depends on when it was computed: base constraints, variances, the relation cache. A
layer that showed every reader exactly the objects and ids its own history would create would have to replay that
checker's creation sequence, which means running that checker.

So in the default mode every shared design rests on an argument main already relies on. Stealing gives each checker
a timing-dependent history today, and output is the same for every history. That is tested, not proven. This
design's job is to give every checker a history that main could also have had.

## 3. The design

### 3.1 Shape

The base is the spike's frozen seed plus forks (spike section 2), changed in four places.

1. **Late freeze, no ninth thread.**
   - Pool checker 0 checks the seed sample first: the spike's program-derived 10-permille spread, taken out of the
     other queues.
   - It then freezes its arena region and continues its own queue as a fork of its own frozen state.
   - Checkers 1-7 start their queues at once, as plain checkers inside scratch regions (`Region::new_scratch`,
     arena.rs:925). Diagnostics already escape scratch regions.
   - At the freeze, each of checkers 1-7 finishes its current file and copies out what must survive: its
     diagnostics and its deferred type-argument checks (checker.rs:977-980). It then retires the region and becomes
     a fork.

   The sample files leave the other queues, so the total work is unchanged and stealing rebalances it. A checker that
   is inside a heavy file at the freeze switches when that file ends; nobody waits. This is the spike's F
   (throwaways in freed regions, 6f0dfcd9) plus B (the seed checker keeps going: predicted to save 11-16 ms, never
   built). It drops the ninth thread, which made the seed 10-30 ms slower (t3code `T_w` went from 246 to 274 ms
   beside 8 busy throwaways).
2. **Ids.** A fork's `type_count` starts at checker 0's count at the freeze (the spike also numbered fork objects
   from `K_t` up). So a fork's ids follow the creation order of one sequential history: checker 0's prefix, then the
   fork's own work. Symbol ids of seed objects are assigned before the freeze, as in spike round 1.
3. **Go mode off.** Under `go_compatible_history()` the pool creates plain checkers. The same rule already applies to
   the union front cache (unioncache.rs:62), the inference memo (infermemo.rs:78), stealing (checkerpool.rs:840),
   leaf freeing (execute.rs:313) and deferred type-argument checks (checker_02.rs:1037).
4. **The read path of section 5.2.**

The layer exists only in the CLI's type-check pass with two or more checkers. One checker, the language server, the
API, `--build` and the test harnesses run main's path. The spike made the same choice: for the language server the
frozen region would have to outlive edits, which is weeks of work.

### 3.2 What is shared and what is not

| part | shared (frozen) | per fork |
| --- | --- | --- |
| types, signatures, checker symbols, index infos, mappers, link records and interning-map entries the seed created | the objects (`K_t` 9-345 K types, `B_W` 2.4-35.7 MiB at 10 permille: spike section 4.4) | objects the fork creates, with ids above `K_t` |
| intrinsic and global types (`any_type` ... `global_array_type`, checker.rs:1118-1202) | the seed's, copied as handles into each fork | nothing |
| lazy fields of frozen objects that a fork can write (the spike found 47) | the seed's value where set | a side table entry, reached when the inline value is unset (section 3.4) |
| lazily computed object-flag families (types.rs:966-1000) | those that read no lazy field (`CouldContainTypeVariables`, for example) computed for every frozen type at the freeze | the others (`MembersResolved`, `IdenticalBaseType*`, `IsUnknownLikeUnion` and `IsNeverIntersection`, which resolve members, `IsConstrainedTypeVariable`, which reads constraints) in the side table, like the 47 fields; which families qualify is part of the classification |
| tables hung off frozen objects (`ReferenceInstantiations` types.rs:2167, `ConditionalRoot.instantiations` 2740, `UnionRare.constituent_map` 2423, property caches 2411-2412) | the seed's table, read-only, `FrozenCell` instead of `RefCell` | entries in the fork's own side tables |
| 27 link stores (checker.rs:1055-1081) | the seed's records | a record copied on first `get` (spike `LinkCopy`) |
| interning maps (union, intersection, tuple, indexed access, template literal, string mapping, literal, ... checker.rs:1001-1050) | the seed's map | the fork's map; the frozen map is probed only for keys whose handles are all frozen |
| relation caches, flow and inference memos, stacks, scratch pools | none | start empty, as in the spike |
| diagnostics | the seed's collection | carried into every fork (spike section 2) |

### 3.3 The fields

`types.rs` has 208 interior-mutable fields (my count over struct bodies). About 80 are in link records, which are
per checker by construction because they live in the link stores. About 130 are on arena objects: types,
signatures, index infos, predicates, aliases, conditional roots and rare tails. The spike's discovery build
`mprotect`ed the frozen chunks and named the field of every fault. It split the arena fields into 47 that a fork
writes on a frozen object and 79 that it never writes (spike section 3). Both lists must be derived again on current
main, because #212 and #217 changed deferred-check and tuple state.

**Deterministic or asker-dependent.** For a frozen seed this split does not decide what is shared. Everything the
seed filled came from one sequential history, so a fork may read it even where another history would have filled it
differently. The split matters only for a build that merges several builders (section 6, options a and c).

Deterministic fields are a function of the declarations:

- declared types, type parameters and this-types (`InterfaceType.all_type_parameters`, `outer_type_parameter_count`,
  `this_type`, types.rs:2221-2223);
- declared members, signatures and index infos (2228-2231);
- tuple element infos (2300-2304) and literal fresh/regular pairs (1923-1924);
- union and intersection constituents (2402) and the type arguments of non-deferred references (2154);
- the object-flag families that read no lazy field.

Asker-dependent fields depend on who computes them and when:

- base constraints (`ConstrainedType.resolved_base_constraint`, types.rs:1950;
  `Checker::structured_type_base_constraints`, checker.rs:1089), which the depth guard can cut
  (open-history-dependence.md section 1);
- apparent types derived from base constraints (`MappedType.resolved_apparent_type` 2343, `IntersectionRare` 2430);
- results guarded by `push_type_resolution`, which can meet a cycle (`ValueSymbolLinks.resolved_type` 273,
  `Signature.resolved_return_type` 2805, `InterfaceType.resolved_base_types` 2224-2227);
- variances (`VarianceLinks`, 607-609; the sequelize case);
- conditional branches that depend on relations (2750-2754);
- anything reported at `current_node` (TS2589, TS2590).

### 3.4 Side tables, ids and the instantiation tables

A fork keeps four side tables:

1. **Values of the 47 fields on frozen objects, and of the flag bits that follow lazy state.** The spike's overlay
   held 1.9-2.8 K cells per fork at 32 checkers (5.1 K at most), about 0.3 MiB with its 2 KiB pages of object-flag
   words (spike section 6). With the families that read no lazy field computed at the freeze, the remaining flag bits
   fit in cells, leaving about 0.03-0.05 MiB per fork (estimate, 16 B per cell). Computing those families for `K_t`
   types at the freeze is work on checker 0's thread: at 10-50 ns per type, 1-17 ms (estimate), which stealing
   spreads over the other checkers.
2. **Instantiations with any private argument under frozen targets.**
3. **Link records copied on first `get`.**
4. **The fork's own interning maps.**

A per-fork dirty bitmap over the frozen range (one bit per 64 bytes: 30-70 KiB per fork for a 15-36 MiB seed)
replaces the spike's process-wide one. The process-wide bitmap took atomic writes into cache lines that every fork
shared.

Ids stay unique within each fork's view: frozen ids are at most `K_t`, private ids are above it, and all of them
stay below the 2^28 that `RelationKey` packs (relater_types.rs:139-146); t3code-server makes 1.44 M types at one
checker.

`object_type_instantiations` (checker.rs:1091) holds Go's `ObjectType.instantiations` for anonymous, mapped and
deferred targets. Its split:

- the seed's map is frozen;
- the fork's map holds every instantiation the fork creates, including those with all-frozen arguments that the seed
  never made;
- a lookup whose key handles are all frozen tries the frozen map first, then the fork's;
- any other lookup tries only the fork's map, since a private id cannot occur in a frozen key.

`InterfaceType.instantiations` (types.rs:2220), `TypeAliasLinks.instantiations` (532),
`ConditionalRoot.instantiations` (2740) and the lazy member and mapped tables (checker.rs:1084, 1092) split the same
way. The cost is the second probe for all-frozen keys that the seed lacked; section 5.3 measures it.

## 4. Exactness inventory

| # | item | where | how it reaches output | how the design keeps it identical |
| --- | --- | --- | --- | --- |
| 1 | union constituent order | the last line of `compare_types` (utilities.rs:768-769). Ties that main's audit found: fresh/regular literals, object-literal variants, references whose arguments tie that way, symbol-less type parameters (perf-order-independence.md) | printed unions; which property an error names first | a fork's ids follow one history's creation order (section 3.1, item 2), the same class of history that stealing produces. Go mode: off |
| 2 | recursion cutoffs | `is_deeply_nested_type` counts only `t.id >= last_type_id` (relater_1.rs:1083-1091); recursion identities are pointers (1166-1204) | relation and inference results, TS2589-like errors | as 1 |
| 3 | circularity reports | `push_type_resolution` / `find_resolution_cycle_start_index` read the asker's stack (checker_09.rs:2023-2061); `type_resolution_has_property` (2066-2089) reads link records and lazy fields | which declaration gets TS2456/TS7022/TS2502; which reader gets `any` | a fork reads the seed's finished resolutions, as a checker that had checked the seed's files first would; the seed's cycle entries are those of a pool checker's own prefix. Admitted: a sequelize-like program can print another of main's variants |
| 4 | error types from failed resolution | `report_circularity_error` returns `errorType` (checker_09.rs:2092-2119), cached as the symbol's type; unresolved names in `error_types` (checker.rs:1034) | `any` in printed types; errors that go missing downstream | inherited with the seed's caches, as 3 |
| 5 | library symbols whose declaration involves inference or a checked file | global and module augmentations, merged per checker by `initialize_checker`; JS libraries under `maxNodeModuleJsDepth` | members and types of augmented library interfaces | the seed is a pool checker that runs after the global merge, so its values are the full program's. This rules out any build on a partial program, such as a seed built during the parse (spike section 10.5) |
| 6 | diagnostics in declaration files under `skipLibCheck: false` | filed while declarations are resolved; the split check runs pieces of one `.d.ts` on several checkers (checkerpool.rs:903) | errors in `.d.ts` files | the seed's collection is carried into every fork, so the checker that checks a `.d.ts` reports what the seed found while resolving it; deduplicated per file |
| 7 | diagnostics a checker files in files it does not check; deferred type-argument checks | checker.rs:977-980; checker_02.rs:1028-1050 | errors in other files | carried from the seed (spike a5d2a57); under late freeze also copied out of each scratch region before it is retired |
| 8 | budgets (TS2589, TS2859) | instantiation depth 100 and count 5M per expression count calls, so cached work decides whether they fire | whether the error fires | a fork sees the seed's cached instantiations and members. Same class as main's cache dependence: the audit in perf-order-independence.md says it can occur and was not observed |
| 9 | base constraints cut by the depth guard | `get_resolved_base_constraint`; checker.rs:1089, types.rs:1950 | TS2536/TS2339 on TanStack/router | inherited from the seed if the seed cut one. Admitted: on router main already prints 5 or 7 errors depending on the assignment |
| 10 | relation cache and elaboration | per-checker relations (checker.rs:1246-1250) | elaboration text (rxjs) | forks start with empty relation caches, as in the spike. Admitted, same class |
| 11 | tsrs-only memos | union front cache (unioncache.rs:10-24, 210-216: stores only calls that created no type other than the result, judged by `type_count`); inference and flow memos (infermemo.rs:191, flowmemo.rs:460) | none, if their rules hold | their rules are stated per checker in terms of `type_count`, and a fork's count continues the seed's. Re-verify with `TSRS_UNION_CACHE=shadow` |
| 12 | symbol and node ids | assigned process-wide by CAS (tsrs_ast utilities_1.rs:33-107); last resort of `compare_symbols_worker` (utilities.rs:457-459) | order of same-named symbols without declarations | the seed assigns ids before the freeze. Already timing-dependent in main: one hit on vscode, not history-dependent (perf-order-independence.md) |
| 13 | identity relation key | swap by id (checker_09.rs:679) | none (a cache key) | ids stay unique in each fork's view (section 3.4) |
| 14 | `--extendedDiagnostics` counters | per checker | the Types, Symbols and Instantiations lines | they change, as they do with stealing; not part of the contract; unchanged in Go mode |
| 15 | `--checkerAssignment go` | compat.rs:20-31 | all of the above | the layer is off, so Go mode runs main's code path |

Evidence so far for this shape of history: every run of the spike printed main's diagnostics. That covers the Mac
matrix (eight projects at 4, 16 and 32 checkers, two seed sizes), the budget curve, and 1,200 timed Linux runs in
round 2 at 8 and 16 checkers (spike sections 5 and 10). Gaps the spike left open, which this design must close
before landing (section 8): the conformance suite, fourslash and `tools/ci/determinism.sh` never ran with the switch
on, and Go mode was never checked.

## 5. Cost 1: the read path

### 5.1 What the spike's floor paid for

Measured:

- `foff` (the prototype compiled in, switch off, no fork): +2.3..+4.8% wall, paired, at 8 checkers on Linux
  (section 10.1).
- Single-threaded instructions on the Mac (section 4.2):

  | project | main | `foff` |
  | --- | ---: | ---: |
  | t3code-server | 48.21 G | 50.28 G (+4.3%) |
  | formbricks-web | 51.75 G | 53.36 G (+3.1%) |
  | xstate-main | 7.63 G | 7.74 G (+1.5%) |

- `on0` (forks of an empty seed): +4.3..+8.0% wall. After the spike's strategy E1 (cheaper fork map reads), `on0`
  cost t3code-server +4.65% and formbricks-web +3.10% in instructions.

The mechanisms, read in the branch's code (`git log origin/spike/shared-graph-seed`):

1. **`OvCell` on 47 fields.**
   - `get` on an unset value calls `dirty(addr)`, which loads `FROZEN_LO` and `FROZEN_SPAN` (two relaxed loads of
     globals), compares, and in range loads a word of the dirty bitmap.
   - `set` loads `ANY_FROZEN` on every write (tsrs_core/src/sharedgraph.rs on the branch).
2. **Link stores.** Each of the 27 stores got a `parent` pointer, tested on every `try_get` miss and every record
   creation, with copy-on-first-get (links.rs diff).
3. **Two-level interning maps** (`basedmap.rs`). The fork's own map is probed first, then the base on a miss, and
   every map carries a removed-keys set. The spike's E1 (4030d080) added a range test against the same globals.
4. **`object_flags_lazy()`** at about 70 sites: an overlay word, looked up by type id, for the lazily computed flag
   families.
5. **`with_ref()`**: a `frozen()` test on every `RefCell` borrow of an object that might be frozen.
6. **Code size and layout.** After the spike's E1 no symbol was above 0.3% in the Linux profile (section 10.3); the
   cost was spread thin.

### 5.2 The proposed read path

Three ways to tell a shared object from a private one:

- **A bit in the type header.** It costs no space: the 24-byte header (types.rs:1104-1112) has 3 bytes of padding
  after the one-byte `data_tag` (1308). But a read of a payload field would have to load the header. Signatures,
  index infos, symbols, mappers and link records have no common header.
- **Pointer tagging.** A `P` is a 32-bit handle that counts 8-byte units over a 32 GiB reservation (ptr.rs:128-136,
  reserve.rs:18-20), so no bit is spare. But the reservation sits at a constant address (`BASE_ADDR =
  0x4001_0000_0000`, reserve.rs:41, chosen because "a loaded base cost ~25% more instructions"). If a window of the
  reservation is kept for the frozen region, for example its top 4 GiB, then "is this object frozen" is one
  comparison of the handle (or of the address minus a constant) against a constant. It needs no load and works for
  every kind of object. **This is the option chosen.**
- **An indirection for shared objects only** (a separate handle type). Every function that takes a `P<Type>` would
  have to handle both, which is the monomorphized checker the spike considered and did not build ("doubles the
  checker's code", section 10.5).

With the window, each spike mechanism changes as follows:

| spike mechanism | in this design |
| --- | --- |
| 1, `OvCell` | one comparison on unset reads and one on writes of the 47 fields; no load |
| 4, `object_flags_lazy()` | gone: the families that read no lazy field are computed at the freeze, so their reads stay plain; the others take the same one comparison as the 47 fields, and only when their "computed" bit is unset |
| 5, `with_ref()` | gone: the tables hung off types become `FrozenCell`, which has no borrow counter in release builds (frozen.rs:15-16, 47-63); this also drops the counter writes `RefCell` does today |
| 3, two-level maps | `keyBuilder` ANDs the handles it writes (`write_type`, checker_09.rs:448; 31 sites), and a lookup probes the frozen map only when all of them are frozen |
| 2, link stores | still a null test of the seed's store on every miss and every record creation. Same in kind as the spike; this is the part the experiment has to price |

### 5.3 The falsifying experiment (R1)

**The change.** On main, add exactly the read path of section 5.2 to the default build, with the window never
allocated so that every object is private:

1. the window test helpers;
2. `ShCell` (the spike's `OvCell` with the constant test) on the fields that forks write on frozen objects (the
   spike's list of 47, re-derived);
3. two-level hooks with the AND-reduced key test in the interning maps;
4. the shared-target test at every lookup and insert of the four instantiation tables;
5. the seed-store null test on link-store misses;
6. `FrozenCell` for the tables hung off types.

Go mode is not touched. This is 2-3 days of work, most of it ported from the spike branch.

**The measurement.** `depot ci dispatch --repo maschwenk/tsrs --workflow pr-verify.yml --ref <branch> --input cpus=16
--input projects=t3code-server,formbricks-web,cal-diy,supabase-studio,mikro-orm,vscode,xstate-main --input
checkers=1,8 --input reps=10 --input poison=false`. This gives:

- single-threaded instructions through `bench/count.py`, which repeat to about 0.001% on the bench machine
  (bench/README.md "Regression flag");
- ten interleaved runs per binary at 8 checkers on the scoreboard's machine type;
- a check that the diagnostics are identical.

**The kill threshold.**

- Kill if single-threaded instructions rise more than 1% on any project (the AGENTS.md bar).
- Kill if the 8-checker paired wall median rises more than 1% on two or more of the six losing projects. That is
  half the 2% budget; the build needs the other half.
- A pass needs at most +0.5% instructions everywhere.

pr-verify builds plain release binaries. A PGO build could move the result in either direction, so a pass should be
repeated with `bench.yml`'s PGO build before stage 1.

## 6. Cost 2: the build

### 6.1 The budget

- +2% wall at 8 checkers is 21-49 ms: cal-diy 21, formbricks-web 23, supabase-studio 25, mikro-orm 31, vscode 35,
  t3code-server 49 (spike section 10.5).
- At 10 permille the seed's serial time `T_w` was 0.01-0.21 s on the Mac (section 4.4) and 139-274 ms with FA on
  Linux (section 10.2).
- -5% peak needs `T_w` of about 90-150 ms on Linux (about 300 ms on vscode).
- Each ms of `T_w` saved 0.3-0.95 MiB of peak, with no knee (section 10.3).

### 6.2 Options

**(a) Parallel build with deterministic numbering at a barrier** (Bun's technique). Eight builders each check 1/8 of
the sample with their own checker, then merge equal records and number them by (task, index).

- Critical path, estimated: at least L + (T_w - L)/8 + merge time, where L is the library core that a cold checker
  builds before its files' own work. From throwaway productivity p = 0.24-0.49, L is about (1 - p) T_w, which gives
  0.57-0.79 T_w: 80-215 ms at 10 permille. The merge would rewrite every handle of up to 8 x 70-345 K types; it was
  not measured.
- Memory: as the seed.
- Exact only for deterministic fields: two builders fill asker-dependent fields differently, and the merge would
  have to pick one.
- **Fails on wall.**

**(b) Dependency-ordered start.** Threads begin on files whose library needs are already seeded.

- Critical path, measured with F: the exposed part (1 - p) T_w is 100-175 ms on the app projects.
- Memory: as the seed.
- Exact under the history argument.
- **Fails on wall.**

**(c) Bun-style barrier steps for everything.**

- Critical path: step 1 is a single task, then 8, then 64, then the rest (bun driver/lib.rs:261-273). bun check was
  slower than tsrs on all 17 projects on the 64-vCPU board and on all six of today's memory losses on the 16-vCPU
  board (mem-round4.md, introduction and section 7).
- Memory: Bun's.
- Not tsgo's numbering, by construction:
  - unions fall back to the creation step and are re-sorted at barriers (check/unions.rs:2295-2310,
    types.rs:2548-2571);
  - `isDeeplyNestedType` uses creation order (check/relate.rs:3983);
  - diagnostic texts can differ from tsc's (check/sink.rs:10-12).
- **Fails on exactness and on effort**: it rewrites the checker's state model.

**(d) Persisting the layer between runs.**

- Critical path: about 1-5 ms to map a 15-36 MiB image on a warm run (design-persisted-frontend.md section 5: 0.57 µs
  per 16 KiB page). A cold run has nothing to map and gains nothing.
- Memory: the seed's whole saving, with no `T_w` and no rebuild.
- Exact if every input is hashed. It needs a persisted front end too, because the image refers to AST nodes and
  binder symbols by handle.
- **Helps users and the language server, not the cold scoreboard.**

**(e) Global libraries only** (`lib.*.d.ts`, `@types`, reference directives) instead of the whole declaration-file
closure.

- Critical path: could be built in parallel before the pass, as #202 forces the global lazy lists in 1 ms.
- Memory: about 0.5-5% peak at 8 checkers (estimate, section 7.1).
- Exact.
- **Fails the 5% bar.**

**(f) Late freeze (this design).**

- Critical path, estimated: the floor plus the rebuild of the freed pre-switch graphs. The estimate is the paired
  FA - `on0` difference (section 10.1) less what the ninth thread and the discarded seed work cost: 11-16 ms for B
  plus 10-30 ms of contention (section 10.3), i.e. 21-46 ms, 1-5 points of these walls:

  | project | FA - `on0`, measured | late freeze, estimated |
  | --- | ---: | ---: |
  | formbricks-web | +2.5% | -2..+0.4% |
  | supabase-studio | +2.2% | -2..+0.3% |
  | mikro-orm | -1.4% | about 0 or below |
  | vscode | +0.9% | about 0 or below |
  | cal-diy | +7.8% | +3..+6% |
  | t3code-server | +11.6% | +9..+11% |

- Memory: as FA, -5.7..-14.4%.
- Exact under the history argument.
- **Fails on wall for cal-diy and t3code-server.**

### 6.3 What would have to be true

The spike's decomposition still holds. The extra wall equals the wait for the seed plus the change in the check span,
to within 10 ms (section 10.2). Late freeze replaces the wait with checks that do real work, so what remains is:

    floor + rebuild + (seed thread contention, now 0)

For four projects that is within the budget only if the floor is near zero. For cal-diy and t3code-server the
rebuild alone exceeds it:

- On t3code-server, 54% of what an extra checker creates is Effect generics instantiated with the project's own
  types (mem-per-checker-duplication.md section 3). No seed holds those, and every fork rebuilds them after its
  pre-switch graph is freed: FA cost +16.7% user CPU there (section 10.1).
- A fork cannot avoid the rebuild by keeping its pre-switch graph. It would then hold two objects for one type (its
  own `Array<string>` and the seed's), and a union could contain both.

Turning the layer on only where it pays needs a predictor that works before the check starts, and none exists:

- static weights miss the costly files (mem-round4.md section 3: `bincli.ts` ranks 1008 of 1,518);
- the spike's critical-file test showed the tail does not follow from file weight (711e3488).

## 7. Memory

### 7.1 Expected saving at 8 checkers, and bun

Sources:

- The tsrs and bun columns are the 16-vCPU scoreboard at bench 55c2d9ce (notes/mem-round4.md section 7, on branch
  `notes/discarded-work`, 81b504bc).
- "FA peak" is measured (spike section 10.1, run 6slgs9vngp). It stands in for this design's seed.
- "Library-only ceiling" is an estimate: a perfect, free share of the library-only types and symbols. It takes
  mem-per-checker-duplication.md section 4's 16-checker savings, scales them by 7/15 and by 1.0-1.5 for the steeper
  per-checker slope below 8 checkers (section 1 of that note: 1.2-1.5x), and divides by the Linux 8-checker peak.
- "Global only" multiplies the ceiling by the share of lib and `@types` among declared objects (that note's package
  table): 13 / 28 / 48 / 46 / 40% on t3code / formbricks / supabase / cal-diy / vscode.

| project | tsrs / bun GiB (bun/tsrs) | library-only ceiling | global only | FA peak | FA vs bun | gap closed | wall within +2%? |
| --- | --- | ---: | ---: | --- | ---: | ---: | --- |
| t3code-server | 1.76 / 1.12 (0.64x) | 4.3-6.5% | 0.6-0.8% | 1.60 (-9.6%) | 0.70x | 25% | no (estimate +9..+11%) |
| supabase-studio | 1.39 / 1.21 (0.87x) | 5.8-8.7% | 2.8-4.2% | 1.24 (-10.4%) | 0.98x | 83% | only with a free read path |
| mikro-orm | 1.60 / 1.40 (0.88x) | not estimated | not estimated | 1.37 (-14.4%) | 1.02x | all, 0.03 GiB below bun | only with a free read path |
| cal-diy | 1.44 / 1.29 (0.89x) | 7.3-10.9% | 3.4-5.0% | 1.22 (-13.7%) | 1.06x | all, 0.07 below | no (estimate +3..+6%) |
| formbricks-web | 1.68 / 1.60 (0.95x) | 5.4-8.1% | 1.5-2.3% | 1.46 (-12.6%) | 1.10x | all, 0.14 below | only with a free read path |
| vscode | 1.92 / 1.86 (0.97x) | 1.3-2.0% | 0.5-0.8% | 1.81 (-5.7%) | 1.03x | all, 0.05 below | only with a free read path |

The seed beats the library-only ceiling on vscode and mikro-orm because it also holds project hub types from its
sample files. Under both constraints the answer to "what memory is left" is: nothing on cal-diy and t3code-server,
and the FA column on the other four only if R1 shows a free read path. A layer that cannot be switched on per
project costs wall on the first two whenever it is on.

### 7.2 What it cannot reach; a v2

The frozen layer holds what the seed's history created. It cannot hold instantiations created later during checking.
Two classes of those matter:

- **Library generics instantiated with library arguments that the seed never made**: node_modules library-only
  instantiations are 12-29% of an extra checker's types on the app projects, and library-only unions and literals
  another 11-34% (mem-per-checker-duplication.md section 3).
- **Library generics instantiated with project types**: 54% of t3code-server's extra types and 50% of its extra
  symbols. These need the project's types shared as well, which means sharing the whole checker (Bun's design). No
  type layer reaches them, v1 or v2. This is why t3code-server stays at 0.70x even with FA.

**v2, a shared instantiation table with publication.**

- When a fork misses in both the frozen and its own table for a target whose arguments are all shared, it creates
  the instantiation in the shared window under a placeholder taken by CAS, then publishes it.
- Other forks wait for it, as #202's lazy lists do (lazylist.rs:230-292: spin, then a striped condvar).
- Published ids come from a range between the seed's and the forks' private ids. That keeps the history argument
  valid: every published object depends only on objects published before it, so "all published objects first" is a
  creation order that a single checker could have had.

**Can v2 be exact?** Only for published shells, or for values computed in a fresh checker context (empty
resolution stack, zero budgets, empty relation caches) that met no cycle, budget, diagnostic or relation along the
way. Anything else stays private. Three unknowns remain:

- what survives those exclusions was not measured;
- the waits are the lazy-list problem at a finer grain: drizzle-orm paid +9% wall from 2,439 waits before global
  lists were forced first (mem-round4.md section 1);
- two forks that each hold one placeholder and wait for the other's deadlock unless the wait-for chain is checked.

The spike plan called this design B: "the only design that can hang".

## 8. Effort, risk, staging

**Work, estimated:**

| work | estimate |
| --- | --- |
| R1 (section 5.3) | 0.5 week |
| Field classification on current main (discovery build with `mprotect`; the 47/79 lists and the link-record fields again) | 0.5 week |
| The window in reserve.rs, the frozen region, the per-fork dirty bitmap and side tables (`ShCell`) | 1 week |
| Late freeze: the sample on checker 0, scratch-region checkers 1-7, the switch (finish the file, copy out diagnostics and deferred checks, retire, fork) | 1 week |
| Tables: two-level interning maps, the instantiation-table split, link-store read-through, flag families at the freeze, `FrozenCell` swaps | 1 week |
| A generated `Checker::fork` over the 347 fields of `Checker` (checker.rs:941-1340) instead of the spike's hand-written 345 | 0.5 week |
| Memos keyed by `type_count`, Go mode off, `TSRS_CENSUS` / heap census and the `plain-ptrs` build made aware of the frozen region | 0.5-1 week |
| Fidelity gates (stage 2) | 1-2 weeks, open-ended if a history-dependent difference appears |

The fidelity gates are: conformance in the default mode, with `TSRS_LAZY_MEMBERS=0`, and canonical with 4 checkers per
test; fourslash; `tools/regressions.sh`; `tools/ci/determinism.sh` with the layer on; arena poison; `mprotect` as a
CI mode; the bench projects at 1, 8 and 16 checkers through verify.py; and the 38k-file codebase, which the owner
runs. **Total: 6-8 weeks to a landable pull request**, if stage 1's kill criterion does not fire.

**Risks:**

- Fidelity rests on the history argument, which is tested rather than proven, with three programs open.
- The invariant burden: every new checker field has to be added to the fork, and every new lazy field has to be
  classified. An unclassified write to a frozen object is a silent race unless `mprotect` is on.
- Size: the spike was about 2,400 lines over 32 files.
- The language server and the API are excluded.

**Reusable from `origin/spike/shared-graph-seed`:**

- `basedmap.rs`, with the spike's E1 skip;
- `links.rs`'s fork, read-through and `LinkCopy`;
- `sharedgraph.rs`'s freeze, `mprotect` and fault handler, with the discovery logging (`TSRS_SHARED_GRAPH_PROTECT=log`,
  `TSRS_SHARED_GRAPH_LOG_OWNED`);
- the 47/79 field lists;
- F's scratch-region throwaways (6f0dfcd9);
- `Checker::fork` as a starting list;
- `TSRS_TIMELINE`;
- `tools/perf/sharedprobe.sh` with the `perf-probe` workflow.

Not reusable: `OvCell`'s range test against globals, `object_flags_lazy`, `with_ref`, and the separate seed thread.

**Stages:**

**Stage 0: R1, the read path on main with nothing shared (2-3 days).**

- Gate: single-threaded instructions at most +0.5% on t3code-server, formbricks-web, vscode and xstate-main; paired
  wall at 8 checkers within ±1% on the six losing projects.
- Kill: more than +1% instructions on any project.

**Stage 1: late freeze and forks on stage 0's read path, Go mode off (2-3 weeks).**

- Gate (AGENTS.md): at least 5% peak at 8 checkers on two or more of the six; wall within +2% paired (10 reps) at 8
  checkers on all 17 bench projects; single-threaded instructions at most +1%; diagnostics identical to main in
  every cell.
- Kill: cal-diy or t3code-server above +2% wall. Section 6.2 predicts +3..+6% and +9..+11%.

**Stage 2: landing (2-3 weeks).**

- Gate: every fidelity gate above identical to main.
- Kill: any output that main does not print, outside the three open programs.

**Stage 3: v2.** Not planned.

## 9. Recommendation

Do not build. The numbers that decide it are already measured:

- Even if the read path cost nothing, which no measurement has shown (the spike's cost +2.3..+4.8%), the best seed
  strategy measured +7.8% wall on cal-diy and +11.6% on t3code-server above the empty-seed floor at 8 checkers (spike
  section 10.1).
- Folding the seed into a pool checker removes an estimated 21-46 ms of that, 1-2 points on t3code-server and 2-4.5
  on cal-diy, which leaves both over the 2% bar.
- The layer cannot be switched on per project before the check starts. So it costs wall on the scoreboard.
- The memory it buys (-5.7..-14.4% at 8 checkers) flips four losses that are already within 3-13% of bun, and leaves
  t3code-server, the largest gap, at 0.70x.

The 2-3-day read-path experiment (section 5.3) is the cheapest way to close the direction for every use, warm runs
and 32-checker machines included. It is not a reason to start the build.

## 10. Not determined

- The cost of the redesigned read path. Nothing was built, and the spike's +2.3..+4.8% is for a different path.
- The rebuild cost of late freeze. It is estimated as FA - `on0` less the spike's estimates for its seed thread, and
  FA - `on0` mixes two variants' noise (paired interquartile ranges of several percent).
- mikro-orm's per-checker breakdown, which mem-per-checker-duplication.md does not cover; its ceilings are not
  estimated.
- Linux values of the library-only and global-only ceilings: section 7.1 scales Mac numbers, and the global shares
  are of declared objects, not of bytes.
- The merge cost of option (a).
- Whether a frozen layer is exact on the conformance suite, fourslash and the 38k-file codebase. The spike ran none
  of them with the switch on.
