# perf-clustered-assignment: placing files on checkers by the modules they share

Question (2026-10-07): each checker builds its own types for the declarations its files use, so at 16-32 checkers the
app projects repeat most of their checker work (notes/mem-per-checker-duplication.md: 63-78% of checker instructions at
32 checkers). Can the assignment put files that use the same heavy modules on the same checkers, so fewer checkers
build each module's types? The output cannot change (any assignment gives the same diagnostics,
notes/perf-order-independence.md); total checker CPU, wall time and per-checker memory can.

Result in one paragraph: yes at 16 and 32 checkers, by a few percent. The locality assignment (directory-subtree groups
placed with FENNEL over import edges) already holds most of what the import graph offers, and random perturbations of it
only cost instructions (+0-5.5%). What it missed: import edges into library declaration files that are not type checked
carry no weight there, and every edge counts the same whatever it leads to. Scoring a checker by the worth (node
count^0.5) of the modules a group shares with it, scaled per group to the group's own import edges, then moving single
groups while that lowers the summed worth of modules held per checker, lowers the instructions of a static check by
3.5-14.6% at 16 checkers and 3.6-13.4% at 32 on the five app projects, 2.7-4.3% on vscode (6-26% of the work more
checkers repeat); the offline objective it minimizes predicts the measured instructions with rank correlation 0.73-0.91
on four of the five projects. With stealing much of that is lost at the tail, and it took a second change, thieves that
keep taking from one queue, to keep it. On the 64-vCPU runner (dist builds, 12 interleaved runs) the branch lowers
wall time by 2-8% on formbricks-web, supabase-studio (16), vscode, next-packages-next and t3code-server (16), CPU by
0-5% and peak RSS by 0-4% on the app projects, with one cell worse: mui-docs at 16 checkers, +1.8-3.4% wall in two
of four rounds (a tail effect, CPU and memory unchanged). Below 16 checkers the same changes hurt (mui-docs +5.9% wall
and cal-diy +3.8% CPU at 8, mikro-orm +3.7% wall at 4), so both apply from 16 checkers on. Diagnostics are the same as
main's in every run; pr-verify's one difference (drizzle-orm) is a diagnostic that depends on check order in main and in
tsgo alike (section 6).

Base: origin/main 3540ffa. macOS numbers: Apple M5 Max (18 cores), `cargo build --release`, shared with other agents
(load 10-30), so they are instructions retired and peak footprint (`/usr/bin/time -l`), not wall. Flags `--noEmit
--incremental false --pretty false --extendedDiagnostics --checkers N` from each checkout root (mui-docs `-p docs`,
formbricks-web `-p apps/web/tsconfig.typecheck.json`, cal-diy `-p apps/web`, supabase-studio `-p apps/studio`,
t3code-server `-p apps/server`, vscode `-p src`). Sections 1-4 compare static assignments (`--checkerAssignment
locality` or `file:<path>`: no stealing), so an assignment's instruction count is reproducible to ~1%.

## 1. The ceiling: what more checkers repeat

Instructions retired, G (process total; the 1-checker run is the work done once):

| project | 1 | 16 | 32 | repeated at 16 | repeated at 32 | peak MiB 1 / 16 / 32 |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| t3code-server | 48.8 | 110.0 | 133.2 | +61.2 | +84.5 | 856 / 1,791 / 2,236 |
| formbricks-web | 54.7 | 91.5 | 116.4 | +36.8 | +61.7 | 1,367 / 2,191 / 2,732 |
| cal-diy | 42.7 | 86.0 | 100.8 | +43.2 | +58.0 | 830 / 1,717 / 2,149 |
| supabase-studio | 65.8 | 117.8 | 143.2 | +52.1 | +77.5 | 965 / 1,775 / 2,256 |
| mui-docs | 55.1 | 115.4 | 151.0 | +60.3 | +95.8 | 778 / 1,467 / 1,923 |
| vscode | 103.6 | 120.2 | 127.9 | +16.6 | +24.3 | 1,934 / 2,420 / 2,647 |

Most of it cannot be clustered away. Share of the checked weight whose files reach a package's declaration files
within one / two / any number of imports through non-library files (from `TSRS_ASSIGNMENT_DUMP`):

| project | package (declaration nodes) | 1 hop | 2 hops | any |
| --- | --- | ---: | ---: | ---: |
| t3code-server | effect (507K) | 92% | 97% | 99% |
| t3code-server | @opencode/schema, @cursor/sdk, @opencode/protocol (393K, 293K, 160K) | 0.1-1.2% | 1-2% | 30-31% |
| formbricks-web | zod (51K) | 10% | 62% | 79% |
| formbricks-web | @types/react (19K) | 33% | 52% | 64% |
| formbricks-web | next, lucide-react (73K, 127K) | 14% | 25% | 43-50% |
| cal-diy | @prisma/client (32K) | 27% | 28% | 85% |
| cal-diy | next (234K) | 19% | 29% | 56% |
| supabase-studio | @types/react, lucide-react, next | 22-70% | 45-79% | 85-92% |
| mui-docs | @types/react (19K) | 65% | 85% | 95% |

