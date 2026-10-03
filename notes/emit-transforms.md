# emit/transforms (E3 + E4) checkpoint

Branch `emit/transforms`, merged with main d3a2598 (emit/core).

Ported (crates/tsrs_transformers): tstransforms/{typeeraser,importelision,runtimesyntax,utilities}.rs,
inliners/constenum.rs, destructuring.rs, moduletransforms/{esmodule,externalmoduleinfo,utilities}.rs, ast_ext.rs
(Go ast helpers missing from tsrs_ast). usestrict.rs and impliedmodule.rs are E1's (already complete on main).

commonjsmodule.go is split: commonjsmodule.rs (go 1–559), commonjsmodule_2.rs (560–985),
commonjsmodule_3.rs (986–1240: variable statements, top-level nested statements), commonjsmodule_4.rs (1241–end).
Write these in chunks of <=150 lines per tool call.

Next: finish commonjsmodule, build, run gates (js baselines, conformance types/symbols both lazy modes,
fourslash, -D warnings check, emit_gate test), monorepo oracle, draft PR (label coder-task-generated).

## Status (2026-10-03)
All E3/E4 files ported. Gates: conformance errors+types/symbols identical to main d3a2598 (default and
TSRS_LAZY_MEMBERS=0), fourslash 4066/63 same pass list, `-D warnings` workspace check clean, emit_gate passes.
js baselines 8579/15197 pass, 0 fail (main: 1365). Oracle (js mode): 85/103 packages fully identical, 0 different files.

Decisions: `single_or_many` keeps Go's nil-vs-empty distinction (visitTopLevelVariableStatement returns nil when no
statements were produced, so an `if`/label body becomes `{ }`/`;` like Go). Harness source-file cache key now includes
`external_module_indicator_options.force` (Go keys on the whole SourceFileParseOptions).
Shared-file edits: tsrs_testrunner/src/compile.rs (cache key), docs/EMIT.md.
