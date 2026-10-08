# mem-lazy-dts-members: parse and bind the member lists of unchecked declaration files on first use

On the projects where `bun check` beats tsrs on memory, most of the program is `node_modules` declaration files that
are parsed and bound in full and never type-checked (`skipLibCheck`). The checker resolves declarations lazily
(notes/lazy-members.md), so of an interface, class or type literal in such a file it only needs the members some
checker looks up. This change parses those member lists as usual, throws the nodes away when nothing in them reaches
outside the list, and parses and binds them again the first time anything asks for them (V8-style lazy parsing,
applied to member lists, with lazy binding of the same lists). It is a front-end change, so the saving is about the
same number of bytes at every checker count. Output is byte-identical. It does not make parsing faster on Linux: a
needed list is parsed twice, and the first parse still runs in full (section 6).

Result, Linux 64 vCPUs at the default 32 checkers (9 interleaved runs, medians): formbricks-web -7.8% peak RSS
against main (2.82 -> 2.60 GiB), cal-diy -2.3%, supabase-studio -2.7%, t3code-server -1.1%, vscode -0.8%, with wall
times within the runner's noise (formbricks-web 0.653 -> 0.627 s, the others +-1%). At 4 checkers -12.7% / -4.1% /
-4.7% / -3.2% / -0.5%, single-threaded -16.2% / -7.3% / -6.5% / -5.2% / -0.6%. On the Mac with `--noCheck` -25% /
-15% / -13% / -10% / -1%, and 0.8-4.4% fewer instructions single-threaded.

## 1. Ceiling (census)

Tool (branch `mem/lazy-dts-census`, not merged): the alloc-profile build with `TSRS_LAZY_DTS_CENSUS=1`
(`tsrs_core::lazydts_census`, `tsrs_compiler` lazydts_census.rs). The parser and the binder record, per member of
every interface, class, type literal and module block of a declaration file, the arena bytes it took plus the thread's net heap bytes (binder symbol tables keep their
entries on the heap); before the first checker is created the compiler registers those lists in the files the program
will not check, with their owner and member symbols. From then on a read of a registered list's nodes
(`NodeList::nodes`), of an owner's `members` / `exports`, and a symbol-table hit or declarations read of a member
symbol set a bit, tagged with the phase (before checker creation, inside `new_checker`, while checking). A list
counts as asked for if its nodes were read or its owner's table was (members for interfaces and type literals,
members or exports for classes, exports for module blocks): exactly what forces a list in the design below. "Never
asked for" counts the outermost such lists (all enclosing candidate lists asked for). One run per project at 16
checkers on the Mac (main 6e07a55 + tool).

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
  in where the list was), publishes the slice (Release) and wakes waiters. Other threads spin for a few tens of
  microseconds (most lists take a few microseconds: formbricks-web forces 9,400 lists in 40-80 ms of thread time),
  then wait on a condvar; the forcing thread's own reads during binding (re-entrant) get the list being bound. Before
  the checkers are created, the lists that every checker's `initialize_checker` forces at once (interfaces and classes
  declared in several script files or `declare global` blocks, which `mergeSymbolTable` clones and merges) are forced
  in parallel on the worker pool (about 1 ms): on cal-diy at 32 checkers the summed waiting went from 100 ms to under
  20 ms and checker creation back from 7 to 4 ms. Allocation goes to the forcing
  thread's own arena (out of any scratch region). Before the file is bound a lazy list reads as empty, which only the
  file's parser (parent pointers, the import scan) and binder can observe.

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

Linux, Depot 64 vCPUs (`tools/perf/lazydtsprobe.sh`: main fcb9803 against this branch at 224d7c4, the branch also with
`TSRS_LAZY_DTS=0`, interleaved, 9 runs per cell, medians). Peak RSS in GiB, wall in s (range of the lazy runs).

