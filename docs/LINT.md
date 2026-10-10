# Oxlint type-aware linting

`tsrs headless` is a drop-in backend for the tsgolint subprocess protocol used by Oxlint. Point Oxlint at a native
tsrs binary; no Oxlint changes are needed:

```sh
cargo build --release -p tsrs_cli
OXLINT_TSGOLINT_PATH="$PWD/target/release/tsrs" oxlint --type-aware
```

The backend currently implements `typescript/no-floating-promises`. Rule options, suggestions (`--fix-suggestions`),
source overlays, per-file tsconfig discovery, project references, TypeScript diagnostics (`oxlint --type-check`) and
debug timing frames are supported. Headless always type-checks each requested file. `report_syntactic` and
`report_semantic` only select which TypeScript diagnostics are returned; neither disables checking, including when
the rule list is empty. `noCheck`, `@ts-nocheck` and `skipLibCheck` still suppress their usual TypeScript reports,
but do not bypass the check needed by linting.

Both entry points parse the same headless config and prepare rule options once, before creating the program.
The checker checks an expression statement, then dispatches its configured native rules immediately. Rules live
in `tsrs_checker::lint`; `tsrs_linter` selects requested files from the project system and requests checks. There
is no binder candidate list, compiler-host wrapper, file-completion callback, or separate lint pass.

Headless uses the existing project discovery and its loaded programs, including configured projects, solution
references and inferred projects. Requested paths select the files to check within those programs; imports and
other dependencies remain available for type information. Imported files use their discovered project's options
even when they are absent from the config's root-file list. Repeated paths and symlink aliases resolving to the
same source file in a program are checked once. Headless uses the project pool's diagnostics checker for each
program; the native `--lint` flag continues to use the compiler's checker pool.

Project membership can differ from tsgolint's root-file-based config selection: an imported file may now belong
to a configured project instead of an inferred one. Its compiler options and config errors then apply to linting.
The shared project system currently has a deadlock on self-extending configs and a directory-order difference
that affects competing global declarations. See [the ecosystem comparison](../notes/lint-project-discovery.md)
for reductions, observed differences and measurements.

A node-link flag prevents repeated dispatch when type inference and deferred checking revisit a statement. Each
checker buffers its diagnostics; the compiler collects only the files assigned to that checker. Configured
declaration files are checked whole rather than split across checkers. Diagnostics are
sorted into source order for output. TypeScript deliberately skips `with` bodies, so lint visits that skipped
subtree explicitly; ordinary code needs no additional AST traversal. Timing `calls` counts node dispatches.

Oxlint may send other type-aware rules in the same request; tsrs silently skips those until they are ported.

The wire interface is payload version 2: JSON on stdin and length-prefixed JSON frames on stdout. `headless` is an
integration interface rather than a general command-line UI. Payload version 1 and tsgolint's Go profiling flags are
not implemented.

The rule implementation follows tsgolint at commit `6d20f350733b76ce3cf4a6c52ece675412fefca5`, while all type
questions use tsrs's pinned TypeScript 7 implementation. This can differ from a tsgolint build using another
TypeScript commit.

## Native compiler flag

The normal compiler accepts the same v2 JSON configuration through `--lint`:

```sh
tsrs -p tsconfig.json --noEmit --lint lint.json
```

```json
{
  "version": 2,
  "configs": [{
    "file_paths": ["src/index.ts"],
    "rules": [{"name": "no-floating-promises", "options": {"ignoreVoid": true}}]
  }]
}
```

Paths are relative to the working directory. The compiler's input files still come from its usual command line
or tsconfig; only configured files in that program are linted. Rule options and `source_overrides` use the headless
format. The normal compiler reports its usual diagnostics, independently of the payload's reporting switches,
and adds text lint errors with a failing exit status. Suggestions remain available through headless. Lint errors
do not change TypeScript's emit decisions; use `--noEmit` for checking only.

This flag requires a fresh compilation: build mode and incremental compilation are rejected so a cached build
cannot silently skip linting. For an incremental project, use `--incremental false` (and disable `composite` if
set). Headless already creates fresh programs and needs no additional flag.

## Rule tests

```sh
cargo test -p tsrs_linter
# Run one imported upstream case:
cargo test -p tsrs_linter --test no_floating_promises test_no_floating_promises_rule_invalid_0 -- --exact
```

`crates/tsrs_linter/tests/` imports the complete `no_floating_promises_test.go` suite from tsgolint commit
`24b18b48c47b7ae0d84e21a0e974575026e27908`: 77 valid and 112 invalid cases, including the cases constructed by Go
loops. The original `node:test` case remains ignored, as upstream marks it `Skip: true`; 188 cases run and pass.
The crate also has 21 local checks, including mandatory semantic checking, once-per-node dispatch, deferred and
skipped subtree coverage, per-file options across imported files, pending output in forked checkers, inferred-project lookups through
symlinked package aliases, and project discovery across ancestors and references. Nine CLI integration checks cover
the protocol and native flag. Depot CI runs both suites in its fast-crates job.

The Rust rule tester uses the upstream fixture files and tsconfig settings, and checks diagnostic count, order,
message IDs, the specified UTF-16 line/column positions, suggestion count/order/IDs and exact edited output. It
also checks every invalid case against the 112 original snapshots (message text, help, labeled ranges and
suggestion descriptions), and applies autofixes for up to ten passes, as upstream does. Missing snapshots fail;
Rust output never regenerates the expectations.

The data, snapshots and MIT license are checked in under `tests/fixtures/tsgolint/`. Running the suite requires
neither Go nor a tsgolint checkout. The suite covers the implemented rule; suites for other tsgolint rules are not
yet ported.
