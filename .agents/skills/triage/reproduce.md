# Reproduce

Reproduce only; do not diagnose or edit tracked source files. Always write
`<triageDir>/report.md`, including on an early exit or infrastructure failure.

## Decide whether to continue

Read the issue body and all comments, with later maintainer or reporter
clarifications taking precedence.

Return a skipped result when:

- `not-actionable`: it is a feature request, question, general performance
  idea, behavior explicitly listed as unsupported in
  `docs/COMPATIBILITY.md`, or otherwise not a bug report.
- `missing-details`: there is no runnable input or steps, or no expected
  behavior/output to compare.
- `unsupported-version`: it is demonstrated only on an old tsrs version and
  cannot be shown on the checked-out revision.
- `host-specific` or `unsupported-runtime`: reproduction fundamentally
  requires a host or runtime absent from the Ubuntu runner.
- `maintainer-override`: a commenter whose `authorAssociation` is `MEMBER`,
  `COLLABORATOR`, or `OWNER` explicitly says not to auto-triage the issue.

Do not treat a report as stale merely because its stated version is old: first
try its minimal reproduction against the checkout.

## Build the two compilers

Build tsrs from the repository root:

```sh
cargo build --release --locked -p tsrs_cli
```

Compare it with the TypeScript Go compiler at the exact
`workspace.metadata.typescript.commit` in `Cargo.toml`. Reuse a matching
checkout in `<triageDir>/ts-ref` when present. Otherwise make a sparse checkout
there, then build `./cmd/tsc` from its `tsc` directory with `GOTOOLCHAIN=auto`.
Keep generated binaries and scratch data under `triageDir`.

## Run the reproduction

Create the smallest project that preserves the report. Inspect linked source
and package scripts before executing them; prefer copying the relevant files
over installing or running an entire third-party project.

Run tsrs and the pinned Go compiler with the same files, config, working
directory, and flags. Use `--pretty false` and single-threaded checking when
the interface supports it. Capture stdout, stderr, exit status, panics, and
timeouts without losing their ordering. Confirm the control case works when a
small control is meaningful.

The issue is reproduced only when the checked-out tsrs differs from the pinned
Go compiler. A difference from the JavaScript `tsc` alone is not enough.

## Report and return

Write `report.md` with the issue claim, exact commands, compiler revisions,
minimal input, complete observed outputs, exit statuses, and one of:

- `REPRODUCED`: include the exact Go-versus-Rust difference.
- `NOT REPRODUCED`: include what was tried and what matched.
- `SKIPPED: <reason>`: include one supported `skippedReason` and why.

Return `reproducible: true` only for `REPRODUCED`. Return `skipped: true` only
for a documented early exit; otherwise an unsuccessful attempt is not skipped.
