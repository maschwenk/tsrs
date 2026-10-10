# Rust-owned build roots and scoped config hosts (2026-10-10)

This continues the full ownership goal from `c78be4a3`. It is an unfinished branch checkpoint, not a completed
graph migration or a performance change ready to land. Earlier ownership evidence, `perf-round2-followups.md`,
`mem-round4.md` section 6 and `docs/RUST.md` Techniques were reviewed. The constraint is the user's requirement
to remove the custom memory model. This changes ownership boundaries, not checker sharing, config resolution
algorithms, graph traversal order or semantic IDs.

## Config host borrows

Config/command parsing takes a short host borrow, including nested extended-config cache calls. The synchronous
package-config resolver can borrow its host. Persistent compiler/project/API resolvers explicitly retain owned
host data through `DefaultResolver<'static>` / `ResolverOptions<'static>`; that object bound is not a fabricated
static reference. The project cache passes parsing and expiration callbacks for the load call rather than storing
callbacks whose argument type demanded static host/cache fields. Entry locking, expiration and owner-set behavior
remain the same. The unsafe `assume_static` adapter disappears.

Session config hosts are owned fields. The compiler borrows itself as the config host instead of leaking a
filesystem/directory wrapper on every project-reference parse. CLI and harness config helpers use scoped values,
and harness config filesystems now drop after parsing. Config records, source files and cached entries themselves
still use legacy graph allocations. The test utility's static VFS/config hosts also remain legacy.

A lifecycle case resolves a package's `tsconfig` field and nested `extends`, exercises repeated cache hits, drops
the scoped host/filesystem, and then consumes the returned config. A weak filesystem owner confirms that parsing
and the extended cache did not retain it. It verifies host lifetime independence, not complete config graph
ownership.

## Build/result ownership

Build orchestrators use `Arc`; their hosts are shared Rust owners with a weak back-reference to the orchestrator.
Compiler and incremental host adapters retain the host. Scoped worker/task operations borrow their roots instead
of requiring static references. Manual API orchestrator freeing, leaked orchestrator/host boxes and raw box
reconstruction are removed. On unwind, ordinary root destruction releases unreported program roots before legacy
task regions, breaking the temporary task-result/program/region retention cycle.

The broader CLI API build check exposed a diagnostic lifetime bug: non-testing tasks released their program
before the API serialized its checker diagnostics. The focused output-equality case failed with corrupted text
(a JSON UTF-8 boundary panic); the broader run also crashed. Config borrowing alone did not fix it. Tasks with
errors now keep their program through reporting, and the orchestrator retains those programs. Returned
`OrchestratorResult` / `BuildOutcome` values keep the actual orchestrator alive until their diagnostics are
consumed. The API backend's heterogeneous result owner uses ordinary `Arc` type erasure, with no pointer recovery
or custom thread traits.

The existing API/CLI output-equality case now passes. A new lifecycle case returns an error outcome, rebuilds with
different source text, disposes the backend, checks the old diagnostic text/source, then drops the outcome and
observes final orchestrator destruction through a weak owner. This establishes that boundary's retention and
release. Raw diagnostic handles can still be extracted without their owner; borrowed/owned diagnostic graph APIs
remain part of the full goal.

Build tasks still use `P<BuildTask>` and legacy regions. API build systems are still leaked, command roots still
use `P`, and testing hooks still have static interfaces. Host operations that need the orchestrator upgrade its
weak back-reference; an independently retained testing program does not acquire the outer build root. The full
tsctests/testing-hook audit remains required. These remaining boundaries must migrate before claiming the build
subsystem's memory model is completely Rust-owned.

## Measurements

Apple M3 Max / macOS / Rust 1.99.0, fat-LTO release builds without PGO and the Xcode 14.5 SDK. Compiler workloads
are pinned at `41f652ab2df5077b1115f73eaeefc2fe9f674132`. Five interleaved rounds per binary/project/mode follow
warm-up with no concurrent validation jobs. `/usr/bin/time -l` reports retired instructions and peak RSS. Exit
codes and stdout hashes agree in all cells. The 80 samples and verified source/binary hashes are in
`rust-owned-build-roots-results.json`; comparisons include original Oxc `3dac06d8`, symbol storage `c78be4a3`,
this stage and pre-Oxc `18175c0e`.

| Project/mode | Instructions, `c78be4a3` → this stage | Peak RSS, `c78be4a3` → this stage |
| --- | --- | --- |
| Compiler, single | 2.337582 → 2.338763 G (+0.05%) | 115.03 → 115.06 MiB (+0.03%) |
| Compiler, default | 2.958779 → 2.960448 G (+0.06%) | 163.09 → 160.27 MiB (-1.73%) |
| Compiler-Unions, single | 4.859437 → 4.868668 G (+0.19%) | 117.00 → 116.98 MiB (-0.01%) |
| Compiler-Unions, default | 6.176321 → 6.175095 G (-0.02%) | 162.83 → 162.72 MiB (-0.07%) |

This does not clear the performance-change threshold. Cumulative single-thread instructions remain +10.64% /
+5.49% against original Oxc. Default RSS remains +104.94% / +98.55% against pre-Oxc, and +11.77% / +13.60% against
original Oxc. The full instruction/memory gates fail; no overall improvement is claimed. These no-emit Compiler
measurements do not establish API rebuild memory bounds or quantify retaining erroneous build programs. Linux
RSS/lifecycle measurements and larger workloads remain required after the graph/address-routing migration.

## Verification

Final API 77, compiler 20, execute 2, language service 67, project 105 (two existing ignored), module resolver 23
(one existing ignored), tsoptions 29 (one existing ignored), and native LSP 41 cases pass. CLI API 3, CLI default
emit/build 6 and the separate API/CLI output-equality case pass, as do two compiler compile-fail doctests and all
28 regression fixtures. The tsctests entry has no scenario dump; it does not establish tsc/tsbuild baseline parity.
The glibc RSS cases are not run on macOS: their `malloc_trim` / `/proc` assertions require Linux. A broad initial
`api::` filter included those platform-specific cases; the native-supported checks were subsequently run separately.

Full six-baseline classifications over 15,197 pinned TypeScript variants exactly match both original Oxc and
`c78be4a3`: diagnostics 13,462; types/symbols 12,779 each; JS 13,392; JS maps 149; sourcemap text 156, with no
crashes/timeouts. Default mode (`TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`) also has identical
classification sets. Fourslash has the same 4,066 pass / 63 fail / 417 skip sets over 4,546 cases with native
watcher access. The README capability table adds the build diagnostic ownership evidence; cited conformance and
upstream-client counts are unchanged, and the generated benchmark section is untouched.

Workspace checks, release builds, all CLI features and wasm32-wasip1 pass. The ratchet has three baseline findings
and none new; source checks pass with 24 reviewed custom thread implementations and no inventory/baseline changes.
Validation uses the fresh `target/rust-ownership` directory. Census runtime, full emit/incremental oracles,
tsctests scenario baselines and Linux/glibc RSS checks remain unaudited by this stage.

Remaining scope includes build task/system/command owners, owned AST/type/symbol/signature/inference/config/
snapshot/cache referents, borrowed graph access instead of fabricated static references, parse-cache accounting,
unchecked thread traits, address-routing/thread arenas and recycling/rewind compatibility APIs. Keep the complete
ownership goal active until those and the full runtime/oracle/lifecycle/performance audit are actually complete.
