# Rust-owned program data and resolution hosts (2026-10-10)

## Scope and status

This continues the full memory-model migration from `a183691d`. It is an unfinished migration branch, not a
completed replacement of the Go-style graph or a performance change ready to land. The earlier dense-link,
LSP lifetime and residency evidence was reviewed (`rust-owned-links.md`, `lsp-memfix.md`, `mem-round4.md`,
`perf-round2-followups.md` and `docs/RUST.md` Techniques). This changes production ownership boundaries under
the user's migration requirement; it does not retry shared checker graphs or speculative reclamation.

- Program versions retain their shared processed-file containers and project-reference mapper through `Arc`.
  Reuse clones those owners, and freeing the final version drops the containers normally. The address-pair
  `SharedProgramData` API and its region callback, `free_unshared_program`, manual mapper/host freeing and the
  panic-only `FreeMapperOnUnwind` hook are removed.
- Project-reference redirect data is built exclusively and then shared through `Arc`. The mapper caches its
  DTS-faking host, which retains the redirect data rather than the mapper. This preserves host/cache reuse
  without a reference cycle. A retained host can still answer redirect queries after the mapper drops.
- `DefaultResolver`, `ResolverOptions` and API resolver templates retain strong resolution-host owners.
  Compiler hosts, standalone/snapshot API filesystems and source-definition host adapters are retained by those
  owners. The API no longer fabricates a static reference from a raw host allocation or frees it manually;
  early error returns and unwinding drop it normally.
- Auto-import registry builders and alias resolvers retain their clone host through `Arc`. The host's source
  filesystem, wrapped resolver filesystem and realpath closures also retain strong owners. The registry's
  `assume_static` transmute is removed. Module resolvers formerly allocated into scratch sidecars now use
  ordinary stack or shared Rust ownership.
- The mapper and DTS-faking host modules forbid unsafe code. No unsafe thread traits or inventory entries are
  added. Parsed-config/module-cache/AST pointers inside the containers remain legacy graph edges.

The base `Region` is still retained by every program version. `KnownSymlinks` and graph allocations still route
through its address registry, so `adopt_owner` and `enter_owner` remain until those data migrate. The `Program`
root itself is still leaked and manually freed; checkers still retain a static program reference. Config-parser
hosts still use the older static host contract. An owned container or host does not establish ownership of the
graph pointers inside it. The next stages must remove these remaining boundaries and the raw AST/type graph.

The following checkpoint (`rust-owned-checker-inputs.md`) removes the static checker and pool input references,
owns file-list containers, and drops inputs normally if pool construction panics. The outer program, checker
leases and graph referents still need migration.

## Local measurements

Apple M3 Max / macOS / Rust 1.99.0, locked fat-LTO release build without PGO, using the same pinned Compiler
projects and `/usr/bin/time -l` method as the previous stages. Five interleaved rounds after warm-up compare
the previous owned-link executable, this implementation, original Oxc and pre-Oxc. All 80 runs have identical
output and expected status 2. `rust-owned-program-data-results.json` records the raw rows, executable hashes,
base commit and changed Rust source hashes.

| Project | Mode | Instructions (G), owned links → this stage | Change | Peak RSS (MiB), owned links → this stage | Change |
| --- | --- | ---: | ---: | ---: | ---: |
| Compiler | single | 2.137482 → 2.136917 | -0.026% | 114.66 → 114.67 | +0.014% |
| Compiler | default | 2.508479 → 2.507964 | -0.021% | 143.72 → 143.08 | -0.446% |
| Compiler-Unions | single | 4.632965 → 4.632836 | -0.003% | 104.78 → 104.84 | +0.060% |
| Compiler-Unions | default | 5.726055 → 5.741984 | +0.278% | 146.56 → 145.27 | -0.885% |

This boundary introduces no material local regression against the previous stage. It does not meet the
performance landing bar or recover the cumulative migration's regressions. Against original Oxc, Compiler
single-thread instructions remain +1.075%, and Compiler-Unions default RSS remains +2.514% (+3.56 MiB).
Against pre-Oxc, default RSS remains +82.96% / +76.75% (78.20 → 143.08 MiB / 82.19 → 145.27 MiB).
The full migration's memory and instruction gates still fail. Default-mode scheduling and peak RSS vary between
publishes; these short runs make no wall-time claim. Linux instruction counts, larger pinned projects and
comparable headline publishes are still required.

## Verification

The three new ownership cases verify that program reuse keeps shared containers until the final version drops,
failed resolver construction releases its host, and a cached DTS host retains redirects without retaining its
mapper or leaking the compiler host. Existing compiler, module-resolution, filesystem, config, auto-import,
API, project, incremental, build and LSP suites pass. Workspace, wasm32-wasip1 and checker feature checks pass;
the lint ratchet has three baseline findings and none new, and the source inventory has 31 reviewed custom
Send/Sync implementations. The five Linux/glibc API RSS assertions remain unvalidated on macOS.

All six conformance result sets over 15,197 pinned TypeScript variants are identical to both the original Oxc
branch and the previous owned-link checkpoint: diagnostics 13,462; types/symbols 12,779 each; JS 13,392;
JS maps 149; sourcemap text 156. The final run uses the baseline's 120-second per-case limit and has no crashes
or timeouts. The first attempt used the runner's 20-second default and timed out on
`intersectionConstructorReductionCrash`, which also took 21.5 seconds in the saved baseline; it passes both
the focused retry and final full run. The default-history types/symbols classifications also match both saved
baselines, retaining their existing `objectLiteralNormalization` failure. All 28 regression fixtures pass.
These comparisons preserve result sets; they do not claim every variant passes.

Full fourslash runtime, Linux RSS and large pinned-project performance gates remain necessary for the final
migration. No capability count change is claimed, and the README benchmark section is untouched.
