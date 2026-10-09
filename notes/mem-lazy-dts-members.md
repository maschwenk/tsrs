# mem-lazy-dts-members: parse and bind the member lists of unchecked declaration files on first use

On the projects where `bun check` beats tsrs on memory, most of the program is `node_modules` declaration files that
are parsed and bound in full and never type-checked (`skipLibCheck`). The checker resolves declarations lazily
(notes/lazy-members.md), so of an interface, class or type literal in such a file it only needs the members some
checker looks up. This change parses those member lists as usual, throws the nodes away when nothing in them reaches
outside the list, and parses and binds them again the first time anything asks for them (V8-style lazy parsing,
applied to member lists, with lazy binding of the same lists). It is a front-end change, so the saving is about the
same number of bytes at every checker count. Output is byte-identical. It does not make parsing faster on Linux: a
needed list is parsed twice, and the first parse still runs in full (section 6).

Result, Linux 64 vCPUs at the default 32 checkers (10 interleaved runs against main e80e776, medians): formbricks-web
-7.1% peak RSS (2.81 -> 2.62 GiB), cal-diy -1.6%, supabase-studio -1.8%, t3code-server -1.5%, drizzle-orm -1.8%,
xstate-main -2.2%, vscode and webpack 0, with wall time within +-1% on all eight projects (drizzle-orm +0.3%). At 4
checkers -12.0% / -4.9% / -4.7% / -3.2% / -3.7% / -3.6% / 0 / 0, single-threaded (earlier run) -16.2% / -7.3% / -6.5%
/ -5.2% (formbricks-web / cal-diy / supabase-studio / t3code-server). On the Mac with `--noCheck` -25% /
-15% / -13% / -10% / -1%, and 0.8-4.4% fewer instructions single-threaded.

Round 2 (section 7) measured what the lists left eager could add (namespace and module bodies, lists with import types
or eager JSDoc): 0.8-1.6% of the 8-checker peak on the 16-vCPU runner, at most 4.3% for every list of every kind.
Rejected.

## 1. Ceiling (census)

Tool (branch `mem/lazy-dts-census`, not merged): the alloc-profile build with `TSRS_LAZY_DTS_CENSUS=1`
(`tsrs_core::lazydts_census`, `tsrs_compiler` lazydts_census.rs). The parser and the binder record, per member of
every interface, class, type literal and module block of a declaration file, the arena bytes it took plus the thread's
net heap bytes (binder symbol tables keep their entries on the heap); before the first checker is created the compiler
registers those lists in the files the program will not check, with their owner and member symbols. From then on a
read of a registered list's nodes (`NodeList::nodes`), of an owner's `members` / `exports`, and a symbol-table hit or
declarations read of a member symbol set a bit, tagged with the phase (before checker creation, inside `new_checker`,
while checking). A list counts as asked for if its nodes were read or its owner's table was (members for interfaces
and type literals, members or exports for classes, exports for module blocks): exactly what forces a list in the
design below. "Never asked for" counts the outermost such lists (all enclosing candidate lists asked for). One run per
project at 16 checkers on the Mac (main 6e07a55 + tool).

| project | unchecked .d.ts parse+bind MB | in member lists | never asked, all kinds | with module blocks eager | design scope¹ | Mac peak 16 (MiB) | share | Linux peak 32 (MiB) | share |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| formbricks-web | 433 | 367 | 312 | 269 | 254 | 2,184 | 11.6% | 2,949 | 8.6% |
| cal-diy | 179 | 121 | 93 | 86 | 69 | 1,921 | 3.6% | 2,632 | 2.6% |
| supabase-studio | 225 | 138 | 107 | 96 | 74 | 1,745 | 4.2% | 2,345 | 3.1% |
| t3code-server | 131 | 104 | 83 | 81 | 45 | 2,210 | 2.0% | 2,857 | 1.6% |
| vscode | 49 | 39 | 19 | 16 | 12 | 2,137 | 0.6% | 2,714 | 0.5% |

¹ Interfaces, classes and type literals whose first parse has no disqualifier (section 2), outermost eligible lists
never asked for. The Linux peaks are the README table's (2026-10-07); the bytes are the same arena bytes on both
(compressed pointers on both), so the share is bytes over that peak.

