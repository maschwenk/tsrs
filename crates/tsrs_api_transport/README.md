# tsrs_api_transport

Transport for the tsgo-compatible programmatic API (`tsrs --api`, `tsrs --api --async`), ported from
microsoft/TypeScript at the commit pinned in the workspace `Cargo.toml`
(`[workspace.metadata.typescript].commit`). Upstream sources (Apache-2.0, see the repository `NOTICE`):

| this crate | pinned upstream |
| --- | --- |
| `msgpack.rs` | `tsc/internal/api/protocol_msgpack.go` |
| `jsonrpc.rs` | `tsc/internal/jsonrpc/baseproto.go`, `jsonrpc.go`, `tsc/internal/ipc/protocol_jsonrpc.go` |
| `conn_sync.rs` | `tsc/internal/ipc/conn_sync.go` |
| `conn_async.rs` | `tsc/internal/ipc/conn_async.go` |
| `timing.rs` | `tsc/internal/ipc/timing.go` |
| `transport.rs` | `tsc/internal/ipc/transport*.go`, connection setup in `tsc/internal/api/server.go` |
| `callbackfs.rs` | `tsc/internal/api/callbackfs.go` |
| `requestfs/` (feature `requestfs`) | `tsc/internal/api/requestfilesystem/*.go` |

## Contract with the API session

