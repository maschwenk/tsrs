# Rust 1.99 MSRV and release audit

Date: 2026-10-11

## Scope

`rustup check` reports Rust 1.99.0 (`b940084d7`, released 2026-10-01) as the current stable toolchain. The repository
already pinned that compiler in `rust-toolchain.toml`, but none of its Cargo packages declared an MSRV. The workspace
now declares `rust-version = "1.99"` and every member inherits it, so Cargo and published package metadata agree with
the build, test, bench and release toolchain.

This audit covers the stable releases in the preceding 12 months: [1.91.0](https://doc.rust-lang.org/releases.html#version-1910-2025-10-30),
[1.92.0](https://doc.rust-lang.org/releases.html#version-1920-2025-12-11),
[1.93.0](https://doc.rust-lang.org/releases.html#version-1930-2026-01-22),
[1.94.0](https://doc.rust-lang.org/releases.html#version-1940-2026-03-05),
[1.95.0](https://doc.rust-lang.org/releases.html#version-1950-2026-04-16),
[1.96.0](https://doc.rust-lang.org/releases.html#version-1960-2026-05-28),
[1.97.0](https://doc.rust-lang.org/releases.html#version-1970-2026-07-09),
[1.98.0](https://doc.rust-lang.org/releases.html#version-1980-2026-08-20) and
[1.99.0](https://doc.rust-lang.org/releases.html#version-1990-2026-10-01), including their patch releases.

## Adopted

- Rust 1.95's `Atomic::try_update` replaces the three remaining hand-written compare-exchange loops: source-text
  registration, snapshot `try_ref`, and the compiler's file queue. The closures are side-effect-free and keep the
  same ordering and stop conditions.
- Rust 1.97's integer `bit_width` replaces five `BITS - leading_zeros()` spellings. This also more directly expresses
  the Go `bits.Len` operation used by the port.
- Rust 1.97's Cargo `build.warnings` setting replaces `RUSTFLAGS=-D warnings` in CI. It denies warnings in local
  packages without making dependency artifacts depend on a global `RUSTFLAGS` value.
- Rust 1.98's `bool::ok_or_else` and `str::strip_circumfix` replace matching manual conversions where overlap was
  already rejected.
- Rust 1.99's `raw_borrows_via_references` lint is enabled workspace-wide. A non-test workspace check had no
  findings; three test-only pointer identity helpers now form raw borrows directly rather than through references.
- Cargo 1.99 disables incremental compilation when `CI` is set, so the redundant `CARGO_INCREMENTAL=0` declarations
  were removed from the GitHub and Depot workflows.

Rust 1.91's `PanicHookInfo::payload_as_str` was already used by the test runner. No further compatible hook sites
needed it.

## Reviewed but not changed

- Rust 1.98's endian-aware UTF-16 decoders do not preserve the current odd-byte behavior: the port's `chunks_exact`
  paths ignore a trailing byte, while the new lossy APIs emit a replacement character. Changing that requires an
  upstream-fidelity decision and dedicated tests.
- Rust 1.99's `String::from_utf8_lossy_owned` could replace some borrowed conversion plus `into_owned` call sites, but
  this repository first validates through `simdutf8`, and the standard API does not guarantee reuse of the input
  allocation. That is a separate measured allocation experiment, not part of an MSRV metadata change.
- The new `Vec`/`String` raw-parts and `Box`/`NonNull` APIs do not match the arena ownership transfers here. The
  arena's `ManuallyDrop<Vec<T>>` path moves the elements into different storage before freeing the old allocation.
- `array_windows`, slice-to-array accessors, `Vec::push_mut`, `cold_path`, `cfg_select!`, path extension helpers,
  zeroed `Box`/`Rc`/`Arc`, and `VecDeque` conditional-pop/retention APIs had no production call site that became
  clearer without changing Go-shaped control flow or introducing an unmeasured hot-path change.
- The new Cargo `debug` profile and edition-2024 workspace dependency feature override do not replace any current
  configuration: the workspace has an intentional custom `dev` profile and no member overrides an inherited
  dependency's default features.

No performance claim is made for this change; it updates metadata, safety policy, CI configuration and equivalent
standard-library spellings.
