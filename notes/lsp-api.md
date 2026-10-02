# lsp-api: compiler- and checker-side API for the language service

Wave agent `api` (branch `lsp-api`). Scope: `tsrs_core::context::Context`, the pluggable checker pool and program
reuse in `tsrs_compiler`, and the checker's language-service API (`services.go`, `exports.go` and every checker
function `ls` / `lsp` / `project` / `fourslash` call).

## 1. `tsrs_core::context::Context` (Go `context.Context`)

`crates/tsrs_core/src/context.rs` (module now `pub mod context`, still glob re-exported at the crate root).

| Go | Rust |
| --- | --- |
| `context.Background()` | `Context::background()` / `Context::default()` |
| `context.WithValue(ctx, k, v)` / `ctx.Value(k).(T)` | `ctx.with_value(v)` / `ctx.value::<T>()` (the key is the value's type: use a private newtype per key, as Go uses an unexported key type) |
| `core.WithRequestID` / `core.GetRequestID` | `with_request_id(&ctx, id)` / `get_request_id(&ctx) -> &str` |
| `core.WithCheckerLifetime` / `core.GetCheckerLifetime` | `with_checker_lifetime` / `get_checker_lifetime` (`CheckerLifetime` moved here unchanged) |
| `locale.WithLocale` / `locale.FromContext` / `locale.HasLocale` | `with_locale` / `locale_from_context` / `has_locale` (`Locale(String)`, English only) |
| `context.WithCancel` / `CancelFunc` | `ctx.with_cancel() -> (Context, CancelFunc)`; `cancel.call()`; parent cancellation propagates to children (weak child list), a child of a canceled parent starts canceled |
| `context.WithCancelCause` / `context.Cause` | `with_cancel_cause()` / `CancelCauseFunc::call(Some(cause))` / `ctx.cause()` |
| `context.WithTimeout` | `with_timeout(d)` (see Deviations) |
| `ctx.Err()`, `context.Canceled`, `context.DeadlineExceeded` | `ctx.err() -> Option<ContextError>`, `ContextError::{Canceled, DeadlineExceeded}`; `ctx.is_canceled()` |
| `ctx.Done() == nil` | `ctx.done_is_nil()` (no cancel node in the chain) |
| `<-ctx.Done()` / `select` with timeout | `ctx.wait()` / `ctx.wait_timeout(d) -> bool` |
| `context.AfterFunc(ctx, f)` / `stop()` | `ctx.after_func(f) -> AfterFuncStop` / `stop.stop()`; `f` runs on its own thread (Go: own goroutine) |

Signatures take `ctx: &Context` (Go passes the interface by value; `&Context` avoids a refcount per call). The
checker's old unit `pub struct Context` (printer_types.rs) is now `pub use tsrs_core::context::Context`;
`get_diagnostics_exported`, `get_suggestion_diagnostics`, `get_diagnostics`, `check_source_file` take `&Context`.
`Checker::was_canceled()` already existed (exports.go:171, returns the `was_canceled` field, always false since
`is_canceled()` does not poll yet: phase 4).

## 2. `tsrs_compiler`: pluggable checker pool, program reuse

- `CheckerPool` trait (checkerpool.go:24): `fn get_checker(&self, ctx: &Context, file: Option<P<SourceFile>>) ->
  CheckerHandle`. `CheckerHandle` derefs (mut) to `Checker`; dropping it (or `release()`) is Go's `done()`, run
  exactly once. Two kinds: the built-in pool's `MutexGuard<'static, Box<Checker>>` (its checker slots are leaked
  with the program, so no borrow of the pool), and `unsafe CheckerHandle::from_raw(NonNull<Checker>, release)` for
  external pools that track exclusivity themselves (Go project pool's `heldBy`). `PooledChecker` is a `Send + Sync`
  owner wrapper (`Checker` is not `Send`) for such pools; `as_non_null()` gives the address for `from_raw`. Usage is
  shown in `program_test.rs` (`testPool`).
- `ProgramOptions.create_checker_pool: Option<CreateCheckerPool>` (`Arc<dyn Fn(&'static Program) -> Box<dyn
  CheckerPool>>`); `CreateModuleResolver` is now an `Arc` too (Go func values are copyable; `UpdateProgram` forwards
  both). `ProgramOptions::from_config(ProgramConfig, host, pools, resolver)`.
- `Program.checker_pool: OnceLock<programCheckerPool>` = `Compiler(checkerPool)` (Go `compilerCheckerPool != nil`) or
  `External(Box<dyn CheckerPool>)`. The built-in pool stores `program: &'static Program` like Go; its batch paths
  (`for_each_checker_group_do`, `for_each_checker_parallel`, global diagnostics) are called on the concrete type, no
  dynamic dispatch per file.
- Program reuse: `files` / `files_by_path` moved from `processedFiles` into `Program` (per program); the rest of
  `processedFiles` and the project-reference mapper are `&'static` and shared between a program and the programs
  `ReuseProgram` derives from it, exactly what Go's shallow `processedFiles` copy shares. `ReuseProgram` clones the
  files slice and `filesByPath` (as Go), clones the processing-diagnostics list, program diagnostics and the emit
  blocking set (Go shares those slices/maps), `resolutionData.Clone()` (`clone_data`), and re-creates the lazily
  computed state Go re-creates (include processor, common source directory, declaration diagnostic cache,
  packages map, hasTSFile); `unresolvedImports` / `knownSymlinks` / `packageNames` are `tryReuse`d.
- `SourceFile.hash: OwnedCell<u128>` added (Go `SourceFile.Hash`, set by the project parse cache), and the file loader
  records `DuplicateSourceFiles` like Go's `getProcessedFiles` (casing variants, package-deduplicated files).
- `CompilerHost` trait: unchanged. It already matches Go's interface minus the content-mapper methods (out of scope);
  the loader already obtains every file through `host.get_source_file` (sequential and prefetch paths) and binding
  skips already-bound files (`!f.is_bound()` / `bind_once`), so `project/compilerhost.go`'s parse-cache host
  (returning a parsed + bound file it owns) can be implemented outside the crate.

### Go `Program` methods called from `internal/{project,ls,lsp,fourslash}`

All exist with Go-equivalent signatures (`ast.HasFileName` params are `P<SourceFile>`, `context.Context` is
`&Context`; checker-returning methods return `CheckerHandle`):

BindSourceFiles, CommandLine, ContentMapperExtensions (always empty: content mappers out of scope),
DeepImportPackageNames (new), DuplicateSourceFiles (new), FileExists, GetCheckerPool (new), GetConfigFileParsingDiagnostics,
GetCurrentDirectory, GetDeclarationDiagnostics(ctx), GetDefaultResolutionModeForFile, GetEmitModuleFormatOfFile,
GetGlobalDiagnostics(ctx), GetGlobalTypingsCacheLocation, GetImpliedNodeFormatForEmit, GetImportHelpersImportSpecifier,
GetJSXRuntimeImportSpecifier, GetLibFileFromReference, GetModeForUsageLocation, GetNearestAncestorDirectoryWithPackageJson,
GetPackageJsonInfo, GetProgramDiagnostics, GetResolvedModuleFromModuleSpecifier, GetResolvedProjectReferences,
GetResolvedTypeReferenceDirectiveFromTypeReferenceDirective, GetSemanticDiagnostics(ctx), GetSourceFile,
GetSourceFileByPath, GetSourceFileFromReference, GetSourceFiles, GetSuggestionDiagnostics(ctx), GetSymlinkCache,
GetSyntacticDiagnostics(ctx), GetTypeChecker(ctx), GetTypeCheckerForFile(ctx, file),
GetTypeCheckerForFileExclusive(ctx, file) (new), GetUnresolvedImports (new), HasSameFileNames (new), HasTSFile (now cached
once like Go), Host, IsGlobalTypingsFile, IsLibFile, IsSourceFileDefaultLibrary, IsSourceFileFromExternalLibrary,
IsSourceFromProjectReference, ModuleResolutionError, Options, RangeResolvedProjectReference (new),
RangeResolvedProjectReferenceInChildConfig (new, project references not ported: never visits), ResolvedPackageNames (new),
SourceFiles, UnresolvedPackageNames (new), UpdateProgram (new), ReuseProgram (new), UseCaseSensitiveFileNames,
UsesUriStyleNodeCoreModules. Also added: ForEachCheckerParallel, GetSemanticDiagnosticsForIncremental,
GetResolvedProjectReferenceFor, PackageJsonCacheEntries, Program().

Not ported: `Emit` (lsp/server.go:1858; emit is not ported in tsrs), `ContentMapperProject` (out of scope), `Tracing`.
Package-level: `SortAndDeduplicateDiagnostics`, `NewProgram`, `ProgramOptions`, `CompilerHost`, `CheckerPool`,
`DuplicateSourceFile` exist; `WriteFileData`, `EmitOptions` (emit) do not.

## 3. `tsrs_checker`: language-service API

(see the table below; ported by the helper on branch `lsp-api-checker`, cherry-picked here)

## Shared-file edits

- `tsrs_core/src/lib.rs`: `mod context` -> `pub mod context`.
- `tsrs_ast/src/ast.rs`: `SourceFile.hash: OwnedCell<u128>` (Go `SourceFile.Hash`), 0 until a parse cache sets it.
- `tsrs_cli/src/tsc/emit.rs`, `tsrs_testrunner/src/{compile,types_dump,type_symbol_baseline}.rs`: pass
  `&Context::default()` to the ctx-taking entry points.

## Deviations

- The built-in pool's `getCheckerNonExclusive` / `getCheckerForFileNonExclusive` lock the checker (Go returns it
  unlocked): Rust hands out `&mut Checker` only under the lock. Same as before this change; nested acquisition of the
  same built-in checker on one thread would deadlock where Go would alias (no caller does this; Go's own contract
  forbids nested acquisitions for the project pool).
- `new_program` runs the pool factory after `verify_compiler_options` (Go: before), because the factory takes
  `&'static Program` and the program is leaked after verification. Unobservable: pools create checkers lazily.
- External-pool fallback in `collectCheckerDiagnosticsFromFiles` / `GetDeclarationDiagnostics`: Go's work group ->
  the compiler's rayon worker pool (256 MB stacks); single-threaded runs go last-queued first like Go's work group.
- `Context::with_timeout`: the deadline is observed when the context is polled (`err`, `is_canceled`, `wait*`);
  expiry alone does not fire `after_func` callbacks (Go fires them from its timer). Callers in Go (session watch
  requests) only poll / wait.
- `DuplicateSourceFile` has no content-mapper fields.
- Programs, checker slots and processed-file data are leaked (`&'static`), per the LSP.md memory plan (phase 4).

## Needs from others

- `tsrs_project` pool: implement `CheckerPool`, hand out checkers with `PooledChecker::as_non_null` +
  `CheckerHandle::from_raw` (see `program_test.rs`). Its parse cache should set `SourceFile.hash`.

## Doubts

- `CheckerHandle` holding a `MutexGuard` is `!Send`: a handle must be released on the thread that acquired it. Go's
  release funcs may run anywhere; no LS/project call site moves a checker between goroutines mid-use, but a pool
  release triggered from another thread (e.g. `context.AfterFunc`) must not own a handle.
