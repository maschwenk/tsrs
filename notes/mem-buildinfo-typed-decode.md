# Typed `.tsbuildinfo` decoding

Issue #258 item 1 identified the generic JSON tree in `BuildInfo::unmarshal` as a warm-incremental peak-memory
candidate. This prototype decodes directly into the `BuildInfo` vectors with serde. Only the small, open-ended
`options` object remains a generic `json::Value` subtree.

Base: `8529d96a` (2026-10-10), macOS arm64. The comparison binary was one release build with a temporary environment
switch between the old and new decoder, so all code outside decoding was identical.

## Design

`serde_json` alone is not strict enough for the compiler's JSON contract: it accepts duplicate names in generic or
ignored objects. A first pass therefore validates syntax, finite numbers and duplicate names at every depth without
retaining values. Its integer fast path only scans digits, which matters for `fileIdsList`. The second pass fills the
typed structures directly. The validator and typed result never coexist with a generic document tree.

The typed decoder preserves null/missing defaults, unknown-field handling, tuple variants, empty versus missing
`fileInfos`, option insertion order and the custom diagnostic/emit shapes. Tests compare all wire shapes against the
old decoder and cover malformed input, duplicate names and fractional/out-of-range `i32` values. Integer bounds are
stricter than the old port's saturating `f64 as i32` conversion and match Go's typed JSON decode.

Serde and serde_json are already linked into the CLI through `tsrs_api_transport`; this only adds direct dependencies
from `tsrs_incremental`.

## Isolated decoder

A temporary example generated the large part of the 38k-file build info: one `fileIdsList` containing 9.5 million
IDs (54,889,897 encoded bytes). One optimized development binary was measured with `/usr/bin/time -l`:

| decoder | decode time | instructions | peak RSS |
| --- | ---: | ---: | ---: |
| generic `Value`, then typed | 269 ms | 7.848 B | 630.9 MB |
| strict scan, then typed | 139 ms | 6.806 B | 97.1 MB |
| change | **-48%** | **-13.3%** | **-84.6%** |

The 97 MB includes the 55 MB input and about 38 MB of final `i32` storage. A strict typed pass without the validator
was 97 ms and 5.475 B instructions; optimizing validation to scan ordinary integers without converting them recovered
most of that difference while retaining duplicate-name checks in unknown fields.

## Pinned vscode warm incremental

Setup used `python3 bench/run.py --setup-only --projects vscode`, hence vscode `9cf0128b9822` and the repository's
normal `npm ci --ignore-scripts` install. A cold incremental run produced a 12,888,797-byte build info. The measured
command was a no-edit warm run:

```text
tsrs -p src --noEmit --incremental --tsBuildInfoFile /tmp/tsrs-vscode-258.tsbuildinfo --pretty false
```

Default-mode RSS, three interleaved runs:

| decoder | maximum RSS samples | median |
| --- | --- | ---: |
| generic | 1,394.3 / 1,391.0 / 1,387.8 MB | 1,391.0 MB |
| typed | 1,329.0 / 1,329.5 / 1,334.9 MB | 1,329.5 MB |
| change | | **-61.5 MB (-4.42%)** |

Median wall time was 1.28 -> 1.27 s. With extended diagnostics, `BuildInfo read: unmarshal` was 54 -> 34 ms. Error
output, counters and the steady-run behavior were the same; as expected, no build info was rewritten.

The default-mode RSS improvement is real but does not by itself reach the 5% memory landing bar. The generic tree's
local peak overlaps only the early part of parallel program construction; later, the retained snapshot reference sets
overlap the program's actual peak.

Single-thread instruction samples (`--singleThreaded`, three interleaved runs) were:

| decoder | instructions | median |
| --- | --- | ---: |
| generic | 27.886 / 27.759 / 27.882 B | 27.882 B |
| typed | 27.348 / 27.695 / 26.794 B | 27.348 B |
| change | | **-1.92%** |

That clears the 1% instruction threshold in the median, but macOS `/usr/bin/time` counters have visible run-to-run
spread. Confirm this row with Linux `bench/count.py` before landing. If the deterministic count does not clear 1%,
leave the decoder as a prototype and combine the idea only with issue #258 item 2's compact retained reference sets;
those sets, not typed `fileIdsList`, are present at the default-mode program peak.
