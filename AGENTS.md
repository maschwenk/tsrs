# Notes for coding agents

`CONTRIBUTING.md` and `docs/PORTING.md` have the porting rules and the test workflow; `docs/RUST.md` has the Rust
practices. This file adds the rules below.

## General

- When you run tsrs on private code (to test performance, hill-climb or reproduce a bug), say only that it was run on
  a large private monorepo. Never divulge anything else about that repo: not its name, paths, package or file names,
  size, types or code. This holds in code, comments, notes, commit messages and pull requests.
- Do not edit the benchmark section of `README.md` between `<!-- bench:start -->` and `<!-- bench:end -->`. The bench
  workflow (`.depot/workflows/bench.yml`) rewrites it; change its generator, `bench/run.py`, instead.

## Keep the lint ratchet green

Run `tools/lint/ratchet.py` before a change lands. It runs a chosen set of clippy lints (clones, allocations,
argument passing, hash iteration order, `unsafe`) and fails when a file has more findings of a lint than
`tools/lint/baseline.tsv` records. `docs/RUST.md` explains the lints and the rules no tool checks.

- Fix what it reports in the code you wrote. Do not fix baseline findings in passing; paying them down is its own
  change (`docs/RUST.md`).
- If the flagged code is intended, use `#[expect(clippy::<lint>, reason = "...")]` on the smallest item. Never a bare
  `#[allow]`, and never raise `baseline.tsv`.
- Run `tools/lint/source.py` too: a new `unsafe impl Send`/`Sync` needs a SAFETY comment and an `--update` that puts it
  in the reviewed inventory, and a `Relaxed`/`Acquire`/`Release` ordering needs a comment saying why it is enough.

## Performance and memory changes

**Find what was already tried first.** Most ideas have been. Search the notes (`rg -il '<a few keywords>' notes/
docs/RUST.md`), read the "Measured and rejected (do not redo)" list in `notes/perf-round2-followups.md` and the
"Techniques" section of `docs/RUST.md`, and read the newest round's summary (the highest-numbered
`notes/mem-round*.md` for memory against bun check). If a note rejected the idea, start only if something is
different, and say what in the brief or the pull request: new evidence, a changed constraint, or the condition the
note gave for revisiting it. For example, a type graph shared between checkers was rejected four times as a default
(notes/mem-shared-base.md, notes/perf-checker-processes.md, notes/perf-shared-checker.md,
notes/spike-shared-graph.md), and landed (#281) only when something changed: under `--maxMemory` the pool no longer
waits for the seed, which met the condition the spike gave for revisiting it. Whatever the outcome, add it to that
list.

**Bring numbers.** A performance change needs instructions retired and peak RSS, before and after, and a note in
`notes/`.

**Clear the bar.** A change lands only if it gains at least one of:

- 1% of single-threaded instructions on at least one `bench/projects.json` project (`bench/count.py`, deterministic);
- 2% of wall time in the README benchmark's wall-time table, read across two publishes on the same hardware;
- 5% of peak memory at the default checker count.

A change that adds a thread, a cache, an overlap between phases or an invariant later code must keep needs a clear
margin over that bar: a sub-percent gain never justifies it, whichever measure it is on. A finding below the bar is
still worth keeping: land it as a note in `notes/` (the negative results there are read before every new round) and
leave the code alone. Why: a review on 2026-10-07 found that five changes carried most of the gain since 0.5.0, while
a tail of 2-4 ms changes added helper threads and overlaps for about 5% of wall in total and broke the PGO and BOLT
training profiles once (notes/perf-serial-steps.md, notes/perf-front-end-fixed-costs.md).

## Keep the README capability table current

`README.md` has a table, "What it does and doesn't do", listing every user-visible capability with its status and
evidence. When you finish a large chunk of work, update the table in the same pull request. A large chunk is:

- a ported subsystem or emit wave, or a feature PR;
- a capability that starts or stops being supported, becomes the default, or ships in an npm release;
- a change in a pass count the table cites (conformance, `.types` / `.symbols`, `.js` / `.js.map` /
  `.sourcemap.txt`, fourslash, the emit oracle);
- a known gap that is closed, or a new one you find.

Small fixes that change none of these need no update.

How to update it:

- Measure numbers at the commit you describe: `tsrs-test run --suite all --baselines types,symbols,js,jsmap,sourcemap`,
  `tsrs-fourslash run` and `tools/oracle/emit/` (`docs/EMIT.md`, `docs/LSP.md`). If you copy a number from `notes/`
  or a PR instead, say so in the PR description.
- Compare against tsgo built from the pinned commit (`Cargo.toml`, `[workspace.metadata.typescript]`), not the npm
  nightly.
- Keep the status column to a short state: `yes`, `no`, `ignored`, or the condition it needs (such as
  `--incremental`). Put numbers and caveats in the evidence column.
- Keep the README intro, `docs/EMIT.md`, `docs/LSP.md` and `docs/STATUS.md` consistent with the table.
