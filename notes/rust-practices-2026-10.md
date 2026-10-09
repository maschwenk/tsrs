# rust-practices-2026-10: what the most regarded Rust repositories do that tsrs does not

Question: across the Rust projects people point to as well run, which linting, testing and build practices does tsrs
lack, and which of them would make it faster or less sloppy? Answer: on the performance side, none of the build and
toolchain levers those projects pull is left unmeasured here (`docs/RUST.md`, "Techniques"); the gaps are in
hygiene, and they are measurable. On the morning of 2026-10-09 (main 7ff55ce2): the tree is not rustfmt-formatted
(20,277 hunks in 646 of 723 hand-written files), CI ran the unit tests of 12 of 31 crates (469 tests in the other 14
never ran), test code was not linted (239 findings), 26 crate dependencies were unused, `deny.toml` had no job,
`--all-features` did not compile, and the edition was 2021. By the evening, PRs 235 to 248 had moved to edition 2024,
enforced the unused-variable, mut and import lints, turned debug info off in dev and test, and stripped the release
binaries; this note's pull request, rebased onto that main (cab0f26e), fixes the rest of the cheap half. The rest is
ranked at the end.

## Method

Four surveys on 2026-10-09, each claim read from a file on the project's default branch that day: `Cargo.toml`
(`[workspace.lints]`, `[profile.*]`), `clippy.toml`, `rustfmt.toml`, `.cargo/config.toml`, `deny.toml`, `justfile` or
`xtask`, `.config/nextest.toml`, CI workflows, fuzz directories, contributor docs.

- JS and Python tooling: ruff, uv, oxc, biome, rolldown, swc, turborepo, deno.
- Compilers, language servers, ports: rust-analyzer, rustc (`compiler/`, `src/tools/tidy`, rustc-perf), wasmtime,
  typst, fish-shell (C++ to Rust), uutils coreutils (GNU test suite), sudo-rs.
- `unsafe` and threads: tokio, crossbeam, hashbrown, bevy (`bevy_ecs`), rayon, bumpalo, smallvec, ripgrep, regex,
  memchr, parking_lot, polars.
- Build profiles and distribution: the above plus fd, bat, zed, helix, nushell, alacritty, tree-sitter, next.js.

Local measurements: `main` at 7ff55ce2 unless a later commit is named, Rust 1.99.0, Apple M-series,
`CARGO_INCREMENTAL=0` where a time is given.

## Where tsrs already matches or leads

Fat LTO with one codegen unit and PGO for the release binaries (ruff, uv and rust-analyzer ship PGO; nobody else in
the survey does, and only rustc ships BOLT). mimalloc, measured against jemalloc and glibc (`notes/perf-build-level.md`;
ruff, uv, polars and fd chose jemalloc on Linux without a published comparison), and since PR 248 the `mimalloc-safe`
crate that oxc and rolldown use. `rustc-hash` 2 with hashbrown 0.17, the modal choice (ruff, uv, oxc, rolldown,
rust-analyzer, zed, typst, wasmtime, deno). 53 `const` size assertions on arena types (rustc has `static_assert_size!`
on 70 types, rust-analyzer and ruff a handful each). `#[cold]` and `#[inline(never)]` slow-path splits. A lint set
modeled on bevy's and oxc's, `#[expect]` with a reason instead of `#[allow]`, the `*_unchecked` ban. Instructions
retired and peak RSS per merge with a regression comment (the rustc-perf model; the JS tooling projects use CodSpeed
for it). Generated-code freshness in CI. Nightly differential parser fuzzing. The determinism gate. Miri on the arena
free-list tests and the conformance suite on a `panic = "abort"` build. Actions pinned by commit, read-only tokens,
`persist-credentials: false`. Since the same day: edition 2024 (PR 247; 16 of 19 projects in the build survey are on
it), stripped release binaries (PR 245; the modal choice), no debug info in dev and test (PR 238; oxc). A ledger of
measured and rejected techniques, which no surveyed project keeps.

