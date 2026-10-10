# Oxc allocator migration

Date: 2026-10-10

## Decision

The arena backend is now `oxc_allocator` 0.153.0. The custom chunk allocator, 32 GiB address reservation,
compressed-pointer build configuration, per-size block recycling, partial bump rewind and explicit huge-page chunk
management were removed. `P<T>` is a native `NonNull<T>` identity pointer, and `Option<P<T>>` uses the pointer niche.

This revisits the earlier rejection of Oxc's compile-time no-`Drop` rule in `docs/RUST.md`. The constraint changed:
resource-owning values no longer need to live directly in Oxc chunks. Fixed values that do not need destruction use
Oxc; a value that needs `Drop` is placed in a heap sidecar registered with the same arena owner. Sidecars are dropped
in reverse allocation order before the Oxc allocator releases its chunks. `SourceFile` demonstrates the intended
split: fixed graph data stays in the arena and locks, maps, parse options, position maps and caches live in its
owner-paired `SourceFileState` sidecar.

The follow-up in `notes/rust-owned-arenas.md` replaces the original unused raw-pointer `arena_owner` API with
safe typed indexed storage. This API uses Rust-owned vectors rather than Oxc allocations:

- `ArenaBuilder<T>` is exclusive allocation state and `seal` removes allocation access.
- `ArenaKey<T>` carries an arena identity and resolves only while its owner is borrowed; `LocalKey<T>` is an
  owner-relative edge. Strings and collections are ordinary owned Rust fields.
- `OwnedRoot`, `OwnedGraph` and `ArenaGroup` keep roots and all contributing sealed owners together.

The existing region, scratch and `P<T>` APIs remain as a compatibility layer while call sites move toward explicit
owners. They still select an Oxc owner. Generic, keyed and specialized checker link tables, the core node-builder
and emit link stores, their map storage and value-symbol rare tails have moved to owned Rust storage
(`notes/rust-owned-links.md`). Shared processed-file and redirect containers and resolution hosts now have Rust
owners (`notes/rust-owned-program-data.md`). Checkers, node-builder hosts and pools retain shared input data
independently of the outer program, and file-list containers use shared Rust arrays
(`notes/rust-owned-checker-inputs.md`). Checker leases and built-in slot arrays now have Rust owners, and project
checkers own their regions directly (`notes/rust-owned-checker-leases.md`). Compiler/incremental roots now use
shared Rust owners and retain their graph regions (`notes/rust-owned-program-roots.md`: fidelity preserved,
instruction/RSS gates fail). Symbol-table buffers and filter/extra records use Rust `Vec`/enum ownership
(`notes/rust-owned-symbol-storage.md`), removing raw container allocation and four unchecked thread traits.
AST/symbol/type/file referents and snapshot caches still use legacy pointers.

## Removed behavior

- `free!`, `free_slice!`, `new_recycled` and their helpers are compatibility no-ops. Oxc releases allocation storage
  only when the owner is dropped.
- Parser checkpoints cannot partially rewind Oxc. `arena_rewindable` returns false, so declaration-file lazy member
  conversion keeps its initial parse rather than creating a lazy record.
- `TSRS_FREE_LEAVES` is disabled. The former reservation guaranteed that a retired address was never reused. Native
  allocation does not, and some external tables still use addresses as keys. Leaf retirement stays off until every
  such entry can be removed before owner teardown.
- The `compressed-ptrs` and `plain-ptrs` Cargo features remain accepted only so old build commands do not fail; both
  select the same native-pointer representation.

## Validation and measurement

The migration is validated with workspace checks, focused core/AST/compiler tests, the alloc-profile feature build,
and the lint/source gates. No instruction, wall-time or peak-RSS measurement was collected because the owner
explicitly requested migration without measurement. Consequently this note makes no performance or memory claim;
the normal benchmark gates must characterize the new baseline before a later optimization relies on it. The
follow-up measurement now does that: default-mode peak RSS is 75–85% above the pre-Oxc build on the two pinned
Compiler workloads. See `notes/rust-owned-arenas.md` for the before/after numbers and limitations.

## Follow-ups

1. Move remaining implicit thread-arena roots to `OwnedRoot`/`OwnedGraph` ownership boundaries.
2. Remove external native-address keys at owner teardown, then reassess leaf-region retirement.
3. Reintroduce speculative parsing only if Oxc gains a suitable stable reset boundary or parsing gets a dedicated
   disposable owner.
4. Delete the compatibility free/recycle/checkpoint APIs once their callers no longer depend on those spellings.
