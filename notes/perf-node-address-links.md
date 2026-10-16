# Symbol links after removing node IDs

PR #555 replaces assigned node IDs with allocation addresses. Its first version replaced the dense
`symbol_node_links` store with the generic hashed `LinkStore`. The node header remained 24 bytes, and
CI instructions increased on all 17 projects: Compiler +1.308%, vscode +0.943%, median +0.433%.
Restoring a private atomic index was explicitly rejected: the node must have no assigned index. The final
header/identifier follow-up below reduces instructions and single-threaded RSS against the original dense-ID baseline.

The initial storage fix changes only `NodeLinkStore`. Divide the allocation address by `size_of::<Node>()` to obtain
a slot. Two live nodes cannot share a slot because their headers alone occupy that many nonoverlapping
bytes. Compressed handles use the equivalent division in eight-byte units. A two-level sparse table
allocates groups of eight values on demand; each second-level page holds 128 group pointers. Plain
pointers use a hash table of pages to avoid a directory spanning the virtual address space.

Values remain in fixed arena allocations. Existing callers can keep their references across recursive
checks, so no checker algorithms or cache APIs change. Forks copy entire groups from their frozen
parent, preserving untouched values and explicit empty overrides. The node header stays 24 bytes.

This revisits address-indexed storage under a changed constraint: dense IDs are no longer available.
The rejected handle arrays in `mem-dense-link-tables.md` concerned sparse signature links. Here the
values are four-byte symbol handles, address units are a whole node header, and groups have eight
slots. Other link stores are unchanged.

## Storage-fix measurements

Apple M5 Max, macOS, release builds, `/usr/bin/time -l`. Public TypeScript benchmark suite pinned to
`41f652ab2df5077b1115f73eaeefc2fe9f674132`. Commands use `--noEmit --incremental false --pretty false`;
single mode also uses `--singleThreaded` and `RAYON_NUM_THREADS=1`. Interleaved runs, one warmup per
binary discarded, medians of ten single runs and six default runs. Output and exit status identical.

The controlled comparison keeps the original PR's source baseline: dense = `d4f5b06f`, hash =
`dcb3d5ac`, fixed = `dcb3d5ac` with the final `links.rs` change. This isolates storage from the parser
inlining changes subsequently brought in by rebasing onto `6dd3374b`. These are local hardware
counter measurements, not Linux `bench/count.py` results or a wall-time claim.

| Project | Mode | Dense instructions (G) | Hash instructions (G) | Fixed instructions (G) | Dense peak RSS (MiB) | Hash peak RSS (MiB) | Fixed peak RSS (MiB) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Compiler | single | 1.75216 | 1.77696 | 1.74988 | 57.25 | 61.81 | 59.10 |
| Compiler-Unions | single | 3.24300 | 3.26667 | 3.24091 | 59.91 | 64.05 | 61.62 |
| Compiler | default | 2.06102 | 2.08706 | 2.05768 | 78.78 | 83.24 | 81.18 |
| Compiler-Unions | default | 3.91792 | 3.93979 | 3.91359 | 83.56 | 87.48 | 85.64 |

Against the PR's hash store: single instructions -1.52% / -0.79%, single RSS -4.39% / -3.78%, default
RSS -2.48% / -2.10%. The instruction regression is recovered locally; memory still exceeds the original
dense store by 2.5–3.2%. No overall speedup or qualifying memory gain over the original dense store is claimed.

Earlier candidates, measured on the same public projects:

- Inline hash buckets holding both node and symbol removed the separate arena values but retained most
  of the instruction regression (Compiler +1.03% versus dense).
- Custom linear probing remained +1.00% versus dense and did not justify another hash implementation.
- Address bitmaps with packed values saved memory but still cost +0.91% instructions versus dense.
- Fixed groups of 32 recovered instructions; groups of eight reduce unused slots with a small instruction
  tradeoff. Compiler's store census falls from about 2.72 MiB to 2.47 MiB, including its arena allocations.

## Validation

Conformance pass sets are unchanged: 13,458 errors, 12,779 types, 12,779 symbols. Default canonical
threading retains only the existing `objectLiteralNormalization` type/symbol mismatch. Emit pass sets
are unchanged: 13,462 errors, 13,392 JS/declaration, 149 JS maps, 156 source-map records.

AST/checker/API/codec and core checks pass, as do the 36 regression projects, arena recycling and LSP
lifetime checks, shared-parent isolation, plain pointers, sizing/shared-graph feature compilation, WASM
compilation, workspace Clippy, and source lint. The link-store checks retain references across growth
and exercise copying, empty overrides and nested forks.

