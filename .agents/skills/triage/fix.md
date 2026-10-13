# Fix

Implement a fix only for a reproduced parity bug with a specific, medium- or
high-confidence diagnosis. Otherwise append why no safe fix was attempted and
return `fixed: false` with `commitMessage: null`.

Read `<triageDir>/report.md`, `CONTRIBUTING.md`, `docs/PORTING.md`,
`docs/RUST.md`, and any nearer `AGENTS.md` before editing.

## Implement the Go behavior

Port the relevant Go function mechanically: keep its decomposition, operation
order, names, and control flow recognizable. Do not redesign the algorithm,
add an unrelated feature, or clean up neighboring code. Preserve existing user
changes. Do not use bare `#[allow]`; an intentional Clippy exception is a
smallest-scope `#[expect(clippy::<lint>, reason = "...")]`.

Add a regression that fails before the fix and passes after it. For a CLI
parity defect outside the conformance suite, add a minimal
`testdata/regressions/<descriptive-name>/` project and generate
`expected.txt` from the pinned Go compiler, normalizing its project path to
`<dir>`. Use the subsystem's established focused test format for LSP, API,
Wasm, or other non-CLI defects.

Update `docs/COMPATIBILITY.md` and the README summaries only if the fix changes
a user-visible capability status or closes a listed gap.

## Verify

Run the new regression against the fixed tsrs binary and the pinned Go
compiler. Run the affected crate's checks/tests and `tools/regressions.sh`.
Run `tools/lint/ratchet.py` when it exists, then `tools/lint/source.py`; on a
revision before the ratchet script exists, run the full Clippy command in
`docs/RUST.md` instead. Run any generated-code freshness check relevant to the
edited files.

Do not claim success when a required check failed. Remove reproduction files,
build artifacts, debug output, and instrumentation from tracked changes. Review
`git diff` and `git status`, but do not commit or push.

Append the changed behavior, Go function followed, test evidence, commands and
results, and final diff summary to `report.md`. Return `fixed: true` only when
the reproduction now matches Go and the relevant tests pass. Use a commit
message of the form `<area>: <what and why>`; otherwise return null.
