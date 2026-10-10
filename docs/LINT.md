# Oxlint type-aware linting

`tsrs headless` is a drop-in backend for the tsgolint subprocess protocol used by Oxlint. Point Oxlint at a native
tsrs binary; no Oxlint changes are needed:

```sh
cargo build --release -p tsrs_cli
OXLINT_TSGOLINT_PATH="$PWD/target/release/tsrs" oxlint --type-aware
```

The backend currently implements `typescript/no-floating-promises`. Rule options, suggestions (`--fix-suggestions`),
source overlays, per-file tsconfig discovery, project references, TypeScript diagnostics (`oxlint --type-check`) and
debug timing frames are supported. Oxlint may send other type-aware rules in the same request; tsrs silently skips
those until they are ported.

The wire interface is payload version 2: JSON on stdin and length-prefixed JSON frames on stdout. `headless` is an
integration interface rather than a general command-line UI. Payload version 1 and tsgolint's Go profiling flags are
not implemented.

The rule implementation follows tsgolint at commit `6d20f350733b76ce3cf4a6c52ece675412fefca5`, while all type
questions use tsrs's pinned TypeScript 7 implementation. This can differ from a tsgolint build using another
TypeScript commit.

## Rule tests

```sh
cargo test -p tsrs_linter
# Run one imported upstream case:
cargo test -p tsrs_linter --test no_floating_promises test_no_floating_promises_rule_invalid_0 -- --exact
```

`crates/tsrs_linter/tests/` imports the complete `no_floating_promises_test.go` suite from tsgolint commit
`24b18b48c47b7ae0d84e21a0e974575026e27908`: 77 valid and 112 invalid cases, including the cases constructed by Go
loops. The original `node:test` case remains ignored, as upstream marks it `Skip: true`; 188 cases run and pass.
The crate also has 12 local regression tests. Depot CI runs both suites in its fast-crates job.

The Rust rule tester uses the upstream fixture files and tsconfig settings, and checks diagnostic count, order,
message IDs, the specified UTF-16 line/column positions, suggestion count/order/IDs and exact edited output. It
also checks every invalid case against the 112 original snapshots (message text, help, labeled ranges and
suggestion descriptions), and applies autofixes for up to ten passes, as upstream does. Missing snapshots fail;
Rust output never regenerates the expectations.

The data, snapshots and MIT license are checked in under `tests/fixtures/tsgolint/`. Running the suite requires
neither Go nor a tsgolint checkout. To regenerate it from the pinned checkout:

```sh
python3 crates/tsrs_linter/tests/import_tsgolint.py /path/to/tsgolint
```

Regeneration needs Go 1.18 or newer and rustfmt. The importer executes the upstream Go table constructors with a
recording tester, preserving code, options, auxiliary files and computed suggestion outputs. It checks the
upstream revision and snapshot coverage, then generates a separate Rust test for each upstream case. This
imports the suite for the implemented rule; suites for other tsgolint rules are not yet ported.
