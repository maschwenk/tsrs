# mem-round4: peak memory against `bun check` at the defaults (2026-10-07/08)

Goal set by the owner: lower tsrs's peak memory where `bun check` still beats it, without losing speed. Scoreboard: the
README table at each tool's default on the 64-vCPU Depot runner (tsrs 32 checkers, bun 64 threads). At the start
(bench of f7eaf6a) tsrs used more memory than bun on five projects: t3code-server 2.79 vs 1.56 GiB (0.56x),
formbricks-web 2.88 vs 2.20 (0.76x), cal-diy 2.57 vs 2.02 (0.79x), drizzle-orm 1.15 vs 1.01 (0.88x), supabase-studio
2.29 vs 2.12 (0.93x); it was faster than bun on all seventeen. The round ran four Opus agents and the coordinator's own
measurements; every number below is from the Depot 64-vCPU runner unless it says Mac.

Status (2026-10-10): the scoreboard framing above is dated. The README bench now runs on the 16-vCPU Depot runner
(`depot-ubuntu-24.04-16`) with tsrs at its default 8 checkers and bun at 16 threads (README.md;
`default_checkers_for_parallelism` in crates/tsrs_compiler/src/checkerpool.rs gives 8 on 16 cores and still 32 on 64).
The 64-vCPU figures in this note, including the section 5 table and its projection, are a record of that board; the
16-vCPU gaps are in notes/mem-compact-ast-sizing.md section 1. This note has six sections; the "section 7" that
notes/design-shared-type-layer.md cites is on branch `notes/discarded-work` (81b504bc), not on main.

## 1. Where the gap is

- Single-threaded, tsrs is within ~9% of bun (t3code 871 vs 802 MiB). The whole gap is what each extra checker holds.
- Per extra checker tsrs adds 21-47 MiB on the app projects (t3code 8 -> 16 -> 32 -> 64 checkers: 1.84, 2.22, 2.80,
  3.47 GiB); bun adds 6-15 MiB per thread (1.06, 1.11, 1.33, 1.60 GiB). bun shares one type graph between its
  threads and keeps only task-local scratch per thread (notes/bun-check-memory.md, notes/mem-per-checker-duplication.md
  section 6); a tsrs checker rebuilds the library graph its files touch. [bench-compare 4gv48c0n96, 10 reps]
- What a checker rebuilds is used: the use census on t3code, cal-diy and formbricks finds only 2-5% of the new types
  never read; the never-read 11-14% of allocations is small symbols, links and name lists in many pools, and removing
  all of it exactly would be 6-9% of peak (notes/mem-never-read-apps.md).
- Garbage (unreachable, never freed) is 4.4-7.0% of what an extra checker holds, the largest single pool 3.7%
  (`export *` tables on formbricks); free lists hold <= 6 KB at any time; hash tables are already at hashbrown's
  minimum sizes (notes/mem-checker-scratch.md).
