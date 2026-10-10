# Resolver allocations, 2026-10-10

Measurements compare the implementation against `0188eed0333447c52f57a9ed8afd4ab29f65d088`.
For the draft PR, `codex/resolver-allocations` was rebased onto `aa7911e7` (main) to exclude unrelated
linter commits. The measurements below predate that rebase. The original checkout and its unrelated
edits are untouched.

## Prior work and source

Read the rejected list in `notes/perf-round2-followups.md`, the Techniques section of `docs/RUST.md`,
`notes/mem-round4.md` section 7, and the resolver / front-end notes before implementing.
The existing 64-shard caches and cached filesystem remain in place. Directory listings, `openat`,
new persistent caches, shared checker graphs, and a realpath prefix cache are outside this experiment.

The reference is the local oxc-resolver checkout at `2e976e6489ff902f315d0179a667f8d0465dcee7`:
`src/cache/thread_local.rs` retains a 256-byte `PathBuf`; cached-path helpers reuse it during path
and extension operations. Its borrowed cache probes avoid building owned keys before a lookup.
Here paths remain TypeScript lexical strings: `PathBuf` would change cross-platform/URL spelling.

## Implementation retained for review

- Reuse per-thread strings for extension trials, module suffixes, package.json paths, node_modules
  directories and normalized joins. Take the string out of its cell during a callback so a host that
  re-enters resolution cannot overwrite it or panic from an outstanding TLS borrow. Buffers above
  64 KiB are discarded; ordinary buffers begin with 256 bytes. There are five slots per participating
  thread, with no cross-thread lock and no new unsafe code.
- Probe module/type-directive caches with borrowed composite keys, preserving every key field and
  field hashing order. Allocate key strings on insertion. Package.json lookup borrows canonical text.
- Borrow request names/directories and path directory/base views. Walk slash-normalized ancestors
  by slicing. Join into the scratch string, then own only the resulting candidate. Owned path APIs
  remain available; backslashes, roots, URL forms, case folding and trailing separators retain their rules.
- Iterate built-in and configured conditions without allocating strings or a vector per request.
  Match cached patterns without cloning each candidate; retain the first longest-prefix wildcard.
- Consume the parsed package.json tree so strings and dependency keys move into typed fields.
  Duplicate validation, repeated top-level fields, partially merged invalid maps and ordered exports
  retain their existing behavior. The public borrowed decoding entry point remains available.
- Borrow known result extensions and scan configured suffixes without collecting a reference vector.

## Allocation probe

`tools/perf/resolver-allocations.rs` is a standalone driver using the existing `alloc-profile` allocator.
All numbers include driver setup and result hashing. They are allocation calls, not bytes or peak RSS.
It resolves 1,000 relative files, 1,000 missing relative files, 1,000 exported packages and 1,000 missing
packages. `resolve` creates ten fresh resolver instances on one cached virtual filesystem; `suffix`
adds `.native`, `.web`, and empty suffixes; `hits` repeats twenty rounds on one resolver. `json` parses
the pinned xstate-main package.json 20,000 times.

| Probe | Baseline | Extension/suffix scratch only | Full change | Full allocation reduction |
| --- | ---: | ---: | ---: | ---: |
| Mixed resolution | 2,980,666 | 2,870,667 | 1,150,670 | 61.4% |
| Module suffixes | 3,536,791 | 3,086,793 | 1,366,796 | 61.4% |
| Repeated cache hits | 898,546 | 887,547 | 411,550 | 54.2% |
| package.json | 4,560,017 | 4,560,017 | 3,620,017 | 20.6% |

Every result hash matches baseline. The probe is a workload check, not the semantic oracle: it hashes
resolved paths, extensions and package IDs, and the JSON workload hashes the name. The Go oracle and
conformance runs below cover behavior more fully. Peak instrumented heap is unchanged at 8.0 MB
(mixed), 10.2 MB (suffix), and 3.7 MB (hits). Most removed allocations were short-lived.
The recorded JSON counts used a hardcoded input filename; the checked-in driver's filename argument
adds a few fixed setup allocations, with the same per-parse reduction.

To reproduce on either checkout, create an external scratch crate (no workspace or dependency changes):

```sh
export RESOLVER_CHECKOUT=/path/to/tsrs-checkout
export RESOLVER_PROBE=/tmp/resolver-probe
python3 - <<'PYCODE'
import os, pathlib, json
root = pathlib.Path(os.environ['RESOLVER_CHECKOUT'])
probe = pathlib.Path(os.environ['RESOLVER_PROBE'])
(probe / 'src').mkdir(parents=True, exist_ok=True)
(probe / 'src/main.rs').write_text((root / 'tools/perf/resolver-allocations.rs').read_text())
manifest = '[package]\nname = "resolver-allocation-probe"\nversion = "0.0.0"\nedition = "2021"\n[dependencies]\n'
for name in ['tsrs_core', 'tsrs_module', 'tsrs_vfs']:
    features = ', features = ["alloc-profile"]' if name == 'tsrs_core' else ''
    manifest += name + ' = { path = ' + json.dumps(str(root / 'crates' / name)) + features + ' }\n'
manifest += '[profile.release]\ndebug = 1\n'
(probe / 'Cargo.toml').write_text(manifest)
PYCODE
cargo build --offline --release --manifest-path "$RESOLVER_PROBE/Cargo.toml"
"$RESOLVER_PROBE/target/release/resolver-allocation-probe" resolve
"$RESOLVER_PROBE/target/release/resolver-allocation-probe" suffix
"$RESOLVER_PROBE/target/release/resolver-allocation-probe" hits
"$RESOLVER_PROBE/target/release/resolver-allocation-probe" json /path/to/xstate-main/package.json
```

