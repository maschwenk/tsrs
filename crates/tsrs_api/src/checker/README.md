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

`coverage_table.rs` has one row per `Method` constant in pinned `proto.go` (172, same order as core's
`methods.rs`). 115 are owned by this lane (the 111 checker/symbol/type/signature methods plus the four
language-service backed ones: `getCompletionsAtPosition`, `getReferencedSymbolsForNode`,
`getSignatureUsages`, `getImportAdderEdits`). `tests.rs` checks the table against core's inventory and
`ts-ref/tsc/internal/api/proto.go` (when checked out) and that every `Implemented`/`Tested` row is
dispatched.

Status today: all 115 are `Tested` = dispatched and exercised by a real-program Rust test (real
`tsrs_project` snapshot of a tsconfig project + the program's persistent API checker; fixtures cover
generics, unions, aliases, overloads, `.d.ts` imports, labeled tuples, mapped / conditional /
substitution / template-literal types, enums, JSDoc, unicode identifiers and UTF-16 positions after
non-BMP text, plus stale / cross-project / released / malformed handle validation).

What `Tested` does **not** mean yet (open gaps, for the integration lead):

* `pinned_go_differential` compares 71 results against `testdata/go_probe_b85298b6.jsonl`, recorded from
  pinned Go's `Session.HandleRequest` (built from b85298b6 with go1.27.1) over the same fixture and
  request sequence: type strings and flags at 40 positions, symbol names/flags/ownership, file-owned
  lookups without a snapshot (parent, members, exports, export symbol), `resolveName` with and without a
  location, type-parameter constraints, and stale / foreign-project / forged / released handle errors.
  All 71 match. The one tolerated difference is wording: on a malformed `type` param tsrs says
  `field "type": expected an unsigned integer in range`, Go prints its json/v2 unmarshal error. Both
  are `api: invalid request`. `objectFlags` are excluded: they include lazily computed cache bits
  (`MembersResolved`), and those already differ between two Go queries of the same type. This is a
  Go-server comparison, not a Node-client run; the broad upstream client suite belongs to the parity lane.
* Through core's `Session` every handler that returns or accepts a node handle, a source-file descriptor
  or an encoded node fails with an explicit `api: unsupported` until `tsrs_api_codec` provides node
  index tables / `EncodeNode` and core provides `newSourceFileDescriptor` / the source-file cache
  (`SessionHost` in `checker.rs`). In practice that is nearly every method (symbol responses carry
  declaration handles). The unit tests use a test-only node table instead.
* File-owned symbol references without a snapshot (`getParentOfSymbol`, `getMembersOfSymbol`,
  `getExportsOfSymbol`, `getExportSymbolOfSymbol` on `kind: 0` references) need core's
  `getCachedSourceFile` lease cache.
* Panics from deep checker invariants are converted to errors by core's `catch_unwind`; the API checker
  is not discarded afterwards (Go keeps it too).
