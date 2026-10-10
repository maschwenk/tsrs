# lsp-project: `tsrs_project` (Go `internal/project` + `dirty`, `logging`, `background`)

Wave `project` of the language-server port: the session, snapshots, overlays, project collection builder, config
file registry, parse cache, checker pool and background queue (docs/LSP.md, crate map; `dirty` and `logging` now live
in `tsrs_projectutil`). Not ported: ATA, api.go `APIUpdate`, content mappers, telemetry bodies (hooks kept, nothing
sent). This note keeps the deviations from Go that still hold.

## Deviations

- dirty values are `Shared<T>` (Arc). Mutation is copy-on-write (`make_mut`): a caller still holding an old handle
  keeps the old value where Go would see the in-place mutation (a race in Go). `dirty.Map`/`Box` never hold their
  locks during callbacks (Go has none); `SyncMapEntry` locks exactly like Go. Proxy entries do not keep Go's copy of
  the target's value (all access goes through `proxyFor`).
- `SyncMap.Range` snapshots the dirty map before iterating (Go sync.Map semantics allow it); Go's quirk of continuing
  into the base map after the callback stops the dirty iteration is kept.
- Goroutines: background queue and timer callbacks run on 6 shared 512 MB-stack worker threads (`background/queue.rs`);
  `time.AfterFunc` -> one timer thread (`background/timer.rs`). More than ~5 concurrently sleeping debounce tasks
  would delay other background work. Go `go func` parallel loops in the builder (HandleAPIRequest,
  DidRequestProjectTrees work group) run sequentially (tsrs WorkGroup order: last queued first).
- compilerHost keeps the project's ID instead of `*Project`, and the builder's parse cache / config registry builder
  / ctx instead of `*ProjectCollectionBuilder`; `freeze` clears them. Content-mapper host methods omitted.
- `CreateProgram` returns the concrete checker pool (the factory stashes it) since `Program.GetCheckerPool()` is
  `dyn CheckerPool` (Go type-asserts `*checkerPool`).
- tsoptions takes `&'static dyn ParseConfigHost`/`ExtendedConfigCache`; the registry builder is passed with an
  assumed 'static lifetime (`assume_static` in extendedconfigcache.rs, unsafe, used only for the duration of the parse
  call).
- `ParsedCommandLine.Errors` assigned after creation in Go -> passed into `new_inferred_project_command_line`.
- `reflect.DeepEqual(CompilerOptions)` -> Debug-rendering comparison; diagnostic slices compared by identity
  (projectcollectionbuilder.rs).
- `logging.formatTime` prints UTC (no time zone database in std).
- Snapshot converters' line-map closure holds the SnapshotFS (not the snapshot: would be a cycle).
- overlayFS `processChanges` processes files in first-seen order (Go: random map order).

Status (2026-10-10): the wave-era memory deviations are gone: disposed checkers are parked with their checker region
and programs are freed by program owners (docs/LSP.md, memory plan); compiler project references are ported.
