# Depot runner sizing — 2026-10-08

Use 16 vCPU for the regular Bun comparison, 32 for before/after verification and profiling, and reserve 64 for explicit scaling experiments. This changes CI allocation, not the compiler. Keep the fixed 8-vCPU instruction-regression machines and PGO build unchanged.

## Allocation evidence

Depot CPU utilization is a fraction of the allocated vCPUs; memory utilization is a fraction of the runner RAM. Recent successful runs sampled through `depot ci metrics`:

| work | previous allocation | observed utilization | decision |
| --- | ---: | --- | --- |
| PR verification | 64 vCPU | 6.49–6.55% average CPU (about 4.2 busy cores), 45.7–53.1% peak CPU, 4.29–4.71% peak memory | 32 vCPU; default checks are 1,4,16,32 threads |
| general probes | 64 vCPU | 5.9–12.9% average CPU, 27.2–32.8% peak CPU, 4.3–4.8% peak memory | 32 vCPU; skip default probe thread counts above the runner size |
| generated-code, lint-ratchet, skip-marker | 4 vCPU | 9–25% average CPU, at most 46% sampled peak CPU, at most 8.1% peak memory | 2 vCPU |
| node packaging | 4 vCPU | 67–69% average CPU, about 99% peak CPU | keep 4 |
| Rust test/build jobs | 8 vCPU | sampled CPU peaks around 97% | keep 8 |

Sources: Depot runs `835zlxgt88`, `fs50g4hfzp`, `lfx5pbvxdr` (CI and verification), `jt37zj47mj` and `8w1p2bw42m` (probes). Checker counts are software workers and may exceed the hardware count. Both the head-to-head and verification workflows permit explicit 64-checker experiments on 16 vCPU; the default general probe skips oversized counts. The layout probe also retains capacity for its existing 32-checker diagnostic-identity pass.

The runner rate at [Depot's published $0.003/vCPU-minute](https://depot.dev/pricing) is $0.048/min for 16, $0.096/min for 32, and $0.192/min for 64, before plan allowances. These are rate reductions, not guaranteed bill savings: a smaller machine can take longer. The earlier trial usage was gross compute-equivalent usage, not a confirmed invoice.

## Actual 16- and 32-vCPU comparison

Both runs measure compiler source from `cccadf7c` (unchanged from main `990d32d5`), using a separate PGO dist build on each runner, actual Depot AMD EPYC 9R45 runners, the same project configuration, and Bun `1.4.3-canary.1+620b50f6a`. Each compiler uses its default thread count: TSRS 8 checkers on 16 vCPU and 16 checkers on 32; Bun uses the available vCPUs. One untimed warm-up and ten interleaved measurements per compiler/project, with compiler order rotated each repetition.

- 16 vCPU: [Depot run `8527jr2xzj`](https://depot.dev/orgs/qw3zz3l8cs/workflows/kmbqhnz0pf?job=6fx236bj4k)
- 32 vCPU: [Depot run `hf4j9d5mqz`](https://depot.dev/orgs/qw3zz3l8cs/workflows/lzqtw4vlg3?job=57frjb8mz8)

| project | 16 vCPU TSRS / Bun (s) | TSRS speedup | 32 vCPU TSRS / Bun (s) | TSRS speedup |
| --- | ---: | ---: | ---: | ---: |
| vscode | 1.555 / 1.709 | 1.10x | 0.808 / 1.011 | 1.25x |
| mui-docs | 1.233 / 7.832 | 6.35x | 0.908 / 7.072 | 7.79x |
| t3code-server | 1.890 / 2.931 | 1.55x | 1.419 / 2.601 | 1.83x |
| mikro-orm | 1.347 / 2.038 | 1.51x | 0.872 / 1.499 | 1.72x |

| project | 16 vCPU TSRS / Bun peak RSS (GiB) | 32 vCPU TSRS / Bun peak RSS (GiB) |
| --- | ---: | ---: |
| vscode | 1.94 / 1.87 | 2.13 / 2.27 |
| mui-docs | 1.29 / 11.67 | 1.60 / 11.09 |
| t3code-server | 1.77 / 1.12 | 2.15 / 1.35 |
| mikro-orm | 1.60 / 1.40 | 1.97 / 2.09 |

TSRS wins all four projects at both sizes. Its diagnostics match the pinned 7.1-dev reference in every project at both sizes. The 16-vCPU box is sufficient for the routine benchmark; 32 remains useful for explicit scaling comparisons. The 64-checker-on-16-vCPU combination is supported but was not measured here; do not infer its performance from these default-mode results.

Raw results: [16 vCPU](../bench/results/compare/2026-10-08-cccadf7cd14b-16t.json), [32 vCPU](../bench/results/compare/2026-10-08-cccadf7cd14b-32t.json), with human-readable `.md` files beside each.

The complete head-to-head job (including PGO build/setup and all four compilers) took 1,741.5 seconds on 16 vCPU and 1,583.0 seconds on 32. Multiplying attempt duration by the published rate gives about $1.39 vs $2.53: 45% less compute-equivalent cost for 10% more wall time. These are one-run observations with separate builds/caches, not an invoice or a measured monthly saving. This head-to-head job is distinct from the full automatic benchmark workflow. The 16-vCPU run peaked at 19.7% of RAM and averaged 27.2% of allocated CPU; the 32-vCPU run peaked at 10.2% of RAM and averaged 15.1% of CPU.

This is a bounded four-project comparison, not the full application suite. Wall time includes compiler startup. CPU counts are virtual CPUs, not a claim about physical laptop cores. TSRS follows TypeScript 7.1-dev while Bun follows 7.0; diagnostic counts can differ, so timings do not establish identical compiler behavior.

## Validation and history

The 32-vCPU verification smoke run [`3h7t3mnt46`](https://depot.dev/orgs/qw3zz3l8cs/workflows/vxlxp18rrc?job=rkztprpqj4) completed in 3m49s: four projects, one timing repetition, checker counts 1/4/16/32, poisoned-arena checks and single-threaded instruction counts. All 24 diagnostic-identity cells passed. Instructions changed by at most 0.001%; the compiler source did not change. Peak memory was 6.05% of the 32-vCPU runner's RAM; average CPU 15.3%, peak 97.0%. This smoke run is not a full-suite duration comparison with the old allocation.

The three downsized CI jobs passed on 2 vCPU at `43bfaa06`: generated-code 29s, lint-ratchet 31s, skip-marker 4s. Local source lint and the Rust lint ratchet passed. Workflow structure/expression validation passed with actionlint's shellcheck and pyflakes integrations disabled; the changed standalone probe script passed shellcheck and `bash -n`.

Reporting tests use a committed real benchmark fixture to verify that historical 64-vCPU rows retain their hardware, mixed 8/16-vCPU merges retain per-cell provenance, and history begins a new baseline instead of treating a hardware change as a compiler regression. The automatic workflow's complete application suite on 16 vCPU remains to be validated after merge; the old README measurements remain labeled as 64 vCPU until replaced by measured output. Backfills predating the new `checkers16` mode measure only their default `wide` mode on the actual 16-vCPU runner.

Rollback: restore the previous runner labels and checker mode together. Head-to-head comparisons accept `cpus=64`, verification accepts `cpus=64`, and general probes accept `runner=depot-ubuntu-24.04-64` when explicitly testing large servers. Keep hardware provenance and the history baseline guard.
