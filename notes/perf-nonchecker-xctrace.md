# Non-checker performance investigation (2026-10-11)

The largest measured targets outside checking are scanning/parsing/AST construction, filesystem calls during
loading and resolution, and binding. On this Mac, Formbricks exposes the filesystem and comment-scanning costs;
VS Code exposes identifier construction and binding. Sequential graph loading and collection are much smaller.
These are investigation targets, not measured optimization wins. No compiler code changed.

Follow-up: [binder and identifier traversal experiments](perf-nonchecker-traversal.md) measured two small
specializations suggested by these captures. Both were reverted because no qualifying gain was established.

## xctrace is blocked; the captures below use sample

The requested `xcrun xctrace` investigation could not record an Instruments trace. All three preflight commands
terminated with SIGKILL, with no stdout or stderr, including outside the sandbox:

```sh
xcrun xctrace version
xcrun xctrace list templates
xcrun xctrace record --template 'Time Profiler' --output /tmp/tsrs-nonchecker-xctrace/preflight.trace \
  --launch -- /usr/bin/true
```

Direct invocation of the installed xctrace also failed. Xcode is 15.4, Instruments 15.3, macOS 27.0.1.
`notes/mem-xctrace.md` records the earlier runtime signature failure on this installation. This run confirms the
startup failure, not a new diagnosis of its cause. No Xcode files or security settings were modified. A working
Xcode/Instruments installation is still required for the requested xctrace recording.

Instead, this investigation collected uninstrumented `/usr/bin/time -l` counters, existing frontend phase timers,
and `/usr/bin/sample` stacks. These are **not xctrace results**.

## Method and scope

Compiler `63ac5a23a4b6253e85378e6b4bb56e2fcec91731`, release optimization, fat LTO, one codegen unit, symbols and
line tables retained, no PGO. Host: 14-core arm64 Mac, macOS 27.0.1. Public benchmark checkouts were reused from
the memory investigation: VS Code, Formbricks and t3code at `bench/projects.json`'s revisions. Formbricks and
t3code Git revisions were rechecked; VS Code is the archive-backed checkout validated in `notes/mem-xctrace.md`.
Program counts were 10,427 / 10,455 / 2,945 files, with 371 / 0 / 6 diagnostics, respectively.

Full checks use `--noEmit --incremental false --extendedDiagnostics --pretty false`. Isolated frontend runs use
`--noEmit --incremental false --listFilesOnly --pretty false`, without extended diagnostics. Single-threaded runs
add `--singleThreaded` and `RAYON_NUM_THREADS=1`; default runs clear inherited thread/profiling overrides. Each
baseline cell has a warm-up followed by three measurements, with cell order rotated. Retained measurement runs
were sequential.

Important isolation details:

- `--listFilesOnly` still constructs and binds the program, but skips global/semantic diagnostics and emit.
- `--noCheck` is less suitable: global diagnostics still initialize checkers, and it changes lazy declaration
  parsing eligibility.
- **`--extendedDiagnostics` can itself initialize checkers in a list-only run.**
  `statistics_from_program` calls `symbol_count`/`type_count`, which reach `retired_checkers`, `pool.state()` and
  `for_each_checker_parallel`. Statistics can also force shared lazy lists. The reported Total time is captured
  before Statistics. Therefore phase-counter runs are separate from the checker-free stack captures and baseline.
  See `crates/tsrs_execute/src/tsc/statistics.rs:111`, `crates/tsrs_compiler/src/program.rs:2236` and
  `crates/tsrs_compiler/src/checkerpool.rs:739`.
- `--singleThreaded` disables speculative prefetch; `RAYON_NUM_THREADS=1` alone retains that algorithm on one
  worker. The single/default comparison is not a controlled experiment on thread count alone.

## Uninstrumented baseline

Medians; instructions are billions, RSS is MiB. Full and frontend are different workloads, **not before/after
versions**. Frontend includes writing the file list and omits later diagnostic work, so its instruction ratio is
a guide to the budget outside checking, not an exact removable fraction of a full run.

| project | mode | full wall s | frontend wall s | full instructions G | frontend instructions G | full peak RSS MiB | frontend peak RSS MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | default | 1.43 | 0.38 | 101.889 | 23.283 | 2,084.1 | 1,168.0 |
| vscode | single | 8.94 | 1.82 | 87.650 | 23.098 | 1,605.2 | 1,139.2 |
| formbricks-web | default | 0.90 | 0.22 | 77.702 | 16.868 | 1,954.9 | 731.0 |
| formbricks-web | single | 3.81 | 1.28 | 43.919 | 16.601 | 1,100.0 | 685.2 |
| t3code-server | default | 1.55 | 0.10 | 132.368 | 7.775 | 2,122.9 | 390.8 |
| t3code-server | single | 3.68 | 0.55 | 41.770 | 7.522 | 733.3 | 355.1 |

