# perf-hub-edit-shortcut: skip the propagated declaration signatures when an edit re-checks nearly everything

A deliberate departure from tsgo, proposed in `perf-dev-loop2.md` ("Why a hub edit is still slower than cold"), behind
`TSRS_HUB_SHORTCUT=0`. This note has the exactness argument, the trigger, and the measured cost on later runs.

## What changes

`collectAllAffectedFiles` (affectedfileshandler.go) works in two phases for each changed file F:

1. `getFilesAffectedBy(F)`: compute F's declaration signature. If it changed (and F does not affect the global scope,
   and no `isolatedModules`), walk F's referencing files, computing each one's declaration signature, and continue
   through the files whose signature changed. Returns the visited set V.
2. `handleDtsMayChangeOfAffectedFile` for every file of V: drop its cached diagnostics; and for F (a changed file whose
   signature changed) drop the diagnostics of `referencedBy(referencedBy(F))` and everything that references those,
   transitively, giving each of them its version as signature (`updateShapeSignature(file, true)`). If any of those
   files affects the global scope, every file loses its diagnostics instead.

V always contains `referencedBy(F)`, so the files that lose their diagnostics are F's whole referenced-by closure C(F)
(or every file, once C(F) holds a file that affects the global scope), whatever the propagated signatures turn out
to be. The propagated signatures decide only (a) the signatures stored for V and (b) V itself, which is added to the
pending-emit set.

The shortcut: after F's own signature is computed and found changed, compute C(F) from the reference map (no emit).
If the files that will be re-checked are at least half of the program's non-library files, give every file of C(F)
except F its version as signature and return C(F) as the affected set, skipping the walk and its declaration emits.
In the global-scope case (F affects the global scope, phase 2 computes a declaration signature for *every* file, one
emit per file), the files take their version too. F itself keeps its computed declaration signature.

## Exactness argument

### When Go stores a file's version as its signature

- Every file of a run without an old program (programtosnapshot.go:149, `signature = version`): every cold run.
- Every file that phase 2 reaches outside V (`handleDtsMayChangeOf` -> `updateShapeSignature(file, true)`,
  affectedfileshandler.go:324), and every file under `handleDtsMayChangeOfGlobalScope`.
- `updateShapeSignature(file, false)` for declaration files, JSON files, and files whose declaration emit writes
  nothing (`computeDtsSignature` returns "").
- A tsbuildinfo `fileInfos` entry in string form means version = signature (buildInfo.go:98, 122).

So a version-signature on any file is a state tsgo itself produces (after a cold run, every file has one). The
shortcut produces it on more files; it never produces a value of another kind.

### Every reader of a stored signature, and what a version-signature does to it

Let s_d be the declaration signature Go would store for file X (hash of its d.ts text plus declaration diagnostics)
and s_v the version the shortcut stores (hash of its source text). Same hash function, so s_v = s_d only when the
two strings are equal, in which case nothing differs at all.

1. `updateShapeSignature` (affectedfileshandler.go:111), "did X's shape change": decides whether a later walk
   continues through X, and for a changed file whether anything beyond it is affected. A later run compares a fresh
   d.ts hash h with the stored value. With s_v, h = s_v only if X's new d.ts text equals X's source text at the time
   of the shortcut run; otherwise "changed". So the walk continues through X at least whenever it would with s_d,
   except in that degenerate case, which Go has with every version-signature it stores (cold runs).
2. `isChangedSignature` (affectedfileshandler.go:52) for a changed file: gates phase 2's closure. Same comparison as
   (1): with s_v it reports "changed" at least as often, so more files lose their diagnostics, never fewer.
   In the `isolatedModules` branch it also stops the walk at referencing files whose version equals their stored
   signature, so a version-signature stops that walk earlier; but phase 2 then walks `referencedBy(referencedBy(F))`
   transitively anyway (the code after the `isolatedModules` block runs in both modes), so the files that lose their
   diagnostics and get a pending emit are the same. Only reachable if a later run turns `isolatedModules` on (the
   shortcut itself never runs under it).
3. `programtosnapshot.go:108`: copies the old signature into the new snapshot. No decision.
4. `emitfileshandler.go:211`: when a declaration is emitted for a file whose signature equals its version, the
   signature becomes the emitted d.ts hash. Emit only; under a later emitting run a version-signature gets upgraded,
   exactly as after a cold run.
5. `buildinfotosnapshot.go:125` and `snapshottobuildinfo.go:239`: under `composite`, the stored signature seeds and is
   compared with the emit signatures (whether a d.ts output changed, `latestChangedDtsFile`, which `--build` uses for
   downstream projects). The shortcut never runs under `composite`.
6. `buildInfo.go:98-112`: encoding (a file whose signature equals its version is written in the short form). Bytes
   only.
7. `--build` up-to-date checks (build/buildtask.go): input mtimes, roots, `latestChangedDtsFile`, package.json lists;
   never `fileInfos[].signature`. The shortcut does not run under `--build` anyway.
8. The tsctests harness prints signatures and "(computed .d.ts)" / "(used version)" (test output only).

### Where it must not apply, and why

- Any emit (`noEmit` false): declaration outputs on disk do not depend on this, but emit signatures, `dtsChangeTime`
  and `latestChangedDtsFile` do (readers 4-5), and the pending-emit set grows from V to C(F) (more files emitted).
  Restricted to `noEmit`. Under `noEmit`, every file is already pending emit with the full kind (the cold run adds
  them all; a switch to `noEmit` from an emitting run re-adds them all, since `noEmit` affects emit), so returning
  C(F) instead of V changes nothing there; checked on the corpus and the fixtures (`affectedFilesPendingEmit`
  identical).
