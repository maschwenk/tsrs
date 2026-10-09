# Upstream patches: lazy member follow-ups to microsoft/TypeScript#64475 / #64526

Go versions of the five checker laziness changes prototyped in tsrs (notes/mem-lazy.md), as a commit stack against
microsoft/TypeScript, tested the way upstream CI tests and measured on a 38k-file private TypeScript monorepo (5.9M lines; "the private monorepo" below). Opened upstream as drafts on 2026-10-02: L6 as microsoft/TypeScript#64600 and L5 as #64601 (rebased onto main
59f5b0233f; vet, gofmt, checker and testrunner tests re-run on the rebased commits). L1, L11 and L10 are not opened.

| # | patch | prototype | what | depends on |
| --- | --- | --- | --- | --- |
| 1 | `patches/0001-Give-tuple-references-lazy-member-tables.patch` | L1 | tuple references get lazy member tables (one condition) | #64475 |
| 2 | `patches/0002-Answer-empty-object-type-checks-...patch` | L11 | `isEmptyObjectType` / `removeSubtypes` answer from lazy tables | #64475 |
| 3 | `patches/0003-Find-unmatched-properties-...patch` | L10 | `getUnmatchedProperties` on a lazy target without instantiating | #64475 |
| 4 | `patches/0004-Don-t-copy-union-and-intersection-...patch` | L6 | union/intersection property caches: no eager copy, allocate on store | nothing (also in `patches/standalone-main/`) |
| 5 | `patches/0005-Instantiate-conditional-types-...patch` | L5 | conditional instantiation without the throwaway composite mapper | nothing (also in `patches/standalone-main/`) |

- `pr-01-...md` .. `pr-05-...md`: title and description per commit. `pr-04` and `pr-05` are final (line 1 = title,
  line 3 on = body, ready for `gh pr create`); `pr-01`..`pr-03` are still drafts with a `**Title:**`/`**Base:**` header.
- `review-notes.md`: likely pushback and the answers we have.
- `results/`: raw per-run measurements (JSON lines from `tools/measure.py`) and `results/testfiles.md`. `equiv*` are
  the Go-vs-Rust counter runs (b85298b6 base), `main*` the per-commit runs on main, `variance*` repeated 4-checker
  runs. Files without a suffix used the first L10 translation for L10/L6/L5, `-v2` an intermediate one, `-v4` the
  final code (counters are identical across the three; allocations differ, see below).
- `tools/`: the measurement scripts used below, and `variance-debug.patch` (throwaway instrumentation for the
  6-symbol variation, review-notes.md).

## To open (L6 and L5, against main)

Current base: main **82f0546163** (2026-10-01). Both are one commit each, independent of each other and of
#64475/#64526. From `$TSRS_WORK/TypeScript-upstream` (`$TSRS` = this repo); `fork` is
https://github.com/maschwenk/TypeScript (exists; neither branch name is taken there):

```sh
cd $TSRS_WORK/TypeScript-upstream
git remote add fork https://github.com/maschwenk/TypeScript.git    # once; the checkout only has origin
git fetch origin main && git log --oneline -1 origin/main          # if main moved: git rebase origin/main <branch>, re-run the gates

git push fork perf/union-property-cache
tail -n +3 $TSRS/upstream/pr-04-union-property-cache.md | gh pr create -R microsoft/TypeScript --draft --base main \
  --head maschwenk:perf/union-property-cache \
  --title "don't copy union and intersection properties into the augmented property cache" --body-file -

git push fork perf/conditional-instantiation-mapper
tail -n +3 $TSRS/upstream/pr-05-conditional-instantiation-mapper.md | gh pr create -R microsoft/TypeScript --draft --base main \
  --head maschwenk:perf/conditional-instantiation-mapper \
  --title "instantiate conditional types without a combined mapper for the cache lookup" --body-file -
```

Add the `#64475`/`#64526` cross-links by hand if wanted; the texts reference them by number. L1, L11, L10 stay local
until #64475 lands (they need its lazy tables); see "Local branches".

## Local branches (in `$TSRS_WORK/TypeScript-upstream`, none pushed)

