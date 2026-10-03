# Node API checker lane

Port of the checker / symbol / type / signature request handlers of the pinned tsgo API
(microsoft/TypeScript `b85298b6a81f772d080b0455de0ca9d744cd6fd6`, `tsc/internal/api/session.go` and
`proto.go`). Wire shapes, id ownership and nullability follow the Go structs and handlers one to one;
there is no separate "tsrs subset" API.

## Contract with the core session

The core lane owns the crate root, snapshots, source-file leases, the AST encoder / node index table and
dispatch. It integrates this module through:

```rust
// in core's request dispatch, before/after its own methods:
if let Some(result) = tsrs_api::checker::handle(&host, method, &params) { /* map result */ }
```

* `CheckerHost` (checker/host.rs) — implemented by core:
  * `context()` — request `Context`; handlers add `CheckerLifetime::API` and acquire the program's
    persistent API checker exactly once per request (exclusive, not reentrant; no callbacks run while
    it is held).
  * `snapshot(handle) -> SnapshotScope` — keeps the snapshot (programs, checker pools, source files)
    alive until the scope drops; carries that snapshot's `Arc<CheckerRegistry>`.
  * `acquire_cached_source_file(descriptor)` — Go `acquireCachedSourceFile` for file-owned symbol
    references that carry no snapshot.
  * `source_file_descriptor(file)` / `source_file_node_id(file)` — Go `newSourceFileDescriptor` /
    `sourceFileNodeID` (must match what `getSourceFile` responses report).
  * `node_handle(node)` / `resolve_node_handle(program, handle)` — Go `nodeHandleFrom` /
    `resolveNodeHandle` (`"<index>.<kind>.<path>"` over the encoder's node index table).
  * `encode_node(node)` — Go `encoder.EncodeNode(node, nil)` for `typeToTypeNode` /
    `signatureToSignatureDeclaration`; the handler returns `CheckerResponse::EncodedNode(bytes)` and the
    transport emits raw binary (msgpack) or `{ "data": base64 }` (JSON-RPC).
* `CheckerRegistry` (checker/registry.rs) — one per API snapshot handle, stored in core's snapshot data.
  Core must call `registry.release()` before releasing the snapshot's last API reference (and
  `release_project(id)` if a project is dropped from a still-live snapshot). After release every lookup
  is a client error; no raw pointer outlives its snapshot.
* Errors: `CheckerError { kind: InvalidRequest | Client, message }` = Go `ErrInvalidRequest` /
  `ErrClientError`. Handlers do not panic on bad input (wrong JSON types, out-of-range numbers, wrong
  type kind for a property, stale/foreign handles); panics from deep checker invariants are not caught
  here — core/runtime should convert them to an error response like Go's `recover` in the ipc loop.

## Id ownership (same as Go)

* Symbol ids: `tsrs_ast::get_symbol_id` (process-wide counter, not an address). Non-transient symbols of
  non-content-mapped files are *file-owned* (`reference.kind = 0`, carries the source-file descriptor) and
  are resolved through the file, never through a registry; all others are *snapshot-owned*
  (`kind = 1`, snapshot + canonical project) and registered snapshot-wide.
* Type / signature ids: per-checker ids, registered per (snapshot, project), and pinned to the id of the
  API checker that produced them. A replaced API checker (e.g. after cancellation) makes older handles a
  stale-handle error instead of resolving against another checker. Cross-project and cross-snapshot
  handles are rejected.
* Positions in requests are UTF-16 offsets, converted with the file's position map before
  `GetTouchingPropertyName`.

## Coverage

`coverage_table.rs` has one row per `Method` constant in pinned `proto.go` (172), with the owning lane and
this lane's status; `tests.rs` checks the table against `ts-ref` when present and checks every row marked
`Implemented`/`Tested` is actually dispatched. Status meanings: `Planned` (owned, not dispatched),
`Implemented` (dispatched, no dedicated real-program test yet), `Tested` (dispatched and exercised by a
real-program test in `tests.rs`).

Not owned here (recorded for the integration lead): the four language-service backed methods
`getImportAdderEdits`, `getReferencedSymbolsForNode`, `getSignatureUsages`, `getCompletionsAtPosition`
(lane `Ls`), and all snapshot / program / source-file / diagnostics / emit / config / build / module
resolution / print methods (lane `Other`).
