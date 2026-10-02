# lsp-proto: `tsrs_lsproto` (Go `internal/lsp/lsproto` + `internal/jsonrpc`)

## Ported

| Go | Rust |
| --- | --- |
| `lsproto/_generate/generate.mts` | `tools/gen-lsproto/generate.mts` (Go's model processing verbatim, Rust emitter) |
| `lsproto/_generate/fetchModel.mts` | `tools/gen-lsproto/fetchModel.mts`; `metaModel.json` + `metaModelSchema.mts` committed (vscode-languageclient 10.1.1, meta model 3.18.0) |
| `lsproto/lsp_generated.go` | `src/lsp_generated.rs` (generated; `node tools/gen-lsproto/generate.mts`) |
| `lsproto/lsp.go` | `src/lsp.rs` |
| `lsproto/util.go` | `src/util.rs` |
| `lsproto/jsonrpc.go` | `src/lsproto_jsonrpc.rs` (renamed: module `jsonrpc` is Go package `jsonrpc`) |
| `lsproto/baseproto.go` | `src/baseproto.rs` |
| `lsproto/structcodec.go` | `src/structcodec.rs` (helpers the generated codecs call) |
| `jsonrpc/jsonrpc.go`, `jsonrpc/baseproto.go` | `src/jsonrpc/jsonrpc.rs`, `src/jsonrpc/baseproto.rs` (`tsrs_lsproto::jsonrpc::…`) |
| json/v2 default codecs (reflection) | `src/json.rs`: `trait Json { GO_TYPE; to_json; from_json }`, impls for primitives / `Option` / `Vec` / `Box` / `OrderedMap` / `[u32; 2]` / `Value`, `JsonError` (= json/v2 `SemanticError` text) |
| Go `error` values with `ErrorCode`/`io.EOF` sentinels | `src/error.rs`: `Error { message, tags }`, `is_code`, `is_eof`, `as_error_code`, `wrap_code` (`fmt.Errorf("%w: %w", code, err)`) |
| `baseproto_test.go`, `lsp_test.go`, `lsp_json_test.go` | `src/baseproto_test.rs`, `src/lsp_test.rs`, `src/lsp_json_test.rs` |

API mapping (for porters): `lsproto.Foo` -> `tsrs_lsproto::Foo`; fields snake_case (`_vs_foo` -> `vs_foo`, keywords
get `_`: `type_`); optional `*T` -> `Option<T>` (`Box` only for `SelectionRange.parent`); required `*T` -> `T`;
`[]T`/`[]*T` -> `Vec<T>`; maps -> `OrderedMap` (insertion order; Go's order is random; a pointer value
`map[K]*V` -> `OrderedMap<K, Option<V>>`); `LSPAny` -> `Value`,
`LSPObject` -> `OrderedMap<String, Value>`; unions -> structs of `Option`s with Go's field names; enums -> newtypes
(`SymbolKind(pub u32)`, string enums `MarkupKind(pub &'static str)`, interned on decode, so `Copy` and usable as
`match` patterns) with consts `SymbolKind::File`; `Method(pub &'static str)` with `Method::TextDocumentHover`;
`lsproto.TextDocumentHoverInfo` -> `TEXT_DOCUMENT_HOVER_INFO: RequestInfo<HoverParams, HoverResponse>`;
`req.UnmarshalParams[*T]()` -> `req.unmarshal_params::<T>()`; `info.UnmarshalResult(resp.Result)` ->
`info.unmarshal_result(resp.result.as_ref())`; `TextDocumentURI()` / `TextDocumentPosition()` / `GetLocation(s)()`
-> traits `HasTextDocumentURI` (returns `&DocumentUri`) / `HasTextDocumentPosition` / `HasLocation(s)`;
`(*ClientCapabilities).Resolve()` -> `ClientCapabilities::resolve()`; `String()` -> `string()` + `Display`.
Derives: `Clone, Debug, Default, PartialEq` everywhere, plus `Copy` when all fields are (`Position`, `Range`, …) and
`Eq, Hash` when no float/any/map is inside. Outbound `RequestMessage.params` / `ResponseMessage.result` are
`Option<Value>` (filled with `to_json()` by `new_request_message` etc.); Go keeps `any` and marshals at write time.

Gates: `cargo check -p tsrs_lsproto` 0 warnings (1.4 s clean check of the crate, 6.9 s debug build); `cargo test -p
tsrs_lsproto` green (30 ported unit tests + oracle); `python3 tools/gen-lsproto/check.py`: 3686 Go names checked,
0 missing; regenerating reproduces `lsp_generated.rs` byte for byte.

Differential test: `tools/oracle/lsproto/` (Go program `tsrs-oracle-lsproto`, copy in `ts-ref/tsc/cmd/`) fills every
protocol type with random values by reflection, mutates them (dropped / null / wrong-kind / unknown / reordered
fields, wrong top-level kind) and adds hand-written edge cases; `run.sh` records Go's `json.Unmarshal` +
`json.Marshal` result per line in `crates/tsrs_lsproto/tests/testdata/oracle.txt` (6869 cases, all 569 types) and
`DocumentUri.FileName()` results in `filename.txt`. `tests/oracle.rs` requires identical output and identical error
text (except json/v2's "after offset N"). Three extra seeds × 4 samples (40494 cases) also pass (not committed:
`tsrs-oracle-lsproto gen <seed> 4 | python3 tools/oracle/lsproto/mutate.py`, then `check`).

## Shared-file edits

- `Cargo.lock`: `tsrs_lsproto` depends on `tsrs_vfs` (for `bundled::is_bundled` in `DocumentUri::file_name`).
- `ts-ref/tsc/cmd/tsrs-oracle-lsproto/` (oracle program, as PORTING.md allows; source of truth in `tools/oracle/lsproto/`).

## Deviations

- Decoding goes through a parsed `Value`, not a token stream. Consequences: (1) json/v2 rejects `1.0` / `1e2` for
  integer fields ("invalid syntax"); the port accepts integral numbers in any notation. (2) Error texts are
  json/v2's (`json: cannot unmarshal … into Go lsproto.X within "/ptr": …`, pointer semantics including the fresh-
  decoder rules of buffered unions and replayed discriminated fields), except top-level errors lack json/v2's
  `after offset N`, raw values in messages are re-marshaled compactly, syntax errors of the JSON text itself have the
  `tsrs_core::json` parser's wording, and json/v2 says "cannot" or "unable to" at random per process (we: "cannot").
  (3) Discriminator and literal values are compared decoded (Go compares raw bytes, so `"begin"` differs).
