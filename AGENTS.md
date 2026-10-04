# Notes for coding agents

`CONTRIBUTING.md` and `docs/PORTING.md` have the porting rules and the test workflow. This file adds two rules.

## Keep the lint ratchet green

Run `tools/lint/ratchet.py` before a change lands. It runs a chosen set of clippy lints (clones, allocations,
argument passing, hash iteration order, `unsafe`) and fails when a file has more findings of a lint than
`tools/lint/baseline.tsv` records. `docs/RUST.md` explains the lints, the rules no tool checks, and which performance
techniques are in place, rejected or untried.

- Fix what it reports in the code you wrote. Do not fix baseline findings in passing; paying them down is its own
  change (`docs/RUST.md`).
- If the flagged code is intended, use `#[expect(clippy::<lint>, reason = "...")]` on the smallest item. Never a bare
  `#[allow]`, and never raise `baseline.tsv`.
- A performance change needs numbers (instructions retired and peak RSS, before and after) and a note in `notes/`.
  Check `docs/RUST.md` "Techniques" first: it lists what was already measured and rejected.

## Keep the README capability table current

`README.md` has a table, "What it does and doesn't do", listing every user-visible capability with its status and
evidence. When you finish a large chunk of work, update the table in the same pull request. A large chunk is:

- a ported subsystem or emit wave, or a merged feature PR;
- a capability that starts or stops being supported, becomes the default, or ships in an npm release;
- a change in a pass count the table cites (conformance, `.types` / `.symbols`, `.js` / `.js.map` /
  `.sourcemap.txt`, fourslash, tsctests, the emit oracle);
- a known gap that is closed, or a new one you find.

Small fixes that change none of these need no update.

How to update it:

- Measure numbers at the commit you describe: `tsrs-test run --suite all --baselines types,symbols,js,jsmap,sourcemap`,
  `tsrs-fourslash run`, the tsctests harness and `tools/oracle/emit/` (`docs/EMIT.md`, `docs/LSP.md`). If you copy a
  number from `notes/` or a PR instead, say so in the PR description.
- Compare against tsgo built from the pinned commit, not the npm nightly.
- Keep the status column to a short state: `yes`, `no`, `ignored`, or the condition it needs (such as
  `--incremental`). Put numbers and caveats in the evidence column.
- Keep the README intro, `docs/EMIT.md`, `docs/LSP.md` and `docs/STATUS.md` consistent with the table.
- Do not name the private monorepo; call it "the 38k-file codebase".
- Do not edit the benchmark section between `<!-- bench:start -->` and `<!-- bench:end -->`. The bench workflow
  (`.depot/workflows/bench.yml`) rewrites it.
