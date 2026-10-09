# Release profile binary size

`cargo build --release` now uses the same fat LTO and single codegen unit as the `dist` profile, disables debug
information and strips symbols. On macOS arm64 at 3183bbd3, a clean `tsrs_cli` release build changed from
33,226,472 bytes to 20,233,280 bytes (-39.1%). The rebuilt binary passed its `--version` smoke test.

The LTO and codegen-unit settings were already measured for shipped binaries in `notes/perf-pgo.md`; symbol and
debug-information stripping changes file metadata rather than generated code, so it does not change retired
instructions or peak RSS.

The Linux `dist` build must remain unstripped until after BOLT. The release and benchmark workflows override
`CARGO_PROFILE_DIST_STRIP=none` for the PGO-use build that BOLT consumes, and `bolt.sh` strips the optimized `tsrs`
only after BOLT training, optimization and correctness comparisons finish. The PGO instrumented build must set the
same value: cargo hashes the profile, `strip` included, into every crate's `-C metadata` and so into every symbol
name, and the profile-use build looks a function's counts up by that name. Until 2026-10-09 the instrumented build
inherited `strip = "symbols"`, and the Linux release and bench binaries were built with no profile data
(notes/perf-pgo.md).
