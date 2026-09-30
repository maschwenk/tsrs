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

## 2026-09-30 (later): compiler-level fixes, node-builder skeleton merged

Conformance on main: **10,519 pass / 2,876 codes / 67 fail / 0 timeout / 0 crash** (suite wall time ~11 s).

Of the 67 fails: 59 are declaration diagnostics (TS2883/TS4xxx/TS9xxx… from the declaration-emit pipeline, not ported:
~7k Go lines in `transformers/declarations`, `emitresolver.go`, isolatedDeclarations); 6 are placeholder-printer effects;
2 (`mutuallyRecursiveInference`, `recursiveMappedTypes`) depend on the Go test harness running JS emit before collecting
diagnostics (the `tsgo` CLI itself reports what we report).

Project error injection (same 15 errors appended to 3 files in a clone): identical (file, line, col, code) sets and
exit code versus the reference. A mutation-testing campaign is in progress (`tools/mutate`, `notes/fix-project.md`).

In flight: node-builder body wave (branches `body/nb-1..6`) replacing the placeholder type printer.
