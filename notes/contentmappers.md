# Content mappers: port plan and status

Go reference: `ts-ref/tsc/internal/` at the pinned commit (`Cargo.toml` `[workspace.metadata.typescript]`). Go is the
spec (`CONTRIBUTING.md`); every ported function keeps its Go name, structure and order (`docs/PORTING.md`), with a
`// <file>.go:<line>` comment above it.

## What a content mapper is

A content mapper is an external process, declared in a tsconfig `contentMappers` entry and described by the
`typescript.contentMapper` object of its npm package's `package.json`, that turns otherwise unsupported files
(`.vue`, `.astro`, `.svelte`) into virtual TypeScript during program construction. The compiler spawns it with
`--runExternalCode`, talks JSON-RPC over its stdio (Content-Length frames; methods `initialize`, `openProject`,
`transform`, `closeProject`), parses the returned text as `<file>.<ext>` (plus unnamed supplemental outputs
`<file>.<n>.<ext>`), and keeps a span map that maps virtual positions back to the original text so diagnostics are
reported at their original locations. Go's design: `contentmapper/contentmapper.go` (definitions), `host.go` (error
kinds, result types, `Project` and `Host` interfaces), `hostimpl.go` (process host, project leases, protocol decode
and position normalization), `transform.go` (parse into source files); `spanmap/spanmap.go` (the span map);
`ipc/` + `jsonrpc/` (the connection); hooks in `compiler/fileloader.go`, `filesparser.go`, `program.go`,
`emitter.go`, `host.go`, `diagnosticwriter/diagnosticwriter.go`, `ast/ast.go`, `ast/diagnostic.go`,
`execute/tsc.go`, `execute/tsc/compile.go`, `execute/tsc/statistics.go`, `execute/build/*.go`,
`execute/incremental/*.go`; tests in `contentmapper/*_test.go`, `spanmap/spanmap_test.go`,
`compiler/contentmapper_test.go`, `tsoptions/contentmappers_test.go`, `execute/tsctests` (7 `tsc` + 2 `tsbuild`
scenarios) with the in-process test mappers of `testutil/contentmappertest`.

## Scope

Phase 1 (this branch): the compiler, `tsrs` (tsc), `tsrs -b`, incremental build info, and `tsrs --api
--runExternalCode` accepting the flag. Phase 2 (later): the language server and the API project system
(`project/`, `lsp/server.go`, `ls` span-map features; 55 fourslash tests). `--watch` is not ported in tsrs, so the
watch hooks (`execute/watcher.go`, `tsctests/contentmapper_watch_test.go`) stay out.

## Crate layout

Rust cannot mirror Go's package graph exactly: `tsrs_api_transport` (Go `api` transport + `ipc`) optionally depends on
`tsrs_project`, which depends on `tsrs_compiler`, so anything the compiler depends on cannot use
`tsrs_api_transport`. Hence:

| Go | Rust | notes |
| --- | --- | --- |
| `spanmap/spanmap.go` | new crate `crates/tsrs_spanmap` (`spanmap.rs`) | deps: `tsrs_core` only. `tsrs_ls/src/spanmap.rs` re-exports it (its value types `Feature`, `Fidelity`, `MappedPosition`, `MappedSpan` already match Go) |
| `ipc/protocol.go`, `protocol_jsonrpc.go`, `conn.go`, `conn_async.go`; `jsonrpc/jsonrpc.go`, `baseproto.go` | new crate `crates/tsrs_ipc` | deps: `tsrs_core` (json). Only what the mapper host needs: an async JSON-RPC connection over a `Read`+`Write` pair, concurrent `call`, incoming requests answered by a `Handler`, EOF/error termination. `tsrs_api_transport` keeps its own copy for now (it predates this crate); a later change may make it reuse `tsrs_ipc` |
| `contentmapper/contentmapper.go` (`Definition`, `Manifest`, `Mapper` and methods, `IsSupportedVirtualExtension`), `tsoptions/contentmappers.go` | `crates/tsrs_tsoptions/src/contentmappers.rs` | the definitions stay in tsoptions (Go's tsoptions imports contentmapper; the host crate below imports tsoptions instead). `OptionPathSegment` lives here too |
| `contentmapper/host.go`, `hostimpl.go`, `transform.go` | new crate `crates/tsrs_contentmapper` (`host.rs`, `hostimpl.rs`, `transform.rs`, `process.rs`) | deps: `tsrs_core`, `tsrs_diagnostics`, `tsrs_ast`, `tsrs_parser`, `tsrs_spanmap`, `tsrs_tsoptions`, `tsrs_ipc`. `process.rs` is `cmd/tsc/sys.go` `spawnProcess`/`childProcess` (std::process) |
| `testutil/contentmappertest/*.go` | new crate `crates/tsrs_contentmappertest` | the in-process test mappers and spawner, used by `tsrs_execute`'s tsctests harness (and the fourslash harness in phase 2) |
| `ast/ast.go` content-mapper fields and methods, `ast/diagnostic.go` `displayMessageArgs` | `crates/tsrs_ast` | `tsrs_ast` depends on `tsrs_spanmap` |
| compiler hooks | `crates/tsrs_compiler` | `host.rs` (`CompilerHost` methods), `fileloader.rs`, `filesparser.rs`, `file_include.rs`, `program.rs`, `emitter.rs` |
| `diagnosticwriter/diagnosticwriter.go` `resolve`, `MessageChain` note | `crates/tsrs_diagnostics` (the writer port) | |
| `execute/tsc.go`, `tsc/compile.go`, `tsc/statistics.go`, `build/*.go`, `incremental/*.go` | `crates/tsrs_execute`, `crates/tsrs_incremental` | |

Go `json.Value` is `tsrs_core::json::Value`; Go `json.Marshal` of structs is written by hand with
`tsrs_core::json` (field order = Go struct order, `omitempty`/`omitzero` honored) because tsrs has no serde.

## Threading and memory

Content-mapped files are parsed inside the file loader's parallel parse, so the host is `Send + Sync` (mutexes
where Go has them) and `transform` is called concurrently from several threads; the async connection supports that.
`SourceFile` keeps its content-mapper info in a field written once before the file is published (`OwnedCell`, like
the other parser-set fields); span maps are arena objects (`P<SpanMap>`), original text is `&'static str`.

