# SIMD UTF-8 validation at the VFS boundary (2026-10-10)

## Decision

Keep `String::from_utf8` in `tsrs_vfs::internal::decode_bytes`. An experiment with `simdutf8` did not establish
enough end-to-end benefit to meet AGENTS.md's performance gate. Single-threaded instructions changed by -0.074%
on Compiler, -0.033% on Compiler-Unions and +0.009% on vscode. Default-mode peak RSS changed by -0.401%, 0.000%
and +0.129%, respectively. These are median paired changes on macOS arm64, not Linux `bench/count.py` results.

The dependency, implementation and experimental unit tests were removed. This note records the negative result;
it does not claim that SIMD validation cannot help another workload or architecture. Revisit with a profile that
shows UTF-8 validation taking a material share of total work, or new Linux measurements that meet the gate.

## Experiment

Oxc at `4b756621b65ab54c19dc66fbdcb3cd902d662c20` uses `simdutf8::basic::from_utf8` after `fs::read`, followed by
`String::from_utf8_unchecked`, in `apps/oxfmt/src/core/utils.rs` and `crates/oxc_linter/src/utils/mod.rs`.
Its workspace dependency is `simdutf8 = { version = "0.1.5", features = ["aarch64_neon"] }`.

The tsrs candidate used the same dependency and changed only the UTF-8 validation at the end of
`crates/tsrs_vfs/src/internal/internal.rs::decode_bytes`. UTF-16 decoding and BOM removal stayed as before.
Unlike Oxc's file readers, tsrs replaces malformed UTF-8 lossily, so the candidate retained that fallback:

```rust
if simdutf8::basic::from_utf8(&s).is_ok() {
    // SAFETY: simdutf8 validated every byte, and s has not been modified since validation.
    unsafe { String::from_utf8_unchecked(s) }
} else {
    String::from_utf8_lossy(&s).into_owned()
}
```

Both implementations reuse the owned allocation for valid UTF-8. No scanner loops or other SIMD operations
were changed. Prior scanner experiments in `notes/perf-parse.md` are separate from this file-loading boundary.

## Method

Apple M3 Max, 14 cores, 36 GiB RAM, macOS 27.0.1 (26A434), rustc 1.99.0 (b940084d7), LLVM 23.1.1,
`aarch64-apple-darwin`. Both binaries used `cargo build --release --locked --offline -p tsrs_cli` (fat LTO,
one codegen unit, no PGO or BOLT).

The measured source was the working tree based on `f88d3266df0175412b6caacea8ea2080bbe1a377`, including existing
linter changes subsequently committed as `32328b8d` and uncommitted Unicode identifier changes (moving
`unicode-id-start = "=1.1.2"` to core). Those changes were present in both binaries. A SHA-256 inventory of 841
Rust source/manifests/lock files confirmed that only the four experiment files differed between builds: root
`Cargo.toml`, `Cargo.lock`, the VFS manifest and `internal.rs`. These measurements are not a clean-main result.

Workloads use the revisions in `bench/projects.json`:

- Compiler and Compiler-Unions: typescript-benchmarking `41f652ab2df5077b1115f73eaeefc2fe9f674132`, under
  `cases/solutions/Compiler` and `cases/solutions/Compiler-Unions`.
- vscode: `9cf0128b9822dcd8a533f08ef7312956a4390301`, project `src`, dependencies installed with
  `npm ci --ignore-scripts --no-audit --no-fund`.

Each project/mode had one warm-up per binary, then seven sequential pairs, alternating before/after order each
round. `/usr/bin/time -l` collected instructions retired, cycles, maximum resident set size and wall time:

```sh
/usr/bin/time -l <binary> -p <project> --noEmit --incremental false --pretty false
```

Single mode adds `--singleThreaded` and `RAYON_NUM_THREADS=1`; default mode leaves both unset. `TSRS_HISTORY`
and `TS_TEST_PROGRAM_SINGLE_THREADED` were unset in both modes. Every run returned status 2 and before/after
diagnostics were byte-identical in all 42 pairs. The projects report diagnostics in this setup, including missing
Electron declarations in vscode; this is a comparison of identical inputs and diagnostics, not a clean build.

The [raw measurements](perf-simdutf8.csv) include all 84 measured runs in execution order. Before/after values
below are independent medians; change is the median of the seven paired percentages, so it need not equal the
ratio of those medians. Negative means less. RSS is in MiB and instructions in billions.

| project | mode | instructions, before → after | paired change | peak RSS, before → after | paired change | wall seconds, before → after | paired change |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Compiler | single | 2.139604 → 2.137993 | -0.074% | 56.750 → 56.609 | -0.220% | 0.17 → 0.17 | 0.00% |
| Compiler | default | 2.521605 → 2.519368 | -0.137% | 77.984 → 77.812 | -0.401% | 0.08 → 0.08 | 0.00% |
| Compiler-Unions | single | 4.819737 → 4.818383 | -0.033% | 59.062 → 58.906 | -0.291% | 0.35 → 0.35 | 0.00% |
| Compiler-Unions | default | 5.974896 → 5.955442 | -0.065% | 82.125 → 81.953 | 0.000% | 0.17 → 0.17 | 0.00% |
| vscode | single | 97.088725 → 97.464928 | +0.009% | 1597.828 → 1597.719 | -0.007% | 10.23 → 10.42 | +2.04% |
| vscode | default | 108.656841 → 107.624108 | -0.654% | 1945.078 → 1952.734 | +0.129% | 2.72 → 2.53 | -1.57% |

The Compiler workloads are too short for useful wall-time conclusions. vscode's paired wall-time changes range
from -5.61% to +61.72% single-threaded and -26.24% to +4.88% at the default. The apparent default improvement is
not reliable evidence of a speedup. This run is also not the two headline benchmark publishes required for the
wall-time gate. Linux's deterministic instruction-count gate was not run; no Linux runner was available locally.
The measured changes do not justify adding a dependency and an unsafe conversion.

## Validation of the candidate

`cargo test --offline -p tsrs_vfs` passed all 91 tests, including four experimental tests for allocation reuse,
valid Unicode and BOMs, malformed UTF-8 (including vector boundaries), and UTF-16 BOM/surrogate handling.
The warnings-denied workspace check, `wasm32-wasip1` check, lint ratchet and source lint also passed. The full
conformance suite was not rerun; the candidate was discarded after measurement. No production code or published
capability counts change in this findings-only update.
