# Fix-wave workflow

All function bodies are ported; from here the conformance suite and the Go reference drive the work.
Several agents fix different failure clusters at the same time. This is how they avoid stepping on each other.

## Your worktree

- You get a private git worktree `$TSRS_WORK/wt/<agent-name>` on branch `fix/<agent-name>`,
  created from `origin/main`. Work only there (never in the main checkout `$TSRS_WORK/tsrs`
  or another agent's worktree). `ts-ref` inside it is a symlink to the Go reference checkout.
- Build with your own target dir: `CARGO_TARGET_DIR=$PWD/target cargo build --release -p tsrs_testrunner -p tsrs_cli`
  (release: the suite runs in ~25 s; a debug build is much slower to run).

## Landing changes (trunk-based, through GitHub)

Land small, often — every coherent fix, not one big drop at the end:

```sh
git add -A . && git commit -m "<area>: <what and why>"
git fetch -q origin main && git rebase origin/main        # resolve conflicts if any, keep both sides' intent
CARGO_TARGET_DIR=$PWD/target cargo check --workspace      # must be 0 errors, 0 warnings after the rebase
# re-run the tests you touched, and before every push a full-suite run to check for regressions (see below)
git push -q origin HEAD:main                              # if rejected: fetch, rebase, check, push again
```

Never force-push, never rewrite `origin/main`, never commit build output. If a rebase conflict is in code you do not
understand, read both versions and the Go source; do not blindly take one side.

## Running tests

- Full suite: `./target/release/tsrs-test run --suite all` -> summary table; per-class name lists in
  `target/test-results/{pass,codes,fail,crash,timeout}.txt`; `summary.json` has the first difference per test.
- One test / a cluster: `tsrs-test run --filter <substring|regex>`, `tsrs-test show <suite/name>` (expected vs actual),
  `tsrs-test crashes --top 20` (panics grouped by location).
- Classes: `pass` = baseline byte-identical; `codes` = same (file, line, col, code) set but message text differs;
  `fail` = different diagnostics; `crash`; `timeout`.
- **No regressions**: before each push, compare `pass.txt` against the list from before your change
  (`comm -23 <(sort before/pass.txt) <(sort target/test-results/pass.txt)` must be empty, or every lost test must be
  explained by an upstream change you just rebased onto). Record the new totals in your commit message.
- Reference behavior for anything outside the suite: `$TSRS_WORK/bin/tsgo-ref` (same commit
  as the port). Oracle tools for scanner/parser/binder/module resolution/printer live in `tools/oracle/` with binaries in
  `$TSRS_WORK/bin/`.

## How to fix

- The Go source (`ts-ref/tsc/internal/…`) is the specification. Find the Go function behind the wrong behavior, read it
  next to the Rust port (`// file.go:LINE` origin markers; `docs/sigs/checker.txt` maps names), and make the Rust do
  what the Go does. Most bugs are small porting slips: a dropped branch, a swapped argument, `intersects` vs
  `contains`, nil-vs-empty, an `unwrap()` where Go tolerates nil, evaluation order, a wrong flag constant.
- Never special-case a test, never paper over a symptom, never "improve" on Go. If Go has a quirk, port the quirk.
- Fix root causes in shared helpers rather than at one call site; then look for the same slip elsewhere
  (grep for the pattern) — sibling bugs are common because the same agent ported neighboring functions.
- When a signature changes, run `python3 tools/sigs-from-rust.py` to refresh `docs/sigs/checker.txt`.
- Keep a short log in `notes/fix-<agent-name>.md` (root causes found, patterns worth grepping for, things you could not
  resolve). It is committed with your work and read by the other agents.
- Do not stop at the first fix. Work through your cluster until it is done or what remains is blocked on something
  outside your scope — then say exactly what.
