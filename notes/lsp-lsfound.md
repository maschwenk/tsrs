# lsp-lsfound: language-service foundation (astnav, lsutil, format, sourcemap, lsconv/linemap)

All into crate `tsrs_ls`, one Rust file per Go file, `// file.go:LINE` origin markers checked against the Go source.
Sourcemap, lsconv/linemap, lsutil formatcodeoptions/userpreferences were ported by a sub-agent (details in the last
section, merged from its notes).

## Ported

| Go (`ts-ref/tsc/internal/…`) | Rust (`crates/tsrs_ls/src/…`) |
| --- | --- |
| `astnav/tokens.go` (+ `tokens_test.go`) | `astnav/tokens.rs` (+ `tokens_test.rs`) |
| `ls/lsutil/{asi,children,utilities,completednode,organizeimports}.go` (+ `utilities_test.go`) | `lsutil/*.rs` (+ `utilities_test.rs`) |
| `ls/lsutil/formatcodeoptions.go` minus `FromLSFormatOptions`/`ToLSFormatOptions`, `userpreferences.go` (+ test) | `lsutil/formatcodeoptions.rs`, `lsutil/userpreferences.rs` (+ `userpreferences_test.rs`) |
| `format/{api,context,indent,rule,rulecontext,rules,rulesmap,scanner,span,util}.go` (+ the 5 test files) | `format/*.rs` (+ `api_test.rs`, `comment_test.rs`, `format_test.rs`, `indent_test.rs`, `indent_getindentation_test.rs`, `oracle_test.rs`) |
| `sourcemap/{decoder,source_mapper,lineinfo,util,source,generator}.go` (+ `generator_test.go`) | `sourcemap/*.rs` (+ `generator_test.rs`, `source_mapper_test.rs`) |
| `ls/lsconv/linemap.go` | `lsconv/linemap.rs` |

Not ported (by brief): `symbol_display.go` (hover agent), `lsconv/converters.go` and the two lsproto functions of
formatcodeoptions.go (need lsproto).

## Tests (`cargo test -p tsrs_ls --lib`: 81 pass)

