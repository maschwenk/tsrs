# checker-11 notes (checker_11.rs = checker.go 21764–23953)

Status (2026-10-10): condensed to the one deviation that checker_11.rs cites (`fill_missing_type_arguments`); the
other port-wave items were minor nil-handling translations that are visible where they occur. The rest was a
port-wave handoff; its signature requests are done
(`instantiate_type_alias` takes and returns `Option`, `TypeNodeLinks.outer_type_parameters` is
`Cell<Option<&'static [P<Type>]>>`), and `resolved_type_arguments` / `resolved_properties` are now
`OptionThinSliceCell` (types.rs), so an empty result is no longer read as "not computed".

## `fill_missing_type_arguments`: mapper snapshot

Go passes the `result` slice itself to `newTypeMapper(typeParameters, result)` and keeps writing `result[j]` for later
`j`, so lazily-evaluated instantiations (and `compareTypeMappers` on Array mappers) see the final values. The arena
mapper takes a snapshot (`alloc_slice(&result)`) at each iteration. Only observable for forward references in type
parameter defaults (already an error) or ordering ties; faithful emulation needs a mutable-target mapper.
