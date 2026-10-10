# Headless lint through project discovery (2026-10-11)

Headless now opens the requested paths through `SnapshotHost`, selects their source files from the discovered
programs, and asks those programs to check them. Delete the separate `TsConfigResolver` and the linter's duplicate
configured/inferred program constructors. The existing project system handles ancestor configs, solution references,
imported files and inferred projects; rule callbacks still run inside the checker.

The batch-open entry point calls the same discovery helper as editor document opens, then retains/cleans projects
once for the batch. The first draft called the complete single-file open operation repeatedly; ecosystem runs
exposed the repeated scans of every open file. The final version shares the open helper with the editor path and
accumulates its existing retention sets. No new config-search algorithm, cache or worker is introduced.

The result holds the snapshot until callers finish consuming diagnostics, keeping their source-file pointers alive.
Headless uses one diagnostics checker per discovered program. Native `--lint` continues to use the compiler's
checker groups. This is an architectural simplification, not a performance optimization claim. Reviewed the earlier
lint measurement notes, `perf-round2-followups.md`, `docs/RUST.md` Techniques and the memory-round summary first.

## Oxlint comparison

Compare the isolated source at `66a941734c922eea4660cf37979051c8287d475d` plus this refactor, built with
`cargo build --locked --release -p tsrs_cli`, against the installed `oxlint-tsgolint` 7.0.2003 native binary.
The latter identifies its Go module as `v0.25.1-0.20260924142225-eb9339115edd+dirty` and uses Go 1.26.8. It is the
packaged backend, not a compiler oracle at tsrs's pinned TypeScript commit. Oxlint is 1.87.0 on both sides.
Unrelated working-tree changes to numbers, identifiers and the scanner are excluded from the tsrs build.

Use each of the 100 git checkouts under `oxc-ecosystem-ci/repos` as the working directory, with this config:

```json
{"plugins":["typescript"],"categories":{"correctness":"off"},"rules":{"typescript/no-floating-promises":"error"}}
```

```sh
OXLINT_TSGOLINT_PATH=/absolute/path/to/backend \
  oxlint --type-aware --disable-nested-config --threads 2 \
  -c /absolute/path/to/comparison.json --format json .
```

Only the backend path changes. This measures the one implemented rule, leaves repository ignores in force, and
avoids repository-specific rules or plugins. Backend thread settings remain at their defaults; `--threads 2`
controls the Oxlint frontend. There is a 60-second process-group timeout per corpus run. Compare full rule
messages, help, filenames and labeled spans as multisets, preserving duplicate counts and ignoring scheduling order.
The checkouts are used as found; dependencies are not installed for this experiment.

tsgolint and tsrs each complete 96 checkouts. The 96 common completed runs select 543,756 files and report
6,689 versus 6,619 rule diagnostics, respectively. **86/96 match exactly for the rule; ten differ.**
Of the matching repositories, 65 have nonzero findings and 21 have none. The mismatches are:

| Repository | tsgolint findings | tsrs findings |
| --- | ---: | ---: |
| DefinitelyTyped | 4174 | 4183 |
| cal | 34 | 22 |
| eui | 6 | 5 |
| glide | 39 | 34 |
| insomnia | 18 | 12 |
| jitsi-meet | 91 | 59 |
| react-router | 22 | 2 |
| sentry-javascript | 293 | 302 |
| twenty | 17 | 4 |
| wp-calypso | 118 | 119 |

The comparison does not establish full backend equivalence. Only 12 of the 96 complete diagnostic multisets
(including config diagnostics) match; config help text also differs between the implementations.

### Where the differences come from

A temporary instrumented build records each requested file's discovered project, the reasons a project is skipped,
and time spent loading, selecting and checking. It is kept only in the ignored artifact directory; neither the
instrumentation nor the experimental source/config overrides is in the product or the ecosystem checkouts.

**84 missing findings are config selection plus the existing skip-on-config-error policy.** The packaged resolver
matches root-file lists. Shared discovery tests actual program membership, so imported files can belong to a
configured project even when absent from its root list. `run_linter` skips that project's selected files when parsing
or program diagnostics are present. The trace accounts for every missing finding in these six repositories:

| Repository | Missing findings | Selected config and reason it is skipped |
| --- | ---: | --- |
| cal | 12 | `packages/app-store/tsconfig.json`: missing `@calcom/tsconfig/react-library.json` |
| eui | 1 | `packages/eui/.storybook/tsconfig.json`: removed `baseUrl` option |
| insomnia | 6 | `packages/insomnia/tsconfig.json`: removed `baseUrl` option |
| jitsi-meet | 32 | `tests/tsconfig.json`: missing Mocha, WebdriverIO and Node type packages |
| react-router | 20 | `packages/react-router/tsconfig.json`: missing Node types |
| twenty | 13 | Five app `tsconfig.spec.json` projects: missing Node and/or Vitest types |

For example, cal's `packages/app-store/jelly/api/callback.ts:61` is imported into the app-store program; the
packaged resolver lints it with inferred settings. Jitsi production files are imported by the test project, and
Twenty's affected files belong to call-recorder, postcard, hello-world, fireflies and document-generator test
projects. These are observable consequences of reusing discovery, not missing rule callbacks. The responsible
selection is `find_or_create_default_configured_project_worker` in `projectcollectionbuilder.rs`; the config-error
skip is in `tsrs_linter::run_linter`.

**Program membership also explains ten extra findings.** In DefinitelyTyped, `types/webvis/tsconfig.json` lists
only `index.d.ts` and `webvis-tests.ts`; shared discovery includes the imported `test/*.ts` files and produces nine
findings. Replaying just this package gives nine versus zero. In wp-calypso,
`packages/calypso-apps-builder/tsconfig.json` explicitly lists `index.js`, but its missing base config leaves
`allowJs` disabled. Discovery cannot find the JS file in that program and uses an inferred project, adding the
finding at line 81. The packaged resolver selects the broken config and skips the file. Replacing that config
in memory with a valid `allowJs` config makes the full headless rule outputs match (123 findings each).

**Inferred globals explain Sentry and the Node part of DefinitelyTyped.** Sentry has unrelated script fixtures
sharing the inferred program. The nine differing `run()` calls resolve to an async declaration in
`public-api/startSpan/basic/subject.js`. Adding `export {}` to those nine fixtures in memory makes both full
headless outputs match (294 rule findings). DefinitelyTyped combines current, v25, v24 and v22 Node declarations;
conflicting ambient declarations produce order-dependent overloads and promise labels. Three identical-input
runs of the packaged backend produce 1,463, 1,474 and 1,464 Node findings. Restricting both to the current Node
version gives 383 identical findings. These differences were also reproduced before this discovery refactor.
Direct headless counts in these reductions precede Oxlint's suppression of diagnostics from ignore comments.

**Glide's five misses come from file order in the shared overlay filesystem.** Both paths load the same 217
source files, but in different orders. `OverlayFS::get_accessible_entries` removes existing directory children and
appends the opened children in hash-map order; `merge_cached_directory_entries` also ranges a map. Glob expansion
preserves the supplied order, so opening the requested files changes the configured program's root/source order.
Glide has competing global `ChromeUtils` declarations in `@types/gecko.d.mts` and generated Gecko declarations;
the changed order loses the specialized `importESModule` return types. Four missing findings are `DOM.scroll` calls
at lines 299, 303, 307 and 311 of `GlideHandlerChild.sys.mts`; the fifth is `Autocmds.invoke` at `browser.mts:224`.
Native `tsrs --lint` produces all 39 findings in both default and single-threaded mode. Giving both headless
backends the same explicitly sorted root list through an in-memory config override produces 39 identical findings.
Replacing just the two namespace loaders with direct imports also produces 39 identical findings. Reporting
semantic diagnostics first does not fix the original result. The directory-order behavior lives in
`tsrs_project/src/overlayfs.rs` and `snapshotfs.rs`; the linter's path filtering is not dropping the statements.
This ordering issue remains open.

**Rolldown is a shared config-registry deadlock.** The single file
`crates/rolldown/tests/rolldown/errors/tsconfig_error/circular_extend/main.ts` reproduces it: its tsconfig contains
`{"extends":"./tsconfig.json"}`. Native compilation returns TS18000; headless blocks, and the packaged Go backend
reports a deadlock. `ConfigFileRegistryBuilder::reload_if_needed` runs inside an entry mutation while the entry's
mutex is held, then `update_extending_configs` calls `change_if` on that same config entry. A sample of the reduced
Rust process shows its headless thread waiting on a mutex. The problematic code is in the existing shared registry
(`configfileregistrybuilder.rs` and `tsrs_projectutil/src/dirty/syncmap.rs`), not a reason to restore a lint resolver.
This bug remains open.