- Namespaces: formbricks-web's googleapis is 235 MB of its 433 (`export declare namespace compute_alpha { ... }`, one
  per API), 229 MB of it never asked for. Keeping module blocks eager costs 43 MB there and 2-11 MB elsewhere,
  because their interfaces and classes are lazy on their own; they are left eager in this round (section 6).
- Global merging at checker creation (`mergeSymbolTable` over interfaces declared in several global files) forces
  1.1-2.7 MB of interface lists per project; namespaces and ambient module bodies forced there are 1-37 MB, but those
  stay eager anyway.
- Per member: of the members of asked-for lists (formbricks 101,743), 24% are never looked up by name nor have their
  declarations read (formbricks 28 MB, cal-diy 15, supabase 19, t3code 11, vscode 10): about 1% of peak more for a
  per-member design, not attempted.
- Disqualified, outermost-level lists, MB (overlapping): import types 26.6 / 61.2 / 4.5 / 113 / 0.1 (formbricks /
  cal-diy / supabase / t3code / vscode); eager JSDoc (`@see`, `@link`) 16.5 / 14.0 / 16.6 / 11.9 / 3.8; `this`
  7.0 / 8.5 / 8.0 / 5.1 / 6.0; not rewindable (a number cache pin or a chunk boundary inside the list) 16.8 / 7.9 /
  14.9 / 16.5 / 0.9 before the scanner fix below, 7.6 / - / - / 5.2 / - after it.

## 2. Design

Scope: the CLI's batch compile (`perform_compilation`, not `--incremental`, `--build` or watch) when no declaration
file is type-checked (`skipLibCheck` or `noCheck`); interface, class and type literal member lists of declaration
files. Off in the language server, the API and the test harnesses; `TSRS_LAZY_DTS=0` turns it off,
`TSRS_LAZY_DTS=stats` prints counts, and `TSRS_LAZY_DTS=force` in `tsrs-test` makes every declaration file lazy (also
checked ones) to test forcing.

- **Parser** (`parse_member_list_lazily`). The list is parsed as usual inside an arena checkpoint. It becomes lazy if
  it is non-empty, the parser had no pending error at its start, and the parse added no diagnostic, no eager JSDoc
  (`@see` / `@link`), no reparsed clone, no comment directive and no source flag (each of those keeps arena data the
  list allocated), the arena can discard everything since the checkpoint (same chunk, no pin or free), and nothing in
  it reaches outside the list when bound (`lazy_list_blocked`: `this` / `this` types and `super`, which set
  `ContainsThis` on enclosing containers; `infer`, which declares its type parameter in the enclosing conditional
  type; import types and calls, which the file's import list is collected from right after parsing; bodies,
  decorators, `async`, initializers and expressions other than names, property accesses and signed numbers, which
  make flow nodes or flow effects). Then the arena is rewound to the checkpoint, nested lazy records inside the list
  are dropped with it, and the owner gets a `NodeList` whose long-form `ThinSlice` points at a `LazyNodeList` record:
  `pos`/`end`, the parsing context (type or class members), the parser's context flags and enclosing parsing
  contexts, and (once the owner node exists) the owner. The scanner, the counters (`Identifiers`, nodes) and the
  file's diagnostics are those of the full parse. Speculative parses truncate the record list on rewind like their
  other parser data. The scanner's number caches now pin the arena only when the cached strings are not slices of
  the source text (they were pinned for every first occurrence of a number in a file, which also cancelled
  speculative rewinds).
- **Binder** (`bind_lazy_member_list`). An unparsed list has no members yet, so binding the owner's children skips
  it; right after, the binder records `container`, `this_container`, `block_scope_container`, `last_container`,
  `current_flow` and its `unreachable_flow` on the record and registers the record on the owner symbol
  (`Symbol::set_lazy_list`, a field of the symbol's tables tail). In an unusual state (flow targets or labels set,
  unreachable code, no owner symbol) it parses the list on the spot and binds it as if it had never been lazy.
  Members of the owner symbol are filled by its declarations in source order; a later declaration of the same
  symbol in the same file that fills a table the list fills (an interface or class: `members`; a namespace or enum:
  `exports`, which only a class list fills) forces the pending list first (`force_pending_lazy_list`), so at most the
  last declaration's list of a symbol is ever pending and table insertion order is the eager one. Value and function
  merges do not force. A list the binder never reached is marked so that forcing only parses it.