The libraries the duplication note found repeated (effect, zod, @types/react) reach most of each project, so every
checker needs them whatever the assignment. What can be grouped are the mid-sized libraries and, mostly, the
project's and workspace's own modules (mui-docs' `@mui/material` is a workspace source package, not node_modules).

## 2. Signals tried

Each variant was computed offline (Python over the `TSRS_ASSIGNMENT_DUMP` files and import edges, reproducing the
locality groups exactly) and checked with `--checkerAssignment file:<path>`. Instructions, G, 16 checkers; the
baseline is the locality assignment.

### 2a. A charge per library package a group brings onto a checker

FENNEL's score minus `lambda * (declaration nodes of the packages the group reaches within two imports that the
checker does not have yet)`:

| lambda (cost per package) | t3code-server | formbricks-web | cal-diy |
| --- | ---: | ---: | ---: |
| 0 (locality) | 110.1 | 91.6 | 86.2 |
| 1e-5 (nodes), two hops | 106.7 (-3.1%) | | |
| 1e-4 (nodes), two hops | | 89.1 (-2.7%) | 84.5 (-2.0%) |
| 1e-5 (nodes), one hop | 108.1 (-1.8%) | 92.5 (+0.9%) | 83.4 (-3.3%) |
| 1e-4 (nodes), one hop | 133.1 (+20.9%) | 91.3 (-0.3%) | 86.6 (+0.4%) |
| 1e-4 (nodes), three hops | 133.5 (+21.3%) | 88.9 (-3.0%) | 92.5 (+7.4%) |
| 3e-3 (sqrt nodes), two hops | 107.0 (-2.8%) | 91.2 (-0.5%) | 83.2 (-3.5%) |
| 3e-2 (sqrt nodes), two hops | 129.2 (+17.4%) | 88.3 (-3.6%) | 85.4 (-0.9%) |
| 30 / 300 (each package 1), two hops | 141.1 / 133.7 (+28% / +21%) | 95.5 / 96.3 (+4% / +5%) | 92.3 / 93.3 (+7% / +8%) |

The right factor differs by 10x between projects, and a strong one costs far more than it saves: it overrides the
import affinity and scatters a project's own files, whose library generics instantiated with project types are 54% of
what an extra checker repeats on t3code. Scaling the charge per group to its own import edges (so it is at most `mu`
times the group's affinity, the form kept below) gave -6.8% to +6.0% per project at mu 0.1-1, not monotonic in mu:
mui-docs got worse at every mu (its libraries are workspace sources, which a node_modules package signal cannot see).

### 2b. The noise floor: perturbing the locality assignment

To know whether -2 to -7% means anything, the locality FENNEL step was run with random tie noise (up to 0.1 times the
group's edge count), three seeds per project: t3code 111.3-113.4 (+1-3%), formbricks 93.1-93.4 (+2%), cal-diy
87.6-90.9 (+1.6-5.5%), mui-docs 115.7-117.8 (0 to +1.7%). No perturbation helped: locality is a good local optimum, and an
assignment that does better did so for a reason. Runs of one assignment agree within 0.1-1.3 G.

### 2c. Affinity by shared modules, at file granularity

A group's modules: its files and the files they import (project, workspace or library), each worth node_count^gamma.
FENNEL's score plus `mu * (group's import edges + 1) * (worth of the group's modules the checker holds) / (worth of all
the group's modules)`:

| variant | t3code-server | formbricks-web | cal-diy | supabase-studio | mui-docs |
| --- | ---: | ---: | ---: | ---: | ---: |
| imports, gamma 0.5, mu 0.3 | -7.8% | -0.4% | -2.7% | -3.1% | +6.7% |
| imports, gamma 0.5, mu 0.7 | +1.9% | -3.0% | -5.2% | -1.9% | -0.6% |
| imports, gamma 0.5, mu 1 | **-12.6%** | **-3.1%** | **-4.7%** | **-2.6%** | **-0.4%** |
| imports, gamma 0.5, mu 1.5 | +10.0% | -1.3% | -2.6% | -1.3% | -0.9% |
| imports, gamma 0.5, mu 2-3 | +14 to +21% | +2 to +5% | +6 to +12% | +1 to +5% | -2 to +5% |
| imports of imports, gamma 0.5, mu 0.3 | -8.1% | -1.3% | -4.0% | -1.2% | +0.6% |
| imports of imports, gamma 0.5-1, mu 1 | +3% | +2-3% | +8-9% | 0 to +2% | -1 to +5% |

(The mui-docs column except mu 1 was measured with a prototype grouping that differed from Rust's on 856 files: on a
case-insensitive file system Rust groups by lowercased paths. Each entry is against the baseline of the same grouping.)
mu 1 was the only setting that won everywhere at 16, but at 32 it cost mui-docs +3.3%, and t3code swings from +1.9% to
-12.6% to +10% between mu 0.7, 1 and 1.5: one large group landing on one checker or another.

### 2d. Which offline objective predicts the instructions

Over the ~100 measured assignments, the rank correlation (Spearman) between an objective computed from the assignment
and the measured instructions, per project at 16 checkers (n = 15-25 assignments; at 32, n = 5):

| objective (summed over checkers) | t3code | formbricks | cal-diy | supabase | mui-docs |
| --- | ---: | ---: | ---: | ---: | ---: |
| worth^0.5 of the checker's files only | -0.06 | 0.29 | -0.05 | 0.31 | -0.44 |
| worth^0.5 of its files and their imports | 0.73 | **0.90** | **0.91** | **0.84** | 0.29 |
| worth^1 of its files and their imports | 0.76 | 0.84 | 0.73 | 0.73 | 0.22 |
| worth^0.5 of files, imports and their imports | 0.78 | 0.69 | 0.88 | 0.59 | 0.22 |
| worth^0.5 of the transitive import closure | 0.76 | 0.73 | 0.59 | 0.04 | 0.34 |
| import edges cut (what FENNEL minimizes) | 0.79 | 0.49 | 0.58 | 0.50 | 0.42 |

So the cost of a checker is close to "the modules its files are or import directly, weighed by the square root of
their size", which is what 2c's affinity rewards greedily. mui-docs is the exception: its 13k demo files import from a
few hundred component modules every checker ends up holding.

### 2e. Minimizing that objective directly

Greedy placement by module overlap alone (no import edges), heaviest group first under the 101% cap: worse than
locality (cal-diy objective 144K against 132K): the first k groups land on empty checkers with nothing to share. A
refinement pass instead: visit the groups lightest first; move a group to the checker where its modules cost least
(gain = worth of the modules only that group holds on its checker - worth of its modules the destination does not
hold), if the gain is positive and the destination stays under 101% of an average load; repeat until no group moves
(4-9 passes) or 10 passes.

The first run of this used FENNEL's cap, `max(largest group, 101% of average)`, and lowered mui-docs by 23%, by
piling groups onto the checker that holds `packages/mui-icons-material/lib/index.mjs`, a single JS file weighing 2.5x
an average share at 16 checkers (t3code's `schema.gen.ts` is 1.36x at 32, cal-diy's Prisma `User.ts` 1.12x). With an
imbalanced static load the check took 0.88 -> 1.10 s; the refinement's cap is now 101% of the average only.

Instructions, G, static assignment:

| project | checkers | locality | refined from locality | mu 1 | mu 1 + refined |
| --- | ---: | ---: | ---: | ---: | ---: |
| t3code-server | 16 | 110.1 | 106.9 (-2.9%) | 96.2 | **94.0 (-14.6%)** |
| formbricks-web | 16 | 91.6 | 91.7 (+0.1%) | 88.8 | **86.8 (-5.2%)** |
| cal-diy | 16 | 86.2 | 84.6 (-1.9%) | 82.2 | **82.2 (-4.7%)** |
| supabase-studio | 16 | 117.3 | 117.0 (-0.2%) | 114.3 | **113.2 (-3.5%)** |
| mui-docs | 16 | 115.4 | 111.5 (-3.4%) | 114.8 | **110.6 (-4.2%)** |
| t3code-server | 32 | 133.4 | 126.7 (-5.0%) | 117.3 | **115.5 (-13.4%)** |
| formbricks-web | 32 | 117.5 | 114.0 (-3.0%) | 112.0 | **109.1 (-7.2%)** |
| cal-diy | 32 | 101.7 | 102.6 (+0.9%) | 96.6 | **96.5 (-5.2%)** |
| supabase-studio | 32 | 144.3 | 139.8 (-3.1%) | 138.6 | **137.1 (-5.0%)** |
| mui-docs | 32 | 150.8 | 135.7 (-10.0%) | 155.7 | **145.3 (-3.6%)** |
| vscode | 16 | 120.2 | | 117.2 | **117.0 (-2.7%)** |
| vscode | 32 | 127.9 | | 122.3 | **122.4 (-4.3%)** |

Peak with mu 1 + refined: t3code -127 / -235 MiB, formbricks -90 / -130, cal-diy -78 / -149, supabase -50 / -112,
mui-docs -73 / -92, vscode -60 / -117 (16 / 32 checkers). Capping the refinement at 3 passes keeps most of it (within
0.1-2.2 G of 10 passes). Of the work repeated at 16 / 32 checkers (section 1) this removes 26% / 21% on t3code, 13% /
13% on formbricks, 9% / 9% on cal-diy, 8% / 9% on supabase, 8% / 6% on mui-docs and 19% / 23% on vscode.

## 3. What landed

`crates/tsrs_compiler/src/affinity.rs` (`ModuleAffinity`) and hooks in checkerpool.rs:

- `locality_associations` builds the modules of each group (its files and their resolved import targets, deduplicated,
  in file order) and passes them to `get_checker_associations_in_order`, which adds the scaled affinity (mu 1, gamma
  0.5) to FENNEL's score; the Go assignment passes `None` and is unchanged. Then `refine`, at most 3 passes (section 5:
  the passes after the third did not show with stealing, and they are on the critical path). It keeps a per-module
  bitset of the checkers that hold it and counts the modules every checker holds once per group.
