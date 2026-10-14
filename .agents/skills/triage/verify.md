# Verify

Verify only; do not edit source. Read `<triageDir>/report.md` and always append
the verdict and its evidence.

## Apply the project contract

The pinned TypeScript Go compiler is definitive:

- `bug`: the checked-out tsrs behavior differs from the pinned Go compiler.
- `intended-behavior`: tsrs matches it, or the requested capability is
  explicitly out of scope in `docs/COMPATIBILITY.md`.
- `unclear`: the available reproduction cannot establish either conclusion.

Diagnostic code, span, message, order, exit status, JavaScript/declaration
output, and build artifacts all count as behavior. A comparison with the
JavaScript compiler, an older TypeScript release, or personal expectations
does not override the pinned Go result.

Confirm that both binaries used the same input, config, working directory, and
relevant flags. Read the corresponding Go source when the output comparison
could be explained by a test setup difference. Use documentation, comments,
and history only as supporting evidence; they cannot make a Go parity defect
intentional.

Append the reporter's claim, verdict, comparison evidence, and confidence to
`report.md`. Return `high` confidence for a direct deterministic comparison,
`medium` for strong but incomplete evidence, and `low` when ambiguity remains.
