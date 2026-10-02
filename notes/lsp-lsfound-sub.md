# lsp-lsfound sub-agent: lsutil options, sourcemap, lsconv/linemap

## Ported

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

## Shared-file edits

- `crates/tsrs_core/src/goslices.rs`: added `sort_func` (Go `slices.SortFunc`, the pdqsort from `zsortanyfunc.go`, same
  order of equal elements) and its test. `source_mapper.go` sorts mappings with `slices.SortFunc` and then deduplicates and
  binary-searches, so the order of mappings with equal positions decides which one `GetSourcePosition` /
  `GetGeneratedPosition` return. Additive.

## Deviations

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

## Needs from others

- `FromLSFormatOptions` / `ToLSFormatOptions` (formatcodeoptions.go:94, :105): need `tsrs_lsproto::FormattingOptions`.
- `UserPreferences` JSON: wire `marshal_json_to` / `unmarshal_json_from` into the `tsrs_lsproto::Json` trait when it lands.
- lsfound: optionally move the generator declaration into `sourcemap/mod.rs` (see above).

## Doubts

- `RawSourceMap.version`: Go v2 rejects a non-integer literal like `3.0` / `3e0` for an `int`; `Value::Number` is an f64,
  so the port accepts any integral number.
- Decoder accumulators are `i32` (Go `int`, 64-bit); results differ only past 2^31.
- `ECMALineInfo.text` is an owned `String` (Go shares the string); fine for the overlay cache, but a copy per call if a host
  builds it on the fly.
