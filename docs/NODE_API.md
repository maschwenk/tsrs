# Node programmatic API (`tsrs --api`)

Port of the TypeScript 7 native API at microsoft/TypeScript `b85298b6a81f772d080b0455de0ca9d744cd6fd6`
(the commit in the workspace `Cargo.toml`): server side `tsc/internal/api` + `tsc/internal/ipc`, client
side `packages/typescript/src/{api,ast}` exported as `unstable/sync` and `unstable/async`. The wire
format, method names, params and response schemas are the pinned Go ones; tsrs does not define its own
protocol or a one-shot CLI wrapper, and it is not compatible with TS5 `createProgram` objects.

Status: **integration candidate, in progress**. The matrix at the end is the source of truth: `supported`
means a core Rust test against a real compiler session exercises it; `partial` means it is implemented
and dispatched but only covered by the upstream Node suites, a lane's own unit tests, or has a documented
divergence. Unimplemented methods return an explicit `api: unsupported: ...` error, never a fake success.

Evidence on the integration branch (pinned upstream client, `node tools/node-api/run-upstream.mjs
--binary target/debug/tsrs --suite upstream --filter <file>`): `test/sync/api.test.ts` 339/339 (three
consecutive runs; one earlier run had a single unreproduced failure), `test/async/api.test.ts` 348/348,
`sync/ast` 111/111, `sync/astnav` + `async/astnav` 4/4 each, `sync/api-generators` 43/43, parity `rss` 2/2.
These are suite results, not byte-level response parity; the parity lane's Go-oracle comparison is the
authority for that.

