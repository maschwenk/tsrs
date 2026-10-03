# Emit plan: JavaScript, declarations, source maps, `--build`

Status (2026-10-03): **wave E1 (pipeline) landed on branch `emit/core`.** The `TSRS_EMIT=1` gate (section 6) is
implemented; without it tsrs still forces `--noEmit` and writes nothing (release guard: `crates/tsrs_cli/tests/emit_gate.rs`).
Under the gate, `Program::emit` runs the full Go pipeline: `.d.ts` files are printed (the declaration transformer is
complete), and every script transformer that is not ported yet is a gate stub that panics with
`emit: <file>.go not ported` once its `SubtreeFacts` early return does not apply. Progress: section 13.

## Why the work stopped after phase 0

A cold build of the 97 packages that emit with tsc in the private monorepo adds up to ~49 s of task time
(19 s wall). Emit would make tsrs complete, but it would not make that build meaningfully faster, so the usage
budget went to the LSP port instead. This document is written so the work can be resumed without
re-deriving anything: it lists what exists, what is missing, the order to build it in, the waves, the gates and
the harness.

Reference commit: the same as the rest of the port (`ts-ref/tsc` = microsoft/TypeScript b85298b6a81f, nightly
7.1.0-dev.20260929). Reference binary: `$TSRS_WORK/bin/tsgo-ref`.

## 1. Ground rules for whoever resumes this

- Same porting rules as the checker (PORTING.md, BODY_PORTING.md): one Rust file per Go file, functions in Go
  order, snake_case Go names, `// file.go:LINE` origin markers, Go quirks kept, no "improvements", never
  special-case a test. Signatures come from `tools/gosig` (section 9).
- **The default behavior does not change until the user signs off.** Released binaries are used as a type checker
  in a large private monorepo, and a release must never start writing files. Every emit path stays behind
  `TSRS_EMIT=1` (section 6). Do not remove the gate and do not tag releases from emit work.
- Land on `main` in small commits (rebase, gates, `git push origin HEAD:main`, never force-push). Emit is additive
  and gated, so partial waves can land as long as the gates hold:
  - conformance suite errors plus `--baselines types,symbols` byte-identical to the base binary, in the default
    mode and with `TSRS_LAZY_MEMBERS=0`, with single- and multi-threaded test programs (compare whole
    `target/test-results` trees);
  - `cargo check --workspace` with 0 errors and 0 warnings, also on the newest available rustc (CI uses
    rustc 1.99 with `-D warnings`; avoid APIs deprecated there, such as `Atomic::fetch_update`);
  - from the harness wave onwards, the emit pass counts (`--baselines js`, and later the source-map baselines)
    must not lose a pass. Record them in the progress table (section 12).
- The private monorepo is read-only and is never named in commits or notes. End-to-end emit tests use the
  open-source projects in `$TSRS_WORK/bench-cache/solutions` (xstate-main, webpack, vscode, mui-docs) and write
  output only under the worktree's `target/`.
- At most 2 sub-agents at a time (model opus), each in its own worktree
  (`git worktree add $TSRS_WORK/wt/emit-<wave> -b fix/emit-<wave> origin/main`), and they do not spawn helpers.
  Waves must be restartable: commit small steps and keep `notes/emit-<wave>.md` up to date.

## 2. Go's emit pipeline (call flow)

```
execute/tsc.go  performCompilation / performIncrementalCompilation (incremental/)  / build.Orchestrator (-b, build/)
  └ execute/tsc/emit.go  EmitFilesAndReportErrors
       ├ compiler.GetDiagnosticsOfAnyProgram          (already ported: tsrs_compiler)
       ├ Program.Emit(EmitOptions{WriteFile})         compiler/program.go:1875
       │   ├ HandleNoEmitOptions (noEmit / noEmitOnError)
       │   ├ getSourceFilesToEmit -> sourceFileMayBeEmitted        (ported: emitter.rs)
       │   └ per file, in parallel (work group): newEmitHost (acquires and locks the file's checker),
       │     outputpaths.GetOutputPathsFor, emitter.emit():
       │       ├ emitJSFile:  getScriptTransformers chain -> printer.NewPrinter -> printSourceFile
       │       │     metadata? -> typeEraser -> importElision? -> runtimeSyntax -> legacyDecorators?
       │       │     -> jsx? -> estransforms.GetESTransformer(target) -> useStrict -> module transformer
       │       │     -> constEnumInliner (unless isolatedModules)
       │       ├ emitDeclarationFile: DeclarationTransformer + SupplementalReferencesTransformer
       │       │     -> printer (OnlyPrintJSDocStyle, NoEmitHelpers, declaration map via MapSourcePosition)
       │       └ printSourceFile: sourcemap.Generator, //# sourceMappingURL, .map file, EmitBOM, writeText
       │   └ CombineEmitResults (input order)
       ├ SortAndDeduplicateDiagnostics, report, listFiles (TSFILE: lines for --listEmittedFiles)
       └ exit status from EmitSkipped + diagnostics
```

`GetESTransformer` (estransforms/definitions.go) picks a chain by target. Even `ESNext` runs
`esDecorator + classFields`; ES2021–ES2026 add `using`; every older target adds more, down to ES2015 and below,
which run every estransform. Each transformer returns early on `SubtreeFacts`, so files without the syntax pass
through, but the transformers still have to exist and run.

## 3. Inventory: Go files, line counts, tsrs status

Line counts are `wc -l` of the non-test Go files. "Funcs" is the number of Go functions and methods; "in tsrs"
counts functions whose snake_case name is defined in the Rust crate that should hold them (for the script
transformers, the few name matches are accidental, so the honest figure is 0).

### printer/ (11,744 lines), Rust crate `tsrs_printer` (10.9k Rust lines)

