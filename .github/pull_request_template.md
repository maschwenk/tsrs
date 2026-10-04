## What and why

<!-- Which behavior changes, and the Go function(s) it follows (file:line at the pinned commit). -->

## Evidence

- [ ] `cargo check --workspace` has 0 errors and 0 warnings
- [ ] `tools/lint/ratchet.py` passes (no new clippy findings; `docs/RUST.md`)
- [ ] No conformance tests lost (`comm -23 before/pass.txt target/test-results/pass.txt` is empty); new totals:
- [ ] A regression test (conformance test, unit test, or `testdata/regressions/<name>/`) covers the change
