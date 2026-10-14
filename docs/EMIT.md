# Emit: JavaScript, declarations, source maps, `--build`

Emit is enabled by default, as in tsgo. JavaScript and declaration emit, source maps, incremental builds and
project-reference builds are ported. The [compatibility table](COMPATIBILITY.md#what-it-does-and-doesnt-do)
records the current baseline counts and limitations.

`noEmit`, `emitDeclarationOnly`, `noEmitOnError`, `incremental` and `composite` control what is written. An
incremental `--noEmit` run can still write `.tsbuildinfo`, as in Go. The former `TSRS_EMIT` environment variable
has no effect; `crates/tsrs_cli/tests/default_emit.rs` covers that behavior.

The [original port plan](history/EMIT.md) is kept separately. The rules for maintaining the port are in
[PORTING.md](PORTING.md); the reference commit and npm nightly are pinned in `Cargo.toml`.

## Emit pipeline

```
execute/tsc.go  performCompilation / performIncrementalCompilation (incremental/)  / build.Orchestrator (-b, build/)
  └ execute/tsc/emit.go  EmitFilesAndReportErrors
       ├ compiler.GetDiagnosticsOfAnyProgram          (tsrs_compiler)
       ├ Program.Emit(EmitOptions{WriteFile})         compiler/program.go:1875
       │   ├ HandleNoEmitOptions (noEmit / noEmitOnError)
       │   ├ getSourceFilesToEmit -> sourceFileMayBeEmitted        (emitter.rs)
       │   └ per file, in parallel (work group): newEmitHost (acquires and locks the file's checker),
       │     outputpaths.GetOutputPathsFor, emitter.emit():
       │       ├ emitJSFile:  getScriptTransformers chain -> printer.NewPrinter -> printSourceFile
       │       │     metadata? -> typeEraser -> importElision? -> runtimeSyntax -> legacyDecorators?
       │       │     -> jsx? -> estransforms.GetESTransformer(target) -> useStrict -> module transformer
       │       │     -> constEnumInliner (unless isolatedModules)
       │       ├ emitDeclarationFile: DeclarationTransformer + SupplementalReferencesTransformer
       │       │     -> printer (OnlyPrintJSDocStyle, NoEmitHelpers, declaration map via MapSourcePosition)
       │       └ printSourceFile: sourcemap.Generator, //# sourceMappingURL, .map file, EmitBOM, writeText
       │   └ CombineEmitResults (input order)
       ├ SortAndDeduplicateDiagnostics, report, listFiles (TSFILE: lines for --listEmittedFiles)
       └ exit status from EmitSkipped + diagnostics
```

`GetESTransformer` (estransforms/definitions.go) picks a chain by target. `ESNext` selects
`esDecorator + classFields`, but both constructors return nil when `useDefineForClassFields` is on (the default from
ES2022), and `esDecorator` also returns nil under `experimentalDecorators`, so with default options no estransform
runs at ESNext. ES2021–ES2026 add `using`; every older target adds more, down to ES2015, which runs every
estransform. Each transformer returns early on `SubtreeFacts`, so files without the syntax pass through.

## Implementation

### Crates

`tsrs_transformers` holds the transform chain, visitor utilities, resolver, transform options and emit-host
trait. `tsrs_declarations` depends on it and re-exports the shared transformer and resolver interfaces.

Go keeps `EmitHost` in the printer package, but its resolver needs the checker. The trait therefore lives in
`tsrs_transformers`, allowing `tsrs_printer` to stay independent of the checker. `ReferenceResolverRef` represents
Go's choice between the emit resolver and a plain binder resolver.

### Transformer handles

Go returns `*transformers.Transformer`, whose visitor closes over the concrete
transformer. In Rust, `P<Transformer>` points at the `base: Transformer` field of an arena-allocated concrete
transformer: `tx.get().base.new_transformer(visit, Some(ctx))` returns `P::from_static(&tx.base)`, and the `visit`
closure captures the `P<XTransformer>` (`&self` methods, `Cell`/`RefCell` fields), exactly as
`DeclarationTransformer` does. Factories are `fn(&TransformOptions) -> Option<P<Transformer>>`
(`TransformerFactory`; `None` is Go's nil from `newClassFieldsTransformer` and friends). Go's package-level
`transformers.Chain(...)` vars are functions that call `chain(&[...])(opts)` (estransforms/definitions.rs).

### Checker access

Go's `newEmitHost` locks the file's checker for the whole emit of that file. Rust lends the
checker to a `CheckerSlot` for the whole `emitter.emit()` (transform + print, JS + declarations), and `Resolver`
borrows it per call, the mechanism `get_declaration_diagnostics` already uses.

### Threading

`Program::emit` (`tsrs_compiler/src/program_emit.rs`) runs one emit pass per
checker group on the checker threads (`for_each_checker_group_do` over the files to emit, files in program order
within a group), like `get_declaration_diagnostics`. Each file's `EmitResult` goes into a slot indexed by its
position in the emit list, and `combine_emit_results` runs in input order, which is Go's observable order. With an
external checker pool (the project system), files take their checker from the pool one by one, as in Go. A fresh
text writer is created per file (Go pools them; the output is the same); emit contexts come from
`get_emit_context()`. `EmitOptions.write_file` is a `&(dyn Fn(&str, &str, &mut WriteFileData) -> Result<(),
String> + Sync)`, since it is called from the checker threads.

### Arena and regions

Each file's emit (and each file's declaration-diagnostics transform) runs in a scratch
region of its own (`arena::Region::new_scratch` + `enter_scratch`), freed once the file is printed and written
([region design and measurements](../notes/mem-emit-regions.md)). Transformed and synthesized nodes, emit contexts, transformer objects, printer scratch
and the node builder's per-request state (its contexts, trackers, recovery boundaries, pseudo types, links, and
the nodes it makes through a per-file emit context's factory, `NodeFactory::scratch`) are allocated there. The
checker escapes to its own arena (`CheckerSlot::with` calls `escape_scratch`), and so do diagnostics and the
emit host's program calls; the checker caches keyed by nodes that can be synthesized (the referenced-import links of
JSX factory names, accessible-chain entries whose scope is a synthesized node) drop those keys when the file's
transform ends. At most 2,048 transformed files wait for the print workers. The census (alloc-profile + plain-ptrs
build, `TSRS_CENSUS=1`) records a freed region as would-free and checks that nothing reachable points into it;
`TSRS_ARENA_POISON=1` fills it and never reuses it.