| Go file | lines | funcs | tsrs status |
| --- | ---: | ---: | --- |
| printer.go | 6341 | 378 | **ported** (printer_1/2/3.rs; printer oracle 16480/16481 identical in 7 modes). Missing: source-map paths (`SourceMapGenerator` is an uninhabited enum, printer_1.rs:54; `emit_pos` and friends hit `unreachable!` at printer_3.rs:1150/1158), `lineCharacterCache`, `PrintHandlers.MapSourcePosition`. The emit-helper printing (`emit_helpers`, printer_2.rs:2162) is ported but never exercised. |
| factory.go | 1315 | 91 | **20 of 91**: generated names, `NewStringLiteralFromNode`, a few expression helpers. Missing (71): `NewThis/True/False/CommaExpression`, logical/strict-inequality helpers, `InlineExpressions`, `CreateExpressionFromEntityName`, `RestoreEnclosingLabel`, `CreateForOfBindingStatement`, method/global/`Function.call`/`Array.slice` calls, `RestoreOuterExpressions`, `EnsureUseStrict`, `SplitStandardPrologue`/`SplitCustomPrologue`, `GetLocalName[Ex]`/`GetExportName[Ex]`/`GetDeclarationName[Ex]`/`GetNamespaceMemberName`/`GetExternalModuleOrNamespaceExportName`, every `New*Helper` constructor (decorate, metadata, param, disposable resources, class private field get/set/in, assign, rest, await, async generator/delegator/values, awaiter, ES decorate + context objects, run initializers, template object, prop key, set function name, import default/star, export star, rewrite relative import extensions), `NewObjectDefinePropertyCall`, `NewReflectGet/SetCall`, `NewFunctionBindCall`, `NewImmediatelyInvokedArrowFunction`, `NewExportDefault`, `NewExternalModuleExport`, `NewAssignmentTargetWrapper`. |
| helpers.go | 558 | 1 | **type only**: `EmitHelper`, `Priority`, `compareEmitHelpers`. Missing: the ~26 helper definitions (`__decorate`, `__metadata`, `__param`, `__addDisposableResource`, `__disposeResources`, `__classPrivateField{Get,Set,In}`, `__await`, `__asyncGenerator`, `__asyncDelegator`, `__asyncValues`, `__rest`, `__awaiter`, async-super helpers, `__esDecorate`, `__runInitializers`, `__makeTemplateObject`, `__propKey`, `__setFunctionName`, `__createBinding`, `__setModuleDefault`, `__importStar`, `__importDefault`, `__exportStar`, `__rewriteRelativeImportExtension`). These are data (name, import name, priority, text); copy the text byte for byte. |
| emitcontext.go | 1067 | 87 | **86 of 87** (side tables, environments, visitor hooks; ported for the declaration transformer). |
| namegenerator.go | 405 | 25 | ported |
| textwriter.go, singlelinestringwriter.go, semicolon_writer.go, emittextwriter.go | 565 | 88 | ported |
| utilities.go | 935 | 51 | 49 of 51 |
| generatedidentifierflags.go, emitflags.go | 95 | 9 | ported |
| emitresolver.go | 127 | (interface) | Rust `tsrs_declarations::Resolver` (32 methods) over `tsrs_checker::EmitResolver`; the interface has 37 methods, so add the 5 missing wrappers when a transformer needs them |
| emithost.go | 23 | (interface) | missing; becomes a trait implemented by the compiler's emit host |
| syntheticfile.go | 53 | 2 | missing (used by synthesized-file emit) |
| changetrackerwriter.go | 250 | 36 | language service only; not part of emit |

### transformers/ (24,418 lines), Rust: `tsrs_declarations` today, `tsrs_transformers` planned

| Go file | lines | tsrs status |
| --- | ---: | --- |
| transformer.go | 41 | ported, in `tsrs_declarations/src/transformers.rs` (`Transformer` base) |
| utilities.go | 375 | 3 of 22 ported (`IsSimpleCopiableExpression`, `IsOriginalNodeSingleLine`, `IsSimpleInlineableExpression`) |
| chain.go | 62 | missing |
| modifiervisitor.go | 28 | missing |
| destructuring.go | 510 | missing |
| declarations/transform.go | 3002 | **ported completely** (transform_1/2/3.rs), but only run for diagnostics: no `.d.ts` is printed |
| declarations/diagnostics.go, tracker.go, util.go, supplementalreferences.go | 1234 | **ported** |
| tstransforms/typeeraser.go | 393 | missing |
| tstransforms/importelision.go | 153 | missing |
| tstransforms/runtimesyntax.go | 995 | missing (enums, namespaces, parameter properties) |
| tstransforms/utilities.go | 29 | missing |
| tstransforms/legacydecorators.go | 1038 | missing |
| tstransforms/metadata.go | 390 | missing |
| tstransforms/typeserializer.go | 511 | missing |
| moduletransforms/commonjsmodule.go | 2162 | missing |
| moduletransforms/esmodule.go | 372 | missing |
| moduletransforms/externalmoduleinfo.go | 390 | missing |
| moduletransforms/impliedmodule.go | 53 | missing |
| moduletransforms/utilities.go | 118 | missing |
| estransforms/classfields.go | 3618 | missing |
| estransforms/esdecorator.go | 2751 | missing |
| estransforms/using.go | 799 | missing |
| estransforms/namedevaluation.go | 535 | missing |
| estransforms/utilities.go | 289 | missing |
| estransforms/classthis.go, definitions.go, usestrict.go | 122 | missing |
| estransforms/async.go | 984 | missing |
| estransforms/forawait.go | 856 | missing |
| estransforms/objectrestspread.go | 593 | missing |
| estransforms/optionalchain.go | 240 | missing |
| estransforms/taggedtemplate.go | 175 | missing |
| estransforms/logicalassignment.go, exponentiation.go, nullishcoalescing.go, optionalcatch.go | 289 | missing |
| jsxtransforms/jsx.go | 1209 | missing |
| inliners/constenum.go | 102 | missing |

Ported so far: 4,277 of 24,418 lines (the declaration transformer and the base). Left: ~20,100 lines.

### sourcemap/ (1,021 lines), planned crate `tsrs_sourcemap`

