# Thread-local resolver path buffers

PR scope: reuse strings only in `try_extension` and `try_file`'s module-suffix loop. The other
allocation experiments are excluded. Based on oxc-resolver `2e976e6489ff902f315d0179a667f8d0465dcee7`,
`src/cache/thread_local.rs` (256-byte scratch path), adapted to TypeScript lexical string paths.
Prior work consulted: `docs/RUST.md` Techniques, `notes/perf-round2-followups.md` rejected list,
`notes/mem-round4.md`, and the front-end/cache notes.

Two thread-local strings retain capacity across probes; only successful paths become owned results.
Taking a string out of its cell before calling the host allows nested resolution on the same thread.
Buffers above 64 KiB are discarded. Coverage includes reentry, separate threads, Unicode, oversized
paths and recovery after a panicking callback. Resolution order, path spelling and traces are unchanged.

## Measurements

Historical measurements of this isolated change against `0188eed0333447c52f57a9ed8afd4ab29f65d088`,
before rebasing onto main `aa7911e7`. Apple M3 Max (14 cores, 36 GiB), macOS arm64, Rust 1.99.0.
These are the scratch-only results, not the broader allocation experiment's results.

The standalone `tools/perf/resolver-allocations.rs` driver uses the existing `alloc-profile` allocator.
Each run makes 40,000 requests: relative files, missing relative files, exported packages and missing
packages, in ten resolver instances on one cached virtual filesystem. Suffix mode adds `.native`,
`.web` and the empty suffix. Counts include setup and result hashing; result hashes match baseline.

| Probe | Allocation calls before → after | Reduction |
| --- | ---: | ---: |
| Mixed resolution | 2,980,666 → 2,870,667 | 3.69% |
| Module suffixes | 3,536,791 → 3,086,793 | 12.72% |

Whole-project `dist` builds use the pinned xstate and VS Code revisions and install commands in
`bench/projects.json`. Seven alternating runs per version, `--noEmit --incremental false --pretty false
--extendedDiagnostics`; single-threaded adds `--singleThreaded`, `RAYON_NUM_THREADS=1`.
`/usr/bin/time -l` measures retired instructions (billions) and maximum resident set size (MiB):

| Project | Single-threaded instructions before → after | Delta | Default peak RSS before → after |
| --- | ---: | ---: | ---: |
| xstate-main | 7.059502 → 7.053194 | -0.09% | 256.20 → 255.59 |
| vscode | 98.143507 → 98.107235 | -0.04% | 1942.70 → 1943.78 |

macOS instruction counts include kernel work and noise. These results are below the usual landing
thresholds; deterministic Linux `bench/count.py` verification remains outstanding. Retained in the
draft PR at the owner's request. Saved binaries and logs: `/private/tmp/tsrs-resolver-results/{base,scratch-base,scratch}`.

## Reproduce the allocation probe

Create a scratch Cargo binary outside the workspace, copy the driver to `src/main.rs`, and point its
path dependencies `tsrs_core`, `tsrs_module` and `tsrs_vfs` at the checkout being measured. Enable
`features = ["alloc-profile"]` on `tsrs_core`. Use the same driver against each revision:

```sh
cargo build --offline --release --manifest-path /tmp/resolver-probe/Cargo.toml
/tmp/resolver-probe/target/release/resolver-allocation-probe resolve
/tmp/resolver-probe/target/release/resolver-allocation-probe suffix
```

## Validation

On the narrowed branch: 25 module cases pass; all 734 usable corpus-derived Go resolver oracle
scenarios match, including traces. The allocation probe reproduces both recorded counts and result
hashes after rebasing onto `aa7911e7`. Lint ratchet and source checks pass; no new unsafe code.
