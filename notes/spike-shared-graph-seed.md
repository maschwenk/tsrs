# spike-shared-graph-seed: can the shared-graph seed's wall cost be hidden? (2026-10-09, negative)

Question (round 1 of the seed-cost work): the shared-graph prototype (notes/spike-shared-graph.md, branch
`spike/shared-graph`) lowers peak memory 5-10% at 8 checkers on the 16-vCPU runner but costs 7-13% wall, because the
seed is built serially. Can the wall cost go to within +2% of main on t3code-server, formbricks-web, cal-diy,
supabase-studio, mikro-orm and vscode (8 checkers, median of 7-10 interleaved runs) while at least two of them keep
-5% peak?

**Answer: no, not with this design.** The best measured combination (seed with throwaway checkers that are freed,
early seed start, cheaper fork reads) keeps -5.7..-14.4% peak but costs +6.0..+16.2% wall (paired). Two floors
block it, each measured on the runner:

1. **The compiled-in read paths alone cost +2.3..+4.8% wall** (`foff`: the feature build with the switch off,
   paired, 8 checkers) and the empty-seed forks +4.3..+8.0% (`on0`). Every variant below starts from there, above
   the +2% bar on all six projects, before any seed exists.
2. **The 8 threads that wait for the seed cannot do useful work without a graph of their own.** Letting them check
   files meanwhile (strategy F) is only 24-49% productive on the app projects (72% on vscode): most of a cold
   checker's time on a file is building the library types the file pulls in, which the fork then gets again from
   the seed or rebuilds. So F hides about a third of `T_w`, not all of it.

Diagnostics were byte-identical to main (`off`) in every cell: 6 projects x 8 and 16 checkers x every variant, two
runs of 600 timed runs each.

## What was built (branch `spike/shared-graph-seed`, worktree `wt/seed-cost`)

All behind `--features shared-graph` and `TSRS_SHARED_GRAPH=1`; `TSRS_TIMELINE=1` prints the timeline.

