# Rust-owned compiler and incremental program roots (2026-10-10)

This continues the full ownership goal from `9500bf1e`. Prior ownership/region evidence and the rejection list
were reviewed in `rust-owned-program-data.md`, `rust-owned-checker-inputs.md`, `rust-owned-checker-leases.md`,
`perf-round2-followups.md`, `mem-round4.md` section 6 and `docs/RUST.md` Techniques. This replaces the remaining
root ownership boundary; it does not retry shared checker graphs or claim the cumulative performance gates pass.
This is an unfinished branch checkpoint. The measured regressions below rule out landing it as a performance
change, and the complete ownership goal remains active.

Compiler program construction/reuse returns `Arc<Program>`. The leaked root and unsafe `free_program` API are
removed. Retained project/API/language-service/auto-import/harness values own roots; short-lived helpers borrow
them. Program and checker operations no longer require a static root. The API contention gate also retains the
program while a lease is held. Fourslash's serialized roots retain themselves instead of an extra erased project
owner. The emit host retains input data, not the outer root or its pool.

Program data retains its construction region, the shared full-build base, and source-file/config regions. The
project's separate `programOwner`, manual checker/program freeing and weak-owner validity gate disappear. This
retention still discovers file regions through the legacy address registry; it is a bridge to typed AST ownership.
Built-in checkers now own separate regions, selected while their owned leases are held. Keeping checker allocation
separate from the input regions avoids node-builder/input owner cycles. Pool teardown precedes the outer data
owner's drop. A lease may keep its slots, checker, inputs and graph regions after the outer root drops.

Incremental program roots also use `Arc`, and their operations borrow them without reconstructing `P` roots from
static references. Their snapshots and cache entries remain legacy graph data. The incremental owner retains its
snapshot region and its preceding owner because copied cache entries/testing snapshots can still refer into that
graph. This must be replaced by owned snapshot/cache referents before claiming fine-grained reclamation. API build
teardown releases roots left in task results by unwinding before dropping task regions; build tasks and the
orchestrator itself still have legacy arena/static ownership.

The emit harness retains its pre-emit root along with the compilation result: a diagnostic-count mismatch can
return pre-emit checker diagnostics. Dropping just the pre-emit root would free their checker region before the
baseline consumes them. This preserves their actual owner instead of relying on a leaked program.

Binding must select the source file's owner, not a private checker region. Emit's no-check repeat can bind cached
files for the first time; storing their symbols/flow nodes in that checker's region leaves dangling references in
the source-file cache after the root drops. The initial full emit run spun and crashed; a 439-case focused run
confirmed this retention bug. Selecting the file owner in `bind_source_file_worker` restores all 439 classifications
and the complete emit run. A new case verifies that bound symbols survive destruction of the unrelated active
checker region. The cache's implicit thread arena is still legacy storage, not complete AST ownership.

`CompileAndEmitResult` retains its compiler or incremental root as well. Its diagnostics may refer into a checker
region or an earlier incremental snapshot; returning their raw handles alone would lose the owner. Its input
borrows an `Arc`, cloning the owner only for the returned result. A case exercises both paths, consumes a returned
diagnostic after the caller drops its roots/region, and observes final program/region destruction when the result
drops. Arbitrary extraction of raw graph handles remains part of the broader graph API migration.

## Measurements

Apple M3 Max / macOS / Rust 1.99.0, fat-LTO release builds without PGO, matching Xcode 14.5 SDK. Five interleaved
runs per binary/project/mode after warm-up, with no other validation jobs running. Instructions retired and peak
RSS come from `/usr/bin/time -l`. Exit codes and stdout hashes agree in every cell. The 80 raw samples, binary
checksums and changed Rust source hashes are in `rust-owned-program-roots-results.json`.

| Project/mode | Instructions, `9500bf1e` → this stage | Peak RSS, `9500bf1e` → this stage |
| --- | --- | --- |
| Compiler, single | 2.136437 → 2.348237 G (+9.91%) | 112.48 → 115.11 MiB (+2.33%) |
| Compiler, default | 2.508547 → 2.954289 G (+17.77%) | 146.27 → 162.70 MiB (+11.24%) |
| Compiler-Unions, single | 4.632389 → 4.871978 G (+5.17%) | 104.86 → 117.52 MiB (+12.07%) |
| Compiler-Unions, default | 5.740291 → 6.186204 G (+7.77%) | 144.75 → 163.08 MiB (+12.66%) |

Cumulative single-thread instructions against original Oxc are +11.09% / +5.60%. Default RSS against pre-Oxc is
+109.22% / +98.31% (77.77 → 162.70 MiB / 82.23 → 163.08 MiB). These fail the full instruction/memory gates and
the local incremental limits. Private checker regions now exercise the address registry/allocation-routing
compatibility path in the CLI; these measurements do not isolate how much of the cost comes from it. The next
graph ownership work must replace that path and recover these costs. No speed or memory improvement is claimed.

## Verification

Final API 77, checker 18, compiler 20, incremental 6, language service 67, project 105 (two existing ignored), and
native LSP 41 cases pass, along with the returned-diagnostic ownership case and two compiler compile-fail doctests.
The tsctests test entry finds no scenario dump; it does not validate tsc/tsbuild scenario baselines. Those and the
full emit/incremental oracle remain required. Linux/glibc API RSS assertions are not exercised on macOS.

Full six-baseline classifications over 15,197 pinned TypeScript variants exactly match both original Oxc and
`9500bf1e`: diagnostics 13,462; types/symbols 12,779 each; JS 13,392; JS maps 149; sourcemap text 156. The corrected
full run has no crashes/timeouts. Default mode (`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`)
also has identical classifications, including the existing diagnostic/type failures. Final fourslash has the same
4,066 pass / 63 fail / 417 skip sets over 4,546 cases; its native watcher access is enabled. All 28 regression
fixtures pass. No capability counts change, so the README capability/benchmark tables remain unchanged.

Workspace checks, all CLI features, wasm32-wasip1 and final release builds pass. The final ratchet has three
baseline findings and none new; source checks pass with 28 reviewed custom thread implementations and no
inventory/baseline increases. Validation uses the fresh `target/rust-ownership` directory, not archived artifacts
from the old shared target directory.

Remaining scope includes owned AST/type/symbol/signature/snapshot/cache referents, borrowed graph access instead of
fabricated static references, parser/config host leaks, manual API orchestrator freeing, parse-cache accounting,
unchecked thread traits, address-routing/thread arenas and the recycling/rewind compatibility APIs. The complete
CLI/LSP/API/incremental/emit/oracle, Linux lifecycle/RSS and performance audit is still required.
