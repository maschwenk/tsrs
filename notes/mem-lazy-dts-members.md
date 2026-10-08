# mem-lazy-dts-members: parse and bind the member lists of unchecked declaration files on first use

On the projects where `bun check` beats tsrs on memory, most of the program is `node_modules` declaration files that
are parsed and bound in full and never type-checked (`skipLibCheck`). The checker resolves declarations lazily
(notes/lazy-members.md), so of an interface, class or type literal in such a file it only needs the members some
checker looks up. This change parses those member lists as usual, throws the nodes away when nothing in them reaches
outside the list, and parses and binds them again the first time anything asks for them (V8-style lazy parsing,
applied to member lists, with lazy binding of the same lists). It is a front-end change: the saving is the same at
every checker count, and parse and bind get a little faster. Output is byte-identical.

Result, Linux 64 vCPUs at the default 32 checkers: formbricks-web -6.0% peak RSS against main (2.82 -> 2.65 GiB, no
wall change), cal-diy and supabase-studio about -4% / -3%, t3code-server and vscode under 2% (TBD final table). On the
Mac at 16 checkers -9.6% / -2.2% / -3.6% / -1.7% / -1.5%; with `--noCheck` -26% / -15% / -13% / -10% / -2%.

## 1. Ceiling (census)

Tool: the alloc-profile build with `TSRS_LAZY_DTS_CENSUS=1` (`tsrs_core::lazydts_census`, `tsrs_compiler`
lazydts_census.rs). The parser and the binder record, per member of every interface, class, type literal and module
block of a declaration file, the arena bytes it took plus the thread's net heap bytes (binder symbol tables keep their
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
- **Forcing** (`LazyNodeList::force_nodes`). `NodeList::nodes` (the only accessor of a list's slice) checks the
  long-form bit it already tested; a lazy list goes to the record. `Symbol::members` and, for class lists,
  `Symbol::exports` check the tail's lazy field. The first reader parses the list again from `pos` with the recorded
  context (a fresh parser on the file's text, nested lists eager), sets the members' parent to the owner, binds them
  with a fresh binder seeded with the recorded state (containers the members add to the container chain are spliced
  in where the list was), publishes the slice (Release) and wakes waiters. Other threads wait on a condvar; the
  forcing thread's own reads during binding (re-entrant) get the list being bound. Allocation goes to the forcing
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

TBD

## 5. Gates

TBD

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

## Reproduce

```sh
CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile
cd <project> && TSRS_LAZY_DTS_CENSUS=1 TSRS_LAZY_DTS_CENSUS_TSV=/tmp/files.tsv <worktree>/target/prof/release/tsrs -p <tsconfig> \
  --noEmit --incremental false --extendedDiagnostics --checkers 16
TSRS_LAZY_DTS=stats tsrs -p <tsconfig> --noEmit ...      # lists made lazy / deferred / parsed again
TSRS_LAZY_DTS=force tsrs-test run --suite all --baselines types,symbols   # every declaration file lazy; trees must equal main's
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref <branch> --input script=tools/perf/lazydtsprobe.sh \
  --input projects=formbricks-web,cal-diy,supabase-studio,t3code-server,vscode --input probe_args='--reps 7 --checkers 1,4,16,32 --no-strace'
```
