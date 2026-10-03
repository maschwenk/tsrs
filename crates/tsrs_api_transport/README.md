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

`tests/fixtures/go_frames.json` was produced by running `tests/fixtures/go_frames_gen.go.txt` as a Go
test in a package next to the pinned `protocol_msgpack.go` (renamed package) inside the pinned
`tsc` module: `GOWORK=off FRAMES_OUT=... go test ./internal/zzframes/`.
