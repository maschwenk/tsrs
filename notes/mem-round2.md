# mem-round2: representation changes for peak memory (after mem-layout and mem-lazy)

Same rules as notes/mem-layout.md: only layout / representation changes, no checker semantics. Gates for every
step: the suite (errors and `--baselines types,symbols`) byte-identical to the base commit in the default mode and
with `TSRS_LAZY_MEMBERS=0`; the `--extendedDiagnostics` counters unchanged in both modes at 1 and 4 checkers; the
private monorepo 0 errors.

Measurement: `/usr/bin/time -l` on the pristine private-monorepo checkout, interleaved rounds, "GiB" = peak memory
footprint / 2^30. The machine was shared with other agents (load 8 to 70), so check times are noisy; instructions
retired are the stable CPU-work measure.

Status (2026-10-10): the sizes below are as of 263f98b; most structs have shrunk since (notes/mem-round3.md,
mem-layout3.md, mem-small.md, mem-pointer-compression.md). One rejection here is superseded: recycling
`InferenceContext`s landed in notes/mem-recycle.md step 2, with an escape bit that marks contexts whose mappers were
kept (`InferenceContext::recycle`, crates/tsrs_checker/src/checker.rs). The per-step tables and the before/after
profiles were removed (git history has them).

## Steps (single-threaded / 4-checker peak, GiB, default mode)

| step | change | single | 4 checkers |
| --- | --- | --- | --- |
| 1 | `Symbol` 72 -> 64 (`name`, `declarations` as 12-byte packed cells) | 8.133 -> 8.040 | 10.92-10.97 -> 10.79-10.84 |
| 2 | `SymbolTable` entries 24 -> 16 (key read from the symbol) | 8.037-8.040 -> 7.819 | 10.80-10.85 -> 10.52-10.56 |
| 3 | `TypeMapper` 24 -> 16 (two words, kind in the low bits) | 7.818-7.819 -> 7.690-7.693 | 10.52-10.57 -> 10.35-10.36 |
| 4 | lazy member tables: shared type-argument list, slices, a `SymbolTable` for instantiated members | 7.693 -> 7.603 | 10.32-10.35 -> 10.19-10.21 |
| 5 | `InferenceContext` 128 -> 64, `InferenceInfo` 96 -> 48 (rare fields in a tail) | 7.603 -> 7.447 | 10.19 -> 9.98-10.00 |
| 6 | `StructuredType` 64 -> 56 (abstract-construct-signature cache moved to a checker map) | 7.447 -> 7.390 | 9.98-9.99 -> 9.91-9.92 |
| 7 | `StructuredType` 56 -> 48 (base constraints in a checker map) | 7.390-7.391 -> 7.344 | 9.90-9.91 -> 9.83-9.85 |
| 8 | symbol tables searched linearly up to 16 entries (32: +0.6% instructions on 4 checkers for 0.03 GiB more) | 7.344 -> 7.316 | 9.84-9.85 -> 9.81 |
| 9 | identifier and literal text in 8 bytes (`PackedStr`) | 7.357-7.359 -> 7.261-7.264 | 9.84-9.87 -> 9.76 |
| 10 | `Symbol` 64 -> 56 (name as `PackedStr`, 32-bit id) | 7.327-7.337 -> 7.166-7.168 | 9.77-9.86 -> 9.60 |
| 11 | instantiation caches only on instantiation targets | 7.162-7.169 -> 7.130 | 9.59-9.68 -> 9.55-9.58 |
| 12 | union / intersection packed slice cells, packed key-property name | 7.124-7.126 -> 7.102-7.106 | 9.55-9.58 -> 9.51-9.52 |
| 13 | relation cache slots 20 -> 17 bytes | 7.101-7.102 -> 7.075-7.076 | 9.50-9.56 -> 9.45-9.54 |
| 14 | `Signature` 104 -> 88 (rare fields in a tail) | 7.078-7.079 -> 7.049-7.057 | 9.46-9.52 -> 9.42-9.45 |
| 15 | emit-context entries 160 -> 32 bytes (rare fields boxed) | 7.055-7.056 -> 7.021 | 9.44-9.49 -> 9.38-9.40 |