- rolldown: tsrs exceeds 60 seconds; the packaged backend exits with a Go deadlock in `TsConfigResolver`.
- azure-devops-ui-public: Oxlint selects no files on either run.
- bun: the Oxlint command terminates abnormally on both runs.
- next: both outputs report an invalid-UTF-8 fixture; these runs are excluded from the completed comparison.

Only Outline has a root `node_modules` directory in this corpus; its existence does not verify a complete dependency
installation. Missing dependencies and obsolete config options limit how much code is actually checked, so these
numbers describe the local checkouts, not dependency-complete upstream projects.

## Repeated measurements

Apple M3 Max, 36 GiB RAM, Rust 1.99.0 release profile. After compilation and the corpus sweep finish, run one
warm-up pair and three interleaved measured pairs per project, alternating backend order. Values are medians.
The outer timer measures end-to-end Oxlint wall time. A transparent backend wrapper selected through
`OXLINT_TSGOLINT_PATH` runs `/usr/bin/time -l -o "$metrics" /absolute/path/to/backend "$@"`, preserving protocol
stdout while recording the backend's own retired instructions and peak RSS. Outer-process instruction counters
do not include the backend's work and are not used here. No `RAYON_NUM_THREADS`, `GOMAXPROCS` or `GOMEMLIMIT` override
is set. Columns give **tsgolint → tsrs**.

| Repository | Oxlint wall (s) | Backend instructions (G) | Backend peak RSS (MiB) |
| --- | ---: | ---: | ---: |
| App | 3.19 → 4.99 | 77.18 → 88.00 | 2136.8 → 1840.4 |
| excalidraw | 0.39 → 0.61 | 13.48 → 7.87 | 352.5 → 209.4 |
| outline | 0.61 → 0.95 | 25.31 → 12.39 | 485.3 → 260.7 |
| vscode | 2.64 → 2.83 | 82.12 → 47.48 | 2873.4 → 1713.4 |

The four samples use less backend memory with tsrs (14–46%), but end-to-end time is 7–56% longer. Instruction
counts fall on three samples and rise on App. tsrs always performs semantic checking and uses one diagnostics
checker per project. The commands exercise each backend's own checking and scheduling behavior; these figures
are local backend tradeoffs, with the dependency/config limitations above. All 32 repeated runs preserve the
corpus rule-diagnostic multisets.

A separate diagnostic build times the stages within headless for three warm replays of the captured inputs.
These are medians in seconds, not another end-to-end comparison against Go:

| Repository | Load discovered programs | Select requested paths | Check selected files |
| --- | ---: | ---: | ---: |
| App | 1.48 | 0.0045 | 2.43 |
| excalidraw | 0.12 | 0.0003 | 0.44 |
| outline | 0.15 | 0.0010 | 0.75 |
| vscode | 1.19 | 0.0068 | 1.06 |

Selection itself costs less than 8 ms in every replay. Loading accounts for 38% of App and 52% of VS Code
headless time, and checking dominates Excalidraw and Outline. App checks 61 selected inferred files in a 1,591-file
program; the other selected projects have config errors. The current headless `run_on_program` acquires one
`CheckerLifetime::Diagnostics` checker and checks selected files in a serial loop. The packaged Go backend uses
its worker scheduler and requests full semantic diagnostics only when asked to report them. Thus the implementations
do different checking work with different parallelism. These observations locate the time spent, but do not establish
how much a scheduling change would save; no new scheduler or performance workaround is included here.

The checkout revisions are App `a1f71ac453d1`, excalidraw `abeeaeba217a`, outline `05db32f8c1255`, and vscode
`ef9a35223cbb`. The current-base corpus run also completes DefinitelyTyped in 38.95 s with a command peak RSS of
10,311 MiB; that single run is separate from the repeated measurements.

Artifacts, raw JSON diagnostics, repository revisions, commands and the comparison driver are in the ignored
`target/scratch/discovery-review/` directory (`current-discovery`, `current-comparison.json` and
`current-measurements`, with reductions and traces in `diagnose`; older-base experiments are kept separately).
The corpus pass is not used for speed claims; only the repeated runs above are timed comparisons.

## Validation

The isolated source passes 21 local lint checks, all 188 runnable upstream rule cases and 112 original snapshots
(plus the upstream-revision metadata check), nine CLI integrations and 104 project checks. The existing upstream
skip remains. Workspace compilation including test targets, lint ratchet and source checks pass.
The full CLI suite also passes on this compiler base.
