# Node programmatic API (`tsrs --api`)

Port of the TypeScript 7 native API at microsoft/TypeScript `b85298b6a81f772d080b0455de0ca9d744cd6fd6`
(the commit in the workspace `Cargo.toml`): server side `tsc/internal/api` + `tsc/internal/ipc`, client
side `packages/typescript/src/{api,ast}` exported as `unstable/sync` and `unstable/async`. The wire
format, method names, params and response schemas are the pinned Go ones; tsrs does not define its own
protocol or a one-shot CLI wrapper, and it is not compatible with TS5 `createProgram` objects.

Status: **in progress**. The matrix at the end is the source of truth; a method is only `supported`
when a real compiler-backed test exercises it. Everything else returns an explicit
`api: unsupported: method "..." is not implemented by tsrs` error, never a fake success.

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
- Positions: the encoder writes the pinned wire positions; UTF-16 conversion happens in the JS client.

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
| 2 | `releaseSourceFile` | core | not implemented |  |
| 3 | `retainSourceFile` | core | not implemented |  |
| 4 | `getCachedSourceFile` | core | not implemented |  |
| 5 | `batchRequests` | core | supported | batch_test (nesting error, pagination with continuation tokens); binary-in-batch base64 path untested until source-file methods land |
| 6 | `initialize` | core | supported | config_test |
| 7 | `createSnapshot` | core | supported | program_test, module_resolution_test, requestfs_test (full/layer request filesystems, removedPaths); fileNotifications alias expansion through request symlinks not applied |
| 8 | `updateSnapshot` | core | supported | program_test, requestfs_test (layers compacted over full, retained base, release of base) |
| 9 | `getCurrentLanguageServerSnapshot` | core | not implemented |  |
| 10 | `createBuildOrchestrator` | core | supported | tsrs_cli api::tests (in-process CLI build backend; fresh orchestrator per call instead of Go's recheckAllProjects reuse) |
| 11 | `disposeBuildOrchestrator` | core | supported | tsrs_cli api::tests |
| 12 | `build` | core | supported | tsrs_cli api::tests (references, up-to-date rebuild) |
| 13 | `buildReferences` | core | supported | tsrs_cli api::tests |
| 14 | `cleanBuild` | core | supported | tsrs_cli api::tests |
| 15 | `cleanReferences` | core | partial | dispatches to the ported Go clean(onlyReferences); no dedicated test yet |
| 16 | `createModuleResolver` | core | supported | module_resolution_test (default, static entries, callback) |
| 17 | `releaseModuleResolver` | core | supported | module_resolution_test |
| 18 | `resolveModuleName` | core | supported | module_resolution_test (standalone, snapshot-scoped callback); inProgressSnapshot path exercised only via callbacks during program build |
| 19 | `parseCommandLine` | core | supported | config_test |
| 20 | `readConfigFile` | core | supported | config_test |
| 21 | `parseJsonConfigFileContent` | core | supported | config_test |
| 22 | `parseConfigFile` | core | supported | config_test |
| 23 | `createSourceFile` | core | not implemented |  |
| 24 | `createSourceFileFromFile` | core | not implemented |  |
| 25 | `transpileModule` | core | supported | transpile_test |
| 26 | `transpileModuleFromFile` | core | supported | transpile_test |
| 27 | `transpileDeclaration` | core | supported | transpile_test (upstream api.test.ts expectations); observed: no TS9007 for `export function g() { return 1; }` under isolatedDeclarations, unverified against tsgo |
| 28 | `transpileDeclarationFromFile` | core | supported | transpile_test |
| 29 | `getDefaultProjectForFile` | core | supported | program_test |
| 30 | `getSymbolAtPosition` | checker | not implemented |  |
| 31 | `getSymbolsAtPositions` | checker | not implemented |  |
| 32 | `getSymbolAtLocation` | checker | not implemented |  |
| 33 | `getSymbolsAtLocations` | checker | not implemented |  |
| 34 | `getSymbolOfSourceFile` | checker | not implemented |  |
| 35 | `getSymbolsOfSourceFiles` | checker | not implemented |  |
| 36 | `getTypeOfSymbol` | checker | not implemented |  |
| 37 | `getTypesOfSymbols` | checker | not implemented |  |
| 38 | `getDeclaredTypeOfSymbol` | checker | not implemented |  |
| 39 | `getNonMissingTypeOfSymbol` | checker | not implemented |  |
| 40 | `getSourceFile` | core | not implemented |  |
| 41 | `getSourceFileNames` | core | supported | program_test |
| 42 | `getSourceFileMetadata` | core | supported | program_test |
| 43 | `getModeForUsageLocation` | core | not implemented |  |
| 44 | `getModeForResolutionAtIndex` | core | not implemented |  |
| 45 | `getResolvedModule` | core | not implemented |  |
| 46 | `getResolvedModuleFromModuleSpecifier` | core | not implemented |  |
| 47 | `getResolvedTypeReferenceDirective` | core | not implemented |  |
| 48 | `getResolvedTypeReferenceDirectiveFromTypeReferenceDirective` | core | not implemented |  |
| 49 | `getConfigFileNames` | core | supported | program_test (extends chain, synthetic null) |
| 50 | `getConfigSourceFile` | core | not implemented |  |
| 51 | `resolveName` | checker | not implemented |  |
| 52 | `getSymbolsInScope` | checker | not implemented |  |
| 53 | `getSignaturesOfType` | checker | not implemented |  |
| 54 | `getResolvedSignature` | checker | not implemented |  |
| 55 | `getTypeAtLocation` | checker | not implemented |  |
| 56 | `getTypeAtLocations` | checker | not implemented |  |
| 57 | `getTypeAtPosition` | checker | not implemented |  |
| 58 | `getTypesAtPositions` | checker | not implemented |  |
| 59 | `getParentOfSymbol` | checker | not implemented |  |
| 60 | `getMembersOfSymbol` | checker | not implemented |  |
| 61 | `getExportsOfSymbol` | checker | not implemented |  |
| 62 | `getExportSymbolOfSymbol` | checker | not implemented |  |
| 63 | `getSymbolOfType` | checker | not implemented |  |
| 64 | `getTargetOfType` | checker | not implemented |  |
| 65 | `getFreshTypeOfType` | checker | not implemented |  |
| 66 | `getRegularTypeOfType` | checker | not implemented |  |
| 67 | `getTypesOfType` | checker | not implemented |  |
| 68 | `getTypeParametersOfType` | checker | not implemented |  |
| 69 | `getOuterTypeParametersOfType` | checker | not implemented |  |
| 70 | `getLocalTypeParametersOfType` | checker | not implemented |  |
| 71 | `getThisTypeOfType` | checker | not implemented |  |
| 72 | `getAliasTypeArgumentsOfType` | checker | not implemented |  |
| 73 | `getAliasSymbolOfType` | checker | not implemented |  |
| 74 | `getObjectTypeOfType` | checker | not implemented |  |
| 75 | `getIndexTypeOfType` | checker | not implemented |  |
| 76 | `getCheckTypeOfType` | checker | not implemented |  |
| 77 | `getExtendsTypeOfType` | checker | not implemented |  |
| 78 | `getBaseTypeOfType` | checker | not implemented |  |
| 79 | `getConstraintOfType` | checker | not implemented |  |
| 80 | `getTypeParameterOfMappedType` | checker | not implemented |  |
| 81 | `getConstraintTypeOfMappedType` | checker | not implemented |  |
| 82 | `getNameTypeOfMappedType` | checker | not implemented |  |
| 83 | `getTemplateTypeOfMappedType` | checker | not implemented |  |
| 84 | `getTypeParametersOfSignature` | checker | not implemented |  |
| 85 | `getParametersOfSignature` | checker | not implemented |  |
| 86 | `getThisParameterOfSignature` | checker | not implemented |  |
| 87 | `getTargetOfSignature` | checker | not implemented |  |
| 88 | `getContextualType` | checker | not implemented |  |
| 89 | `getContextualTypeForArgument` | checker | not implemented |  |
| 90 | `getAwaitedType` | checker | not implemented |  |
| 91 | `getBaseTypeOfLiteralType` | checker | not implemented |  |
| 92 | `getNonNullableType` | checker | not implemented |  |
| 93 | `getTypeFromTypeNode` | checker | not implemented |  |
| 94 | `getWidenedType` | checker | not implemented |  |
| 95 | `getParameterType` | checker | not implemented |  |
| 96 | `getTypeParameterAtPosition` | checker | not implemented |  |
| 97 | `isArrayLikeType` | checker | not implemented |  |
| 98 | `isTypeAssignableTo` | checker | not implemented |  |
| 99 | `getShorthandAssignmentValueSymbol` | checker | not implemented |  |
| 100 | `getTypeOfSymbolAtLocation` | checker | not implemented |  |
| 101 | `typeToTypeNode` | checker | not implemented |  |
| 102 | `signatureToSignatureDeclaration` | checker | not implemented |  |
| 103 | `typeToString` | checker | not implemented |  |
| 104 | `isContextSensitive` | checker | not implemented |  |
| 105 | `getReturnTypeOfSignature` | checker | not implemented |  |
| 106 | `getRestTypeOfSignature` | checker | not implemented |  |
| 107 | `getTypePredicateOfSignature` | checker | not implemented |  |
| 108 | `getBaseTypes` | checker | not implemented |  |
| 109 | `getPropertiesOfType` | checker | not implemented |  |
| 110 | `getApparentPropertiesOfType` | checker | not implemented |  |
| 111 | `getApparentType` | checker | not implemented |  |
| 112 | `getReducedType` | checker | not implemented |  |
| 113 | `getPropertyOfType` | checker | not implemented |  |
| 114 | `getTypeOfPropertyOfType` | checker | not implemented |  |
| 115 | `getIndexInfoOfType` | checker | not implemented |  |
| 116 | `getIndexInfosOfType` | checker | not implemented |  |
| 117 | `getConstraintOfTypeParameter` | checker | not implemented |  |
| 118 | `getDefaultFromTypeParameter` | checker | not implemented |  |
| 119 | `getBaseConstraintOfType` | checker | not implemented |  |
| 120 | `getTypeArguments` | checker | not implemented |  |
| 121 | `getImportAdderEdits` | checker | not implemented |  |
| 122 | `getTrueTypeOfConditionalType` | checker | not implemented |  |
| 123 | `getFalseTypeOfConditionalType` | checker | not implemented |  |
| 124 | `getConstantValue` | checker | not implemented |  |
| 125 | `getSignatureFromDeclaration` | checker | not implemented |  |
| 126 | `getExportSpecifierLocalTargetSymbol` | checker | not implemented |  |
| 127 | `getAliasedSymbol` | checker | not implemented |  |
| 128 | `getImmediateAliasedSymbol` | checker | not implemented |  |
| 129 | `getTargetSymbol` | checker | not implemented |  |
| 130 | `getExportSymbolOfSymbolForChecker` | checker | not implemented |  |
| 131 | `getFullyQualifiedName` | checker | not implemented |  |
| 132 | `getExportsOfModule` | checker | not implemented |  |
| 133 | `getMemberInModuleExports` | checker | not implemented |  |
| 134 | `getJsDocTags` | checker | not implemented |  |
| 135 | `getDocumentationComment` | checker | not implemented |  |
| 136 | `isArrayType` | checker | not implemented |  |
| 137 | `isReadonlySymbol` | checker | not implemented |  |
| 138 | `getReferencesToSymbolInFile` | checker | not implemented |  |
| 139 | `getReferencedSymbolsForNode` | checker | not implemented |  |
| 140 | `getSignatureUsages` | checker | not implemented |  |
| 141 | `getCompletionsAtPosition` | checker | not implemented |  |
| 142 | `getSyntacticDiagnostics` | core | supported | program_test |
| 143 | `getBindDiagnostics` | core | supported | program_test |
| 144 | `getSemanticDiagnostics` | core | supported | program_test |
| 145 | `getSuggestionDiagnostics` | core | partial | dispatches; no fixture asserting a specific suggestion yet |
| 146 | `getDeclarationDiagnostics` | core | supported | program_test |
| 147 | `getProgramDiagnostics` | core | supported | program_test (empty case) |
| 148 | `getGlobalDiagnostics` | core | supported | program_test (empty case) |
| 149 | `getConfigFileParsingDiagnostics` | core | supported | program_test (client-supplied diagnostics round trip) |
| 150 | `printNode` | core | not implemented |  |
| 151 | `formatNodeForInsertion` | core | not implemented |  |
| 152 | `emit` | core | supported | program_test (write-through, no TSRS_EMIT), requestfs_test (full filesystem returns emittedFilesContents, no disk write) |
| 153 | `emitToString` | core | supported | program_test |
| 154 | `getJavaScriptEmit` | core | supported | program_test |
| 155 | `getDeclarationEmit` | core | supported | program_test |
| 156 | `getAnyType` | checker | not implemented |  |
| 157 | `getStringType` | checker | not implemented |  |
| 158 | `getNumberType` | checker | not implemented |  |
| 159 | `getBooleanType` | checker | not implemented |  |
| 160 | `getVoidType` | checker | not implemented |  |
| 161 | `getUndefinedType` | checker | not implemented |  |
| 162 | `getNullType` | checker | not implemented |  |
| 163 | `getNeverType` | checker | not implemented |  |
| 164 | `getUnknownType` | checker | not implemented |  |
| 165 | `getBigIntType` | checker | not implemented |  |
| 166 | `getESSymbolType` | checker | not implemented |  |
| 167 | `getNonPrimitiveType` | checker | not implemented |  |
| 168 | `getWellKnownSymbols` | checker | not implemented |  |
| 169 | `getWellKnownSignatures` | checker | not implemented |  |
| 170 | `startCPUProfile` | core | not implemented |  |
| 171 | `stopCPUProfile` | core | not implemented |  |
| 172 | `saveHeapProfile` | core | not implemented |  |

## Known gaps (core lane, tracked for follow-up)

- Request filesystems are ported (`crates/tsrs_api/src/requestfs.rs`) but enter the project snapshot as a
  plain host filesystem: LSP overlay rebasing and alias expansion of client `fileNotifications` through
  request symlinks (Go `ExpandFileChanges`) are not applied. Callback filesystems (`--callbacks`, Go
  `callbackfs.go`) are ported in `crates/tsrs_api/src/callbackfs.rs`; like Go, invalid callback responses
  panic and become request errors (a panic on a worker thread can poison shared caches; not yet hardened).
- Build orchestration runs the CLI's `tsc -b` orchestrator in-process through `tsrs_api::build::BuildBackend`
  (installed by `tsrs --api`; library sessions without a backend report the methods as unsupported). Each
  call builds a fresh orchestrator rather than reusing tasks like Go's `recheckAllProjects`; each leaks a
  small system/orchestrator allocation.
- `tsrs --api` currently exits 1 with an explicit message: the wire runtime (runtime lane) is not merged
  into this branch yet.
- Source files / AST (`getSourceFile`, `createSourceFile*`, leases, `printNode`, ...) wait on the codec
  lane's encoder and node index tables.
- `transpile*`, `batchRequests`, `getSourceFileMetadata`, resolution queries, profiling and
  `getCurrentLanguageServerSnapshot` (needs an LSP-attached session; standalone sessions return a
  client error in Go too) are not implemented yet.
- Memory: request-time allocations outside project/checker regions go to the request thread's arena
  (same as the language server today) and are not reclaimed until the thread exits.