The isolated frontend instructions are 26.4% / 37.8% / 18.0% of the corresponding single-threaded full-run totals.
macOS counts include kernel work and are not the deterministic Linux `bench/count.py` gate. Host load and wall
time varied: default full-run ranges were 1.22–1.46 / 0.89–0.90 / 1.49–1.78 seconds. No speedup is claimed.

The full-run default-mode phase rows locate the wall-time budget without subtracting separate processes:

| project | config ms | program construction (Parse) ms | parallel parse/bind/resolve ms | sequential load ms | collect files ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| vscode | 80 | 281 | 251 | 10 | 4 |
| formbricks-web | 17 | 190 | 158 | 17 | 4 |
| t3code-server | 4 | 96 | 82 | 6 | 2 |

The parallel, sequential-load and collection rows are inside Parse; do not add them to it. Config plus Parse is
about 25% / 23% / 6% of the corresponding process-wall medians. Parallel parsing also binds; the near-zero Bind
row is not evidence that binding is free.

## Worker phase counters

Three separate default-mode runs with `TSRS_FRONTEND_STATS=1` and extended diagnostics. Values are medians of
accumulated **worker wall seconds**, not process wall time or exclusive CPU time. Lock timers overlap the other
categories; stats collection adds overhead. Single-threaded runs do not traverse the parallel timer sites.

| project | job wall | parse | bind | read | resolve | metadata | SyncMap lock timer |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| vscode | 2.489 | 0.997 | 0.592 | 0.485 | 0.310 | 0.063 | 0.095 |
| formbricks-web | 2.431 | 0.885 | 0.240 | 0.697 | 0.481 | 0.058 | 0.084 |
| t3code-server | 0.837 | 0.336 | 0.151 | 0.149 | 0.168 | 0.017 | 0.019 |

Parse is roughly 36–40% of job-wall accumulation. Binding is 24% on VS Code; reads and resolution together are
roughly 48% on Formbricks. These ratios prioritize investigation; they do not predict whole-program savings.
The timer sites are `crates/tsrs_compiler/src/host.rs:129` and `crates/tsrs_compiler/src/filesparser.rs:797`.
The SyncMap timer measures the entire lock acquisition, including uncontended/timer cost, not just blocking.
The old separate VFS lock probes are unused; their zero rows do not establish absence of VFS contention.

## Stack evidence and ranked targets

Two captures per project/mode used `/usr/bin/sample <pid> 8 1 -mayDie -file <report>`. Short default-mode runs
initially exited before sample finished attaching, producing empty call graphs. For usable captures, the driver
launched the target, sent SIGSTOP immediately, started sample, waited 1.5 seconds for attachment, then sent
SIGCONT. Startup observations from that pause and idle join/park stacks are excluded. These perturbed runs are
used only for stack attribution; the baseline above supplies timing and memory numbers.

The analyzer demangles with `rustfilt`, obtains self counts by subtracting direct children, and deduplicates
recursive ancestors for inclusive totals. Every tree passed the nonnegative-self-count check. Default-mode
denominators across two captures are 5,093 / 6,062 / 1,902 compilation stack observations. **These are observed
stack residency, not CPU-time percentages**: a filesystem syscall can be running or blocked. The smaller t3code
captures have noticeably more variance. No executing checker functions were found; a folded generic arena
symbol containing `tsrs_checker::Type` has binder/AST callers and is not evidence of checker execution.

1. **Scanner CPU: comment bodies on Formbricks, identifiers on VS Code/t3code.** Formbricks
   `Scanner::scan_byte_slash` accounts for 365 default-mode self observations (6.0%) and 240 single-threaded
   observations (9.6%). Of the default samples, 331 map to the multiline-comment ASCII body loop at
   `crates/tsrs_scanner/src/scanner.rs:1023`; `parse_semicolon` is the dominant caller. VS Code's corresponding
   shares are only 1.2% / 1.4%. This workload contrast makes the comment loop a more specific next CPU target
   than generic allocation reduction. `scan_byte_identifier` has 5.3% / 2.6% / 5.1% default self shares across the
   three projects, chiefly the loop at `scanner.rs:1307`; single-threaded VS Code/t3code shares are 7.2% / 7.7%.
   `create_identifier_with_diagnostic` contributes another 2.8% / 1.5% / 2.1% default self share, with its hottest
   lines at `crates/tsrs_parser/src/parser_3.rs:2051` around token advancement and identifier construction.
2. **File opens, directory enumeration and resolution probes, particularly Formbricks.** Filesystem contexts
   account for 34.6% / 53.3% / 35.3% of default compilation stack observations. Formbricks repeats at 51.5–54.7%;
   t3code varies from 26.7–42.4%. `DirFS::read_file` (`crates/tsrs_vfs/src/osvfs/os.rs:594`, `fs::read`) accounts
   inclusively for 17.6% / 32.3% / 20.9%; `DirFS::read_dir` (line 574, `fs::read_dir`) for 11.0% / 10.8% / 3.9%.
   `open`, `open_nocancel`, `stat` and `lstat` dominate the syscall leaves, rather than byte copying alone.
   A future xctrace recording should distinguish kernel execution from waits and attribute file/directory probes
   to config discovery versus module resolution. The present data does not establish duplicate reads or that
   fewer workers would improve wall time.
