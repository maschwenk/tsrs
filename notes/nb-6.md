# nb-6: tsrs_modulespecifiers bodies + compiler host side

Ported every function of `internal/modulespecifiers/{compare,preferences,specifiers,util}.go` and Go
`Program.GetSymlinkCache` (+ `ForEachResolvedModule`, `ForEachResolvedTypeReferenceDirective`, `forEachResolution`) in
tsrs_compiler. (`ProcessEntrypointEnding` was left out at the time; it is now in util.rs as `process_entrypoint_ending`.)

Status (2026-10-10): this was a porting-wave log. The signature-change, shared-file and "needs from others" lists are
done and were removed. The old "project references are not ported" item is no longer true: program.rs
`get_project_reference_from_source` and emithost.rs implement it through `project_reference_file_mapper`, and no
`todo!("compiler: project references")` is left.

## Validation

- `cargo test -p tsrs_modulespecifiers`: port of specifiers_test.go plus a few helper tests.
- `cargo test -p tsrs_compiler modulespecifiers_oracle`: differential test against Go. Go oracle
  `tools/oracle/modulespecifiers/main.go` (built from `ts-ref/tsc/cmd/tsrs-oracle-modulespecifiers`, binary
  `bin/tsrs-oracle-modulespecifiers`) loads in-memory projects (vfstest + bundled libs, `tsc -p`-style config parsing,
  `compiler.NewProgram`) and prints `GetModuleSpecifiersForFileWithInfo` for (importing, target, ending, override mode,
  preference) plus the sorted `GetSymlinkCache().DirectoriesByRealpath()`. Scenarios come from
  `tools/oracle/modulespecifiers/gen.py`; the data is checked in under
  `crates/tsrs_compiler/testdata/modulespecifiers_oracle/` (regeneration commands are at the top of
  `crates/tsrs_compiler/src/modulespecifiers_oracle_test.rs`). They cover relative/index/endings/.mts/.cts/.d.mts/.tsx,
  nodenext (existing-import reuse), allowImportingTsExtensions, paths/baseUrl, project-relative boundary, rootDirs,
  node_modules (types/main/exports conditions+patterns+null/@types/typesVersions/no package.json) under
  nodenext/node10/bundler, sibling node_modules, symlinked workspace packages (symlink found by resolution and by
  package.json dependency), package.json imports, case-insensitive FS, JSON/arbitrary extensions, pnpm, duplicate
  packages (redirect targets), exports directory/array/versioned-types/custom conditions, own-package symlink.
  At port time: 18 scenarios, 161 requests, all identical to Go.

## Deviations from Go

- `ModuleSpecifierPreferences.get_allowed_endings_in_preferred_order` is a method over stored captures (`prefs`,
  `importing_source_file`, `old_import_specifier`) instead of a boxed closure field: a `'static` closure cannot capture
  the borrowed `&dyn Host`/`&CompilerOptions`. Every caller holds the same host/options Go captured.
- `is_excluded_by_regex` compiles patterns with Rust `regex` instead of Go `regexp` (both RE2-like; only reachable with
  `AutoImportSpecifierExcludeRegexes`, which the checker never sets).
- Go iterates `sync.Map`-backed sets (symlink directories, `GetRuntimeDependencyNames`) and Go maps
  (`sourceFileMetaDatas`) in random order; Rust iterates hash order. Results are sorted afterwards
  (`getAllModulePathsWorker`) or order-insensitive, as in Go.
- `get_symlink_cache` unwraps `get_source_file_by_path(meta path)` where Go would pass nil into
  `SourceFileMayBeEmitted` (and crash); never observed in the oracle scenarios.

## Go quirks kept on purpose

- `tryGetModuleNameAsNodeModule`'s retry loop passes the unchanged `parts` every iteration (Strada updated
  `parts.packageRootIndex`).
- Exports directory mappings produce e.g. `dirs/d/dist/d/deep/file.js`.
