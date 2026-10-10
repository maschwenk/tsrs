# exp-arena-probe: the no-malloc ceiling for checker-thread heap allocations

Question: if every heap allocation a checker thread makes went into that thread's arena (a bump, or a block from the
arena's size-class free lists) instead of mimalloc, how many instructions and how much memory would change? That
bounds what the remaining "Go-style" containers (`Vec`, `FxHashMap`, `String` and `Box` fields of arena objects and of
the `Checker`, notes/perf-heap-churn.md "What is left") could gain by moving into the arena, before any of them is
rewritten. Branch `exp/arena-probe` on top of `perf/heap-churn` (580ba230, vscode at 22.3M heap allocations).

Result: no instruction win is available from the allocation source. With the probe's own dispatch held constant, the
arena path (bump + free lists) costs the same as mimalloc's fast path to within noise (-0.12% to +0.08% on the five
projects, minimum-based; medians -0.4% to +0.1% on four of them). Routing everything (`all`) is +0.2% to +0.5% slower
than routing nothing and costs +27% to +62% peak memory single-threaded and +30% to +97% at 16 checkers, because
blocks above 512 B have no free class and every grown-out table or vector leaks. Routing only the sizes the free
lists recycle (`512`) is memory-neutral to slightly worse (+0.35% to +1.74% single-threaded, -0.3% to +1.0% at 16
checkers): the arena holds freed blocks per exact class where mimalloc reuses pages across classes. Output
byte-identical in every run (single-threaded stdout and exit, default-mode diagnostics, poisoned, 28/28 regression
cases). The probe stays behind a feature (`arena-probe`) and changes nothing in the shipped binary.

## The probe

`--features arena-probe` on `tsrs_cli` (`crates/tsrs_cli/Cargo.toml`, `crates/tsrs_core/Cargo.toml`) installs
`tsrs_core::arena_probe::ArenaRouting` as the global allocator in place of mimalloc (`crates/tsrs_cli/src/main.rs`,
`crates/tsrs_core/src/arena_probe.rs`). While a thread is in arena mode it serves every allocation of at most
`TSRS_ARENA_PROBE_MAX` bytes (default 512; `all` for everything) from the thread's current arena: a block of the
size's free class (`arena::free_class`, sizes rounded to 8) if the list has one, else a bump (`Arena::alloc_layout`).
A `dealloc` of a block in the pointer reservation (`reserve::BASE_ADDR`, so an arena block) goes back to the free
list when the current arena's current chunk holds it and the size has a class (`Arena::owns_current_chunk`, new,
O(1)); otherwise the block leaks (a retired chunk, a size above 512 B, or a block freed on another thread). A block
born on mimalloc stays there, `realloc` included. Arena mode is a flag on the thread's current arena
(`Arena::probe`), not a thread-local of its own: on macOS every `thread_local!` is a `tlv_get_addr` call, and
`arena::CURRENT` is the one lookup a real arena-backed container would pay too. Counters per arena, summed at exit
(`arena-probe: <name> <value>` on stderr): `routed_allocs`, `routed_bytes` (after rounding), `recycled_frees`,
`leaked_frees`, `leaked_bytes`, `heap_allocs` (allocations above the limit, sent to mimalloc while in arena mode).

What flips arena mode (`tsrs_core::arena_probe::enter_arena_mode`, a guard):

| site | covers |
| --- | --- |
| `checkerpool.rs` `run_work_group`, single-threaded branch (the `--singleThreaded` checker; the calling thread, mode scoped to the group) | the three groups below, on the main thread |
| `checkerpool.rs` `run_work_group`, the `checker-N` threads (guard dropped before `release_own_arena`, so the arena the next pass inherits is out of arena mode) | the same, one thread per checker |
| group 1: checker creation (`checkerpool.rs:716`, `SlotChecker::new`) | the `Checker`'s own tables and link stores |
| group 2: `for_each_checker_parallel` (`checkerpool.rs:780`; callers `program.rs:886`, `:2223-2251`, `checkerpool.rs:787`, `checkerpool_stats.rs:105,241`) | global diagnostics and the stats passes |
| group 3: the type-check pass (`checkerpool.rs:977`), stealing and split files included | checking, and the lazy `.d.ts` member lists reparsed on first use on the checker thread (`parser_1.rs:2981` `reparse_lazy_list`, hook set at `:2974`) |

