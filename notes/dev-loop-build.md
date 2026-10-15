# dev-loop-build: the edit-build-test loop without LTO

Question: an agent's loop is edit, `cargo build --release -p tsrs_cli -p tsrs_testrunner`, `tsrs-test run`. How much
of it is the build, and what does the suite lose without the release profile's fat LTO?

Answer: the build was 92% of the loop. Fat LTO with one codegen unit re-optimizes and relinks everything on every
edit, so a one-line checker edit cost about as much as a clean build (176 s against 181 s). The `dev` profile
(`release` settings with `lto = "off"`, 256 codegen units, incremental) rebuilds the same edit in 4.9 s, and the
conformance suite gives byte-identical pass/fail lists in the same wall time. `lto = "off"` in `[profile.dev]`
(`cargo test`) cuts
its edit rebuild from 13.9 s to 5.2 s. Performance numbers still come from `release`.

## Host and method

Boxy dev container, Intel Xeon Platinum 8375C @ 2.90 GHz, 8 vCPUs, rustc 1.99.0 (rust-lld, the default linker on
x86_64 Linux). "Edit" is a real body change in `crates/tsrs_checker/src/relater_2.rs`
(`type_related_to_some_type_worker`, a `std::hint::black_box` call inserted and then bumped), not a `touch`, which
incremental compilation can skip. Each figure is one run, edits two runs; wall time on a shared host is +-5%.

## Profiles (tsrs + tsrs-test)

| profile | clean build | after a checker edit | suite wall (CPU) |
| --- | --- | --- | --- |
| `release` (fat LTO, 1 CGU) | 181 s | 176 s | 15.4 s (47 s) |
| `release`, `lto = false`, 16 CGU (= `ci` before disabling ThinLTO) | 59 s | 45 s | 14.6 s (47 s) |
| `release`, `lto = false`, 256 CGU, incremental | 61 s | 18-27 s | 16.0 s (50 s) |
| same at opt-level 2 | 59 s | 18 s | 14.7 s (49 s) |
| same at opt-level 1 | 49 s | 14 s | 14.5 s (52 s) |
| **`dev`: `lto = "off"`, 256 CGU, incremental** | **46 s** | **4.9 s** | **15.6 s (54 s)** |
| `lto = "off"`, opt-level 1 | 42 s | 4.8 s | 16.5 s (57 s) |

`lto = false` is not "no LTO": with more than one codegen unit and opt-level > 0, rustc still runs ThinLTO across
the crate's own codegen units. `-Ztime-passes` on the tsrs_checker rebuild (opt-level 1, `lto = false`) put 11.5 s
of a 27.9 s compile in `LLVM_thinlto`; `"off"` removes that pass, and with it most of the rebuild.

Other edits with `dev`: a new public function in tsrs_core rebuilds in 0.9 s (`release`: 184 s); a body change in a
generic tsrs_core function, which every dependent crate instantiates again, in 19.9 s.

The suite's wall time does not depend on the profile: one test, `compiler/intersectionConstructorReductionCrash`,
takes 13.2 s of the 15 s, and all 15,197 variants together take 51 s of CPU. Making that test faster is a checker
performance change, under the bar in AGENTS.md.

## Same results

- `conformance-gate.sh` (errors, `.types`, `.symbols`): `pass`, `fail`, `types-pass`, `types-fail`, `symbols-pass`,
  `symbols-fail` lists identical between `dev` and `release` (13,458 passes); 15.8 s against 15.8 s.
- The default-mode gate (`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`) and `tools/regressions.sh`
  pass on `dev`.
- fourslash: 4,066 passes and 63 failures on both, identical pass lists. Building tsrs-fourslash after a checker edit
  takes 17.5 s with `dev` and 273 s with `release`; the run takes 12.9 s against 9.8 s.

### Generated fourslash tests

The 4,546 generated test bodies used to share a crate with the hand-written harness. On an arm64 macOS development
host, a `dev` timing build spent 61.2 s compiling that crate: 5.4 s in the frontend and 55.8 s in code generation.
Moving the harness into an optimized `tsrs_fourslash_harness` crate and compiling only the generated glue at
opt-level 0 cut `tsrs_fourslash` to 6.2 s (5.0 s frontend, 1.3 s code generation), about 90%. The optimized harness
crate took 3.1 s, and the whole candidate build took 29.54 s. The release profile is unchanged.