- `Handler::handle_request(cx, method, params) -> Result<Response, ApiError>`: `params` is the raw
  payload (JSON text, or empty). `Response::Json(bytes)` must be one JSON value (validated before it is
  written; an invalid value becomes an error response instead of a corrupt frame). `Response::Binary`
  is `api.RawBinary`: verbatim tuple payload in MessagePack mode, a base64 JSON string under JSON-RPC
  (what Go's `json.Marshal([]byte)` produces). `ApiError { code, message }`: the client sees `message`
  (MessagePack `Error` tuple payload / JSON-RPC `error.message`). Session errors use
  `CODE_INTERNAL_ERROR`, as `ipc.handleRequest` does. Panics are caught and reported as `panic: <msg>`.
- `getServerTiming` / `resetServerTiming` are answered by the connection (`ConnOptions.collect_timing`
  = `--timing`), never by the handler.
- `Caller::call(method, params)` (server-to-client) is obtained before the connection runs
  (`LateCaller`, Go's `SetConnection`) and can be used from any thread while a request is in flight.
  - Sync (MessagePack): calls are serialized; the response is read inline. A client request that
    arrives while waiting (the client re-entering the API from a callback) is dispatched on the waiting
    thread with the protocol lock released and `RequestContext.depth >= 1`. A handler must not hold a
    non-reentrant resource (an exclusive checker lease, a session mutex) across a callback if a nested
    request could need it: reject such nested requests with an error instead of blocking.
  - Async (JSON-RPC): every request runs on its own thread (Go: goroutine), responses are routed by
    `"api<N>"` IDs, and no lock is held while waiting.
- Run ends on EOF (`Ok`) or a fatal transport error. On exit the `CancellationToken` is set, pending
  calls fail with `ipc: connection closed`, and (async) all handler threads are joined.
- stdout is the wire. Nothing else in the process may write to stdout once the transport starts;
  diagnostics go to stderr.

## Adapter for core (`crates/tsrs_cli/src/api.rs`, owned by core)

`tsrs_api::handler` (core) and this crate have the same shape; the CLI adapts them without either
crate depending on the other:

```rust
struct SessionHandler(Arc<tsrs_api::Session>);
impl tsrs_api_transport::Handler for SessionHandler {
    fn handle_request(&self, _cx: &RequestContext, method: &str, params: &[u8]) -> Result<transport::Response, transport::ApiError> {
        match self.0.handle_request(method, params) {
            Ok(tsrs_api::Response::Json(s)) => Ok(transport::Response::Json(s.into_bytes())),
            Ok(tsrs_api::Response::Binary(b)) => Ok(transport::Response::Binary(b)),
            Err(e) => Err(transport::ApiError::internal(e.to_string())), // Go: CodeInternalError + err.Error()
        }
    }
    fn handle_notification(&self, _cx: &RequestContext, method: &str, params: &[u8]) {
        let _ = self.0.handle_notification(method, params);
    }
}
struct ClientConnAdapter(Arc<dyn tsrs_api_transport::Caller>);
impl tsrs_api::handler::ClientConn for ClientConnAdapter {
    fn call(&self, method: &str, params: &str) -> tsrs_api::ApiResult<String> {
        let raw = self.0.call(method, Some(params.as_bytes())).map_err(|e| ApiError::internal(e.to_string()))?;
        String::from_utf8(raw).map_err(|e| ApiError::internal(e.to_string()))
    }
}
// main (server.go Run):
let options = ServeOptions { protocol, collect_timing: timing, max_frame_bytes: DEFAULT_MAX_FRAME_BYTES };
tsrs_api_transport::serve(pipe.as_deref(), options, |caller| {
    let mut fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    if !callbacks.is_empty() || case_sensitive.is_some() {
        let cb = Arc::new(CallbackFs::new(fs, &CallbackConfig::parse(&callbacks)?, case_sensitive));
        cb.set_connection(caller.clone());
        fs = cb;
    }
    let session = Session::new(SessionOptions { fs, binary_responses: !is_async, .. });
    session.set_connection(Arc::new(ClientConnAdapter(caller)));
    Arc::new(SessionHandler(session))
})
```

`--callbacks` is a comma-separated list (`CallbackConfig::parse`; unknown names are an error).
Exclusive checker leases are not reentrant: in sync mode a nested request arrives on the thread that
is blocked in a filesystem callback (`RequestContext.depth >= 1`, not visible through core's
`Handler`); a session that holds a lease or a session-wide lock while reading files must not let a
nested request block on it.

## Request filesystem (feature `requestfs`)

`requestfs::new_for_update(params, base: &AnyFs, cwd, &mut FileChangeSummary) -> Result<AnyFs, String>`
is Go `NewForUpdate`: `full` replaces the host, `layer` overlays it, `removedPaths` are negative lookups,
explicit directory listings, request and host symlinks (cycle-safe), layer-over-layer compaction, and
file-change summaries with symlink aliases. `RequestFs` implements `FS`, `FileHandleSource`,
`LayeredFileSystem` (`overlays`) and `FileChangeExpander`, and exposes `base_file_system` /
`with_base_file_system` (Go `RebasableFileSystem`). Remaining integration (core / tsrs_project):
`layer_overlay_file_system` must rebase a request filesystem over the overlay FS the way Go's
`layerOverlayFileSystem` does (tsrs_project's `FsRef` has no rebasable variant yet), and errors map to
`api: client error: ...`. Duplicate JSON keys in `files`/`directories` are last-wins here (Go's decoder
rejects them).

## Tests

- `tests/go_frames.rs`: byte-exact writes and reads against frames produced by the pinned Go code.
- `tests/conn.rs`: real OS pipes: split frames, frame limits, callbacks, nested requests, remote
  errors, EOF with calls pending, protocol violations.
- `tests/node_roundtrip.rs` + `tests/node/roundtrip.test.mjs`: the pinned Node `sync` and `async`
  clients (from `ts-ref`, `node --conditions=@typescript/source`) against
  `examples/transport_test_server.rs`, including fs callbacks with every serverFS sentinel, callback
  re-entry, astral/lone-surrogate text, 5 MiB payloads, timing, server exit, and `--pipe` sockets.
  Skipped with a message if node or ts-ref is missing (`TSRS_REQUIRE_NODE_TESTS=1` makes that fail).
- `tests/requestfs_differential.rs` (feature `requestfs`): 142 query results and every step's
  file-change summary compared with the pinned Go request filesystem over the same scenarios, plus a
  real-OS-filesystem layering test.

## Deliberate differences from Go

- Frames are bounded (`ServeOptions.max_frame_bytes`, default 1 GiB) and allocated as bytes arrive,
  never from the declared length; header lines are capped at 8 KiB.
- A frame truncated after its first byte is an explicit `UnexpectedEof` error; Go reports some of
  these as a clean `io.EOF`.
- JSON syntax error texts come from serde_json, not encoding/json/v2.
- Lone UTF-16 surrogates in callback string values (`\udXXX` from JSON.stringify) decode to U+FFFD.

## Not supported

- Windows named pipes (`--pipe` on Windows returns an `Unsupported` error). The pinned sync Node client
  always uses `--pipe` on Windows, so the sync client is unsupported there.
- `$/cancelRequest` and other notifications are ignored, exactly as the pinned session does.
- External content-mapper spawning (`--runExternalCode`) is not part of this crate.

## Fixtures

`tests/fixtures/requestfs_go.json` comes from `requestfs_gen.go.txt` placed in the pinned
`tsc/internal/api/requestfilesystem` package: `GOWORK=off SCENARIOS=<requestfs_scenarios.json>
SCENARIOS_OUT=<out> go test -run ZZDump ./internal/api/requestfilesystem/`.

`tests/fixtures/go_frames.json` was produced by running `tests/fixtures/go_frames_gen.go.txt` as a Go
test in a package next to the pinned `protocol_msgpack.go` (renamed package) inside the pinned
`tsc` module: `GOWORK=off FRAMES_OUT=... go test ./internal/zzframes/`.