- Linux residency slack at 32 checkers: arena blocks resident but unused 2.3-5.8% of peak (fixed, #199), mimalloc's
  retained heap 4.7-8.2% (partially free pages; a forced purge at the peak finds 4-5 MiB), stacks 4-11 MiB
  (notes/mem-linux-residency-32.md).
- The front end: node_modules declaration files are 83% of formbricks' program text (190 of 228 MiB), 58-65% on
  cal-diy, supabase and drizzle, 47% on t3code, 7% on vscode; they are parsed and bound in full and never checked.
  Lazy member lists for those files (#202, `TSRS_LAZY_DTS`): the parser keeps a record of an interface, class or type
  literal member list in an unchecked declaration file and rewinds the arena; the first reader parses and binds it
  again. Ceiling (bytes never asked for): formbricks 254 MB, supabase 74, cal-diy 69, t3code 45, vscode 12. Measured at 32
  checkers against main e80e776 (10 reps): formbricks -7.1% (wall -0.1%), xstate-main -2.2%, drizzle-orm -1.8%,
  supabase -1.8%, cal-diy -1.6%, t3code -1.5%, vscode and webpack 0; at 4 checkers formbricks -12.0%, cal-diy -4.9%,
  supabase -4.7%, drizzle -3.7%, xstate -3.6%, t3code -3.2%; single-threaded instructions -0.8% to -4.4% (less
  binding). Exact: suite trees identical in the default mode, with `TSRS_LAZY_MEMBERS=0`, with every list forced lazy
  (`TSRS_LAZY_DTS=force`) and forced multi-threaded; diagnostics identical in 48 corpus cells; poison clean;
  pr-verify 102/102. The first version cost drizzle-orm +9% wall at 32 checkers: 2,439 waits on 1,062 lists, 1,871 of
  them on `@types/node` (three copies, each via a package's `/// <reference types="node" />`) and 330 on `bun-types`.
  The rule that removed it: before a multi-checker pass, the lazy lists of the global libraries (default libs, `lib`
  and `/// <reference lib>`, automatic type directives, `types` and `/// <reference types>`, plus their `/// <reference
  path>` closure) are forced in parallel on the worker pool; module-scoped packages stay lazy. That costs 1 ms and
  nothing measurable in memory (formbricks -7.8% -> -7.1% against a different main build), and leaves 97 waits. What
  remains on drizzle is +0.3% (10 reps) to +1.9% (20 reps, paired +2.5%, IQR -0.6..+3.8) on a 0.18 s run: the lists
  the checkers need are parsed twice. nuxt +1.5% in one 20-rep run; everything else within +-1%.

## 2. What landed

| PR | change | Linux, 32 checkers | wall |
| --- | --- | --- | --- |
| #199 | a finished thread hands its arena to the next thread (97 -> 33 arenas); a finished checker trims the resident, never-used end of its last 2 MiB block | drizzle -6.4%, t3code -2.0%, formbricks -2.1%, cal-diy -2.0%, supabase -1.9%, vscode -1.4%, small programs -5..-16% | paired -1.4..+0.3% (noise) |
| #204 | `DiagnosticsCollection` keys diagnostics without cloning the file's `Path` (an `Arc` every checker incremented) | | drizzle-orm -4% wall at 32 checkers, cal-diy and vscode -2..-4% (agent's measurement) |
| #202 | lazy member lists of unchecked declaration files (`TSRS_LAZY_DTS`), global libraries pre-forced in parallel | formbricks -7.1%, xstate -2.2%, drizzle -1.8%, supabase -1.8%, cal-diy -1.6%, t3code -1.5% | within +-1% except drizzle +0.3..+1.9% and nuxt +1.5% (one run) |
| #197 | census decodes array mappers' type lists again (half of the "garbage" was misread); free-list statistics | tool | |
| #198, #201 | notes | | |

## 3. The checker count is the lever, and no rule can set it

Measured scaling on the 64-vCPU runner (release build, 5 reps; wall s / peak MiB at 8 / 16 / 24 / 32 checkers):

| project | 8 | 16 | 24 | 32 | 16 -> 32 |
| --- | --- | --- | --- | --- | --- |
| t3code-server | 2.02 / 1,886 | 1.64 / 2,295 | 1.51 / 2,570 | 1.54 / 2,845 | -6% wall, +24% memory |
| formbricks-web | 0.91 / 1,966 | 0.71 / 2,282 | 0.68 / 2,643 | 0.67 / 2,938 | -6%, +29% |
| cal-diy | 0.89 / 1,539 | 0.72 / 2,025 | 0.65 / 2,396 | 0.62 / 2,697 | -14%, +33% |
| supabase-studio | 1.02 / 1,514 | 0.74 / 1,832 | 0.65 / 2,127 | 0.57 / 2,340 | -23%, +28% |
| drizzle-orm | 0.25 / 642 | 0.21 / 895 | 0.19 / 1,021 | 0.19 / 1,198 | -14%, +34% |
| vscode | 1.54 / 2,035 | 0.93 / 2,194 | 0.72 / 2,614 | 0.63 / 2,712 | -32%, +24% |

At 16 checkers tsrs beats bun's default on both axes on formbricks (0.61 s / 2.23 GiB vs 0.94 / 2.22) and cal-diy
(0.62 / 1.98 vs 1.39 / 2.03) and ties it on drizzle; t3code cannot flip at any count (tsrs at 8 checkers holds 1.84
GiB, bun at 64 threads 1.60). The 32-checker default buys 6-14% wall on the app projects for 24-34% memory and 32% wall
on vscode. A rule that stops adding checkers at the critical path would need per-file costs: on t3code the heaviest
file (bincli.ts, 13% of the check work at 2 checkers) ranks 1008 of 1,518 by static weight (nodes and text), because
its cost is Effect inference, not size; the static weights find the generated schema file (rank 0, 6% of the work)
and nothing else in the top five. `--checkerCostCache` knows the costs but only from a previous run. A dynamic rule
cannot see the floor at t = 0 either: with 8 checkers t3code's queue drains in ~1 s, inside the 1.2 s heavy-file
floor, so a staggered start still starts every checker. Left as a documented trade for the owner: the default count
on >= 32 cores (32 today; 16 would be -20..-30% peak on app projects for -6..-32% speed).

## 4. Rejected this round, with numbers (details in notes/mem-checker-scratch.md, notes/mem-never-read-apps.md, notes/mem-linux-residency-32.md, notes/mem-lazy-dts-members.md)

- Package redirects: 0 duplicate name+version copies of node_modules packages in formbricks, cal-diy, supabase, t3code.
- Mapper interning: mappers are created only on instantiation-cache misses, so they are unique by construction.
- `export *` export tables as owned values: -1.9% on formbricks and cal-diy at 32 checkers (branch
  mem/export-star-tables); below the bar.
- D1, lazy-table construct signatures instantiated only for `isWeakType`: -1.1% on t3code at 32 checkers; removed.
- `mi_collect` at parse end or checker end: 0-1%; smaller stacks: 4-11 MiB total; trimming parse workers' blocks:
  the handoff reuses them without a system call.
- Shrink-to-fit / initial capacities: tables already minimal; a smaller-step table design would be -1.7..-2.2%.
- Removing every never-read object exactly: 6-9% ceiling, in many separate mechanisms.

## 5. Scoreboard after #199 (bench 8bf9611; #202 and #204 were not yet in it)

| project | tsrs before | tsrs after #199 | bun check | tsrs / bun |
| --- | ---: | ---: | ---: | ---: |
| drizzle-orm | 1.15 GiB | 1.09 GiB | 1.01 GiB | 1.08x (was 1.14x) |
| supabase-studio | 2.29 | 2.24 | 2.14 | 1.05x |
| formbricks-web | 2.88 | 2.83 | 2.23 | 1.27x |
| cal-diy | 2.57 | 2.50 | 2.03 | 1.23x |
| t3code-server | 2.79 | 2.73 | 1.61 | 1.70x |
| vscode | 2.65 | 2.61 | 2.89 | 0.90x |
| mikro-orm | 2.64 | 2.57 | 2.82 | 0.91x |
| webpack | 753 MiB | 706 MiB | 775 MiB | 0.91x |

Wall times unchanged within noise. With #202's -7.1% on formbricks and -1.5..-2.2% elsewhere the next publish should
put formbricks near 2.63 GiB and drizzle near 1.07 GiB; supabase and drizzle are within 5-8% of bun, formbricks and
cal-diy within 18-23%, and t3code stays at 1.7x for the structural reason in section 1.

## 6. What is left

- A type graph shared between checker threads (bun's design) is the only change that closes the t3code-class gap;
  the earlier verdicts stand (notes/perf-shared-checker.md, notes/mem-shared-base.md: months, not exact).
- mimalloc's retained heap (4.7-8.2% of peak at 32 checkers) needs allocator settings; the runtime knobs measured
  in notes/mem-no-thp.md cost check time.
- Still eager in #202: namespace bodies (+43 MB on formbricks), lists with import types (30-60 MB never asked for on
  t3code and cal-diy), lists with eager JSDoc (4-17 MB); per-member laziness would add ~1%. The mode is off in
  incremental, build, watch, LSP and API runs.
