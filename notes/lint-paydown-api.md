# lint-paydown-api: the lint ratchet in the Node API crates

The Node API crates (`tsrs_api`, `tsrs_api_codec`, `tsrs_api_transport`) held 195 of the 312 baseline findings;
notes/lint-paydown-project.md left them alone while their agent was active. That agent finished with the 0.3.0 release
and is paused. 192 are cleared and 3 are left; the baseline goes from 312 to 120. Same method as
notes/lint-paydown-project.md: fixes that keep behavior, `#[expect]` only with a reason that names the constraint.

| Lint | Before | After | How |
| --- | ---: | ---: | --- |
| `needless_pass_by_value` | 53 | 0 | 42 handlers take `wire::Params`, a one-field wrapper around `&Value`: it now derives `Copy`, and its accessors take `self`. The four fieldless query enums in `handlers_symbols.rs` derive `Copy`; `present<T>` requires `T: Copy` (every caller passes an `Option` or a `P`). The transport borrows each `Message` and `Result<Response, ResponseError>` instead of moving it in. |
| `clone_on_ref_ptr` | 52 | 0 | `Arc::clone(&x)` / `Rc::clone` / `Weak::clone`; three sites that coerce to `Arc<dyn …>` spell the concrete type. |
| `disallowed_types` | 37 | 0 | std `HashMap` -> `rustc_hash::FxHashMap` in seven files. All are internal registries and lookups; none is serialized. |
| `iter_over_hash_type` | 8 | 3 | 5 `#[expect]` (below), 3 left. |
| `redundant_clone`, `implicit_clone`, `assigning_clones` | 18 | 0 | Dropped or `clone_from`. |
| `borrow_as_ptr`, `ref_as_ptr` | 11 | 0 | `&raw const`, `std::ptr::from_ref`. |
| `trivially_copy_pass_by_ref` | 3 | 0 | By value; `compare_file_symbols` keeps `&` with `#[expect]` because `sort_by` passes references. |
| `format_push_string` | 4 | 0 | `write!`. |
| `significant_drop_in_scrutinee` | 3 | 0 | The value is taken out of the mutex before the `match` / `if let`, so a removed orchestrator or resolver is dropped outside the lock. |
| `undocumented_unsafe_blocks` | 2 | 0 | Both `&'static SourceFile` extensions in `sourcefiles.rs` state the invariant: the file outlives the request, and the cached table is evicted when its arena region is freed (`store_table`). |
| `allow_attributes` + `_without_reason` | 2 | 0 | The unused `_assert_sig` placeholder is deleted. |
| `disallowed_methods` | 1 | 0 | `#[expect]`: the decoder's `from_utf8_unchecked` is required, not a speedup. Strings are WTF-8 (JS lone surrogates), which the checked `from_utf8` rejects. `is_wtf8` checks the bytes first. |
| `unnecessary_box_returns` | 1 | 0 | `descriptor_normalized` returns `Value`. |

## iter_over_hash_type

Matched to `ts-ref/tsc/internal/api/requestfilesystem` and the snapshot code. Each `#[expect]` marks a loop where Go
ranges over a map or set too and the order cannot reach output: `entries` (both lists are sorted right after, as
Go sorts them), the child-insert loop in `compose`, `overlays` (builds a map), `expand_file_changes` (adds to the same
set), and the snapshot change lists (sorted before they are serialized).

Left, because the order can reach output (Go is random there as well):

- `compose`'s merge loop appends overlay names to an unsorted directory listing.
- `add_symlink_entries` appends symlinks to the listing in iteration order.
- `walk_symlinks` feeds `aliases_for_path`, whose callers return the aliases in that order.

## Gates

- `cargo check --workspace` with `-D warnings` on 1.99 and 1.95; the ratchet on macOS (120) and Linux (118, `rust:1.99`).
- The three crates' tests on macOS: 119 pass. One fails on main too: `pinned_node_clients_round_trip` expects
  `/var/folders/…` and gets `/private/var/folders/…` (macOS `/var` is a symlink). `tests/memory_test.rs` calls glibc's
  `malloc_trim` and does not link on macOS.
- The Node API parity suites and Linux packages (Depot) and the macOS package (GitHub) on the pull request.
