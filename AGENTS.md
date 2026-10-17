# Notes for coding agents

`CONTRIBUTING.md` and `docs/PORTING.md` have the porting rules and the test workflow; `docs/RUST.md` has the Rust
practices. This file adds the rules below.

## Build and test

Build with `cargo build --profile dev -p tsrs_cli -p tsrs_testrunner` and run `target/debug/tsrs-test`, not
`--release`: an edit rebuilds in seconds instead of minutes, with the same test results (notes/dev-loop-build.md).
Use `--release` only for performance numbers.

## Lint

Run `cargo clippy --all-targets --config 'build.warnings="deny"'` and
`tools/lint/source.py` before a change lands (`docs/RUST.md`).

- Fix what they report in the code you wrote.
- If the flagged code is intended, use `#[expect(clippy::<lint>, reason = "...")]` on the smallest item. Never a bare
  `#[allow]`.

## Performance and memory changes

**Find what was already tried first.** Most ideas have been. Search the notes (`rg -il '<a few keywords>' notes/
docs/RUST.md`), read the "Measured and rejected (do not redo)" list in `notes/perf-round2-followups.md` and the
"Techniques" section of `docs/RUST.md`, and read the newest round's summary (the highest-numbered
`notes/mem-round*.md` for memory against bun check). If a note rejected the idea, start only if something is
different, and say what in the brief or the pull request: new evidence, a changed constraint, or the condition the
note gave for revisiting it.

**Bring numbers.** A performance change needs instructions retired and peak RSS, before and after, in the pull
request.

**Clear the bar.** A change lands only if it gains at least one of:

- 1% of single-threaded instructions on at least one `bench/projects.json` project (`bench/count.py`, deterministic);
- 2% of wall time in the wall-time table in [`bench/RESULTS.md`](bench/RESULTS.md), read across two publishes on
  the same hardware;
- 5% of peak memory at the default checker count.

A change that adds a thread, a cache, an overlap between phases or an invariant later code must keep needs a clear
margin over that bar: a sub-percent gain never justifies it, whichever measure it is on (notes/perf-serial-steps.md
has why). A change below the bar does not land.

## Keep the capability table current

[`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md)'s table "What it does and doesn't do" lists every user-visible
capability with its status and evidence.
Update it in the same pull request when a change ports a subsystem or emit wave, adds a feature, makes a capability
start or stop being supported, become the default or ship in an npm release, changes a pass count the table cites,
or closes or finds a known gap. How to update it is in a comment above the table. Keep the root and npm README
compatibility summaries consistent with it.

## Private code

When you run tsrs on private code (to test performance, hill-climb or reproduce a bug), say only that it was run on a
large private monorepo. Never divulge anything else about that repo: not its name, paths, package or file names, size,
types or code. This holds in code, comments, notes, commit messages and pull requests.
