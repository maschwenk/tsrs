# nb-6: tsrs_modulespecifiers bodies + compiler host side

Ported every function of `internal/modulespecifiers/{compare,preferences,specifiers,util}.go` (56 functions; only
`ProcessEntrypointEnding` stays out, LS auto-imports) and Go `Program.GetSymlinkCache` (+ `ForEachResolvedModule`,
`ForEachResolvedTypeReferenceDirective`, `forEachResolution`) in tsrs_compiler.

Validation:
- `cargo test -p tsrs_modulespecifiers`: port of specifiers_test.go plus a few helper tests.
- `cargo test -p tsrs_compiler modulespecifiers_oracle`: differential test against Go. Go oracle
  `tools/oracle/modulespecifiers/main.go` (built from `ts-ref/tsc/cmd/tsrs-oracle-modulespecifiers`, binary
  `bin/tsrs-oracle-modulespecifiers`) loads in-memory projects (vfstest + bundled libs, `tsc -p`-style config parsing,
  `compiler.NewProgram`) and prints `GetModuleSpecifiersForFileWithInfo` for (importing, target, ending, override mode,
  preference) plus the sorted `GetSymlinkCache().DirectoriesByRealpath()`. Scenarios (`gen.py`, checked in under
  `crates/tsrs_compiler/testdata/modulespecifiers_oracle/`): relative/index/endings/.mts/.cts/.d.mts/.tsx, nodenext
  (existing-import reuse), allowImportingTsExtensions, paths/baseUrl, project-relative boundary, rootDirs, node_modules
  (types/main/exports conditions+patterns+null/@types/typesVersions/no package.json) under nodenext/node10/bundler,
  sibling node_modules, symlinked workspace packages (symlink found by resolution and by package.json dependency),
  package.json imports, case-insensitive FS, JSON/arbitrary extensions, pnpm, duplicate packages (redirect targets),
  exports directory/array/versioned-types/custom conditions, own-package symlink. 18 scenarios, 161 requests: all
  identical to Go.

## Signature changes

All internal to the crate (the checker only calls `get_module_specifiers` and `count_path_components`, unchanged).
- `ModuleSpecifierPreferences` (types.rs): the `get_allowed_endings_in_preferred_order: Box<dyn Fn(ResolutionMode)>`
  field -> stored captures (`prefs`, `importing_source_file`, `old_import_specifier`) + method
  `get_allowed_endings_in_preferred_order(&self, host, compiler_options, mode)`. A `'static` boxed closure cannot capture
  the borrowed `&dyn Host`/`&CompilerOptions` (no lifetime params); every caller holds the same host/options Go captured.
- By-value non-Copy structs -> references in `pub(crate)` fns: `ModulePath`, `Info`, `UserPreferences`,
  `ModuleSpecifierPreferences`, `specPair` (`compare_paths_by_redirect`, `get_all_module_paths[_worker]`,
  `compute_module_specifiers`, `get_local_module_specifier`, `try_get_module_name_as_node_module`,
  `try_directory_with_package_json`, `validate_ending`, `get_preferred_ending`, `get_module_specifier_preferences`,
  `get_module_specifier_with_preferences`); `pub get_allowed_endings_in_preferred_order(prefs: &UserPreferences, ..)`.
  Public entry points keep `UserPreferences` by value.
- `&[&str]` -> `&[String]` where the Go data is `[]string` held as `Vec<String>`: `is_excluded_by_regex(excludes)`,
  `try_get_module_name_from_exports[_or_imports](conditions)` (`module::get_conditions` returns `Vec<String>`),
  `try_get_module_name_from_root_dirs`/`get_paths_relative_to_root_dirs(root_dirs)`.
- `get_node_module_path_parts -> Option<NodeModulePathParts>` (was `Option<P<..>>`; the struct is Copy, a P leaks per call).
- New private helpers: `index_of_ending` (Go `slices.Index`), `module_resolution_is_node_next`, `cut` (`strings.Cut`).

## Shared-file edits

- `crates/tsrs_compiler/src/program.rs`: `Program.known_symlinks: OnceLock<P<KnownSymlinks>>` (Go `knownSymlinks
  lazyValue`), `get_symlink_cache`, `for_each_resolved_module`, `for_each_resolved_type_reference_directive`, free fn
  `for_each_resolution`. `checker_program.rs`: `get_symlink_cache` forwards to it (was `todo!()`).
- `crates/tsrs_compiler/src/lib.rs`: `#[cfg(all(test, feature = "checker"))] mod modulespecifiers_oracle_test;` + testdata.
- `tools/sigs-from-rust.py`: skip `*_test.rs` files; regenerated docs/sigs/modulespecifiers.txt.
- docs/CHECKER.md modulespecifiers paragraph (bodies ported).

## Needs from others

- Project references are not ported in tsrs_compiler: `ModuleSpecifierGenerationHost::get_project_reference_from_source`
  keeps its pre-existing `.map(|_| todo!("compiler: project references"))` (unreachable: the mapper always returns None).

## Doubts

- `is_excluded_by_regex` compiles patterns with Rust `regex` instead of Go `regexp` (both RE2-like; only reachable with
  `AutoImportSpecifierExcludeRegexes`, which the checker never sets).
- Go iterates `sync.Map`-backed sets (symlink directories, `GetRuntimeDependencyNames`) and Go maps
  (`sourceFileMetaDatas`) in random order; Rust iterates hash order. Results are sorted afterwards
  (`getAllModulePathsWorker`) or order-insensitive, as in Go.
- `get_symlink_cache` unwraps `get_source_file_by_path(meta path)` where Go would pass nil into
  `SourceFileMayBeEmitted` (and crash); never observed in the oracle scenarios.
- Faithfully kept Go quirks: `tryGetModuleNameAsNodeModule`'s retry loop passes the unchanged `parts` every iteration
  (Strada updated `parts.packageRootIndex`); exports directory mappings produce e.g. `dirs/d/dist/d/deep/file.js`.
