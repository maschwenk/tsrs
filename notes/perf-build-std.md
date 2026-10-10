# perf-build-std: std built with tsrs's profile, a non-PIE Linux binary, and other build flags

Question: on top of the shipped build (fat LTO, one codegen unit, PGO, BOLT on Linux), what do these give?

- B: the standard library compiled from source inside the workspace build (`-Zbuild-std=std,panic_unwind`, on the
  pinned stable 1.99.0 through `RUSTC_BOOTSTRAP=1`), so std is PGO-instrumented, trained and optimized with tsrs's
  profile instead of shipping as the stock prebuilt rlibs.
- C: B + `-Zlocation-detail=none`.
- D: B + a non-PIE executable (`-Crelocation-model=static -Clink-arg=-no-pie`), x86_64 Linux only (macOS requires PIE).
  DA: the same on the shipped build without build-std.
- E: `-Ztune-cpu=znver4` (instruction scheduling for Zen 4/5; the ISA stays baseline x86-64).
- F: `-Cno-vectorize-loops -Cno-vectorize-slp` (no auto-vectorization).

Answer: **non-PIE on x86_64 Linux: -0.9% to -2.3% single-threaded instructions on all 17 bench projects (1% or more on
16; mui-docs -0.91%), adopted** in release.yml, bench.yml and bench-compare.yml. Rejected: build-std (-0.04% to -0.27%
on the probe, -0.42% to +0.27% on the bench), `-Zlocation-detail=none` (no speed; -6.6% stripped binary),
`-Ztune-cpu=znver4` (-0.39% to +0.02%), no auto-vectorization (-0.17% to +0.04%, `.text` -0.1%, instruction-cache misses
not lower). `panic=immediate-abort` is not viable and was not measured. Output was byte-identical across every variant
on every project. Wall time moved inside the runners' noise for every variant.

## Method

- Every binary is built with the release pipeline's commands: instrumented build of tsrs + tsrs-test + tsrs-fourslash,
  `.github/scripts/pgo-train.sh`, `llvm-profdata merge`, final build (`--emit-relocs` and
  `CARGO_PROFILE_DIST_STRIP=none` on Linux, `symbols` on macOS, the same value in both builds), and on Linux
  `.github/scripts/bolt.sh` with `BOLT_TSRS_ONLY=1`, as bench.yml. A variant's flags go into both PGO builds.
- Linux probe: a one-off Depot workflow on one `depot-ubuntu-24.04-32` (AMD EPYC 9R45, 32 vCPU, 126 GB) built each
  variant from scratch and measured them interleaved with `tools/perf/layout-measure.py` (3-5 rounds, `--checkers 1` and
  the default count, `perf stat` for one `--singleThreaded` run, and the byte comparison of every binary's output at 1,
  4 and 32 checkers), then `bench/count.py` (user-space instructions and peak RSS of one `--singleThreaded` run with
  `RAYON_NUM_THREADS=1`, the regression flag's measure, deterministic to about 0.001%). Runs `cgjxc6xwcl` (A B C D, 3
  rounds), `67mc3vb2kc` (A DA, 5 rounds) and `g9fjgvd826` (A E F, 5 rounds). Each probe builds and trains its own A; the
  three A builds' counts agree within 0.2%. The workflow and its build script stayed on the branch
  `perf/build-std-probe` (not merged).
- Linux bench: `.depot/workflows/bench.yml` with the variant wired into its build job, started from the branch with
  `depot ci run` (runs `hqqzfwf8r9` for B, `swsvn5kjl3` for DA), compared with main's two publishes at the same source
  (7c3e1948 and 9c905fb5: docs-only commits apart). Same CPU model and glibc in all four runs. Between the two main
  publishes the counts moved by -0.04% to +0.10% (PGO retraining).
- macOS: Apple M-series, 18 cores, other agents building at load 6-20. Three interleaved rounds of `/usr/bin/time -l`,
  medians. macOS counts include kernel work and move by a few percent between runs, so they can rule out a large change
  only.

cal-diy's bench input changed under the branch runs: the measure job missed its cache, reinstalled the project and
checked 10,227 files with 144 errors (tsgo also 144), against 10,170 and 136 on main. Its bench rows below compare the
two branch runs with each other; the probes ran every variant on the same checkout.

## Results, Linux probe (bench/count.py, single-threaded)