- **Size limit.** A list longer than 64 KB of text stays eager: parsing and binding it again takes a millisecond
  or more while every checker that needs it waits. supabase-studio's generated OpenAPI `operations` and `components`
  interfaces (65-437 KB each) are needed by most checkers; on the first Linux runs they cost it 0.1-0.3 s of summed
  waiting at 32 checkers. The limit costs at most 0.5% of peak on the bench projects (Mac, 32 checkers).
- **Forcing** (`LazyNodeList::force_nodes`). `NodeList::nodes` (the only accessor of a list's slice) checks the
  long-form bit it already tested; a lazy list goes to the record. `Symbol::members` and, for class lists,
  `Symbol::exports` check the tail's lazy field. The first reader parses the list again from `pos` with the recorded
  context (a fresh parser on the file's text, nested lists eager), sets the members' parent to the owner, binds them
  with a fresh binder seeded with the recorded state (containers the members add to the container chain are spliced
  in where the list was), publishes the slice (Release) and wakes waiters. Other threads spin briefly (256 `spin_loop`s,
  some microseconds: most lists take 5-30 µs; formbricks-web forces 9,400 lists in 40-80 ms of thread time), then
  sleep on one of 64 condvars picked by the record's address; the forcing thread's own reads during binding
  (re-entrant) get the list being bound. A first version spun 16K times: on the x86 runner that burned CPU next to
  the hyperthread sibling (drizzle-orm +15% user time and +9% wall at 32 checkers). Allocation goes to the forcing
  thread's own arena (out of any scratch region). Before the file is bound a lazy list reads as empty, which only the
  file's parser (parent pointers, the import scan) and binder can observe.
- **Shared lists** (`force_shared_lists`, fileregions.rs). Before the checkers of a multi-checker pass are created,
  two sets of lists are forced in parallel on the worker pool (1-4 ms), because every checker asks for them at the
  start and would otherwise wait for whichever checker got there first:
  - the global libraries: every declaration file that enters the program through a default lib, the `lib` option or
    `/// <reference lib>`, an automatic type directive, the `types` option or `/// <reference types>`, and the files
    those pull in with `/// <reference path>` (`@types/node`'s `index.d.ts` references the rest of the package).
    Module-scoped packages that enter only through imports (prisma, react's own files, zod, next, googleapis) stay
    lazy;
  - interfaces and classes that merge into the global scope more than once (declared in several script files or
    `declare global` blocks), which `initialize_checker` clones and merges in every checker.
  Both are properties of the program, not of a run. On drizzle-orm (16 checkers, Mac) readers waited 2,439 times on
  1,062 lists for 70 ms in all, 1,871 of the waits on `@types/node` (three versions, each pulled in by
  `/// <reference types="node" />` from a different package) and 330 on `bun-types`; with the rule 97 waits for 3 ms.
  The rule costs no measurable memory: those lists are forced early in a multi-checker run anyway (Mac, 4 and 16
  checkers, formbricks-web / cal-diy / supabase-studio / t3code-server: -10 to +14 MiB, inside the run-to-run spread;
  Linux at 32 checkers 0-20 MiB, below the 0.01 GiB resolution of the probe on most projects). Single-threaded runs
  and one-checker runs skip it.

## 3. Exactness

- Parse: a lazy list is parsed again by the same parser code from the same scanner position with the same context
  flags and enclosing parsing contexts, and the first parse had no error, so the members are the same nodes (kinds,
  positions, flags, JSDoc flags, children) with the owner as parent. Nothing outside the list kept a pointer into the
  discarded memory: the parser's lists that can receive such pointers are checked unchanged, nested records are
  truncated, and a pin cancels the rewind. `TSRS_ARENA_POISON=1` (rewound memory filled with 0xA5 and never reused)
  ran clean on five projects.
- Bind: the members are bound with the binder state they would have seen, into the same owner tables in the same
  order (the merge rule), with the same flow nodes on their identifiers and the same container chain. What binding a
  list could change outside it is blocked at parse time (`this`, `infer`, flow). Two outputs differ: bind diagnostics
  of lists bound by a reader are dropped (only lists of files the CLI does not check are lazy, and it reports bind
  diagnostics only for checked files) and their symbols are not counted, so `--extendedDiagnostics` `Symbols` drops
  (formbricks-web at 16 checkers 5.75M -> 4.80M). `Types` and `Instantiations` are unchanged single-threaded and with
  a named assignment on seven projects.
- When: readers reach members only through `NodeList::nodes` and the owner's tables, which force first, and node
  and symbol ids are assigned on first use per thread, so nothing observable depends on when or by which thread a list
  is forced.
- A bug the counters caught during development: an `infer` inside a type literal that is a mapped type's template in a
  conditional's `extends` (Prisma's runtime types) was declared into the conditional's locals only when the list was
  forced, after the checker had read them. Blocked now; testdata/regressions/lazy-dts-members covers it (without the
  block its first error disappears), merged interfaces with different type parameter names, class statics merged with
  a namespace, an interface merged with a value and a namespace, nested and parameter type literals, global
  interfaces in two files, and the eager cases.

## 4. Numbers

Linux, Depot 64 vCPUs (`tools/perf/lazydtsprobe.sh`: main e80e776, which has #204, against this branch at 7b1a14e,
interleaved, 10 runs per cell at 32 checkers and 5 at 4, medians; "paired" is the median of the per-run ratios). Peak
RSS in GiB, wall and user+sys in s.

| project | checkers | wall main -> branch | paired | peak main -> branch | user+sys main -> branch |
| --- | ---: | --- | ---: | --- | --- |
| formbricks-web | 32 | 0.659 -> 0.658 (-0.1%) | +0.5% | 2.81 -> 2.62 (**-7.1%**) | 12.93 -> 13.00 |
| cal-diy | 32 | 0.593 -> 0.591 (-0.3%) | -0.1% | 2.57 -> 2.53 (-1.6%) | 12.51 -> 12.65 |
| supabase-studio | 32 | 0.575 -> 0.577 (+0.4%) | +0.9% | 2.23 -> 2.19 (-1.8%) | 13.73 -> 13.67 |
| t3code-server | 32 | 1.460 -> 1.450 (-0.7%) | -1.1% | 2.72 -> 2.68 (-1.5%) | 19.17 -> 19.16 |
| drizzle-orm | 32 | 0.180 -> 0.181 (+0.3%) | +1.6% | 1.10 -> 1.08 (-1.8%) | 3.14 -> 3.19 |
| vscode | 32 | 0.606 -> 0.611 (+0.8%) | +0.8% | 2.61 -> 2.61 (0.0%) | 15.70 -> 15.66 |
| webpack | 32 | 0.133 -> 0.133 (0.0%) | -0.4% | 0.69 -> 0.69 (0.0%) | 2.67 -> 2.67 |
| xstate-main | 32 | 0.121 -> 0.120 (-0.8%) | -0.4% | 0.46 -> 0.45 (-2.2%) | 1.78 -> 1.79 |
| formbricks-web | 4 | 1.424 -> 1.435 (+0.8%) | +0.7% | 1.66 -> 1.46 (-12.0%) | 6.54 -> 6.57 |
| cal-diy | 4 | 1.326 -> 1.317 (-0.7%) | -0.1% | 1.23 -> 1.17 (-4.9%) | 5.75 -> 5.78 |
| supabase-studio | 4 | 1.590 -> 1.589 (-0.1%) | -0.1% | 1.27 -> 1.21 (-4.7%) | 7.28 -> 7.27 |
| t3code-server | 4 | 1.936 -> 1.947 (+0.6%) | +0.4% | 1.24 -> 1.20 (-3.2%) | 8.18 -> 8.15 |
| drizzle-orm | 4 | 0.350 -> 0.349 (-0.3%) | +0.9% | 0.54 -> 0.52 (-3.7%) | 1.71 -> 1.75 |
| vscode | 4 | 2.705 -> 2.715 (+0.4%) | +0.6% | 1.83 -> 1.83 (0.0%) | 12.53 -> 12.62 |
| webpack | 4 | 0.346 -> 0.343 (-0.9%) | -0.9% | 0.42 -> 0.42 (0.0%) | 1.63 -> 1.63 |
| xstate-main | 4 | 0.210 -> 0.212 (+1.0%) | +1.0% | 0.28 -> 0.27 (-3.6%) | 1.01 -> 1.04 |

Second runs at 32 checkers (branch f65dc87 against main e80e776 + bench results): 20 interleaved runs: cal-diy 0.0%,
formbricks-web +0.5% (paired +0.1%, peak -7.1%), drizzle-orm +1.9% (paired +2.5%, IQR -0.6..+3.8), nuxt +1.5%; 10
runs: t3code-server -1.0%, playwright +0.8%. So drizzle-orm sits at +0.3% to +1.9% across two probes. Its remaining
2-4 ms are 1 ms of shared-list forcing before the checkers start and the second parse of lists the checkers need.

An earlier run (main fcb9803, 9 runs, before the shared-list rule) had single-threaded peaks -16.2% / -7.3% / -6.5% /
-5.2% / -0.6% (formbricks-web / cal-diy / supabase-studio / t3code-server / vscode); the rule does not apply there.
`Parse time` (which includes binding) on Linux single-threaded is 1-4% longer than with the mode off (formbricks-web
0.883 -> 0.905 s; the walk for disqualifiers and the rewinds cost more than the skipped binding saves where the arena
has huge pages), and unchanged at 32 checkers. The runner's instruction counter returns nothing (`instructions:u`
reads 58), so instructions come from the Mac.

**The wall cost that the shared-list rule removed: drizzle-orm** (21 MB of declaration text, 0.18 s at 32 checkers,
10,846 errors). Before the rule it was +9% wall and +13% user time at 32 checkers against main fcb9803, +4.5% after
#204 (which removed contention on the reporting file's `Path` `Arc` in `DiagnosticsCollection::add`, where checkers
that had waited on the same lists ran in lockstep), and about +8% against main with #204 (0.173 -> 0.187 s). The rest
was the waiting itself: 2,439 waits on 1,062 lists, mostly `@types/node` and `bun-types`, all global libraries entered
through type reference directives. With the rule: +0.3% (0.180 -> 0.181 s, paired +1.6%, the runner's spread is 2-4%).

Mac (M5 Max, 18 cores, 3 interleaved runs, medians; main cdcfc0e against 744c2ad; peak footprint in MiB, instructions
in billions; a loaded machine, so walls are not compared):

| project | single: peak, instructions | 16 checkers: peak, instructions | `--noCheck`: peak |
| --- | --- | --- | --- |
| formbricks-web | 1,304 -> 1,090 (-16.4%), 53.6 -> 51.2 (-4.4%) | 2,180 -> 1,978 (-9.3%), 91.8 -> 90.5 (-1.3%) | 967 -> 724 (-25.1%) |
| cal-diy | 813 -> 755 (-7.1%), 41.8 -> 40.4 (-3.2%) | 1,955 -> 1,857 (-5.0%), 98.5 -> 97.1 (-1.4%) | 469 -> 400 (-14.7%) |
| supabase-studio | 926 -> 865 (-6.6%), 60.7 -> 58.6 (-3.3%) | 1,747 -> 1,683 (-3.7%), 109.9 -> 109.1 (-0.8%) | 594 -> 516 (-13.1%) |
| t3code-server | 768 -> 729 (-5.1%), 48.4 -> 47.9 (-1.2%) | 2,222 -> 2,175 (-2.1%), 153.9 -> 153.4 (-0.3%) | 432 -> 387 (-10.4%) |
| vscode | 1,611 -> 1,601 (-0.6%), 101.7 -> 100.9 (-0.8%) | 2,112 -> 2,097 (-0.7%), 116.6 -> 115.7 (-0.8%) | 1,179 -> 1,162 (-1.4%) |

Against the ceiling: the peak drops at 32 checkers on Linux (about 225 / 60 / 60 / 30 / 20 MiB, from GiB rounded to
two places) are 80-90% of the census's design-scope bytes on the first four projects.

`TSRS_LAZY_DTS=stats` on formbricks-web at 16 checkers: 116,694 lists lazy, 116,645 deferred by the binder, 9,374
parsed again, 422 of them while their file was bound (merges and unusual states).

## 5. Gates

- Conformance suite with `.types` / `.symbols`: whole result trees identical to main's in the default mode, with
  `TSRS_LAZY_MEMBERS=0`, and with `TSRS_LAZY_DTS=force` (every declaration file of every test lazy, checked or not:
  390,935 lazy lists, 79,453 forced by the checker and the baseline walks), also with
  `TS_TEST_PROGRAM_SINGLE_THREADED=false` and with `--baselines js` (JS and declaration emit). 13,458 error baselines
  pass (+2 codes, 2 fail as on main), 12,779 `.types`, 12,779 `.symbols`.
- Fourslash 4,066 pass / 63 fail, same lists. `cargo test -p tsrs_cli` integration tests pass (the binary's unit
  tests do not link on macOS, as on main). `tools/regressions.sh` 25 of 25 with the new lazy-dts-members case.
- Diagnostics byte-identical to main on formbricks-web, cal-diy, supabase-studio, t3code-server, vscode, webpack and
  xstate single-threaded and at 1, 4, 16 and 32 checkers (42 cells); `Types` and `Instantiations` identical
  single-threaded and with `--checkerAssignment locality` (14 cells); `TSRS_ARENA_POISON=1` single-threaded and at
  32 checkers on the five app projects, identical. A declaration-emit project gave identical `.d.ts` files.
- `RUSTFLAGS="-D warnings" cargo check --workspace --locked` (also with `alloc-profile`), `tools/lint/ratchet.py`
  (no new findings), `tools/lint/source.py` (no new `unsafe impl`; every atomic ordering commented).

## 6. Left and rejected

- Module blocks (namespace bodies): another 43 MB on formbricks-web (googleapis), 1-11 MB elsewhere. Needs the
  binder's per-body state computed at parse time (module instance state, export context flag, `locals`) and the
  ambient module import scan; not in this round.
- Import types (`import("x").T` in member types) keep a list eager: 30-60 MB of never-asked bytes on t3code-server
  and cal-diy. The file's import list holds the specifier nodes; a lazy list would need them kept outside it.
- Eager JSDoc (`@see` / `@link`, parsed with the file so unused-identifier checks see the references): 4-17 MB per
  project of lists stay eager.
- Per-member laziness (names eager, types lazy): about 1% of peak more (section 1).
- A waiting reader parsing the list itself and publishing by compare-and-swap (the alternative to the shared-list
  rule): not built. Parsing can race deterministically, but binding cannot (it fills the owner symbol's tables, which
  other threads read), so a waiter would still wait for the bind; and the rule already removes 96% of the waits for
  no measurable memory.
- Record overhead: 96 bytes per lazy list (formbricks-web 117K lists, 11 MB, 0.4% of peak); the record could shrink
  by about a third.
- Forcing re-parses: a list that is needed is parsed twice (once to establish it is error-free and plain). A skipping
  scanner (brace matching) would be cheaper but cannot know whether the list has syntax errors, which the CLI reports
  for every file before checking; caching that would need a persisted front end (rejected, #36).
- `--incremental`, `--build`, watch, the language server and the API keep eager lists: nothing measured there, and the
  builder, the project system and the API read trees in ways not audited here.

The first three were measured again in round 2 (section 7) and rejected.

## 7. Round 2: namespace and module bodies, import types, eager JSDoc (measured and rejected)

The question: of the three kinds of list this change left eager, (a) namespace and module bodies (`declare namespace`,
`declare module "x" { }`, `declare global { }`), (b) member lists with an import type (`import("x").Y`) and (c) member
lists with eager JSDoc (`@see` / `@link`), how many bytes does no reader ever ask for, as a share of the peak at the
scoreboard's checker count (8, on the 16-vCPU runner)? The bar (AGENTS.md) is 5% of that peak on at least one of the
projects where `bun check` still uses less memory.

**Method.** The census, rebuilt on main 872027a (branch `mem/lazy-dts-census-2`, not for merging): the alloc-profile
build with `TSRS_LAZY_DTS_CENSUS=1` runs with lazy lists off and records, per member list and module block of every
declaration file the program does not check, its parse and bind bytes (arena plus the thread's net heap), and why it
could not be lazy: the reasons of `lazy_list_blocked` and `parse_member_list_lazily` one by one (`this`, import
types, `infer`, flow, diagnostic, eager JSDoc, not rewindable, other, the 64 KB limit), and for module blocks also
imports, re-exports and nested ambient modules or `global` blocks, and `export { x }`. A list counts as asked for when
anything read its nodes or the owner table it fills (`members` for interfaces and type literals, `members` or
`exports` for classes, `exports` for module blocks) after the program was built, or when it is one of the lists
forced before checking (the global libraries and the global merges of `force_shared_lists`). The report then
replays the parse with a given rule for which lists are lazy and whether forcing a list keeps its nested lists lazy,
and sums the bytes of lazy lists nobody asked for, less 96 bytes per lazy record (section 6). S0 is this change
(member lists without a disqualifier, nested lists eager when forced); the other rules add to it: (a) every module
block without a disqualifier (`infer` allowed), forced bodies keep their nested lists lazy; (a\*) the same without
the 64 KB limit for module blocks; (b) member lists whose only disqualifier is an import type; (c) member lists whose
only disqualifier is eager JSDoc; "everything" makes every list of every kind lazy whatever it contains, nested lists
lazy when forced. One census run per project at 8 checkers on the Mac.

The census is an upper bound of what the peak loses: S0 predicts 229 / 61 / 42 / 64 MiB for formbricks-web /
cal-diy / t3code-server / supabase-studio, and the measured saving of this change at 8 checkers on the 16-vCPU runner
(`TSRS_LAZY_DTS=0` against main, 3 runs) is 218 / 46 / 34 / 52 MiB (75-95% of the census).

**Numbers** (MiB, net of records; peaks of main 872027a at 8 checkers: Linux on Depot `depot-ubuntu-24.04-16`, 3
interleaved runs, median; Mac M5 Max, peak footprint, 3 runs, median):

| project | Linux peak | Mac peak | (a) module bodies | (a\*) no size limit | (b) import types | (c) eager JSDoc | (a)+(b)+(c) | (a\*)+(b)+(c) | share of Linux peak | everything | share |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| formbricks-web | 1,710 | 1,686 | 7.9 | 53.9 | 2.5 | 3.0 | 13.6 | 60.4 | 0.8% / 3.5% | 73.2 | 4.3% |
| cal-diy | 1,468 | 1,451 | 4.8 | 5.0 | 6.8 | 1.8 | 13.6 | 13.8 | 0.9% / 0.9% | 20.9 | 1.4% |
| t3code-server | 1,805 | 1,789 | 0.5 | 0.5 | 24.3 | 4.3 | 29.3 | 29.6 | 1.6% / 1.6% | 35.7 | 2.0% |
| supabase-studio | 1,407 | 1,371 | 6.3 | 6.5 | 1.0 | 7.0 | 14.7 | 16.8 | 1.0% / 1.2% | 30.2 | 2.1% |
| mikro-orm | 1,641 | - | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

mikro-orm has no `skipLibCheck`: every declaration file is checked, so nothing is lazy there (Linux, 8 checkers:
1.604 GiB with `TSRS_LAZY_DTS=0`, 1.603 without). Shares of the Mac peak are within 0.1 point of the Linux ones.

Below the bar everywhere: all three kinds together are 0.8-1.6% of the 8-checker peak, 0.9-3.5% with module bodies of
any size, and the bound of the whole mechanism (every list lazy, whatever it contains) is 1.4-4.3%. Rejected; no
code. Where the bytes are:

- (a) Within the 64 KB limit, module bodies hold little: their interfaces and classes are lazy already, so a lazy body
  saves only the statement nodes, their symbols and the records (stripe's `namespace Stripe` blocks 2-4 MiB, date-fns,
  playwright-core). formbricks-web's 54 MiB without the limit are mostly googleapis (about 40 MiB): one namespace per
  API version, 399 of the 901 longer than 64 KB and 21 longer than 1 MB (compute alpha: 10.6 MB), which a forced body
  would scan again in full while other checkers wait. Every `declare global` and module augmentation body is asked
  for (`initialize_checker` merges them), and the global libraries' ambient modules are forced before checking; not
  forcing the module bodies among them adds 0.6-0.7 MiB.
- (b) t3code-server's 24 MiB are @opencode/schema (11.6), @redis/client (5.4) and @opencode/protocol (3.2); cal-diy's 7
  are @trigger.dev/core. Of the never-asked lists with an import type (t3code-server 40.8 MiB, cal-diy 13.5), the rest
  have another disqualifier too or sit inside a list that is forced.
- (c) effect (3.5-4.1 MiB), kysely, @aws-sdk.
- Keeping nested lists lazy when a member list is forced: 0.2-4.5 MiB (supabase-studio 4.5).

**Why each kind was left eager** (what a design would have to do, for the record):

- (b) The file's import list is collected right after its parse (`collect_external_module_references`, parser
  references.rs): with `PossiblyContainsDynamicImport` set, `for_each_dynamic_import_or_require_call` finds every
  `import` in the text with a regex and descends the tree to the node there; a pending lazy list reads as empty, so
  its import types would be missed and their modules never loaded. Finding the positions in the skipped text is cheap
  (the scan already does it), but the entries of `file.imports` are the specifier nodes themselves, and the file
  loader resolves every one before any checker exists, taking the resolution mode from the `resolution-mode`
  attribute of the specifier's `ImportType` parent (`get_mode_for_usage_location`); include reasons and their
  diagnostics point at them, and the checker's type printer compares them. A pre-scan could record text and mode, but
  `file.imports` would then hold nodes that are not the tree's (a different node, no parent until forced), and forcing
  these lists at load time saves nothing.
- (a) Binding a body changes state outside it that must exist before any checker: the owner symbol's flags
  (`declare_module_symbol` runs `get_module_instance_state` over the body: value or namespace module, const-enum-only;
  computable at parse time except for `export { x }`, whose alias target the walk looks up in the enclosing
  statement lists, not yet parsed when the body ends), the declaration's `ExportContext` flag (`set_export_context_flag`:
  whether the body has an export declaration or `export =`; parse-time), the two tables the body fills (the
  namespace symbol's `exports`, and the declaration's `locals` for what is not exported, as in googleapis's
  `export {};` bodies: reads of either would have to force), and, inside ambient module bodies, the program's module
  references (imports and nested `module "x"` / `global` blocks go into `file.imports` and `module_augmentations`, the
  problem of (b)). Pattern ambient modules and `ambient_module_names` come from the names, not the bodies. A module
  declaration is not a flow container, so the flow graph runs through the body; ambient bodies make no flow nodes
  unless they have initializers or expressions (the flow disqualifier). Same-file merges of namespaces would force at
  bind time, as for interfaces; the census does not count those as asked, so it overstates (a) slightly.
- (c) `@see` / `@link` comments are parsed with the file (`with_jsdoc_worker`) because `check_source_element_worker`
  reads them (`eager_jsdoc`) to mark `@link` targets referenced for the unused-identifier checks of checked files; the
  CLI's check never reads them in an unchecked declaration file. A forced list would have to add its members' JSDoc to the
  file's `jsdoc_cache`, which lazy JSDoc already guards (`jsdoc_mu`, `resolve_jsdoc`): feasible, but worth 2-7 MiB.

## Reproduce

```sh
git checkout mem/lazy-dts-census    # the census tool
CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile
cd <project> && TSRS_LAZY_DTS_CENSUS=1 TSRS_LAZY_DTS_CENSUS_TSV=/tmp/files.tsv <worktree>/target/prof/release/tsrs -p <tsconfig> \
  --noEmit --incremental false --extendedDiagnostics --checkers 16
TSRS_LAZY_DTS=stats tsrs -p <tsconfig> --noEmit ...      # lists made lazy / deferred / parsed again
TSRS_LAZY_DTS=force tsrs-test run --suite all --baselines types,symbols   # every declaration file lazy; trees must equal main's
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref <branch> --input script=tools/perf/lazydtsprobe.sh \
  --input projects=drizzle-orm,formbricks-web,cal-diy --input probe_args='--reps 10 --checkers 32 --no-strace'

# Round 2 (section 7): the census with module blocks, disqualifier reasons and the design scenarios
git checkout mem/lazy-dts-census-2
CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile
cd <project> && TSRS_LAZY_DTS_CENSUS=1 TSRS_LAZY_DTS_CENSUS_TSV=/tmp/files.tsv <worktree>/target/prof/release/tsrs -p <tsconfig> \
  --noEmit --incremental false --extendedDiagnostics --checkers 8     # report on stderr, before the alloc profile
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref <branch> --input runner=depot-ubuntu-24.04-16 \
  --input script=tools/perf/lazydtsprobe.sh --input projects=formbricks-web,cal-diy,t3code-server,supabase-studio,mikro-orm \
  --input probe_args='--reps 3 --checkers 8 --no-strace --variant off:TSRS_LAZY_DTS=0'
```