### Writing files

Go's `emitHost.WriteFile` is `program.Host().FS().WriteFile`. Rust: the compiler's
`EmitHost::write_file` calls `program.host().fs().write_file` (`tsrs_vfs`): the CLI's cached FS over the OS FS,
which creates missing directories (iovfs `write_file_ensuring_dir`); the harness's recorder FS over the
in-memory test FS. `Program::emit` is called by the CLI (`tsrs_execute::tsc::emit`, unless `--listFilesOnly`;
under `noEmit` it returns early in `handle_no_emit_options`), by the incremental builder for shape signatures
(`EmitOnlyBuilderSignature`, `tsrs_incremental`), by the Node API (`emit`, emit to string / selected files,
`transpile`) and by the harness under `--baselines js`.

### Source maps

`print_source_file` keeps Go's structure; the printer takes an
`Option<&mut SourceMapGenerator>` (`tsrs_sourcemap::Generator`) in `write` and implements Go's source-map paths
(printer_3.rs); `get_source_mapping_url`, `get_source_map_directory` and `get_source_root` are ported.

### Helpers

Go's helper definitions are `static`s in `tsrs_printer/src/helpers_defs.rs` (text copied byte for
byte), handed out as `SP<EmitHelper>` with `SP::from_static`, so identity comparisons behave like Go's pointers.
The rest of factory.go is `tsrs_printer/src/factory_2.rs` (`PrivateIdentifierKind` is an enum with `as_str`).

### Content mappers

Content mappers work in `tsc` and `tsc -b` ([content-mapper note](../notes/contentmappers.md)): a content-mapped file is not
emitted unless declarations are, and declaration maps point into the original text through the span map. The
conformance runner still skips the `runExternalCode` cases.

### Checker history

