# perf-shared-tuple-elements: one set of element type parameters for all tuple targets

type-fest used 983 MiB at one checker where bun check uses 468 MiB (notes/perf-excalidraw-typefest.md). The largest
item was tuple targets. Each target (one per arity, element flags, readonliness and labels) got a fresh type parameter
and a property name string for every element, as `createTupleTargetType` does in Go. At one checker, type-fest built
7,543 targets with 4.58M elements, which made 4.6M type parameters (245 MB) and 3.8M index-name strings. Bun shares
the element type parameters across all targets.

This change does the same within a checker. `Checker::tuple_elements` holds, for each index `i`, one type parameter
and the property name `"i"`, created the first time a target needs index `i`. Every target uses them:

- its `all_type_parameters` list is `[E0, ..., En-1, this]`, and its `this` type parameter stays its own
- its element property symbols stay its own too, with `resolved_type` set to the shared `Ei`

## Result (Mac)

type-fest, interleaved medians:

| checkers | peak base / new (MiB) | peak | instructions base / new (G) | instructions | n |
| --- | --- | ---: | --- | ---: | ---: |
| single-threaded | 976 / 693 | -29.0% | 47.31 / 46.05 | -2.7% | 2 |
| 1 | 983 / 701 | -28.7% | 47.50 / 46.22 | -2.7% | 3 |
| 8 (default on the 16-vCPU scoreboard) | 1,714 / 1,182 | -31.0% | 61.59 / 59.09 | -4.1% | 5 |
| 16 | 1,964 / 1,341 | -31.7% | 63.70 / 60.82 | -4.5% | 3 |

`Types:` falls from 5,101,743 to 520,592 at one checker.

The 15 bench projects that could be set up show no change beyond noise:

- At 8 checkers (3 reps), peak moved by -1.3% to +0.4% and instructions by -2.0% to +0.8%.
- The projects are vscode, xstate-main, webpack, mui-docs, Compiler, Compiler-Unions, cal-diy, formbricks-web,
  supabase-studio, t3code-server, next-packages-next, next-root, nuxt, playwright and drizzle-orm. arktype, zod and
  rxjs from #206 moved by less than 1% as well.
- mikro-orm and storybook could not be measured: their yarn 4 install fails in the link step on this Mac (ENOENT while
  persisting `wrap-ansi`), even with a private cache.
- Single-threaded instructions on these projects carry kernel noise of up to ±5% between runs: next-root ranges 19.06 to
  20.10 G on one binary. So no gain is claimed for them.

Linux was not measured.

## Exactness: the audit

### Who reads a target's own type parameters

| reader | uses the parameters through |
| --- | --- |
| `create_tuple_target_type` | builds the list, the element symbols' `resolved_type`, and `resolved_type_arguments` (the target as a reference to itself) |
| `resolve_type_reference_members` (checker_09.rs `get_reference_member_type_arguments`) and the lazy tuple member tables (`create_lazy_member_table`) | a mapper from the target's own list to the reference's arguments plus `this` |
| `get_tuple_base_type` | `Array<E0 \| ... \| En-1>` (an indexed access `Ei[number]` for variadic elements), instantiated by the same mappers |
| `get_variances` | never: tuples take `array_variances` |
| `create_lazy_member_table`'s "arguments are the target's own parameters" test | per target |
| `nodebuilderimpl_2.rs` type-argument printing | the count only, which does not change |

Two of these readers produce types that more than one target now shares:

- **The base type.** Targets of one arity and one variadic pattern now share the `Array<E0 | ... | En-1>` object. Each
  reference still instantiates it through its own mapper.
- **The member union.** A union of symbol-less type parameters is ordered by type id. Shared parameters are created in
  index order, so `E0 | ... | En-1` keeps its order.

### Where a shared parameter could be observed outside a mapper

A local build tagged every element parameter with its target. It logged each place where one could be observed
outside its own target's mappers:

- a mapper that would capture another target's parameter of the same index
- a mapper whose sources mix two targets
- an element parameter reaching `is_related_to`, `infer_from_types` or the node builder
- a union or intersection that mixes targets, or that mixes element parameters with other symbol-less type parameters
- an inference context over element parameters
- a type reference taking another target's raw element parameter as an argument

The build ran over the conformance suite (default and `TSRS_LAZY_MEMBERS=0`), fourslash, type-fest, excalidraw and
the eight cached bench projects. It logged none of these events, except one kind.

That kind was the type-reference argument. `Array<E0>` and `ReadonlyArray<E0>` appeared 54-76 times per project:
`get_tuple_base_type`'s union of a single element is the element itself, so that is the base type of a one-element
target. A sanity counter confirmed that the build did see element parameters pass through mappers.

### The element symbols cannot be shared

`create_union_or_intersection_property` (checker_11.rs) treats two constituent properties whose
`get_target_symbol` is the same symbol as instantiations of one declaration. If their types are equal, it reuses the
first property instead of building a synthetic union property.

If the element symbols were shared per (index, optional, readonly), property `0` of `[string] | [string, number]`
would take that path, where Go builds a union property from two different symbols. So the per-element property
symbols, their value links, and the `TupleElementInfo` lists stay per target. They are most of what is left between
tsrs (701 MiB) and bun (468 MiB) on type-fest at one checker. The index names are compared by value only, so sharing
them is exact.

## Gates

- **Conformance**, `--baselines types,symbols`: 13,458 pass / 2 codes / 2 fail; 12,779 types; 12,779 symbols. The
  result trees are identical to main in four modes: default, `TSRS_LAZY_MEMBERS=0`, `TSRS_HISTORY=canonical`, and
  canonical with 4 checkers per test.
- **js / jsmap / sourcemap baselines**: 13,392 / 149 / 156, with trees identical to main.
- **Fourslash**: 4,066 pass / 63 fail, with a result tree identical to main.
- **Diagnostics are byte-identical to main** on these projects:
  - at 1, 4 and 16 checkers and under `--checkerAssignment go` at 4: type-fest, excalidraw, t3code-server,
    formbricks-web, cal-diy, supabase-studio, vscode, webpack, xstate and mui-docs
  - at 1, 4 and 16 checkers under the locality assignment, single-threaded, and in Go mode: rxjs (6,158 errors), zod
    (31) and arktype
- **Determinism**: `tools/ci/determinism.sh` on xstate-main and webpack plus the regression cases compared 504 runs,
  all identical.
- **Arena poison**: `TSRS_ARENA_POISON=1 TSRS_FREE_LEAVES=1` output equals main on webpack and excalidraw.
- **Other checks**:
  - `tools/regressions.sh` passes 26 of 26.
  - `cargo test -p tsrs_cli` passes, with `api::memory_tests` cfg'd out locally.
  - `RUSTFLAGS="-D warnings" cargo check --workspace --locked` is clean.
  - The lint ratchet and `tools/lint/source.py` are ok.

## Reproduce

```sh
git checkout origin/bench/bun-pr-projects -- bench/projects.json bench/run.py bench/overlays/type-fest  # not committed
python3 bench/run.py --setup-only --projects type-fest
cd bench/.work/solutions/type-fest
/usr/bin/time -l tsrs -p . --noEmit --incremental false --pretty false --extendedDiagnostics --checkers 8
```

The audit build was local and is not committed. It is about 120 lines of hooks in `TypeMapper::map`,
`is_related_to_ex`, `infer_from_types`, `type_to_type_node`, `get_union_type_ex`, `get_intersection_type_ex` and
`create_type_reference_ex`, keyed by a side table filled in `create_tuple_target_type`.
