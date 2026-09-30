# Status

## 2026-09-30: first end-to-end runs

All checker bodies are merged and the workspace compiles. Type/symbol/signature printing (`printer.rs`) still has
**temporary placeholder bodies** (`type#<id>`, bare symbol names) until the node-builder port lands, so message text
that mentions types does not match yet.

Conformance (`tsrs-test run --suite all`, 15,197 variants, 1,735 skipped as unsupported by the harness, ~22 s):

| class | count | meaning |
| --- | --- | --- |
| pass | 10,479 | baseline byte-identical |
| codes | 2,871 | same (file, line, col, code) set; text differs (placeholder type names) |
| fail | 111 | different diagnostics (mostly declaration-emit TS4xxx/TS2883/TS9xxx and TS5055, not ported) |
| timeout | 1 | `compiler/intersectionConstructorReductionCrash` |
| crash | 0 | |

Project (`tsrs -p apps/project`, release build, single checker thread, placeholder printer), against
`tsgo-ref --singleThreaded` at the same commit:

| | tsrs | tsgo (single-threaded) |
| --- | --- | --- |
| errors | 0 | 0 |
| files | 37,942 | 37,942 |
| symbols | 25,966,437 | 25,973,354 |
| types | 9,637,422 | 9,639,962 |
| instantiations | 44,879,286 | 44,884,281 |
| check time | 27.1 s | 33.1 s |
| total wall | 31.1 s | 41.0 s |
| peak RSS | 20.5 GB | 18.0 GB |

Counter differences (<0.03%) are expected while the node builder is a placeholder (Go creates a few types while
printing speculative error messages).
