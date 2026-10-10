# perf-hub-edit-shortcut: three ways to make a whole-program edit cheaper, and what came of them

Follow-up to `perf-dev-loop2.md` ("Why a hub edit is still slower than cold"). On the 38k-file codebase an edit to
a widely imported file re-checks every file, and before that Go's algorithm computes the declaration signatures of
the ~22k files its walk visits. Three ideas were built and measured; only the third landed.

## Outcomes (do not retry the first two)

| idea | result | why |
| --- | --- | --- |
| 1. Version-as-signature shortcut: under `--noEmit`, when an edit re-checks >= 50% of the program, store each closure file's version as its signature and skip the walk (draft #64, never merged) | **rejected** | Hub edit 10.7 -> 7.9 s, global `.d.ts` 18.0 -> 7.2 s, same diagnostics; but every file it skipped then re-checks the whole program on its next body-only edit (1.0 -> 7.6 s), once per file, 22k files after one hub edit. Also differs from tsgo's stored signatures (4 tsctests baselines change). |
| 2. Deferred signatures: drop the predicted re-check set before the check, run Go's unchanged walk after it on warm checkers (branch `perf/hub-edit-deferred-signatures`, parked) | **rejected** | Exact diagnostics and pending emits, and the predicted re-check set matched Go's in every run; but only ~1 s faster on the hub edit (11.1 -> 10.0 s), and the stored signatures differ from base for 7-11 of ~22k files: printed declarations whose member order follows type creation order (a union of object literals prints the filled-in `image?: undefined; presentation?: undefined` in the other order once the check has created its types first). tsgo itself is not reproducible on these files: 3 tsgo runs of the same hub edit stored 2 different signatures for `menu/tests/fixtures/index.ts`, and base already differs from tsgo on 3 files. Equality with base would require computing the signatures in the pre-check checker state, which gives up the gain. |
| 3. Batched global-scope signatures: when a change affects the global scope, phase 2 asked for every file's declaration signature one emit at a time (27k emits); compute them with one emit | **landed** | Each checker emits its files in the same sorted order, so every checker goes through the same states: tsbuildinfo byte-identical to base, same Types/Symbols/Instantiations. Global `.d.ts` edit 17.96 -> 10.53 s; nothing else changes. Section "Batched global-scope signatures" below. |

## Idea 1 as built (draft #64, not on main)

Status (2026-10-10): the ~120-line exactness argument, trigger design and check log for idea 1 were removed in a
condense; they described code that never merged (`TSRS_HUB_SHORTCUT` does not exist on main). What stays is what it
did, what it changed relative to tsgo, and the measurements.

`collectAllAffectedFiles` (affectedfileshandler.go) drops the diagnostics of a changed file F's whole referenced-by
closure C(F) (or of every file, once C(F) holds a file that affects the global scope) whatever the propagated
declaration signatures turn out to be; the signatures decide only what is stored for the visited set V and V itself,
which is added to the pending-emit set. The shortcut: under `--noEmit` (not `composite`, `--build`,
`isolatedModules` or `assumeChangesOnlyAffectDirectDependencies`), after F's own signature is computed and found
changed, compute C(F) from the reference map; if the files to be re-checked are at least half of the non-library
files and every emittable file of C(F) is already pending a full emit, give every file of C(F) except F its version
as signature and skip the walk and its declaration emits.

Differences from tsgo: `fileInfos[].signature` of V minus F holds the version instead of the d.ts hash; the tsbuildinfo
`packageJsons` / `missingPackageJsons` lists are subsets (corpus hub edit: 3,532 of 3,549 missing package.jsons);
`--extendedDiagnostics` counters are lower. Reported diagnostics and pending emits identical. With it on by default,
4 tsctests baselines changed (374 -> 370 pass; only stored signatures, buildinfo `original`/`size` and
"(used version)" lines; before the pending-emit condition was added, 8 changed).

### Re-check set sizes (why the threshold was 50%)

Re-check set size per file, from each corpus's reference map (`affectsGlobalScope` files included; a file whose
referenced-by closure reaches one re-checks everything), as a share of the non-library files:

| corpus | files | < 10% | 10-49% | 50-99% | 100% |
| --- | --- | --- | --- | --- | --- |
| the 38k-file codebase | 37,863 | 12,673 | 0 | 0 | 25,190 |
| vscode `src` | 10,353 | 7,744 | 1,046 | 0 | 1,563 |
| webpack | 1,557 | 749 | 631 | 46 | 131 |
| xstate | 1,444 | 527 | 1 | 0 | 916 |

