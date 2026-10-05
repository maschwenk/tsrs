# perf-probes-2026-10-05: hot/cold text grouping and string hashing

Two candidates from docs/RUST.md ("Not tried"), measured on the bench machine (`depot-ubuntu-24.04-8`, AMD EPYC
9R45, glibc 2.39) with `depot ci run`. Neither is adopted.

## Hot/cold text grouping with PGO (`-Wl,-z,keep-text-section-prefix`)

Bun passes it when a PGO profile is loaded, so the linker keeps PGO's hot and cold functions grouped. Method: one
training run, one profile, two `dist` builds from it, A (as released) and B (`-Clink-arg=-Wl,-z,keep-text-section-prefix`).
B has a 3.7 MB `.text.hot` and a 4.1 MB `.text.unlikely` next to a 5.9 MB `.text`; A has one 13.7 MB `.text`. Each
project was type-checked 9 times per binary, alternating which goes first, both single-threaded and with the default
checkers; medians
of user-space cycles, instructions and wall time (B relative to A):

| project | cycles, one thread | cycles, default | wall, one thread | wall, default |
| --- | ---: | ---: | ---: | ---: |
| vscode | +0.49% | +0.53% | +0.55% | +0.15% |
| mui-docs | +0.43% | +0.50% | +0.53% | +0.44% |
| webpack | +0.41% | +0.67% | +0.19% | +0.07% |
| xstate-main | +0.52% | +0.44% | +0.41% | +0.95% |
| Compiler | +0.01% | +0.16% | -0.65% | -0.42% |

Instructions were unchanged (within 0.03%). The grouped binary is about 0.5% slower in cycles on every project
large enough to measure. A guess at why: PGO's "hot" set is 3.7 MB, too broad to fit the caches, and grouping it
moves hot code away from the warm code it calls. Rejected.

## Identifiers that carry their hash

oxc's `Ident` stores a precomputed hash so its maps never rehash a name. The question was what share of tsrs's
instructions string hashing takes. Method: callgrind on a `--release` build, one thread, self cost grouped by the
source file of the inlined code:

| | xstate-main | webpack | Compiler |
| --- | ---: | ---: | ---: |
| total instructions | 7.45 G | 14.97 G | 2.48 G |
| Fx hashing (rustc-hash, every key type) | 1.42% | 1.27% | 0.68% |
| hashbrown probing and rehashing | 4.65% | 4.21% | 2.90% |
| memcmp / bcmp | 0.52% | 0.75% | 0.41% |

The largest hashing sites are name lookups: `get_property_of_type_worker` (0.45% / 0.17% / 0.13%),
`SymbolMap::insert` (0.19% / 0.22% / 0.07%) and `get_symbol` (up to 0.14%). A precomputed hash removes only the
hashing part, about 1% at most, while every stored name grows by a hash field. Not adopted; if name lookups grow,
`get_property_of_type_worker` is the place to look first.