| Go file | lines | tsrs status |
| --- | ---: | --- |
| generator.go | 387 | **ported** (`tsrs_sourcemap`, emit/sourcemaps; moved from `tsrs_ls::sourcemap`, which re-exports the crate) |
| decoder.go | 253 | **ported** |
| source_mapper.go | 313 | **ported** (shared with the language service) |
| lineinfo.go, util.go, source.go | 68 | **ported** (`impl Source for SourceFile` lives in `tsrs_sourcemap`, orphan rule) |

### outputpaths/ (314 lines)

| Go file | lines | tsrs status |
| --- | ---: | --- |
| outputpaths.go, commonsourcedirectory.go | 314 | **ported** in `tsrs_tsoptions::outputpaths` (all 22 functions: `GetOutputPathsFor`, `ForEachEmittedFile`, JS/declaration/source-map/build-info paths). `tsrs_compiler/src/outputpaths.rs` duplicates the common-source-directory subset; consolidate onto `tsrs_tsoptions::outputpaths` during wave E1. |

### compiler/ (emit parts)

| Go | lines | tsrs status |
| --- | ---: | --- |
| emitter.go | 576 | partly: `sourceFileMayBeEmitted` (without the content-mapper line; content mappers are not supported), `getSourceFilesToEmit`, `getDeclarationDiagnostics` (emitter.rs). Missing: the `emitter` type, `emit`, `emitJSFile`, `emitDeclarationFile`, `getScriptTransformers`, `getModuleTransformer`, `runScriptTransformers`, `runDeclarationTransformers`, `printSourceFile`, `writeText`, `shouldEmitSourceMaps`, `getSourceRoot`, `getSourceMapDirectory`, `getSourceMappingURL`, `declarationMapSource`. |
| emitHost.go | 138 | partly: the declaration-emit surface (emithost.rs: output paths, module-specifier host, `DeclarationEmitHost`). Missing: `Options`, `SourceFiles`, `IsEmitBlocked`, `WriteFile`, `GetEmitModuleFormatOfFile`, the `printer.EmitHost` impl. |
| program.go | | `IsEmitBlocked`, `blockEmittingOfFile`, `GetEmitModuleFormatOfFile`, `CommonSourceDirectory`, `getSourceFilesToEmit`, `GetDiagnosticsOfAnyProgram` exist. Missing: `Emit` (program.go:1875), `EmitOptions`/`EmitResult`/`SourceMapEmitResult`, `CombineEmitResults`, `HandleNoEmitOptions`. |

### checker/ and binder/ (what the transformers call)

- `checker/emitresolver.go` (1,323 lines): **all 63 functions ported** (`tsrs_checker/src/emitresolver.rs`).
- `binder/referenceresolver.go`: ported (`tsrs_binder::ReferenceResolver`).

### execute/ (10,375 lines), Rust crate `tsrs_cli` (type-check subset)

| Go | lines | tsrs status |
| --- | ---: | --- |
| tsc/emit.go | 162 | partly: `EmitFilesAndReportErrors` without the `Program.Emit` call; `listFiles` without `--listEmittedFiles` |
| tsc.go | 408 | partly: no `-b` (returns "not supported"), no `performIncrementalCompilation`, `--noEmit` forced |
| incremental/ (snapshot, buildInfo, affected files, emit files handler, program, …) | 3,535 | missing |
| build/ (orchestrator, buildtask, uptodatestatus, host, parseCache, compilerHost) | 2,312 | missing |
| tsctests/ (harness: runner.go, sys.go, fs.go, readablebuildinfo.go) | 1,355 | missing |
| watcher.go, watchmanager/ | 1,198 | out of scope (watch) |

### Test harness (Go `testutil`)

| Go | lines | role |
| --- | ---: | --- |
| tsbaseline/js_emit_baseline.go | 291 | `.js` baseline: inputs, JS outputs, `.d.ts` outputs, `DtsFileErrors`, noCheck re-emit comparison |
| tsbaseline/sourcemap_baseline.go | 130 | `.js.map` baseline with the source-map-visualization link |
| tsbaseline/sourcemap_record_baseline.go | 34 | `.sourcemap.txt` baseline |
| harnessutil/sourcemap_recorder.go | 364 | span recorder behind `.sourcemap.txt` |
| harnessutil/recorderfs.go | 45 | output recorder FS |
| harnessutil/harnessutil.go | 1,273 | `compileFilesWithHost` (pre/post-emit programs), `CompilationResult` (output ordering, `Repeat`), `GetSourceMapRecord` |

## 4. Baselines available as gates

`ts-ref/tsc/testdata/baselines/reference/`:

| baseline | compiler | conformance | total |
| --- | ---: | ---: | ---: |
| `.js` (JS + `.d.ts` per test variant) | 6,176 | 6,018 | **12,194** |
| `.js.map` | 131 | 20 | 151 |
| `.sourcemap.txt` | 138 | 20 | 158 |
| `transpile/` (`.js`, `.d.ts`; transpile_runner.go) | | | 41 files |
| `tsc/` (tsc_test.go: incremental, emit, declaration emit, …) | | | 224 files |
| `tsbuild/` (tscbuild_test.go: `--build`) | | | 192 files |
| `tscWatch/`, `tsbuildWatch/` | | | 107 files (watch, out of scope) |

What the `.js` baselines exercise (from the test variants; approximate):

| option | variants |
| --- | ---: |
| `target: es2015` / `es6` | **10,533 (86%)** |
| `target: esnext` | 636 |
| `target: es2022` | 541 |
| no target (LatestStandard) | 207 |
| other targets (es2016–es2021, es2023+) | 277 |
| no `module` (ES2015 modules for an es2015 target) | 9,709 |
| `module: commonjs` | 1,598 |
| `module: node16/18/20/nodenext` | 435 |
| `module: esnext` / `es2015` / `es2020` / `es2022` / `preserve` | 439 |
| `declaration` or `composite` | 1,738 |
| `jsx` | 418 |
| `experimentalDecorators` | 246 (`emitDecoratorMetadata` 77) |
| `sourceMap` / `inlineSourceMap` | 149 |

