# lsp-fsgen: fourslash generator, harness and runner (`tools/gen-fourslash`, `tsrs_fourslash`)

`tools/gen-fourslash` (Go, `go/types`) turns every Go fourslash test into a Rust function in
`crates/tsrs_fourslash/src/tests/gen/gen_NN.rs` (one `pub mod` per Go file, one `pub fn` per Go function) plus a
registry in `gen/mod.rs` (name, Go file, line, fn, skip reason). The harness (`fourslash.rs`, `baselineutil.rs`,
`statebaseline.rs`, `harnessutil.rs`, `tsbaseline.rs`, `testing.rs`, `test_parser.rs`) is a port of Go's and drives the
in-process server. How to regenerate and run, and the pass counts: docs/LSP.md, "Fourslash". This note keeps the
mapping rules of the generator and the harness's deviations from Go. (It absorbs the
former note of the wave that wired the harness to the in-process server.)

Generator options: `-survey` prints node kinds / call targets / interface conversions, `-sigs` the fourslash API the
tests use, `-parser-inputs FILE` the constant contents for the parser oracle (`tools/oracle/fourslash-parser/run.sh`;
at the time all 4,484 constant contents produced the same files, markers, ranges, symlinks and global options as Go's
`ParseTestData`, including Go's parse failures). Generation takes about 5 s and is deterministic; test files that
`go test` does not build (Go build constraints, e.g. `*_js_test.go`) are skipped with `go/build` `MatchFile`.

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
- `defer CALL` -> the rest of the block runs under `go::run` (catch_unwind), then CALL, then `go::resume`;
  `defer testutil.RecoverAndFail(t, msg)` -> `testutil::recover_and_fail(t, msg, result)`.
- Closures that use the FourslashTest receive it as first parameter (PORTING.md callback rule): harness callbacks
  (`GoToEachMarker`), local closures; harness-returned closures (`done`, `reset`, `VerifyCompletionsResult`
  fields) are `Box<dyn FnOnce(&mut FourslashTest, &T, ..)>` and called as `done(&mut *f, t)`.
- A leading `t.Skip(...)` becomes the registry skip reason (and is dropped from the body, so
  `--include-skipped` runs the test).
- Package-level declarations of test files live in their file's module; cross-file references are resolved
  after the modules are assigned to `gen_NN` files.

## Harness deviations

- `fourslash.AnyTextEdits` / `NoTextEdits` are compared by pointer identity in Go; Rust uses sentinel values (one
  edit with an impossible text) and `is_any_text_edits` / `is_no_text_edits`.
- `GetRangesByText` takes `&self` (cache in a `OnceLock`) because tests call it inside arguments of `&mut self`
  methods. `MarkerByName` fails the test for an unknown name instead of returning nil.
- `scriptInfos` is `Arc<RwLock<FxHashMap<..>>>` shared with the converters' line-map callback (Go shares the map);
  `getScriptInfo` returns a snapshot. Go mutates the shared `*scriptInfo` and later conversions in the same function
  see the edit; the Rust methods re-read the script info after each edit at the same points.
- Markers are shared `Arc<Marker>` / `Arc<RangeMarker>`; Go edits the shared objects in place on every edit.
  `editScriptAndUpdateMarkersWorker` replaces the edited markers by updated copies in `markers`, `marker_positions`,
  `ranges` and the ranges' `marker` links; a marker a test took out before an edit keeps the old position.
- `handleServerRequest` reads `f.userPreferences` from the client's router goroutine; the Rust handler reads an
  `Arc<Mutex<UserPreferences>>` copy that `Configure` keeps in sync.
- `assertDeepEqual` prints both values' Debug forms instead of `cmp.Diff`; `diagnosticsIgnoreOpts` is applied by
  clearing `severity`, `source`, `related_information` on both sides.
- Go maps that the harness iterates (`Symlinks`, `GlobalOptions`, `MarkerPositions`, baselines) are `OrderedMap`
  (insertion order instead of Go's random order). `VerifyBaselineNonSuggestionDiagnostics` iterates the script infos
  sorted by name (the output is sorted either way).
- `tsbaseline.rs` is the pretty=false subset of `GetErrorBaseline` over fourslash diagnostics; the global error /
  library / tsconfig count assertions of `iterateErrorBaseline` are not ported (fourslash diagnostics always have a
  file).
- `Marker.Data` is an `OrderedMap<String, Any>` holding what Go's encoding/json produces for `any` (empty = Go nil;
  object markers always have keys).
- Two hand-ported lsutil fields are `Option<Vec<String>>` where Go has `[]string`
  (`AutoImportFileExcludePatterns`, `AutoImportSpecifierExcludeRegexes`); the generator wraps them in `Some`.
- The generator hoists the arguments of a FourslashTest method call into `let`s when one passes the test to a
  closure (`f.verify_completions(t, m, path_completion(&mut *f, ..))`); Go evaluates the receiver first, but it is
  a plain variable.
- `T::run` names subtests `parent/name` with whitespace replaced by `_` (Go's `rewrite` also escapes unprintables).
- An unrecovered server-thread panic ends the whole Go test binary; tsrs's worker processes turn it into one failing
  test, so a crash does not hide later results (but also cannot fail the run as a whole).

## Doubts

- `go::Any` derives `PartialEq` by value (Go compares interface values holding pointers by identity); only the
  harness's expected-vs-actual comparisons use it.
- `int` -> `i32` everywhere in the harness API (PORTING.md would use `usize` for counts/indexes); chosen so the
  generator needs no per-parameter decisions.