Not covered, by design: program construction, parsing and binding (the rayon workers, `program.rs`; the parse pool
stays separate even with `--singleThreaded`, so the main thread never parses), the leaf classification thread
(`checkerpool.rs:728`), `Lazy lists: shared` (`fileregions.rs:171`, on the worker pool before the checkers), the
diagnostics sort and report, emit. In the single-threaded runs below the main thread leaves arena mode between
groups.

`SymbolMap`'s `EntryVec` (`tsrs_ast/src/symbol.rs:308-411`) calls `std::alloc::{alloc, realloc, dealloc}` directly;
those go through the global allocator, so a symbol table a checker creates is routed too (one 8-byte-entry buffer
per table, grown 4, 8, +50%), while a binder table (born on a parse thread) stays on mimalloc when the checker grows
it. A `realloc` of an arena-born block is always allocate, copy, free; mimalloc grows a block in place when the new
size fits its class.

Interaction with parser checkpoints: `free_block` bumps the arena's epoch (`arena.rs:790`), and `rewindable`
(`arena.rs:513-515`) refuses a rewind after any epoch change. A lazy member list reparsed on a checker thread runs the
parser there, and a parser `Vec` that grows and frees its old buffer (for example `lazy_lists`, `parser_1.rs:905`)
is now a recycled free between a nested `parse_member_list_lazily` checkpoint (`parser_1.rs:887`) and its
`arena_rewindable` test (`:896`), so a nested type-literal list that base would make lazy can come back eager, and a
`mark`/`rewind` speculation inside the reparse (`parser_1.rs:534,550`) keeps its nodes instead of rewinding them. The
probe does not count these. They do not change the work the counters see: single-threaded `--extendedDiagnostics`
runs of base, `512` and `all` on vscode and xstate-main print the same `Symbols`, `Types`, `Instantiations` and every
`Lazy *` counter, and stdout is byte-identical. The memory they keep is inside the leaked bytes below (a few nodes per
nested list; not measured separately).

## Method

- Binaries: base = clean 580ba230, probe = the same tree with `--features arena-probe`; both
  `cargo build --release --locked -p tsrs_cli` (fat LTO, no PGO: relative comparisons only, as pr-verify does). One
  probe binary, three settings: `TSRS_ARENA_PROBE_MAX=512` ("512"), `=all` ("all"), and `=1` ("p1": routes nothing but
  size-1 allocations, 0.3M on vscode, so it measures the probe's own dispatch, the `CURRENT` lookup, three flag loads
  and one counter per allocation, with mimalloc still doing the work).
- Instructions: `/usr/bin/time -l` "instructions retired", `-p <project> --noEmit --incremental false
  --singleThreaded --pretty false`, `RAYON_NUM_THREADS=1`, the four settings interleaved with the order rotated per
  rep, 3 reps per round. Two rounds: the first without a warm-up (its first base run of every project ran on a cold
  page cache and came out 1-8% high, so medians over the pooled 6 samples are quoted, and minima as the check that
  does not depend on it), the second with a discarded warm-up run and the `p1` setting. Peak RSS from the same runs
  and from a default-mode run (16 checkers on this machine, `--extendedDiagnostics`) per rep.
- Machine: an M-series Mac, 18 cores, shared with other agents the whole time (load average 6-11). The count is not
  exact run to run: xstate-main repeats to 0.1-0.3%, the larger projects to 1-2.5%. Wall time was not recorded on
  purpose. "instructions retired" here includes kernel time; never compare with Linux `bench/count.py` numbers.
- Identity: single-threaded stdout and exit compared byte for byte per rep (no `--extendedDiagnostics` in that run,
  so no volatile lines); default-mode `error TS` lines and exit compared per rep; one poisoned run (below).
- Raw outputs: `/tmp/tsrs-oxc/probe/` on the machine (`runs/`, `runs2/`, `post/`); the scripts `ab3.sh`, `ab4.sh`
  (`ab3.sh` + warm-up + `p1`), `post.sh`, `post2.sh` next to them.

## Instructions

Medians of the pooled 6 samples (`p1`: 3), spread = (max - min) / median; a delta counts only when it is at least
twice the larger spread, which on this machine holds for xstate-main and webpack (`all`) and is borderline elsewhere.

