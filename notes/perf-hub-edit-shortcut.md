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
