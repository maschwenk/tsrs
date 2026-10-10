# design-persisted-frontend: what caching parsed + bound library files would buy

Measurements behind `docs/PERSISTED_FRONTEND.md`. The idea: write the parsed + bound AST of files that do not change
between runs (bundled `lib.*.d.ts`, `node_modules`) to a cache file once and `mmap` it on later runs, so parse + bind
of those files becomes page faults and only the pages the checker reads become resident. Branch
`design/persisted-frontend` (documents only); the instrumentation is on `design/persisted-frontend-probe` (pushed,
not for main). Base: origin/main ff92b7f. Machine: this 18-core Mac (6 performance + 12 efficiency cores, 16 KiB
pages), load average 10-24 during the runs, one 38k-file run at a time.

**Result: the payoff is small.** Library files are 26% of the nodes, not most of them. Their parse + bind is 0.33 s
of CPU on one core, which comes to about 0.02-0.03 s of wall time at 18 threads and about 0.08 s at 4. A full check
touches 73% of their pages, so lazy page-in could keep at most about 0.1-0.15 GiB of a 7 GiB peak out of memory.
Most of the front end's time goes to program construction (file system calls and module resolution), and a cache
of ASTs does not touch that.

## 1. How the corpus splits

The probe's per-file table covers one full run with 4 checkers. A file counts as cacheable if it is a bundled lib, under
`node_modules`, or a workspace `.d.ts` (`packages/*/dist`, content-hash validated like the others). Counts come from
`SourceFile.node_count` / `identifier_count` / `symbol_count`. "Arena + text" is the parse + bind output in the file's
region, with the text copied in (the language server's layout).

| class | files | text MiB | lines | nodes | identifiers | binder symbols | arena + text MiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| bundled libs | 79 | 2.8 | 58,101 | 141,038 | 49,650 | 31,802 | 11.1 |
| `node_modules` | 9,650 (12 of them not `.d.ts`) | 70.3 | 1,748,898 | 4,947,438 | 1,721,212 | 969,519 | 370.3 |
| workspace `.d.ts` | 697 | 3.6 | 88,417 | 534,063 | 207,554 | 86,709 | 32.0 |
| project | 27,516 | 124.4 | 4,046,238 | 15,794,017 | 5,351,036 | 2,694,832 | 1,077.9 |
| total | 37,942 | 201.1 | 5,941,654 | 21,416,556 | 7,329,452 | 3,782,862 | 1,491.2 |
| **cacheable share** | **27.5%** | **38.1%** | **31.9%** | **26.3%** | **27.0%** | **28.8%** | **27.7%** |

The `--extendedDiagnostics` counters (Files, Lines, Identifiers) match these totals. `Symbols` there counts checker
symbols too.

## 2. Parse + bind time

### Front-end wall time by worker count

`--noCheck --incremental false --extendedDiagnostics`, with `RAYON_NUM_THREADS` sizing the parse/bind pool
(`worker_pool()` in `tsrs_compiler::program` is a default rayon builder, so it honours that variable). The rounds
were interleaved; round 1 ran with a cold file cache and is dropped. Values are medians of rounds 2-5, in seconds.

| threads | "Parse time" (all of program construction) | of which parallel parse + resolve | Bind time | Total |
| --- | --- | --- | --- | --- |
| 1 | 2.365 | 2.140 | 0.459 | 3.03 |
| 4 | 0.862 | 0.692 | 0.119 | 1.11 |
| 8 | 0.754 | 0.579 | 0.066 | 0.97 |
| 18 (default) | 0.709 | 0.520 | 0.035 | 0.89 |

### CPU by file class

The probe takes thread CPU time around each file's parse (`host.get_source_file`) and bind. Run: 1 thread, two runs,
mean.

| class | parse CPU s | bind CPU s |
| --- | --- | --- |
| cacheable (libs + `node_modules` + workspace `.d.ts`) | 0.216 | 0.114 |
| project | 0.592 | 0.368 |
| all | 0.808 | 0.482 |

Parse + bind of every file is 1.29 s of the 3.03 s single-threaded front end. The other 1.7 s is the include glob,
file reads, `stat` probes and module resolution (notes/speed-frontend.md: 114k `stat`s, 40k reads, 5.3k `readdir`s,
bound by the kernel past ~8 threads). **The cacheable files' parse + bind is 0.33 s of CPU: 11% of the
single-threaded front end, and less of the parallel one.**

### Upper bound on wall time saved

The estimate assumes a cache hit costs nothing and the parse work scales perfectly. Bind is its measured wall time
times the cacheable share of bind CPU (23.7%). Parse is the cacheable parse CPU divided by the effective
parallelism.

| threads | bind saved | parse saved | total saved | share of the `--noCheck` front end |
| --- | --- | --- | --- | --- |
| 1 | 0.109 | 0.216 | 0.33 s | 11% |
| 4 | 0.028 | 0.054 | 0.08 s | 7% |
| 8 | 0.016 | 0.027 | 0.04 s | 4% |
| 18 | 0.008 | 0.012-0.022 (12 of the 18 cores are efficiency cores) | 0.02-0.03 s | 2-3% |

A cache hit is not free (section 5). It pays about 0.57 µs per 16 KiB page for the pages the checker reads, but saves
the 0.75 µs zero-fill fault the parser pays on fresh arena pages, so those roughly cancel. Hashing or `stat`-validating
10.4k files costs ~0.01 s of CPU. Building live `SourceFile` headers (design, "Hit") costs ~1 µs per file. **Net:
~0.3 s at 1 thread, ~0.06 s at 4, ~0.01-0.02 s at 18.** The cold full run is 6.87 s on this Mac (common brief,
reference), so the 18-thread saving is 0.2-0.3%, inside the ±2% run-to-run noise.

## 3. How much of the library ASTs the rest of the run touches

### Method (`TSRS_PFE_PROBE=1`, probe branch)

1. Every parsed file gets its own region, as in the language server (`tsrs_project::parsecache`), with its text
   copied in. Every region chunk is its own page-aligned anonymous mapping, so each page belongs to exactly one file.
2. Binding enters the file's region (`arena::enter_owner`).
3. Just before the checkers are created (`checkerPool::create_checkers`), every page of every region gets `mprotect
   PROT_NONE`.
4. A SIGSEGV/SIGBUS handler records the page's state and reopens it: read-only after a read, read-write after a
   write. A write is detected from the ESR's WnR bit in the ucontext, so concurrent faults from 4 checker threads
   cannot misclassify a page.
5. The first-touch program counters are kept too, and symbolized with `atos`.

This is the page set a lazily mapped cache would page in (touched) and copy-on-write (written). It is exact at the
hardware page size, 16 KiB here; Linux x86 would see 4 KiB pages.

The use census of notes/mem-use-census.md (field-level read bits) lives on an unmerged branch, instruments the
checker's cells, and only reports checker-phase objects. Page protection needs no accessor changes and measures the
quantity that matters for `mmap`: pages, not fields.

**Not counted:**

- Heap-owned parts of a file: symbol-table entry buffers, `parse_options` strings, the `OnceLock` contents.
- Reads before checker creation: the CLI collects syntactic and bind diagnostics first.

Probe overhead: the run is slower (check 11.4 s vs 7.4 s; ~195k faults), but its output is unchanged. Diagnostics
are identical, and the `--extendedDiagnostics` counters (Symbols 16,549,988, Types 13,788,912, Instantiations
76,888,800) match a normal run.

### Results: pages touched after binding (16 KiB pages over each file's used range)

| class | pages (MiB) | full check, 4 checkers: touched | written | checker creation only (`--noCheck`): touched | written |
| --- | --- | --- | --- | --- | --- |
| bundled libs | 762 (12) | 93.0% | 42.9% | 57.7% | 0 |
| `node_modules` | 29,771 (465) | 73.5% | 16.8% | 35.8% | 0 |
| workspace `.d.ts` | 2,479 (39) | 58.0% | 32.4% | 28.5% | 0 |
| **cacheable** | 33,012 (516) | **72.8%** | **18.5%** | **35.8%** | 0 |
| project | 85,423 (1,335) | 99.9% | 93.5% | 33.1% | 0 |

The `node_modules` row by file size, full check:

| file size (pages) | files | arena + text MiB | touched | written |
| --- | --- | --- | --- | --- |
| 1 | 6,567 | 36.9 | 100% | 11.7% |
| 2-3 | 1,908 | 50.4 | 94.0% | 19.5% |
| 4-15 | 996 | 92.4 | 69.9% | 20.3% |
| 16-63 | 134 | 60.5 | 62.3% | 25.3% |
| 64+ | 33 | 130.1 | 50.1% | 12.3% |

**Every file is touched at least once.**

- The first reader of 37,942 pages is `SourceFile::is_bound` from checker creation (the global merge walks every
  file), so every one-page file counts as touched.
- In large declaration files about half the pages are read.
- Checker creation alone (the `--noCheck` columns, which is also roughly what a language server does before its
  first diagnostics) reads 36% of the cacheable pages.

**Who touches and who writes.** First-touch PCs, full check; counts are pages, across all classes.

Writes:

- `assign_node_id`: 47,264.
- `assign_symbol_id`: 35,470. Lazy ids live in the node header and the symbol, and the checker writes them on the
  first link-store access (`tsrs_ast::utilities_1`).
- `Node::eager_jsdoc`: 2,364. Lazy JSDoc nodes allocated in the owner region, next to the file's data.

Reads:

- `SourceFile::is_bound`: 37,942.
- `Node::eager_jsdoc`: 18,348 (deprecation checks parse JSDoc on demand, including `.d.ts`).
- `for_each_return_statement`: 11,630.
- `is_reachable_flow_node_worker`: 4,531.
- `get_late_bound_symbol`: 2,425.

The writes matter for the design: a `MAP_PRIVATE` page that the checker writes becomes a private dirty copy, i.e.
the same memory it costs today.

### What this bounds

| | cacheable memory today | resident with lazy page-in | saved |
| --- | --- | --- | --- |
| CLI, full check (peak 7.0 GiB) | 0.41 GiB (arena + text, packed) | ~73% of the pages | ≤ 0.11-0.15 GiB (≤ 2%) |
| language server after opening one file (2.4 GiB RSS) | 0.41 GiB | ~36% (checker creation) plus that file's needs | ~0.2-0.25 GiB (~10%) at start, falling toward the CLI figure as more files are checked |

Clean (read, unwritten) mapped pages are also shared between processes that map the same cache: 54% of the cacheable
pages after a full check, ~0.22 GiB per additional process (two editor windows, editor + CLI). That is the one memory
effect that grows with use.

The heap-owned parts not counted above are small: symbol-table entry buffers ~31 + 9 MB for the whole program
(notes/mem-frontend.md).

## 4. Language server cold start

`tools/pfe-probe/lsp_first.py` (probe branch) runs `initialize`, `didOpen` of a 302-line service file of the private
monorepo, then `textDocument/diagnostic`, and times each step. `tools/lsp-mem/lsp_mem.py --edits 0` reports the same
"opened" time. Release build, three interleaved runs each:

| server | time to first diagnostics (s) | RSS at that point |
| --- | --- | --- |
| tsrs | 1.12 / 1.20 / 1.23 (median 1.20) | 2.40-2.41 GiB |
| tsgo-ref | 1.97 / 1.76 / 1.74 (median 1.76) | |
| tsrs, `RAYON_NUM_THREADS=1` | 3.81 / 3.65 | |
| tsrs, `RAYON_NUM_THREADS=4` | 1.65 / 1.45 | |
| tsrs, `RAYON_NUM_THREADS=8` | 1.28 / 1.25 | |

Startup is dominated by building the program (the CLI's `--noCheck` front end is 0.89 s at 18 threads); checking the
opened file adds the rest. Applying section 2's estimate, the AST cache would save ~0.02-0.03 s of 1.20 s at 18
threads, ~0.06-0.08 s of ~1.55 s at 4, and ~0.3 s of ~3.7 s at 1.

## 5. Prototype: page-in cost of a mapped image vs reading it

`tools/pfe-probe/mmapx.c` (probe branch): a 512 MiB file (the size of the cacheable pages) in the page cache. It
compares `read()` into fresh memory, `mmap(MAP_PRIVATE)` plus touching 73% of pages at random (the measured share),
copy-on-write of 23% of the touched pages, touching every page, and zero-fill faults of fresh anonymous memory. Three
repetitions agreed within 10%.

| | 1 thread | 4 threads |
| --- | --- | --- |
| `read()` the whole image | 0.042 s (12 GB/s) | — |
| `mmap` | 0.002 s | 0.002 s |
| touch 73% of pages | 0.014 s (0.57 µs/page) | 0.012 s (no scaling: the faults serialize) |
| copy-on-write 23% of those | 0.013 s | 0.015 s |
| touch all pages | 0.018 s (0.56 µs/page) | 0.016 s |
| anonymous zero-fill, all pages | 0.025 s (0.75 µs/page) | 0.027 s |

Answers:

- Reading versus mapping the image costs about the same on macOS: both are a few tens of milliseconds for 0.5 GiB.
  `mmap` wins on memory (untouched pages are never resident; clean pages are shared) and loses nothing on time.
- Page-in is no more expensive than the zero-fill faults that parsing into the arena pays today.

**Not measured:**

- Linux: no Linux machine was used. The hypothesis is 4 KiB pages, 4x the faults, cheaper each, with fault-around.
- A cold page cache: `purge` needs root. At NVMe speeds, 0.5 GiB is ~0.1 s.

## 6. Other projects in the monorepo

Small packages, each a separate `tsrs -p` (probe, `--composite false --incremental false`). Library parse + bind is
most of their front end, but each run is tens of milliseconds:

| project | cacheable files | cacheable parse + bind CPU | project parse + bind CPU | total wall |
| --- | --- | --- | --- | --- |
| packages/logger | 259 | 0.018 s | 0.000 s | 0.022 s |
| packages/order-amounts | 442 | 0.034 s | 0.002 s | 0.041 s |
| packages/phone | 1,033 | 0.075 s | 0.000 s | 0.061 s |
| packages/olympus-client | 683 | 0.039 s | 0.065 s | 0.502 s |
| apps/vesta | 3,227 | 0.200 s | 0.156 s | 0.994 s (stale-build errors) |

A CI that runs one process per package would repeat the `node_modules` parse in each one. The in-process answer
already exists: the `--build` orchestrator shares `.d.ts` / `.json` `SourceFile`s between projects
(`tsrs_execute/src/build/host.rs` `get_source_file`, Go `execute/build/host.go:55`; the path was
`tsrs_cli/src/build/host.rs` when this note was written).

## Reproducing

```sh
git checkout design/persisted-frontend-probe
CARGO_BUILD_JOBS=8 cargo build --release -p tsrs_cli
cd <the 38k-file codebase>
TSRS_PFE_PROBE=1 TSRS_PFE_PROBE_OUT=/tmp/pfe.tsv tsrs -p . --noEmit --extendedDiagnostics --pretty false --incremental false
# stderr: per-class summary ("pfe:" lines) and first-touch PCs (atos -o target/release/tsrs -arch arm64 <pc>)
RAYON_NUM_THREADS=1 TSRS_PFE_PROBE=1 ... --noCheck   # per-class parse/bind CPU on one core; touches = checker creation only
tools/pfe-probe/threads.sh                           # front end at 1/4/8/18 threads, interleaved
python3 tools/pfe-probe/lsp_first.py tsrs <project> <file>
clang -O2 -o mmapx tools/pfe-probe/mmapx.c && ./mmapx /tmp/img.bin 512 0.73 1
```

The probe is unsafe by design (signal handler, `mprotect` of live memory) and changes the CLI's allocation layout. It
is for measurement only; normal builds with `TSRS_PFE_PROBE` unset behave as before.
