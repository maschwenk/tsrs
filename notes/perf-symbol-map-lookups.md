# perf-symbol-map-lookups: the checker's symbol table lookups

Follow-up to notes/perf-checker-cpu4.md "Profile", where `SymbolMap::search` (2.9%) and `SymbolMap::position` (2.0%)
were the largest self costs of the vscode check that are not an algorithm. Same rules as notes/perf-checker-cpu3.md:
representation and code shape only, identical diagnostics, baselines and `--extendedDiagnostics` counters, no change to
which symbols or types exist or to the order a table iterates in. Base: origin/main 6baca34.

## What a lookup is

`SymbolTable` (crates/tsrs_ast/src/symbol.rs) is the insertion-ordered entry array of notes/perf-checker-cpu3.md
change 1: one 8-byte word per entry (symbol handle plus a key fingerprint of length and 12 hash bits), a 64-bit Bloom
filter in the table header while the table has at most 16 entries, a hashbrown index of positions above that. A
lookup hashes the name (rustc-hash 2), tests the filter, and only then calls the out-of-line `search` (a linear
fingerprint scan, or the index), which compares the text by pointer and length first. Keys are compared as text;
nothing is interned.

## Census

Site counts (`--features site-counts` with a local patch on `SymbolTable::lookup` / `get` / `has` / `lookup_entry`,
not committed), Mac, single-threaded, whole run:

| | vscode | t3code-server | supabase-studio |
| --- | ---: | ---: | ---: |
| lookups | 37.7M | 15.1M | 80.2M |
| ended by the table's filter | 40% | 30% | |
| linear table, hit (~3.4 entries visited) / miss | 25% / 1% | | |
| indexed table (> 16 entries), hit / miss | 22% / 12% | | |
| empty table | 3% | 4% | |
| name of at most 16 bytes / 17-32 / longer | 85% / 14% / 1% | | 86% / 14% / 0.1% |
| same table and name pointer as the lookup just before | 5.5% | | |
| same name text as the lookup just before (any table) | 42% | 30% | 64% |

Table sizes at lookup (vscode): 0: 3%, 1: 8%, 2-4: 23%, 5-8: 22%, 9-16: 9%, 17-64: 22%, 65-512: 10%, more: 1%.

Where they come from (vscode): a type's own members in `get_property_of_type` (`get_member_of_structured_type_ex`)
40%, the `Function` / `Object` fallback after a miss there (`get_property_of_object_type`) 14%, the name resolver's
`get_symbol` 17%, the binder's `declare_symbol` 5%, `get_export_of_module` 3%, the rest spread over ~190 sites. On
supabase-studio the fallback is 41% of all lookups (33M): most property lookups there miss the type's own members.

What a lookup costs, measured on the 64-vCPU Linux runner with `bench/count.py` (user-space instructions, exact,
`--release`) on a build that repeats part of every lookup, picked by an environment variable (local patch, not
committed; the percentages are against the same build repeating nothing):

| repeated per lookup | vscode | t3code-server | formbricks-web | supabase-studio | cal-diy |
| --- | ---: | ---: | ---: | ---: | ---: |
| the name's hash | +0.98% | +0.99% | +1.56% | +3.25% | +1.84% |
| the search past the filter | +2.70% | +2.93% | +2.19% | +2.77% | +2.63% |
| the whole lookup (hash, filter, search) | +3.61% | +3.83% | +3.81% | +6.29% | +4.52% |

So a lookup is about 100 instructions on vscode (3.78 G over 37.7M), about a third of it the hash.

## Change: one hash per property lookup, and no `Function` / `Object` fallback for names they do not have

`get_property_of_type` on an object type looks the name up in the type's own members; on a miss (or a member that is
not a value) it tests the type for call and construct signatures to pick `Function`, `CallableFunction` or
`NewableFunction`, looks the name up there, then in `Object`. Each of those lookups hashed the name again, and the
fallback ran for nearly every miss, though almost no name is a member of those four types.

