# Diagnose

Diagnose only; do not implement a fix. Read `<triageDir>/report.md` first and
always append a diagnosis, even when confidence is low.

If reproduction did not establish a difference from the pinned Go compiler,
append `DIAGNOSIS SKIPPED: no Go-versus-Rust difference` and return
`confidence: null`.

## Locate the divergence

Use the Go source at the pinned commit as the specification. Its source is
normally under `<triageDir>/ts-ref/tsc/internal/`; create that exact checkout
if reproduction did not. Map its packages to Rust crates with
`docs/PORTING.md`, then compare the complete Go function with its Rust port.
Do not reason from the old TypeScript JavaScript implementation or from memory.

Narrow the first observable divergence with focused tests, backtraces, or
temporary instrumentation. Keep instrumentation small, never log private or
secret data, and remove every tracked instrumentation change before returning.
Check nearby tests, blame, commits, and `notes/` for an already-known constraint
or rejected approach.

For a performance or memory report, follow `AGENTS.md`: find what was already
tried and do not claim a regression or improvement without the required
instruction, wall-time, and peak-RSS evidence. Do not run against private code.

## Report and return

Append:

- the first mismatching Go and Rust functions, with paths and lines;
- the precise semantic or control-flow difference that explains the output;
- a minimal, Go-shaped fix approach and likely affected crate;
- evidence, remaining uncertainty, and `high`, `medium`, or `low` confidence.

Use `high` only when the execution evidence and Go comparison identify the
specific defect. Use `medium` when the cause is well supported but not fully
isolated. Use `low` for a hypothesis. Return that confidence.
