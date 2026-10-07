# mem-zero-copy-libs: the bundled libs are parsed from the binary's text

The default libraries (`lib.*.d.ts`) are in the binary through `include_str!` (crates/tsrs_vfs/src/bundled/embed.rs,
`EMBEDDED_CONTENTS`), served under `bundled:///libs/`. The compiler host read them like any other file:
`FS::read_file` returned a `String` copy of the embedded text (embed.rs, `WrappedFS::read_file`) and
`parse_source_file_owned` leaked that copy as the file's `&'static str` (crates/tsrs_parser/src/parser_1.rs). So every
run held each lib's text twice, once in the binary and once on the heap, for the life of the process. `lib.dom.d.ts`
alone is 2.35 MB; vscode's lib set (ES2024, ESNext.Disposable, DOM, DOM.Iterable, WebWorker.ImportScripts) is about
2.9 MB.

Found by reading Bun's `bun check` (oven-sh/bun PR 44361), which drops the default-library text under `skipLibCheck`
and falls back at about 22 checker sites that would have read it. That transfer was rejected: tsrs's compact
identifiers read their text out of the registered source text, and diagnostics about lib declarations quote it, so
the text has to stay. What does transfer is the zero-copy half: the text is already static.

## Change

- `tsrs_vfs::bundled::embedded_file(path)` returns the embedded `&'static str` of a `bundled:///` path (the function
  behind the previously unused `WrappedFS::read_embedded_file`).
- `tsrs_parser::parse_source_file_embedded(opts, &'static str, kind)`: outside a region it is the static parse entry
  (`parse_source_file_static`, which registers the text for compact identifiers), with no copy and nothing to leak;
  inside a region (the language server) it behaves like `parse_source_file_owned`: the region holds the text the file
  reads and forgets its slot when it is freed.
- `compilerHost::get_source_file` (crates/tsrs_compiler/src/host.rs) tries `embedded_file` first and otherwise reads
  through the file system as before. Disk libs (`--lib` paths, the `noembed` layout) are unaffected.

The file reads the same bytes through the same kind of `&'static str`, so identifiers, line maps and diagnostics are
unchanged; `--listFiles` still prints the `bundled:///` names.

## Measurement

vscode `src` (10,427 files), macOS M5 Max, `--features alloc-profile` build, `--noCheck` (the front end; the libs are
parsed once and shared by every checker, so the saving is the same at any checker count), two runs each:

| | main (05cfebf) | this change |
| --- | --- | --- |
| heap at exit (non-arena, live) | 186.6 MB | 183.8 MB (-2.8 MB) |
| heap at its peak | 241.2 MB | 238.9 MB (-2.3 MB) |
| arena requested | 873.9 MB | 873.9-874.0 MB |
| heap allocations | 10,199,729-10,199,893 | 10,200,217-10,200,879 |
| peak memory footprint | 1,264.6-1,266.1 MB | 1,258.3-1,264.0 MB |

2.8 MB is 0.1% of the 2.87 GiB peak at the 32-checker default on the 64-vCPU bench machine: the smallest item of the
Bun study (notes/bun-check-memory.md), taken because it is exact and costs nothing. The release-build diagnostics are
byte-identical to main's on vscode (371 errors), webpack (840) and xstate-main (0) at 4 checkers.

## Gates

`cargo check --workspace` clean; `tools/lint/ratchet.py` none new; `tools/lint/source.py` ok; `cargo test -p tsrs_vfs`.
The conformance suite runs in CI on the pull request.
