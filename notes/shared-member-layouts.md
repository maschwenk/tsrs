# Shared class/interface member layouts (microsoft/TypeScript#64765)

Port of [microsoft/TypeScript#64765](https://github.com/microsoft/TypeScript/pull/64765), commit
`ecc74a8976efe6d8177d64a9d7c3bd11a838e514` ("Reuse scratch storage when building member layouts"). The default
checker follows that Go implementation. It supersedes the #64475 class/interface lazy-member tables; the #64526 lazy
mapped-type path remains in place.

## Port

`MemberLayout` records member keys, order and origins once for a class or interface when all of its bases allow the
layout to be shared. `SharedMemberSlots` holds the mapper, instantiated bases and per-instantiation resolved slots.
Lookups, full property/table materialization, weak/empty type tests, relation unmatched-property walks, node building
and services use those slots just as the Go change does.

The upstream follow-ups rebuild a per-instantiation layout when a base instantiation fails and reuse checker-owned
entry and name-index scratch storage while building layouts. Rust checker histories can also instantiate a declared
type-parameter or conditional base to unrelated class/interface layouts. Those sources do not cache a shared layout;
declared class/interface references, whose target layout is stable across type arguments, remain shared.

The Rust representation uses arena `ThinSlice`s and overlay-aware cells because checker graphs can be forked. It
does not store Go's checker pointer in the slot record: member resolution already has `&mut Checker`. On 64-bit,
`MemberLayout` is 48 bytes; `SharedMemberSlots` and `StructuredMembers` are 32 bytes with the default compressed
pointers and 40 bytes with plain pointers. Their 32-bit layouts are 28 bytes. The allocation census knows the four
layout slices and the slot record's base/slot slices, and the old checker-owned lazy-member maps and their heap-census
entries are gone. The heap census includes the retained layout-building scratch storage.

For the repository's reference-equivalence gate only, `--noLazyMembers` / `TSRS_LAZY_MEMBERS=0` retains the eager
pre-#64765 `resolveObjectTypeMembers` path. Default mode is the #64765 algorithm.

## Correctness

- Full conformance, default: errors 13,458 pass + 2 codes + 2 expected failures; `.types` 12,779; `.symbols` 12,779.
- Canonical multi-thread programs: the same error totals and the established 12,778 type/symbol passes (the known
  `objectLiteralNormalization` history difference).
- Eager oracle mode (`TSRS_LAZY_MEMBERS=0`): 13,458 / 12,779 / 12,779.
- `tools/regressions.sh`: every regression passed. Required Clippy and source lints passed.

## Measurement

Release builds of clean `HEAD` (#64475 + #64526) and this port were measured back-to-back on macOS 26.6.2 arm64,
12 logical CPUs, against the public MUI docs benchmark (16,766 files). Each peak is the median of three interleaved
runs after warm-up, recorded by `bench/count.py`/`wait4`; lower is better.

| checker mode | before peak RSS | after peak RSS | change |
| --- | ---: | ---: | ---: |
| single | 738.2 MiB | 724.0 MiB | -1.93% |
| 4 checkers | 1051.0 MiB | 1053.0 MiB | +0.18% |
| default (12 checkers) | 1504.3 MiB | 1526.0 MiB | +1.44% |

The same single-checker run changed symbols/types/instantiations from
2,455,368/826,542/2,719,933 to 2,518,549/829,155/2,732,758; this is the new upstream resolution history, not an
output change. Retired instruction counts are unavailable on this Darwin/arm64 host (`bench/count.py` reports no
`perf_event_open`), so no instruction or CPU improvement is claimed. The measured memory result does not clear the
repository's 5% performance bar; this change lands for exact upstream parity, not as an independent optimization.