On the 38k-file codebase two thirds of the files re-check the whole program when their declaration changes (one
large reference cycle through the files declaring ambient modules, plus test files that affect the global scope) and
the rest re-check under 10%; nothing is in between, so any threshold from 10% to 95% makes the same decisions. 50% fires on
whole-program edits on all four corpora (and webpack's 46 files at 50-59%), and on nothing that re-checks a minority
of the program. The threshold does not control the cost on later runs (below): that cost is per skipped file.

## Measurements

Sweep: single runs, `--noEmit --incremental`, 4 checkers, load 6-13. Start state "H": a cold incremental
tsbuildinfo, then a Go-algorithm hub edit and its revert (so the files those walks visited hold declaration
signatures). Seconds total (emit in parentheses); the shortcut was forced on (also below the threshold) for this table.

| edit | files re-checked | Go algorithm | shortcut |
| --- | --- | --- | --- |
| leaf: `export type` appended to a file nothing imports | 1 | 1.14 (0.20) | 1.06 (0.20) |
| service with 8 importers (closure 1,797 = 4.7%, which reaches a test file that affects the global scope) | 37,863 | 8.12 (0.81) | 7.43 (0.21) |
| service with 23 importers, inside the large cycle | 37,863 | 15.06 (6.32) | 9.45 (0.27) |
| ORM hub, 1,870 importers | 37,863 | 9.87 (1.86) | 8.40 (0.59) |
| global `.d.ts` (`interface` appended) | all | 18.00 (12.05) | 7.19 (0.21) |
| ORM hub, from a cold tsbuildinfo | 37,863 | 10.81 (4.52) | 7.86 (0.56) |
| global `.d.ts`, from a cold tsbuildinfo | all | 16.98 (11.26) | 7.08 (0.20) |

The global case is the largest: Go then computes a declaration signature for every file, one emit per file (27,427
emits, sequential; now batched into one emit, see the last section). Diagnostics identical in every row; tsbuildinfo equal except signatures and package.json lists.

### The cost on the following run

The shortcut leaves the files of V (which Go would give a declaration signature) with their version. The next time
one of those files gets an edit that does not change its declaration (a function body), Go compares the new
declaration signature with the stored one, finds it unchanged and re-checks that one file; after the shortcut the
stored value is the version, so the edit counts as a declaration change and re-checks the file's whole re-check set
(for the files of the large cycle: everything, at which point the shortcut fires again). That file's signature is
computed in that run, so the cost is paid once per file. Measured (body-only edit = a non-exported `const` appended;
same edit after each history; single runs, load 7-16; the two 8-importer rows rerun with the final binary):

| previous run | next edit | after Go algorithm | after shortcut |
| --- | --- | --- | --- |
| service-in-cycle edit | body edit of the 8-importer service (visited by that walk) | 1.38 | 7.08 |
| global `.d.ts` edit (Go computes every file's signature) | body edit of the in-cycle service | 1.18 | 8.41 |
| global `.d.ts` edit | body edit of the 8-importer service | 1.04 | 7.09 |
| ORM hub edit | body edit of the in-cycle service (not visited by Go's walk either) | 11.43 | 8.13 |
| any of the above | body edit of a file nothing imports | 1.1-1.4 | 1.1-1.4 |
| any | body edit of the file the previous run edited | 1.2-1.4 | 1.2-1.4 (that file keeps its computed signature) |

So the shortcut saves 1.5-11 s on the run that edits a widely used file and costs about a cold check (+6-7 s) on the
first later body-only edit of each file that run skipped (22k files for the ORM hub edit from cold, 21.7k for the
in-cycle service, 27k for the global `.d.ts`). One such edit already takes back the saving of an ORM hub edit. It
comes out ahead only when few of the skipped files are edited before their signatures get computed some other way
(an edit to one of their dependencies that Go's walk passes through, or an emitting run).

Go has the same shape of cost after every cold run: all signatures are versions then, and the first edit of a file
in the large cycle walks and re-checks everything, but that walk computes the visited files' signatures, so the cost
is paid once for all of them together rather than once per file.

### Before/after

Base = origin/main `1ab17ae`, new = `8369d76` (this branch before merging origin/main, whose new commits are lint
changes), tsgo = `tsgo-ref`; 4 checkers;
seconds, median of 3 interleaved rounds (range), peak footprint GiB; load 5-13. Each binary starts from its own cold
incremental tsbuildinfo; "hub edit" appends `export type __TsrsHubEditN = number;` to the ORM hub; the next three
rows each start from the hub-edit run's tsbuildinfo, with the hub edit kept.

| scenario | base | new | tsgo |
| --- | --- | --- | --- |
| hub edit | 10.71 (10.63-10.74), peak 6.52 | **7.91** (7.87-7.96), peak 6.42 | 72.6 (70.6-72.9), peak 27.1 |
| then a body edit of a file the hub edit's walk visited | 1.04 (1.03-1.05), peak 2.38 | **7.57** (7.45-7.70), peak 6.38 | 4.54 (4.50-4.61) |
| then a body edit of a file nothing imports | 1.03 (1.03-1.04) | 1.07 (1.04-1.11) | 7.39 (7.36-7.44) |
| then no edit | 1.04 (1.04-1.04) | 1.04 (1.03-1.46) | 4.46 (4.46-4.48) |
| cold, `--incremental false` | 6.62 (6.57-6.77), peak 5.70 | 6.66 (6.57-10.72), peak 5.70 | 18.19 (18.10-18.31), peak 23.3 |

Hub-edit tsbuildinfo, base vs new, 3/3 rounds: equal except 22,306 signatures (all the version) and 17 fewer
`missingPackageJsons`; diagnostics identical.

## The decision it needed

Whether tsrs may store different `signature` values than tsgo in `--noEmit` incremental runs. The trade on this
codebase was negative for an edit loop that touches a shared file and then keeps editing files that depend on it, and
positive only for one-off whole-program edits; rejected (see the outcomes table).

## Not done

- Computing the skipped signatures after the check instead (one emit for all of C(F), no level barriers, warm
  checkers): would keep later runs cheap and differ from tsgo only by storing declaration signatures where tsgo stores
  versions; not measured.

## Batched global-scope signatures (landed)

`collectAllAffectedFiles` with a changed file that affects the global scope (`hasAllFilesExcludingDefaultLibraryFile`):
every file is affected, and `handleDtsMayChangeOfAffectedFile` calls `updateShapeSignature(file, false)` for each, in
sorted path order, each a separate emit of one file on that file's checker. Now the signatures of the files that
still need one are computed first with one emit (`compute_dts_signatures`, as the level walk already does), which runs
each checker's files on that checker in the order given, and `update_shape_signature` takes them from there. Per
checker the sequence of files emitted is unchanged, and the checkers are independent, so the printed declarations,
and with them the signatures and counters, are the same.

Base = origin/main `6ec9917`, new = this change; 38k-file codebase, 4 checkers, each run from the same cold
incremental tsbuildinfo with the edit applied; wall seconds, median of 3 interleaved rounds (range), peak footprint
GiB; load 16-28.

| scenario | base | new |
| --- | --- | --- |
| global `.d.ts` edit (`interface` appended to `src/express.d.ts`) | 17.96 (17.94-18.10), 6.38 | **10.53** (10.38-10.93), 6.44 |
| hub edit (`src/orm/db.ts`) | 11.54 (11.33-17.33), 6.50 | 11.32 (11.32-11.60), 6.52 |
| cycle-service edit (`src/services/menu/models/modifier/index.ts`) | 11.76 (11.71-11.78), 6.52 | 11.66 (11.52-11.85), 6.50 |
| leaf body edit | 1.26 (1.13-1.26), 2.35 | 1.20 (1.20-1.29), 2.33 |
| no edit | 1.14 (1.13-1.15), 2.36 | 1.15 (1.12-1.17), 2.35 |

Emit time of the global edit 11.78 -> 4.21 s (the 27k signatures, now ~55% parallel on 4 checkers).

Exactness: base is deterministic on these edits (the same edit three times gives byte-identical tsbuildinfo for the
global, hub and cycle-service edits, so nothing is excluded); base vs new tsbuildinfo byte-identical in all 15 run
pairs, diagnostics identical, `Types` / `Symbols` / `Instantiations` identical (global edit: 13,828,794 / 16,590,632 /
77,206,595 on both).
