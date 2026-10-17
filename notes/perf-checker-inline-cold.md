# Checker inline/cold balance: unknown-like union memo hits

Measured 2026-10-16 at `4f27a79b`. The checker asks `isUnknownLikeUnionType` from
`isSimpleTypeRelatedTo`; after the first question about a union, the answer is already held in
`IsUnknownLikeUnionComputed` / `IsUnknownLikeUnion`. The old function nevertheless saved six arm64 register pairs on
every call before reading those bits.

The memo-bit test is now `#[inline(always)]`, and only the first computation is an out-of-line `#[cold]`
function. `cargo asm` showed the old function with a 64-byte frame and saved `x19` through `x24`. Plain `#[inline]`
removed one call site but left the other out of line; `#[inline(always)]` removes both hot calls. The cold worker sets
the computed bit before walking constituents, as before, so recursive queries observe the same state.

## Measurement

Clean fat-LTO release builds, pinned public projects, `RAYON_NUM_THREADS=1`, `--singleThreaded --noEmit
--incremental false --pretty false`. `/usr/bin/time -al` on Apple Silicon reports retired instructions including
kernel work. Runs were interleaved; the first cold pair was discarded. Values below are medians of four warm runs
(for `mikro-orm`, a separate four-pair regression run).

| project | baseline instructions | new instructions | change | peak RSS baseline -> new |
| --- | ---: | ---: | ---: | ---: |
| Compiler-Unions | 3.24212 G | 3.19528 G | **-1.44%** | 62.67 -> 62.57 MB |
| mui-docs | 37.02849 G | 36.99421 G | -0.09% | 768.16 -> 768.25 MB |
| mikro-orm | 53.10711 G | 52.90466 G | -0.38% | 1,173.78 -> 1,173.90 MB |

The qualifying project is `Compiler-Unions`; RSS is unchanged within measurement granularity. The Linux
`bench/count.py` suite still needs a Depot run: the local CLI token was invalid during this measurement.

## Other splits rejected in this round

- Splitting the structured/instantiable decision from `isSimpleTypeRelatedTo` (different from the older object-only
  shortcut) changed Compiler-Unions instructions by about +0.04% and made its paired wall runs slightly slower.
- Moving object-property augmentation to a cold helper changed instructions by +0.05% on Compiler-Unions, -0.02%
  on mui-docs, and +0.02% on mikro-orm.
- Directly decoding `TypeMapper` tags and outlining the callback kinds saved about 0.3-0.6% on mikro-orm, below the
  bar; additionally inlining the simple kind changed the result to +0.07%.

## Correctness

- conformance gate: errors 13,458; `.types` 12,779; `.symbols` 12,779; zero crashes and timeouts
- `tools/regressions.sh target/debug/tsrs`: 36/36
- workspace clippy and `tools/lint/source.py`