Allocator-perturbation run (`MALLOC_PERTURB_=165 TSRS_ARENA_POISON=1 cargo test --no-fail-fast -p <pkg> --
--skip released_snapshots --skip create_and_release --skip repeated_transpile --skip repeated_config`; the four
skipped tests measure RSS, which poison mode makes grow by design), counted per test binary at `c2abd68`:
`tsrs_api` lib unit tests 27 (mostly the checker lane's), `batch_test` 1, `callbackfs_test` 2, `config_test` 7,
`module_resolution_test` 2, `program_test` 9, `requestfs_test` 6, `sourcefile_test` 3, `transpile_test` 2
(`tsrs_api` total 59); `tsrs_project` 102 (2 ignored); `tsrs_lsp` 41. "197"/"202" in earlier reports are these
three packages summed at different heads; a count restricted to core-owned `tsrs_api` integration binaries is
32. All pass, 0 fail.

## Lanes and ownership

| lane | branch | owns |
|---|---|---|
| core (integration lead) | `mfs-cx/node-api-core` | `crates/tsrs_api` (except `src/checker.rs`, `src/checker/**`), `crates/tsrs_cli/src/api.rs`, `--api` dispatch in `main.rs`, root `Cargo.toml`/`Cargo.lock`, CLI dependency wiring, this document |
| runtime | `mfs-cx/node-api-runtime` | `crates/tsrs_api_transport` (MessagePack tuple protocol, JSON-RPC framing, sync/async conns, stdio/pipe transports, timing) |
| codec | `mfs-cx/node-api-codec` | `crates/tsrs_api_codec` (AST binary encoder/decoder, string table, node index tables, source-file id) |
| checker | `mfs-cx/node-api-checker` | `crates/tsrs_api/src/checker.rs`, `crates/tsrs_api/src/checker/**`, necessary checker exports |
| sdk | `mfs-cx/node-api-sdk` | `npm/**` (ported `unstable/sync` / `unstable/async` client, AST package, Node tests) |
| parity | `mfs-cx/node-api-parity` | `tools/node-api/**`, `.github/workflows/node-api.yml` |

Cross-lane changes go through core. Other lanes may use a local harness to compile but must not commit
copies of shared modules.

## Integration contract (Rust)

### Session boundary (core, `tsrs_api::handler`)

```rust
pub enum Response { Json(String), Binary(Vec<u8>) }   // Go `any` result / `RawBinary`
pub struct ApiError { pub kind: ErrorKind, pub message: String } // Display == Go err.Error()
pub enum ErrorKind { InvalidRequest, ClientError, Unsupported, Internal }
pub trait Handler: Send + Sync {                         // Go ipc.Handler
    fn handle_request(&self, method: &str, params: &[u8]) -> ApiResult<Response>; // params = raw JSON bytes ("" = absent)
    fn handle_notification(&self, method: &str, params: &[u8]) -> ApiResult<()>;
}
pub trait ClientConn: Send + Sync {                      // Go ipc.Conn.Call (server -> client)
    fn call(&self, method: &str, params: &str) -> ApiResult<String>; // JSON text in/out
}
tsrs_api::Session::new(SessionOptions { cwd, default_library_path, fs, binary_responses, run_external_code }) -> Arc<Session>
session.set_connection(Arc<dyn ClientConn>); session.close();  // Session: Handler
```

- `binary_responses` is true for the sync MessagePack protocol (Go `UseBinaryResponses`); source-file
  responses are then `Response::Binary`, otherwise `{"data": base64}` JSON.
- `echo` and `ping` are handled before method lookup exactly like Go. Unknown methods are
  `InvalidRequest` (`unknown API method "x"`).
- Both protocols put errors on the wire as `jsonrpc.CodeInternalError` with `err.to_string()`.
- Handler panics are caught and returned as `panic: ...` errors (Go conn recovers too).

### Runtime (requested from `tsrs_api_transport`)

The transport must compile without `tsrs_api`. It may define its own handler/conn traits with the same
shape; `crates/tsrs_cli/src/api.rs` (core) adapts them. Needed entry points:

- a sync conn running the msgpack `[type, method, payload]` tuple protocol over stdin/stdout or a pipe,
  supporting re-entrant `call` from inside a request (filesystem callbacks), and an async JSON-RPC conn
  with concurrent request dispatch and outstanding server->client calls;
- `RawBinary`-equivalent results written as msgpack `bin`; `getServerTiming`/`resetServerTiming`
  handled in the conn as in Go `ipc/timing.go`;
- deterministic shutdown: EOF on input ends `run` with `Ok`, malformed frames end it with an error
  without writing non-protocol bytes to stdout.

### Codec (requested from `tsrs_api_codec`)

- `encode_source_file(&SourceFile) -> Result<Vec<u8>, String>` (Go `encoder.EncodeSourceFile`) and
  `set_source_file_id(&mut [u8], u64)`;
- a per-file node index table (Go `encoder.GetNodeIndexTable`) mapping index <-> node so core can
  produce/resolve node handles `"<index>.<kind>.<path>"`;
- `decode_nodes(&[u8])` (Go `encoder.DecodeNodes`) for `printNode` / `formatNodeForInsertion` and
  `createSourceFile` round trips.
- Positions: the encoder itself writes UTF-16 positions (as the pinned Go encoder does); request positions
  from the client are UTF-16 and are converted with the file's position map.

### Checker hook (checker lane, `crates/tsrs_api/src/checker.rs`)

```rust
#[derive(Default)] pub struct CheckerSnapshotState { .. }   // per-snapshot registries, dropped with the snapshot
pub(crate) fn handle(session: &Session, method: &str, params: &json::Value) -> Option<ApiResult<Response>>;
```

Core provides (stable API for the checker lane):

- `Session::snapshot_data(SnapshotID) -> ApiResult<Arc<SnapshotData>>` (pins the snapshot);
- `Session::setup_checker(SnapshotID, &ProjectID) -> ApiResult<CheckerSetup>` with fields
  `sd`, `snapshot`, `project`, `program: &'static Program`, `checker: CheckerHandle`; the setup keeps the
  snapshot alive and holds the API-lifetime checker until dropped (not reentrant);
- `SnapshotData::get_program`, `SnapshotData::checker_state`;
- `session::parse_params`, `session::json_response`; node-handle helpers once the codec lands.

`handle` receives every method whose `methods::METHODS` owner is `Checker` (115 methods). `None` means
"not recognized" and becomes `ApiError::unsupported`.

## Lifetimes

- Snapshot handles map to `Arc<SnapshotData>`; `release` drops the registry reference and the project
  snapshot is dereferenced when the last in-flight request finishes.
- Region pointers (`P<T>`, `&'static Program`) never cross the wire and are only reached through a live
  `SnapshotData`. Handles from another session or from a released snapshot fail with `api: client error`.

## Coverage matrix

Pinned `proto.go` has 172 `Method` constants (core 57, checker 115). Kept in sync with
`crates/tsrs_api/src/methods.rs`.

| # | method | owner | status | evidence / gap |
|---|---|---|---|---|
| 1 | `release` | core | supported | program_test (refcounted; stale/cross-session handles rejected) |
| 2 | `releaseSourceFile` | core | supported | sourcefile_test |
| 3 | `retainSourceFile` | core | partial | descriptor validation tested; positive path via upstream suites |
| 4 | `getCachedSourceFile` | core | partial | via upstream suites only |
| 5 | `batchRequests` | core | supported | batch_test (nesting error, pagination with continuation tokens); binary-in-batch base64 path untested until source-file methods land |
| 6 | `initialize` | core | supported | config_test |
| 7 | `createSnapshot` | core | supported | program_test, module_resolution_test, requestfs_test (full/layer request filesystems, removedPaths); fileNotifications alias expansion through request symlinks not applied |
| 8 | `updateSnapshot` | core | supported | program_test, requestfs_test (layers compacted over full, retained base, release of base) |
| 9 | `getCurrentLanguageServerSnapshot` | core | partial | returns Go's standalone-session client error; LSP-attached sessions not ported |
| 10 | `createBuildOrchestrator` | core | supported | tsrs_cli api::tests (in-process CLI build backend; fresh orchestrator per call instead of Go's recheckAllProjects reuse) |
| 11 | `disposeBuildOrchestrator` | core | supported | tsrs_cli api::tests |
| 12 | `build` | core | supported | tsrs_cli api::tests (references, up-to-date rebuild) |
| 13 | `buildReferences` | core | supported | tsrs_cli api::tests |
| 14 | `cleanBuild` | core | supported | tsrs_cli api::tests |
| 15 | `cleanReferences` | core | partial | ported Go clean(onlyReferences); no dedicated test |
| 16 | `createModuleResolver` | core | supported | module_resolution_test (default, static entries, callback) |
| 17 | `releaseModuleResolver` | core | supported | module_resolution_test |
| 18 | `resolveModuleName` | core | supported | module_resolution_test (standalone, snapshot-scoped callback); inProgressSnapshot path exercised only via callbacks during program build |
| 19 | `parseCommandLine` | core | supported | config_test |
| 20 | `readConfigFile` | core | supported | config_test |
| 21 | `parseJsonConfigFileContent` | core | supported | config_test |
| 22 | `parseConfigFile` | core | supported | config_test |
| 23 | `createSourceFile` | core | supported | sourcefile_test, upstream sync suite |
| 24 | `createSourceFileFromFile` | core | partial | dispatches; exercised by upstream suites |
| 25 | `transpileModule` | core | supported | transpile_test |
| 26 | `transpileModuleFromFile` | core | supported | transpile_test |
| 27 | `transpileDeclaration` | core | supported | transpile_test (upstream api.test.ts expectations); observed: no TS9007 for `export function g() { return 1; }` under isolatedDeclarations, unverified against tsgo |
| 28 | `transpileDeclarationFromFile` | core | supported | transpile_test |
| 29 | `getDefaultProjectForFile` | core | supported | program_test |
| 30 | `getSymbolAtPosition` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 31 | `getSymbolsAtPositions` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 32 | `getSymbolAtLocation` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 33 | `getSymbolsAtLocations` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 34 | `getSymbolOfSourceFile` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 35 | `getSymbolsOfSourceFiles` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 36 | `getTypeOfSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 37 | `getTypesOfSymbols` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 38 | `getDeclaredTypeOfSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 39 | `getNonMissingTypeOfSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 40 | `getSourceFile` | core | supported | sourcefile_test (binary + base64, decoded by tsrs_api_codec), upstream sync/async suites |
| 41 | `getSourceFileNames` | core | supported | program_test |
| 42 | `getSourceFileMetadata` | core | supported | program_test |
| 43 | `getModeForUsageLocation` | core | partial | via upstream sync suite (Program resolved modules) |
| 44 | `getModeForResolutionAtIndex` | core | partial | via upstream sync suite |
| 45 | `getResolvedModule` | core | partial | via upstream sync suite |
| 46 | `getResolvedModuleFromModuleSpecifier` | core | partial | via upstream sync suite |
| 47 | `getResolvedTypeReferenceDirective` | core | partial | via upstream sync suite |
| 48 | `getResolvedTypeReferenceDirectiveFromTypeReferenceDirective` | core | partial | via upstream sync suite |
| 49 | `getConfigFileNames` | core | supported | program_test (extends chain, synthetic null) |
| 50 | `getConfigSourceFile` | core | partial | dispatches (root + extended configs); exercised only by upstream suites |
| 51 | `resolveName` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 52 | `getSymbolsInScope` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 53 | `getSignaturesOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 54 | `getResolvedSignature` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 55 | `getTypeAtLocation` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 56 | `getTypeAtLocations` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 57 | `getTypeAtPosition` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 58 | `getTypesAtPositions` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 59 | `getParentOfSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 60 | `getMembersOfSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 61 | `getExportsOfSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 62 | `getExportSymbolOfSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 63 | `getSymbolOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 64 | `getTargetOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 65 | `getFreshTypeOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 66 | `getRegularTypeOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 67 | `getTypesOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 68 | `getTypeParametersOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 69 | `getOuterTypeParametersOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 70 | `getLocalTypeParametersOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 71 | `getThisTypeOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 72 | `getAliasTypeArgumentsOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 73 | `getAliasSymbolOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 74 | `getObjectTypeOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 75 | `getIndexTypeOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 76 | `getCheckTypeOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 77 | `getExtendsTypeOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 78 | `getBaseTypeOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 79 | `getConstraintOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 80 | `getTypeParameterOfMappedType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 81 | `getConstraintTypeOfMappedType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 82 | `getNameTypeOfMappedType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 83 | `getTemplateTypeOfMappedType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 84 | `getTypeParametersOfSignature` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 85 | `getParametersOfSignature` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 86 | `getThisParameterOfSignature` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 87 | `getTargetOfSignature` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 88 | `getContextualType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 89 | `getContextualTypeForArgument` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 90 | `getAwaitedType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 91 | `getBaseTypeOfLiteralType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 92 | `getNonNullableType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 93 | `getTypeFromTypeNode` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 94 | `getWidenedType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 95 | `getParameterType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 96 | `getTypeParameterAtPosition` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 97 | `isArrayLikeType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 98 | `isTypeAssignableTo` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 99 | `getShorthandAssignmentValueSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 100 | `getTypeOfSymbolAtLocation` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 101 | `typeToTypeNode` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 102 | `signatureToSignatureDeclaration` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 103 | `typeToString` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 104 | `isContextSensitive` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 105 | `getReturnTypeOfSignature` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 106 | `getRestTypeOfSignature` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 107 | `getTypePredicateOfSignature` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 108 | `getBaseTypes` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 109 | `getPropertiesOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 110 | `getApparentPropertiesOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 111 | `getApparentType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 112 | `getReducedType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 113 | `getPropertyOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 114 | `getTypeOfPropertyOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 115 | `getIndexInfoOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 116 | `getIndexInfosOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 117 | `getConstraintOfTypeParameter` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 118 | `getDefaultFromTypeParameter` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 119 | `getBaseConstraintOfType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 120 | `getTypeArguments` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 121 | `getImportAdderEdits` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 122 | `getTrueTypeOfConditionalType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 123 | `getFalseTypeOfConditionalType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 124 | `getConstantValue` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 125 | `getSignatureFromDeclaration` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 126 | `getExportSpecifierLocalTargetSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 127 | `getAliasedSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 128 | `getImmediateAliasedSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 129 | `getTargetSymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 130 | `getExportSymbolOfSymbolForChecker` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 131 | `getFullyQualifiedName` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 132 | `getExportsOfModule` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 133 | `getMemberInModuleExports` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 134 | `getJsDocTags` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 135 | `getDocumentationComment` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 136 | `isArrayType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 137 | `isReadonlySymbol` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 138 | `getReferencesToSymbolInFile` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 139 | `getReferencedSymbolsForNode` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 140 | `getSignatureUsages` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 141 | `getCompletionsAtPosition` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 142 | `getSyntacticDiagnostics` | core | supported | program_test |
| 143 | `getBindDiagnostics` | core | supported | program_test |
| 144 | `getSemanticDiagnostics` | core | supported | program_test |
| 145 | `getSuggestionDiagnostics` | core | partial | dispatches; no fixture asserting a specific suggestion yet |
| 146 | `getDeclarationDiagnostics` | core | supported | program_test |
| 147 | `getProgramDiagnostics` | core | supported | program_test (empty case) |
| 148 | `getGlobalDiagnostics` | core | supported | program_test (empty case) |
| 149 | `getConfigFileParsingDiagnostics` | core | supported | program_test (client-supplied diagnostics round trip) |
| 150 | `printNode` | core | partial | upstream sync printNode/printFile tests pass; recovered printer panics use Go's `Kind…` names (`printing.rs` unit test) |
| 151 | `formatNodeForInsertion` | core | partial | upstream sync formatNodeForInsertion tests pass |
| 152 | `emit` | core | supported | program_test (write-through, no TSRS_EMIT), requestfs_test (full filesystem returns emittedFilesContents, no disk write) |
| 153 | `emitToString` | core | supported | program_test |
| 154 | `getJavaScriptEmit` | core | supported | program_test |
| 155 | `getDeclarationEmit` | core | supported | program_test |
| 156 | `getAnyType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 157 | `getStringType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 158 | `getNumberType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 159 | `getBooleanType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 160 | `getVoidType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 161 | `getUndefinedType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 162 | `getNullType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 163 | `getNeverType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 164 | `getUnknownType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 165 | `getBigIntType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 166 | `getESSymbolType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 167 | `getNonPrimitiveType` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 168 | `getWellKnownSymbols` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 169 | `getWellKnownSignatures` | checker | partial | checker lane: real-snapshot unit tests + pinned-Go golden subset; dispatched through core Session; not byte-compared end-to-end |
| 170 | `startCPUProfile` | core | not implemented |  |
| 171 | `stopCPUProfile` | core | not implemented |  |
| 172 | `saveHeapProfile` | core | not implemented |  |

## Known gaps (core lane, tracked for follow-up)

- Request filesystems are ported (`crates/tsrs_api/src/requestfs.rs`) but enter the project snapshot as a
  plain host filesystem: LSP overlay rebasing and alias expansion of client `fileNotifications` through
  request symlinks (Go `ExpandFileChanges`) are not applied. Callback filesystems (`--callbacks`, Go
  `callbackfs.go`) are ported in `crates/tsrs_api/src/callbackfs.rs`; like Go, invalid callback responses
  panic and become request errors (a panic on a worker thread can poison shared caches; not yet hardened).
- API builds differ from `tsrs -b` in how they run, not in what they write: each `build` call uses a fresh
  CLI orchestrator (the state Go's `recheckAllProjects` leaves; reuse across builds comes from the
  `.tsbuildinfo` files on disk, as for any rebuild), runs one project at a time on the request thread, and
  builds programs single-threaded, so `--builders` and checker parallelism do not speed up API builds and
  `singleThreaded` is forced on (it does not appear in outputs or build info). The output files, `.tsbuildinfo`
  included, and the exit status are byte-identical to `tsrs -b` on the same graph
  (`api_build_outputs_match_cli_build`), and up-to-date rebuilds build nothing (`api::tests`). This is what
  makes per-build memory freeable; it trades build throughput for bounded memory.
- Build orchestration runs the CLI's `tsc -b` orchestrator in-process through `tsrs_api::build::BuildBackend`
  (installed by `tsrs --api`; library sessions without a backend report the methods as unsupported). Clean uses
  the last build's graph like Go; clean existence checks use the uncached filesystem.
- Profiling (`startCPUProfile`, `stopCPUProfile`, `saveHeapProfile`) is not implemented (no pprof
  equivalent). `getCurrentLanguageServerSnapshot` returns Go's standalone-session client error; LSP-attached
  API sessions are not ported.
- Node index tables are cached per live source file keyed by the file's address (lookups never assign the
  file's lazy node id) and evicted when the file's region is freed; Go stores them on the file itself.
- Re-entrancy attribution: snapshot builds read files through a `ScopedFs` that enters the creating
  request's transport `RequestScope`, so client callbacks from compiler worker threads are attributed to
  that request; API builds run their tasks on the request thread. Remaining divergence: a request whose own
  slow client callback holds a contended resource cannot be told apart from callback re-entry by attribution
  alone, so a waiter can still get the re-entrancy error after the transport's configurable grace period
  where Go would wait (or hang). Reads made later by a retained snapshot count against its finished request.
- Re-entrancy: nested requests from client callbacks are served; the build orchestrator lock uses the
  transport's `lock_for_request` (bounded error instead of deadlock); the checker lane gates its API checker
  lease the same way. Request params are validated with Go's strict JSON (unpaired surrogates, duplicate
  members) before decoding; `null` params decode to the zero-value struct. Field-type mismatches have Go's
  invalid-request class and `failed to unmarshal *api.<Type>` prefix, but not Go's exact jsontext wording.
  Every method's top-level params fields (generated from pinned `proto.go` into `paramfields.rs`) are
  checked in Go's field order before any lookup, for core and checker methods alike (`predecode.rs`);
  unknown keys are ignored. Integer fields are checked against the raw number lexeme and the Go type's range
  (`1e3`/`1.0` are invalid syntax; int32/uint32/int/uint64 bounds), array element kinds are checked, and
  null elements of `[]string`-like arrays are zero values, as in Go. Remaining wording differences (runtime
  matrix on f70371e): Go includes the number literal and `: invalid syntax`, reports the first invalid field
  in document order (this reports struct order), qualifies element types (`[]api.BatchRequest`) and prints
  alias names (`core.ModuleKind`). Nested api-package structs (program options, symbol references, import
  adder actions, module-resolution specs, diagnostics, file notifications) are checked the same way before
  any lookup, and `project.SyntheticProjectID` values go through Go's decoder rules (null and non-synthetic
  text are decode errors). Integer IDs are read from their own literal (by position in the parsed params
  tree), so values above 2^53 are exact at any depth; batch items keep their raw params bytes (Go
  `json.Value`). Structs from other Go packages are decoded from the parsed value with Go's rules, checked
  against runtime's c246 session review: integer fields of `core.CompilerOptions` (int32 enums, `*int`) and
  `core.BuildOptions.builders` need plain integer syntax (`1e1` is invalid). `target`, `module` and `jsx`
  are int32 newtypes (as in Go), so any number Go accepts is kept, echoed and compiled with Go's semantics:
  switch defaults and comparisons see the raw value (e.g. `target:12345` transforms maximally, `module:-1`
  emits CommonJS, `jsx:12345` keeps JSX without the "--jsx not set" error); checked against
  runtime-f2-review's cases. `moduleResolution` is an int32 newtype too: a number with no named kind is kept
  (createPrograms, createBuildOrchestrator, builds of import-free projects and their outputs/cleans work as
  in Go), and resolving a module with it reaches the ported resolver's `Unexpected moduleResolution` panic at
  the same point as pinned Go (resolver.go `ResolveModuleName`); Go's server crashes there, tsrs converts that
  panic to the stable client error `unsupported moduleResolution value N (not a ModuleResolutionKind)` and
  stays usable (orchestrator map lock poisoning is tolerated). Unknown `moduleDetection`/`newLine` numbers are echoed and
  compile like Go's fallback (runtime f552 review). Go `*int` options (`maxNodeModuleJsDepth`, `builders`,
  `checkers`) take any int64, exact from the request literal and echoed exactly (`json::Value::Integer`);
  above int64 is out of range. `checkers` is clamped like Go (`max(min(n, files, 256), 1)`; tsrs used to
  turn a negative count into the file count). Prior limitation, not API-specific: a tsconfig float far
  outside int64 (`9.3e18`, `1e19`) for these options saturates in Rust; Go's float-to-int conversion is
  architecture dependent there (amd64 gives MinInt64, arm64 saturates). Malformed
  tristates and unknown options are ignored, as in Go. Request filesystems are type-checked before any
  lookup (`kind`, `files`, `directories`, `symlinks.target/host`, `removedPaths`), with `null` values and
  elements as zero values. Pinned Go crashes on a relative, empty or null project reference path and on a
  missing referenced project; tsrs instead returns a client error for non-absolute paths and reports a
  missing project as a `File '…' not found` program diagnostic. Error wording is encoding/json v1 style.