## Unpacked header and compact identifiers

The next change revisits the packed header under two changed constraints: parent pointers are now four-byte
handles and the node ID field is gone. Ordinary fields fit in 20 bytes with four-byte pointers (24 with eight-byte
pointers). Kind and data-tag reads become direct loads, and setting a parent becomes a direct store. This also
removes the enum transmutes and packed-pointer reconstruction. No ID field, counter, hash or cache is added.

The arena still rounds allocations to eight bytes. A token therefore remains 24 bytes, while a node with a
12-byte, four-aligned payload falls from 40 to 32 bytes. Symbol-link address units round the header size up to
three handle units; truncating `20 / 8` would unnecessarily spread the slots out. `FlowReduceLabelData` explicitly
retains eight-byte alignment because the checker takes a compressed pointer to that payload. The other such
payload, `SourceFile`, has an alignment assertion.

With compressed pointers, identifiers use the marker byte's six spare bits for a four-bit text length and two-bit mode. Their flow-node
handle or source-text index then needs only four payload bytes. Source identifiers of up to 15 bytes occupy
24 bytes including the header. Other identifiers store a `PackedStr` and occupy 32 bytes. Longer source names
therefore retain their old 32-byte allocation size; other stored-text identifiers fall from 40 to 32 bytes.
Full-width source ranges and all 32 node-flag bits are preserved. Plain pointers keep the previous full-width
identifier encoding.

The metadata is initialized before factory hooks run. An initial source identifier also carries the length above
its 20-bit source index until the first flow update; source-index reads mask those bits. Updates preserve the
rare-tail and trailing-comma bits. Both identifier forms have the same payload offset, asserted at compile time,
and the stored-text form uses four-byte alignment. Boundary checks cover both sides of the 15-byte cutoff,
flow transitions within and across files, clearing flow, cloning, parent changes, all flag bits and comma markers.

### Final controlled comparison

Same hardware, public projects, command lines and source baseline as above. The final binary combines the
address IDs, eight-entry symbol groups, unpacked header and compact identifiers. Ten measured interleaved
single-threaded runs and six default-checker runs per binary, after one warmup. Output and exit status match
for every run. These are local hardware counters; Linux CI measurements remain a separate check.

| Project | Mode | Dense instructions (G) | Final instructions (G) | Dense peak RSS (MiB) | Final peak RSS (MiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| Compiler | single | 1.75287 | 1.71870 | 57.23 | 56.78 |
| Compiler-Unions | single | 3.24411 | 3.21008 | 59.91 | 59.50 |
| Compiler | default | 2.04645 | 2.01148 | 77.81 | 77.73 |
| Compiler-Unions | default | 3.90154 | 3.86623 | 83.11 | 83.05 |

Against the dense-ID baseline, single-threaded instructions fall 1.95% / 1.05% and RSS 0.79% / 0.68%.
Default-checker RSS is effectively unchanged (the measured 0.10% / 0.07% differences are too small to claim).
Against the prior PR head's eight-entry address store with the 24-byte header, single instructions fall
1.82% / 0.96%, single RSS 3.91% / 3.42%, and default RSS 3.23% / 2.85%. Against the initial hashed PR,
single instructions fall 3.30% / 1.77% and single RSS 8.12% / 7.10%.

The instruction improvement clears the local numerical threshold on Compiler; it is not a wall-time claim or
proof of a default-memory improvement against dense IDs. The draft still needs the Linux project-wide gate.

Other layouts measured and rejected in this follow-up:

- A 16-byte header with two 22-bit positions and an out-of-line full-width fallback saved about 2% RSS against
  dense IDs but added about 0.8–0.9% instructions. Byte-aligning the data tag did not remove that regression.
- A 16-byte header keeping the end offset directly and a short length in the header moved long-range starts and
  parents out of line. It added about 1.7–2.5% instructions, chiefly by making parent access conditional.
- Four-entry symbol groups with the unpacked header saved only about 0.2–0.3% RSS against eight-entry groups and
  added about 0.05–0.15% instructions. Eight-entry groups remain.

Follow-up validation: conformance pass sets remain 13,458 errors, 12,779 types and 12,779 symbols; emit remains
13,462 errors and 13,392 JS/declaration baselines. AST/checker/API/codec checks pass, including the compact
identifier boundary and factory-hook check and the compressed flow-payload address check. Plain-pointer/shared-graph
checks, WASM, sizing/shared-graph feature compilation, checker recycling, leaf freeing and LSP edits pass.
The AST generator reproduces the checked-in output; workspace Clippy and source lint pass.