**This changes the phase order the original brief proposed.** "Modern targets first, downleveling later" helps only
~1,400 baselines, because 86% of them run the whole ES2016 chain (every estransform). Section 7 deals with that by
landing every transformer as a gate stub first: the `SubtreeFacts` early return is ported and the rest of the body is
`unimplemented!`. That way, every baseline that does not need an unported transformer can pass from the first waves on.

Go unit tests to port alongside (Rust `#[test]`s, cheap regression checks): `printer/printer_test.go` (2,624 lines;
the TestEmit table is already ported), `printer/namegenerator_test.go` (640),
`transformers/tstransforms/typeeraser_test.go` (106), `importelision_test.go` (274), `sourcemap/generator_test.go`
(403), `compiler/emit_test.go` (191), `outputpaths/outputpaths_test.go` (29); `testutil/emittestutil`.

## 5. Target crate map

| Go | Rust | notes |
| --- | --- | --- |
| transformers (transformer.go, chain.go, utilities.go, modifiervisitor.go, destructuring.go) | `tsrs_transformers` (crate root) | new crate. Move `Transformer` and the 3 utilities from `tsrs_declarations/src/transformers.rs`, and `Resolver` from `tsrs_declarations/src/resolver.rs`, into it; `tsrs_declarations` depends on it and re-exports, so existing paths keep compiling (Go: `declarations` imports `transformers`). |
| transformers/{tstransforms, moduletransforms, estransforms, jsxtransforms, inliners} | modules `tsrs_transformers::{tstransforms, …}` | one module dir per Go package, one file per Go file; classfields.go split into `classfields_1.rs`/`_2.rs`, esdecorator.go likewise |
| transformers/declarations | `tsrs_declarations` (unchanged) | |
| sourcemap | `tsrs_sourcemap` | new crate, depends only on `tsrs_core` |
| printer (factory.go rest, helpers.go, syntheticfile.go, emithost.go, source-map paths) | `tsrs_printer` | gosig stubs go to `factory_2.rs`, `helpers_defs.rs`; gosig skips the functions that `factory.rs` already defines |
| compiler emitter.go, emitHost.go, Program.Emit | `tsrs_compiler` (emitter.rs, emithost.rs, new `program_emit.rs`) | keep emit code out of program.rs and checkerpool.rs (the `lsp` branch rewrites both; section 11) |
| execute/tsc emit parts | `tsrs_cli` | |
| execute/incremental | `tsrs_incremental` | new crate |
| execute/build | `tsrs_build` | new crate |
| testutil tsbaseline/harnessutil emit parts, tsctests | `tsrs_testrunner` | |

## 6. The `TSRS_EMIT=1` gate (implemented: tsrs_cli `execute::emit_enabled`, `tsc/emit.rs`)

The rule: **without `TSRS_EMIT=1`, the bytes tsrs prints and the files it writes (none) are unchanged.**

- Read once in `tsrs_cli` (`std::sync::OnceLock<bool>`, `std::env::var("TSRS_EMIT").as_deref() == Ok("1")`).
  Every other value (unset, `0`, `true`, empty) means off. The library crates (`tsrs_compiler::Program::emit`,
  transformers, printer) do not read the variable. They are always compiled and are callable by the test harness,
  and only the CLI decides whether to call them.
- Off (the default, exactly today's behavior): `command_line` keeps appending `--noEmit` (execute.rs:76), `-b` keeps
  returning "not supported", incremental projects are checked from scratch and no `.tsbuildinfo` is read or written,
  `emit_files_and_report_errors` does not call `Program::emit`, and `--help` is unchanged.
- On: the forced `--noEmit` is skipped, so the project's own `noEmit`/`emitDeclarationOnly`/`noEmitOnError` decide,
  as in tsc. `emit_files_and_report_errors` follows Go's `EmitFilesAndReportErrors` (emit unless `listFilesOnly`,
  add the emit diagnostics, `TSFILE:` lines for `--listEmittedFiles`, status from `EmitSkipped`). Once phase 4 has
  landed, `-b` and incremental compilation are enabled too, but still only under the gate.
- Release guard: add a regression test (a `tsrs_cli` integration test or `testdata/regressions/emit-gate`) that runs
  the binary on a small project without `noEmit`, with `TSRS_EMIT` unset, and checks that no file was created.
  Also check `--version` and `--help` output byte for byte. Document the variable in README.md only when the user
  signs off on enabling emit.

## 7. Dependency order and waves

Dependency order (each step needs only the ones above it):

1. Printer completion for emit: factory.go rest, helpers.go definitions, syntheticfile.go, `EmitHost` trait.
2. `tsrs_transformers` base (Transformer, chain, utilities, modifiervisitor, destructuring) and the move of
   `Transformer`/`Resolver` out of `tsrs_declarations`.
3. Compiler pipeline: emitter.go, emitHost.go, `Program::emit`, `CombineEmitResults`, `HandleNoEmitOptions`.
   Output: the transformer chain of `getScriptTransformers`, with every not-yet-ported transformer present as a
   **gate stub** (constructor and `visit` with the Go `SubtreeFacts` early return ported faithfully, then
   `unimplemented!("emit: <file>.go not ported")`). Under that, `.d.ts` printing is nearly free, because the
   declaration transformer is complete.
4. Harness (`--baselines js`) and the emit oracle, so every later wave is measured.
5. Transformers in order of how many baselines they unblock: tstransforms core and inliner, then module transforms,
   then the estransforms, jsx and legacy decorators.
6. sourcemap crate, printer source-map paths, declaration maps, then the source-map baselines.
7. incremental, build, then the tsctests harness.

Wave split. Each wave is sized for one agent; "Go lines" is the code to port, and Rust usually comes out at
1.1–1.3× (the declaration transformer: 4,236 Go lines became 4,667 Rust lines).