Go's inferred declaration types can depend on checker assignment and cache history.
tsrs's default mode removes several of those dependencies, with known exceptions recorded in the
[compatibility table](COMPATIBILITY.md#what-it-does-and-doesnt-do). For a strict reference comparison, use
`--checkerAssignment go --checkers 4 --noLazyMembers` against tsgo with `--checkers 4`; see
[the remaining history dependencies](../notes/open-history-dependence.md).

### Incremental programs

The CLI runs `performIncrementalCompilation` for incremental
projects (reads and writes `.tsbuildinfo`, described below). The `--baselines js` harness wraps the `@incremental` test
variants in `incremental.NewProgram` like Go's `createProgram` (with `testBuildInfoReader`), through
`compiler.ProgramLike`; the default mode keeps the plain program.

## Incremental compilation and builds

### Crates

`execute/incremental` is `tsrs_incremental`, with one file per Go file. Project-reference builds live in
`tsrs_execute::build`, beside `tsrs_execute::tsc`, as in Go. `compiler.ProgramLike`, `HandleNoEmitOptions` and
`GetDiagnosticsOfAnyProgram` live in `tsrs_compiler`; `tsrs_incremental::emit` re-exports the shared emit helpers.

### JSON

`BuildInfo` is marshaled by building a `tsrs_core::json::Value` tree field by field in Go's declaration
order with Go's `omitzero` rules (nil vs empty kept where Go distinguishes them, e.g. `fileInfos: []`) and the
tuple encodings of the custom `MarshalJSON`s; unmarshaling follows the `UnmarshalJSON` fallbacks.

The default CLI can add an optional canonical-check signature to build info for safe warm reuse. Go-history
and API formats retain their reference behavior; see
[the incremental diagnostic rules](../notes/fix-history-dependent-diagnostics.md).

### Version

The tsbuildinfo `version` is `core::version()` as in Go. Go's release builds stamp it with ldflags
(the npm nightly built from the pinned commit: `7.1.0-dev.20260930.4`); tsrs reads `TSRS_TS_VERSION` at build time for
the same purpose (source default `7.1.0-dev`), so a tsrs built with the tsgo version reads and writes
tsbuildinfo files that tsgo accepts, and vice versa. Different versions simply rebuild from scratch, as in Go. Release
builds stamp `[workspace.metadata.typescript] nightly` (Cargo.toml); release.yml checks that the nightly was built from
the pinned commit (or a later one that changes nothing under tsc/) and that the binaries report it.

### Default library path

Go's embedded build reads libs from `bundled:///libs`, the npm (noembed) tsgo from its
executable directory; the path sorts differently against project files, which reorders `referencedMap` keys. The
npm tsrs packages ship the libs next to the binary, and a binary with a `lib.d.ts` beside it reads them from there
like the npm tsgo (`bundled::default_lib_path`); a `cargo` build keeps the embedded ones. `TSRS_LIB_PATH=<dir>`
overrides both (the oracles set it to tsgo's directory).

### Concurrency

Go runs the per-file snapshot and affected-file work on work groups; tsrs runs it on the calling
thread in a deterministic order (sorted paths). The transforms run on the checker threads; each
transformed file is then printed and written on the rayon worker pool, at most 4 writes at a time
([emit measurements](../notes/perf-emit.md)), so the `WriteFile` callbacks only touch thread-safe state.

### Build concurrency

`--builders N` sets the builder count: four by default, one under `--singleThreaded`, with a minimum of one.
Builders take projects from Go's `Order()`, wait for upstream tasks and buffer their output. The reporter
prints those buffers in the same order. Completion signals use a mutex and condition variable; task state,
host caches and reporter types preserve the cross-thread ownership contract. A panicking builder releases
its waiters before the panic resurfaces.

Concurrent programs share the host's cached declaration and JSON source files, bound once and then read-only.
`TSRS_CHECK_SHARED=1` checks for writes to shared objects in debug builds. The incremental oracle fixtures cover
builder counts 1, 4 and 8, dependency errors, cycles, `--stopBuildOnErrors`, `--dry`, `--clean` and `--force`;
[the measurement note](../notes/emit-incremental.md) records the results.

CLI builds retain finished programs, so peak memory grows with the project graph. API builds free finished
programs' heap data with `free_unshared_program`, then release their arenas with the orchestrator's task regions.

### Shape signatures

Shape signatures follow Go: `computeDtsSignature` prints the `.d.ts` through `Program::emit` with
`EmitOnlyBuilderSignature`; files that were never shape-checked keep their version as signature (so the first edit
of a file after a cold build rechecks its importers, exactly like tsgo).

### Scenario harness

The Go scenario tables are closures, so `tools/oracle/tsctests/dump.sh` runs the Go tests once
with a recorder (`runner.patch` + `tsrs_dump.go`) and writes every scenario as JSON: inputs plus the file-system
ops of every edit, both in the incremental run and replayed from scratch for the non-incremental comparison. The
harness (`crates/tsrs_execute/src/tsctests`, a `#[cfg(test)]` module of the driver crate because it drives
`execute::command_line_with_testing`) replays them with a fake clock, the FS differ, the readable buildinfo and the
output sanitizer of `tsctests/sys.go`. Watch scenarios (`tscWatch/`, `tsbuildWatch/`) drive the watcher through
Go's mock watch backend (`tsctests/mockwatch.rs`); content-mapper scenarios use the in-process test mappers of
`tsrs_contentmappertest`.

## Validation

Build the CLI and conformance runner as described in [CONTRIBUTING.md](../CONTRIBUTING.md), then run:

```sh
./target/release/tsrs-test run --suite all --baselines types,symbols,js,jsmap,sourcemap
./target/release/tsrs-test show <suite/name> --js
```

The emit harness mirrors Go's pre- and post-emit programs, output ordering, declaration recompilation and
`noCheck` repeat. Its `baseline.DiffText` uses the `similar` crate's patience algorithm and unified format.
Go's own nondeterministic emit-test skip list is preserved.

### Emit oracle

Build tsgo from the TypeScript commit pinned in `Cargo.toml`, then set `TSGO` to that binary. Set `TSRS` to the
binary under comparison; it defaults to `target/release/tsrs`.

```sh
TSGO=/path/to/tsgo-ref tools/oracle/emit/run.py /path/to/tsconfig.json -- --noEmit false
TSGO=/path/to/tsgo-ref tools/oracle/emit/run.py /path/to/project -- --noEmit false --declaration --sourceMap --declarationMap
TSGO=/path/to/tsgo-ref tools/oracle/emit/run.py /path/to/project --buildinfo -- --noEmit false
```

`run.py` redirects `outDir`, `declarationDir` and `tsBuildInfoFile` to separate reference and Rust trees under
`target/scratch/emit-oracle/<name>`. It compares diagnostics, exit codes and emitted files byte for byte;
`--buildinfo` includes the build-info files. It preserves the project's incremental setting and has no `-b`
mode. Extra compiler flags follow `--`; `--name`, `--go-out`, `--rs-out` and `--json` configure the oracle.

Match both the compiler version and library location when comparing build info. The npm packages read libraries
beside the binary; Cargo builds use embedded libraries unless `TSRS_LIB_PATH` overrides them.

`tools/oracle/emit/monorepo.sh <root> [-j N] [--filter RE] [-- flags]` compares workspace packages whose build
script runs `tsc`. It requires a clean worktree, redirects outputs and checks that the worktree remains clean.
It emits packages with `-p`, including packages whose normal script uses `--build`; it does not validate build
mode. Its summary requires exactly one identical result per selected package and fails on differences,
NotImplemented exits, signals, timeouts, malformed or missing results, and an empty selection. Matching
nonzero compiler statuses from 1 to 4 are accepted. `tools/oracle/emit/test_monorepo.sh` checks it on a disposable
two-package fixture.

## Known limitations

CLI build mode retains finished programs and can use more memory than tsgo on large graphs. Content-mapper conformance cases that need `runExternalCode` are not run yet.

The const-enum inliner checks property accesses during emit, so emit can change where a diagnostic is first
reported, as in tsgo. A `--noEmit` run does not take that path. Match the pinned TypeScript source, compiler
version and library layout when investigating a porting difference.
