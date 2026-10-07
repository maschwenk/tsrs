# perf-memory-traffic-32: cache, TLB and memory behaviour of the check at 32 checkers

notes/perf-checker-64.md split the extra checker CPU at 32 checkers on the 64-vCPU runner into duplicated instructions
(+30%) and CPU per instruction (+11%). The second part looked like memory contention: 32 threads sharing 8 L3 slices.
This note measures it with hardware counters. On current main it is not there: user-mode cycles per instruction at 32
checkers are equal to or lower than at one checker. The extra CPU is the extra instructions. No change is landed. The
two exact changes tried on the largest miss site were within noise.

Machine: Depot `depot-ubuntu-24.04-64` (AMD EPYC 9R45, Zen 5, 64 vCPUs, one thread per core, 8 x 32 MiB L3, one NUMA
node, THP `madvise`), base 4681c2a (release 0.6.0). The kernel exposes the core PMU (`Fam17h+ core perfctr`) with the
Zen 5 named events, but no IBS PMU. That rules out `perf c2c` and `perf mem`, so false sharing is read from the
cross-CCX fill counters instead. Probes: `tools/perf/memprobe.sh` and `memprobe2.sh` on the unmerged branch
`perf/memory-traffic-32-probe` (runs 548f8ps58m and fv73db904b). A/Bs used `tools/perf/abprobe.sh` (both
binaries `--profile dist`, 11 interleaved runs per cell) on branches `perf/memory-traffic-32-ab-a1` / `-a2` (runs
8cgs3dvnw1, x1t11ns2zt). `cargo build --release` for the counter runs, `--noEmit --incremental false --pretty false`.

## 1. Counters, 1 vs 32 checkers (whole process, user space, medians of 3)

| | vscode 1 | vscode 32 | t3code-server 1 | t3code-server 32 |
| --- | ---: | ---: | ---: | ---: |
| check s | 10.82 | 0.448 | 4.49 | 1.594 |
| summed checker CPU s (`TSRS_ASSIGNMENT_STATS=times`) | 10.81 | 13.56 | 4.49 | 20.17 |
| cycles:u G | 54.9 | 66.3 (+21%) | 23.6 | 85.1 (+261%) |
| instructions:u G | 105.4 | 129.7 (+23%) | 51.0 | 195.0 (+282%) |
| **IPC** | **1.92** | **1.96** | **2.16** | **2.29** |
| L2 misses (`l2_cache_req_stat.ic_dc_miss_in_l2`) per 1k instructions | 2.38 | 2.32 | 2.25 | 2.00 |
| fills from DRAM, demand (`ls_dmnd_fills_from_sys.dram_io_all`) per 1k | 0.141 | 0.157 | 0.158 | 0.119 |
| fills from another CCX's cache (`ls_any_fills_from_sys.near_cache`) per 1k | 0.020 | 0.030 | 0.026 | 0.011 |
| fills from a far cache (`far_cache`) | 0 | 0 | 0 | 0 |
| fills from this CCX's L3 (`local_ccx`) per 1k | 0.76 | 0.73 | 0.89 | 0.87 |
| L1 dTLB misses per 1k | 0.95 | 0.74 | 0.85 | 0.85 |
| L2 dTLB misses (page walks) per 1k | 0.054 | 0.019 | 0.027 | 0.020 |
| of which 2 MiB / 4 KiB pages (M) | 0.49 / 0.27 | 0.34 / 0.27 | 0.17 / 0.10 | 0.37 / 0.19 |
| load-queue-full dispatch stalls per 1k | 0.43 | 0.57 | 0.31 | 0.36 |
| store commit cancels (WCB full) | ~0 | ~0 | ~0 | ~0 |
| page faults | 22 K | 34 K | 11 K | 34 K |

What the table says:

- **No per-instruction penalty in user space.** vscode does 23% more instructions at 32 checkers and spends 21% more
  user cycles. t3code-server does 3.8x the instructions (every checker re-resolves its large shared service graph) and
  3.6x the cycles. The +11% CPU per instruction in notes/perf-checker-64.md was measured on CPU time before the
  heavy-file and assignment work (#159, #165, #169, #171). It is not a cache effect on this build. The clock at 32
  checkers is 4.2-4.3 GHz: 63.1 G cycles:u in 14.98 s task-clock in the A/B runs' `perf stat`, and the checker threads
  hold 87% of user cycles (57.9 G) over 13.6 s of checker CPU. The one-checker runs did not record task-clock, so a
  clock change between 1 and 32 checkers is not measured here. Summed checker CPU grows +25% against +23% instructions
  on vscode, which leaves at most a few percent for it.
- **False sharing would show as cross-CCX fills, and there are almost none.** A line written on one CCX and read on
  another comes back as a `near_cache` fill. There are 3.9 M of them at 32 checkers on vscode (2.1 M at one checker,
  from the parallel parse), out of 3.8 G L2 hits and 30 M DRAM fills. Even at a DRAM-like latency of ~400 cycles each,
  that is under 2.5% of cycles. They also include the program's read-shared data (AST, binder symbols), which each CCX
  has to fetch once whatever the layout. The `far_cache` (other socket) count is zero: one NUMA node. The shared atomics
  left on the checker path were already per thread or off by default: node and symbol ids in 1,024-id blocks per checker
  thread (`use_id_blocks`), `FrozenCell` with no counter in release builds, and the statistics counters (`unioncache`,
  `infermemo`, `relater_derived`) only under their env switches.
- **Memory bandwidth is not the limit.** DRAM demand fills per instruction rise 11% on vscode (each checker has its own
  30-40 MiB of tables and one CCX's 32 MiB L3 is shared by 8 checkers) and fall 25% on t3code-server, whose extra
  instructions are mostly duplicated work on data the checker already holds. The extra DRAM fills on vscode are about 2
  M, under 1 G cycles of 66 G at worst, and the IPC does not fall.
- **The huge pages work.** At the peak of a 32-checker t3code-server run, 2.02 of 2.97 GiB RSS is `AnonHugePages`
  (`/proc/<pid>/smaps_rollup`, sampled every 10 ms). L2 dTLB misses (page walks) are 0.02-0.05 per 1,000 instructions,
  and fewer at 32 checkers than at one: 2.5 M per run, which is ~0.4% of cycles at 100 cycles a walk. The arena's 2 MiB
  chunks hold the types. Of the walks that the counters split by page size, the 4 KiB ones (the mimalloc heap, `no_thp`)
  are 0.27 M per run at both counts. Nothing here argues for revisiting notes/mem-no-thp.md.
- **Page faults** grow 22 K -> 34 K (vscode): ~12 K faults of fresh per-checker memory, far from the parse-phase
  `mmap_lock` problem of notes/perf-parse-heap-faults.md. CPU migrations: 27-50 per run at one checker, 100-142 at 32.

## 2. Per-symbol cycles, 1 vs 32 checkers

`perf record -e '{cycles:u,instructions:u}'` at 1 and 32 checkers. For each symbol: the cycles it spends at 32 checkers
minus its 32-checker instructions times its one-checker CPI. That is the cycles a symbol loses to running next to 31
other checkers. The largest values out of 66 G (vscode) / 85 G (t3code-server) cycles:

| vscode, symbol | extra G cycles | CPI 1 -> 32 |
| --- | ---: | --- |
| `SyncMap<Path, Option<SourceOutputAndProjectReference>>::load` (the program's redirect map, read by the checker) | +0.20 | 0.48 -> 0.68 |
| `NameResolver::resolve` | +0.16 | 0.38 -> 0.51 |
| `Node::text` | +0.14 | 0.45 -> 0.56 |
| libc `0x1986a5` (glibc string/memory routine, see below) | +0.12 | 0.77 -> 0.98 |
| `_mi_page_malloc_zero` | +0.10 | 0.48 -> 0.55 |
| `SymbolMap::search` | +0.10 | 0.48 -> 0.50 |
| ... all others | under +0.08 each | |
| `assign_node_id` (fewer contended first writes: ids come in blocks) | -0.42 | 1.16 -> 0.74 |
| `KeyedLinkStore<Node, SymbolLinks>::get` | -0.20 | 0.90 -> 0.63 |

| t3code-server, symbol | extra G cycles | CPI 1 -> 32 |
| --- | ---: | --- |
| libc `0x1986a5` | +0.36 | 0.93 -> 1.14 |
| `is_symbol_unaffected_by_instantiation` | +0.15 | 0.30 -> 0.37 |
| `get_object_type_instantiation` | +0.10 | 0.48 -> 0.56 |
| ... all others | under +0.10 each | |
| `instantiate_type_with_alias_worker` | -0.35 | 0.40 -> 0.38 |
| `assign_symbol_id` | -0.36 | 0.50 -> 0.27 |

The losers and winners cancel: no symbol loses more than 0.4% of the run's cycles. The one site that rises above the
noise on both projects is the checker's lookups into the program's shared maps (the redirect `SyncMap`,
`get_resolved_module`, `get_source_file`: +0.3 G together on vscode, and the top L2-dTLB-miss sites at 32 checkers after
`SymbolMap::search` and `_mi_page_malloc_zero`). Another change of this round takes those lookups out of the check on
the instruction side (a per-checker memo and a shortcut for the redirect map). They are left to it here.

The libc address is inside glibc 2.39's unexported string routines: the nearest dynamic symbol is a placeholder, and its
neighbours 0x198684 / 0x19869e / 0x1986ab are hot too. DWARF call chains at 32 checkers on t3code-server (run
fv73db904b, 11,443 samples) put it at 2.0% of user cycles. 75% of its samples come from
`infer_from_literal_parts_to_template_literal` (relater_1.rs: the `starts_with` / `ends_with` / `find` on the source and
target texts), and 7% from `SymbolMap::search` (name comparison). So it is a byte comparison (memcmp/bcmp family), and
it is t3code-server's template-literal inference doing the same comparisons in every checker. That is instruction-side
work. It is left for the CPU lane, with its CPI (0.93 -> 1.14) as a pointer: the compared texts are cold in the checker
that repeats the inference.

## 3. Miss sites at 32 checkers

`perf record -e <event>:u -c 2003` at 32 checkers, share of the event's samples:

| event | vscode top sites | t3code-server top sites |
| --- | --- | --- |
| DRAM fills | `SymbolMap::search` 3.5%, rehash of `ReferenceInstantiations` 1.7%, rehash of `StringLiteralTypes` 1.6%, `assign_node_id` 1.5%, `Node::modifiers` 1.4% | **rehash of `ReferenceInstantiations` 9.5%**, `instantiate_type_with_alias_worker` 3.6%, `SymbolMap::search` 2.9%, `get_type_of_symbol` 2.0% |
| L2 misses | `SymbolMap::search` 2.3%, `_mi_page_malloc_zero` 1.3%, `get_type_at_flow_node` 1.3% | `instantiate_type_with_alias_worker` 4.7%, `SymbolMap::search` 2.0%, `get_type_of_symbol` 1.7% |
| L2 dTLB misses | `SymbolMap::search` 3.7%, `_mi_page_malloc_zero` 3.7%, program `SyncMap` loads 3.6% | `instantiate_type_with_alias_worker` 7.4%, `_mi_page_malloc_zero` 4.4%, `SymbolMap::search` 2.6% |
| cross-CCX fills | 0 samples of `far_cache`; `near_cache` too rare to sample | same |

The misses are spread thin: apart from one, no site holds more than 4% of an event. The exception is the rehash of a
generic interface's instantiation table (`ReferenceInstantiations`, types.rs). It stores one 4-byte reference per slot
and recomputes each slot's hash from the reference's argument list on every growth. That costs one load for the
reference, one for its argument slice and one for each argument's `id`, all of them cold. It is 1.0% of t3code-server's
cycles at one checker and 1.3% at 32 (CPI ~6.6), 0.25% on vscode. The table is per checker, and so is its rehash.

## 4. Candidates

| | change | exact | memory | result (11 interleaved runs, median, min-max) | kept |
| --- | --- | --- | --- | --- | --- |
| A1 | `ReferenceInstantiations` hashes the argument handles instead of their `id`s (no load per argument; the table is never iterated, so the hash only places slots) | yes (32/32 local cells identical, runner identity on 2 projects) | none | t3code-server 32: check 1362 -> 1401 ms (1311-1472 / 1315-1463), 16: 1479 -> 1492; vscode 32: 411 -> 414, 16: 711 -> 709; cycles:u t3 +0.7%, vscode -1.4% (one run each) | no: noise |
| A2 | A1 plus a 32-bit hash in each slot (8-byte slots): growth rehashes from the slots alone, probes skip on a hash mismatch | yes (same gates: 8 projects x {1, 4, 32, `go`} identical) | +16 MiB summed at 32 checkers on t3code-server (4.1 M slots; peak +11 MiB measured, +0.4%) | t3code-server 32: 1455 -> 1404 ms (1367-1594 / 1331-1509), 16: 1570 -> 1642 (1489-1738 / 1491-1886); vscode 32: 414 -> 425, 16: 726 -> 732; cycles:u t3 +1.1%, vscode +1.3% | no: noise, cycles do not fall |

t3code-server's check at 32 checkers varies by 10-15% between runs of the same binary (stealing order). A 1% site cannot
be resolved there with 11 runs, and neither change moved the cycle count on vscode, where the spread is 5%. The rehash's
DRAM fills are real, but a growing table pays them once per slot per doubling. Making the hash cheaper removes little
because the reference and its argument list still have to be read when a probe matches.

Not tried, and why:

- Padding shared atomics to 64 bytes: there is no contended shared atomic on the check path (section 1), and cross-CCX
  fills are ~0.03 per 1,000 instructions.
- Pinning checkers to CCXs: the scheduler already keeps threads in place (100-142 migrations per run with 32 checkers
  plus the parse threads), cross-CCX traffic is negligible, and pinning is not portable.
- Larger arena chunks or more huge-page coverage: page walks are under 0.1 per 1,000 instructions.
- Prefetching on predictable walks (`SymbolMap::search`, the link stores): their CPI does not grow at 32 checkers (0.48
  -> 0.50 for `SymbolMap::search`), so they are not memory-bound in a way that 32 checkers make worse. Their absolute
  cost is the CPU lane's instruction work.
- Hot/cold splits of hot structs: no struct's miss share stands out (`Node::text`, `Node::modifiers`,
  `Node::initializer` each under 1.5% of DRAM fills), and the packed layouts (notes/mem-layout*.md) leave no room to
  move fields without growing a struct.

## 5. Conclusion

At 32 checkers on the 64-vCPU runner the check phase does not lose cycles to cache misses, TLB misses, false sharing or
memory bandwidth in any measurable amount. User-mode IPC is 1.92 -> 1.96 (vscode) and 2.16 -> 2.29 (t3code-server) from
1 to 32 checkers. Every per-1,000-instruction miss rate is within ±25% of the one-checker value, and the absolute rates
are low (0.03 cross-CCX fills, 0.02 page walks, 0.16 DRAM demand fills per 1,000 instructions). The summed checker CPU
at 32 checkers grows exactly as the instructions do. The lever for the check phase at 32 checkers is therefore the
duplicated instructions (notes/mem-per-checker-duplication.md), and the heavy-file tail is the other
(notes/perf-heavy-files-infer-memo.md). Neither is a memory-layout problem. The largest single miss site (the
`ReferenceInstantiations` rehash, 1.3% of t3code-server's cycles) was tried two exact ways without a measurable change.