| project | base G | 512 | all | p1 (dispatch only) | spread base / 512 / all / p1 |
| --- | ---: | ---: | ---: | ---: | --- |
| xstate-main | 7.009 | +2.48% | +2.72% | +2.55% | 7.7 (cold run; 0.17 in round 2) / 2.4 / 0.29 / 0.10 |
| webpack | 12.986 | +1.61% | +1.92% | +2.14% | 3.3 (0.21) / 1.7 / 0.91 / 1.4 |
| cal-diy | 38.201 | +1.85% | +2.28% | +1.24% | 6.3 (3.2) / 2.5 / 2.2 / 2.5 |
| t3code-server | 46.209 | +1.52% | +1.87% | +1.55% | 1.7 (1.6) / 2.0 / 0.67 / 0.72 |
| vscode | 96.098 | +0.58% | +1.00% | +0.77% | 2.1 (0.72) / 1.4 / 1.1 / 1.0 |

Minima instead of medians (round 1 and 2 pooled; a perturbation only adds instructions, so the minimum is the
steadiest statistic on a loaded machine): xstate-main +2.33 / +2.58 / +2.54%, webpack +1.47 / +1.85 / +1.82%,
cal-diy +2.05 / +2.46 / +1.98%, t3code-server +2.08 / +2.40 / +2.18%, vscode +1.40 / +1.66 / +1.44% (512 / all / p1).

The whole cost is the dispatch. `p1` routes nothing and costs as much as `512`: about 35-50 instructions per
checker-thread allocation (xstate-main: 0.18 G over 2.4M allocations; vscode: 0.74 G over 15.4M), which is the
thread-local lookup of `arena::CURRENT` (a `tlv_get_addr` call on macOS), the flag and limit loads, the counter, and
one more call layer in front of `mi_malloc`. What is left after subtracting it is the arena path against mimalloc's,
round 2 only (the round with `p1`):

| project | 512 - p1 (median) | all - p1 (median) | 512 - p1 (min) | all - p1 (min) |
| --- | ---: | ---: | ---: | ---: |
| xstate-main | -0.08% | +0.18% | -0.12% | +0.20% |
| webpack | -0.41% | -0.15% | -0.22% | +0.03% |
| cal-diy | +2.37% | +0.63% | +0.08% | +0.47% |
| t3code-server | -0.08% | +0.37% | -0.10% | +0.26% |
| vscode | +0.00% | +0.40% | -0.04% | +0.21% |

