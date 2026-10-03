# Review of mfs-cx/emit-builders 535adce (and evidence of the earlier incremental review)

Reviewer branch: `mfs-cx/emit-builders-review` = 535adce + one fix commit. Reference: tsgo built from ts-ref b85298b6a81f
(`go build ./cmd/tsc`, go1.27.1). All fixtures are public synthetic graphs under /tmp (no corpus files).

## Findings

### F1 (high for -b / incremental memory and time): one-file emits spawn every checker thread
- Where: `crates/tsrs_compiler/src/checkerpool.rs` `for_each_checker_group_do` (535adce:604-638) always calls
  `run_work_group(.., state.checkers.len(), ..)`, i.e. spawns one OS thread (512 MB stack reservation, own thread-local
  arena chunk and allocator heap) per checker, even when `files` has one file. `Program::emit`
  (`crates/tsrs_compiler/src/program_emit.rs:106`) reaches it once per affected file, because
  `crates/tsrs_incremental/src/emitfileshandler.rs:142-176` emits the affected files one at a time
  (Go `emitfileshandler.go:124-150` queues them on a WorkGroup; Go `Program.Emit` (program.go:1898) queues one
  goroutine per file, which costs nothing comparable).
- Proof (150-file composite project, `emitDeclarationOnly`, `TSRS_EMIT=1 tsrs -p .`, mimalloc stats): checkers 1/2/4 ->
  threads created 38/346/654, peak RSS 107/286/306 MB; same project non-incremental: 58 threads, 137 MB.
- In `-b` this multiplies with the number of projects, since nothing per thread is returned:
  8 such projects (`files: []` solution, 8 independent references), `-b . --builders 1/4/8`:
  535adce 1692/1732/1751 MB, wall 1.31/1.21/1.20 s; tsgo 192/257/353 MB, wall 0.55/0.34/0.36 s.
  `--checkers 1` or `--singleThreaded` on 535adce: 217 MB / 145 MB.
- Fix (this branch): spawn threads only for checkers that own at least one of `files`; a one-file call runs on the
  calling thread. Checker association and per-checker file order are unchanged, so output is unchanged.
  After: 367/395/419 MB, wall 0.52/0.47/0.46 s at builders 1/4/8. Growth per extra project at builders 4: n1..n4 =
  137/175/251 MB (tsgo 82/121/189 MB), i.e. the same slope as tsgo.
- Not done here (perf only, no output difference): Go emits the affected files of one program in parallel
  (emitfileshandler.go:124); tsrs emits them sequentially.

### F2 (none / informational): programs are never freed in -b
- Go drops each task's program right after building (`orchestrator.go` buildOrCleanProject: `task.result.program = nil`
  unless Testing) and `report` drops `t.result`; memory then returns via GC. 535adce mirrors both (orchestrator.rs
  `build_or_clean_project`, buildtask.rs `report` takes the result), but tsrs values are arena-allocated and
  `free_program` (program.rs:391) is only used by the language server's memory regions, so nothing is reclaimed.
- Measured: with F1 fixed the per-project growth matches tsgo's (above; tsgo also keeps the shared .d.ts/json parse
  cache for the whole build, host.go:52). Not a regression at these sizes; it would show only for builds whose
  projects are individually large compared with the sum Go keeps live. No change proposed.

### F3 (low, panic path only): poisoned locks cascade into secondary panics
- A panic in a checker thread (e.g. a gate stub) poisons task/host mutexes; sibling builders then panic with
  `PoisonError` (seen at buildtask.rs:322) or "build aborted" (buildtask.rs:172). The original panic is printed first,
  no hang at builders 1/4/8 (aborted waiters are released), exit 5. Go would crash on the first panic. No change.

## Scheduler checks with no finding
- `rangeTasks` (Go passes `order`, not `scheduleOrder`, to the builders; so does tsrs), `waitOnUpstream`/`done`,
  `built` + reporting in `Order()`, early program drop, `closeSignal` (no lost wake-up: `abort` stores the flag before
  taking the lock to notify), dts/json parse cache (`parseCache.loadOrStore` faithful), abort/panic release.
- Diamond + independent chain fixture (base -> left,right -> top; solo -> solo2; noEmitOnError; 8 steps: cold, no-op,
  base error, no-op, solo shape change, base fixed, solo restored, no-op), `-b . --verbose --listEmittedFiles`,
  builders 1/4/8, with and without `--stopBuildOnErrors`, 3 repetitions each: all identical to tsgo (output order,
  trees, tsbuildinfo) on 535adce and on this branch.
- Committed fixtures on this branch: run-all.sh (inc1, inc2, b1, b1-outputs) and graph/cycle at builders 1/4/8 identical.

## Gates for the fix (base 535adce built in a worktree)
conformance errors + `--baselines types,symbols` trees identical (default and `TSRS_LAZY_MEMBERS=0`, 13457 pass);
fourslash 4066/63 same pass list; `--baselines js` 1363 pass, same list; `RUSTFLAGS="-D warnings" cargo check
--workspace --locked --all-targets` clean; `tests/emit_gate.rs` 5/5. `TSRS_EMIT` remains opt-in.

## Fixture generator (for reproduction; not committed as a tool)
Each project: `composite`, `emitDeclarationOnly`, strict, 150 files; file i imports T{i-1} and declares an interface
with a mapped type, a key-remapping mapped type, a generic function, a generic class with chained `map` calls and a
few instantiations. Root `tsconfig.json`: `{"files":[],"references":[{"path":"p0"},...]}`.

## Earlier review: incremental da23dd6 + harness 7eb3e04 (no findings)
Local composition 7eb3e04 + emit/transforms f708ab1 (needs ast_ext.rs dropped: duplicates core-2 astutilities.rs),
compared with tsgo via tools/oracle/incremental/steps.sh with `--listEmittedFiles`:
- f1 (14 steps, emitDeclarationOnly + noEmitOnError; error added/fixed, strict/declarationMap/noEmit toggles, export*
  shape edit, global augmentation edit, type-only import, deleted output), f2 (17 steps, CJS JS+d.ts; const enum value,
  body/shape edits, deleted .js, target/declaration/noEmit toggles, syntax error, isolatedModules), f3 (18 steps,
  allowJs/checkJs, global script, foreign-version and corrupt tsbuildinfo, assumeChangesOnlyAffectDirectDependencies,
  lib change, new file, noCheck, outDir/tsBuildInfoFile/emitDeclarationOnly changes), f4 (chained/related diagnostics
  replayed from tsbuildinfo, plain and --pretty), b2 (13 steps -b --verbose incl. deleted upstream .js/.d.ts/
  tsbuildinfo, noEmitOnError upstream), --force/--dry/--clean/--stopBuildOnErrors: identical, also --singleThreaded and
  --checkers 8. The 6 @incremental test variants pass through the incremental harness path.
- Only difference: f3 step 9 (`lib` change removing DOM libs) — tsgo itself is nondeterministic there
  (programtosnapshot.go:162-179 stops at the first removed file in sync.Map order; 10 runs: 6x 11 files emitted, 4x 1),
  tsrs always takes one branch; outputs and tsbuildinfo equal either way.
- Default mode: without TSRS_EMIT an incremental project writes nothing, ignores an existing/corrupt tsbuildinfo, `-b`
  exits 5.