- astnav 11/11: the four "go baseline json" tests reproduce `testdata/baselines/reference/astnav/*.baseline.json`
  byte for byte; the three TS-comparison baselines (`*.baseline.txt`, Go's `baselineTokens` via node + typescript 6.0.3)
  also match when `TSRS_TYPESCRIPT_JS=<path to typescript.js>` is set (skipped otherwise, like `jstest.SkipIfNoNodeJS`;
  verified with typescript 6.0.3 installed under target/scratch); plus the JSDoc/pointer-equality/unit cases.
- lsutil: utilities 3/3, userpreferences 11/11 (sub-agent).
- format 16/16 Go unit tests, plus `oracle_test.rs`: exact edit-for-edit comparison with the Go formatter
  (`tools/oracle/format`, opt-in via `TSRS_FORMAT_ORACLE` / `TSRS_FORMAT_ORACLE_PRESET`). Results: all 12750
  compiler+conformance test files match Go for FormatDocument with default settings, with two non-default presets
  (`alt`: tabs, all optional spaces flipped, braces on new lines, semicolons=remove; `insert`: same + semicolons=insert,
  CRLF) and for `GetIndentation` at every line start (`indent`); checker.ts with the api_test settings matches.
- sourcemap: generator 32/32, source_mapper 7 (no Go tests), linemap 1; tsrs_core goslices 3/3.

## Shared-file edits

- `crates/tsrs_ast/src/ast.rs`: `SourceFile.token_cache` (`Mutex<FxHashMap<TokenCacheKey, P<Node>>>`), `TokenCacheKey`,
  `SourceFile::get_or_create_token`, `create_token` (Go `GetOrCreateToken`/`createToken`, ast.go:2909/2940). Go keeps a
  lazily created per-file `tokenFactory`; a `NodeFactory` handle is not `Sync`, so each call uses a fresh default factory
  (factories carry no state tokens observe). Already cherry-picked onto `lsp` by the lead (4d8e2ef).
- `crates/tsrs_ast/src/utilities_3.rs` (appended, origin markers): `find_last_visible_node`, `is_statement_but_not_declaration`,
  `is_non_whitespace_token`, `is_whitespace_only_jsx_text`, `for_each_child_and_jsdoc`, `is_jsdoc_single_comment_node_list`,
  `is_jsdoc_single_comment_node_comment`, `is_jsdoc_single_comment_node`, `is_trivia`, `has_comment` (private).
- `docs/AST.md`: token cache is now ported.
- `crates/tsrs_core/src/stringutil/{norm.rs,norm_generated.rs}` (+ mod.rs lines): Go `unicode.Is(unicode.Mn, r)`,
  `unicode.IsUpper(r)` and `golang.org/x/text/unicode/norm` `NFD.String(s)` for organizeimports' collation. Tables are
  generated from Go (Unicode 17.0.0, x/text v0.42.0) by `tools/oracle/unicodenorm/main.go`.
- `crates/tsrs_core/src/goslices.rs`: `sort_func` (sub-agent; Go `slices.SortFunc`, same order of equal elements).
- `tools/oracle/format/main.go`, `tools/oracle/unicodenorm/main.go` (copies also in `ts-ref/tsc/cmd/tsrs-oracle-*`).

## Deviations

- Go `context.Context` in `format`: `tsrs_core::context::Context` does not exist on this branch, and the format package
  stores two values in it (`formatOptionsKey`, `formatNewlineKey`). Every Go `ctx` parameter of `format` is a
  `&format::FormatContext` carrying exactly those two values (`with_format_code_settings` returns a new one, like
  `context.WithValue`). When the real Context lands, these values should move into it (or `FormatContext` hangs off it).
- Go node visitors with closure hooks (`ast.NewNodeVisitor` + `VisitNode`/`VisitNodes`/`VisitModifiers` hooks) are kept
  in astnav/children (`Rc<dyn Fn>` hooks over `Rc<Cell<…>>` state, same dispatch through the generated
  `visit_each_child`). In `format/span.go` the visitor's callbacks re-enter the worker, which Rust closures cannot borrow,
  so `execute_process_node_visitor` first collects the children in `VisitEachChild` order (single nodes via `visit`, lists
  via the `VisitNodes` hook, exactly Go's dispatch) and then processes them in that order; processing never changes the
  node's children. The `visiting*` fields are kept and saved/restored as in Go.
- `format/rules.go`: Go's `tokenRangeFromEx(allTokens, …)` appends into `allTokens`' one spare slot twice, so
  `anyTokenIncludingMultilineComments` ends with EndOfFile, not MultiLineCommentTrivia; reproduced (commented).
- `contextPredicate` closures (`isOptionEnabled(sel)` etc.) are leaked `&'static dyn Fn + Send + Sync` (the rules map is
  a process-wide `OnceLock`, like Go's `sync.OnceValue`); rule structs are leaked once.
- `dynamicIndenter` (shared, mutated through pointers in Go) is `Rc<DynamicIndenter>` with `Cell` fields; the
  FormattingContext tristate caches are `Cell`s because rule predicates take `&FormattingContext`.
- `formattingScanner` is owned by the worker (`Option<FormattingScanner>`); `newFormattingScanner` hands it to
  `execute` and resets it afterwards.
- `prepareRangeContainsErrorFunction` uses a stable sort by pos (Go `slices.SortStableFunc`), as Go.
- Exported/unexported pairs colliding after snake-casing: `getTokenAtPosition` -> `get_token_at_position_worker`,
  `detectNamedImportOrganizationBySort` -> `detect_named_import_organization_by_sort_worker`.
- Go `strings.Compare`/`cmp.Compare` -> a local `go_cmp`; `strings.ToLower` -> `go_strings_to_lower`.
- `norm_nfd_string` does not reproduce x/text's Stream-Safe Text Format (U+034F inserted after 30 consecutive
  non-starters); irrelevant for module specifiers / import names.
- `lsutil::get_quote_preference`: Go compares the string preference to `""`/`"auto"`/`"single"`; with the enum, an
  unrecognised configured string parses to `Unknown` (auto-detect) instead of Go's "any other string = double".

## Needs from others

- lead: `astnav` now lives in `tsrs_astnav` on `lsp`. Since d0f3d71 the only change to `astnav/tokens.rs` is origin-marker
  line numbers (f41c2f9: 13 markers corrected, no code change); `astnav/mod.rs` gained test-only re-exports
  (`parse_for_test`, `repo_root`) used by lsutil/format tests.
- lsproto: `FromLSFormatOptions` / `ToLSFormatOptions` (formatcodeoptions.go:94/:105); wiring
  `UserPreferences::marshal_json_to` / `unmarshal_json_from` into `tsrs_lsproto::Json`.
- whoever adds `tsrs_core::context::Context`: carry the format values (see Deviations).
- No checker functions were needed (no `NEEDS(api)` placeholders).

## Doubts

- `GetOrCreateToken` creates tokens on whichever thread asks; they live in that thread's arena (like every other LS
  allocation for now), the cache entry outlives nothing that matters until the phase-4 region work.
- Quote preference parsing (see Deviations).
- Sub-agent doubts: `RawSourceMap.version` accepts `3.0`; decoder counters are `i32`.


## Sub-agent notes (sourcemap, lsconv/linemap, lsutil formatcodeoptions/userpreferences)

### Ported

| Go (`ts-ref/tsc/internal/…`) | Rust (`crates/tsrs_ls/src/…`) |
| --- | --- |
| `ls/lsutil/formatcodeoptions.go` (minus `FromLSFormatOptions` / `ToLSFormatOptions`) | `lsutil/formatcodeoptions.rs` |
| `ls/lsutil/userpreferences.go` | `lsutil/userpreferences.rs` |
| `ls/lsutil/userpreferences_test.go` | `lsutil/userpreferences_test.rs` (declared from userpreferences.rs with `#[path]`) |
| `sourcemap/source.go`, `lineinfo.go`, `util.go`, `decoder.go`, `source_mapper.go` | `sourcemap/{source,lineinfo,util,decoder,source_mapper}.rs` |
| `sourcemap/generator.go` (+ `generator_test.go`) | `sourcemap/generator.rs` (+ `generator_test.rs`); needed for `RawSourceMap`, `SourceIndex`, `NameIndex` |
| `ls/lsconv/linemap.go` | `lsconv/linemap.rs` |

`generator.rs` is declared from `source_mapper.rs` (`#[path = "generator.rs"] mod generator; pub use generator::*;`)
because mod.rs is not mine. Moving it to `sourcemap/mod.rs` (`mod generator; pub use generator::*;`, and deleting the three
lines in source_mapper.rs) is the clean layout.

Tests: userpreferences 11/11 (the 9 Go tests incl. all ParseUnstable cases, plus a field-table coverage test and a
Deterministic-output test), generator 32/32 (all Go tests), source_mapper 7 (no Go tests exist: decoder, data URL parsing,
base64, document position mapping end to end), linemap 1, `tsrs_core::goslices` 3/3 (new `sort_func` test checked against
Go 1.27 output). While lsfound's format/ files were mid-edit the crate did not compile, so the 51 tsrs_ls tests were run in a
scratch crate that `#[path]`-includes only these files (`target/scratch/lsfound-sub/check`): 0 errors, 0 warnings, 51 pass.
userpreferences tests also passed in the real crate before format/ broke.

### Shared-file edits

- `crates/tsrs_core/src/goslices.rs`: added `sort_func` (Go `slices.SortFunc`, the pdqsort from `zsortanyfunc.go`, same
  order of equal elements) and its test. `source_mapper.go` sorts mappings with `slices.SortFunc` and then deduplicates and
  binary-searches, so the order of mappings with equal positions decides which one `GetSourcePosition` /
  `GetGeneratedPosition` return. Additive.

### Deviations

- userpreferences: Go reads the `raw` / `config` / `fallbackConfig` struct tags by reflection. Rust has a static table
  `USER_PREFERENCES_FIELD_TAGS`, one `FieldTag` per tagged field in Go declaration order with the nested
  `FormatCodeSettings` → `EditorSettings`, `InlayHints`, `CodeLens` structs flattened in place (Go's recursion into untagged
  struct fields), carrying the tag strings verbatim and `get` / `get_mut` accessors that return a `FieldRef` / `FieldMut`
  enum (one variant per Go field type the reflection code distinguishes). `collect_field_infos` still parses the tags
  (`,invert`, `;`-separated fallbacks) and panics on an untagged entry; `FieldInfo.field` replaces `fieldPath`, and
  `getFieldByPath` is gone. The `typeParsers` map is one `parse_*` function per entry (same order), dispatched by the
  `FieldMut` variant in `set_field_from_value`; `typeSerializers` is `type_serializers(&FieldRef) -> Option<Option<Value>>`
  (outer None = no entry, inner None = Go nil); `configPathParsers` is a `match` on the path. The test's
  `fillNonZeroValues` walks the table; the "every field has a tag" invariant became a test that destructures every struct
  exhaustively (compile error on a new field), checks the table length and that each entry addresses a distinct field.
- `MarshalJSONTo(enc)` -> `marshal_json_to(&self) -> Result<Value, String>` (the value the encoder would write, keys sorted
  recursively for `json.Deterministic(true)`, bytewise like Go's `slices.Sort`); `UnmarshalJSONFrom(dec)` ->
  `unmarshal_json_from(&mut self, &Value)` (null -> nil map; non-object -> error). Hook them into `tsrs_lsproto::Json` once
  that trait exists.
- `with_config(self, &OrderedMap)` consumes self (Go value receiver). Go `map[string]any` -> `OrderedMap<String, Value>`.
  JSON numbers are f64 only, so Go's `case int` / `case float64` branches are one `Value::Number` branch.
- Types: Go `int` settings fields -> `i32`. `IndentStyle` is a newtype `IndentStyle(pub i32)` with associated consts,
  because `parseIndentStyle` stores any number. `WorkspaceSymbolsScope` is a newtype over `Cow<'static, str>` with consts,
  because it has no parser and stores any string. `SemicolonPreference` gets a `None` variant for Go's zero value `""`.
  `OrganizeImportsCollation` (Go bool) is an enum `Ordinal`/`Unicode`. `[]string` fields are `Option<Vec<String>>` (Go
  serializes nil and empty differently). Embedded `EditorSettings` is field `editor_settings` + `Deref`/`DerefMut`
  (CHECKER.md embedding rule), so `settings.tab_size` works.
- sourcemap: `Mapping` is a Copy value (Go allocates it in the decoder's arena); `MappingsDecoder::next() -> Option<Mapping>`
  (None = done), `values()` is an iterator. `DocumentPositionMapper` methods take `&self` (Go accepts a nil receiver; callers
  holding `Option<DocumentPositionMapper>` take that path). `Host::get_ecma_line_info` returns `Option<Arc<ECMALineInfo>>`;
  `create_ecma_line_info` returns `Arc` (overlayfs caches it); `compute_lsp_line_starts` returns `Arc<LSPLineMap>` for the
  same reason. `RawSourceMap` JSON goes through `to_json` / `from_json` (Go's struct codec: field order, `omitzero` on
  `sourcesContent`, case-sensitive names, unknown names ignored, null -> zero). Go's `encoding/base64` StdEncoding is a small
  private encoder (generator.rs) / decoder (source_mapper.rs). Unexported `tryGetSourceMappingURL` ->
  `try_get_source_mapping_url_` (collides with util.go's exported one after snake-casing). `Generator::String()` ->
  `string(&mut self)` (it commits the pending mapping), no `Display`.
- Go's `tryParseBase64Url` slices `url[:6]` after `charset=` without a length check; the Rust port panics the same way.

### Needs from others

- `FromLSFormatOptions` / `ToLSFormatOptions` (formatcodeoptions.go:94, :105): need `tsrs_lsproto::FormattingOptions`.
- `UserPreferences` JSON: wire `marshal_json_to` / `unmarshal_json_from` into the `tsrs_lsproto::Json` trait when it lands.
- lsfound: optionally move the generator declaration into `sourcemap/mod.rs` (see above).

### Doubts

- `RawSourceMap.version`: Go v2 rejects a non-integer literal like `3.0` / `3e0` for an `int`; `Value::Number` is an f64,
  so the port accepts any integral number.
- Decoder accumulators are `i32` (Go `int`, 64-bit); results differ only past 2^31.
- `ECMALineInfo.text` is an owned `String` (Go shares the string); fine for the overlay cache, but a copy per call if a host
  builds it on the fly.
