# perf-release-profile: fat LTO and abort-on-panic for development releases

Question: should the ordinary `release` profile use the same fat LTO and one codegen unit as `dist`, strip symbols,
omit debug information and abort on panic? Answer: yes. Fat LTO had already cleared the performance bar on full
benchmark projects (`notes/perf-pgo.md`: about -2% instructions without PGO). The complete requested profile also
retires 3.8% fewer instructions on a checked-in single-threaded stress case, uses 3.8% less peak RSS and makes the
binary 47% smaller. Production panic-recovery paths were removed so every shipped entry point consistently aborts.

Linux PGO binaries keep symbols for the intermediate BOLT rewrite (`CARGO_PROFILE_DIST_STRIP=false`), and
`.github/scripts/bolt.sh` strips the final artifact. Other release and dist builds use the profile setting directly.

This revisits the rejection in `notes/perf-build-level.md`. That experiment could not run because `panic = "abort"`
exposed an arena use-after-free. The bug was fixed in `notes/fix-arena-recycle-uaf.md`, and source checks, Miri and an
abort-profile conformance job now guard the invariant that it violated. The other earlier blocker was behavioral:
the LSP, API and test runners recovered panics. This change deliberately removes those recovery boundaries.

## Measurement

Base `3183bbd3`, new this PR, rustc 1.99.0, macOS 26.6.2 arm64. Both binaries were clean
`cargo build --release --locked -p tsrs_cli` builds in separate worktrees. Five runs each under `/usr/bin/time -l`:

```text
tsrs -p testdata/regressions/union-too-complex-canonical-order \
  --noEmit --incremental false --pretty false --singleThreaded
```

| binary | instructions retired (median) | peak RSS (median) | file size |
| --- | ---: | ---: | ---: |
| old release: 16 CGUs, unwind, line tables | 1,010,560,559 | 25,657,344 B | 33,226,472 B |
| new release: 1 CGU, fat LTO, abort, stripped | 972,372,661 (-3.8%) | 24,674,304 B (-3.8%) | 17,471,840 B (-47.4%) |

Both builds report the same expected TS2590 diagnostic. This small repository fixture validates the complete profile
and the fixed abort code generation; the landing threshold comes from the earlier fat-LTO measurements on vscode,
mui-docs and the 38k-file codebase in `notes/perf-pgo.md`.

`cargo check --workspace --all-targets --locked`, the API, project and transport tests, and both lint ratchets pass.
Production Rust sources contain no `catch_unwind`, `AssertUnwindSafe` or `resume_unwind`; test-only panic assertions
remain because the test profile still unwinds.
