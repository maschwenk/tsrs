# Rust representation audit

Branch `refactor/rust-repr-layouts` starts at `e99e1816`. This audit removes Rust representation attributes that
did not participate in FFI, unsafe decoding, pointer ABI, or an allocation/prefix/packed layout. It is an
owner-requested Rust data-model cleanup, not a performance claim.

## Audit

Outside the language-service tree, the repository had 107 `repr` attributes. This change removes 83:

- 71 fixed representations from ordinary internal enums;
- 11 `repr(transparent)` attributes from ordinary single-field wrappers; and
- `repr(C)` from `IdGroup`.

No `From`, `Into`, `TryFrom`, or replacement conversion helper was introduced. `QuoteChar` keeps its character
values as direct hexadecimal discriminants because a default-representation enum's discriminants must be
`isize`-compatible. The six `repr(i32)` enums in `tsrs_ls` are out of scope and unchanged.

The 24 retained attributes have a concrete layout contract:

- unsafe-decoded tags: `tsrs_ast::Kind` and `NodeDataTag`;
- pointer ABI: both configurations of `P<T>` and `SP<T>`;
- C FFI: `HeapArea`, `FSEventStreamContext`, `Timespec`, `MachHeader64`, `SegmentCommand64`, and `PthreadAttr`;
- unsafe prefix/allocation layouts: `NodeAlloc`, `NodeAllocRare`, `IdentifierWithText`, `LazyNodeList`, `TypeAlloc`,
  `UnionOrIntersectionRare`, `UnionRare`, and `IntersectionRare`; and
- packed storage: `PackedSlot`, `PackedHashKey`, `LinkSlot`, `PackedSlice`, and `PackedOptionSlice`.

## Layout

`rustc -Zprint-type-sizes` on the release checker library showed the expected compiler-selected layouts. Examples
of enums formerly fixed at `u32`/`i32` (`TypeMapperKind`, `JsxReferenceKind`, `SymbolAccessibility`,
`TypeReferenceSerializationKind`, `SignatureKind`, `IndexKind`, `MemberOverrideStatus`,
`MembersOrExportsResolutionKind`, `TypePredicateKind`, and `AssignmentKind`) changed from size/alignment 4/4 to
1/1. Enums already fixed at one byte, including `TypeDataTag` and `ExhaustiveState`, stayed 1/1. Single-field
wrappers such as `MapperCell` and `ObjectFlagsCell` stayed 4/4. Removing `repr(C)` did not change `IdGroup`: it is
264 bytes aligned to 4 in both builds.

The native and `wasm32-wasip1` workspace checks also compile all existing layout assertions: `Type` and `Node`
remain 24 bytes; the AST/type allocation offsets, pointer niches, packed slots, and `IdGroup` size are unchanged.

## Screening measurement

Release binaries from the base and this branch were measured on macOS arm64 against pinned public VS Code at
`9cf0128b9822`. Each cell is the median of three runs after a warm-up; base and changed binaries alternated within
each round. The invocation was the benchmark's `--noEmit --incremental false --extendedDiagnostics --pretty false`
check, with the listed checker count. Diagnostics without timing counters were byte-identical.

| checkers | base wall | changed wall | delta | base peak RSS | changed peak RSS | delta |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 7.270 s | 7.272 s | +0.03% | 1599.4 MiB | 1599.1 MiB | -0.02% |
| 4 | 2.776 s | 2.774 s | -0.07% | 1810.6 MiB | 1812.6 MiB | +0.11% |
| 16 | 2.003 s | 1.977 s | -1.31% | 2117.4 MiB | 2123.1 MiB | +0.27% |
| 32 | 2.035 s | 2.097 s | +3.06% | 2622.3 MiB | 2619.4 MiB | -0.11% |

The host's benchmark cache contained only VS Code, so the other public projects were not available for a local
before/after run. `bench/count.py` reports `instructions: null` with `no perf_event_open on darwin/arm64`; retired
instructions therefore require the Linux benchmark job. The wall-time spread and sub-0.3% RSS deltas do not
establish a performance or memory improvement.

## Validation

- `cargo check --workspace --locked`
- `cargo check -p tsrs_wasm --target wasm32-wasip1 --locked`
- 119 core/scanner unit tests passed; one Miri negative control remained ignored
- all 29 regression projects passed
- conformance reference history: 13,458 errors / 12,779 types / 12,779 symbols
- conformance canonical history: 13,458 errors / 12,778 types / 12,778 symbols
- fourslash: 4,066 passed / 63 known failures / 417 skipped
- `tools/lint/ratchet.py` and `tools/lint/source.py`
- all eight generators reproduced the pre-generator patch exactly