On main 82f0546163, rebased 2026-10-01 from edf7da4e93. The 8 upstream commits in between touch `checker.go` only in
`checkInterfaceDeclaration` / `checkEnumDeclaration` / `checkExportsOnMergedDeclarations` (#64566), away from all seven
changes; every cherry-pick applied cleanly and `git range-diff` shows `=` for every commit. One of them (#64093) adds a
lib file, so the private monorepo now has 37,943 files and the 4-checker file assignment shifted (4-checker absolute
numbers below differ from the edf7da4e93 ones for that reason).

| branch | head | what |
| --- | --- | --- |
| `perf/union-property-cache` | 2a3563f0b7 | L6 on main (to open, pr-04) |
| `perf/conditional-instantiation-mapper` | 8dc563d641 | L5 on main (to open, pr-05) |
| `perf/lazy-base` | 15e6d92251 | main + #64475 + #64526, squashed (= `patches/base/`) |
| `perf/lazy-tuple-members` | cc5011c63f | + L1 (on `perf/lazy-base`) |
| `perf/lazy-empty-object-checks` | ae4d097199 | + L11 (on `perf/lazy-tuple-members`) |
| `perf/lazy-unmatched-properties` | 7d1605f445 | + L10 (on `perf/lazy-empty-object-checks`) |
| `lazy-stack-main` | 602f265a99 | + L6 + L5 on top, for measuring the stack (= `patches/`) |

The edf7da4e93-based branches (`lazy-base`, `lazy-stack`, `lazy-stack-ref`, `standalone-L6-L5`) are kept unchanged.
`patches/` was regenerated from the new branches (`git am --keep-cr` on 82f0546163 reproduces the branch trees
exactly); the L6/L5 commit messages on the two `perf/` branches carry the re-measured numbers, the copies in
`lazy-stack-main` still have the edf7da4e93 ones.

Gates on 82f0546163 (README "Tests"), at every commit of all seven branches: `go vet` clean, go1.27 `gofmt -l` empty,
`go test ./internal/checker/... ./internal/testrunner/...` ok (also with `TS_TEST_PROGRAM_SINGLE_THREADED=false`), 0
files in `testdata/baselines/local`, `npx hereby lint` 0 issues, `npx hereby check:format` clean. On both `perf/` L6/L5
branches and `lazy-stack-main`: `go test ./...` ok except `internal/fswatch` (macOS fsevents subtests time out under
machine load, on branches that don't touch it; `go test ./internal/fswatch/...` passes on a rerun at lower load, and
on main). `-race` with concurrent test programs on `lazy-stack-main`: ok (380 s).

## How the stack was built

- Fresh clone of microsoft/TypeScript at main **edf7da4e93** (2026-10-01), Go workspace (`go.work`: `tsc/`, `tools/`),
  go1.27.0 via `GOTOOLCHAIN=auto`. PR heads fetched read-only as references: #64475 at 2aefafb5, #64526 at 05ead00c
  (both based on 0681ef7f, 31 commits behind main).
- **Base: main + #64475 + #64526, each squashed into one commit and rebased onto main** (`patches/base/`), rather than
  #64526's head: #64526 conflicts with the merged #64521 (`somePropertyReducesToNever`, ordered counts), and that
  resolution has to be written anyway. The resolution mirrors tsrs (notes/lazy-members.md): both the skipped
  constituent's lookups and the final check iterate `counts.Entries()` in discovery order; the skipped constituent's
  hits are `counts.Set(name, count+1)` during iteration (existing keys only, so the iteration order is unchanged).
  With that base, private-monorepo counters equal the `tsgo-pr-b` numbers in notes/lazy-members.md exactly (below), and the
  PRs' two tests pass.
- One commit per change, in the suggested order, in the style of Max's PRs (no counters, no new abstractions beyond
  the helper each change needs, comments where his PRs have them). Commits 1-3 each add one compiler test whose
  baselines were generated on **unmodified main** (no #64475/#64526 either), like Max's tests.
- The same stack was cherry-picked onto **b85298b6** (the commit tsrs ports, `ts-ref`) for the Go-vs-Rust counter
  comparison; the patches are identical (`git range-diff` shows `=` for all seven commits).
- L6 and L5 also apply to plain main unchanged (`patches/standalone-main/`, same messages), tested and measured there.

Branches from that first build in `$TSRS_WORK/TypeScript-upstream` (local only, on edf7da4e93): `lazy-base` (main +
PRs), `lazy-stack` (the five commits), `lazy-stack-ref` (same on b85298b6), `standalone-L6-L5`, and the PR worktrees
`../TypeScript-pr-64475`, `../TypeScript-pr-64526`. The current branches are in "Local branches" above.

Apply: `git am --keep-cr patches/base/*.patch patches/*.patch` on main 82f0546163 (`--keep-cr`: the baselines are
CRLF and the repo has `* -text`; without it `git am` strips the CRs and the baseline tests fail).

## Tests (upstream CI's commands)

At every commit of the stack (and of `standalone-L6-L5`), in `tsc/`:

```sh
GOTOOLCHAIN=auto go vet ./internal/checker/...                                      # clean
$(GOTOOLCHAIN=auto go env GOROOT)/bin/gofmt -l internal/checker                     # empty (PATH gofmt is older than go1.27)
GOTOOLCHAIN=auto go test -count=1 ./internal/checker/... ./internal/testrunner/...  # ok, ok
find testdata/baselines/local -type f | wc -l                                       # 0 (no new/changed baselines)
npx hereby lint                                                                     # 0 issues (tsc and tools); needs `npm ci`
npx hereby check:format                                                             # clean
```

On the top commit additionally: `go test ./...` (all packages; `internal/astnav` needs `npm ci` for its node-based
baseline, then passes; `internal/fswatch`'s `TestFSEventsWatchFileDifferentCasing` times out under machine load, on
unmodified main too, and passes when run alone, the same macOS watcher test #64521's description mentions), and
`TS_TEST_PROGRAM_SINGLE_THREADED=false go test ./internal/testrunner/...` (concurrent test programs, i.e. multiple
checkers per program; also on `standalone-L6-L5`). `go test -v` counts 13,485 `TestLocal` cases, 0 failures. Race:
`TS_TEST_PROGRAM_SINGLE_THREADED=false go test -race ./internal/checker/... ./internal/testrunner/...` on the top
commit passes with no race reports (380 s).

## Go vs Rust: counters per commit (the equivalence evidence)

> The per-candidate `TSRS_LAZY_*` switches named below were removed from tsrs on 2026-10-09 (the landed candidates
> follow `TSRS_LAZY_MEMBERS` now; notes/mem-lazy.md). To rerun a per-commit row, build tsrs from a commit before that
> day, such as the e8d4196 these numbers came from.

The private monorepo = a pristine read-only checkout (37,942 files, 0 errors at every commit), `-p <project> --noEmit
--incremental false --extendedDiagnostics`, Go built from `lazy-stack-ref` (b85298b6 + base + commits), Rust =
`target/release/tsrs` of this worktree (built at tsrs e8d4196) with the matching switches (`TSRS_LAZY_MEMBERS=0` for the reference; for the
PRs row all of `TSRS_LAZY_TUPLES/EMPTY/UNMATCHED/PROP_CACHE/COND_MAPPER=0`; then each switch turned on in stack
order; `TSRS_LAZY_HAS_PROP=0` throughout). 4 checkers: Go's default assignment, tsrs with `--checkerAssignment go`.

| build | single: symbols / types / instantiations, Go = Rust | 4 checkers: symbols / types / instantiations, Go = Rust |
| --- | --- | --- |
| b85298b6 (reference) | 25,973,354 / 9,639,962 / 44,884,281 | 39,704,001 / 16,200,921 / 89,981,648 |
| + #64475 + #64526 | 15,331,397 / 9,630,120 / 44,820,708 | 22,813,093 / 16,186,849 / 89,882,712 |
| + L1 tuples | 13,297,831 / same / same | 19,555,571 / same / same |
| + L11 empty-object | 13,082,798 / same / same | 19,243,779 (Go: 3 of 4 runs; 1 run 19,243,785) |
| + L10 unmatched | 12,811,032 / same / same | 18,693,678 (Go: 4 of 6 runs; 2 runs 18,693,684) |
| + L6 property cache | 12,811,032 / same / same | 18,693,678 |
| + L5 conditional mapper | 12,811,032 / same / same | 18,693,678 |

Every single-threaded number is identical between Go and Rust in every run, and the reference and PR rows equal the
numbers in notes/lazy-members.md / notes/mem-lazy.md. With 4 checkers, Go builds sometimes come out 6 symbols
higher: 9 of 52 runs from L11 on across all files in `results/`, 0 of 33 runs of main, PRs, L1 and main+L6/L5, 0 with
1 checker, the Rust port constant. The most frequent Go value equals Rust. Since then traced (review-notes.md, "The
6-symbol variation"): one `Partial<X>` resolved in full or not, depending on a per-process type creation order inside
a checker that unmodified main also has; reproduced on L1 and with the checkers run one after another.

Also equal on the PRs' tests and the new tests (`results/testfiles.md`, symbols/types/instantiations, single file
with `--strict --target esnext`):

| test | tsgo b85298b6 = tsrs opt-out | tsgo + PRs = tsrs PRs only | tsgo + PRs + stack = tsrs default |
| --- | --- | --- | --- |
| instantiatedReferenceLazyMembers | 57787/31559/33474 | 56702/31559/33474 | 56176/31559/33474 |
| mappedTypeLazyMembers (`--lib esnext`) | 11358/5292/4048 | 10785/5292/4050 | 10547/5292/4050 |
| tupleLazyMembers | 58418/31701/34120 | 57240/31701/34120 | 56506/31701/34120 |
| lazyMembersEmptyObjectType | 57635/31441/33277 | 56429/31441/33277 | 55983/31441/33277 |
| lazyMembersUnmatchedProperties | 58382/31867/34379 | 57301/31867/34379 | 56722/31867/34379 |

## Measurements (Go on main, per commit)

Go built from main edf7da4e93 + base + commits. Medians of 3 rounds, builds interleaved within a round, one process
at a time; 18-core machine shared with other jobs (load 8-18). heap = `Memory used` from `--extendedDiagnostics`
(live heap after two GCs; spread between runs < 0.02 GB), allocs = `Memory allocs` (cumulative mallocs;
deterministic single-threaded), peak = `peak memory footprint` from `/usr/bin/time -l` (GB = 10^9 bytes; spreads by
up to 0.8 GB single-threaded and 1.5 GB with 4 checkers between runs of one build, so small peak deltas are noise).
Check time moved by up to +-17% between builds with identical counters and is not claimed.

Single-threaded:

| build | symbols | types / instantiations | heap GB | allocs | peak GB |
| --- | --- | --- | --- | --- | --- |
| main | 25,972,740 | 9,639,290 / 44,879,960 | 14.04 | 148.91M | 18.02 |
| + #64475 + #64526 | 15,330,783 | 9,629,448 / 44,816,393 | 11.01 | 148.82M | 13.88 |
| + L1 | 13,297,217 (-13.3%) | same | 10.47 (-4.9%) | 148.94M | 13.12 |
| + L11 | 13,082,184 (-1.6%) | same | 10.41 (-0.5%) | 149.04M | 12.92 |
| + L10 | 12,810,418 (-2.1%) | same | 10.37 (-0.4%) | 149.29M (+0.25M) | 13.02 |
| + L6 | 12,810,418 | same | 10.25 (-1.2%) | 148.47M (-0.82M) | 13.07 |
| + L5 | 12,810,418 | same | 10.25 | 144.13M (-4.34M) | 12.70 |

4 checkers:

| build | symbols | types / instantiations | heap GB | allocs | peak GB |
| --- | --- | --- | --- | --- | --- |
| main | 39,701,992 | 16,199,269 / 89,962,864 | 21.54 | 248.76M | 25.48 |
| + #64475 + #64526 | 22,811,135 | 16,185,197 / 89,863,935 | 16.32 | 250.10M | 19.50 |
| + L1 | 19,553,613 (-14.3%) | same | 15.41 (-5.5%) | 250.23M | 19.94 |
| + L11 | 19,241,821 (-1.6%) | same | 15.32 (-0.6%) | 250.31M | 19.12 |
| + L10 | 18,691,689 (-2.9%) | same | 15.22 (-0.7%) | 250.83M (+0.5M) | 18.32 |
| + L6 | 18,691,689 | same | 15.00 (-1.5%) | 249.48M (-1.35M) | 18.64 |
| + L5 | 18,691,689 | same | 14.99 | 240.56M (-8.9M) | 19.15 |

Rows main..L11 are from `results/main.jsonl`; L10..L5 from `results/main-v4.jsonl` (the final L10 code), whose L11
re-run gave the same heap/allocs (10.41 GB / 149.04M; 15.32 GB / 250.33M) and is the reference for the L10 deltas.
Whole stack vs the PRs: symbols -16.4% / -18.1%, heap -0.76 GB (-6.9%) / -1.33 GB (-8.1%), allocs -3.2% / -3.8%.

L6 and L5 on plain main (`standalone-L6-L5`): L6 heap 14.04 -> 13.85 GB (-1.4%) single, 21.54 -> 21.26 GB (-1.3%)
4 checkers, allocs -1.0M / -1.4M; L5 allocs 147.91M -> 143.56M (-4.35M) / 247.32M -> 238.50M (-8.8M), heap same.

### Re-measured on main 82f0546163 (the numbers in pr-04 / pr-05)

`results/main-82f0546163.jsonl`, same method (medians of 3, interleaved, one process at a time, load 24-48 this
time), on the same pristine private-monorepo checkout as before (37,943 files with the new lib file, 0
errors). L5 is now measured against main itself (its own branch), not main + L6. Stack = `perf/lazy-unmatched-properties`
(main + PRs + L1 + L11 + L10), then L6, then L5 (`lazy-stack-main~1`, `lazy-stack-main`).

| build | symbols, single / 4 ch | heap GB, single / 4 ch | allocs, single / 4 ch |
| --- | --- | --- | --- |
| main | 25,972,751 / 39,067,109 | 14.03 / 21.25 | 148.86M / 246.89M |
| main + L6 | same | 13.84 (-1.4%) / 20.96 (-1.4%) | 147.86M (-1.0M) / 245.02M (-1.9M) |
| main + L5 | same | 14.03 / 21.24 | 144.51M (-4.35M) / 238.12M (-8.8M) |
| stack to L10 | 12,810,429 / 18,479,183 | 10.36 / 15.07 | 149.24M / 248.70M |
| + L6 | same | 10.23 (-1.3%) / 14.84 (-1.5%) | 148.42M (-0.82M) / 247.09M (-1.6M) |
| + L5 | same | 10.23 / 14.84 | 144.08M (-4.34M) / 238.41M (-8.7M) |

Types / instantiations: main 9,639,290 / 44,879,960 single, 15,968,284 / 88,419,340 with 4 checkers; stack
9,629,448 / 44,816,393 and 15,954,673 / 88,320,433. Single-threaded symbols are +11 vs edf7da4e93 everywhere (the new
lib file); types and instantiations single-threaded are unchanged. Heap spread between runs: < 0.002 GB single, up to
0.15 GB with 4 checkers (more than before; still below the L6 deltas). 4-checker allocs of L10 spread 248.4-251.5M.
L5's malloc delta on the stack is now 4,340,793 (prototype: 4,340,555 composites). Changes vs the old pr-04/pr-05
texts: file count 37,942 -> 37,943, every absolute number above, L6 4-checker allocs -1.4M -> -1.9M on main and
-1.35M -> -1.6M on the stack, L6 single heap on the stack -1.2% -> -1.3%, L5 base changed to plain main, and the test
line (fswatch note). The relative effects are otherwise the same.

### Go vs Rust, effect per commit

| commit | Go symbols, single / 4 ch | Rust symbols (notes/mem-lazy.md) | Go memory | Rust memory |
| --- | --- | --- | --- | --- |
| L1 | -13.3% / -14.3% | -13.3% / -14.3% | heap -4.9% / -5.5%, peak -5.5% single | peak -4.7% / -5.1% |
| L11 | -1.6% / -1.6% | -1.6% / -1.5% (on L1 alone) | heap -0.06 / -0.09 GB | peak -0.04 GB |
| L10 | -2.1% / -2.9% | -1.8% / -2.5% (on L1 alone) | heap -0.04 / -0.10 GB, allocs +0.17% / +0.2% | peak -0.04 / -0.06 GB |
| L6 | 0 | 0 | heap -0.13 / -0.22 GB, allocs -0.82M / -1.35M | -0.11 / -0.13 GB (2.37M / 3.39M entries) |
| L5 | 0 | 0 | allocs -4.34M / -8.9M, heap 0 | -0.10 / -0.17 GB (arena), 4.34M / 7.68M mappers |

(The Rust L11/L10 percentages in the notes were each measured on L1 alone, not stacked; the stacked Rust counts are
the equivalence table above, identical to Go. Rust 4-checker per-path numbers in the notes use the locality
assignment.)

## What did not transfer one to one

- **L10 needed two Go-specific fixes.** The direct translation of the sketch (separate `getLazyPropertiesInOrder` +
  fallback `getPropertiesOfType(target)`, real target properties looked up with `getPropertyOfType(target, name)`)
  gave the same counters but **+1.69M mallocs single-threaded (+1.1%)**. An exact allocation profile
  (`GODEBUG=memprofilerate=1 --pprofDir`, L11 vs L10) put ~1M of them in `getTypeWithThisArgument` under
  `getReducedApparentType`: for a type-parameter target the apparent type is recomputed on every lookup, which
  allocates a type-argument slice even when the reference is cached, and the discriminant path now did one target
  lookup per property. The final version (a) folds the helper into `getPropertiesOfTypeLazily`, which returns what
  `getPropertiesOfType` returns when there is no table, so the apparent type is computed once, and (b) returns the
  reduced type with the table and looks real target properties up there (same result: `getPropertyOfType` reduces
  its argument first, and the reduced type is a plain object reference). Remaining cost: +0.25M mallocs (+0.17%),
  the per-table property lists. Counters unchanged by both fixes (re-verified against Rust).
- **L10 list construction** goes through `addInheritedMembers` + `getNamedMembers` (exactly what `resolveLazyMembers`
  does) instead of the prototype's first-wins name set; the difference would only show if a declared member had no
  `Value` flag, where `addInheritedMembers` lets a base property replace it. Same counters everywhere measured.
- **L5 is an allocation win in Go, not a heap win** (as the notes predicted): the composites are garbage at once.
  Go's malloc delta (4,340,242) matches the prototype's count of avoided composites (4,340,555) within 313.
- **No per-path counters in Go**: Max's PRs have none, so the Go patches have none; the per-path numbers in the PR
  texts come from tsrs, and say so.
- **4-checker run-to-run variation** of 6 symbols (above). Traced on 2026-10-01 (review-notes.md, "The 6-symbol
  variation"): not global symbol ids, not concurrency, not introduced by L11. Unmodified main already creates types in
  a run-dependent order inside a checker (type ids at a fixed site shift between runs even with the checkers run one
  after another); with the lazy tables that order decides whether one lib `Partial<X>` gets resolved in full by
  `isWeakType` under `typeRelatedToSomeType`, its 6 optional members. The order-dependent loop on main is not located.
  Single-threaded runs are constant and equal Rust.

## Reproduction

```sh
# Clone and base
git clone https://github.com/microsoft/TypeScript.git TypeScript-upstream && cd TypeScript-upstream
git checkout -b lazy-stack-main 82f0546163     # edf7da4e93 for the first build's numbers
git am --keep-cr <tsrs>/upstream/patches/base/*.patch <tsrs>/upstream/patches/*.patch
git fetch origin pull/64475/head:pr-64475 pull/64526/head:pr-64526     # references only

# Tests (see "Tests"); npm ci once for hereby and astnav
npm ci && cd tsc && GOTOOLCHAIN=auto go test -count=1 ./... && TS_TEST_PROGRAM_SINGLE_THREADED=false GOTOOLCHAIN=auto go test -count=1 ./internal/testrunner/...

# Builds per commit (main-based; for the equivalence runs the same on b85298b6:
#   git checkout -b lazy-stack-ref b85298b6 && git cherry-pick main..lazy-stack)
for c in $(git rev-list --reverse main..lazy-stack); do git checkout -q $c; (cd tsc && go build -o $BIN/tsgo-$(git log --format=%h -1) ./cmd/tsc); done

# Rust prototype
cd <tsrs worktree> && CARGO_TARGET_DIR=$PWD/target cargo build --release -p tsrs_cli

# The private monorepo runs (read-only checkout; --noEmit --incremental false is mandatory there), medians of 3, interleaved
python3 upstream/tools/measure.py out.jsonl 3 single,multi "prs=$BIN/tsgo-<base>" "L1=$BIN/tsgo-<c1>" ...
python3 upstream/tools/measure.py eq.jsonl 1 single,multi "go-L1=$BIN/ref-L1" \
  "rs-L1=target/release/tsrs:TSRS_LAZY_TUPLES=1;TSRS_LAZY_EMPTY=0;TSRS_LAZY_UNMATCHED=0;TSRS_LAZY_PROP_CACHE=0;TSRS_LAZY_COND_MAPPER=0;TSRS_LAZY_HAS_PROP=0"
python3 upstream/tools/summarize.py out.jsonl

# PR tests and new tests, Go vs Rust (BIN holds ref-base, ref-prs, ref-L5 built from lazy-stack-ref)
BIN=... TF=<dir with the five .ts files> bash upstream/tools/testfiles.sh

# Allocation profile (exact; ~30 min per run)
GODEBUG=memprofilerate=1 $BIN/tsgo-<c> -p <project> --noEmit --incremental false --singleThreaded --pprofDir <dir>
go tool pprof -sample_index=alloc_objects -top -diff_base <before>/*-memprofile.pb.gz $BIN/tsgo-<c> <after>/*-memprofile.pb.gz
```

`measure.py` runs `/usr/bin/time -l <bin> -p <project> --noEmit --incremental false --extendedDiagnostics`
(`--singleThreaded`, or `--checkers 4` plus `--checkerAssignment go` for tsrs) with its cwd outside the private monorepo.
`git -C <pristine root> status --short` was empty after all runs.
