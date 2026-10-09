# lsp-fsgen: fourslash generator, harness and runner (`tools/gen-fourslash`, `tsrs_fourslash`)

## What exists

| Go | Rust |
| --- | --- |
| `fourslash/tests/*_test.go` (4,364 files, 4,559 `Test*` functions) | `crates/tsrs_fourslash/src/tests/gen/gen_NN.rs` (generated, one `pub mod` per Go file, one `pub fn` per Go function) + `gen/mod.rs` (`REGISTRY`: name, Go file, line, fn, skip reason) |
| `fourslash/tests/util/util.go` | `tests/util.rs` (functions, by hand) + `tests/util_gen.rs` (package-level vars as `LazyLock` statics, generated) |
| `fourslash/test_parser.go` | `test_parser.rs` (full port) |
| `testrunner/test_case_parser.go` `ParseTestFilesAndSymlinksWithOptions` | `testrunner.rs` (the Go version with the erroring `parseFile`; tsrs_testrunner is a binary crate with its own non-erroring copy) |
| `fourslash/fourslash.go` | `fourslash.rs`: struct, options/expected-value types, capabilities (`GetDefaultCapabilities*`, `getCapabilitiesWithDefaults`), `newFourslash` up to the server, `getBaseFileNameFromTest`, scriptInfo/converters, marker/range accessors, GoTo* navigation, `GetRangesByText`, `verifyBaselines`; every method that talks to the server calls `FourslashTest::server_unavailable(t, "<Method>")` |
| `fourslash/baselineutil.go` | `baselineutil.rs`: commands, file names/extensions, options, `addResultToBaseline`, `getBaselineContentForFile`, `textWithContext`, `annotateContentWithTooltips`, `codeFence`, `symbolInformationToData` |
| `fourslash/semantictokens.go` | `semantictokens.rs` (decode/format ported, verify stubbed) |
| `fourslash/statebaseline.go` | `statebaseline.rs` (struct only: everything it prints comes from the server's session) |
| `testutil.RecoverAndFail`, `testutil/baseline` | `testutil.rs` (`recover_and_fail`, `baseline::run` = Go's Run/writeComparison: references from `ts-ref/tsc/testdata/baselines/reference`, actuals to `target/fourslash-results/local`) |
| `testing.T` | `testing.rs` (`T`: failures/logs behind a Mutex, `fatal`/`skip` unwind with `FatalPanic`/`SkipPanic`, `run` = subtests) |
| `go test` | `runner.rs` + binary `tsrs-fourslash` (`run [--filter re] [--include-skipped] [-j N] [-v]`, `list`) |
| `ls.SortText*`, `contentmappertest`, `stringtestutil.Dedent` | `ls_shim.rs` (generated code's `ls::`), `contentmappertest.rs`, `contentmapper.rs`, `stringtestutil.rs` |

Regenerate: `cd tools/gen-fourslash && GOTOOLCHAIN=auto go run .` (≈5 s; deterministic). `-survey` prints node
kinds / call targets / interface conversions, `-sigs` the fourslash API the tests use, `-parser-inputs FILE` the
constant contents for the parser oracle. Parser oracle: `tools/oracle/fourslash-parser/run.sh`.

## Generator design (how Go maps to Rust)

- Type info from `go/types`: tests/util and the two test packages (`fourslash_test`, and the 3 files in package
  `fourslash`) are checked from source, everything else from `go list -export` data. No x/tools.
- Types: `string` -> `String` (param `&str`), `int` -> `i32`, `[]T` -> `Vec<T>` (param `&[T]`, `[]string` param
  `&[&str]`), `[]*T` -> `Vec<T>`, `*T` -> `Option<T>`, `map` -> `OrderedMap`, `*testing.T` -> `&T`,
  `*FourslashTest` -> owned from constructors / `&mut FourslashTest` everywhere else, `*Marker`/`*RangeMarker`/
  `*TestFileInfo`/`*MultiMap` -> `Arc<_>` (in struct fields `Option<Arc<_>>`), lsproto/lsutil per their crates.
- Go `any` and all its aliases (MarkerInput, MarkerOrRangeOrName, CompletionsExpectedItem,
  ExpectedCompletionEditRange) -> one enum `go::Any` (variant per dynamic type, `Nil`); interface
  `MarkerOrRange` -> enum. `x.(bool)` -> `x.assert_bool()`.
- `defer CALL` -> the rest of the block runs under `go::run`, then CALL, then `go::resume` on normal return;
  `defer testutil.RecoverAndFail(t, msg)` -> `testutil::recover_and_fail(t, msg, result)`.
- Closures that use the FourslashTest receive it as first parameter (PORTING.md callback rule): harness callbacks
  (`GoToEachMarker`), local closures; harness-returned closures (`done`, `reset`, `VerifyCompletionsResult`
  fields) are `Box<dyn FnOnce(&mut FourslashTest, &T, ..)>` and called as `done(&mut *f, t)`.
- A leading `t.Skip(...)` becomes the registry skip reason (and is dropped from the body, so
  `--include-skipped` runs the test).
- Package-level declarations of test files live in their file's module; cross-file references are resolved
  after the modules are assigned to `gen_NN` files.

## Gates (2026-10-01)

- `cargo check -p tsrs_fourslash`: 0 errors, 0 warnings, all 4,559 tests generated and compiling; untranslated
  constructs: none.
- Compile time of the crate alone (deps built, `CARGO_INCREMENTAL=0`): check 2.8 s, debug build 52 s
  (dev profile is opt-level 1); generated code is 14 MB (one Go test file alone is 4 MB).
- `tsrs-fourslash run`: 4,559 tests, 4,173 fail with "server unavailable: NewFourslash ...", 386 skipped (known
  failing), 0.1 s. With `--include-skipped` all 4,559 fail, 4,547 with server unavailable plus
  `TestImportSuggestionsCache_invalidPackageJson` whose content Go's parser also rejects (and the 11 parent tests of
  subtests, reported with the subtest's message).
- `cargo test -p tsrs_fourslash --lib`: parser unit test + oracle sample (617 cases); the full oracle
  (`TSRS_FOURSLASH_PARSER_ORACLE=target/scratch/fsgen/parser_out.jsonl`): all 4,484 constant contents produce the
  same files, markers (positions, LSP positions, names, object data), named markers, ranges (with marker links),
  symlinks and global options as Go's `ParseTestData`, including Go's parse failures.

## Shared-file edits

- `Cargo.toml`: `tsrs_fourslash` in `[workspace.dependencies]`; `Cargo.lock`.
- `ts-ref/tsc/cmd/tsrs-oracle-fourslash-parser/oracle_test.go` (oracle copy, as PORTING.md allows; source of
  truth `tools/oracle/fourslash-parser/oracle_test.go`).

## Deviations

- `fourslash.AnyTextEdits` / `NoTextEdits` are compared by pointer identity in Go; Rust uses sentinel values (one
  edit with an impossible text) and `is_any_text_edits` / `is_no_text_edits`.
- `GetRangesByText` takes `&self` (cache in a `OnceLock`) because tests call it inside arguments of `&mut self`
  methods. `MarkerByName` fails the test for an unknown name instead of returning nil.
- `scriptInfos` is `Arc<RwLock<FxHashMap<..>>>` shared with the converters' line-map callback (Go shares the map);
  `getScriptInfo` returns a snapshot.
- Go maps that the harness iterates (`Symlinks`, `GlobalOptions`, `MarkerPositions`, baselines) are `OrderedMap`
  (insertion order instead of Go's random order).
- `Marker.Data` is an `OrderedMap<String, Any>` holding what Go's encoding/json produces for `any` (empty = Go nil;
  object markers always have keys).
- Two hand-ported lsutil fields are `Option<Vec<String>>` where Go has `[]string`
  (`AutoImportFileExcludePatterns`, `AutoImportSpecifierExcludeRegexes`); the generator wraps them in `Some`.
- The generator hoists the arguments of a FourslashTest method call into `let`s when one passes the test to a
  closure (`f.verify_completions(t, m, path_completion(&mut *f, ..))`); Go evaluates the receiver first, but it is
  a plain variable.
- `T::run` names subtests `parent/name` with whitespace replaced by `_` (Go's `rewrite` also escapes unprintables).

## Needs from others

- The in-process server (`lsptestutil.LSPClient` over `tsrs_lsp`): replace `FourslashTest::server_unavailable`
  call sites (start with `new_fourslash_impl`: compiler options via `harnessutil.SetOptionsFromTestConfig`, the
  vfs from `testfs`, `initialize`, opening files; `new_done_fn` is ready) and port the method bodies from
  fourslash.go.
- `ls` completions: when `SortText*` / `DeprecateSortText` / `SortBelow` / `ObjectLiteralPropertySortText` exist in
  `tsrs_ls`, point `ls` in `tests/prelude.rs` at them and delete `ls_shim.rs`.
- Content mappers are out of scope: the content-mapper tests build a placeholder spawner.

## Doubts

- `go::Any` derives `PartialEq` by value (Go compares interface values holding pointers by identity); only the
  harness's expected-vs-actual comparisons use it.
- `int` -> `i32` everywhere in the harness API (PORTING.md would use `usize` for counts/indexes); chosen so the
  generator needs no per-parameter decisions.
