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
