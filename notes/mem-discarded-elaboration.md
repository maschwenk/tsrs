# Discarded relater elaboration: how much memory and work it costs (round 4)

**Result: almost nothing on the loss projects.** On the six projects where bun check uses less memory, error
elaboration that is thrown away costs at most 1.34 MiB (mikro-orm at 8 checkers), which is 0.08% of peak. It also
costs under 2 ms of CPU. Counting every node-builder print, kept and discarded, the most any loss project spends is
16.5 MiB of arena (mikro-orm, whose 43,667 errors keep most of it). A lazy-formatting fix cannot reach the AGENTS.md
gain bar (5% of peak at the default checker count, or 1% of single-threaded instructions) on any of the eight projects
measured. The largest saving is on webpack, a control project: 4.8 MiB, or 1.1% of peak at 8 checkers.

The excalidraw pattern (notes/perf-excalidraw-typefest.md) does not recur on these corpora. There, one
type-argument constraint check elaborated hundreds of thousands of failing members. Here the whole check prints between
31 (t3code-server) and 31,538 (mikro-orm) top-level types or symbols.

Branch `exp/discarded-elaboration` (f744cd7a, on 872027a4 = `origin/main`), measurement only, not for merge.

## Method

The instrumentation is `crates/tsrs_checker/src/elab.rs` plus small hooks. It is env-gated:
`TSRS_ELAB=record:<dir>` or `TSRS_ELAB=replay:<dir>`.

- **Prints.** It wraps every node-builder entry point the checker uses for strings:
  - `type_to_string_ex`
  - `symbol_to_string_ex`
  - `signature_to_string_ex`
  - `type_predicate_to_string_ex`
  - `type_parameter_to_string_ex`

  Each is `#[track_caller]` through its thin wrappers, so the print site is the real caller. A top-level print is
  recorded with:
  - the bytes it bumped in the thread arena (a new O(1) `Arena::used_fast`)
  - heap bytes allocated: a counting wrapper over mimalloc, feature `elab-heap` of tsrs_cli, in the record build only
  - its wall time
  - the hash of its output

  A counter on `NodeBuilder::enter_context` confirms coverage: every node-builder context opened during checking was
  inside a hooked print (0 outside, on all eight projects).
- **Windows.** A window is a reporting call of `check_type_related_to_ex` or `check_type_related_to_and_optionally_elaborate`
  (one with an error node). Each window records its caller and the diagnostics reported while it is the innermost open
  window.
- **Discard events:**
  - `restore_error_state` (with its call site) drops the window's prints made since the saved state.
  - Restores after which the relater can put the old chain back (`original_error_chain`, relater_2.rs:1052, 1249,
    1459) only flag the prints and never discard them.
  - A diagnostic reported while a window is open pins that window's pending prints to it. This covers side diagnostics
    such as `c.error` during lazy resolution, which a restore does not drop.
- **Kept.** A diagnostic is kept if it is reachable (through its message chain and related information) from a
  diagnostic that `get_diagnostics` or `get_global_diagnostics` returned. A diagnostic that the collection dropped as
  a duplicate of a kept one also counts as kept, because `equal_diagnostics` compares the formatted text.
- **Classification.** A print is discarded if:
  - it was restored, or
  - its window reported no diagnostic, or
  - neither its pin nor any diagnostic of its window is kept.

  Prints outside every window are never counted as discarded. They are matched against the kept diagnostics' arguments
  for the report only.
- **Upper bound (replay).** A checker's prints are numbered in order, keyed by the first file the checker checks.
  Under a deterministic assignment (1 checker, or `--checkerAssignment locality` at 8), the replay build replaces every
  print the record marked discarded with a 17-byte stand-in and does not call the node builder. Two stand-ins are equal
  exactly when the recorded strings were equal. The control is the same binary replaying a table with no stubs, so
  the hook overhead cancels.
  - Alignment check: 0 kind mismatches and 0 output-hash mismatches on real prints in every run.

Mac (M5 Max, shared, 1-minute load 8-13). Binaries:

- main = 872027a4
- exp = f744cd7a, built twice: the plain build for the A/B runs, and the `--features elab-heap` build for the records

Flags: `--noEmit --incremental false --pretty false --checkers N`.

## Numbers

The 8-checker "main peak" column is the default mode (work stealing; median of 3). The ledger columns are from the
default mode at 8 checkers. The A/B runs use `--checkerAssignment locality` at 8 checkers, which replay needs, and are
medians of 3 interleaved reps (vscode: 9). The peak noise between reps is about ±10 MiB at 8 checkers. Single-threaded
instructions vary by up to 2% between identical runs on this machine, so instruction deltas under 1% are not resolved.
The discarded print time is the better CPU measure.