3. **Binder traversal and AST construction on VS Code.** Inclusive binder stacks account for 20.8% of default
   and 25.3% of single-threaded observations, consistent with its 0.592 worker-seconds of binding.
   `Binder::bind` (`crates/tsrs_binder/src/binder.rs:735`) is 2.7% default self; `bind_children` (line 1935),
   child visitors, declaration arrays, symbol tables and flow nodes distribute the rest. There is no isolated
   new binder function whose removal would recover that entire inclusive share. Constructor/TLS costs alone
   are much smaller than this aggregate.

These percentages overlap where marked inclusive; do not add them or multiply them by full-run wall time to
claim savings. In particular, this evidence ranks hotspots, not unnecessary work.

Prior-art constraints remain in force (`notes/perf-round2-followups.md`, `docs/RUST.md` Techniques,
`notes/mem-round4.md`):

- `notes/perf-parse.md` already rejected `memchr3` comment scanning (+5% cycles) and SWAR whitespace. The
  Formbricks address-level evidence identifies a workload-specific target; it does not justify repeating the
  same implementation. Any new approach needs faithful scanner behavior and whole-project measurements.
- Identifier byte tables, cold diagnostic splits, parser node stacks and static parent visitors already landed.
  Scanner arena strings and numeric identity changes were measured below the usual performance bars; this
  investigation does not reopen those designs.
- Resolver scratch buffers and lightweight metadata were below the bars; source mmap did not lower RSS.
  Older Linux results deprioritized file I/O when the frontend was only 5.6–6.6% of the run. This Mac's larger
  frontend wall budget warrants platform-specific diagnosis, not an unmeasured I/O or cache redesign.
- Serial load/collect refcount reductions and sharded maps already landed. The remaining 6–17 ms load and
  2–4 ms collect rows do not motivate another helper thread. Prior overlaps bought only 3–4 ms and were reverted
  (`notes/perf-serial-steps.md`). Frozen binder tables and compact-tree projects also remain rejected.

No candidate was implemented or shown to clear the landing bar. A future change still needs before/after
instructions and peak RSS, the applicable correctness gates, and the repository's performance threshold.

## Reproduction and validation

Build with the Command Line Tools developer directory because the selected older Xcode linker cannot read the
installed SDK (as documented in the memory investigation):

```sh
DEVELOPER_DIR=/Library/Developer/CommandLineTools \
CARGO_PROFILE_RELEASE_STRIP=none CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
CARGO_TARGET_DIR=target/xctrace cargo build --release -p tsrs_cli --locked
```

From a public project's root, use an absolute binary path:

```sh
/usr/bin/time -l "$TSRS_PROFILE_BINARY" -p "$TSRS_PROFILE_PROJECT" \
  --noEmit --incremental false --listFilesOnly --pretty false > /tmp/frontend-files.txt

TSRS_FRONTEND_STATS=1 "$TSRS_PROFILE_BINARY" -p "$TSRS_PROFILE_PROJECT" \
  --noEmit --incremental false --listFilesOnly --extendedDiagnostics --pretty false > /tmp/frontend-phases.txt
```

Once xctrace starts successfully, the pending recording command is:

```sh
xcrun xctrace record --template 'Time Profiler' --output /tmp/tsrs-frontend.trace \
  --launch -- "$TSRS_PROFILE_BINARY" -p "$TSRS_PROFILE_PROJECT" \
  --noEmit --incremental false --listFilesOnly --pretty false
xcrun xctrace export --input /tmp/tsrs-frontend.trace --toc --output /tmp/tsrs-frontend-toc.xml
```

This command is not a verified recording. Select export schemas from the actual TOC. Capture full checks too,
separating stacks by their calling phase; helper functions called by the checker are not automatically frontend
work merely because their crate is outside `tsrs_checker`.

Artifacts are in `/tmp/tsrs-nonchecker-xctrace/`: preflight JSON, binary provenance, `measure.py`, raw time/output
files, baseline and stats summaries, `sample-paused.py`, stack reports, `analyze-samples.py`, sample analysis and
lint logs. Invalid sandbox counter attempts were discarded before restarting: macOS `time -l` needs access to
`kern.clockrate`. Across the 48 valid baseline runs, full-check diagnostic text and exit codes match per project
across modes/repeats; list-only output and exit codes match separately. All 12 paused profile outputs also match
their frontend baselines. Both required lint scripts pass (three existing clippy findings, none new). No compiler
behavior, README capability status or lint baseline changed. Emit and LSP were not profiled.