1. `HashedName` (tsrs_ast) carries a name with its hash; `SymbolTable::lookup_hashed` skips the hash.
   `get_property_of_type_worker` hashes once for the own-member lookup (when the type's members are resolved) and the
   two fallback lookups (`get_property_of_object_type_hashed`).
2. `NameFilter` (tsrs_ast): a 256-bit Bloom filter of the keys of several tables (two bits per key, from other hash
   bits than a table's own filter). The checker builds one (`augment_filter`) over the member tables of the four
   global types, once all four have their members resolved; until then it answers "maybe" and builds nothing, so it
   never resolves them earlier than the fallback would. When the looked-up type's members are resolved and the filter
   rejects the name, `get_property_of_type_worker` returns `None` at once.

Why the result is the same: a name the filter rejects is a key of none of the four tables, so both fallback lookups
would miss and the function would return `None`. The skipped steps have no effects for a type with resolved members:
`signatures_of_structured_type` takes no lazy-member path then (`may_have_lazy_members` is false for a resolved type,
and the mapped-type early return only counts unresolved types) and returns the resolved signatures;
`get_property_of_object_type` on the four resolved types only reads. A type whose members are not resolved (the lazy
member path) still takes the full fallback. The four types are set once at checker initialization, and a resolved
member table is not written to afterwards (the `set` calls on member tables are on tables being built: resolution,
mapped types, literals, JSX attributes). The only way a resolved type's members are replaced is the base-type cycle
reset (`get_base_types`, `reset_resolved_base_types`), which clears `MembersResolved`; both reset sites drop the
filter, which is built again once all four are resolved.

How often the filter skips the fallback (site counts on the change, single-threaded):

| | vscode | t3code-server | formbricks-web | supabase-studio | cal-diy |
| --- | ---: | ---: | ---: | ---: | ---: |
| property lookups reaching the fallback | 3.88M | 1.23M | 8.45M | 31.81M | 6.99M |
| skipped by the filter | 3.40M (87%) | 1.02M (83%) | 7.97M (94%) | 31.22M (98%) | 6.68M (96%) |
| looked-up type not resolved (full fallback) | 0.34M | 0.17M | 0.16M | 0.14M | 0.15M |

A skip saves the two signature tests, two calls into `get_property_of_object_type` with their member resolution
checks, and up to two hashes: about 190 instructions on supabase-studio.

## Result

`bench/count.py` on the 64-vCPU Linux runner, `--profile dist` (fat LTO), single-threaded (`--singleThreaded`,
`RAYON_NUM_THREADS=1`), base 6baca34 and the change built in the same job (`tools/perf/abprobe.sh`, not committed);
outputs byte-identical in every cell:

| project | base | change | instructions | peak RSS |
| --- | ---: | ---: | ---: | ---: |
| vscode | 100.635 G | 99.634 G | **-0.99%** | 1610 -> 1610 MiB |
| t3code-server | 49.474 G | 49.124 G | **-0.71%** | 770 -> 770 MiB |
| formbricks-web | 50.651 G | 48.688 G | **-3.88%** | 1311 -> 1311 MiB |
| supabase-studio | 62.725 G | 55.251 G | **-11.92%** | 927 -> 928 MiB |
| cal-diy | 39.721 G | 38.016 G | **-4.29%** | 823 -> 823 MiB |

The `--release` builds of the screening runs gave -0.59 / -0.21 / -2.97 / -9.62 / -3.22% for the same change.

Wall time at 32 checkers, vscode, 11 interleaved runs per binary (`tools/perf/abprobe.py`, medians, min-max in
brackets); diagnostics, `--listFiles` and `--explainFiles` output identical:

| | base | change |
| --- | ---: | ---: |
| wall | 568.3 ms (556-585) | 567.1 ms (551-586) |
| Check time | 414 ms (406-429) | 417 ms (403-437) |
| peak RSS | 2706 MiB | 2708 MiB |

No change at 32 checkers on vscode, the project that gains least: 1.0 G instructions spread over 32 checkers is about
1 ms each. Supabase-studio's 7.5 G is where the change shows.

## Tried and dropped

All on the Linux runner, `bench/count.py` single-threaded, against the same base built in the same job.

| candidate | vscode | t3code-server | formbricks-web | supabase-studio | cal-diy |
| --- | ---: | ---: | ---: | ---: | ---: |
| hash once per name-resolver walk (`HashedName` through the resolver's `lookup` hook, `get_symbol_hashed`), alone | 0.00% | +0.01% | +0.01% | +0.01% | 0.00% |
| a cheaper `hash_name` for names up to 16 bytes (one 128-bit multiply, no Fx steps), alone | +0.22% | +0.19% | +0.46% | +0.72% | +0.58% |
| hash once for the property path and the resolver, no filter | -0.05% | +0.14% | -0.45% | -1.63% | -0.48% |

- The resolver walk hashes the name again per scope, but most walks end in the first or second table and the hook
  call around each lookup dominates; nothing measurable.
- The cheaper hash costs more than it saves: its filter and fingerprint bits are worse (more linear scans and index
  probes), and rustc-hash's extra steps for short names are only a few instructions.
- Not tried, from the census: a cache of the last (table, name) (5.5% of lookups repeat exactly, ~0.2% at most); a
  separate negative cache per resolver scope (resolver hash-once already shows the resolver's lookups are not where
  the cost is); a hash index for tables of 9-16 entries (the linear scan visits ~3.4 fingerprints). Global interning
  and identifiers carrying their hash stay rejected (docs/RUST.md "Techniques").

## Gates

Local (Mac), the change's `--release` binary against a base build of 6baca34: `--pretty false` output byte-identical
on vscode, t3code-server, formbricks-web, supabase-studio, cal-diy, mui-docs, webpack and xstate-main single-threaded
and at 4 and 32 checkers, and at 4 and 32 checkers with `--checkerAssignment go`; `--extendedDiagnostics` counters
identical single-threaded (timing and memory rows excluded): 48 of 48 cells. `cargo check --workspace` without
warnings, `tools/lint/ratchet.py`, `tools/lint/source.py`, `cargo test -p tsrs_core -p tsrs_ast -p tsrs_checker -p
tsrs_compiler` (new: `name_filter_keeps_every_key`).

`pr-verify` (17 projects x 1/4/16/32 checkers plus a poisoned-arena run, 64-vCPU runner, `--release`, against main
7d7e54a): identical diagnostics in 102 of 102 cells. Its single-threaded instruction deltas: supabase-studio -9.573%,
mui-docs -4.879%, cal-diy -3.161%, formbricks-web -2.904%, xstate-main -2.231%, playwright -2.153%, storybook
-1.892%, next-packages-next -0.836%, mikro-orm -0.680%, Compiler-Unions -0.560%, vscode -0.360%, next-root -0.207%,
t3code-server -0.130%, nuxt -0.101%, webpack -0.092%, drizzle-orm +0.079%, Compiler +0.270%. Wall and peak RSS moved
within its three-run spread.