| project | A instructions | B build-std | C B + location-detail | D B + non-PIE | DA A + non-PIE (second probe) | D vs B |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 85.054 G | -0.05% | -0.18% | -2.15% | -2.24% | -2.10% |
| xstate-main | 5.899 G | -0.23% | -0.23% | -1.49% | -1.50% | -1.27% |
| webpack | 11.310 G | -0.08% | -0.15% | -1.91% | -1.96% | -1.83% |
| mui-docs | 40.578 G | -0.13% | -0.15% | -0.99% | -1.08% | -0.86% |
| cal-diy | 32.257 G | -0.27% | -0.30% | -1.37% | -1.24% | -1.10% |
| formbricks-web | 40.713 G | -0.22% | -0.25% | -1.42% | -1.32% | -1.20% |
| supabase-studio | 47.505 G | -0.18% | -0.21% | -1.72% | -1.42% | -1.54% |
| t3code-server | 41.450 G | -0.04% | -0.05% | -1.41% | -1.51% | -1.37% |
| drizzle-orm | 11.909 G | -0.25% | -0.29% | -1.90% | -1.81% | -1.65% |

DA is against its own probe's A (85.004 G on vscode, within 0.06% of the first probe's A).

Peak RSS (same runs): B and C within -0.19% to +0.06%; D and DA -0.05% to -1.0% (-0.4 to -2.2 MiB: no relocated
`.data.rel.ro` pages).

Cycles (`perf stat`, one run each, so a percent or two of noise), DA vs A: vscode -2.1%, xstate-main -1.4%, webpack
-2.6%, mui-docs +0.4%, cal-diy +0.6%, formbricks-web -1.5%, supabase-studio -1.1%, t3code-server +1.1%, drizzle-orm
-1.8%. B vs A: -0.6% to +1.2% on seven projects; on supabase-studio and t3code-server the one run of A took 7-17% more
cycles than B, C and D alike. The generic `L1-icache-load-misses` event reads 0 on this VM.

Wall, medians of 5 rounds, DA vs A: `--checkers 1` -2.3% to +0.9% (the baseline's spread ±0.5% to ±4.6%); default
checker count (32 here) -6.9% to +2.5% (spread ±2.2% to ±8.7%). Inside the noise either way; B, C and D likewise (3
rounds).

Sizes, x86_64, BOLT-optimized and stripped as shipped:

| | A | B | C | D | DA |
| --- | ---: | ---: | ---: | ---: | ---: |
| stripped binary | 32.63 MB | 32.64 MB | 30.48 MB (-6.6%) | 30.78 MB (-5.7%) | 30.82 MB (-5.6%) |
| `.text` before BOLT | 14.68 MB | 14.58 MB | 14.48 MB | 14.36 MB | 14.52 MB |
| `R_X86_64_RELATIVE` relocations | 41,068 | 41,010 | 31,046 | 0 | 0 |

## Results, Linux bench (bench/count.py, 17 projects)

Single-threaded instructions against the mean of main's two publishes:

| project | main | build-std | non-PIE | non-PIE peak RSS |
| --- | ---: | ---: | ---: | ---: |
| vscode | 84.65 G | -0.02% | -2.09% | -0.09% |
| xstate-main | 5.88 G | -0.09% | -1.50% | -1.00% |
| webpack | 11.27 G | -0.02% | -1.89% | -0.57% |
| mui-docs | 40.44 G | -0.07% | -0.91% | -0.22% |
| Compiler | 1.86 G | +0.05% | -2.27% | -3.26% |
| Compiler-Unions | 4.41 G | +0.04% | -1.04% | -3.26% |
| cal-diy (other input, see above) | 32.11 G (build-std) | | -1.05% vs build-std | -0.20% |
| formbricks-web | 40.61 G | -0.25% | -1.37% | -0.17% |
| supabase-studio | 47.35 G | -0.27% | -1.35% | -0.14% |
| t3code-server | 41.32 G | -0.03% | -1.54% | -0.22% |
| mikro-orm | 69.06 G | +0.09% | -1.00% | -0.16% |
| next-packages-next | 18.88 G | +0.00% | -1.50% | -0.40% |
| next-root | 12.76 G | -0.13% | -1.79% | -0.47% |
| storybook | 10.17 G | -0.35% | -1.45% | -0.52% |
| nuxt | 9.23 G | -0.42% | -1.55% | -0.48% |
| playwright | 8.04 G | -0.14% | -1.65% | -0.60% |
| drizzle-orm | 11.87 G | -0.22% | -1.89% | -0.50% |

Files, error counts and type counts were the same as main's on the 16 projects with the same input. The build-std run's
16-vCPU default-mode wall (the README table) was -1.8% to +9.6% against the mean of main's two publishes, which is the
runner's noise. The non-PIE run's: -2.5% to +9.8%, median +0.6%, while main's two publishes of the same code differ by a
median -1.2% and up to 9.5% (vscode). The headline needs more publishes to show a wall change of this size, if there is
one.

## Results, macOS (build-std and location-detail; non-PIE does not apply)

Medians of three rounds; the minimum of three in brackets.

| project | A single | B | C | A default | B | C |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 84.36 G | +0.13% [-0.19%] | -0.58% [-0.17%] | 93.21 G | +0.15% | +0.03% |
| webpack | 11.16 G | -0.12% [-0.00%] | -0.05% [+0.04%] | 14.63 G | +0.69% | +0.80% |
| xstate-main | 6.38 G | -2.50% [+0.08%] | +0.32% [+0.05%] | 8.68 G | +0.17% | -0.20% |
| drizzle-orm | 13.09 G | -2.76% [-0.06%] | +0.35% [-0.11%] | 17.03 G | -0.33% | +1.71% |
| cal-diy | 34.09 G | -0.19% [-0.11%] | +2.23% [+0.05%] | 68.00 G | -0.20% | +0.90% |
| formbricks-web | 44.05 G | -0.11% [+0.40%] | -1.28% [-0.15%] | 66.93 G | +0.18% | +0.20% |
| t3code-server | 40.45 G | +0.22% [+0.18%] | -0.03% [+0.25%] | 114.76 G | +0.43% | +0.77% |
| mui-docs | 45.38 G | -0.14% [+2.14%] | -0.82% [-0.37%] | 105.43 G | -0.05% | -0.52% |
| supabase-studio | 50.74 G | -0.95% [-0.86%] | -0.19% [-0.99%] | 78.77 G | +0.24% | +0.03% |

Peak RSS within ±1% (vscode 1,605 MiB in all three). Diagnostics byte-identical in all 108 comparisons, and the
single-threaded `--extendedDiagnostics` counters too. Stripped binary: A 20.86 MB, B 20.66 MB, C 20.21 MB. Build time:
B's instrumented build took 3 min 58 s against A's 3 min 47 s (concurrent builds; on the probe runner five variants
built in parallel in 246 s).

## Why build-std gives so little

Fat LTO already merges the prebuilt std's bitcode (the rlibs ship it), so cross-crate inlining into std was already
there, and std's generic code (`Vec`, sorting, hash tables, iterators, formatting of tsrs's types) is instantiated in
tsrs's crates and instrumented with them. What build-std adds is profile data for std's own non-generic functions. In
the macOS training profile those are 2,683 more functions and +2.7% block counts; the hottest are `memchr_aligned` and
`memrchr_aligned` (0.42% and 0.28% of the counts), `StrSearcher::new` (0.37%), `Vec<u8>::clone` (0.33%), the default
allocator's shims, which tsrs-fourslash (no mimalloc) reaches (0.31% + 0.30%), `Utf8Chunks::next` and `str::from_utf8`.
With a few percent of the work in that code, a PGO gain there cannot reach 1% of the total; it measured 0.04-0.27%.

