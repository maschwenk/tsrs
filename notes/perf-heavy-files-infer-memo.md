# perf-heavy-files-infer-memo: the same inference walk made again and again

The check phase of vscode, t3code-server, mui-docs and formbricks-web at 16-32 checkers is bounded by a few files
(the task brief, 2026-10-07). This note is about the one of them whose cost is the algorithm itself, vscode's
`src/vs/platform/agentHost/test/node/mapSessionEvents.test.ts` (0.35 s in a 0.78 s check at 16 checkers on the 64-vCPU
Linux runner), and the memo that removes it. The same memo pays on every project, most on t3code-server (Effect).
notes/perf-heavy-files.md has the other heavy files and why they are not fixed here.

## The hot path

notes/perf-checker-64.md section 5 found that 62% of this file is `inferTypeArguments` for
`fusionTestEvent<K extends SessionEventType>(type: K, data: SessionEventPayload<K>['data'], ...): SessionEventPayload<K>`
with `SessionEventPayload<K> = Extract<SessionEvent, { type: K }>` over a 155-member union. Tracing every `inferTypes`
call of the isolated file (a tsconfig with `"files": [<the file>]`, 831 files, 0.30 s for the file) shows where:

| `inferTypes` target | calls | ms | `inferFromTypes` steps per call |
| --- | --- | --- | --- |
| `SessionEventPayload<K>` from the contextual type `SessionEvent`, priority `ReturnType` | 112 | 149 | 99,829 |
| the same, priority `None` (the return mapper's context, `inferTypeArguments` in checker.go) | 112 | 132 | 99,829 |
| `SessionEventPayload<K>['data']` (the `data` argument) | 146 | 6 | 143 |
| everything else | 4,297 | 11 | |

Each `event(...)` call sits in an array literal whose contextual element type is `SessionEvent`, so inference first
infers from `SessionEvent` (155 object types) to the return type (155 deferred conditionals
`M_i extends { type: K } ? M_i : never`): `inferToMultipleTypes` tries every source member against every target member,
and each pair goes through `inferToConditionalType` into the true branch (`M_i & { type: K }`, a substitution type),
which is where `K` gets its candidates. 155 x 155 x ~4 = 99,829 steps, 1.3 ms, twice per call, 224 times, with the same
source, the same target and fresh inference infos every time, ending in the same 141 candidates every time. That is
280 of the file's 298 ms of inference. tsgo does the same (it takes 1.6x longer on this file than tsrs).

## The memo

`crates/tsrs_checker/src/infermemo.rs` (module comment) maps the inputs of the walk at the top of `inferTypes` to what
it wrote.

- **Inputs:** a walk starts from a fresh `InferenceState` (empty `visited`, no propagation type, not bivariant), so
  they are the source, the target, the priority, the contravariance, and what the walk reads from each inference info:
  type parameter, candidate and contravariant-candidate lists, priority, `topLevel`, `isFixed`, implied arity (at most
  8 infos).
- **Outcome:** each info's two lists, priority and `topLevel`, and whether the walk cleared the infos' cached inferred
  types (it never reads them).
- **Stored only when** the walk took at least 16 `inferFromTypes` steps, an earlier walk to the same target did too,
  and the walk
  - created no type, symbol or signature, instantiated nothing and left the instantiation counters where they were
    (`type_count + symbol_count + signature_count`, `instantiation_count`, `total_instantiation_count`);
  - added no diagnostic (a new `diagnostic_adds` counter, duplicates included) and took no impure union reduction
    (`union_front_cache.impure`);
  - read no transient state: the flow memo's taint frame around it (`flow_frame_begin` / a new
    `FlowMemo::end_transparent` that leaves the parent's registers as if the frame had not been begun) saw no
    in-progress or circular resolution older than the walk, no `resolvingSignature` and no `flowTypeCache` entry;
  - ran with no language-service inference blocking (`skipDirectInferenceNodes` empty).
- **Why a hit is exact:** such a walk read only finished values (caches that only grow, types that never change)
  besides its inputs, so the same inputs walked again take the same path and write the same outcome. A mapper that
  reads the infos' inferred types runs only inside an instantiation, so a walk that consulted one is never stored. A
  hit writes the outcome and replays what the walk reported to the enclosing flow memo frame (height and flags). Types
  are never freed while their checker lives, so a handle in a key or an outcome never names another type.
- **Cost control:** the first long walk to a target only marks the target; only walks to marked targets are keyed,
  looked up and measured, so the many short walks pay one hash-set lookup. Without the marker the memo cost +0.1-0.2%
  instructions on projects where it never hits (xstate, webpack, mui-docs in the first probe); with it, -0.04% to
  -0.4% there.
- **Modes:** on by default, off under `--checkerAssignment go` (like the union front cache). `TSRS_INFER_MEMO=shadow`
  walks every hit again from the same starting state and panics unless the walk has no effects and ends in the stored
  outcome, height and flags; `=0` / `=1`; `TSRS_INFER_MEMO_STATS=1`; `TSRS_INFER_MEMO_MIN_STEPS` (docs/DEBUGGING.md).

The first version memoized only walks that found no inference info (a no-op memo). It hit 74 times on the file and
saved nothing: the 99,829-step walk does find `K`.

## Results

Instructions: 64-vCPU Linux runner (`depot ci run --workflow .depot/workflows/perf-probe.yml` with an A/B
`tools/perf/probe.sh` that builds origin/main in the same job and runs both binaries), `bench/count.py`, one checker
(`--singleThreaded`, `RAYON_NUM_THREADS=1`), user-space instructions, deterministic. Base 249561e (main moved on to
7262f61 with front-end changes only before this was opened).

| project | base G | memo G | delta | threshold 8 | threshold 32 | peak RSS MiB base / memo |
| --- | --- | --- | --- | --- | --- | --- |
| vscode | 115.52 | 107.26 | -7.15% | -7.44% | -6.95% | 1905 / 1905 |
| t3code-server | 58.32 | 51.39 | -11.89% | -11.95% | -11.58% | 829 / 833 |
| supabase-studio | 65.37 | 64.09 | -1.95% | -2.00% | -1.87% | 945 / 946 |
| Compiler | 2.42 | 2.37 | -2.08% | -2.08% | -2.08% | 57 / 57 |
| cal-diy | 42.00 | 41.35 | -1.55% | -1.72% | -1.52% | 822 / 822 |
| formbricks-web | 55.46 | 54.68 | -1.42% | -1.47% | -1.25% | 1359 / 1361 |
| Compiler-Unions | 5.52 | 5.44 | -1.39% | -1.39% | -1.39% | 61 / 61 |
| xstate-main | 7.56 | 7.53 | -0.37% | -0.40% | -0.04% | 192 / 192 |
| webpack | 14.22 | 14.18 | -0.29% | -0.31% | -0.07% | 310 / 310 |
| mui-docs | 51.33 | 51.31 | -0.04% | -0.05% | -0.05% | 746 / 746 |

Threshold 64 (the first version): vscode -6.86%, t3code -7.44%. 8 is a little better than 16 but costs t3code 13 MiB.

Check time on the same runner, mean of 10 runs (two probes x 5, base and memo alternating), seconds:

| project | 16 checkers base / memo | 32 checkers base / memo |
| --- | --- | --- |
| t3code-server | 1.686 / 1.607 (-4.7%) | 1.539 / 1.440 (-6.4%) |
| vscode | 0.786 / 0.768 (-2.3%) | 0.504 / 0.482 (-4.4%) |
| cal-diy | 0.631 / 0.621 (-1.6%) | 0.503 / 0.497 (-1.2%) |
| formbricks-web | 0.622 / 0.616 (-1.0%) | 0.569 / 0.562 (-1.2%) |
| supabase-studio | 0.682 / 0.678 (-0.6%) | 0.493 / 0.489 (-0.8%) |
| mui-docs | 0.827 / 0.841 (+1.7%) | 0.749 / 0.760 (+1.5%) |
| webpack, xstate-main, Compiler, Compiler-Unions | within ±3% (0.07-0.14 s checks) | within ±3% |

mui-docs and xstate-main are within the runner's run-to-run spread (the first probe had mui-docs at -1.3% at 32
checkers); their instructions do not change.

