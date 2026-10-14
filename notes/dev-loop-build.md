# dev-loop-build: the edit-build-test loop without LTO

Question: an agent's loop is edit, `cargo build --release -p tsrs_cli -p tsrs_testrunner`, `tsrs-test run`. How much
of it is the build, and what does the suite lose without the release profile's fat LTO?

Answer: the build was 92% of the loop. Fat LTO with one codegen unit re-optimizes and relinks everything on every
edit, so a one-line checker edit cost about as much as a clean build (176 s against 181 s). The `fast` profile
(`release` with `lto = "off"`, 256 codegen units, incremental) rebuilds the same edit in 4.9 s, and the conformance
suite gives byte-identical pass/fail lists in the same wall time. `lto = "off"` in `[profile.dev]` (`cargo test`) cuts
its edit rebuild from 13.9 s to 5.2 s. `release` and `dist` are unchanged: performance numbers still come from them.

## Host and method

Boxy dev container, Intel Xeon Platinum 8375C @ 2.90 GHz, 8 vCPUs, rustc 1.99.0 (rust-lld, the default linker on
x86_64 Linux). "Edit" is a real body change in `crates/tsrs_checker/src/relater_2.rs`
(`type_related_to_some_type_worker`, a `std::hint::black_box` call inserted and then bumped), not a `touch`, which
incremental compilation can skip. Each figure is one run, edits two runs; wall time on a shared host is +-5%.

## Profiles (tsrs + tsrs-test)

| profile | clean build | after a checker edit | suite wall (CPU) |
| --- | --- | --- | --- |
| `release` (fat LTO, 1 CGU) | 181 s | 176 s | 15.4 s (47 s) |
| `release`, `lto = false`, 16 CGU (= `ci-native`) | 59 s | 45 s | 14.6 s (47 s) |
| `release`, `lto = false`, 256 CGU, incremental | 61 s | 18-27 s | 16.0 s (50 s) |
| same at opt-level 2 | 59 s | 18 s | 14.7 s (49 s) |
| same at opt-level 1 | 49 s | 14 s | 14.5 s (52 s) |
| **`fast`: `lto = "off"`, 256 CGU, incremental** | **46 s** | **4.9 s** | **15.6 s (54 s)** |
| `lto = "off"`, opt-level 1 | 42 s | 4.8 s | 16.5 s (57 s) |

`lto = false` is not "no LTO": with more than one codegen unit and opt-level > 0, rustc still runs ThinLTO across
the crate's own codegen units. `-Ztime-passes` on the tsrs_checker rebuild (opt-level 1, `lto = false`) put 11.5 s
of a 27.9 s compile in `LLVM_thinlto`; `"off"` removes that pass, and with it most of the rebuild.

Other edits with `fast`: a new public function in tsrs_core rebuilds in 0.9 s (`release`: 184 s); a body change in a
generic tsrs_core function, which every dependent crate instantiates again, in 19.9 s.

The suite's wall time does not depend on the profile: one test, `compiler/intersectionConstructorReductionCrash`,
takes 13.2 s of the 15 s, and all 15,197 variants together take 51 s of CPU. Making that test faster is a checker
performance change, under the bar in AGENTS.md.

## Same results

- `conformance-gate.sh` (errors, `.types`, `.symbols`): `pass`, `fail`, `types-pass`, `types-fail`, `symbols-pass`,
  `symbols-fail` lists identical between `fast` and `release` (13,458 passes); 15.8 s against 15.8 s.
- The default-mode gate (`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`) and `tools/regressions.sh`
  pass on `fast`.
- fourslash: 4,066 passes and 63 failures on both, identical pass lists. Building tsrs-fourslash after a checker edit
  takes 17.5 s with `fast` and 273 s with `release`; the run takes 12.9 s against 9.8 s.

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