Three findings of the surveys confirm earlier decisions here: `panic = "abort"` is not an option while
`catch_unwind` guards the language server and API (uv, oxc and swc ship `abort` and carry dead `catch_unwind`
calls); a target CPU above the x86-64 baseline costs cycles here (`notes/linux-x86-round.md`), and no surveyed project
ships one (polars ships SSE4.2 and AVX2 wheels behind a Python-side CPU check; Bun retired its baseline split into
alias packages); ruff measured BOLT after PGO and declined it (x86-64: -1.24% wall for +4.9% binary; arm64: -0.08%
wall for +11% binary), while `notes/perf-build-level.md` measured -2.7 to -4.0% wall on x86-64, so it stays there;
the aarch64 Linux release runs BOLT unmeasured.

## What tsrs lacked, measured

| Practice | Who does it | tsrs on 2026-10-09 | Status |
| --- | --- | --- | --- |
| `cargo fmt --check` in CI | every surveyed project; oxc and rolldown with `use_small_heuristics = "Max"`, ruff, uv and biome with `style_edition = "2024"` | No `rustfmt.toml`; `cargo fmt --check` reports 20,277 hunks in 646 of 723 hand-written files (checker 5,576, ls 3,166, transformers 1,388, ast 1,148). Of 338,192 hand-written lines, 9.8% are over 100 columns, 4.4% over 120, 2.1% over 140. rustfmt's defaults turn Go-shaped one-liners into six-line chains. | Not done: a one-shot reformat is the owner's call (`git blame` needs `.git-blame-ignore-revs`; the generated fourslash tests need `#[rustfmt::skip]` or a formatting generator). |
| Unit tests of every crate in CI | all | `cargo test` ran 12 crates; the list dates from the first CI commit (2026-10-01) and crates added since were not added to it. 469 `#[test]`s in 14 crates never ran (project 104, api 82, ls 67, lsp 41, sourcemap 39, fswatch 39, lsproto 33, api_transport 30, api_codec 16, checker 7, projectutil 6, transformers 3, incremental 1, execute 1). Run locally with `ts-ref`: 761 pass, 0 fail, 6 ignored, in about 90 s including the compile. Three did not: the memory tests of `tsrs_api` and `tsrs_cli` did not link on macOS (glibc `malloc_trim`), `tsrs_api_transport`'s node round trip needs a `packages/typescript` checkout no job has, and its `requestfs_differential` test (feature `requestfs`) fails on macOS because `std::env::temp_dir()` is a symlink there and the test compares `realpath` against the uncanonicalized path. | Done in this PR: CI runs `cargo test --workspace` (minus the generated fourslash suite, the wasm crate and the round trip), the memory tests are Linux-only by `cfg`. Open: the round trip's checkout, the `temp_dir` canonicalization. |
| Clippy on test targets and all features | all (`cargo clippy --workspace --all-targets --all-features -- -D warnings`) | The ratchet lints library and binary targets with default features. At 7ff55ce2, `--all-targets` added 239 findings in hand-written code (203 in test and example files, 36 in `cfg(test)` parts of library files) and about 20 `unsafe_op_in_unsafe_fn`; by kind, 88 `clone_on_ref_ptr`, 22 `format_push_string`, 16 `disallowed_types` (std maps in tests), 15 `redundant_clone`, 13 `useless_conversion`, and about 25 unused variables, `mut`s and imports that PRs 239, 242 and 246 have since fixed. Generated code added 202, 197 of them in `tsrs_lsproto/src/lsp_generated.rs` (`clone_on_copy`, `clone_on_ref_ptr`: the generator's). `cargo check --all-features` did not compile: two global allocators (`tsrs_core/alloc-profile` and the `ast_oracle` example), and, once PR 246 made unused imports errors, two unused imports in `tsrs_core/src/alloc_profile/`, which only that feature compiles. | `--all-features --all-targets` compiles and is in CI's `cargo check`. Linting the test targets is open (below). |
| Lint levels in `Cargo.toml`, `-D warnings` | all | The ratchet exists because the baseline had findings. In the morning `tools/lint/baseline.tsv` held 11 (2 `needless_pass_by_value`, 1 `dead_code`, 8 `unused_variables`); PR 239 fixed the 8, so it holds 3 in two rows, and PRs 239, 242, 246 and 247 turned `unused_variables`, `unused_mut`, `unused_imports` and `unsafe_op_in_unsafe_fn` into plain warnings at zero. `dead_code` is still allowed workspace-wide. | Open (below). |
| Unused dependencies fail CI | cargo-shear: ruff, uv, oxc, rolldown, swc; cargo-machete: rust-analyzer; cargo-udeps: uutils | 26 unused crate dependencies in 15 manifests (`indexmap` 8, `bitflags` 4, eight in `tsrs_cli`; the same set before and after PRs 236 and 248 swapped two crates) and 2 unused workspace dependencies (`unicode-id-start`, `tsrs_fourslash`). | Done: removed; `cargo shear --deny-warnings` in the `lint-ratchet` job. |
| cargo-deny in CI | oxc, ruff, uv, biome, swc, tokio, bevy, polars, fish, wasmtime | `deny.toml` passed but nothing ran it. | Done: `cargo deny --locked check` in the `lint-ratchet` job. |
| Dev profile without local thin LTO | ruff, uv (`lto = "off"`: "Avoid the local ThinLTO that Cargo enables at nonzero optimization levels") | `[profile.dev] opt-level = 1` gets cargo's thin-local LTO. Clean `cargo build -p tsrs_cli` at cab0f26e (debug info already off): 38.35 s against 26.65 s with `lto = "off"`; at 7ff55ce2 with line tables, 26.96 s against 21.33 s. | Done. |
| `#![forbid(unsafe_code)]` where there is none | sudo-rs, uutils, regex-syntax, regex-lite, `ruff_text_size`, `biome_text_size`; bevy inverts it (`unsafe_code = "deny"` for the workspace, `#![expect(unsafe_code, reason)]` per crate) | 14 of 31 crates have no `unsafe` token. | Done: the attribute on those 14. |
| `typos` in CI | ruff, uv, oxc, rolldown, rust-analyzer, rustc, bevy | 4,436 findings, 214 in hand-written non-test code. Go-faithful names account for most (`datas` 36, `nd` 28, `lod` 26, `fo` 11); the rest are misspellings in comments (`whitespce`, `indenation`, `refrence`, `replacable`, `paramer`, `mimick`). | Open: `_typos.toml` with `extend-exclude` for the generated and reference data and `extend-identifiers` for the Go names, then the comments. |
| Edition 2024 | 16 of 19 projects in the build survey; all 8 JS tooling projects but swc | In the morning a trial `cargo fix --edition --workspace --all-targets` touched 29 files and 263 lines and compiled clean with 51 drop-order warnings to review. | Done on main the same day (PR 247). |
| `cargo nextest` | ruff, uv, biome, turborepo, deno, rust-analyzer; tokio under Miri | `cargo test`. nextest's `slow-timeout = { period = "60s", terminate-after = 5 }` (rust-analyzer) turns a hung checker thread into a failure instead of a 60-minute job; uutils' `retries = 2` is the one setting not to copy. | Open, small. |
| Miri on the `unsafe` crates | 10 of 11 library projects, one or two crates each: hashbrown 1.7 min, bumpalo 3.5, polars-core 5.1, memchr 8.5, regex-automata 9.1, bevy_ecs 27-34 | The arena free-list tests only (`arena-safety` job). `docs/RUST.md` lists all of `tsrs_core` as untried. | Open (`docs/RUST.md`). |
| ThreadSanitizer | crossbeam only (ASan, MSan and TSan in one 11-minute job; suppressions for fence-based synchronization) | Nothing checks data races between the up-to-32 checker threads; the determinism gate sees only their output. | Open (`docs/RUST.md`, "Not tried"). |
| Instruction-count benchmarks on free runners | CodSpeed in oxc, rolldown, ruff, biome, swc, uv; iai-callgrind in smallvec | `pr-verify` measures instructions on a 32-vCPU Depot runner per pull request. | Open (`docs/RUST.md`). |
| `check-private-items = true` | sudo-rs, uutils, bevy | `missing_safety_doc` is on but applies to public items only; the 43 `unsafe fn` here are `pub(crate)`. | Open: one line in `clippy.toml`, then the `# Safety` sections. |
| `rust-version` | every surveyed project | Absent. `rust-toolchain.toml` pins the compiler for rustup users, which is everyone here. | Not done: 31 manifests for a clearer error nobody gets. |

## What this pull request changes

- `.depot/workflows/ci.yml`: `cargo check --workspace --all-targets --all-features --locked` under `-D warnings`;
  `cargo test --workspace` for every crate but `tsrs_fourslash` (the fourslash gate runs its suite from the release
  binary), `tsrs_wasm` (the wasm job) and `tsrs_api_transport`'s node round trip, which runs by target; `cargo deny`
  and `cargo shear --deny-warnings` in the `lint-ratchet` job, the binaries installed by `setup-rust`'s `tools`
  input.
- 26 unused crate dependencies and 2 unused workspace dependencies removed.
- `[profile.dev] lto = "off"`.
- The memory tests of `tsrs_api` and `tsrs_cli` are `cfg(target_os = "linux")`: glibc `malloc_trim` and
  `/proc/self/status`.
- `tsrs_parser` has an `alloc-profile` feature forwarding to `tsrs_core`'s, the `ast_oracle` example installs its
  counting allocator only without it, and two unused imports in `tsrs_core/src/alloc_profile/` are gone.
- `#![forbid(unsafe_code)]` on the 14 crates without `unsafe`.
- `docs/RUST.md`, `CONTRIBUTING.md`.

## Next, ranked

1. **Format the tree once** (`cargo fmt`, a `rustfmt.toml`, `.git-blame-ignore-revs`, `cargo fmt --check` in CI).
   The one practice every surveyed project shares and the first thing a Rust reader notices. Choose the width first:
   `max_width = 120` with `use_small_heuristics = "Max"` keeps most Go-shaped lines on one line; the default (100)
   rewraps 33,104 lines. Medium; coordinate with open branches.
2. **Lint the test targets and retire the ratchet.** Fix or `#[expect]` the remaining findings in test code and the
   3 in the baseline, make `tools/gen-lsproto` emit code without the 197 clone findings (or an `#![allow]` header for
   generated files, as prost and bindgen do), then move the lint levels into `[workspace.lints]`, run
   `cargo clippy --workspace --all-targets --all-features -- -D warnings` in CI, flip `dead_code` to `warn`, and
   delete `tools/lint/ratchet.py` and `baseline.tsv`. Medium.
3. **Miri on all of `tsrs_core`** with `--features plain-ptrs`, path-gated to the `unsafe` crates or nightly, Miri
   pinned by date (tokio). Medium.
4. **ThreadSanitizer on the checker pool**, cron only. Medium; expect suppressions for the 79 commented `Relaxed`
   orderings.
5. **Instruction-count benchmarks on GitHub-hosted runners** (CodSpeed or iai-callgrind) over a fixed corpus at one
   checker, so pull requests get an instruction signal without Depot time; keep `bench.yml` for wall and memory.
   Medium.
6. **nextest** with rust-analyzer's `slow-timeout`, no retries, JUnit output. Small.
7. **typos**: config, then the roughly 60 real misspellings. Small.
8. **`check-private-items = true`** and `# Safety` sections on the 43 `unsafe fn`. Small config, medium writing.
9. **BOLT on aarch64**: one before/after on an arm runner, or drop the step from that release job. Small.

Not worth doing, with the reason: `panic = "abort"` (`catch_unwind` in the language server and API), a target CPU
above x86-64 or an AVX2 build (`notes/linux-x86-round.md`; only polars ships CPU tiers, Bun stopped), jemalloc
(`notes/perf-build-level.md`), `opt-level = "s"` for cold crates (same note, noise), `rust-version` (pinned
toolchain), astral's `hawk` (experimental, "not intended for public consumption"), loom (tokio's sync suite alone
takes 139 minutes; only a narrow work-queue model would fit), `cargo vet` (wasmtime only). `std::hint::likely` and
`unlikely` are still nightly-only ("perma-unstable" tracking); `std::hint::cold_path()` is stable since 1.95 and is in
`docs/RUST.md` as a candidate.