Heavy files (per-file CPU, `TSRS_FILE_TIMES`, mean over the runs where the file was in the top three):

| file | 16 checkers base / memo | 32 checkers base / memo |
| --- | --- | --- |
| vscode `mapSessionEvents.test.ts` | 0.346 / below 0.10 (out of the top three) | 0.370 / below 0.10 |
| vscode `copilotAgentSession.test.ts` (now the heaviest) | 0.149 / 0.137 | 0.168 / 0.152 |
| t3code `server.ts` | 1.147 / 1.115 | 1.266 / 1.194 |
| t3code `binCli.ts` | 1.028 / 1.003 | 1.173 / 1.134 |

On the isolated vscode program (Mac, one checker) `mapSessionEvents.test.ts` goes from 0.30 s to 0.04 s; at 16 and 32
checkers vscode's check is no longer bound by one file (the next heaviest is 0.14 s).

Stats (`TSRS_INFER_MEMO_STATS=1`, one checker, threshold 32 build): vscode 947 hits skipping 21.4M `inferFromTypes`
steps; t3code-server 12,778 hits skipping 3.0M steps (63,834 at 16 checkers).

## Gates

- Output identical to origin/main (`--pretty false`, diagnostics and exit code) on the ten bench projects at 1, 16 and
  32 checkers on the Linux runner, and at 1, 4 and 16 checkers on the Mac in four configurations: default,
  `TSRS_INFER_MEMO=shadow` with threshold 8, threshold 1 (every walk of a marked target memoized), and the default of
  the final build. No shadow panic.
- Single-threaded `Types`, `Instantiations` and `Symbols` counters identical on all ten projects (the memo never
  changes what is created).
- Conformance suite (`tsrs-test run --suite all --baselines types,symbols`), Go history mode and
  `TS_TEST_PROGRAM_SINGLE_THREADED=false TSRS_HISTORY=canonical`, memo forced on (`TSRS_INFER_MEMO=1` / default) and in
  shadow mode with threshold 1: pass lists identical to origin/main's (13,458 pass, 2 codes, 2 fail; 12,779 /
  12,779 `.types` / `.symbols`; canonical: the by-design `objectLiteralNormalization` difference only), no crash.
- `testdata/regressions/infer-memo-repeated-walks`: the vscode shape at 24 members with 8 errors, tsgo-ref's output;
  identical with the memo off, on and in shadow mode (125 hits at the defaults).
- `cargo check --workspace`, `tools/lint/ratchet.py` (one `#[expect(clippy::set_contains_or_insert)]`, the target is
  marked after the walk), `tools/lint/source.py`, `cargo test -p tsrs_checker`, `tools/regressions.sh`.