| wave | phase | contents | Go lines | depends on |
| --- | --- | --- | ---: | --- |
| **E1 pipeline** | 1 | printer factory.go rest (71 funcs) + helpers.go + syntheticfile.go + emithost.go; `tsrs_transformers` base (chain, utilities rest, modifiervisitor; move Transformer/Resolver); compiler emitter.go rest + emitHost.go rest + `Program::emit`/`CombineEmitResults`/`HandleNoEmitOptions` (per checker group on the checker threads, like `get_declaration_diagnostics`); CLI `TSRS_EMIT` gate + `--listEmittedFiles`; gate stubs for every transformer; `.d.ts` printing | ~3,300 | — |
| **E2 harness + oracle** | 1 | section 8 (`--baselines js`, pre/post-emit programs, output ordering, DtsFileErrors, noCheck repeat) + `tools/oracle/emit` + `docs/EMIT.md` tables | ~800 + tooling | E1 (can start in parallel on the Go side) |
| **E3 tstransforms core** | 1 | typeeraser, importelision, runtimesyntax, tstransforms/utilities, inliners/constenum, estransforms/usestrict | 1,722 | E1 |
| **E4 module transforms** | 1 | commonjsmodule, esmodule, externalmoduleinfo, impliedmodule, moduletransforms/utilities, destructuring.go | 3,605 | E1 |
| **E5 class fields** | 1 | classfields.go (3,618; split at ~1,800 across two sessions if needed), namedevaluation, classthis, estransforms/utilities | 4,470 | E3 |
| **E6 ES decorators + using** | 1 | esdecorator.go, using.go | 3,550 | E5 (namedevaluation, utilities) |
| **E7 source maps** | 2 | `tsrs_sourcemap` (generator, decoder, lineinfo, util, source; source_mapper for LS reuse), printer source-map paths (`SourceMapGenerator` -> `tsrs_sourcemap::Generator`, `emitPos`/`setSourceMapSource`/`lineCharacterCache`, `MapSourcePosition`), emitter glue (`getSourceMappingURL`, `getSourceMapDirectory`, inline maps, `sourceRoot`/`mapRoot`, `declarationMap`), harness `.js.map` + `.sourcemap.txt` (sourcemap_recorder.go) | ~1,900 | E1, E2 |
| **E8 ES2016–ES2020 small transforms** | 3 | exponentiation, logicalassignment, nullishcoalescing, optionalchain, optionalcatch, taggedtemplate, objectrestspread | 1,297 | E1 |
| **E9 async** | 3 | async.go, forawait.go | 1,840 | E1 |
| **E10 JSX** | 3 | jsxtransforms/jsx.go (also needed by the xstate-main oracle: it compiles `.tsx` with `react-jsx`) | 1,209 | E1 |
| **E11 legacy decorators** | 3 | legacydecorators, metadata, typeserializer | 1,939 | E3 |
| **E12 option sweep** | 3 | driven by failing baselines: `noEmitOnError`, `emitDeclarationOnly`, `outDir`/`rootDir`/`declarationDir` rules, `removeComments`, `preserveConstEnums`, `importHelpers`/`noEmitHelpers`, `isolatedModules`/`verbatimModuleSyntax`, `emitBOM`, `newLine`. These options are already handled inside the Go code ported by earlier waves, so this is a fix wave, not a port wave | fixes | E1–E11 |
| **E13 incremental** | 4 | execute/incremental (snapshot, buildInfo read/write, affected files, emit files handler, program, referencemap) + `performIncrementalCompilation` in tsc.go | 3,535 | E1, E7 |
| **E14 build** | 4 | execute/build (orchestrator, buildtask, uptodatestatus, host, parseCache, compilerHost) + tsc.go `-b` entry | 2,312 | E13 |
| **E15 tsctests harness** | 4 | runner.go, sys.go, fs.go, readablebuildinfo.go; the scenario tables in tsc_test.go (5,146) and tscbuild_test.go (4,790) are data, so extract them mechanically; gate = `tsc/` and `tsbuild/` baselines (watch scenarios skipped) | ~1,400 + tables | E13, E14 |

Total: ~20,100 transformer lines + ~1,900 printer + 1,021 sourcemap + ~700 compiler + ~5,900 execute + ~2,200 harness,
so ~32k Go lines, roughly 35–40k Rust lines. With 2 agents at a time, the order is: E1 alone, then E2 ∥ E3, E4 ∥ E5,
E6 ∥ E8, E9 ∥ E10, E11 ∥ E7, E12, then E13 → E14 ∥ E15.

Phase gates:

- Phase 1 (E1–E6): `--baselines js` pass count, recorded per wave; the emit oracle on a modern-target project.
  xstate-main needs E10 (jsx) as well, so either pull E10 into phase 1 or use a no-JSX subset of it first.
- Phase 2 (E7): `.js.map` and `.sourcemap.txt` pass counts; the oracle with `--sourceMap --declaration
  --declarationMap`.
- Phase 3 (E8–E12): `.js` pass count close to the checker's pass rate. Exceptions: Go's own `skippedEmitTests`, and
  tests that fail their error baseline.
- Phase 4 (E13–E15): `tsc/` and `tsbuild/` baselines; the oracle on a multi-project repo (`-b`, then a no-op rebuild,
  a touched file, and a deleted output).

## 8. Baseline harness plan (`tsrs-test --baselines js`)