The full suite took 4.3 s: 4,066 passed, 63 known failures, and 417 skipped. Before and after result files were
byte-identical (SHA-256):

- `pass.txt`: `272f329e11eaab849d5c312cd0f1e5922c09f5f95fb566e249e6d82e2acc8cf7`
- `fail.txt`: `68a77b3bf208d66221f3aa41cf78cc9e26a302e7d54ce7e1bbf1afee3ce024f8`
- `skip.txt`: `ccb960a7fa1f53597fcb5de33d680fcfdaad7b9829ae12adbebb247425606b0d`

One upstream fixture constructs a 583,770-byte file containing five long runs of blank lines. Emitting those runs
literally made one generated Rust shard 589,068 lines. The generator now emits a runtime builder for local constants
with newline runs of at least 1,024 bytes: the shard is 5,363 lines, and an isolated `tsrs_fourslash` compile fell
from 5.79 s (4.83 s frontend) to 4.70 s (3.78 s frontend), with the suite hashes above unchanged.
Extracting all four strings of at least 64 KiB into generated data files cut Rust source from 13.5 MiB to 7.0 MiB
but did not improve on that (4.69 s): `include_str!` still makes rustc load and embed the data. Not adopted.

The canonical generated wrapper (outer `RecoverAndFail`, constant content, `NewFourslash` with default capabilities,
inner `done`) now lives in one non-generic harness helper. The generator used it for 4,325 of 4,546 tests, cutting
the generated shards from 163,056 to 132,781 lines and their `go::run` closures from 9,103 to 453. A paired isolated
crate rebuild fell from 6.14 s to 4.03 s (-34%): frontend 4.82 s to 3.31 s, code generation 1.32 s to 0.72 s. The
suite took 5.9 s before and 5.5 s after; all three result hashes above remained identical.

The same helper now also runs the canonical `t.Parallel()` prologue, and the generator passes the one-use content
literal directly instead of declaring a local constant. That removes another 8,649 generated lines (132,781 to
124,132). A warm paired isolated crate rebuild fell from 3.81 s to 3.65 s (-4.2%): frontend 3.11 s to 2.96 s and
code generation 0.70 s to 0.69 s. The suite took 4.8 s; all three result hashes above remained identical.

For correctness CI, `lto = false` still ran ThinLTO within every optimized crate. On the same 12-core arm64 host and
commit, a clean `ci` build of `tsrs`, `tsrs-test` and `tsrs-fourslash` took 43.81 s with that setting and
34.57 s with `lto = "off"`, a 21.1% reduction. Keeping 16 codegen units was faster than raising it to 256 (37.32 s)
and had less test-runtime cost, so `ci` changes only the LTO setting. Warm conformance was 9.52 s against
9.70 s and fourslash 3.86-3.90 s against 4.01 s; all result lists were identical. With the generated-source change
above as well, the final clean build took 33.56 s. Release remains unchanged.

## `cargo test` (the CI "fast crates" list)

| | before | after |
| --- | --- | --- |
| build after a checker edit (`--no-run`) | 13.9 s | 5.2 s (`lto = "off"` in `[profile.dev]`) |
| run | 83.7 s | 57.0 s |

The run was 43.6 s of `tsrs_cli/tests/recycle_checkers.rs`, one test that ran tsrs four times per regression case,
one case after another; its cases now run on all cores (14.7 s). The rest is `cargo test` running test binaries one
at a time: cargo-nextest 0.9.148 runs the same tests in 21 s, but the custom-harness `tsrs_contentmappertest`
`mapper_test` binary cannot be listed by it (`-E 'not binary_id(tsrs_contentmappertest::mapper_test)'` and a
separate `cargo test --test mapper_test`, 3.2 s), and `tsrs_ipc`'s
`conn_async_test::call_with_timeout_gives_up_with_deadline_exceeded` failed once under its load ("failed to write
response: Broken pipe" from the mapper end in `shut_down`; 0 failures in 20 runs alone). Not adopted here.
