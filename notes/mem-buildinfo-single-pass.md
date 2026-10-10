# Single-pass typed `.tsbuildinfo` decoding

Issue #258 item 1 identified the generic JSON tree in `BuildInfo::unmarshal` as a warm-incremental peak-memory
candidate. This prototype extends the existing strict JSON parser with a typed streaming reader and decodes known
fields directly into `BuildInfo`. Unknown fields are validated and discarded; only the open-ended `options` object
becomes a generic `json::Value` subtree.

Base: `8529d96a` (2026-10-10), macOS arm64. PR #259 is the two-pass serde prototype used for the direct decoder
comparison below.

## Design

The reader handles object and array framing, strings, booleans, bounded `i32` values and general `f64` values. Every
object tracks its names and rejects duplicates, including in discarded unknown fields. Discarding recursively checks
syntax and finite numbers without retaining values. This preserves the general decoder's strict behavior in one pass,
without a serde dependency or an artificial nesting limit.

The typed decoder preserves null/missing defaults, unknown-field handling, tuple variants, empty versus missing
`fileInfos`, option insertion order and the custom diagnostic/emit shapes. Differential tests cover all wire shapes,
malformed input, duplicate names, fractional/out-of-range `i32` values and unknown data nested beyond 128 containers.

## Isolated decoder

An uncommitted release example timed only `BuildInfo::unmarshal`. The synthetic document contains one `fileIdsList`
of 9.5 million IDs and is 55,945,044 encoded bytes. `/usr/bin/time -l` covers identical input generation as well as
the timed decode:

| decoder | decode time | process instructions | peak RSS |
| --- | ---: | ---: | ---: |
| generic `Value`, then typed (`main`) | 163 ms | 6.006 B | 629.9 MB |
| strict scan, then serde typed (PR #259) | 154 ms | 6.600 B | 97.9 MB |
| single-pass typed | **51 ms** | **3.961 B** | **97.8 MB** |

Against `main`, the single pass is 69% faster in the timed region, retires 34% fewer whole-process instructions and
uses 84.5% less peak memory. Against the serde prototype it is 67% faster and retires 40% fewer whole-process
instructions, with the same memory. The serde result is slower than its earlier 139 ms sample because this generated
ID distribution differs; all three rows here use the same binary shape and input generator.

The pinned vscode build info is 12,888,797 bytes with 10,427 files and 1,963,498 IDs. Five interleaved runs of the
same release example gave these medians:

| decoder | decode time | process instructions | peak RSS |
| --- | ---: | ---: | ---: |
| generic `Value`, then typed (`main`) | 52 ms | 1.134 B | 173.3 MB |
| strict scan, then serde typed (PR #259) | 53 ms | 1.190 B | 30.1 MB |
| single-pass typed | **26 ms** | **0.597 B** | **28.8 MB** |

The process counters include startup and reading the input; the decode-time column does not. The single-pass row
retires 47% fewer instructions than `main` and 50% fewer than the serde prototype in this isolated process.

## Pinned vscode warm incremental

The command and cached checkout are the same as the typed-decoder measurements on PR #259:

```text
tsrs -p src --noEmit --incremental --tsBuildInfoFile /tmp/tsrs-vscode-258.tsbuildinfo --pretty false
```

Three interleaved default-mode runs gave median peak RSS of 1,389.8 MB on `main` and 1,327.1 MB with the single-pass
decoder: **-62.6 MB (-4.51%)**. Median wall time was 1.34 -> 1.27 s, although one `main` run was a 2.42 s outlier.
Extended diagnostics reported `BuildInfo read: unmarshal` at 56 -> 20 ms and total build-info reading at 85 -> 53
ms. Error output, counters and steady-run behavior matched, and no build info was rewritten.

Five interleaved `--singleThreaded` samples gave median instructions of 27.847 B on `main` and 26.500 B with the
single-pass decoder, **-4.84%**. The samples were visibly bimodal (the two decoder medians were stable within their
clusters but macOS counters moved between clusters), so Linux `bench/count.py` must still confirm the deterministic
instruction result before landing.

The default-mode RSS result remains below the 5% memory bar: once the generic tree is gone, the later retained
snapshot reference sets determine the program peak. If deterministic Linux instructions do not clear 1%, combine
this decoder with issue #258 item 2's compact retained reference sets rather than landing it alone.