Implemented on `emit/core` for `.js` (`crates/tsrs_testrunner/src/emit_harness.rs`: OutputRecorderFS,
compileFilesWithHost's pre-/post-emit programs, newCompilationResult's output ordering, DoJSEmitBaseline with
DtsFileErrors and the noCheck repeat; `compile.rs`: `verify_javascript_output`, `SKIPPED_EMIT_TESTS`, the
`Recompile` hook for `CompileFilesEx` re-runs). `tsrs-test show <name> --js` shows the first differing hunk.
Deviations: `baseline.DiffText` (Go's patience diff, used only by the 2 baselines with a `!!!! File ... differs ...
in noCheck emit` section) uses the `similar` crate's patience algorithm and unified format; a stub panic on a
checker thread with multi-threaded test programs is reported as `<unknown panic>` (the panic hook's message is
thread-local). `.js.map`/`.sourcemap.txt` are E7.

Port Go's flow exactly (testrunner/compiler_runner.go, testutil/harnessutil, testutil/tsbaseline):

- New kinds in `--baselines`: `js` (`.js`), `jsmap` (`.js.map`), `sourcemap` (`.sourcemap.txt`), next to
  `types,symbols`. They use more bits in `EXTRA_BASELINES` and are passed to workers like the existing ones. Results
  go to `js-<class>.txt` etc., artifacts to `<suite>/<name>.js.{actual,diff}`, add `tsrs-test show <name> --js`, and
  `tools/cluster-diffs.py --baseline js`.
- Compilation when any emit baseline is requested: Go's `compileFilesWithHost`. A pre-emit program
  (`TraceResolution` off) collects config/program/syntactic/semantic/global(/suggestion)/declaration diagnostics. A
  post-emit program runs `Emit` first, writing into the harness file system's output recorder (recorderfs.go),
  then collects the same diagnostics. If the counts differ, the ad-hoc "Pre-emit (N) and post-emit (M) diagnostic
  counts do not match!" diagnostic is added. The error baseline uses the post-emit diagnostics, as Go's does. Under
  `--baselines js` this should fix the 4 known emit-order harness artifacts (STATUS.md:
  `mutuallyRecursiveInference`, `recursiveMappedTypes`, and the 2 codes tests). **The default mode (no emit
  baselines) keeps today's single type-check program**, so the existing gates stay byte-identical.
- `newCompilationResult`: classify outputs into JS (`.js`/`.json` family) / DTS / maps. Order them by the program's
  source files (JS, then DTS, then map per input), with the leftovers sorted by unit name. `getOutputPath` handles
  `outDir`/`declarationDir`/common source directory.
- `DoJSEmitBaseline`: run only when the test has non-`.d.ts` files. Skip Go's `skippedEmitTests` (8 names in
  compiler_runner.go). Then: the header `//// [<path>] ////`, the inputs (`otherFiles` then `toBeCompiled`, joined
  with `\r\n`), the JS outputs (a `.json` output that fails to parse gets an error baseline instead, when the test has
  no errors), `\r\n\r\n` + the DTS outputs, and the "Expected at least one js file" fatal check.
  `prepareDeclarationCompilationContext` + `compileDeclarationFiles` re-compile the emitted `.d.ts` files with the
  `//// [DtsFileErrors]` section (including its panics, which classify as fail). The `noCheck` repeat
  (`result.Repeat({"noCheck": "true"})`) re-compiles with `noCheck` and adds the `!!!! File … differs from
  original emit in noCheck emit` sections with `baseline.DiffText`. Finally, `NoContent` when empty, and the
  baseline path with `.js`.
- `DoSourcemapBaseline`: inline-map and map-count checks, `fileOutput` per map, and `createSourceMapPreviewLink`
  (base64 of the JS, the map and the sources after Go's `url.QueryEscape`+`QueryUnescape` round trip; port it
  literally).
- `DoSourcemapRecordBaseline`: `GetSourceMapRecord` over `EmitResult.SourceMaps` with `sourcemap.DecodeMappings`
  and sourcemap_recorder.go.
- Multi-threaded programs (`TS_TEST_PROGRAM_SINGLE_THREADED=false`) must give identical emit baselines. Go emits
  files in parallel, and its harness sorts outputs by input order.
- Incremental test variants: Go's `createProgram` wraps the program in `incremental.NewProgram` when
  `incremental` is set. Until E13, run them as plain programs and list them as a known gap (only `.tsbuildinfo`
  content differs; it is not a baseline here).

## 9. Signatures (`tools/gosig`)

No signatures are generated yet. When resuming, write one config per Go package, modeled on
`tools/gosig/declarations.json`, and generate stubs in the wave's worktree at the start of the wave, so they match the
then-current main:

- `transformers.json` (package `transformers`; outDir `crates/tsrs_transformers/src`), `tstransforms.json`,
  `moduletransforms.json`, `estransforms.json` (classfields/esdecorator chunked by line range), `jsxtransforms.json`,
  `inliners.json`, `sourcemap.json`, `printer_emit.json` (factory.go -> `factory_2.rs`, helpers.go ->
  `helpers_defs.rs`; gosig skips the hand-written functions already in `tsrs_printer`), and later `incremental.json`,
  `build.json`.
- Common settings: `resultOnlyPackages` = ast, core, printer, checker, binder, transformers (and declarations for the
  emitter); `arenaTypes` = every transformer struct (`typeEraserTransformer`, `RuntimeSyntaxTransformer`,
  `CommonJSModuleTransformer`, `classFieldsTransformer`, `flattener`, `chainedTransformer`, …: they are captured by
  visitor closures like the declaration transformer); `typeMap` additions: `*transformers.TransformOptions` ->
  `&TransformOptions`, `*transformers.Transformer` -> `P<Transformer>`, `transformers.TransformerFactory` ->
  `TransformerFactory` (`fn(&TransformOptions) -> Option<P<Transformer>>`), `printer.EmitResolver` -> `Resolver`,
  `binder.ReferenceResolver` -> `ReferenceResolverRef` (the emit resolver or a plain `ReferenceResolver`; Go
  chooses one in `getScriptTransformers`), `*printer.EmitHelper` -> `P<EmitHelper>`, `*sourcemap.Generator` ->
  `&mut Generator`, plus everything in declarations.json's map. Then run `tools/sigs-from-rust.py` so that
  `docs/sigs/*.txt` is generated from the sources.

## 10. Design notes for E1 (decided and implemented on `emit/core`)