- With 10 passes the dumped Rust assignment equals the Python prototype's on all five projects at 16 and 32 checkers.
- The thief keeps its victim (section 4).
- Both apply only from 16 checkers on (`affinity::MIN_CHECKERS`, section 5); below that the placement and the stealing
  are main's.
- `TSRS_MODULE_AFFINITY=off` restores the old placement and `TSRS_STEAL_STICKY=0` the old victim choice (for A/B
  runs); `TSRS_MODULE_AFFINITY=<mu>`, `TSRS_MODULE_AFFINITY_GAMMA` and `TSRS_MODULE_AFFINITY_PASSES` are for
  experiments. The cost cache path (`--checkerCostCache`) keeps its own placement.

The assignment is on the critical path (notes/perf-front-end-fixed-costs.md), and the new work costs 2-13 ms. So the
branch also computed the assignment on its own thread while the checkers are created; main has since done the same
(#162, notes/perf-serial-assign-overlap.md), and the branch now uses main's version. What remains here: the import graph
is built while the groups are formed (they do not depend on each other), and the paths are sorted in parallel (a total
order, ties by index). `Checkers: create` and `Checkers: assign files`, ms, medians on the runner (round 2, section 5):
the measured main (3540ffa, before #162) runs them in sequence, the branch side by side, so the larger one gates the
check:

| project | checkers | main: create + assign | branch, 3 passes: create, assign | branch, 10 passes: assign |
| --- | ---: | --- | --- | ---: |
| vscode | 16 | 3 + 10 = 13 | 5, 14 | 15 |
| vscode | 32 | 5 + 11 = 16 | 6, 17 | 24 |
| mui-docs | 16 | 5 + 12 = 17 | 6, 14 | 16 |
| mui-docs | 32 | 8 + 15 = 23 | 8, 16 | 19 |
| supabase-studio | 16 | 5 + 6 = 11 | 6, 9 | 10 |
| supabase-studio | 32 | 6 + 7 = 13 | 7, 10 | 12 |
| next-packages-next | 16 / 32 | 6 / 7 | 3-4, 4-5 | 4 / 5 |

## 4. Stealing

In the default mode files move at the tail: on cal-diy at 16 checkers about 900 files (28% of its 3.2k checked files)
are run by a checker other than their owner, each bringing its modules along, and the static weights mispredict the
cost enough that t3code-server's -14% of static instructions became -1% of CPU with stealing (round 1). Thieves took
from the back of the queue with the most work left, re-chosen for every file. Now a thief keeps taking from the queue it
took from last while that queue has at least half the work of the fullest one, so it takes consecutive files of one
directory region (`queues_next`; the exactly-once stealing test runs with it). Without it the new placement cost
cal-diy CPU and memory on the runner: CPU +2.2% [+0.5, +3.6] and peak +1.4% [+0.6, +2.3] at 32 checkers in round 1,
CPU +1.3% [+0.4, +2.3] and peak +0.7% [+0.3, +1.7] at 16 in round 2; with it those cells are -0.5 to -1.1% CPU and
0 to -1.2% peak. The sticky rule alone (old placement) moved CPU -1.1% to +1.2% depending on the cell (round 2
`nocluster`): it helps the clustered placement more than the old one.

## 5. On the 64-vCPU runner

`depot-ubuntu-24.04-64`, `.depot/workflows/perf-probe.yml` with `script=tools/perf/clusterprobe.sh`: main (the merge
base, 3540ffa) and the branch built as `--profile dist` one after the other in the same directory (main's binary had
the same sha256 in every round), then 10-12 interleaved runs per cell of each variant in the default mode (stealing on);
medians, with a 95% bootstrap interval of the median ratio in brackets. CPU is the process's user + system time; peak is
max RSS; "checker CPU" is the per-checker CPU summed over the checkers (`TSRS_ASSIGNMENT_STATS=times`, one or two
separate runs, since that mode turns leaf freeing off). Every run of every variant printed the same diagnostics as
main.

| round | run | variants | what changed in between |
| --- | --- | --- | --- |
| 1 | vjvjdwmm3x | main, cluster, off (old placement, new binary), sticky (cluster + sticky thieves) | |
| 2 | wqjcx11zf2 | main, branch (cluster + sticky, 10 passes), nosticky, nocluster, passes3 | sticky on by default |
| 3 | 8j4pl3djjv | main, branch (3 passes), nosticky, nocluster, off | at most 3 passes |
| 4 | grxqbt5htn | main, branch, rate (victim by time left), offrate | |
| 5 | dvn2xrjf2k | main, branch, nosticky at 4, 8 and 16 checkers (+ mikro-orm) | module affinity from 8 checkers on |
| 6 | czbkknp6kv | main, branch at 4 and 8 checkers (+ mikro-orm) | both from 16 checkers on (as landed) |

The branch as it lands at 16 and 32 checkers, round 3 (12 runs per cell; the 16-checker gate added later does not
change anything at these counts). The measured main predates #162, which overlaps the assignment with checker creation
as the branch does; the round's `off` variant (the branch's binary with the old placement and stealing) stands in for
main after #162, and the branch against it reads the same: wall -2.4 / -3.2% on vscode, -5.9 / -8.0% on
formbricks-web, -7.6 / -0.7% on supabase-studio, -3.8 / -2.6% on t3code-server, +3.1 / -1.0% on mui-docs, CPU -1 to -5%
on the app projects except cal-diy at 16 (+0.2%), next-packages-next at 16 +1.5% CPU and +1.4% peak (16 / 32
checkers).

| project | checkers | wall s, main -> branch | wall | check | CPU (user+sys) | checker CPU s | peak MiB, main -> branch | peak |
| --- | ---: | --- | ---: | ---: | ---: | --- | --- | ---: |
| vscode | 16 | 0.899 -> 0.870 | -3.2% [-3.7, -1.2] | -3.5% [-4.6, -1.8] | -3.0% [-3.7, -1.1] | 11.5 -> 11.2 | 2244 -> 2188 | -2.5% [-2.8, -2.2] |
| vscode | 32 | 0.632 -> 0.608 | -3.9% [-5.4, -1.5] | -6.5% [-7.9, -4.0] | -3.9% [-5.0, -3.1] | 14.1 -> 13.4 | 2811 -> 2711 | -3.5% [-3.7, -3.2] |
| next-packages-next | 16 | 0.253 -> 0.248 | -2.1% [-2.8, -1.3] | -2.6% [-3.7, -1.3] | +0.8% [+0.1, +2.1] | 2.5 -> 2.5 | 723 -> 728 | +0.7% [-0.7, +1.4] |
| next-packages-next | 32 | 0.211 -> 0.204 | -3.2% [-4.2, -1.4] | -2.8% [-4.8, -1.0] | -2.4% [-4.2, -1.4] | 2.8 -> 2.8 | 844 -> 825 | -2.3% [-3.2, -1.7] |
| mui-docs | 16 | 1.007 -> 1.041 | +3.4% [+2.0, +4.9] | +4.6% [+3.1, +6.3] | -0.1% [-0.9, +1.1] | 9.2 -> 9.4 | 1695 -> 1694 | -0.1% [-0.4, +0.4] |
| mui-docs | 32 | 0.935 -> 0.928 | -0.7% [-3.2, +1.1] | -0.1% [-4.1, +1.6] | -1.7% [-2.9, -0.9] | 12.1 -> 11.7 | 2203 -> 2159 | -2.0% [-2.5, -1.7] |
| formbricks-web | 16 | 0.735 -> 0.690 | -6.1% [-8.2, -4.5] | -7.5% [-9.7, -5.4] | -1.6% [-3.1, -1.0] | 7.9 -> 7.5 | 2316 -> 2275 | -1.8% [-2.2, -1.1] |
| formbricks-web | 32 | 0.700 -> 0.644 | -8.0% [-11.2, -5.5] | -9.8% [-14.3, -6.3] | -1.3% [-3.1, -0.3] | 11.7 -> 11.4 | 2979 -> 2943 | -1.2% [-1.8, -0.8] |
| cal-diy | 16 | 0.722 -> 0.710 | -1.7% [-3.1, +0.0] | -2.0% [-3.1, +0.0] | -0.3% [-2.2, +0.4] | 8.2 -> 8.0 | 2032 -> 2017 | -0.7% [-1.8, +0.1] |
| cal-diy | 32 | 0.605 -> 0.593 | -2.0% [-3.5, +1.0] | -1.9% [-4.0, +1.4] | -1.2% [-1.9, +0.3] | 11.6 -> 11.3 | 2723 -> 2712 | -0.4% [-1.8, +0.0] |
| supabase-studio | 16 | 0.808 -> 0.753 | -6.8% [-7.8, -5.8] | -8.2% [-9.2, -6.4] | -4.5% [-5.1, -3.9] | 9.5 -> 9.0 | 1890 -> 1827 | -3.3% [-3.7, -3.0] |
| supabase-studio | 32 | 0.623 -> 0.610 | -2.1% [-3.2, -0.8] | -2.1% [-3.5, +0.1] | -3.7% [-5.2, -2.9] | 13.1 -> 12.6 | 2414 -> 2328 | -3.6% [-3.8, -3.2] |
| t3code-server | 16 | 1.630 -> 1.566 | -3.9% [-7.0, -0.4] | -4.1% [-7.2, -0.5] | -2.4% [-4.0, -0.6] | 15.1 -> 14.5 | 2285 -> 2285 | -0.0% [-0.6, +0.8] |
| t3code-server | 32 | 1.452 -> 1.446 | -0.4% [-2.5, +2.6] | -0.6% [-2.7, +2.7] | -2.4% [-3.5, -1.8] | 18.4 -> 18.6 | 2907 -> 2860 | -1.6% [-2.3, -1.0] |

Round 4 repeated the same code at 16 and 32 (10-12 runs per cell): the wall deltas moved by up to 2.2 points between
the two rounds (supabase-studio at 32: -2.1% then +0.1%; cal-diy at 32: -2.0% then -0.3%), the CPU deltas by up to 1.6
points, the peak deltas by up to 1.3; wall, CPU and peak were again lower or equal on every cell but the two below.
Two cells are not lower or equal:

- mui-docs at 16 checkers: wall +3.4% [+2.0, +4.9] in round 3 and +1.8% [+1.0, +3.1] in round 4 (+0.1% and +0.4%,
  intervals across 0, in rounds 1 and 2), CPU and peak unchanged. Its check ends on a floor of 0.77-0.79 s that every
  variant, main included, has: a few costly files on checkers that take nothing from others (one holds 68 files for
  0.77 s). Which variant lands closest to the floor is a matter of how the last files pack; the clustered placement
  lands 2-3% further from it in two of four rounds. Picking the victim by its owner's time left at its rate so far
  (`rate`, round 4) brought it to -4.9% [-5.8, +1.4], but cost cal-diy at 32 3.5% [+2.1, +3.9] of CPU and 1.7% of peak,
  so it is not in the branch.
- next-packages-next at 16: CPU +0.8% [+0.1, +2.1] (round 3) and +2.1% [+0.6, +2.7] (round 4), while its wall is 2-4%
  lower.

Rounds 1-2 say what the parts do (deltas against main): the new placement alone (`cluster`, round 1) lowered CPU on 11
of 14 cells but raised it on cal-diy at 32 (+2.2% [+0.5, +3.6], peak +1.4%); with the sticky thief added every app
project cell was lower or equal. The old placement on the new binary (`off`) was within the noise except vscode at 32
(-3.6% wall from running the assignment while the checkers are created). Ten refinement passes against three (round
2): CPU and peak within 0.6 points on every cell, wall as good or better with three (vscode at 32: -6.1% against
-3.5%).

Below 16 checkers (round 5, when the module affinity applied from 8 checkers on and the sticky thief everywhere, 10 runs
per cell): at 8 checkers vscode, next-packages-next, formbricks-web, supabase-studio and t3code-server were lower or
equal (t3code -5.6% wall, -3.8% CPU), but mui-docs took 5.9% [+2.6, +9.0] longer with 4.2% more CPU, and cal-diy used
3.8% [+2.9, +4.6] more CPU and 1.7% more memory; at 4 checkers (affinity off) mikro-orm took 3.7% [+2.9, +4.4] longer
with 1.4% more CPU and 2.3% more memory, which the sticky thief alone causes (without it, -0.1%). pr-verify had shown
the same at 4 checkers before the gate: t3code-server +22.4% wall (on the Mac: 81 -> 89 G instructions with stealing,
78.4 against 79.3 G static; the clustered placement makes the real costs uneven and about 900 files move instead of
500), mikro-orm +3.4%. At 4-8 checkers each checker already holds most modules, so there is little to save and the
uneven costs win; at 16 and 32 the saving wins. Hence the gate at 16. The final code at 4 and 8 checkers, round 6:
| project | checkers | wall s, main -> branch | wall | check | CPU (user+sys) | checker CPU s | peak MiB, main -> branch | peak |
| --- | ---: | --- | ---: | ---: | ---: | --- | --- | ---: |
| vscode | 4 | 2.638 -> 2.635 | -0.1% [-0.7, +0.4] | -0.2% [-0.5, +0.5] | +0.1% [-0.4, +0.7] | 9.9 -> 10.0 | 1879 -> 1879 | +0.0% [-0.3, +0.5] |
| vscode | 8 | 1.467 -> 1.462 | -0.3% [-0.6, +0.3] | -0.2% [-0.5, +0.5] | -0.1% [-0.3, +0.4] | 10.5 -> 10.4 | 2022 -> 2027 | +0.3% [-0.0, +0.5] |
| next-packages-next | 4 | 0.559 -> 0.557 | -0.4% [-1.2, +0.2] | -0.5% [-1.3, +0.4] | -0.1% [-1.3, +0.4] | 2.0 -> 2.0 | 580 -> 580 | -0.2% [-0.8, +1.1] |
| next-packages-next | 8 | 0.350 -> 0.346 | -1.1% [-1.9, -0.3] | -0.3% [-1.6, +1.1] | +0.2% [-0.7, +1.0] | 2.2 -> 2.1 | 646 -> 644 | -0.2% [-0.9, +0.4] |
| mui-docs | 4 | 1.498 -> 1.494 | -0.2% [-0.9, +0.4] | +0.4% [-0.3, +1.0] | +0.4% [-0.5, +1.5] | 5.2 -> 5.2 | 1105 -> 1107 | +0.2% [-0.5, +0.7] |
| mui-docs | 8 | 1.255 -> 1.262 | +0.6% [-1.7, +7.6] | +1.5% [-1.3, +8.5] | +0.3% [-0.8, +6.1] | 7.4 -> 7.4 | 1369 -> 1375 | +0.5% [-1.0, +1.6] |
| formbricks-web | 4 | 1.412 -> 1.408 | -0.3% [-1.3, +0.9] | -0.2% [-1.4, +1.4] | +0.1% [-1.1, +1.4] | 4.8 -> 4.8 | 1706 -> 1712 | +0.4% [-0.1, +0.6] |
| formbricks-web | 8 | 0.896 -> 0.895 | -0.1% [-1.3, +1.4] | +0.4% [-1.1, +1.9] | -0.5% [-1.3, +0.9] | 6.0 -> 5.9 | 1967 -> 1963 | -0.2% [-0.5, +0.0] |
| cal-diy | 4 | 1.318 -> 1.308 | -0.7% [-3.4, +1.6] | -0.3% [-3.4, +2.3] | +0.1% [-0.9, +1.5] | 4.9 -> 4.8 | 1263 -> 1256 | -0.5% [-0.9, +0.1] |
| cal-diy | 8 | 0.912 -> 0.908 | -0.4% [-2.4, +0.9] | +0.2% [-2.2, +1.5] | +0.1% [-1.4, +1.5] | 6.0 -> 6.0 | 1552 -> 1551 | -0.0% [-1.7, +0.8] |
| supabase-studio | 4 | 1.700 -> 1.701 | +0.0% [-1.0, +2.3] | +0.5% [-1.1, +2.7] | +0.6% [-0.8, +2.1] | 6.1 -> 6.2 | 1300 -> 1301 | +0.1% [-0.4, +1.0] |
| supabase-studio | 8 | 1.070 -> 1.066 | -0.3% [-1.3, +1.0] | +0.1% [-1.2, +1.9] | +0.9% [+0.1, +1.5] | 7.2 -> 7.3 | 1510 -> 1515 | +0.3% [-0.4, +0.9] |
| t3code-server | 4 | 1.976 -> 1.955 | -1.1% [-6.8, +2.5] | -1.2% [-7.1, +2.8] | -1.5% [-2.6, +0.6] | 7.4 -> 7.3 | 1276 -> 1277 | +0.0% [-1.1, +0.8] |
| t3code-server | 8 | 1.935 -> 1.928 | -0.4% [-2.6, +4.0] | -0.2% [-2.6, +4.1] | -0.2% [-0.7, +0.4] | 12.7 -> 12.5 | 1891 -> 1890 | -0.0% [-0.4, +0.2] |
| mikro-orm | 4 | 2.142 -> 2.182 | +1.8% [-0.5, +3.5] | +1.9% [-0.6, +4.2] | +1.9% [-0.3, +3.6] | 8.1 -> 8.0 | 1425 -> 1429 | +0.3% [-0.2, +0.7] |
| mikro-orm | 8 | 1.305 -> 1.303 | -0.2% [-0.8, +1.2] | +0.0% [-0.8, +1.0] | -0.4% [-1.3, +0.4] | 9.3 -> 9.4 | 1675 -> 1676 | +0.1% [-0.3, +0.5] |

All within the noise: no wall interval above 0, mikro-orm at 4 down from +3.7% to +1.8% [-0.5, +3.5], and the one
interval above 0 (supabase-studio CPU at 8, +0.9% [+0.1, +1.5]) is in a cell where the only difference from main is that
the assignment runs while the checkers are created; 48 intervals give about two such by chance.

## 6. Output

`--pretty false` output, main against the branch (default mode, with stealing), on the ten old bench projects at 4, 16
and 32 checkers, and with `TSRS_STEAL_STICKY=0` at 16: byte-identical, same exit codes, in all 40 cells (vscode 448
lines, webpack 1,027, cal-diy 164, Compiler 53, Compiler-Unions 54, t3code 11, supabase 9; xstate-main, mui-docs and
formbricks-web clean). On the runner every timed and stats run of every probe cell had the same diagnostics hash and
exit code as main. pr-verify (`tools/perf/verify.py`, release builds of main and the branch, 3 runs per cell at 1, 4, 16
and 32 checkers on all 17 bench projects, plus a poisoned-arena run at 16) on the final commit, run `jvg6pq5w88`:
identical in 101 of 102 cells, wall lower by median on 14 of 17 projects (at most +0.6% on the others), single-threaded
instructions within 0.001%; the one difference is drizzle-orm at 32 checkers, below. It also shows drizzle-orm at 16
checkers with 4.9% more peak (831 -> 872 MiB; 4.7% in the earlier run `hc4w662d36`) at -1.0% wall, a project and count
the probe rounds did not cover.

pr-verify flagged one project, drizzle-orm (32 checkers in both runs, and the poisoned 16-checker run in
`hc4w662d36`), and the cause is older than this branch and shared with tsgo: whether
`drizzle-kit/tests/cli-check.test.ts(75,3)` gets `TS2769: No overload matches this call` (10,846 errors instead of
10,845) depends on which files a checker checked before it. Main's own binary, on the Mac:

| run | main | branch |
| --- | --- | --- |
| 1, 4, 16 checkers (16 twice) | without | without |
| 32 checkers, default (twice) | with, with | without, with |
| 16 checkers, `random:1`, `random:2` | without | without |
| 32 checkers, `random:3`, `random:4` | with | with |
| 32 checkers, `--checkerAssignment locality` | without | with |

tsgo has it too: 7.0.2 reports it at 32 checkers but not at 1 or 4, the 7.1.0-dev.20260930.4 nightly at 4 but not at 1
or 32. So it is not an assignment-independent diagnostic, in TypeScript or here; a change of placement moves it between
cells (pr-verify: main has it at 32 checkers and not at 16, the branch the other way round). Nothing in this branch
changes what a checker does; fixing the order dependence is a separate checker task (which check of the `cli-check`
call depends on earlier state was not traced).

## 7. Not done

- A placement that sees module costs directly (the work census per module) instead of node counts; the objective in 2d
  is a proxy that fits mui-docs poorly.
- Stealing that picks files by module overlap with the thief (the sticky rule is a cheap stand-in).
- Moving pairs of groups (swaps): single moves are blocked by the 101% cap once the checkers are full, which is where
  the refinement stops on most projects.
- Balancing on cost rather than static weight: the clustered placement concentrates costly files (Effect services,
  data-grid demos), which the static weight does not see, so stealing has more to fix. That is what limits the gain
  with stealing, what makes the 4-8 checker range lose, and probably what moves mui-docs' tail at 16. The victim by
  time left (`rate`, round 4) is one attempt; a cost estimate that counts a file's imported module worth is another.
- The drizzle-orm order dependence (section 6).

## Reproduce

```sh
cargo build --release --locked -p tsrs_cli
cd bench/.work/solutions/<project>
# the assignment inputs (files and import edges) for offline placements, and a placement fed back in:
TSRS_ASSIGNMENT_DUMP=/tmp/<project> tsrs -p <tsconfig> --noEmit --incremental false --pretty false --checkers 16
/usr/bin/time -l tsrs -p <tsconfig> ... --checkers 16 --checkerAssignment file:/tmp/assignment.txt
# static, branch against main's placement (instructions retired, peak):
/usr/bin/time -l tsrs -p <tsconfig> ... --checkers 16 --checkerAssignment locality
TSRS_MODULE_AFFINITY=off /usr/bin/time -l tsrs -p <tsconfig> ... --checkers 16 --checkerAssignment locality
# the runner probe (main and the branch as `--profile dist` in one directory, interleaved, default mode):
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref perf/clustered-assignment \
  --input projects=vscode,next-packages-next,mui-docs,formbricks-web,cal-diy,supabase-studio,t3code-server \
  --input script=tools/perf/clusterprobe.sh --input probe_args='--reps 12 --stats-reps 1 --checkers 16,32'
```

The offline prototype (Python over the dump: the locality groups, the package charge, the module affinity, the
refinement, the objective correlations) was not committed; section 2 describes each variant fully enough to rebuild it,
and the Rust placement reproduces the final one exactly.