## Not ported (recorded, not silently dropped)

- The language server and API project system (phase 2).
- `--watch` hooks.
- Go's `context` cancellation: the host is closed explicitly (`Host::close`, also on drop of the owning session).

## Verification

- `tsrs_spanmap`, `tsrs_contentmapper` unit tests: ports of `spanmap_test.go`, `transform_test.go`, the host tests of
  `host_test.go` that do not need a real process, `compiler/contentmapper_test.go`, `tsoptions/contentmappers_test.go`.
- tsctests: `tools/oracle/tsctests/dump.sh` (needs Go; `target/tsctests-dump`), then
  `TSCTESTS_FILTER=contentMapper cargo test --release -p tsrs_execute tsctests` with the content-mapper filter in
  `crates/tsrs_execute/src/tsctests/mod.rs` removed. All 7 `tsc` and 2 `tsbuild` scenarios must match their Go
  baselines byte for byte.
- A real out-of-process mapper: `testdata/contentmapper/` runs a small Node mapper through the production spawner.
- The usual gates: `cargo check --workspace --locked` (CI uses `-D warnings`), `tools/lint/ratchet.py`,
  `tools/lint/source.py`, `tools/gen-check.sh`, the conformance and fourslash suites unchanged, `pr-verify`.

## Status

Wave 1 (foundations) is done. What later waves build on:

- `tsrs_spanmap` (all of `spanmap.go`; `spanmap_test.go`, 31 tests). Methods take `&SpanMap`. Rust cannot give an
  inherent method and an associated function the same name, so each method Go calls on a possibly-nil receiver also
  exists as a free function of that name taking `Option<&SpanMap>`, with Go's nil branch:
  `tsrs_spanmap::virtual_to_original_span(file.span_map().as_deref(), loc)`. `new` and `unmarshal` return
  `P<SpanMap>`. `Kind` is a newtype over `i32`, like `ModuleKind`, because a decoded map can carry any kind (which
  `validate` reports). `alias_for_virtual_span` returns `Option<Segment>`. `virtual_to_original_position_exact`
  returns `(TextPos, bool)` because Go also returns the mapped position with `false`. `tsrs_ls::spanmap` re-exports
  the crate.
- `tsrs_ast`: `SourceFile::set_content_mapper_info` (panics if already set) and the accessors (`span_map()` is
  `Option<P<SpanMap>>`). `ContentMapperSourceFileInfo` holds `&'static` strings and slices, so build them with
  `alloc_str` / `alloc_slice`. Also `MappedDiagnosticDirective` / `MappedDiagnosticDirectivePolicy`, and the alias
  substitution in `Diagnostic::display_message_args`.
- `tsrs_tsoptions::contentmappers`:
  - `is_supported_virtual_extension`, `Mapper::{diagnostic_name, identity, transform_identity,
    marshal_declared_options}`, `OptionPathSegment` and `resolve_content_mapper_manifest`.
  - `transform_identity` returns a `u128` (Go `xxh3.Uint128`). Go's `fmt.Sprintf("%x", id.Bytes())` is
    `format!("{id:032x}")`.
  - Go `json.Value` is the raw JSON text in a `String` (`Definition::options`, the values of
    `marshal_declared_options`). `gojson::compiler_option_to_go_json` marshals one `CompilerOptions` field by json tag.
  - `tsoptions::get_content_mapper_option_diagnostic_location` finds the mapper in `content_mappers()` by address
    (Go compares `*Mapper`), so pass a reference into that list.
  - `ErrProjectUnavailable` (`contentmapper.go:26`) is left for the host crate, which owns the error type.
- Already ported with the definitions, so the host and incremental waves should not port them again:
  `TestMapperDiagnosticName` (`host_test.go`) and `TestStaticContentMapperTransformIdentity`
  (`execute/incremental/buildinfo_contentmapper_test.go`), in `tsrs_tsoptions/src/contentmappers_test.rs`.
