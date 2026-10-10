# mem-link-tables-landed: inline link values and 4-byte buckets in four link stores

The later TypeScript PR 64711 port replaces change A's symbol-node store and the value-symbol ID store; see
`notes/mem-dense-link-stores.md`. Changes B and C remain. This note records the earlier layout and measurements.

The three changes that notes/mem-dense-link-tables.md (https://github.com/maschwenk/tsrs/pull/134) designed and
prototyped, as production code. Base: origin/main 0b4d117. Machine: Apple M5 Max, 18 cores, 16 KiB pages,
macOS 26.6. Peaks are `/usr/bin/time -l` peak memory footprint, MiB = 2^20 bytes; "main" is a release build of
origin/main at e4f82c0 (0b4d117 changed only CI files).

## What changed (crates/tsrs_checker/src/links.rs)

| commit | store | before | after |
| --- | --- | --- | --- |
| A | `symbol_node_links` (`NodeLinkStore`, links.rs:591) | node id -> 128-id `IdGroup` of 16-bit offsets -> 8-byte slot (4-byte value padded) in a 4,096-value chunk | node id -> block table per 1,024 ids -> `[SymbolNodeLinks; 32]` group, the value in the group (`InlineIdStore`, links.rs:474) |
| B | `signature_links`, `type_node_links` (`KeyedLinkStore`, links.rs:130) | hashbrown bucket of key + index (8 bytes + control byte) | bucket of the index (4 bytes + control byte); the key in the value's padding (`KeyedLinks`, links.rs:113) |
| C | `symbol_reference_links` (`SymbolReferenceLinkStore`, links.rs:217) | key + index bucket, 8-byte arena slot per 4-byte `SymbolFlags` | key + flags in the 8-byte bucket, no arena value |

Sizes (docs/RUST.md rule 3): `SignatureLinks` 12 -> 16 bytes and `TypeNodeLinks` 24 -> 24 with compressed pointers
(the `PSlot` sizes, 16 and 24, are unchanged); with plain pointers 24 -> 32 and 24 -> 32. `SymbolReferenceLinks` is
gone (its one field lives in the bucket). Assertions in types.rs and links.rs.

**Stable addresses** (documented at each store): `LinkStore`, `KeyedLinkStore` and `IdLinkStore` keep their values
in arena chunks that are never reallocated, freed or recycled, and `InlineIdStore` keeps them in arena groups with the
same guarantee, so every reference they hand out stays valid for the store's lifetime. Callers rely on it:
`get_resolved_symbol` keeps its symbol-node links across `resolve_name` (checker_07.rs:1356-1362),
`run_without_resolved_signature_caching` keeps `P<SignatureLinks>` in a vector across a nested check
(services.rs:403-421). `InlineIdStore` hands out `&'static V` instead of `P<V>`: a 4-byte cell is 4-aligned and a
`P` handle names 8-byte units; no code names `P<SymbolNodeLinks>`, and the 24 call sites are unchanged. Only
`SymbolReferenceLinkStore` hands out no reference: its flags move when the table grows, and its three callers read or
OR them on the spot (checker_01.rs:566, 574; checker_13.rs:2435).

Semantics: ids are assigned by the same calls in the same order (`get` and `try_get` still call `get_node_id`). One
difference, internal: `NodeLinkStore::try_get` answers with an unset value for a node whose own links were never
created if another id of its group of 32 has links; its two callers read `resolved_symbol` only
(nodebuilderimpl_1.rs:493-494, flowmemo.rs:544), which is unset either way.

## Numbers

### Peak footprint and instructions (3 interleaved rounds, median and range)

| project, checkers | main | this branch | change | instructions, main -> branch (G) |
| --- | --- | --- | --- | --- |
| vscode, 1 | 1,986.9 (1,986.5-1,987.7) | 1,934.0 (1,932.0-1,934.3) | -52.9 (-2.7%) | 111.3-113.2 -> 111.2-112.8 |
| vscode, 32 | 2,797.0 (2,779.8-2,808.3) | 2,724.8 (2,722.2-2,728.1) | -72.2 (-2.6%) | 139.5-141.1 -> 138.5-138.7 |
| t3code-server, 1 | 866.8 (863.2-870.1) | 860.6 (857.2-860.9) | -6.2 (-0.7%) | 55.2-56.1 -> 55.1-55.3 |
| t3code-server, 32 | 2,826.9 (2,820.2-2,841.3) | 2,782.2 (2,781.1-2,783.8) | -44.7 (-1.6%) | 220.2-221.1 -> 219.6-220.6 |
| vscode, `--singleThreaded` | 1,952.8 (1,952.2-1,955.6) | 1,906.2 (1,905.2-1,909.8) | -46.6 (-2.4%) | 111.72-112.44 -> 110.74-111.76 (median 112.40 -> 110.94, -1.3%) |
| t3code-server, `--singleThreaded` | 836.6 (833.8-836.8) | 827.3 (824.8-827.9) | -9.3 (-1.1%) | 55.22-55.48 -> 54.76-55.32 (median 55.36 -> 54.87, -0.9%) |

Instructions do not rise; the single-threaded medians fall 0.9-1.3%, inside a run-to-run spread of ~0.7% (macOS's
count includes page-fault work). Per lookup: symbol-node links take three dependent loads after the node's id
instead of four, the keyed stores the same as before on a hit, reference kinds two fewer.

### Per store, alloc-profile build (MiB, summed over checkers)

Arena rows from `TSRS_ALLOC_PROFILE_TOP` (by type), heap rows from `TSRS_HEAP_CENSUS=1` (rounded to 0.1 MiB per
checker), one run each of main and the branch:

| store | part | vscode 1 | vscode 32 | t3 1 | t3 32 |
| --- | --- | --- | --- | --- | --- |
| symbol_node_links | value chunks `[PSlot<SymbolNodeLinks>]` | 33.8 -> 0 | 38.8 -> 0 | 6.8 -> 0 | 19.2 -> 0 |
| | offset groups + dense forms + group index | 9.0 -> 0 | 32.0 -> 0 | 1.8 -> 0 | 10.6 -> 0 |
| | `[SymbolNodeLinks; 32]` groups + block tables + block index | 0 -> 17.8 | 0 -> 33.2 | 0 -> 3.6 | 0 -> 13.9 |
| | **net** | **-25.0** | **-37.6** | **-5.0** | **-15.9** |
| signature_links | buckets (values unchanged: 21.5 / 25.3 / 4.5 / 11.9) | 18.0 -> 10.0 | 20.9 -> 11.0 | 4.5 -> 2.5 | 11.6 -> 6.9 |
| type_node_links | buckets (values unchanged: 22.6 / 40.4 / 5.2 / 23.9) | 18.0 -> 10.0 | 24.9 -> 12.8 | 2.3 -> 1.3 | 14.8 -> 8.3 |
| symbol_reference_links | value chunks (buckets unchanged: 9.0 / 17.8 / 2.3 / ~6) | 6.2 -> 0 | 7.9 -> 0 | 1.1 -> 0 | 3.7 -> 0 |
| **all four** | | **-47.2** | **-67.5** | **-9.1** | **-30.8** |

Arena requested (whole process): vscode 1,507.4 -> 1,476.4 MiB at 1 checker, 1,953.8 -> 1,916.1 at 32; t3code-server
717.0 -> 711.0 and 2,582.5 -> 2,549.4 (at 32 checkers the rest of the arena moves by a few MiB between runs with work
stealing). The peak falls by 5-6 MiB more than the arithmetic on vscode (fewer partly used heap pages, inferred), by
14 MiB more on t3code-server at 32 checkers, whose peak varies by ~20 MiB between runs, and by 3 MiB less on
t3code-server at one checker.

Against the design note's model: 47.2 MiB at one checker on vscode as modeled (49.5 MB); 67.5 against 61.6 MiB at 32,
the symbol-node links 3.2 MiB and the two bucket tables 2.7 MiB better than modeled (table capacities are powers of
two and differ between runs).

## Gates

| gate | result |
| --- | --- |
| `cargo test -p tsrs_checker` | ok: 7 unit tests. New: A `inline_ids_reach_their_own_stable_cells` (scrambled ids across blocks and group boundaries, addresses unchanged after later growth, wide ids) and `inline_index_growth_keeps_every_value` (3,000 blocks in ascending order: the index regrows by a quarter many times); B `keyed_store_finds_each_key_through_rehashes` (60,000 keys, ~15 rehashes, absent keys miss); C `reference_kinds_accumulate_in_their_slots`. The two offset-group tests stay for `value_symbol_links`, including offsets past 16 bits |
| `cargo check --workspace` | 0 errors, 0 warnings; `cargo check -p tsrs_checker --features tsrs_core/plain-ptrs` too |
| `tools/lint/ratchet.py` | ok, none new |
| `tools/lint/source.py` | ok: no new `unsafe impl` or atomics; the new `unsafe` blocks (B's chunk lookup, as `LinkStore::at`) carry `SAFETY` comments |
| `tools/regressions.sh` | 19 / 19 pass |
| diagnostics: `cmp` of full `--pretty false` output (stdout, stderr, exit status) against main | identical on vscode, t3code-server, webpack, xstate-main, cal-diy, supabase-studio, formbricks-web, mui-docs at 1, 4 and 16 checkers (24 runs; 371 / 6 / 840 / 0 / 136 / 9 / 0 / 0 errors) |
| `TSRS_ARENA_POISON=1`, alloc-profile build, 16 checkers | vscode and t3code-server: no panic, stdout identical to main |
| census free-gate (`TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1`, plain pointers), vscode | 1 checker, 4 checkers, and 1 checker with `TSRS_LAZY_MEMBERS=0`: 0 strongly reachable freed blocks, 0 references to freed or rewound blocks (40,350,638 checked), output identical |

## Not measured

- Linux (huge-page arena; mimalloc built with `no_thp`). The arena savings (symbol-node groups, reference-kind
  chunks) are arena bytes on every platform; the bucket savings are mimalloc tables.
- The conformance suite (no `ts-ref` checkout here; CI runs it on the pull request).
- Plain-pointer builds (Windows): `SignatureLinks` and `TypeNodeLinks` grow by 8 bytes there against a 12-byte slot
  saved per bucket; not measured.
- Not done here, from the design note: smaller value chunks for small stores (about 7 MB at 32 checkers) and
  mapped-symbol links through the value-symbol groups (15 MB on t3code-server at 32 checkers).