## Why non-PIE helps

In a PIE every address of a static, a vtable or a string is computed RIP-relative, and a `match` compiled to a jump
table loads a 32-bit offset and adds the table's address before the jump. Disassembly of B and D (the same code but for
the relocation model): B has 106,248 RIP-relative `lea`s and 2,559 `movslq (base,index,4)` loads feeding 2,480
register-indirect jumps; D has 70,087 `movl $address` immediates (shorter encodings for the same work), 789 compares
against an address immediate, 880 loads that fold an absolute address into the addressing mode, and 2,442 jump tables
that are a single `jmp *table(,%reg,8)`. The parser, scanner and checker dispatch on node kinds through those tables.
The loader also no longer reads 0.99 MB of relocation entries and writes the 0.9 MB of `.data.rel.ro` they patch (41,068
relocations) at startup; in the non-PIE binary those pages are touched only when used, which is about the 0.4-2.2 MiB
peak-RSS change.

What it costs: the executable image always loads at the same address (the stack, heap, mmap regions and shared libraries
are still randomized). The reference compiler ships the same way: `typescript@7.0.2`'s linux-x64 `tsc` is an
`ELF 64-bit LSB executable`, not a `pie executable` (Go's default for linux/amd64). BOLT handles the non-PIE binary
unchanged (`--emit-relocs` is still needed), and bolt.sh's byte comparison passed. Not applied: aarch64 Linux (not
measured; it addresses statics with `adrp`/`add` and uses PC-relative jump tables with or without PIE, so the savings
here need not carry over) and macOS (arm64 requires PIE). With `--target` given explicitly, RUSTFLAGS do not reach build
scripts and proc macros, which must stay position-independent.

## build-std details

- `-Cprofile-generate` works with build-std without extra setup: `profiler_builtins` is `#![no_core]` and rustc loads
  the prebuilt one from the sysroot. (Its build script also takes `LLVM_PROFILER_RT_LIB`; not needed.)
- `RUSTC_BOOTSTRAP=1` reaches build scripts, which could switch nightly features on. The `rustc-cfg` output of every
  dependency's build script was the same as without it; only std's own crates add theirs.