Copy the same driver into the baseline checkout before building its probe. Build each version against
its own source tree; retain each binary before changing the source. On this Mac, builds used
`DEVELOPER_DIR=/Library/Developer/CommandLineTools` and the matching `CC` / `CXX` to avoid an unrelated
Xcode 15 / SDK 27 mismatch.

## Whole-project measurements

Apple M3 Max, 14 cores, 36 GiB RAM, macOS arm64, Rust 1.99.0, `dist` profile. Both source revisions use
identical compiler settings. Projects and install commands are pinned by `bench/projects.json`:
xstate `fbee62e7c1586315ed478c2fedf530d7e0ff5a3e` (`-p .`) and VS Code
`9cf0128b9822dcd8a533f08ef7312956a4390301` (`-p src`). xstate's prescribed pnpm install completed its
postinstall; VS Code uses `npm ci --ignore-scripts --no-audit --no-fund` and reports missing Electron
and original-fs declarations at baseline, with exit 2. xstate exits 0.

Seven alternating baseline/new runs per project and mode, warm files, using
`--noEmit --incremental false --pretty false --extendedDiagnostics`. Single-threaded adds
`--singleThreaded` and `RAYON_NUM_THREADS=1`; default leaves the checker count alone.
`/usr/bin/time -l` collects instructions retired and maximum resident set size. These macOS counters
include kernel work and have noise; they are **not** `bench/count.py`'s deterministic user-space count.
No Linux host/counter is available in this session, so the repository's instruction landing gate is
not certified by these measurements. Wall values are local screens, not two README publishes.

Medians (instructions in billions, RSS in MiB, wall in seconds):

| Project / mode | Instructions before → after | Delta | Peak RSS before → after | Delta | Wall before → after |
| --- | ---: | ---: | ---: | ---: | ---: |
| xstate-main, single | 7.060291 → 7.041523 | -0.27% | 178.64 → 178.62 | -0.01% | 0.57 → 0.57 |
| xstate-main, default | 9.753709 → 9.727583 | -0.27% | 255.86 → 256.23 | +0.15% | 0.13 → 0.13 |
| vscode, single | 98.202199 → 98.017080 | -0.19% | 1596.92 → 1596.92 | +0.00% | 9.79 → 9.64 |
| vscode, default | 109.016024 → 108.873138 | -0.13% | 1942.50 → 1951.52 | +0.46% | 2.55 → 2.60 |

All fourteen runs per project/mode have identical ordered diagnostics and exit status. These runs use
1,536 files on xstate. VS Code's prescribed no-script install is deliberately unchanged between builds.
The isolated extension/suffix scratch stage was smaller still: xstate 7.059502 → 7.053194 G
(-0.089%) and VS Code 98.143507 → 98.107235 G (-0.037%) single-threaded. Its default peak RSS
was 256.20 → 255.59 MiB on xstate and 1942.70 → 1943.78 MiB on VS Code.

The combined prototype does not meet the 1% instruction screen or the 5% default-memory bar on either
project. The local wall medians do not demonstrate the required 2% improvement across two publishes.
The strong microbenchmark allocation reduction is too small a part of these whole-program runs to
justify this patch's TLS buffers, borrowed-key invariant and additional path/decoder entry points.

## Validation and disposition

Validation at the measured revision:

- `cargo test -p tsrs_core -p tsrs_module -p tsrs_vfs`: 101 + 27 + 87 passed; two ignored oracle/corpus
  entries. The separately generated resolver oracle passes all 734 usable scenarios (737 generated,
  three rejected inputs excluded), each with tracing enabled and disabled, against pinned Go
  `b85298b6a81f772d080b0455de0ca9d744cd6fd6`.
- Reference conformance: 13,458 errors, 12,779 types and 12,779 symbols pass. Canonical mode:
  13,458 / 12,778 / 12,778. Every pass/fail/crash/timeout list is byte-identical before/after in both
  modes. The canonical gate exits 1 both before and after because its existing
  `conformance/objectLiteralNormalization` types/symbols failure is below the reference-mode minimum;
  there are no crashes or timeouts and no new failures.
- All 28 `testdata/regressions` projects pass.
- Workspace check, wasm32-wasip1 check, lint ratchet (no new findings), and source checks pass.
  No baseline ratchet counts were raised and no new unsafe code was introduced.

After rebasing onto `aa7911e7`, the targeted suites pass again (108 core, 27 module, 88 VFS),
all 734 usable resolver oracle scenarios match, and the lint ratchet and source checks pass. Production
resolver/path changes are byte-identical to the measured implementation; the rebase preserves main's
additional path coverage. Whole-project performance and conformance were not rerun after the rebase.

The original checkout was left untouched. The owner explicitly requested reapplying the implementation
and retaining it on `codex/resolver-allocations` for review on 2026-10-11. The branch includes the code,
regression coverage and standalone probe. The local artifact directory
`/private/tmp/tsrs-resolver-results` contains the saved patch and binaries, before/after logs,
`measure-final.json` and a SHA-256 manifest.

Decision: retain the implementation for the owner's review. The measured allocation reductions and
whole-project results above are unchanged; the local screen has not demonstrated the repository's
usual landing thresholds. A Linux `bench/count.py` run on a resolver-dominated pinned workload could
provide further evidence, including default peak RSS and diagnostic comparisons. These macOS numbers
do not certify a deterministic Linux result.