(cal-diy's +2.37% median is one slow `512` rep of three; its minimum agrees with the others.) So a bump or a free-list
pop plus a free-list push, reached through the thread-local, costs what `mi_malloc` plus `mi_free` cost, and routing
the large blocks too (`all`: no reuse, more chunks, every `realloc` a copy, `alloc_zeroed` written by hand where
mimalloc hands out fresh zero pages) is slightly slower than leaving them on mimalloc.

## Memory

Peak RSS, medians of the pooled samples (single-threaded RSS repeats exactly; 16-checker RSS to 0.3-3%):

| project | base single MiB | 512 | all | p1 | base 16 checkers MiB | 512 | all | p1 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| xstate-main | 179.4 | +1.48% | +26.8% | +0.64% | 259.9 | +0.28% | +30.3% | +0.08% |
| webpack | 310.1 | +0.77% | +36.1% | +0.39% | 455.4 | -0.27% | +38.9% | +1.07% |
| cal-diy | 773.4 | +0.35% | +61.9% | +0.17% | 1511.2 | +0.17% | +96.9% | -0.06% |
| t3code-server | 733.6 | +1.74% | +40.7% | +0.17% | 1841.6 | +1.03% | +63.5% | +0.31% |
| vscode | 1603.7 | +1.22% | +34.1% | +0.08% | 1976.4 | -0.09% | +38.3% | -0.32% |

Where it moves (`TSRS_MEM_SPLIT=1` at `check end`, single-threaded):

| project / setting | arena used MiB | heap live MiB | heap resident MiB | arena + heap resident |
| --- | ---: | ---: | ---: | ---: |
| vscode base | 1071.7 | 420.9 | 433.1 | 1504.8 |
| vscode 512 | 1133.1 (+61.4) | 355.8 (-65.1) | 390.1 (-43.0) | 1523.2 (+18.4) |
| vscode all | 1812.1 (+740.4) | 188.6 (-232.3) | 236.1 (-197.0) | 2048.2 (+543.4) |
| xstate-main base | 97.3 | 46.4 | 47.2 | 144.5 |
| xstate-main 512 | 105.6 (+8.3) | 37.5 (-8.9) | 41.6 (-5.6) | 147.2 (+2.7) |
| xstate-main all | 184.3 (+87.0) | 20.4 (-26.0) | 24.2 (-23.0) | 208.5 (+64.0) |

With `512` the live bytes that move (65 MiB on vscode) take about the same room in the arena (61 MiB: 8-byte rounding
against mimalloc's classes is a wash), but mimalloc gives back 43 MiB of pages where the arena grows 61: a freed
block waits on its exact class's list and is never used for another size or returned, which is the +1.2% single-
threaded. With `all` the leaked bytes (below) are the growth: 528 MiB leaked on vscode of a 740 MiB arena increase.

## Counters

Single-threaded, rep 1 (counts repeat to 0.02% between runs). mimalloc's own total for base (`MIMALLOC_SHOW_STATS=1`):
vscode 22.3M allocations, 2.0 GiB allocated, 427.8 MiB peak live; xstate-main 3.0M, 268.9 MiB, 47.2 MiB peak.

| project | setting | routed allocs | routed MiB | recycled frees | leaked frees | leaked MiB | heap allocs (> limit) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| xstate-main | 512 | 2,560,607 | 87.9 | 2,401,933 | 1,804 | 0.13 | 33,991 |
| xstate-main | all | 2,595,785 | 166.5 | 2,401,031 | 35,965 | 63.5 | 0 |
| webpack | 512 | 2,499,582 | 102.4 | 2,351,365 | 3,563 | 0.22 | 71,146 |
| webpack | all | 2,581,175 | 248.3 | 2,350,072 | 78,039 | 114.6 | 0 |
| cal-diy | 512 | 9,233,806 | 320.4 | 8,773,369 | 31,397 | 1.7 | 199,239 |
| cal-diy | all | 9,484,217 | 922.2 | 8,761,358 | 266,453 | 493.0 | 0 |
| t3code-server | 512 | 14,420,074 | 507.3 | 13,652,944 | 22,040 | 1.0 | 216,601 |
| t3code-server | all | 14,642,925 | 860.1 | 13,649,463 | 244,304 | 284.8 | 0 |
| vscode | 512 | 17,058,414 | 553.3 | 16,019,714 | 18,790 | 1.3 | 337,638 |
| vscode | all | 17,410,011 | 1231.8 | 16,013,322 | 354,054 | 528.0 | 0 |
| vscode, 16 checkers | 512 | 21,013,996 | 715.0 | 19,754,899 | 18,184 | 1.4 | 498,469 |
| vscode, 16 checkers | all | 21,585,648 | 1744.9 | 19,776,832 | 518,490 | 799.2 | 0 |

Reading it: 76-78% of a vscode check's heap allocations are made on the checker thread (17.1-17.4M of 22.3M; the
rest is parse, bind and program construction). 92-94% of the routed blocks come back through the free lists; of what
does not, under `512` almost all is blocks in retired chunks (18,790 on vscode, 1.3 MiB), under `all` it is the
354K blocks above 512 B (1.5 KiB on average: hash-table doublings, vector growth), 528 MiB. The difference between
`all`'s and `p1`'s totals (17.4M routed against 15.1M `heap_allocs` + 0.3M routed) is the ~2.0M `realloc`s a vscode
check makes on checker threads: each a copy in the arena.

## Exactness

- Single-threaded stdout and exit identical to base in all 105 measured runs (5 projects, 21 per project across the
  two rounds), default-mode diagnostics and exit identical in all 105.
- `tools/regressions.sh` with the probe binary, `TSRS_ARENA_PROBE_MAX=all` and `=512`: 28/28, once the six
  `arena-probe:` stderr lines are filtered (the script compares `2>&1`; base 28/28 through the unmodified script).
- Poisoned run (`TSRS_ARENA_POISON=1 TSRS_ARENA_PROBE_MAX=all`, vscode, `--checkers 16`): diagnostics and exit (2)
  equal to base's default-mode run.
- `cargo check` with `-D warnings` with and without the feature, `tools/lint/ratchet.py` (none new),
  `tools/lint/source.py`.

## Caveats

- macOS only. On Linux with compressed pointers thread-arena chunks from the second one on are whole 2 MiB
  transparent huge pages (`arena.rs:36-43`, `reserve.rs:27`) while the CLI's mimalloc is built `no_thp`
  (notes/mem-no-thp.md), so bytes moved from the heap into the arena land on huge pages there: the `all` RSS
  penalty would round up further and the `512` delta could move either way. The instruction result is a per-call
  path cost and should transfer; the Linux count (`bench/count.py`, exact) was not run.
- The probe over-counts a real arena-backed container by its dispatch (the `p1` row); it under-counts nothing. A
  container that received the arena by reference instead of looking up `arena::CURRENT` would save the thread-local
  call, which is a design choice available to a rewrite but not measured here.
- Release build without PGO on a loaded machine: the large projects repeat only to 1-2.5%, so for them the `512`/`all`
  against `p1` differences are bounded (within about +-0.5%) rather than measured. xstate-main and webpack carry the
  conclusion.
- `all` never frees a block above 512 B and copies on every `realloc`; it is the memory floor and the speed ceiling of
  "bump everything", not a design.

## What it means for the design

The ceiling for changing where a container's bytes live, with the container's behaviour unchanged, is zero
instructions: the free-list path and mimalloc's fast path cost the same, and the only savings on offer come from
not allocating. For the shortlist of candidate arena-backed containers (the inventory ranked 15 from the heap
profile), by what each actually removes:

| candidate | what it changes | instruction ceiling from this note | memory |
| --- | --- | --- | --- |
| `SymbolTable` `EntryVec` buffer in the arena (3.7M alloc + realloc on vscode) | allocation source only; the same growth sequence | ~0 (512 - p1); the buffer's `realloc`s become copies | neutral: already inside `512`'s +0.35..+1.74% single, -0.3..+1.0% at 16 checkers |
| `LazyVec` candidate lists, small instantiation tables (`PackedMap`), `LazyMappedTable` `Rc`, `InferMemo` entries, link-store chunks, diagnostic argument strings | allocation source only | ~0 each; counts of 0.02-1.8M are below the 9M pairs 1% needs even at 100 instructions a pair | neutral to worse (the `512` rows); the per-block saving mimalloc's headerless classes leave is the 8-byte rounding |
| per-call scratch stack for `Vec<P<Type>>` / `Vec<P<Symbol>>` temporaries | removes the pair and its dispatch (one mark/truncate per function instead) | the diffuse remainder after perf/heap-churn is 3-5M on vscode: 0.3-0.5% at ~100 instructions a pair (the rate notes/perf-heap-churn.md's 19M removed for -2.15% implies); 1% needs ~27-32% of all remaining checker-thread allocations on the densest projects (xstate-main 0.37 and t3code-server 0.31 allocations per 1000 instructions, vscode 0.18) | none |
| pooled per-call maps (`somePropertyReducesToNever`, `every_lazy_property`, grammar `seen`) and `flow_type_cache` pooling | removes the table growth pairs | the one measured site is -0.88% on mui-docs and -0.83% on mikro-orm (branch `perf/property-count-pool`); two or three sites together are the only route to 1% in this family | none |
| `&'static str` keys, `Cow` / stored-slice returns, snapshot-free table walks, build-then-copy member resolution | removes allocations and the copies behind them | the copy is the larger half (perf/heap-churn: returning stored data beat inline storage); 1.35M snapshots on vscode are ~0.14% as pairs, more with their copies | none |

So of the 15 candidates only the ones that remove allocation pairs, and chiefly the copies behind them, can still
clear 1% on some project: the scratch stack if it covers about a third of the remaining checker-thread temporaries
on t3code-server or xstate-main (not measured; those two have the most allocations per instruction), the pooled-map
family at two or more sites, and stored-slice returns where a copy goes with the allocation. Moving a container into
the arena without changing its behaviour (the first two rows) should not be attempted for speed.

Memory: the `all` rows answer whether the arena needs free classes above 512 B. They say the opposite: with no reuse
above 512 B the arena is +27% to +62% worse single-threaded and up to +97% at 16 checkers, and adding exact-fit
classes for the 354K large blocks a vscode check frees (hash-table doublings mostly) would be a second mimalloc with
worse page reuse, as the `512` rows already show at small sizes (+18 MiB pinned on vscode where mimalloc returned 43).
Large tables and vectors stay on mimalloc or in Regions; the 5% peak-memory bar is not reachable by moving small
blocks either (the `512` rows: -0.3% to +1.0% at the default checker count). The memory story for the remaining
Go-style containers is therefore not an allocator change but fewer and smaller containers (notes/mem-checker-heap.md
"a different table design"), which this probe does not bound.

Not retried here, by the "Measured and rejected" list: bump regions per scope (notes/mem-scoped-arenas.md), and the
oxc_allocator migration branches (`codex/oxc-allocator-migration`, `codex/rust-arena-ownership`: +75-99% peak RSS and
+4-21% instructions from dropping the handles, free lists and rewinds), whose instruction penalty this note explains:
the arena is not faster than mimalloc per call, so a migration pays its dispatch and copies and earns nothing back.
