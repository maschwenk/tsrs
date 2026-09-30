# fix-types-baseline: `.types` / `.symbols` baselines

## What was added

- `crates/tsrs_testrunner/src/type_symbol_baseline.rs`: port of `testutil/tsbaseline/type_symbol_baseline.go`
  (walk order `forEachASTNode`, node filter, `writeTypeOrSymbol`, the line-merging `iterateBaseline`, header,
  `removeTestPathPrefixes`, NoContent rule). Same program as the error baseline; the type walk runs before the symbol
  walk on the same checker (Go order), each under its own panic guard (Go's per-subtest `RecoverAndFail`).
- Harness (`compile.rs` `verify_types_and_symbols`): Go's `verifyTypesAndSymbols` — skipped for `@noTypesAndSymbols`,
  files = toBeCompiled + otherFiles present in the program, `hasErrorBaseline = len(diagnostics) > 0`.
- CLI: `tsrs-test run --baselines types,symbols` (or `--types --symbols`); `show <name> --types [--symbols] [--full]`.
  Classes per test in `summary.json` (`types`, `types_diff`, `symbols`, …), lists `types-{pass,fail,crash,…}.txt`,
  `symbols-*.txt`, artifacts `<suite>/<name>.{types,symbols}.{actual,diff}`. Errors-only runs are unchanged and keep
  the previous types/symbols results in `summary.json`.
- `tools/cluster-diffs.py --baseline types|symbols`.
- Checker: `services_subset.rs` -> `services.rs` (+ `SkipAlias`, `GetRootSymbols`, `GetMappedTypeSymbolOfProperty`,
  `getImmediateRootSymbols`, `tryGetTarget`, `GetExportSymbolOfSymbol`, `GetExportSpecifierLocalTargetSymbol`,
  `GetShorthandAssignmentValueSymbol`); the walker itself only needs `GetTypeAtLocation` / `GetSymbolAtLocation` /
  `SymbolToStringEx` / `NewNodeBuilder` / `IsTypeAny`, which were already ported. `pub use` of `new_node_builder`,
  `is_type_any` from `tsrs_checker`.
- `tsrs_ast`: `SemanticMeaning`, `get_meaning_from_declaration`, `is_label_name` (+ helpers).

## Results (runnable variants that produce a `.types`/`.symbols` baseline: 12,779)

| step | types | symbols |
| --- | --- | --- |
| first run | 12,776 | 12,779 |
| + printer: internal-name prefix escapes as `\uFFFD` | 12,778 | 12,779 |

The error-baseline suite was already matching on the non-error behavior: no checker divergence surfaced.

## Fixes

| cluster | tests | root cause | fix |
| --- | --- | --- | --- |
| `(typeof E)["\uFFFDmissing"]` printed as `"<DEL>missing"` | enumWithBigint, privateNameEnum | Go's internal symbol-name prefix is the invalid byte 0xFE; the port uses U+007F. Go's `escapeStringWorker` decodes 0xFE as a stray `RuneError` and escapes it; U+007F is left raw | `tsrs_printer::escape_string_worker` treats a *leading* U+007F followed by more text as that stray byte. A U+007F elsewhere (or alone, `"\177"` in octalLiteralAndEscapeSequence) is user text and stays raw, as in Go |

## Remaining

| test | why |
| --- | --- |
| compiler/declarationEmitObjectAssignedDefaultExport | Go reports a declaration-emit diagnostic (TS2883, not ported), so `hasErrorBaseline` is true and an `any` import prints `any`; we have no diagnostics and print the intrinsic name `error`. Resolves with declaration diagnostics (the test is in `fail.txt`). |

## Notes for others

- The Go harness runs JS emit (`verifyJavaScriptOutput`) between the error and the types baselines. Emit is not
  ported; on this corpus that made no observable difference.
- Representation limit: U+007F doubles as Go's 0xFE internal-name prefix. The checker already treats any name that
  starts with it as internal (Go `name[0] == '\xFE'`); only the printer's string escaper needed the mapping.