| commit | strategy | switch |
|---|---|---|
| 0a7272f5 | F-lite: while the seed is built, each pass thread checks files from the back of its own queue with its plain (throwaway) checker; at the freeze it adds the throwaway's global diagnostics to the pool's and becomes a fork. The throwaway is leaked. | `TSRS_SHARED_GRAPH_THROWAWAY=1` |
| 0a7272f5 | Bug fix: the seed picked its files before `classify` marked the leaves, so it could check a leaf file (with F, a throwaway freed that leaf under the seed: SIGBUS). `fileregions::will_be_leaf` excludes them. | always |
| 6f0dfcd9 | F full: the throwaway checks inside a scratch region (`Region::new_scratch` + `enter_scratch`), which is retired at the switch. Diagnostics already escape scratch regions (diagnostic.rs), so nothing is copied. `=protect` mprotects the retired pages `PROT_NONE`: no fault on five projects (Mac). | `TSRS_SHARED_GRAPH_THROWAWAY_FREE=1` |
| 7128326c | A: the seed starts before the leaf `prepare`, which runs beside checker creation as with the switch off; the seed waits for it (a `OnceLock`) before choosing files. Saves the 4-8 ms serial prepare. | `TSRS_SHARED_GRAPH_EARLY=1` |
| 4030d080 | E1: the `P<Type>`-keyed two-level maps (lazy member, lazy mapped, object-type instantiation tables) skip the seed's map for a key outside the frozen region (a fork's own types never are). Mac, one checker, instructions vs main: on0 t3code-server +5.30% -> +4.65%, formbricks-web +4.89% -> +3.10%. | always |
| 711e3488 | A throwaway checks a "critical" front item (weight >= N permille of a checker's share) first. Off: on t3code-server the one such item (339 permille) took 290 ms and was not the tail, which no weight predicts. | `TSRS_SHARED_GRAPH_CRITICAL=<permille>` |

Not built: G (a plain fallback for poor seeds; it cannot get below the `foff` floor), B, C, D, H (rejected in the
analysis: B saves 11-16 ms, C has no knee, D cannot share ids, H fails both bars).

## Numbers (Linux, `depot-ubuntu-24.04-16`, 10 interleaved reps, medians; paired = median of per-rep ratios vs off)

Variants: `off` = default build (main's code); `foff` = feature build, switch off; `on0` = empty seed; `on` = 10
permille seed; `F` = on + throwaways (kept); `Ffree` = on + throwaways freed; `FA` = Ffree + A (+E1);
`FA2.5` = FA with a 2.5 permille seed.

Run 2f40939vl4 (commit 6f0dfcd9, before A and E1), 8 checkers:

| project | peak on | peak F | peak Ffree | wall paired on0 | on | F | Ffree | user Ffree vs on |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| t3code-server | -3.7% | +11.7% | -10.7% | +4.3% | +9.4% | +12.8% | +9.2% | +11.7% |
| formbricks-web | -9.1% | +4.2% | -12.7% | +4.4% | +9.5% | +5.1% | +6.3% | +17.7% |
| cal-diy | -11.8% | +10.1% | -13.2% | +3.3% | +9.4% | +8.7% | +10.3% | +20.5% |
| supabase-studio | -7.8% | +5.4% | -10.5% | +4.6% | +10.2% | +7.9% | +7.2% | +13.9% |
| mikro-orm | -9.6% | +4.0% | -14.4% | +3.6% | +14.6% | +4.7% | +9.3% | +11.4% |
| vscode | -1.9% | +1.5% | -5.2% | +2.5% | +7.7% | +3.3% | +3.7% | +1.4% |

Plan step 1's stop rule (F - on0 <= +1% paired on four projects): met on two (formbricks +0.7, vscode +0.8; mikro
+1.1, supabase +3.3, cal +5.4, t3code +8.5). Freed throwaways lower the peak below `on` (their files' graphs go away
with the region, like leaf files), but cost wall.

Run 6slgs9vngp (commit dbeed2b5: A and E1 in), 8 checkers — the final table against main:

| project | peak off MiB | FA peak | FA wall paired | FA user | FA2.5 peak | FA2.5 wall paired | foff wall paired | on0 wall paired |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| t3code-server | 1808 | -9.6% | +16.2% | +16.7% | -2.4% | +8.6% | +4.2% | +4.6% |
| formbricks-web | 1714 | -12.6% | +6.8% | +9.7% | -2.6% | +7.7% | +2.3% | +4.3% |
| cal-diy | 1451 | -13.7% | +13.0% | +8.7% | -5.0% | +9.0% | +3.5% | +5.2% |
| supabase-studio | 1413 | -10.4% | +6.8% | +9.4% | -4.1% | +4.3% | +2.5% | +4.6% |
| mikro-orm | 1643 | -14.4% | +6.6% | +8.5% | -5.5% | +14.9% | +4.8% | +8.0% |
| vscode | 1967 | -5.7% | +6.0% | +6.4% | -1.0% | +6.1% | +3.1% | +5.1% |

At 16 checkers: FA peak -7.8..-23.6%, wall +5.1..+11.9% paired; FA2.5 peak -1.8..-9.9%, wall +2.4..+9.2%.

Where the throwaways' CPU goes (run 2f40939vl4, `Ffree` vs `on`, 8 checkers, sums over the checkers, ms): a
throwaway runs until the seed is frozen, `T_w` after the pass starts. The fork CPU it saves against `on` (same seed)
is the productive part.

| project | T_w | throwaway CPU | fork CPU on | fork CPU Ffree | productive |
|---|---:|---:|---:|---:|---:|
| cal-diy | 229 | 1752 | 5575 | 5158 | 24% |
| formbricks-web | 188 | 1807 | 5837 | 5243 | 33% |
| supabase-studio | 201 | 1702 | 7223 | 6692 | 31% |
| mikro-orm | 263 | 2173 | 10038 | 9078 | 44% |
| t3code-server | 274 | 3337 | 14087 | 12436 | 49% |
| vscode | 114 | 932 | 12622 | 11951 | 72% |

The seed also runs 10-30 ms longer beside 8 busy throwaways (t3code `T_w` 246 -> 274 ms): 9 threads on 8
physical cores.

## Why the target is out of reach, in numbers

- Even with every throwaway millisecond productive, F's wall is the `on0` wall, and `on0` is +4.3..+8.0% paired
  (+2.5..+4.6% in the earlier run). E1 removed 0.6-1.8 points of single-checker instructions; the Linux profile
  (analysis section 5.E) spreads the rest over many functions at <=0.3% each, and `foff` (no fork at all) is already
  +2.3..+4.8%. Reaching +2% would need the compiled-in cost near zero, i.e. a different read path design, not
  fixes to this one.
- With 24-49% productivity, the exposed part of `T_w` is (1 - p) x `T_w` = 100-175 ms on the app projects, 5-15% of
  their 1.0-2.1 s walls, on top of the floor.
- A smaller seed shrinks `T_w` linearly and the peak saving with it (FA2.5: `T_w` 27-74 ms, peak -1.0..-5.5%, wall
  still +4.3..+14.9% because the floor stays).

## Reproduce

```sh
git checkout spike/shared-graph-seed
depot ci dispatch --repo maschwenk/tsrs --workflow perf-probe.yml --ref spike/shared-graph-seed \
  --input runner=depot-ubuntu-24.04-16 \
  --input projects=t3code-server,formbricks-web,cal-diy,supabase-studio,mikro-orm,vscode \
  --input script=tools/perf/sharedprobe.sh \
  --input probe_args='variants=off,foff,on0,FA,FA2.5 --checkers 8,16 --reps 10 --no-strace --no-perf-stat'
```

A leading `variants=` in `probe_args` picks the variants (`off`, `foff`, `on`, `on<p>`, `on0`, `F`, `F<p>`, `Ffree`,
`FA`, `FA<p>`). About 25 runner-minutes for 600 runs.