- `composite` and `--build` (readers 5 and 7).
- `assumeChangesOnlyAffectDirectDependencies`: phase 2 returns early, so the dropped diagnostics are V itself, which
  then does depend on the signatures. Returning C(F) would re-check files Go deliberately leaves alone and could
  change the reported diagnostics.
- `isolatedModules`: `getFilesAffectedBy` returns F alone before any walk; nothing to skip.

### What does differ from tsgo

- `fileInfos[].signature` of C(F) minus F: version instead of the d.ts hash (for the files of V; the rest of C(F)
  has the version in tsgo too).
- `packageJsons` / `missingPackageJsons` in the tsbuildinfo are what the run looked up; the skipped declaration
  emits look up package.jsons for module specifiers, so these lists are subsets of tsgo's (corpus hub edit: 3,532 of
  3,549 missing package.jsons). Read by `--build` up-to-date checks only, and by the tsbuildinfo-rewrite decision
  (program.go:341).
- `--extendedDiagnostics` counters of the run (Types, Symbols, Instantiations): lower, since the skipped emits
  created types.
- Reported diagnostics: identical. Every file of the closure is checked either way, against the same program.
- Pending emits: unchanged. Returning C(F) instead of V gives every file of C(F) a pending emit of the full kind.
  That is a superset of Go's and would make a later emitting run re-emit more files (with the same content), so the
  shortcut only applies when every emittable file of C(F) is already pending a full emit. That holds in the steady
  `--noEmit` state (a cold run makes every file pending and `--noEmit` never clears them) and fails after an emitting
  run until the next cold one, where Go's walk runs. Found by the tsctests `tsc/noEmit/changes-*` scenarios, which
  alternate `--noEmit` and emitting runs.

## The trigger

After F's own declaration signature is computed and has changed (so one d.ts emit for F, as in Go), and only with
`noEmit`, without `composite`, `--build`, `isolatedModules`, `assumeChangesOnlyAffectDirectDependencies`, and with
`TSRS_HUB_SHORTCUT` not `0`:

- `rechecked` = the number of files that will lose their diagnostics: every non-library file if some file of C(F)
  other than F affects the global scope, else |C(F)|. For the global-scope case of F itself (phase 2's per-file
  signatures), the whole program.
- fire when `rechecked * 100 >= 50 * (non-library files)` and every emittable file of C(F) is already pending a full
  emit (above).

Deterministic: it depends only on the reference map, the stored file infos and the options, all known before any
signature work.

### Choosing X

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
the rest re-check under 10%; nothing is in between, so any X from 10 to 95 makes the same decisions. X = 50 fires on
whole-program edits on all four corpora (and webpack's 46 files at 50-59%), and on nothing that re-checks a minority
of the program. X does not control the cost on later runs (below): that cost is per skipped file.

## Measurements

Sweep: single runs, `--noEmit --incremental`, 4 checkers, load 6-13. Start state "H": a cold incremental
tsbuildinfo, then a Go-algorithm hub edit and its revert (so the files those walks visited hold declaration
signatures). Seconds total (emit in parentheses); the shortcut was forced on (also below X) for this table.

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
emits, sequential). Diagnostics identical in every row; tsbuildinfo equal except signatures and package.json lists.

### The cost on the following run

The shortcut leaves the files of V (which Go would give a declaration signature) with their version. The next time
one of those files gets an edit that does not change its declaration (a function body), Go compares the new
declaration signature with the stored one, finds it unchanged and re-checks that one file; after the shortcut the
stored value is the version, so the edit counts as a declaration change and re-checks the file's whole re-check set
(for the files of the large cycle: everything, at which point the shortcut fires again). That file's signature is
computed in that run, so the cost is paid once per file. Measured (body-only edit = a non-exported `const` appended;
same edit after each history):

| previous run | next edit | after Go algorithm | after shortcut |
| --- | --- | --- | --- |
| service-in-cycle edit | body edit of the 8-importer service (visited by that walk) | 1.20 | 10.48 |
| global `.d.ts` edit (Go computes every file's signature) | body edit of the in-cycle service | 1.18 | 8.41 |
| global `.d.ts` edit | body edit of the 8-importer service | 1.12 | 8.08 |
| ORM hub edit | body edit of the in-cycle service (not visited by Go's walk either) | 11.43 | 8.13 |
| any of the above | body edit of a file nothing imports | 1.1-1.4 | 1.1-1.4 |
| any | body edit of the file the previous run edited | 1.2-1.4 | 1.2-1.4 (that file keeps its computed signature) |

So the shortcut saves 1.5-11 s on the run that edits a widely used file and costs about a cold check (+6-9 s) on the
first later body-only edit of each file that run skipped (22k files for the ORM hub edit from cold, 21.7k for the
in-cycle service, 27k for the global `.d.ts`). One such edit already takes back the saving of an ORM hub edit. It
comes out ahead only when few of the skipped files are edited before their signatures get computed some other way
(an edit to one of their dependencies that Go's walk passes through, or an emitting run).

Go has the same shape of cost after every cold run: all signatures are versions then, and the first edit of a file
in the large cycle walks and re-checks everything, but that walk computes the visited files' signatures, so the cost
is paid once for all of them together rather than once per file.
