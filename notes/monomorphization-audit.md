# monomorphization-audit: where the checker's generic code goes

Question (docs/RUST.md, "Monomorphization audit"; from rust-analyzer's style guide and oxc's `cargo llvm-lines`
workflow): does the port's `impl FnMut` callback convention (PORTING.md, "Function signatures") compile so many copies
that a thin generic shim over a non-generic `&mut dyn FnMut` body would pay off? Answer: no. Generic copies are
mostly std iterator, `collect` and hashbrown code; the port's own closures are 3.6% of the crate's LLVM IR, and the
largest generic tsrs function is 1.1% of it (1.3% with its closures) and sits on a hot path. Nothing is changed.

## Method

`cargo llvm-lines --release -p tsrs_checker --lib` (cargo-llvm-lines 0.4.48, main `25274c1`, 2026-10-05). It counts the
LLVM IR lines of every function instance before optimization: a proxy for codegen time and code size, not runtime.

## Results

The crate: 860,924 lines in 18,163 function copies of 7,149 distinct functions. tsrs's own functions are 513,847 lines
(59.7%); their closures 30,938 lines (3.6%) in 749 entries.

Largest generic functions by total lines:

| function | copies | lines | share |
| --- | ---: | ---: | ---: |
| `Vec::from_iter` (`collect`) | 249 | 23,539 | 2.7% |
| `FnOnce::call_once` | 529 | 13,183 | 1.5% |
| `slice::Iter::fold` | 109 | 11,874 | 1.4% |
| `Iterator::try_fold` | 169 | 11,257 | 1.3% |
| `Vec::extend_desugared` | 128 | 11,121 | 1.3% |
| `LocalKey::try_with` (`thread_local!` access) | 163 | 10,265 | 1.2% |
| `Checker::filter_type` | 44 | 9,674 | 1.1% |
| hashbrown `find_or_find_insert_index` / `find` / `insert` | 108 / 112 / 79 | 23,067 | 2.7% |
| `tsrs_core::ptr::with_arena` | 160 | 6,801 | 0.8% |

Largest tsrs generics: `filter_type` (44 copies, 9,674 lines, plus 1,892 lines in 132 closure copies),
`with_arena` (160 copies), `p_layout` (89), `P::new`'s closure (80), `Arena::track_drop` (122),
`Arena::alloc_with` (89), `LinkStore::get` (25 copies, 3,550 lines), `find_ancestor` (37), `every_type` (25),
`some_type` (24), `NodeBuilderImpl::visit_and_transform_type` (5 copies, 4,900 lines).

## Why nothing changes

- The arena helpers (`with_arena`, `p_layout`, `alloc_with`, `track_drop`, `P::new`) are generic over the allocated
  type and have small bodies; they are the allocation fast path, where a copy per type is the point.
- `filter_type` is the one candidate for the shim pattern (each of its 39 call sites passes a different predicate).
  It runs during narrowing; a `&mut dyn FnMut` body would add an indirect call per union member to save about 1% of
  one crate's IR. Not worth it without a measured payoff, and the bench's instruction counts would show any cost.
- The std and hashbrown copies come from ordinary iterator and map use; there is no single place to cut them.

To repeat: `cargo install cargo-llvm-lines`, then the command above; `| head -40` shows the largest entries.
