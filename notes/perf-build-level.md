# perf-build-level: speed from how the binary is built and laid out (Linux x86-64)

Question: on top of the shipped PGO + fat-LTO build, what do BOLT, huge pages for the program text,
`panic = "abort"`, another allocator, `opt-level = "s"` for cold crates and a larger PGO training set give? Every
binary here is a PGO + fat-LTO `dist` build made with the release workflow's commands; nothing changes source code
except the allocator switch.

Answer: **BOLT** gives -2.7% / -3.2% / -1.0% wall time with 1 / 4 / 8 checkers on the 38k-file codebase and -3.4% to
-4.0% on vscode, with identical output and gate results; proposed for the Linux release builds (PR "release: BOLT for
the Linux binaries"). Everything else is rejected: text huge pages add nothing measurable on top of it, `panic =
"abort"` crashes (a latent memory bug, below), mimalloc v3 (current) beats mimalloc v2, jemalloc and glibc by 3-14%,
`opt-level = "s"` for the cold crates and a training set with vscode are inside the noise.

## Host and method

Cloud sandbox microVM, Intel Xeon Platinum 8375C @ 2.90 GHz (Ice Lake), 18 vCPUs, 47 GiB, no swap, kernel 7.2.9,
THP `enabled` = `madvise` (read-only file THP available: BOLT's `-hugify` gets `FilePmdMapped` text). rustc 1.99.0
(LLVM 23.1.1), BOLT from the LLVM 23.1.1 release tarball, `cargo-pgo` 0.3.0 installed but not used (the pipeline below
is plain `llvm-bolt`, which is what a workflow step runs). All runs in one session on one host.

Every run: `tsrs -p . --noEmit --incremental false --pretty false --extendedDiagnostics --checkers N` under
`perf stat -e instructions,cycles,branch-misses,iTLB-load-misses,L1-icache-load-misses,itlb_misses.walk_active,icache_16b.ifdata_stall`
and `/usr/bin/time`. Binaries interleaved, order rotated every round; tables give medians (wall range in brackets) and
the median of the per-round paired ratios against the baseline (range in brackets). The 38k-file codebase is checked
with its workspace packages unbuilt (40,542 error lines, the same md5 in every run of every binary); vscode is
`src` at the bench commit, held out of every training set. This host's round-to-round wall spread is +-5-10%; a
PGO build relinked with `--emit-relocs` (same code, shifted addresses) moved by -1.5% / -0.6% / +1.9%, which is the
size of a layout accident here.

## Reproducing the release build

origin/main 68636c6, the workflow's commands, clean target directories, 18 cores:

| step | time |
| --- | --- |
| instrumented build (tsrs, tsrs-test, tsrs-fourslash) | 415 s |
| training (`.github/scripts/pgo-train.sh`, bench projects already cloned) | 39 s |
| `llvm-profdata merge` | < 1 s |
| final build (tsrs) | 246 s (365 s for tsrs + tsrs-test + tsrs-fourslash, with other builds running) |
| BOLT (`.github/scripts/bolt.sh` on the PR branch: instrument 3 binaries, train, optimize, gates, byte comparison) | 224 s, plus 18 s to fetch the tools |

## Summary

Big corpus, paired against the PGO binary (the allocators against mimalloc v3 built from the same patched tree),
1 / 4 / 8 checkers. Size: unstripped `.text`, and the stripped file.

| experiment | wall | cycles | instructions | max RSS | size | verdict |
| --- | --- | --- | --- | --- | --- | --- |
| BOLT (12 rounds) | -2.7% / -3.2% / -1.0% | -2.7% / -3.0% / -0.9% | -0.2% / -0.3% / 0.0% | 0 | hot text 2.8 MB + cold 2.3 MB next to the original 13.6 MB; stripped 22.3 -> 30.2 MB | **proposed** |
| BOLT on vscode (7 rounds) | -3.4% / -4.0% / -3.9% | -2.6% / -3.8% / -3.7% | -0.2% | 0 | | |
| BOLT + `-hugify` (text on 2 MiB pages, 7 rounds) | -4.9% / -2.4% / -2.7% | -4.7% / -3.4% / -2.4% | -0.2% | 0 | +1.4 MB | rejected: same as BOLT within noise |
| `panic = "abort"` | crashes | - | - | - | `.eh_frame` -0.26 MB, `.gcc_except_table` -0.45 MB; stripped +0.65 MB | rejected (below) |
| mimalloc v2 (2.3.2) | +6.0% / +2.6% / +0.3% | +7.8% / +3.8% / +1.7% | +0.7% / +0.6% / +0.3% | -3.5% / -3.7% / -2.7% | same | rejected |
| jemalloc (tikv-jemallocator 0.6.1, 5.3.0) | +4.1% / +5.4% / +4.5% | +7.4% / +6.4% / +3.1% | +2.7% / +3.1% / +2.7% | -6.6% / -6.4% / -6.2% | +0.4 MB | rejected |
| glibc malloc | +11.4% / +12.3% / +14.3% | +10.2% / +8.5% / +6.3% | +6.2% / +6.1% / +5.6% | -6.3% / -6.3% / -5.9% | same | rejected |
| `opt-level = "s"` for 13 cold crates | -0.6% / +1.5% / -1.1% | -0.3% / +0.7% / -0.9% | -0.1% / 0.0% / -0.2% | 0 | `.text` 13.61 -> 12.81 MB, stripped -0.77 MB | rejected: noise |
| PGO training + vscode at 8 checkers | +0.5% / -0.1% / -1.4% | +0.7% / +1.8% / -0.9% | -3.3% / -2.8% / -2.8% | 0 | `.text` +0.4 MB | left as is |

PGO binary at 4 checkers for scale: 14.67 s wall, 201.9 G cycles, 297.3 G instructions, 5.88 GiB peak.

## 1. BOLT

Pipeline (instrumentation mode; the VM has no LBR): the final PGO build linked with `-Clink-arg=-Wl,--emit-relocs`
(the `.text` is the same size and code, at addresses 48 bytes later), `llvm-bolt -instrument` on each of tsrs,
tsrs-test and tsrs-fourslash, each run on its part of `pgo-train.sh` (tsrs: xstate-main and webpack; tsrs-test: the
conformance suite; tsrs-fourslash: the fourslash suite), `merge-fdata`, then `llvm-bolt -reorder-blocks=ext-tsp
-reorder-functions=cdsort -split-functions -split-all-cold -split-eh -icf=1 -use-gnu-stack` (cargo-pgo's defaults)
plus `-update-debug-sections` (without it BOLT drops the line tables that panic backtraces use; the code is
byte-identical either way). Training tsrs takes 5 s instrumented; 92.8% of the big corpus's cycles land in the
reordered hot text, 0.2% in functions the profile never saw.

| checkers | bin | n | wall s (range) | cycles G | instr G | max RSS GiB | br-miss M | iTLB miss M | L1i miss M | iTLB walk cyc % | icache stall cyc % | md5s | size MB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | bolt | 7 | 42.43 (38.00-43.63) | 159.4 | 216.3 | 4.49 | 736 | 38.4 | 4672 | 1.53 | 3.95 | fac13d | 34.1 |
| 1 | bolt-huge | 7 | 40.23 (38.96-43.56) | 152.4 | 216.2 | 4.48 | 737 | 38.2 | 4676 | 1.38 | 3.81 | fac13d | 35.4 |
| 1 | pgo | 7 | 42.26 (41.27-45.78) | 160.3 | 216.6 | 4.49 | 745 | 31.5 | 5859 | 1.47 | 5.07 | fac13d | 101.7 |
| 4 | bolt | 7 | 14.47 (13.70-15.87) | 198.5 | 297.1 | 5.88 | 918 | 44.2 | 5914 | 1.41 | 3.72 | fac13d | 34.1 |
| 4 | bolt-huge | 7 | 14.25 (13.73-16.43) | 197.7 | 296.7 | 5.88 | 919 | 45.4 | 5926 | 1.30 | 3.63 | fac13d | 35.4 |
| 4 | pgo | 7 | 14.67 (14.20-16.42) | 201.9 | 297.3 | 5.88 | 927 | 36.2 | 7422 | 1.33 | 4.71 | fac13d | 101.7 |
| 8 | bolt | 7 | 11.35 (10.35-11.64) | 269.7 | 398.8 | 7.56 | 1155 | 52.9 | 7511 | 1.27 | 3.45 | fac13d | 34.1 |
| 8 | bolt-huge | 7 | 11.22 (9.94-12.82) | 272.8 | 398.2 | 7.57 | 1152 | 53.6 | 7498 | 1.14 | 3.31 | fac13d | 35.4 |
| 8 | pgo | 7 | 11.09 (10.95-11.70) | 271.3 | 399.1 | 7.56 | 1161 | 43.5 | 9356 | 1.20 | 4.35 | fac13d | 101.7 |

(This is the second session, 7 rounds; the first, 5 rounds, gave -2.4% / -5.1% / -1.0% wall. The summary row pools
both.) vscode:

| checkers | bin | n | wall s (range) | cycles G | instr G | max RSS GiB | br-miss M | iTLB miss M | L1i miss M | iTLB walk cyc % | icache stall cyc % | md5s | size MB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | bolt | 7 | 18.22 (16.55-19.01) | 71.9 | 99.4 | 2.30 | 372 | 25.2 | 2502 | 1.88 | 4.56 | 61f936 | 34.1 |
| 1 | bolt-huge | 7 | 17.48 (16.61-19.38) | 69.4 | 99.4 | 2.30 | 371 | 25.3 | 2493 | 1.70 | 4.32 | 61f936 | 35.4 |
| 1 | pgo | 7 | 18.61 (17.66-19.50) | 73.3 | 99.6 | 2.31 | 376 | 17.5 | 3098 | 1.63 | 5.75 | 61f936 | 101.7 |
| 4 | bolt | 7 | 4.87 (4.68-5.26) | 71.5 | 104.4 | 2.58 | 392 | 25.5 | 2613 | 1.79 | 4.66 | 61f936 | 34.1 |
| 4 | bolt-huge | 7 | 4.91 (4.79-5.18) | 71.9 | 104.4 | 2.58 | 392 | 25.6 | 2615 | 1.63 | 4.82 | 61f936 | 35.4 |
| 4 | pgo | 7 | 5.06 (4.73-5.75) | 74.3 | 104.6 | 2.57 | 399 | 18.2 | 3252 | 1.54 | 5.65 | 61f936 | 101.7 |
| 8 | bolt | 7 | 2.81 (2.69-3.00) | 74.4 | 108.9 | 2.82 | 411 | 26.0 | 2731 | 1.80 | 4.41 | 61f936 | 34.1 |
| 8 | bolt-huge | 7 | 2.78 (2.68-2.93) | 74.5 | 108.9 | 2.82 | 411 | 26.0 | 2734 | 1.62 | 4.29 | 61f936 | 35.4 |
| 8 | pgo | 7 | 2.85 (2.73-3.05) | 76.2 | 109.1 | 2.83 | 417 | 17.9 | 3395 | 1.44 | 5.59 | 61f936 | 101.7 |

What changes: L1 instruction-cache misses -20%, cycles stalled on instruction fetch (`icache_16b.ifdata_stall`) from
4.3-5.1% of cycles to 3.4-4.0%, branch misses -1%. Instructions barely move (-0.2%): the gain is fetch, not work.
iTLB misses go up 23-30% (the hot code now sits in a new segment away from the original text, which cold paths
still use), and the page-walk cycles stay at 1.1-1.8% of cycles.

Gates, on BOLT-optimized binaries from the same pipeline: conformance 13,458 pass / 0 crash / 0 timeout (gate
minimum 13,458); `tsrs-test run --suite all --baselines types,symbols,js,jsmap,sourcemap` result trees identical to
the PGO-only binary's (13,462 / 12,779 / 12,779 / 13,392 / 149 / 156 pass); fourslash 4,066 pass / 63 fail (gate
4,066 / 63), result trees identical. The shipped `tsrs` itself cannot run those suites (they are other binaries),
so its stdout was compared byte for byte with the PGO binary's: the 38k-file codebase at 1, 4 and 8 checkers
(8,213,694 bytes each) and vscode at 1 and 4 checkers: identical, BOLT and BOLT + hugify.

Proposed: `.github/scripts/bolt.sh` and two steps in `release.yml` for the two Linux targets, which run the gates on
the BOLT-optimized tsrs-test and tsrs-fourslash and the byte comparison of tsrs on both bench projects before the
binary is staged. aarch64 was not measured here (BOLT supports its instrumentation; the workflow's dry run is the
check). Not changed: `.depot/workflows/bench.yml`, which builds the README benchmark's binary with the release
commands, would need the same step to keep measuring what ships. (Status 2026-10-10: bench.yml now runs `bolt.sh`
too; notes/perf-binary-layout.md.)

## 2. Huge pages for the program text

The PGO binary spends 1.2-1.5% of its cycles in instruction-side page walks (`itlb_misses.walk_active`), just over
the 1% bar. BOLT's `-hugify` places the hot text on 2 MiB boundaries and remaps it at startup; the running process
showed 4 MiB of `FilePmdMapped` text. Against BOLT alone it is within noise in both directions (-4.9% vs -2.7% wall
with one checker, -2.4% vs -3.2% with four), iTLB misses and walk cycles do not fall, and it adds a startup remap.
Rejected. A 2 MiB-aligned, `MADV_HUGEPAGE`-advised text for the PGO-only binary was not built: the remap reaches the
same pages more directly and gained nothing measurable.

## 3. `panic = "abort"`

Update: the crash is fixed (notes/fix-arena-recycle-uaf.md): `recycle_mapper_with_targets` freed a list it held as a
reference parameter, and LLVM deleted the free-list link write. The reasoning below about shipping it still holds;
the speed was not re-measured.

Not measurable: every `panic = "abort"` build segfaults. The PGO build (trained on the conformance suite only, since
the instrumented binary itself crashed on xstate-main in 12 of 12 tries and on webpack) and a plain `cargo build
--release` with `CARGO_PROFILE_RELEASE_PANIC=abort` both crash on xstate-main, webpack, vscode and the 38k-file
codebase, with one checker too, in `tsrs_core::arena` `pop_free` (`arena.rs:343`): the next pointer read from a
recycled block is garbage, i.e. a freed slice (inference-info or type lists from `alloc_slice_recycled`) was written
after it was freed. The same `--release` build with debug assertions (which poison freed blocks) does not crash.
No panic happens in these runs, so the abort strategy only changes code generation: every call becomes `nounwind`,
which lets LLVM move stores across calls. That points to an aliasing or lifetime bug in the arena recycling that
the unwind build happens not to expose, not to anything panic-related. It deserves its own investigation (Miri on
`tsrs_core`, docs/RUST.md "Not tried", is the tool), since a different inlining decision could expose it in the
shipped build.

What shipping it would need even then: the language server recovers from a panicking request
(`tsrs_lsp` `recover`, Go's `recover()`), the background queue keeps serving after a panicking task, the Node API
turns a panic into an error response and unwinds through `callbackfs` on purpose, the project builder rolls back on
unwind (`tsrs_project/src/snapshot.rs`), and the fourslash runner uses panics for failed and skipped tests. A
CLI-only binary or a second profile would be needed. The size gain is the unwind tables (0.7 MB); the speed gain is
unknown but cannot be large: PGO already moves landing pads out of the hot path. Not worth a second binary.

## 4. Allocator

The allocator at this measurement was mimalloc **v3** (3.3.2: `libmimalloc-sys` 0.1.49 builds v3 unless its `v2`
feature is on), default options (no `secure`, no `extended`; its own `MADV_HUGEPAGE` advice). Variants built from one patched
tree (a `tikv-jemallocator` dependency in tsrs_cli, the global allocator picked by `--cfg`, mimalloc v2 by
`CARGO_FEATURE_V2=1` so no crate's metadata hash changes) with the base PGO profile (tsrs_cli's own functions lose
their profile in all four, the same for each).

| checkers | bin | n | wall s (range) | cycles G | instr G | max RSS GiB | br-miss M | iTLB miss M | L1i miss M | iTLB walk cyc % | icache stall cyc % | md5s | size MB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | je | 5 | 46.65 (45.04-47.89) | 175.1 | 222.6 | 4.19 | 743 | 42.5 | 6072 | 1.82 | 5.04 | fac13d | 103.4 |
| 1 | mi2 | 5 | 46.86 (43.71-48.60) | 175.6 | 218.1 | 4.34 | 743 | 34.1 | 5928 | 1.52 | 4.90 | fac13d | 102.6 |
| 1 | mi3 | 5 | 44.65 (42.44-45.24) | 162.9 | 216.7 | 4.49 | 741 | 33.6 | 5870 | 1.57 | 5.06 | fac13d | 102.7 |
| 1 | sys | 5 | 48.22 (47.11-50.60) | 179.0 | 230.0 | 4.21 | 847 | 41.3 | 6370 | 1.67 | 4.97 | fac13d | 102.3 |
| 4 | je | 5 | 16.34 (15.60-16.90) | 224.2 | 306.0 | 5.50 | 925 | 56.1 | 7739 | 1.85 | 4.80 | fac13d | 103.4 |
| 4 | mi2 | 5 | 15.66 (15.14-16.44) | 217.0 | 298.8 | 5.66 | 927 | 40.3 | 7510 | 1.35 | 4.71 | fac13d | 102.6 |
| 4 | mi3 | 5 | 14.97 (14.86-16.02) | 205.2 | 297.1 | 5.88 | 923 | 37.8 | 7453 | 1.34 | 4.62 | fac13d | 102.7 |
| 4 | sys | 5 | 17.18 (16.47-17.32) | 228.0 | 315.0 | 5.51 | 1054 | 48.8 | 8090 | 1.55 | 4.84 | fac13d | 102.3 |
| 8 | je | 5 | 11.78 (11.46-12.00) | 282.0 | 412.1 | 7.10 | 1159 | 72.1 | 9737 | 1.87 | 4.45 | fac13d | 103.4 |
| 8 | mi2 | 5 | 11.04 (10.84-11.87) | 269.6 | 402.4 | 7.36 | 1159 | 49.3 | 9438 | 1.36 | 4.27 | fac13d | 102.6 |
| 8 | mi3 | 5 | 11.17 (10.59-12.08) | 274.0 | 400.9 | 7.57 | 1154 | 47.3 | 9374 | 1.27 | 4.24 | fac13d | 102.7 |
| 8 | sys | 5 | 12.70 (12.13-13.18) | 293.6 | 422.7 | 7.11 | 1317 | 59.0 | 10176 | 1.40 | 4.40 | fac13d | 102.3 |

mimalloc v3 is fastest at every checker count; the others use 3-7% less peak memory and cost 3-14% wall. Keep it.

## 5. `opt-level = "s"` for cold crates

`[profile.dist.package.<crate>] opt-level = "s"` for tsrs_ls, tsrs_lsproto, tsrs_lsp, tsrs_project,
tsrs_projectutil, tsrs_api, tsrs_api_transport, tsrs_api_codec, tsrs_fswatch, tsrs_transformers, tsrs_declarations,
tsrs_sourcemap, tsrs_astnav. Opt-level is part of cargo's crate metadata hash, so this build has its own instrumented
build and training. `.text` shrinks by 0.8 MB (5.9%); the check run does not change (table below, "os"). The cold
crates are already laid out away from the hot code by PGO, so shrinking them does not tighten the hot text. Rejected.

## 6. PGO training set

The base instrumented binaries, plus one more run: vscode `src` at 8 checkers, merged with the release training
profiles ("train-vscode" below). It retires 2.8-3.3% fewer instructions on the 38k-file codebase, but cycles and wall
move by -1.4% to +1.8%, inside this host's spread, with no consistent sign. The training set still covers the hot
paths; left as is.

| checkers | bin | n | wall s (range) | cycles G | instr G | max RSS GiB | br-miss M | iTLB miss M | L1i miss M | iTLB walk cyc % | icache stall cyc % | md5s | size MB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | os | 5 | 42.74 (40.86-44.20) | 160.6 | 216.4 | 4.48 | 744 | 31.9 | 5790 | 1.53 | 4.99 | fac13d | 94.7 |
| 1 | pgo | 5 | 43.51 (42.01-44.47) | 163.3 | 216.6 | 4.49 | 746 | 30.7 | 5866 | 1.40 | 4.93 | fac13d | 101.7 |
| 1 | train-vscode | 5 | 42.20 (41.84-45.65) | 159.1 | 209.5 | 4.49 | 750 | 28.4 | 5940 | 1.58 | 5.60 | fac13d | 104.6 |
| 4 | os | 5 | 15.45 (14.62-16.75) | 211.8 | 296.9 | 5.87 | 926 | 38.7 | 7359 | 1.37 | 4.70 | fac13d | 94.7 |
| 4 | pgo | 5 | 15.37 (14.47-17.24) | 207.1 | 296.9 | 5.86 | 926 | 38.1 | 7446 | 1.35 | 4.57 | fac13d | 101.7 |
| 4 | train-vscode | 5 | 15.35 (14.30-16.30) | 210.7 | 288.5 | 5.88 | 935 | 33.7 | 7540 | 1.40 | 5.12 | fac13d | 104.6 |
| 8 | os | 5 | 11.28 (10.53-11.83) | 279.0 | 398.8 | 7.57 | 1161 | 52.9 | 9295 | 1.33 | 4.18 | fac13d | 94.7 |
| 8 | pgo | 5 | 11.69 (10.67-11.94) | 277.7 | 399.4 | 7.55 | 1165 | 44.5 | 9431 | 1.23 | 4.27 | fac13d | 101.7 |
| 8 | train-vscode | 5 | 11.27 (10.52-11.99) | 275.1 | 388.6 | 7.56 | 1170 | 41.2 | 9449 | 1.29 | 4.51 | fac13d | 104.6 |

## Not done

- Combining BOLT with a larger BOLT training set: 99.8% of the big corpus's cycles already run in profiled code.
- `-reorder-functions=hfsort+` or other BOLT layouts, and a function order file at link time without BOLT.

## Reproducing

```sh
# PGO as released: .github/workflows/release.yml; then, for BOLT:
RUSTFLAGS="-Cprofile-use=<merged.profdata> -Clink-arg=-Wl,--emit-relocs" cargo build --profile dist --locked \
  -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash
PATH=<llvm-23.1.1>/bin:$PATH .github/scripts/bolt.sh target/dist <work> <pgo bench dir>
# counters
perf stat -e instructions,cycles,branch-misses,iTLB-load-misses,L1-icache-load-misses,itlb_misses.walk_active,icache_16b.ifdata_stall -- \
  tsrs -p . --noEmit --incremental false --pretty false --extendedDiagnostics --checkers 4
```
