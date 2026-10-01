# lazy-members: microsoft/TypeScript#64475 + #64526 in tsrs

Port of Max's two upstream PRs that make member resolution lazier:

- #64475 "build member tables of instantiated classes/interfaces lazily": an instantiated class/interface
  reference gets a `lazyMemberTable` (mapper, instantiated signatures/index infos, instantiated base types, the
  sorted names of declared members that instantiate to themselves) instead of an instantiated member table; member
  lookups (`getPropertyOfTypeEx`), signature/index-info queries, `getSingleSignature`, `isWeakType`,
  `isStringIndexSignatureOnlyTypeWorker` and `isFunctionObjectType` answer from it; members are instantiated one at a
  time and reused if the type is later resolved in full.
- #64526 "build members of keyof mapped types lazily" (stacked): `{ [P in keyof T]: X }` over an object type gets a
  `lazyMappedTable` that creates the member being looked up and the index infos on demand; mapped types answer "no
  signatures" without resolving; `somePropertyReducesToNever` looks members of one lazy constituent up by name
  instead of listing them.

**On by default.** `--noLazyMembers` (CLI) or `TSRS_LAZY_MEMBERS=0` (any binary: `tsrs`, `tsrs-test`) turns it off;
the opt-out mode is reference-identical (suite and counters, below). `--lazyMembers` / `TSRS_LAZY_MEMBERS=1` force it
on. With `--extendedDiagnostics` and the flag on, `tsrs` prints a second table with per-path counts.
Follow-up candidates (each behind its own switch, effective only with this flag on): notes/mem-lazy.md.

## Where it lives

- `tsrs_core::lazymembers`: the flag (`enabled()`, `set_from_cli`) and `LazyMemberStats`.
- `Checker::lazy_members` (read once in `new_checker`), `lazy_member_tables`, `lazy_mapped_tables`,
  `lazy_member_stats`.
- checker_09.rs: `get_property_of_type_ex`, `signatures_of_structured_type`, `index_infos_of_structured_type`,
  `resolve_type_reference_members`, `resolve_object_type_members`, and the #64475 block after `find_index_info`
  (`LazyMemberTable`, `get_ready_lazy_member_table[_worker]`, `prepare_lazy_members`, `resolve_lazy_members`,
  `get_lazy_declared_member`, `get_member_of_[unresolved_]structured_type`, `every_property_of_structured_type`,
  `has_properties_of_structured_type`, `every_lazy_property`, `append_inherited_signatures_and_index_infos`).
- checker_10.rs: `get_single_signature`, `instantiate_symbol` = `is_symbol_unaffected_by_instantiation` +
  `new_instantiated_symbol`, `resolve_mapped_type_members` (+ `append_mapped_type_index_info`,
  `new_mapped_type_member`, `LazyMappedTable`, `get_lazy_mapped_table`, `get_lazy_mapped_type_member`,
  `get_lazy_mapped_type_index_infos`).
- checker_11.rs `some_property_reduces_to_never`, relater_1.rs `is_weak_type`, checker_13.rs
  `is_string_index_signature_only_type_worker`, checker_15.rs `is_function_object_type`.

The flag is read only where a lazy table would be created (`get_ready_lazy_member_table`, `get_lazy_mapped_table`)
and at the two #64526 sites that change work without a table (the mapped-type early return in
`signatures_of_structured_type`, the skipped constituent in `some_property_reduces_to_never`). Everything else in
the diffs (the refactored `get_single_signature`, `is_weak_type`, `is_function_object_type`,
`is_string_index_signature_only_type_worker`, `get_property_of_type_ex`, the `instantiate_symbol` split, the
factored `resolve_mapped_type_members`) runs in both modes, and the opt-out mode is still exactly
reference-identical: **without tables the refactors are behavior- and work-neutral** (one caveat for upstream:
`isStringIndexSignatureOnlyTypeWorker` now uses `hasPropertiesOfStructuredType(t)` instead of
`len(getPropertiesOfType(t))`, which skips `getReducedApparentType`, i.e. `getApparentTypeOfMappedType` for
non-generic mapped types; no observable difference anywhere below).