- **Crates.** `tsrs_transformers` (new) holds transformer.go, chain.go, modifiervisitor.go, utilities.go (the 3
  functions the declaration transformer uses so far), `Resolver` (moved from `tsrs_declarations`, plus wrappers for the
  rest of `printer.EmitResolver`: `is_referenced_alias_declaration`, `mark_linked_references_recursively`,
  `get_constant_value`, `get_type_reference_serialization_kind`, the JSX factory entities,
  `set_referenced_import_declaration` and the embedded `binder.ReferenceResolver` methods), `ReferenceResolverRef`
  (Go's `binder.ReferenceResolver` interface value: `Emit(Resolver)` or `Plain(P<ReferenceResolver<()>>)`, chosen in
  `getScriptTransformers`), `TransformOptions` and the `EmitHost` trait. Go keeps `EmitHost` in package `printer`, but
  its `GetEmitResolver` returns the `Resolver`, which needs the checker, so the trait lives in `tsrs_transformers`
  (`tsrs_printer` does not depend on the checker). `tsrs_declarations` depends on it and re-exports `Transformer`,
  `Resolver` and the 3 utilities, so existing paths keep compiling.
- **Transformer handles.** Go returns `*transformers.Transformer`, whose visitor closes over the concrete
  transformer. In Rust, `P<Transformer>` points at the `base: Transformer` field of an arena-allocated concrete
  transformer: `tx.get().base.new_transformer(visit, Some(ctx))` returns `P::from_static(&tx.base)`, and the `visit`
  closure captures the `P<XTransformer>` (`&self` methods, `Cell`/`RefCell` fields), exactly as
  `DeclarationTransformer` does. Factories are `fn(&TransformOptions) -> Option<P<Transformer>>`
  (`TransformerFactory`; `None` is Go's nil from `newClassFieldsTransformer` and friends). Go's package-level
  `transformers.Chain(...)` vars are functions that call `chain(&[...])(opts)` (estransforms/definitions.rs).
- **Gate stubs.** One file per Go file. Each stub has the Go struct fields its constructor sets, the constructor
  (including Go's nil returns and panics), and `visit` with the Go `SubtreeFacts` early return; past it,
  `unimplemented!("emit: <file>.go not ported")`. Transformers whose Go `visit` has no early return (importelision,
  esmodule, constenum) stop at their first statement. Ported in full because they are tiny or pure dispatch:
  estransforms/definitions.go, usestrict.go, moduletransforms/impliedmodule.go and esdecorator.go's
  `visitSourceFile` (it visits every file for every target below ESNext, so a stub there would block everything).
  async/forawait's early return goes through `fallbackVisitor`, whose own early return always holds while nothing
  is captured.
- **Checker access.** Go's `newEmitHost` locks the file's checker for the whole emit of that file. Rust lends the
  checker to a `CheckerSlot` for the whole `emitter.emit()` (transform + print, JS + declarations), and `Resolver`
  borrows it per call, the mechanism `get_declaration_diagnostics` already uses.
- **Threading (where emit runs).** `Program::emit` (`tsrs_compiler/src/program_emit.rs`) runs one emit pass per
  checker group on the checker threads (`for_each_checker_group_do` over the files to emit, files in program order
  within a group), like `get_declaration_diagnostics`. Each file's `EmitResult` goes into a slot indexed by its
  position in the emit list, and `combine_emit_results` runs in input order, which is Go's observable order. With an
  external checker pool (the project system), files take their checker from the pool one by one, as in Go. A fresh
  text writer is created per file (Go pools them; the output is the same); emit contexts come from
  `get_emit_context()`. `EmitOptions.write_file` is a `&(dyn Fn(&str, &str, &mut WriteFileData) -> Result<(),
  String> + Sync)`, since it is called from the checker threads.
- **Arena and regions.** Transformed and synthesized nodes, emit contexts and transformer objects are allocated with
  `P::new` in the arena of the thread that runs the emit (a checker thread in the CLI and the harness), exactly like
  the nodes the checker's node builder synthesizes. Nothing is freed and no recycling site is added: emit uses no
  free list, no checkpoint and no `arena_pin`, so the census free-gate (notes/mem-recycle.md) has nothing new to
  verify; the precise walk does not visit emit output because nothing in the program points to it after emit. The
  language server (notes/lsp-mem.md regions) does not emit; if it ever does, emit would run inside the checker's
  region and its nodes would be freed with that region, which is correct because emit output is not stored in the
  program.
- **Writing files.** Go's `emitHost.WriteFile` is `program.Host().FS().WriteFile`. Rust: the compiler's
  `EmitHost::write_file` calls `program.host().fs().write_file` (`tsrs_vfs`): the CLI's cached FS over the OS FS,
  which creates missing directories (iovfs `write_file_ensuring_dir`); the harness's recorder FS over the
  in-memory test FS. `Program::emit` is only called by the CLI under `TSRS_EMIT=1` and by the harness under
  `--baselines js`.
- **Source maps** (E7). `print_source_file` keeps Go's structure; when `shouldEmitSourceMaps` is true it is
  `unimplemented!` (`TODO(emit/sourcemaps)`): replace the uninhabited `SourceMapGenerator` with
  `tsrs_sourcemap::Generator`, the two `unreachable!`s in printer_3.rs with the Go bodies, and add the
  `sourceMapGenerator != nil` branch of `printSourceFile` (emitter.rs). `get_source_mapping_url`,
  `get_source_map_directory` and `get_source_root` are ported already.
- **Helpers.** helpers.go definitions are `static`s in `tsrs_printer/src/helpers_defs.rs` (text copied byte for
  byte), handed out with `P::from_static` (now a `const fn`), so identity comparisons behave like Go's pointers.
  The rest of factory.go is `tsrs_printer/src/factory_2.rs` (`PrivateIdentifierKind` is an enum with `as_str`).
- **Content mappers** are not supported by tsrs (the harness skips `runExternalCode`). Go's branch structure is
  kept where emit code checks `ContentMapper()`/`SpanMap()`, with the mapper always absent.
- **Checker assignment.** With several checkers, tsrs assigns files to checkers by directory locality (Go: FENNEL),
  and inferred types printed into `.d.ts` files can depend on which files a checker saw first (2 of the private
  monorepo's 2,325 declaration files differed in property order). The monorepo oracle therefore runs tsrs with
  `TSRS_CHECKER_ASSIGNMENT=go`; with it every declaration file is identical.
- **Incremental programs** (E13) are emitted as plain programs: under the gate, `incremental`/`composite` projects
  write their JS and declarations but no `.tsbuildinfo`, and the harness builds plain programs too.

## 11. Coordination with the LSP port

- The `lsp` branch (crates `tsrs_ls*`, `tsrs_project`, `tsrs_lsp`) rewrites large parts of
  `tsrs_compiler/src/program.rs` (+773 lines) and `checkerpool.rs` (+234), and touches `emithost.rs`. Put
  `Program::emit` in a new file (`program_emit.rs`) and add only a small API to the checker pool, to keep merges
  cheap. Check whether `lsp` has added a general "run with this file's checker" helper by then, and use it.
- `lsp` plans `tsrs_ls::sourcemap` for the parts of Go's `sourcemap` that the language service uses
  (`source_mapper.go`, decoder). Port the whole Go package once in `tsrs_sourcemap` and let `tsrs_ls` depend on
  it, or adopt theirs if it has landed by then. Do not keep two ports.
- `printer/changetrackerwriter.go` is language-service code. Leave it to the LSP effort.

## 12. Oracle: `tools/oracle/emit`

Implemented: `tools/oracle/emit/run.py` (one project, as planned below; `$TSGO` names the reference binary) and
`tools/oracle/emit/monorepo.sh <monorepo root> [-j N] [--filter RE] [-- flags]` (every workspace package whose
`build` script runs `tsc`; outputs in `/tmp/emit-go/<pkg>` and `/tmp/emit-rs/<pkg>`; refuses to start unless the
monorepo's `git status --short` is empty and fails if it changed; `TSRS_CHECKER_ASSIGNMENT=go` by default, see
section 10). `tsc --build` packages are emitted with `-p` until `-b` exists.

The reference binary already emits, so no Go oracle program is needed (add one under `ts-ref/tsc/cmd/` only if
`EmitResult` internals are needed). Planned `tools/oracle/emit/run.py <tsconfig|dir> [--name N] [-- extra tsc flags]`:

- runs `tsgo-ref -p <cfg> --noEmit false --incremental false --outDir <W>/ref [--declarationDir <W>/ref-dts]
  --tsBuildInfoFile <W>/ref.tsbuildinfo <extra>` and `TSRS_EMIT=1 tsrs -p <cfg>` with the same flags into
  `<W>/rs`, where `<W> = target/scratch/emit-oracle/<name>`. Every output path is overridden, so nothing is ever
  written into the project;
- compares the stdout/stderr diagnostics and exit codes, then walks both trees and compares every file byte for
  byte. Reports missing / extra / different files, with the first differing line and a unified-diff excerpt, and a
  summary line `files: N identical, D different, M missing, X extra`; nonzero exit on any difference;
- `--sourceMap --declaration --declarationMap` modes for phase 2; `-b` mode (phase 4) builds a copy of the project
  under `<W>` because `-b` writes next to the sources.

Notes on the bench projects (`$TSRS_WORK/bench-cache/solutions`): xstate-main (`target: esnext`, `module: nodenext`,
`jsx: react-jsx`, `noEmit: true`, `allowImportingTsExtensions`: pass `--noEmit false`. TS5096 is then reported on both
sides and emit still happens); webpack (JS only: `allowJs`, `target: ES2017`, commonjs, which exercises the ES2018
chain on `.js` input); vscode `src` (`ES2024`, nodenext, `preserveConstEnums`, `outDir`; large, a good stress test
for E1–E6); mui-docs (JSX).

## 13. Progress tables

`.js` = `tsrs-test run --suite all --baselines js` (js pass / js total, crash = gate stub reached). Oracle =
`tools/oracle/emit/monorepo.sh <private monorepo>` (103 packages whose `build` runs `tsc`); "dts" mode adds
`--sourceMap false --declarationMap false --emitDeclarationOnly`. Gates (conformance errors + types/symbols
identical to main in both modes, fourslash 4066/63, `-D warnings` check) held for every row.

| date | commit | wave | `.js` pass / total | `.js.map` | `.sourcemap.txt` | oracle | notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-03 | emit/core (E2) | E1+E2 | 1364 / 15197 (12032 crash at stubs, 1800 skip) | — | — | dts: 2325/2325 files identical, 103/103 packages; full: 0 files (stubs: typeeraser 91, importelision 5, metadata 4; 3 packages emit nothing in both) | multi-threaded test programs give the same js pass list (one timeout aside) |
| 2026-10-03 | emit/sourcemaps | E7 (part 1) | — | — | — | printer-level: 38/38 `.js.map` identical where the untransformed JS is identical | `tsrs_sourcemap` crate (32/32 Go generator tests), printer source-map paths, emitter glue, harness recorder (not wired: waits for emit/core). Gates: conformance + types/symbols identical to main (default and `TSRS_LAZY_MEMBERS=0`: 13457/12778/12778), fourslash 4066/63 same pass list |

## 14. Known gaps and risks

- Go's own harness skips 8 emit tests (`skippedEmitTests`) as nondeterministic; mirror the list.
- Go emits in parallel. Anything order-dependent inside a file (generated names, helper order) is per-file in Go
  too, so per-file determinism is enough. Cross-file state such as `IsEmitBlocked` is computed before emit.
- Temp-name generation (`namegenerator.go`) is ported and validated only through the printer oracle's synthesized
  modes. The transformers will be its first real users.
- The const-enum inliner type-checks property accesses during emit (that is what causes the 4 harness artifacts).
  Running emit from the CLI under `TSRS_EMIT=1` may therefore change which node an error is first reported from,
  exactly as in tsc. The default mode is not affected.
- `tsrs_compiler/src/outputpaths.rs` and `tsrs_tsoptions::outputpaths` overlap; consolidate in E1.
- Incremental programs in the harness and in the CLI are plain programs until E13.