- `compiler_builtins` gets the `dist` profile's `codegen-units = 1`: one object instead of the prebuilt rlib's ~370
  (std's workspace builds it with `codegen-units = 10000` so that every intrinsic is its own object). It linked on macOS
  arm64 and Linux x86-64; a target whose linker cared would need
  `--config 'profile.dist.package.compiler_builtins.codegen-units=10000'`.
- The Linux `dist` build keeps debug information from the prebuilt std (`with debug_info` on the pre-BOLT binary, 43.1
  MB) that a std built with `debug = false` does not have (38.2 MB); the shipped binaries are stripped, so the same
  size.
- `--locked` works; std's dependencies resolve from rust-src's `library/Cargo.lock` and are downloaded from crates.io
  during the build. The `rust-src` component has to be installed (it was not, for 1.99.0, on the Mac).
- `-Zbuild-std` needs `--target`, which every release and bench build already passes, so the paths and the BOLT step do
  not change.

## Not viable: panic=immediate-abort

In 1.99 the `panic_immediate_abort` std feature only raises a compile error that points to
`-Zunstable-options -Cpanic=immediate-abort`, a panic strategy for every crate: no unwinding and no message. The CLI
catches panics (`tsrs_execute/src/build/orchestrator.rs` and the API's `build_for_api` in `tsrs_cli/src/api.rs`),
reports internal panics with their message, and the PGO training runs tsrs-fourslash, which needs unwinding. Same
reasons as `panic = "abort"` in notes/perf-build-level.md, plus the lost message; not measured. `-Zlocation-detail=none`
keeps panic messages but prints `<redacted>` for their file and line.

## E: `-Ztune-cpu=znver4`, F: no auto-vectorization

Third probe, against its own A (bench/count.py, single-threaded; peak RSS within ±0.15% for both):

| project | A | E tune-cpu=znver4 | F no-vectorize-loops, no-vectorize-slp |
| --- | ---: | ---: | ---: |
| vscode | 84.932 G | -0.39% | +0.04% |
| xstate-main | 5.889 G | -0.32% | -0.02% |
| webpack | 11.299 G | -0.38% | -0.04% |
| mui-docs | 40.536 G | +0.02% | -0.01% |
| cal-diy | 32.231 G | -0.21% | -0.17% |
| formbricks-web | 40.683 G | -0.20% | -0.12% |
| supabase-studio | 47.460 G | -0.15% | -0.04% |
| t3code-server | 41.438 G | -0.35% | -0.10% |
| drizzle-orm | 11.900 G | -0.39% | -0.07% |

- Cycles (one `perf stat` run each): E -2.6% to +0.8%, F -1.4% to -0.1%. Wall, medians of 5 rounds: E -2.6% to +1.7%, F
  -2.7% to +1.2%, inside the baseline's spread at both checker counts.
- `.text` before BOLT: A 14.69 MB, E 14.75 MB (+0.4%), F 14.67 MB (-0.1%). Auto-vectorized code is a negligible part of
  tsrs's text, so turning it off frees nothing for the instruction cache. The AMD counters (two runs each on vscode,
  t3code-server, mui-docs and supabase-studio): instruction-cache misses (`ic_tag_hit_miss`) +1.2% to +5.0% for F and
  -0.4% to +0.8% for E; fills from beyond L2 +4% to +12% for F and +7% to +28% for E.
- E needs `RUSTC_BOOTSTRAP=1` as well, and the release binaries also run on Intel. Both rejected.

## Reproducing

```sh
# macOS arm64, variant B (A: drop RUSTC_BOOTSTRAP and -Zbuild-std; C: add -Zlocation-detail=none to both RUSTFLAGS)
rustup component add rust-src --toolchain 1.99.0
RUSTC_BOOTSTRAP=1 RUSTFLAGS=-Cprofile-generate=$PWD/raw CFLAGS=-fno-profile-generate CARGO_TARGET_DIR=inst \
  cargo build --profile dist --locked -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash \
  --target aarch64-apple-darwin -Zbuild-std=std,panic_unwind
.github/scripts/pgo-train.sh inst/aarch64-apple-darwin/dist raw bench/.work
"$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin/llvm-profdata" merge -o merged.profdata raw/*.profraw
RUSTC_BOOTSTRAP=1 RUSTFLAGS=-Cprofile-use=$PWD/merged.profdata CFLAGS=-fno-profile-use \
  cargo build --profile dist --locked -p tsrs_cli --target aarch64-apple-darwin -Zbuild-std=std,panic_unwind
# Linux: the probe (branch perf/build-std-probe)
depot ci run --workflow .depot/workflows/perf-buildstd-probe.yml
```