## Deviations from the Go diffs

- `somePropertyReducesToNever` (#64526): ported onto the merged #64521 version (`OrderedMap` counts, iterated in
  discovery order). The PR as posted iterates a Go map there (`for propName := range counts`, both for the skipped
  constituent's lookups and the final check), which brings back the random creation order of combined properties
  that #64521 removed. Upstream should rebase that hunk onto #64521 the same way (`counts.Entries()` for both
  loops); the Go build used as oracle below (`tsgo-pr-b`) has exactly that change.
- Go's `getSignaturesOfStructuredType` / `getIndexInfosOfStructuredType` return the stored slice; the Rust port's
  versions return a `Vec` copy, so the new call sites use non-copying `signatures_of_structured_type` /
  `index_infos_of_structured_type` (the `Vec` versions now delegate to them).
- `lazyMemberTable.ready` plus the fields `prepareLazyMembers` fills (`unaffected`, signatures, index infos, base
  types) are one `OnceCell<LazyMembers>`: Go assigns them together immediately before `ready = true` and nothing
  reads them earlier. The tables live in `Rc`s in the checker's maps, so dropping a table frees it like Go's
  `delete` + GC; the signature/index-info slices are arena-allocated (they become the resolved members' slices).
- Go iterates `declaredMembers` (a map: random order) in `prepareLazyMembers`, `resolveLazyMembers` and
  `everyLazyProperty`; tsrs's `SymbolTable` is insertion-ordered, so these are deterministic here.
- `lazyMappedTable.members` (a SymbolTable holding nil entries for "being created / no member") is a
  `FxHashMap<String, Option<P<Symbol>>>`; `getLazyMappedTypeMember`'s `(member, ok)` is `Option<Option<P<Symbol>>>`.

## Evidence

Machine: 18 cores, 128 GB, other load ~6 (desktop apps). Project = the pristine read-only checkout
(37,942 files). Every run: 0 errors. Medians of 3 back-to-back runs (off/on interleaved per round); peak = `peak
memory footprint` from `/usr/bin/time -l`; check/total from `--extendedDiagnostics`, wall = `real`. Go runs use
`--incremental false` (see DEBUGGING.md). `tsgo-pr-a` / `tsgo-pr-b` are tsgo at b85298b6 with #64475 / #64475+#64526
applied (the latter with the ordered `somePropertyReducesToNever`, above).

### Port faithfulness: counters equal tsgo with the PRs

| | symbols | types | instantiations |
| --- | --- | --- | --- |
| tsgo-ref, tsrs `--noLazyMembers`, single | 25,973,354 | 9,639,962 | 44,884,281 |
| tsgo-pr-a, tsrs #64475 on, single | 17,231,200 | 9,639,962 | 44,884,281 |
| tsgo-pr-b, tsrs #64475+#64526 on, single | 15,331,397 | 9,630,120 | 44,820,708 |
| tsgo-ref, tsrs `--noLazyMembers`, 4 checkers | 39,704,001 | 16,200,921 | 89,981,648 |
| tsgo-pr-a, tsrs #64475 on, 4 checkers | 25,168,912 | 16,200,921 | 89,981,648 |
| tsgo-pr-b, tsrs #64475+#64526 on, 4 checkers | 22,813,093 | 16,186,849 | 89,882,712 |

Each row's Go and tsrs numbers are identical. Same on the PRs' own tests (`instantiatedReferenceLazyMembers`,
`mappedTypeLazyMembers`: tsrs = tsgo-pr-b = 59,323 / 59,088 symbols), and both tests' upstream baselines
(errors, `.types`, `.symbols`) pass in both modes.

### tsrs, flag off vs on

| run | symbols | types | instantiations | check s | total s | wall s | peak GB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| single, off | 25,973,354 | 9,639,962 | 44,884,281 | 26.32 | 32.27 | 32.53 | 15.34 |
| single, #64475 | 17,231,200 | 9,639,962 | 44,884,281 | 24.98 | 28.96 | 29.24 | 12.78 |
| single, #64475+#64526 | 15,331,397 | 9,630,120 | 44,820,708 | 22.90 | 26.63 | 26.88 | 11.77 |
| 4 checkers, off | 39,704,001 | 16,200,921 | 89,981,648 | 13.22 | 15.20 | 15.61 | 23.32 |
| 4 checkers, #64475 | 25,168,912 | 16,200,921 | 89,981,648 | 11.32 | 13.57 | 13.88 | 18.41 |
| 4 checkers, #64475+#64526 | 22,813,093 | 16,186,849 | 89,882,712 | 11.08 | 13.50 | 13.83 | 17.49 |

(#64475-only rows: build of commit 1, flag on vs its own flag-off runs, which medianed 26.74 s / 15.34 GB single and
13.79 s / 23.29 GB on 4 checkers.) Peak: -17% / -21% from #64475, -23% / -25% with both. Check time: -7% / -18%
from #64475, -13% / -16% with both (single-threaded timings vary by +-2 s on this machine).

### Go, reference vs PRs (same machine, same protocol)

| run | check s | total s | wall s | peak GB |
| --- | --- | --- | --- | --- |
| tsgo-ref, single | 35.09 | 40.53 | 42.46 | 17.76 |
| tsgo-pr-a, single | 37.12 | 43.30 | 45.27 | 15.06 |
| tsgo-pr-b, single | 35.06 | 39.95 | 41.66 | 13.67 |
| tsgo-ref, 4 checkers | 20.50 | 22.09 | 25.32 | 26.14 |
| tsgo-pr-a, 4 checkers | 19.93 | 21.52 | 24.24 | 21.70 |
| tsgo-pr-b, 4 checkers | 18.87 | 20.30 | 23.14 | 19.46 |

Go peak: -15% / -17% from #64475, -23% / -26% with both, the same shape as tsrs and as Max's 37k-file numbers
(19.1 -> 15.3 GiB, -20%, 4 checkers). Go check time on 4 checkers: -3% / -8%; single-threaded #64475 alone was not
faster in these three runs (GC work is a larger share there; noisy).

### Per-path counts (tsrs, Project, both PRs; single / 4 checkers)

| counter | single | 4 checkers |
| --- | --- | --- |
| lazy member tables created | 881,767 | 1,574,463 |
| ... later resolved in full | 124,568 (14%) | 226,971 (14%) |
| declared named members in those tables | 12,020,236 | 20,687,284 |
| ... instantiated (lazily or at full resolution) | 2,064,231 (17%) | 3,925,593 (19%) |
| member lookups answered from a table | 7,162,222 | 14,327,096 |
| signature / index-info / every-property queries answered | 3,993,370 / 1,449,391 / 967,464 | 8,030,209 / 2,619,893 / 2,120,167 |
| lazy mapped tables created | 40,110 | 53,813 |
| ... later resolved in full | 21,353 (53%) | 31,767 (59%) |
| mapped members created lazily / lookups answered | 68,677 / 493,902 | 107,307 / 610,160 |
| mapped index-info computations | 35,953 | 47,204 |
| mapped "no signatures" early returns on unresolved types | 291,675 | 359,575 |
| `somePropertyReducesToNever` skipped constituents | 93,194 | 159,852 |

So 86% of instantiated references never get a member table and 83% of their declared members are never
instantiated; about half of the `keyof` mapped types are still resolved in full eventually.

### Conformance

Opt-out mode (`TSRS_LAZY_MEMBERS=0`): pass lists (errors 13,458 / `.types` 12,779 / `.symbols` 12,779) and all
non-pass artifacts byte-identical to main. Default mode (both PRs): **no variant changes** — same pass lists, and the
4 non-passing variants produce identical output; also with `TS_TEST_PROGRAM_SINGLE_THREADED=false`. No union-order
or text drift surfaced in the suite.