| project | checkers | peak main / off / on | on vs main | wall main / off / on | user+sys main / on |
| --- | ---: | --- | ---: | --- | --- |
| formbricks-web | 32 | 2.82 / 2.82 / 2.60 | -7.8% | 0.653 / 0.661 / 0.627 (0.601-0.669) | 12.90 / 12.90 |
| formbricks-web | 16 | 2.20 / 2.19 / 1.98 | -10.0% | 0.691 / 0.693 / 0.701 (0.688-0.730) | 9.20 / 9.27 |
| formbricks-web | 4 | 1.66 / 1.67 / 1.45 | -12.7% | 1.424 / 1.459 / 1.464 (1.442-1.520) | 6.59 / 6.66 |
| formbricks-web | 1 | 1.30 / 1.30 / 1.09 | -16.2% | 4.805 / 4.812 / 4.755 (4.729-5.381) | 4.84 / 4.80 |
| cal-diy | 32 | 2.57 / 2.57 / 2.51 | -2.3% | 0.594 / 0.591 / 0.590 (0.579-0.624) | 12.52 / 12.71 |
| cal-diy | 4 | 1.23 / 1.22 / 1.18 | -4.1% | 1.315 / 1.325 / 1.326 (1.296-1.330) | 5.82 / 5.83 |
| cal-diy | 1 | 0.82 / 0.82 / 0.76 | -7.3% | 3.581 / 3.610 / 3.620 (3.568-4.152) | 3.61 / 3.65 |
| supabase-studio | 32 | 2.24 / 2.23 / 2.18 | -2.7% | 0.569 / 0.571 / 0.570 (0.566-0.577) | 13.42 / 13.54 |
| supabase-studio | 4 | 1.27 / 1.27 / 1.21 | -4.7% | 1.578 / 1.582 / 1.604 (1.586-1.621) | 7.21 / 7.30 |
| supabase-studio | 1 | 0.93 / 0.93 / 0.87 | -6.5% | 5.146 / 5.145 / 5.163 (5.119-5.290) | 5.19 / 5.20 |
| t3code-server | 32 | 2.71 / 2.72 / 2.68 | -1.1% | 1.450 / 1.436 / 1.458 (1.420-1.469) | 18.94 / 19.00 |
| t3code-server | 4 | 1.24 / 1.24 / 1.20 | -3.2% | 1.952 / 1.940 / 1.951 (1.930-1.970) | 8.13 / 8.14 |
| t3code-server | 1 | 0.77 / 0.77 / 0.73 | -5.2% | 4.835 / 4.859 / 4.865 (4.817-4.968) | 4.87 / 4.89 |
| vscode | 32 | 2.62 / 2.61 / 2.60 | -0.8% | 0.604 / 0.610 / 0.609 (0.597-0.639) | 15.54 / 15.81 |
| vscode | 4 | 1.84 / 1.83 / 1.83 | -0.5% | 2.709 / 2.707 / 2.710 (2.675-2.805) | 12.57 / 12.57 |
| vscode | 1 | 1.59 / 1.59 / 1.58 | -0.6% | 11.933 / 11.980 / 11.959 (11.849-12.119) | 12.04 / 12.06 |

Parse time (`Parse time`, which includes binding) on Linux single-threaded is 1-4% longer than with the mode off
(formbricks-web 0.883 -> 0.905 s, cal-diy 0.530 -> 0.539, t3code-server 0.382 -> 0.390; the walk for disqualifiers
and the rewinds cost more than the skipped binding saves where the arena has huge pages), and the same at 32 checkers
(0.100 s on formbricks-web). The runner's instruction counter returns nothing (`instructions:u` reads 58), so
instructions come from the Mac. An earlier run of the branch before the size limit, the spin and the global-merge
forcing had cal-diy +2-6% wall at 32 checkers (waiting on forced lists); this run has none.

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
- Record overhead: 96 bytes per lazy list (formbricks-web 117K lists, 11 MB, 0.4% of peak); the record could shrink
  by about a third.
- Forcing re-parses: a list that is needed is parsed twice (once to establish it is error-free and plain). A skipping
  scanner (brace matching) would be cheaper but cannot know whether the list has syntax errors, which the CLI reports
  for every file before checking; caching that would need a persisted front end (rejected, #36).
- `--incremental`, `--build`, watch, the language server and the API keep eager lists: nothing measured there, and the
  builder, the project system and the API read trees in ways not audited here.

## Reproduce

```sh
git checkout mem/lazy-dts-census    # the census tool
CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile
cd <project> && TSRS_LAZY_DTS_CENSUS=1 TSRS_LAZY_DTS_CENSUS_TSV=/tmp/files.tsv <worktree>/target/prof/release/tsrs -p <tsconfig> \
  --noEmit --incremental false --extendedDiagnostics --checkers 16
TSRS_LAZY_DTS=stats tsrs -p <tsconfig> --noEmit ...      # lists made lazy / deferred / parsed again
TSRS_LAZY_DTS=force tsrs-test run --suite all --baselines types,symbols   # every declaration file lazy; trees must equal main's
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref <branch> --input script=tools/perf/lazydtsprobe.sh \
  --input projects=formbricks-web,cal-diy,supabase-studio,t3code-server,vscode --input probe_args='--reps 7 --checkers 1,4,16,32 --no-strace'
```
