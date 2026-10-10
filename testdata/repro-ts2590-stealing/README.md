# Repro: TS2590 comes and goes between identical runs in the default mode (#218)

Not for merge. This is a self-contained reproduction and an explanation of the cause, for whoever decides on the fix.

The default mode is meant to print the same diagnostics for every checker count and assignment (README;
docs/DEBUGGING.md "Checker assignment"). For TS2590 it doesn't: on the same project, two runs in a row can disagree.

## Run it

Needs node and a tsrs binary (`cargo build --release -p tsrs_cli`).

    cd testdata/repro-ts2590-stealing
    ./run.sh ../../target/release/tsrs 620 30

`run.sh <tsrs> <slow> [runs] [checkers]` generates the project into a temp dir with `make.mjs`, runs
`tsrs -p <dir> --checkers 2` `runs` times, and prints one digit per run: how many TS2590s that run printed. On main at
da1d0538 (0.9.0), Linux x64, 8 cores:

    slow=600: 000100000100000100000000100001
    slow=620: 011110100101011011110100011100

The same project with stealing off never reports it:

    node make.mjs /tmp/p 620
    tsrs -p /tmp/p --checkers 2 --checkerAssignment locality     # 0 TS2590s in 20 of 20 runs

`<slow>` sets the balance point and depends on the machine. Here 670 and 720 printed 1 in 30 of 30 runs. If every
run prints the same digit, sweep it:

    for s in 400 500 550 600 620 650 700 750; do printf "%s: " $s; ./run.sh ../../target/release/tsrs $s 20; done

The same split without timing: `TSRS_CHECKER_ASSIGNMENT=random:<seed> tsrs -p /tmp/p --checkers 2` reports it for
seeds 1, 2, 4 and 6 and not for 3 and 5, each seed the same on every run. docs/DEBUGGING.md says the default mode
must print the same text for every seed.

## What the project is

`make.mjs` writes about 1,600 files:

- `map.ts`: `ModelMap` maps three keys to unions of 50 object types each.
  `GetModel` is `<T extends keyof ModelMap>(key: T) => ModelMap[T] | undefined`.
- `x/a.ts` and `x/zz.ts` both contain `export const getX: GetModel = key => lookup(key)`, with
  `lookup(key: keyof ModelMap): ModelMap[keyof ModelMap] | undefined`. Relating the arrow to `GetModel` needs the
  write constraint of `ModelMap[T]`, the intersection of the three unions: 50 x 50 x 50 = 125,000 constituents, over
  the 100,000 limit. `a.ts` has `// @ts-ignore` on that line; `zz.ts` doesn't.
- `x/zz.ts` imports `a.ts`, so the locality placement queues it with `a.ts`. Its name sorts last, so it ends the
  queue. `x/fill*.ts` import `a.ts` too; `w/fill*.ts` don't. With 2 checkers, one checker gets `map.ts`, `a.ts`, the
  `x/` fillers and `zz.ts` (last), and the other gets the `w/` fillers.
- `x/a.ts` also has `type Slow = Build<slow>`, a recursive tuple type. It adds check time without adding much static
  weight, so the checker that owns `a.ts` finishes at about the same time as the other one.

`node make.mjs <dir> 0 --no-filler` writes only `map.ts`, `x/a.ts` and `x/zz.ts` (see "tsgo has the same history
dependence" below).

## Why it happens

1. A checker reports this TS2590 only the first time it evaluates the type. Later evaluations in the same checker
   print nothing. `check_cross_product_union` (crates/tsrs_checker/src/checker_13.rs:1175) reports the error and
   returns false. `get_intersection_type_ex` then returns `error_type` (checker_13.rs:643) before it would store a
   result in `intersection_types` (checker_13.rs:606). So the intersection itself is not cached, and some cache above
   it stops the second evaluation. I haven't identified which cache that is; it's the open question for the fix.
2. `a.ts` evaluates the type first and hides the error with `@ts-ignore`. Afterwards, a checker that checked `a.ts`
   reports nothing at `zz.ts`, while a checker that didn't check `a.ts` reports it there.
3. In the default mode, an idle checker steals unstarted files from the back of the queue with the most work left
   (crates/tsrs_compiler/src/checkerpool.rs:838 `stealing_enabled`, :1032 `queues_next`). `zz.ts` is the last file of
   `a.ts`'s queue. If the other checker runs out first, it steals `zz.ts` and reports TS2590. If not, the owner checks
   `zz.ts` itself after `a.ts` and reports nothing. Which happens first depends on timing.

Evidence:

- Stealing off (`--checkerAssignment locality`): 0 of 20 runs report it, against roughly half with stealing.
- With `trace.patch` applied (`TSRS_TRACE_UNION_REDUCTION=1`), 12 runs at slow=670 split like this:

      5  a.ts on checker 1, zz.ts on checker 1 -> no TS2590
      2  a.ts on checker 1, zz.ts on checker 2 -> TS2590
      4  a.ts on checker 2, zz.ts on checker 1 -> TS2590
      1  a.ts on checker 2, zz.ts on checker 2 -> no TS2590

  Every run that reported it had the two files on different checkers, and every run that didn't had them on the
  same one.
- The seeded assignments above give the same split deterministically.

## tsgo has the same history dependence

On the 3-file project (`--no-filler`), tsgo 7.1.0-dev.20260929.1 and tsrs main give:

| | 1 checker | 2 checkers | 3 checkers |
|---|---|---|---|
| tsgo | none | none | `x/zz.ts(5,14): error TS2590` |
| tsrs (default) | none | `x/zz.ts(5,14): error TS2590` | `x/zz.ts(5,14): error TS2590` |

Without the `@ts-ignore`, tsgo at 1 checker reports only `x/a.ts(3,14)`, not `zz.ts`. So the checker logic follows
tsgo, and tsgo's output also depends on which checker evaluates the type first. tsgo's static assignment makes that
repeatable from run to run, while tsrs's stealing makes it vary. Any fix that makes the result independent of the
assignment will differ from tsgo for some checker counts.

## Where this was found

A private monorepo (#218), whose type had about 250 member types per key. In 4 runs of the default mode at 8
checkers, the file with the error landed on checkers 3, 3, 4 and 6, and the file whose `@ts-ignore`'d copy of the
error came first landed on checkers 4, 1, 1 and 4. Those 4 local runs all reported it; on CI, some runs of the same
commit didn't. On a second commit, tsrs with `TSRS_CHECKER_ASSIGNMENT=go` reported the error when the two files were
on different checkers. In the default mode a single checker checked both, the `@ts-ignore`'d file first, and reported
nothing.

## Files

- `make.mjs`: project generator.
- `run.sh`: runs a tsrs binary N times on a fresh project and prints the TS2590 count per run.
- `trace.patch`: temporary tracing on top of `TSRS_TRACE_UNION_REDUCTION` (#219). One line per cross-product check
  of size 1,000 or more (checker id, position, file being checked, union fingerprints), and one line per file a
  checker begins. Apply with `git apply testdata/repro-ts2590-stealing/trace.patch`.
