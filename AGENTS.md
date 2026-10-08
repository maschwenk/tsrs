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
- Run `tools/lint/source.py` too: a new `unsafe impl Send`/`Sync` needs a SAFETY comment and an `--update` that puts it
  in the reviewed inventory, and a `Relaxed`/`Acquire`/`Release` ordering needs a comment saying why it is enough.
- A performance change needs numbers (instructions retired and peak RSS, before and after) and a note in `notes/`.
  Check `docs/RUST.md` "Techniques" first: it lists what was already measured and rejected.
- A performance change must pay for its complexity. It lands only if it clears one of: 1% of single-threaded
  instructions on at least one `bench/projects.json` project (`bench/count.py`, deterministic), 2% of wall time on
  the README's headline table (the default-mode run, read across two publishes on the same hardware), or 5% of peak memory at the
  default checker count; and a change that adds a thread, a cache, an overlap between phases or an invariant that
  later code must keep needs more than a sub-percent gain to justify it, whatever the number. A finding below the
  bar is still worth keeping: land it as a note in `notes/` (the negative results there are read before every new
  round) and leave the code alone. Decided 2026-10-07 after a review of the commits since 0.5.0: five changes carried
  most of the gain, and a tail of 2-4 ms serial-step changes added helper threads and overlaps for about 5% of wall
  in total, one of which broke the PGO and BOLT training profiles (notes/perf-serial-steps.md,
  notes/perf-front-end-fixed-costs.md).

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
