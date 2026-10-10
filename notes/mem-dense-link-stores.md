# Dense link stores: TypeScript PR 64711

Port of [microsoft/TypeScript#64711](https://github.com/microsoft/TypeScript/pull/64711), head
`a6c67fa7c079d98b3b42bc1b57962f2c9318a240`, on tsrs `968e5a12`. The pinned TypeScript reference remains
`b85298b6a81f772d080b0455de0ca9d744cd6fd6`; this ports the four-file change independently of a reference upgrade.

## Storage and IDs

Each node/symbol link store now has an ID generator that reserves 256 consecutive IDs from a separate global
atomic counter. Public IDs use Go's reserved range beginning at `0x1_0000_0000_0000`. IDs in the first 16M positions
of that range use a growable page index; ordinary IDs and reserved IDs beyond that limit use a hash table.
Pages hold 256 optional pointers, with each value allocated on first access in the thread's arena. Looking up a
missing link in an existing page returns `None`. Values stay at their first address across later allocations.

This replaces `InlineIdStore` for symbol-node links and `IdLinkStore` for value-symbol links, renaming
`SymbolArenaLinkStore` to `SymbolLinkStore`. The keyed signature/type-node stores and inline reference-kind buckets
are unchanged. Core `LinkStore` accepts either a pointer or a numeric key; the node builder's pointer keys remain
pointer identities. Heap census reports the fallback tables and page indexes; arena allocation profiling accounts
for the pages and link values.

The compact AST layout remains 24 bytes per node and 32 bytes per symbol with compressed pointers (40 bytes per
symbol with plain pointers). The top bit of the stored 32-bit ID distinguishes the reserved range; API calls decode
the full 64-bit ID. Ordinary IDs support `1..2^31-1`; reserved IDs support `0..2^31-1` relative to their base. Exhausting
either range panics, extending the existing bounded-ID representation to the upstream reserved range without
enlarging every node and symbol. ID publication still uses compare-exchange and preserves the winning assignment
when generators race with each other or with ordinary ID assignment.

The symbol comparator now compares full IDs instead of truncating their difference to 32 bits. Truncation could
reverse the ordering between ordinary and reserved IDs. Existing lookup fast paths still avoid assigning an ID
when a node or symbol has none.

## Previous measurements and why revisit

Read `notes/mem-64.md`, `notes/mem-dense-link-tables.md`, `notes/mem-link-tables-landed.md`,
`notes/perf-round2-followups.md`, `docs/RUST.md` and the current summary in `notes/mem-round4.md` before the port.
The earlier rejection of large ID pages measured pages over the shared ordinary ID space: each checker's links
were sparse in blocks assigned by other consumers. The new upstream evidence is a changed ID allocation scheme:
link stores assign unnumbered objects from their own dedicated blocks, and values are allocated individually.
The port's performance threshold was explicitly waived by the user. This is an upstream storage port; no claim
is made that its reported VS Code memory gain transfers to tsrs's already compact tables.

## Measurements

macOS 27.0.1 arm64, Rust 1.99.0, identical `cargo build --release --locked` builds before and after; the Xcode 14.5
SDK is selected because the installed Apple Clang cannot link the default macOS 27 SDK. Projects are the pinned
`typescript-benchmarking` checkout at `41f652ab2df5077b1115f73eaeefc2fe9f674132`, `cases/solutions/Compiler` and
`cases/solutions/Compiler-Unions`, as listed in `bench/projects.json`. Command:

```sh
/usr/bin/time -l <binary> -p <project> --noEmit --pretty false [--singleThreaded | --checkers 8]
```

Three runs per cell; medians. Default leaves the checker count unspecified. Instructions are macOS's process
counter, including kernel work, rather than Linux's deterministic user-space counter from `bench/count.py`.
Peak RSS is `maximum resident set size`, not `peak memory footprint`; MiB = 2^20 bytes.

| project | mode | instructions, G before -> after | change | peak RSS, MiB before -> after | change |
| --- | --- | --- | --- | --- | --- |
| Compiler | single | 2.151 -> 2.181 | +1.44% | 57.97 -> 59.12 | +1.99% |
| Compiler | default | 2.526 -> 2.564 | +1.51% | 77.69 -> 80.02 | +3.00% |
| Compiler | 8 | 2.769 -> 2.820 | +1.83% | 90.30 -> 93.67 | +3.74% |
| Compiler-Unions | single | 4.831 -> 4.906 | +1.54% | 60.36 -> 61.34 | +1.63% |
| Compiler-Unions | default | 5.983 -> 6.075 | +1.54% | 82.47 -> 83.81 | +1.63% |
| Compiler-Unions | 8 | 6.424 -> 6.530 | +1.65% | 96.08 -> 97.08 | +1.04% |

These small projects regress against the previous compact tsrs tables. They do not establish the memory result on
VS Code or the 38k-file codebase, neither measured here. All measured runs have byte-identical diagnostic stdout
and the same exit status (2, the benchmark sources have errors). At one and eight checkers the diagnostics also
match tsgo built from the pinned reference commit. No speed or memory improvement is claimed for this port.

## Validation

- `cargo check --workspace --locked` with `RUSTFLAGS=-D warnings`: pass.
- `cargo test --locked -p tsrs_core -p tsrs_ast -p tsrs_checker -p tsrs_scanner -p tsrs_cli`: pass, including the
  generator race, consecutive/disjoint blocks, compact encoding, absent-link, fallback and stable-address checks.
- The core/AST/checker library checks also pass with `tsrs_core/plain-ptrs`.
- `cargo check --locked -p tsrs_wasm --target wasm32-wasip1` with `RUSTFLAGS=-D warnings`: pass.
- `tools/lint/ratchet.py` and `tools/lint/source.py`: pass; no baseline changes or new unsafe code.
- `tools/regressions.sh`: 28/28 pass, including unique-symbol name truncation.
- Full conformance, `--suite all --baselines types,symbols --timeout 120`, before and after: identical pass and
  failure lists, zero crashes or timeouts. Go-compatible history: 13,458 diagnostics, 12,779 types, 12,779 symbols.
  Canonical history with `TS_TEST_PROGRAM_SINGLE_THREADED=false`: 13,458 / 12,778 / 12,778, the existing
  `objectLiteralNormalization` difference. `TSRS_LAZY_MEMBERS=0`: 13,458 / 12,779 / 12,779.
- Audit against the pinned Go reference with PR 64711's four-file change applied through a Go build overlay:
  diagnostic stdout, stderr and exit status match on both benchmark projects at one, default and eight checkers,
  and on 25/28 regression projects. The three existing differences (`base-type-cycle-entry`,
  `merged-interface-check-site`, `union-too-complex-cross-product`) are identical before and after this port.
  All 34 Rust comparisons are unchanged before/after; the patched and unmodified Go references also agree on all 34.

The README capability counts and supported behavior are unchanged, so the capability table needs no update.