- Response shapes from paired runs (parity f703): `updateSnapshot` omits an empty `changes` (json/v2
  `omitempty`); `cleanBuild` keeps Go's per-orchestrator existence answers until the next build, so a clean
  after a clean lists the project's outputs again.
- Memory: snapshots free their programs, checkers, emit allocations and a full build's shared data
  (processed files, project-reference mapper, loader and dts-faking resolution hosts) with the build's base
  region (createSnapshot+release cycles stay flat; `tests/memory_test.rs`). The first attempt at freeing
  the mapper exposed a pre-existing use-after-free in `tsrs_project` (`update_program` read the replaced
  program after dropping its owner); fixed, with a perturbation/ASAN-validated regression test
  (`inferred_project_rebuild_frees_safely`). Transpile frees its one-file program including the compiler
  checker pool's checkers (previously leaked; ~5 KiB/call of allocator retention remains), and config
  requests run in scratch regions. API builds allocate in per-task regions (one builder, tasks
  run on the request thread, programs single-threaded and freed after each project) and each build handle
  keeps only its last build (for cleans); 120 rebuilds retain no more than one (`memory_tests` in
  `crates/tsrs_cli/src/api.rs`; ~19 MiB per rebuild before). File texts parsed inside a freeable region are
  copied into it and unregistered with it instead of leaked. Still retained: the request thread's arena use
  outside these paths. Unregistered
  source texts free their memory but not their registry slot; slots are never reused, so a very long-lived
  session that parses more than 2^20 files in freeable regions falls back to the slower text lookup path.
