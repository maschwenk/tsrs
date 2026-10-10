# emit/incremental: E13 incremental (tsbuildinfo) + E14 `--build` + E15 tsctests harness

Waves E13-E15 (2026-10-03): `crates/tsrs_incremental` (Go `execute/incremental`), the `-b` build mode
(Go `execute/build`, now in `crates/tsrs_execute/src/build/`) with `--builders` concurrency, and the tsctests harness.
Design and current status: docs/EMIT.md section 10b; per-wave results: docs/EMIT.md section 13 (progress tables).
This note keeps the measurements and root causes that are not written down elsewhere. Watch mode is not ported.

## Results (head `5b33672`, 2026-10-03)

- Edit sequences (`tools/oracle/incremental/fixtures/run-all.sh`; output, emitted files and tsbuildinfo compared
  after every step against tsgo built from ts-ref `b85298b6`): `inc1`, `inc2`, `b1` (incl. output deletion: a deleted
  upstream `.d.ts` leaves the upstream project "up to date" and the dependent reports TS6305, like tsgo). All
  identical.
- xstate-main (1,536 files, `TSRS_EMIT=1 tsrs -p . --incremental`, best of 3; npm tsgo in brackets): no incremental
  0.43 s (1.02); cold incremental 0.51 s (1.08); no change 0.11 s (0.20); body edit of a leaf file 0.14 s (0.25),
  except the first edit after a cold build (0.53 s; tsgo 1.09 s), which rechecks importers once because a cold build
  stores versions as signatures, as in Go; shape edit of `packages/core/src/types.ts` 0.53 s (1.09). Its tsbuildinfo is
  byte-identical to tsgo's, cold and after an edit.

## Program retention in `-b` (measured, not changed)

Bounded public graph `xsgraph` (one project per xstate-main package, 14 projects plus the root, `incremental` +
`noEmit`, `types: []`), on `e92f7b5`: identical `-b --verbose` output and tsbuildinfo to tsgo-ref at `--builders
1/4/8`. Wall (best of 3) and peak RSS (max of 3): tsrs 1.14 s / 1,119 MB, 0.96 s / 1,161 MB, 0.95 s / 1,175 MB; tsgo
2.10 s / 531 MB, 1.10 s / 1,129 MB, 1.16 s / 1,857 MB. `-b core` alone (the largest project, 137 files): tsrs 216 MB,
tsgo 183 MB.

Go's `buildOrCleanProject` sets `task.result.program = nil` when not testing and the GC reclaims the program (the
host's cached `.d.ts`/JSON files stay). tsrs drops its `P<incremental::Program>` handle at the same point, but the
memory stays: `free_program` (tsrs_compiler) only frees the `Program` struct and its resolution host, not the arena
memory (AST, checker types) that makes up almost all of a program. So tsrs's peak at `--builders 1` grows with the sum
of all projects (1,119 MB for the graph vs 216 MB for its largest project) where Go's tracks the projects in flight
(531 MB). Single-project builds of the 15 projects peak at 320 MB at most in tsrs (253 MB in tsgo), so the graph's
1.1 GB at one builder is retained per-project data (about 60 MB per project above the shared baseline).

Status (2026-10-10): still true for CLI builds. Only API builds free each finished program
(`free_unshared_program` under `use_regions`, crates/tsrs_execute/src/build/orchestrator.rs, `enable_api_regions`).

Public emitting graph `xsdts` (the same packages with `declaration` + `emitDeclarationOnly`, `outDir` and an explicit
`rootDir`), output and trees identical to tsgo-ref at `--builders 1/4/8`. Peak RSS, cold (max of 3) / rebuild of
`core` after deleting its tsbuildinfo:
- `e4c6a96` (with the rejected checker revision `600723a`): 1,281 / 561 MB, 1,319 / 611 MB, 1,339 / 655 MB at
  1 / 4 / 8 builders;
- `535adce` (no checker fix): 1,273 / 559, 1,313 / 607, 1,350 / 647 MB;
- tsgo-ref: 465 / 222, 1,072 / 389, 1,955 / 393 MB.

On this graph the per-project arena retention, not checker threads, sets tsrs's peak at low builder counts. On
`db2ae47` (with the effective checker-thread fix, notes/emit-checker-thread-fix.md) `xsdts` cold wall / peak RSS
(best / max of 3): 1.22 s / 1,128 MB, 0.38 s / 1,311 MB, 0.29 s / 1,371 MB at 1 / 4 / 8 builders; tsgo-ref 2.05 s /
529 MB, 1.02 s / 1,106 MB, 1.29 s / 2,078 MB.

## `m_times` deadlock (fixed in `27da392`)

A tsctests run hung (all threads in futex waits): a checker thread in `WriteFile` -> `host.get_m_time` waited on the
build host's `m_times` mutex, held by a guard created inside `compile_and_emit`'s `EmitInput` argument list for the
whole emit. Reached only when a `.d.ts` differs only in its map option (`differsOnlyInMap`), i.e. once declaration maps
exist; it hangs at `--builders 1` as well, so it is not scheduler-specific. Fix: take the Arc before the call (the
host now clones the inner `Arc<Mutex<..>>` and locks that, crates/tsrs_execute/src/build/host.rs). Reproducer:
fixture `dmap` (composite + emitDeclarationOnly, enable `declarationMap`, `-b` again); the scenarios
`tsbuild/commandLine/different-options` and `tsbuild/sample/when-declarationMap-changes`.

## Known tsctests failure: `tsc/incremental/internal-symbolname-in-tsbuildInfo`

Go's internal symbol prefix is the invalid UTF-8 byte `\xFE`, printed as U+FFFD in TS2783's message and sanitized by
the harness; tsrs uses `\x7f` (tsrs_ast symbol.rs `InternalSymbolNamePrefix`, valid UTF-8), so the message and its
unsanitized symbol id differ. A representation choice, not incremental/build code; left as is. On `db2ae47` the other
tsctests failures were the 31 unported `--help` / `--init` / `--showConfig` / `--locale` / `--generateTrace` outputs
(tsc 187/216, tsbuild 187/190, 0 crashes).
