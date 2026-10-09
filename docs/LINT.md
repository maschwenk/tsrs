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