- `RequestMessage.params` and `ResponseMessage.result` are `Option<Value>` built eagerly; Go's
  `UnmarshalParams` error "unexpected params type %T" (params that are not a raw json.Value) cannot occur.
  `ResponseMessage.raw_result` reproduces Go writing `"result":null` when re-marshaling a decoded response that had
  no result.
- (ported by the lead after Context landed, `Arc<ResolvedClientCapabilities>` for Go's pointer) `WithClientCapabilities` / `GetClientCapabilities` (lsp.go:289/293) were not ported: `tsrs_core::context::Context`
  is not on this branch yet (it is on `lsp-api`). With it they are:
  `ctx.with_value(clientCapabilitiesKey(Arc<ResolvedClientCapabilities>))` and
  `ctx.value::<clientCapabilitiesKey>().map(|k| k.0.clone()).unwrap_or_default()`.
- `jsonKeyCheck` (lsp.go:124) has no counterpart (keys are compared decoded).
- `Resolved*` capability views have no JSON codec (Go gives them json tags but never marshals them).
- `BenchmarkUnmarshalDiscriminatedUnion` (and its `bufferedWorkDoneProgressUnion`) not ported.
- Generated doc comments are `//` like Go's (not `///`, so rustdoc does not compile LSP spec snippets as doctests).

## Needs from others

- `tsrs_core::context::Context` (lsp-api) to add the two client-capabilities context helpers above.

## Doubts

- String enums and `Method` are `&'static str` newtypes, interned on decode (bounded by the distinct values clients
  send). If a porter needs to build one from a dynamic string, `CodeActionKind(intern(&s))`.
- Response type aliases for Go pointer results (`InitializeResponse = *InitializeResult`) are the plain struct (Go
  never returns nil there).