| project | checkers | main peak (MiB) | prints (all / discarded) | all printing arena (MiB) | discarded arena = upper bound (MiB, % of peak) | A/B peak, stub vs control | discarded print time (ms) | A/B instructions, stub vs control |
|---|---:|---:|---|---:|---|---|---:|---:|
| t3code-server | 1 | 773 | 31 / 11 | 0.7 | 0.01 (0.00%) | -1 MiB | 0.1 | -0.3% |
| t3code-server | 8 | 1,801 | 59 / 39 | 0.8 | 0.03 (0.00%) | +0 MiB | 0.2 | +0.0% |
| supabase-studio | 1 | 915 | 616 / 25 | 5.9 | 0.12 (0.01%) | -3 MiB | 0.3 | -0.1% |
| supabase-studio | 8 | 1,384 | 902 / 62 | 9.1 | 0.21 (0.02%) | -2 MiB | 0.7 | +0.0% |
| mikro-orm | 1 | 1,096 | 31,524 / 1,845 | 16.0 | 1.14 (0.10%) | -1 MiB | 1.3 | -0.0% |
| mikro-orm | 8 | 1,608 | 31,538 / 1,857 | 16.5 | 1.34 (0.08%) | -2 MiB | 1.6 | -0.0% |
| cal-diy | 1 | 793 | 422 / 37 | 1.9 | 0.02 (0.00%) | +0 MiB | 0.1 | -0.1% |
| cal-diy | 8 | 1,467 | 574 / 91 | 2.3 | 0.10 (0.01%) | -0 MiB | 0.9 | +0.0% |
| formbricks-web | 1 | 1,125 | 749 / 34 | 2.0 | 0.02 (0.00%) | +1 MiB | 0.1 | -0.0% |
| formbricks-web | 8 | 1,699 | 2,061 / 52 | 4.0 | 0.03 (0.00%) | +4 MiB | 0.1 | +0.0% |
| vscode | 1 | 1,648 | 2,336 / 421 | 2.3 | 0.24 (0.01%) | +0 MiB | 0.5 | -0.0% |
| vscode | 8 | 1,955 | 2,835 / 449 | 2.9 | 0.26 (0.01%) | +2 MiB | 0.5 | -0.0% |
| drizzle-orm (control) | 1 | 410 | 11,152 / 274 | 7.3 | 0.18 (0.04%) | +0 MiB | 0.3 | -0.0% |
| drizzle-orm (control) | 8 | 573 | 32,155 / 274 | 17.7 | 0.18 (0.03%) | -1 MiB | 0.4 | -0.1% |
| webpack (control) | 1 | 323 | 3,758 / 2,230 | 2.6 | 1.49 (0.46%) | -1 MiB | 1.8 | -0.3% |
| webpack (control) | 8 | 439 | 7,432 / 5,854 | 6.1 | 4.76 (1.08%) | -8 MiB (-1.9%; 412-414 vs 420-422 in every rep) | 8.0 | -0.7% |

Heap: the discarded prints allocate at most 1.7 MiB of heap in total on a loss project (webpack: 9.5 MiB). It is
transient, freed before the peak. The heap columns are in `rec/<project>-<n>/summary.tsv`.

**Diagnostics are byte-identical** between main, the stubbed replay and the control: 16 cells × 3 reps, plus 6
more vscode reps.

## Where the discarded work comes from

These are shares of the discarded arena. "entry" is the reporting call; "restore" is where the relater dropped the
chain.

**All six loss projects at 8 checkers** (1.97 MiB in total, 2,550 prints):

| share | arena | prints | site |
|---:|---:|---:|---|
| 46% | 0.91 MiB | 996 | `structured_type_related_to_body`'s success restore (relater_2.rs:987). The structured worker fails while elaborating, then the intersection / union-constraint retry or the extra property check succeeds. Entry: `check_type_argument_constraints` (checker_02.rs:1085). |
| 26% | 0.51 MiB | 971 | `signatures_related_to` over several target signatures (relater_2.rs:2371). The first failing source signature is elaborated, then another one matches. Entry: `check_interface_declaration`'s check against the base interface (checker_03.rs:945), almost all in mikro-orm. |
| 9% | 0.18 MiB | 131 | The conditional default-constraint-then-distributive restore (relater_2.rs:1581), the excalidraw pattern. Entry: `check_type_argument_constraints`. |
| 6% | 0.12 MiB | 54 | `elaborate_element` (relater_1.rs:854). It reports into a vector the caller drops. |
| 4% | 0.08 MiB | 144 | The same conditional restore. Entry: `check_type_arguments` (checker_05.rs:798). |
| 2% | 0.05 MiB | 91 | The `signatures_related_to` restore. Entry: `check_assertion_deferred` (checker_06.rs:2017). |

