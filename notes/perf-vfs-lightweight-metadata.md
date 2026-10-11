# Lightweight VFS metadata probe (2026-10-11)

## Question

Can resolver existence checks avoid building a complete `FileInfo`? `Common::file_exists` and
`Common::directory_exists` used `stat`, even though they only inspect whether the result is a directory.

The experiment adds `IoFS::file_type`, with a default implemented through `stat` so custom file systems retain their
behavior. The OS implementation avoids the basename allocation, size lookup and modification-time conversion. On
Linux it uses `statx(..., STATX_TYPE)` with normal `stat` synchronization semantics, falling back to
`std::fs::metadata` when `statx` is not available. Other platforms use `std::fs::metadata` but still avoid
constructing `FileInfo`.

## Correctness

- `cargo test -p tsrs_vfs`: 88 passed.
- `cargo check --workspace`, `cargo check -p tsrs_wasm --target wasm32-wasip1`, the Linux cross-target
  `cargo check -p tsrs_vfs --target x86_64-unknown-linux-gnu`, `tools/lint/ratchet.py`, and
  `tools/lint/source.py` pass.
- The candidate and base produce byte-identical diagnostics and the same exit status on the public
  `sentry-javascript` checkout used below.

## macOS screening

Base is `c21a36ef`; both binaries are plain release builds. The command is `tsrs -p <public sentry-javascript
checkout> --noEmit`, with five interleaved repetitions per binary. The single-threaded rows add
`--singleThreaded`. Values are medians from `/usr/bin/time -l` on arm64 macOS.

| mode | metric | base | lightweight metadata | change |
| --- | ---: | ---: | ---: | ---: |
| single-threaded | instructions retired | 6,701,849,301 | 6,683,929,539 | -0.27% |
| single-threaded | peak RSS | 241,156,096 B | 238,813,184 B | -0.97% |
| single-threaded | wall | 0.86 s | 0.85 s | -1.2% |
| default | peak RSS | 269,008,896 B | 267,698,176 B | -0.49% |
| default | wall | 0.75 s | 0.75 s | 0.0% |

Single-threaded extended-diagnostic counts are identical: 6,785 files, 17,209 stat calls, 6,960 reads and 3,418
directory reads. The syscall count should not change; the experiment reduces work within each existence probe.

## Verdict

The macOS result is directional but below every landing bar. It also does not exercise Linux's `statx(TYPE)` path,
and macOS `/usr/bin/time` is not the repository's deterministic Linux instruction gate. Keep this as an experimental
branch; before landing, run `bench/count.py` and peak-RSS measurements on the pinned benchmark projects. Do not claim
a qualifying improvement from this screening result.