Each row compares against the binary of the step before; the bases of steps 9 and 10 include upstream compiler
commits that moved the single-threaded peak up. Most of the pass's +2% instructions is step 2 (hashing in
symbol-table lookups: single 315-319 -> 320-325 G).

### Symbol-table keys taken from the symbol (step 2)

Instrumented once: of 16.7M new symbol-table entries on the private monorepo single, 16,684,348 store the symbol's own
name string as the key, 4,399 an equal string at another address, 1 a different text. Symbol names never change after
`Symbol::new`. So an entry is (symbol, `u32` hash of the key, `u32` key length) and the key is read from the symbol;
a key with other text than its symbol's name goes to a side list (`odd_keys`). Lookups compare length and hash before
reading the symbol. Iteration order, `set`-keeps-key and `delete` shifting are unchanged. A variant that stored the
key's first four bytes instead of a hash retired more instructions (more symbols read and compared), so the hash
stayed.

### Global identifier interner (step 9, rejected)

The brief's version of step 9, a global interner (sharded mutex tables of length-prefixed copies, 8-byte pointers to
them), was built first: -0.075 GiB single, but parse time +9% (2.6 -> 2.8 s) and +1% instructions from hashing
and locking 7.8M identifiers, and the shared copies eat part of the win. The packed pointer needs no table, no
copies and no hashing.

| run (2 interleaved rounds) | peak GiB | instructions | parse s (3 runs) |
| --- | --- | --- | --- |
| single, before | 7.357-7.359 | 320-321 G | 2.58-2.61 |
| single, after (`PackedStr`) | 7.261-7.264 (-0.10) | 321-323 G | 2.55-2.60 |
| single, global interner instead | 7.283-7.284 | 324 G | 2.79-2.86 |

## Result

Interleaved, 3 rounds each (base = e8d4196, the start of this pass; final = 263f98b; the final binary also contains
concurrent upstream frontend commits, which on their own moved the single-threaded peak up by ~0.1 GiB):

| run | base peak GiB | final peak GiB | base check s | final check s | instructions |
| --- | --- | --- | --- | --- | --- |
| default, single | 8.133 | 7.020 (-13.7%) | 17.37-17.62 | 17.58-18.11 | 315-317 -> 322-324 G |
| default, 4 checkers | 10.91-10.94 | 9.38-9.43 (-14.2%) | 6.88-6.90 | 6.96-7.19 | 424-425 -> 435-436 G |
| opt-out, single | 11.08 | 9.47 (-14.6%) | | | 343 -> 348 G |
| opt-out, 4 checkers (go assignment) | 17.11 | 14.52 (-15.1%) | | | 507 -> 521 G |

## Rejected and not done

Rejected (brief candidates):

- Mapper interning: mapper identity is observable (`find_active_mapper` compares pointers, `compare_type_mappers`
  short-circuits on identity).
- Type-list interning: instrumented, 10.0M lists / 193 MB through `alloc_slice`, 5.35M / 113 MB distinct, so a
  hash-consing table of the distinct lists would cost more than the 80 MB it saves.
- Recycling `InferenceContext`s: rejected here because a context escapes through its fixing / non-fixing mappers
  into instantiated types. Superseded: notes/mem-recycle.md step 2 recycles them with an escape bit (status line
  above).
- A global identifier interner (section above).

Not done: `SymbolTable` header 32 -> 24 bytes (later done, notes/mem-round3.md A11); pointer-keyed link stores keyed
by id (ids are observable); `ValueSymbolLinks` read-only accesses through `try_get` (~40 MB; needs an audit of every
read site).