By project:

- mikro-orm: 51% from relater_2.rs:987 and 38% from the interface/base signatures restore
- vscode: 70% from relater_2.rs:987
- supabase-studio: 49% from the conditional restore and 26% from `elaborate_element`
- cal-diy: 59% from `elaborate_element` and 21% from diagnostics another checker filed against a file it does not
  check

**Controls:**

- webpack, 8 checkers (4.76 MiB): 52% is diagnostics produced by `check_type_related_to_and_optionally_elaborate`
  (relater_1.rs:592) in a checker that does not own the file. This is the "diagnostics located in another file" class
  (checkJs JS files reach each other's checks). The rest is the relater_2.rs:987 and :1581 restores. At 1 checker
  that class is gone and 1.49 MiB remains.
- drizzle-orm: 11.4 MiB of printing at 8 checkers is not relater work. It happens before the first file is checked:
  `report_merge_symbol_error` during global symbol merging prints 2,976 symbol names (duplicate identifiers) in every
  checker. By construction only the owning checker keeps those diagnostics, so about 10 MiB (1.7% of peak) is
  discarded at 8 checkers and none at 1. This was inferred from the per-checker count, not stubbed. It is below the
  bar, and drizzle is no longer on the loss list.

## Expected gain of a real lazy-formatting fix

The bound is: the discarded arena at most, and in practice less.

- **Loss projects:** 0.0-0.1% of peak and under 2 ms of CPU. That is 0 to 1.3 MiB at both 1 and 8 checkers.
- **webpack:** about 1-2% of peak at 8 checkers (measured -8 MiB with the stub).
- **drizzle:** about 0.04%. Its init-time duplicate prints need a separate fix: report global-merge errors only in
  the file's own checker.

None clears 5% of peak or 1% of instructions, so the expected gain does not justify the fix.

A real fix would get less than the stub, and it is not exact for free. The replay was byte-identical only after these
classes of print were left unstubbed, because each one feeds output:

1. **Prints whose text feeds a side diagnostic.** `c.error` during lazy resolution inside a relation (webpack TS2315
   "Type 'export=' is not generic") is not dropped by `restore_error_state`.
2. **Duplicates of kept diagnostics.** `DiagnosticsCollection::add` dedupes with `equal_diagnostics`, which compares
   the formatted text. Leaving those unprinted produced 13 extra webpack errors.
3. **Chains that are dropped and then put back.** The `original_error_chain` restores (relater_2.rs:1052, 1249,
   1459) can resurrect a dropped chain.
4. **Chain prints in a window that also reported a side diagnostic.** These were pinned to the side diagnostic and so
   looked discarded; t3code-server's TS2322 chain shows the case.

`get_type_names_for_error_display` and `report_relation_error` compare printed strings, so those prints would still be
forced. Stubbing also changed type ids: up to 1,960 recorded-vs-replayed id mismatches on mikro-orm, with output
unchanged. The node builder creates types, so deferring it is a history change in Go mode, as
notes/perf-excalidraw-typefest.md warned.

## Not done

- **No Linux probe.** The largest loss-project bound is 1.34 MiB (mikro-orm, 8 checkers). That is far below the
  ~1% RSS noise of the 16-vCPU runner, and the arena accounting is the same code on both platforms, so a perf-probe
  run could not resolve it.
- **No relation-work measurement.** The cost of the reporting relation itself (as opposed to printing) was not
  measured; the task scoped this to node-builder output. notes/perf-excalidraw-typefest.md found printing to be about
  95% of the site's cost.

## Reproduce

```sh
git -C ~/Developer/tsrs-work/tsrs worktree add wt/discelab origin/exp/discarded-elaboration
CARGO_BUILD_JOBS=8 cargo build --release --locked -p tsrs_cli --features elab-heap   # record build
cp target/release/tsrs tsrs-rec
CARGO_BUILD_JOBS=8 cargo build --release --locked -p tsrs_cli                        # replay build
F=(--noEmit --incremental false --pretty false --checkers 8 --checkerAssignment locality)
TSRS_ELAB=record:/tmp/r ./tsrs-rec -p src $F             # writes /tmp/r/summary.tsv, checkers.tsv, <key>.bin
TSRS_ELAB=replay:/tmp/r target/release/tsrs -p src $F    # stderr: "tsrs elab replay: stubbed N real M ..."
# TSRS_ELAB_DEBUG=<printed string>: where that string was printed and how it was classified
```

The scripts are in this directory: `record.sh`, `replay.sh` (main / stub / no-stub control, interleaved), `nostub.py`,
`agg.py`, `sites.py`, `table.py`, `ab.py`. The raw outputs are in `rec/*/summary.tsv`, `ab/*.time` and `ab-log.txt`.
