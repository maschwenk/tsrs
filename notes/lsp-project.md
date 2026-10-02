# lsp-project: `tsrs_project` (Go `internal/project` + `dirty`, `logging`, `background`)

## Ported (Go file -> Rust file, `crates/tsrs_project/src/`)

All of: dirty/{box,cloneablemap,entry,interfaces,map,mapbuilder,syncmap,util}, logging/{logger,logcollector,logtree},
background/queue (+ background/timer.rs: Go `time.AfterFunc`), refcountcache, parsecache, ownercache,
extendedconfigcache, programcounter, filechange, client, watch, overlayfs, snapshotfs, compilerhost,
configfileregistry, configfileregistrybuilder, checkerpool, project, projectcollection, projectcollectionbuilder
(incl. HandleAPIRequest), snapshothost, snapshot, session, autoimport (calls the tsrs_ls placeholder registry),
api.go's `TryAdoptSnapshotInBackground` (in session.rs). `ata.rs`: only `TypingsInfo` + `NpmExecutor` trait.
Not ported (brief): ata, api.go `APIUpdate`, content mappers, telemetry bodies (hooks kept, nothing sent).

Tests (`cargo test -p tsrs_project`): 105 pass, 2 ignored. session_test.go: all subtests (54 cases incl.
watch/config/locale/closed-file); overlayfs_test.go: all 13; checkerpool_test.go: 24 of 43 (synctest ones run
with real short timeouts; cancellation/contention-with-fake-time ones skipped); projectcollectionbuilder_test.go: 13
(2 ignored: need compiler project references); dirty/syncmap_test.go 4; background/queue_test.go 4; logtree 2.
Test utilities: `projecttestutil.rs` (#[cfg(test)]: Setup, SetupWithOptions, ClientMock call recording, WatchesFile).
Not yet ported: refcountcache_test, extendedconfigcache_test, configfilechanges_test, projectlifetime_test,
untitled_test, snapshotfs_test, snapshot_test, project_test, watch(timeout)_test, bulkcache_test,
customconfigfilename_test, projectcollectiondefaultproject_test, projectreferencesprogram_test.

## API notes (for lsp server)

- `new_session(SessionInit) -> Arc<Session>`, methods `&self` (background work via an internal `Weak`).
- Snapshots: `Arc<Snapshot>` + Go's explicit `ref_`/`deref` counts. Projects: `dirty::Shared<Project>` (Arc,
  ptr-equality); as `Arc<dyn tsrs_ls::Project>` via `shared.arc().clone()`.
- `Client` trait: errors `lsproto::Error`, message args `&[&dyn Display]`.
- `ErrNoProjectForUnknownScriptKind` -> `is_err_no_project_for_unknown_script_kind(&err)` (message prefix).

## Shared-file edits

- `crates/tsrs_project/Cargo.toml`: `xxhash-rust` (xxh3), same version as tsrs_checker. `Cargo.lock` accordingly.

## Deviations

- dirty values are `Shared<T>` (Arc). Mutation is copy-on-write (`Arc::make_mut`): a caller still holding an old
  handle keeps the old value where Go would see the in-place mutation (a race in Go). `dirty.Map`/`Box` never hold
  their locks during callbacks (Go has none); `SyncMapEntry` locks exactly like Go. Proxy entries do not keep Go's
  copy of the target's value (all access goes through `proxyFor`).
- `SyncMap.Range` snapshots the dirty map before iterating (Go sync.Map semantics allow it); Go's quirk of
  continuing into the base map after the callback stops the dirty iteration is kept.
- Goroutines: background queue and timer callbacks run on 6 shared 512 MB-stack worker threads; `time.AfterFunc`
  -> one timer thread (`background/timer.rs`). Go `go func` parallel loops in the builder (HandleAPIRequest,
  DidRequestProjectTrees work group) run sequentially (tsrs WorkGroup order: last queued first).
- compilerHost keeps the project's ID instead of `*Project`, and the builder's parse cache / config registry
  builder / ctx instead of `*ProjectCollectionBuilder`; `freeze` clears them. Content-mapper host methods omitted.
- `CreateProgram` returns the concrete checker pool (factory stashes it) since `Program.GetCheckerPool()` is
  `dyn CheckerPool` (Go type-asserts `*checkerPool`).
- Disposed checkers are leaked (`mem::forget`), not dropped (phase 4: checker region). Parse cache final Deref
  only drops the entry (phase 4: file region) — both hooks marked in comments.
- tsoptions takes `&'static dyn ParseConfigHost`/`ExtendedConfigCache`; the registry builder is passed with an
  assumed 'static lifetime (`assume_static`, unsafe, used only for the duration of the parse call).
- `ParsedCommandLine.Errors` assigned after creation in Go -> passed into `new_inferred_project_command_line`.
- `reflect.DeepEqual(CompilerOptions)` -> Debug-rendering comparison; diagnostic slices compared by identity.
- `logging.formatTime` prints UTC (no time zone database in std).
- Snapshot converters' line-map closure holds the SnapshotFS (not the snapshot: would be a cycle).
- `Session.logCacheStats` skips the auto-import section (placeholder registry has no stats).
- overlayFS `processChanges` processes files in first-seen order (Go: random map order).

## Needs from others

- tsrs_compiler: project references (programs never resolve references, so referenced-config acquisition/release
  via the compiler host and 2 builder tests do not apply).
- tsrs_ls: `ls::Project` has no downcast; server.go's `p.(*project.Project)` needs a lookup by ID.
- Checker cancellation (`was_canceled`) is phase 4; checker-pool cancel-disposal tests not ported.

## Doubts

- Memory: every program pins its compiler host -> SnapshotFS (file contents) forever (programs are leaked).
- 6 background workers: more than ~5 concurrently sleeping debounce tasks would delay other background work.
