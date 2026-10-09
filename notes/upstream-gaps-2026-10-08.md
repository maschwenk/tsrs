# Upstream TypeScript since the pin, and the remaining tsc gaps (2026-10-08)

Two questions: what has microsoft/TypeScript changed in the Go compiler since tsrs's pin, and what does tsrs still not
port against the pinned version. Report only; no code changes. tsrs at `f250e637` (origin/main on 2026-10-08).

## Where the pin stands

| | commit | date (UTC) | version |
| --- | --- | --- | --- |
| tsrs pin (`Cargo.toml [workspace.metadata.typescript]`) | `b85298b6a81f` (#64543) | 2026-09-29 23:01 | labelled 7.1.0-dev.20260929; the code equals nightly `7.1.0-dev.20260930.4` (`9adc871ff4`, one CI-only commit later) |
| upstream main | `fed0bf2414` (#64686) | 2026-10-08 05:40 | `typescript@next` = `7.1.0-dev.20261008.1` (its `gitHead` is `fed0bf2414`); `core.version` still `"7.1.0-dev"`; latest stable `typescript@7.0.2` |

The pin is **57 commits and 8.3 days behind**. 49 of the 57 touch `tsc/internal` or `tsc/testdata`; the other 8 are
CI, the VS Code extension, a dependency bump and the removal of legacy localization handbacks. None of the 49 was
ported as such (no tsrs commit names any of their PRs; each Rust counterpart was read against the Go diff). Four are
already covered by tsrs's own code: #64469 (tsrs has its own re-export index), #64624 (tsrs never had the bug),
#64637 (fixed independently in tsrs `f1f3f7f7`) and #64621 (tsrs was already deterministic; the new test passes).
#64633's crash does not reproduce (tsrs already skips the missing container).

## Measured: tsrs main against upstream main's testdata

The tsrs binaries were built from `f250e637` and run twice: once with `ts-ref` pointing at a checkout of the pin and
once at a checkout of upstream main (`fed0bf2414`), so the second run shows exactly what a pin bump without any
porting would break. Commands are at the end.

Conformance (`tsrs-test run --suite all --baselines types,symbols,js`):

| | variants | errors pass / codes / fail / crash | `.types` pass / fail / crash | `.symbols` pass / fail / crash | `.js` pass / fail / crash |
| --- | ---: | --- | --- | --- | --- |
| pin testdata | 15,197 | 13,462 / 0 / 0 / 0 | 12,779 / 0 / 0 | 12,779 / 0 / 0 | 13,392 / 0 / 0 |
| upstream main testdata | 15,291 | 13,467 / 7 / 73 / 9 | 12,801 / 56 / 9 | 12,820 / 37 / 9 | 13,411 / 61 / 9 |

(One more test, `intersectionConstructorReductionCrash`, timed out at the default 20 s on a loaded machine and passes
alone with `--timeout 120`; it is not counted above as a failure.) With `--baselines jsmap,sourcemap`, `.js.map` goes
from 149 / 0 to **3 pass / 146 fail** and `.sourcemap.txt` from 156 / 0 to 151 / 5: upstream stopped writing
`"sourceRoot":""` into source maps (#64544, below).

95 distinct tests fail at least one baseline. Every one maps to an upstream commit that added or changed that test:

| commit | tests failing | what |
| --- | ---: | --- |
| #63915 source phase imports | 22 (the 21 `importSource*` tests + `importMetaPropertyInvalidInCall` codes) | new syntax, new lib, new diagnostics |
| #64674 computed property names always checked | 15 | checker: diagnostics previously suppressed by a parent grammar error |
| #64461 cyclic structures and truncation in declaration emit | 13 | TS5088 / TS7056 where tsrs writes `any` |
| #64573 `export=` class visibility | 11 | false TS4094; `.types` / `.symbols` alias text |
| #64544 typed path prep | 6 (+146 `.js.map`) | `sourceRoot` dropped from source maps; one `.types` |
| #64651 malformed destructuring emit | 4 (crash) | tsrs `assert!` panics |
| #64636 flaky `typeof import()` qualifier diagnostic | 3 | |
| #64159 strongly typed paths | 2 | TS6059 text, TS2834 for `import "./foo/"` |
| #64604 DOM types | 2 (`.types`) | lib text |
| #64556 cycles in array / tuple serialization | 2 | |
| one each | 15 | #64525, #64530, #64566, #64482, #64646, #64584, #64243, #64640, #64093, #64558, #64545, #64578, #64676, #64574 (crash), #64670 (crash) |

New tests that tsrs already passes at upstream main include #64469's `declarationEmitAlternativeContainingModules`,
#64633's `decoratorMetadataObjectLiteralMethodNoCrash`, #64159's `fileNamesWithEmptyAndDotStems` and
`typeReferenceDirectiveTrailingSlashName`, #64621's stable `getInferTypeParameters` test, #64632's node_modules depth
test and the changed `conditionalTypes1` (#64553).

Fourslash (`tsrs-fourslash run`, the generated tests stay the pin's; only baseline files move): pin 4,066 pass / 63
fail / 417 skip; upstream baselines **4,044 / 85 / 417**. The 22 new failures: 19 are #64159's harness change (state
baselines print `CaseSensitivity:` instead of `UseCaseSensitiveFileNames:`, and `documentHighlights02` searches
rooted file names), 3 are #64160 (top-level imports leave document symbols). Regenerating the fourslash tests from
upstream would add more (new tests, and three format tests whose expectations changed with #64597), and would not
compile until the `UserPreferences` restructure of #64554 is ported (see the language-service table).

## Part 1: upstream commits since the pin, by area

"tsrs" says whether the Rust counterpart has the change: "no" means it still matches the pre-change Go. "Tests" are
the measured failures above, by baseline kind, unless marked "predicted". Sizes are Rust lines to write or change.
"Real projects" is whether output changes on ordinary code (diagnostics, emitted `.js` / `.d.ts` / maps), as opposed
to invalid input, flakiness tsrs does not have, or API surface.

### New syntax, compiler options, libs, program construction and source maps

| commit | change | tsrs | port | real projects | tests |
| --- | --- | --- | --- | --- | --- |
| `2f9fd09a10` #63915 source phase imports | `import source x from "m"` and `import.source("m")`. New contextual keyword `SourceKeyword` (after `DeferKeyword`, so every later `Kind` value and the API's `SyntaxKind` numbers shift by one); scanner keyword; AST helpers (`IsSourcePhaseImport`, `IsImportSourceMetaProperty`, ...); checker global `AbstractModuleSource`, `import.source()` returns `Promise<AbstractModuleSource>`; source-phase specifiers are never resolved (file loader, program, package-name collection); `SupportsSourcePhaseImports` (esnext, nodenext, preserve); diagnostics TS18111-TS18115, TS18061 reworded ("'meta', 'defer', or 'source'"); new lib `lib.esnext.modulesource.d.ts` and `lib` value `esnext.modulesource`; printer emits the `.` of a meta property as a token; organize imports, source definition and auto-import handle the phase | no: `parser_1.rs:2785` handles `defer` only, so `import source x from "m"` gets TS1005 (it goes down the import-equals path); no `SourceKeyword` (`kind.rs`), no scanner entry | medium: ~300 hand-written lines (parser ~60, checker ~85, AST ~35, LS ~35, compiler ~25, the rest small) plus re-running `gen-flags.ts`, `gen-ast.ts`, `gen-diagnostics/gen.py`, `gen-libs/gen.py`, `gen-codec.ts` and `gen-fourslash`. The keyword perfect hash has a collision assert and may need a new multiplier; `source` is a very common identifier and now scans as a keyword token, so any tsrs shortcut that tests for `Identifier` directly needs checking | rare: opt-in syntax (WebAssembly ESM integration) that tsrs rejects and tsgo accepts; ESNext libs gain a global `AbstractModuleSource`; API kind numbers shift | 22 tests: 31 errors + 1 codes (TS18061 text), 24 js, 31 types, 29 symbols (variants); 6 new fourslash tests; lib lists in help / showConfig / tsoptions baselines |
| `bea2e849e9` #64640 deferred imports need a namespace binding | `import defer "m"` keeps an empty `defer` import clause instead of dropping `defer`; new TS18117 | no (`parser_1.rs:2905`, `grammarchecks.rs:2275`); needs #63915's parser branch | ~10 lines + diagnostics regen | no (invalid code tsrs accepts) | 1 test (2 variants): errors, js |
| `e8fd69fc1e` #64457 generated option definitions, JSON schema | Go's hand-written option tables become generated (`options_generated.go`, `declarations_generated.go`); field by field no compiler, build or type-acquisition option was added, removed or renamed, and no description, category or default changed. Real changes: **the 7 watch options (`watchFile`, `watchDirectory`, `fallbackPolling`, `synchronousWatchDirectory`, `watchInterval`, `excludeDirectories`, `excludeFiles`) are removed** and now give TS5023 / TS5072 on the command line (`core.WatchOptions` is deleted); `help-all` loses its WATCH OPTIONS section; the test harness's vary-by set grows by 19 options; the API enum table adds `ScriptTarget` | no (`declswatch.rs`, `commandlineparser.rs:172`, `tsrs_testrunner/src/options.rs:71`) | small: delete ~150 lines of watch-option code, a 5-line vary-by rule; `tools/gen-tsoptions/gen_decls.py` reads Go files that no longer exist (not in `gen-check.sh`) | no for check and emit; tsgo now rejects the 7 flags, tsrs accepts them | 17 `tsoptions` baselines deleted and 7 `removedWatchOptions` added (tsrs's `commandlineparser_test.rs` would compare against missing files); 4 new `tsc/showConfig` scenarios |
| `bf00f213ff` #64093 `Promise.allKeyed` / `allSettledKeyed` | new `lib.esnext.promise.d.ts`; the existing `lib` value `esnext.promise` is remapped to it (it pointed at the es2025 file) | no (`enummaps.rs:128`) | `gen-libs` re-run + 1 line | rare: ESNext code using the methods gets TS2339 from tsrs only | 1 test: errors, types, symbols |
| `de61e69621` #64604 DOM types | `lib.dom.d.ts`, `lib.webworker.d.ts`: `HTMLImageElement.crossOrigin` narrows to `"anonymous" \| "use-credentials" \| null`; `TransformStreamDefaultController.enqueue` and `WritableStreamDefaultWriter.write` require their argument; `SVGFilterElement` loses `href`; `CSSFontFaceRule.style` becomes `CSSFontFaceDescriptors`; new globals (`Origin`, `Serial`, `SerialPort`, `CloseWatcher`, `DocumentPictureInPicture`, `GPUBufferUsage` and other WebGPU namespaces); new APIs (`Navigator.serial`, `Element.setHTML`, `getContext("webgpu")`, ...) | no (libs are the pin's) | `gen-libs` re-run | **yes, for DOM code**: tsgo reports new errors (`img.crossOrigin = "Anonymous"`, argument-less `enqueue()`) that tsrs does not, and accepts new APIs that tsrs rejects; new globals can clash with same-named user globals | 2 types |
| `a4b1410230` #64536 `Array.at` doc text | JSDoc only | no | `gen-libs` re-run | hover text | 0 |
| `59f5b0233f` #64544 typed path prep bugfixes | CLI-visible: (a) `RawSourceMap.SourceRoot` gets `omitzero`, so **no emitted `.js.map` / `.d.ts.map` / inline map contains `"sourceRoot":""` any more**; (b) `/// <reference path>` targets are made absolute before the redirect lookup, so tsbuildinfo records a referenced composite project's `.d.ts` instead of its `.ts`; (c) `tryGetModuleNameAsNodeModule` really tests nested `package.json` directories (`foo/subpkg` instead of `foo/subpkg/main` in printed types, `.d.ts` and auto-imports); (d) realpaths keep the original casing on case-insensitive file systems. Language-service-only: dynamic (non-file URI) path encoding (`tspath/dynamic.go`, 317 lines) and a rewritten source mapper (`sourceRoot`, null and duplicate sources) | no: (a) `tsrs_sourcemap/src/generator.rs:68`, (b) `tsrs_incremental/src/programtosnapshot.rs:352`, (c) `tsrs_modulespecifiers/src/specifiers.rs:781`, (d) `tsrs_compiler/src/projectreferencedtsfakinghost.rs:182` | CLI part small: (a) 1 line, (b) ~5, (c) ~6, (d) ~30. LS part 800-1,000 lines | **(a) every project with `sourceMap` or `declarationMap`**: map files differ byte for byte (consumers behave the same); (b)-(d) rare | 146 `.js.map`, 5 `.sourcemap.txt`, 5 js (inline maps), 1 types; 31 `tsbuild`, 3 `tsc`, 3 `transpile`, 16 fourslash state baselines |
| `ca197b7b85` #64632 restart imports when a file's node_modules depth drops | `filesparser` restarts a file's sub-tasks when it is reached again at a lower depth | no (`tsrs_compiler/src/filesparser.rs:574`) | 2 lines | rare: a workspace package reached first through `node_modules` and later relatively keeps "external library" status (not emitted; JS beyond `maxNodeModuleJsDepth` dropped); depends on file order | 0 (new test passes) |
| `09d4966578` #64637 project-reference diagnostics without a config file | `CreateDiagnosticAtReferenceSyntax` returns nil without a config file | **has it** (`tsconfigparsing.rs:2005`, landed in tsrs `f1f3f7f7`) | none | none | 0 |
| `cb72b2e17b` #64608, `6b3da828d7` #64620 localization data | refreshed `loc/*.json.gz`; legacy handbacks deleted | n/a (`--locale` not ported) | none | none | 0 |
| `64c5fa0b92` #64522, `91521cf229` #64675 | test input fix; Go test-runner filtering | n/a | none | none | 0 (`customConditions` passes) |

### Checker semantics and diagnostics

| commit | change | tsrs | port | real projects | tests |
| --- | --- | --- | --- | --- | --- |
| `237b14a5d9` #64566 merged declaration diagnostic ownership | TS2320 / TS2430 for merged interfaces are reported once, at the program's first declaration (so nothing when that declaration is in a lib or `.d.ts` skipped by `skipLibCheck` / `skipDefaultLibCheck`); TS2473 compares const-ness with the first enum declaration; TS2395 / TS2652 run once per merged symbol | **different fix**: tsrs #165 reports at the first declaration in each declaring file (`checker_03.rs:970 is_interface_check_site`); Go mode keeps the old link. Upstream now reports fewer errors than either | ~20 lines in `checker_03.rs` / `checker_04.rs`, plus replacing tsrs's per-file default rule, `testdata/regressions/merged-interface-check-site` (upstream gives 1 TS2320 where tsrs expects 4), its CLI test and notes/fix-history-dependent-diagnostics.md; small to medium | **yes**: conflicting augmentations of lib or `node_modules` interfaces under `skipLibCheck` stop being reported by tsgo (nuxt's `ImportMeta` TS2320 lines are the likely bench case; not run) | 1 errors (`mergedInterfaceDefaultLibraryDiagnostics`) |
| `619d485a63` #64525 no contextual type from a class's own static property | `getContextualTypeForStaticPropertyDeclaration` ignores a contextual type whose symbol is the class expression itself | no (`checker_14.rs:1970`) | 2 lines | yes, rare: removes a spurious TS7022 and an `any` for `id(class { static foo = id(42) })` (mixins, HOCs) | 1 errors, 1 types |
| `c4d731aae1` #64530 return-type inference filtered by its constraint in recursive calls | `getInferredType` applies the constraint to a pure return-type inference even under `NoConstraintChecks` | no (`inference.rs:1590`) | 1 condition | rare: recursive getter plus a type parameter inferred only from the return type | 1 errors, 1 types |
| `c8e9b7d259` #64553 cache inferences from type arguments | alias / reference type-argument inference goes through `invokeOnce`, and `InferenceKey` gains priority, contravariant and bivariant | no (`inference_types.rs:6` key is `{s, t}`; `infermemo.rs` replays whole top-level walks and is off in Go mode, so it does not cover the within-walk blowup) | ~30 lines in `inference.rs` / `inference_types.rs` | perf: removes exponential check time in chains of generic calls (#64378: 20 calls took 126 s); semantics, rare: the old key skipped a contravariant revisit of the same pair, so new tsgo can infer `never` in correlated-union code | 0 (upstream changed the test input) |
| `ec47d33c23` #64674 computed property names always checked | `checkEnumMember` checks a computed name's expression; the `markLinkedReferences` early return for enum computed names is removed | no (`checker_03.rs:1086`, `checker_14.rs:516`) | ~9 lines | no: the code already has TS1164; this adds follow-on errors | 15 tests: 13 errors, 1 codes, 1 js (`importedValueInEnumComputedNameNoFlake` keeps its import) |
| `a1ef42b9ea` #64646 flaky TS7008 on JS constructor properties | new `checkConstructorDeclaredProperties` resolves every constructor-only property of JS classes | no (`checker_02.rs:2485`, `checker_05.rs:1869`) | ~16 lines | rare: `checkJs` + `noImplicitAny`, `this.x = null` never read | 2 errors |
| `1a78786ca7` #64636 flaky diagnostic from emit on `typeof import()` qualifiers | `markLinkedReferences` stops at a `typeof import("x").a.b` qualifier part | no (`checker_14.rs:564`) | 1 line | rare: a same-named local import is kept in the JS | 2 codes, 1 js |
| `81d3f3f5e8` #64482 flaky diagnostic through `MarkLinkedReferencesRecursively` | `ast.IsInExpressionContext` is false for a function / class expression's own name | no (`tsrs_ast/src/utilities_2.rs`) | 2 lines | rare: shadowed expression name, diagnostic only after emit | 1 codes, 1 types |
| `50d70a3f5f` #64621 stable `getInferTypeParameters` order | infer type parameters sorted with `CompareTypes` (Go iterated a map) | **already deterministic**: tsrs iterates the insertion-ordered symbol table (`checker_12.rs:401`); the order is declaration order, not sorted | 1-3 lines if byte parity with new tsgo matters | no (union member order in printed types) | 0 |
| `fc636a691f` #64584 (checker part) `await using` hint | a failed Disposable check on an AsyncDisposable initializer gets TS18116 "Did you mean to use 'await using'?" | no; TS18116 is not in `tsrs_diagnostics` | ~12 lines + regenerate `generated.rs` | message text only | 1 codes |
| `792ffccb90` #64243 no backtick type in import attribute types | `checkGrammarImportAttributesType` requires a real string literal (TS1555) | no (`grammarchecks.rs:2402`) | 1 line | no | 1 errors |
| `d61a7d2359` #64638 [api] no disk-layout import diagnostics for custom resolutions | new `ResolvedModule.IsCustomResolution`; `resolveExternalModule` skips the `.ts`-extension and rewrite diagnostics for it | no (`tsrs_module/src/types.rs:95`, `checker_08.rs:268`) | ~3 lines | Node API users with custom resolvers only | 0 (Go unit test) |
| `afd02f1486` #64617 `GetTypeAnnotationNode` handles `JSDocParameterTag` | one more kind in the list | no (`tsrs_ast/src/utilities_3.rs:1598`) | 1 line | no (only caller is completions) | 0 |

### Declaration emit and the node builder

| commit | change | tsrs | port | real projects | tests |
| --- | --- | --- | --- | --- | --- |
| `834e7862c0` #64461 report cyclic structures and truncation | `createCyclicStructurePlaceholder` reports TS5088 where the node builder used to write `/*elided*/ any`, and depth > 10 sets `truncating` (TS7056 under `NoTruncation`) | no (`nodebuilderimpl_2.rs:1177`, `:1187`, `:1214`, `:1541`) | ~15 lines | **yes** (`declaration: true`): self-referential inferred types (`return this` in object literals, self-referencing class expressions, functions returning themselves, JS classes in CommonJS, very deep generics) now fail `.d.ts` emit with TS5088 / TS7056; tsrs writes `any` and emits | 13 tests: 11 errors, 11 js, 3 types |
| `edf7da4e93` #64558 preserve reverse mapped types | in declaration emit, reverse-mapped types are serialized instead of becoming placeholders; depth bound of 100 per mapped type | no (`nodebuilderimpl_2.rs:596`, `:951`); needs #64461 | ~30 lines | yes, rare: silent `any` in published `.d.ts` for `unwrap(Record<string, {value: T}>)`-style helpers | 1 errors, 1 js |
| `15dee00a1c` #64556 cycles in array and tuple serialization | arrays / tuples go through `visitAndTransformType`; every type is checked against `visitedTypes`; `CompositeTypeCacheIdentity` gains `inferTypeParameters` | no (`nodebuilder_types.rs:53`, `nodebuilderimpl_2.rs:1502`); needs #64461 | 60-80 lines, mostly moved | rare: recursive local array / tuple types get TS5088; hover prints `children: any` instead of `any[]` | 2 tests (errors, js, 2 types); a fourslash hover test after regeneration |
| `af36d532b0` #64573 `export=` class visibility with `export type` | `isAccessible` follows chained aliases (`resolvedAliasSymbol`) before giving up | no (`printer.rs:1262 is_accessible`) | ~12 lines | **yes**: false TS4094 "Property ... of exported anonymous class type may not be private" that fails `declaration` builds (#64249, a TypeScript 6 to 7 regression reported against `zod-route-schemas`); hover / `.types` text for `export =` aliases | 11 tests: 6 errors, 12 js, 13 types, 7 symbols (variants) |
| `697b847e11` #64545 export modifiers on expando members | the alias branch of `transformExpandoAssignment` also exports earlier expando members | no (`tsrs_declarations/src/transform_3.rs:490`) | 15-20 lines | yes, rare: `Menu.displayName = ...; Menu.Item = MenuItem;` leaves `displayName` unexported in the `.d.ts` | 1 js |
| `a501b11f64` #64469 index re-exporting modules | `getAlternativeContainingModules` uses a one-time index instead of scanning every file | **has an equivalent**: `printer.rs:834 modules_exporting` (tsrs's own lazy index) | none | no | 0 (new test passes) |
| `253bcd8623` #64649 one node builder per emit resolver | the emit resolver owns its emit context and one lazily created node builder; JS and `.d.ts` of a file share the context | no: `emitresolver.rs` builds a node builder per call (`// TODO: cache per-context` at 7 sites); tsrs already memoizes the expensive module-name lookup program-wide (notes/perf-emit.md) | 200-400 lines of signature churn across `emitresolver.rs`, `tsrs_declarations`, `emitter.rs`, `tsrs_ls`; must fit the per-file emit regions | no (perf only) | 0 |

### JavaScript emit

| commit | change | tsrs | port | real projects | tests |
| --- | --- | --- | --- | --- | --- |
| `0bd74f77b2` #64676 CommonJS export assignments for names shadowed in function-local loops | loop bodies below the top level are visited with the normal visitor | no (`moduletransforms/commonjsmodule_4.rs:12`, `:24`) | 2 lines | **yes, silent runtime miscompile**: with CommonJS output and `export { mount }`, a `const mount` inside a loop in any function emits `exports.mount = mount`, overwriting the export when the function runs | 1 js |
| `09b1db0617` #64578 elide empty named imports next to a default import | under `verbatimModuleSyntax`, `import A, { type T }` becomes `import A from` | no (`tstransforms/typeeraser.rs:352`) | ~6 lines | yes, common but cosmetic: tsrs emits `import A, {} from`, same behaviour, different bytes | 1 js |
| `02c1c9a6ab` #64574 `with` statement crash | an elided body becomes an empty statement | no; **tsrs panics** (`commonjsmodule_3.rs:274` `.unwrap()`) | 1 line | no (already an error) | 1 crash |
| `54f05ae22a` #64651 malformed destructuring assignments | three `debug.Assert`s accept any element | no; **tsrs panics** (`esdecorator_2.rs:813`, `:863`, `classfields_2.rs:1457`) | 3 lines | no (syntax errors, fuzz input) | 4 crash (7 variants) |
| `0ba5d992db` #64670 duplicate private names in decorated classes | `getPrivateIdentifier` instead of `accessPrivateIdentifier`; untransformed names left alone | no; **tsrs panics** (`classfields_1.rs:763`, `:977`) | ~6 lines | no (duplicate-identifier error already) | 1 crash (2 variants) |
| `c3f14c2fe8` #64633 decorator metadata on object-literal members | the metadata transformer sets `parent` for object literals | crash already avoided (`tstransforms/metadata.rs:258` filters a missing container) | ~8 lines for fidelity | no | 0 (new test passes) |

The three panics end the whole run: on #64574's input (`with (obj) export let a;` in a `.cts` file) the built
`tsrs` panics at `commonjsmodule_3.rs:274` and exits with status 5, printing none of the file's diagnostics and
writing no output files. Go crashed the same way before these fixes.

### Language service, LSP server, Node API, paths

| commit | change | tsrs | port | users | tests |
| --- | --- | --- | --- | --- | --- |
| `21b260b4e7` #64160 top-level imports leave document symbols | `getDocumentSymbolsForChildren` skips imports and import-equals directly under the source file | no (`tsrs_ls/src/symbols.rs:160`) | ~3 lines | **yes**: the outline / breadcrumbs of nearly every module | 3 fourslash (+1 new test after regeneration) |
| `9055b58852` #64642 `workspace/symbol` on an inferred project without a program | `DidRequestProjectTrees` cleans up and updates the inferred project | no; tsrs reaches `get_program().unwrap()` (`tsrs_lsp/src/server.rs:2055`), recovered as an InternalError response | ~6 lines (`cleanup_inferred_project`, `update_program` exist) | rare: Ctrl+T errors after a file moves into a new inferred project | 1 new fourslash test |
| `f9f8d01292` #64597 misported `TokensAreOnSameLine` | the range ends at the next token's `Pos()`, not `End()` | **tsrs has the same misport** (`tsrs_ls/src/format/context.rs:140`) | 1 token | rare: formatting around template spans and unterminated JSX strings | 3 fourslash after regeneration |
| `bcd7c42255` #64624 idle cache timer stored on Session | one line, fixing a 30 s shutdown hang introduced by #64544 | **no bug**: `tsrs_project/src/session.rs:645` stores the timer | none | none | 0 |
| `43521c80ec` #64554 [api] completion symbols from the current snapshot | `createSnapshot` / `updateSnapshot` take `userPreferences` and `prepareAutoImports`; completions with `includeSymbol` error instead of cloning an auto-import snapshot. `lsutil.UserPreferences` is now generated from `tools/userPreferences.schema.json` (field renames, embedded inlay-hint and code-lens structs, some enum keys case-insensitive) | no (`tsrs_api/src/checker/handlers_ls.rs:100`; `tsrs_ls/src/lsutil/userpreferences.rs` is the old shape) | API ~150 lines; the `UserPreferences` restructure ~1,000 lines or a Rust backend for the generator; `tools/gen-fourslash` needs promoted-field support (~20 Go lines) | Node API users: changed completion semantics. **Blocks regenerating the fourslash tests**: ~85 regenerated tests use the new field names and would not compile | 4 upstream Node API tests |
| `688d86d7f6` #64571 [api] `getSymbol(decl)` | new method `getSymbolOfDeclaration` | no ("unknown API method") | ~40 lines + registry rows | API users | upstream sync / async suites |
| `d2f9dd68cf` #64598 [api] merged-symbol checker methods | new `getMergedSymbol`, `getSymbolOfNode`, `getSymbolOfDeclarationForChecker`, `getParentOfSymbolForChecker`; public checker wrappers | no (`checker_07.rs:1934`-`:1986` are `pub(crate)`) | ~12 lines of exports + ~60 handler lines | API users | upstream sync / async suites |
| `ed4807212c` #64159 strongly typed file paths | a 350-file refactor (rooted path types, `CaseSensitivity`) that also changes behaviour: the API `initialize` response sends `caseSensitivity` instead of `useCaseSensitiveFileNames`, `configFileName` is omitted for inferred projects, API `compilerOptions` are raw and resolved against the server cwd, `DocumentIdentifier` validation is stricter; LSP watch roots normalize `..`; TS6059 explains "Matched by include pattern ..." with a related TS1408; `import "./foo/"` under nodenext gives TS2834; `types: ["pkg/"]` resolves | no (`tsrs_api/src/session.rs:375`, `snapshots.rs:50`, `:130`, `tsrs_lsp/src/lspwatcher.rs:549`) | one-to-one with the Go types: large. For behaviour parity: API ~200 lines, LSP ~20, two fourslash harness lines, the two compiler messages | **API users: the resynced Node client reads `caseSensitivity` and breaks against tsrs**; tsc users rarely (TS6059 text) | 1 errors, 1 codes; 19 fourslash (state-baseline header, `documentHighlights02`) |
| `0ad3777160` #64583 LSP middleware for other VS Code extensions | client side only; the protocol generator moved to `tools/scripts/lsp/` | tsrs's `tools/gen-lsproto` is self-contained (only header comments go stale); `npm/tsrs/package.json` lacks the new `./unstable/vscode` and `./unstable/path` exports | 2 export entries | none on the server | 0 |
| `tsc/cmd` changes (#64159, #64544) | a relative `--api --cwd` is resolved; ATA's `npm install` is cancellable | no (`tsrs_cli/src/api.rs:43`, `lsp.rs:61`) | ~10 lines | rare | 0 |
| `7e5d1c1c1d` #64607, `fed0bf2414` #64686, `4529d07ab5` #64647, `efbf3b5067` #64572 | Go test retries, a moved doc comment, VS Code extension only | n/a | none | none (#64647 makes the LSP-attached API session in part 2 more visible) | 0 |

Upstream's Node API `proto.go` adds five methods since the pin (`getSymbolOfDeclaration`, `getMergedSymbol`,
`getSymbolOfNode`, `getSymbolOfDeclarationForChecker`, `getParentOfSymbolForChecker`) and changes the shape of
`initialize`, `createSnapshot` / `updateSnapshot`, the program and transpile `compilerOptions`, `DocumentIdentifier`
and `ProjectResponse.configFileName`. tsrs's registries (`tsrs_api/src/methods.rs`, `paramfields.rs`,
`checker/coverage_table.rs`, `checker/dispatch.rs`) and docs/NODE_API.md need rows for all of them. Resyncing
`npm/sdk` from upstream also needs the `async-client-connection-loss.patch` hunk offsets updated (#64159 adds imports
at the top of `src/api/async/client.ts`).

## Part 2: gaps against the pinned version

### Verified status

Each item in the README table ("What it does and doesn't do"), checked against `f250e637`. The CLI items were run
against the built binary in a scratch project.

| item (README / docs) | still a gap? | evidence |
| --- | --- | --- |
| `--watch`, `-b --watch` | yes | `error: watch mode (--watch) is not supported by tsrs.`, exit 5 (`crates/tsrs_execute/src/execute.rs:247`, `:453`) |
| `--init` | yes | `error: --init is not supported by tsrs.` (`execute.rs:130`) |
| `--showConfig` | yes | `error: --showConfig is not supported by tsrs.` (`execute.rs:244`) |
| `--help` / `--all` full text | yes | prints the version line and a two-line usage (`execute.rs:139`, `:448`) |
| `--locale` | yes | accepted and ignored; tsconfig values are still validated (TS6048, `tsconfigparsing.rs:475`); the language server also ignores the client's locale for messages |
| `--generateTrace` | yes | accepted and ignored, no directory is written (`checker.rs:1336`: "the tracer ... not ported") |
| content mappers, ATA, telemetry, pprof requests (language server) | yes (content mappers: in the language server only, since the content-mapper port; notes/contentmappers.md) | `tsrs_ls/src/spanmap.rs` placeholder (now a re-export of `tsrs_spanmap`); `tsrs_project/src/ata.rs` keeps only the value type; `session.rs:685` and `:701` telemetry hooks send nothing; `tsrs_lsp/src/server.rs:2197-2252` answer `custom/initializeAPISession`, `runGC`, the four profile requests and `setContentMapperContributions` with "not yet ported" |
| Node API profiling, `getCurrentLanguageServerSnapshot` | yes | `tsrs_api/src/methods.rs:191-193` NotImplemented; `session.rs:415` returns the standalone-session error |
| Windows named pipes | yes | `tsrs_api_transport/src/transport.rs:5` |
| 4 conformance error-baseline failures | **a harness mode, not a gap** | with `--baselines js` (the harness runs emit first, as Go's does) all 13,462 error baselines pass; the gate's default `--baselines types,symbols` run shows the known 2 codes (`incorrectRecursiveMappedTypeConstraint`, `typeParameterWithInvalidConstraintType`) and 2 fails (`mutuallyRecursiveInference`, `recursiveMappedTypes`) |
| 63 fourslash failures | yes, unchanged | 4,066 / 63 / 417 measured: 55 content-mapper tests and 8 `@tsc` tests whose reason still says "tsctests.GetFileMapWithBuild needs emit"; emit has been ported since 0.3.0, so that reason is stale |
| 3 of 190 `tsbuild`, 29 of 216 `tsc` scenarios | not re-measured | needs the Go scenario dump (`tools/oracle/tsctests/dump.sh`, which patches the Go checkout and runs `go test`); by name the failing set is the CLI items here: `--init` (10 `Initialized-TSConfig-*`), `--help` (4, plus `tsbuild/commandLine/help`), `--showConfig` (17 + `configDir-template-showConfig`, some of which error out before printing and pass), `--locale` (2 + 2 in `tsbuild`), `--generateTrace` (2) |
| Windows and Intel macOS binaries | yes | `release.yml` builds aarch64-apple-darwin and x86_64 / aarch64 Linux glibc only; win32-x64 is commented out |

### Gaps the docs do not list

Found by diffing Go's `execute`, `cmd/tsc`, LSP handler table and option tables against tsrs. The option tables
themselves match: 136 compiler, 11 build, 5 type-acquisition and 7 watch options (Go's two extra watch names are
commented out).

- **Content mappers in the CLI, not only the language server.** At the pin `tsc` spawns the mappers named in a
  tsconfig's `contentMappers` when run with `--runExternalCode` (`execute/tsc.go` `performCompilation`,
  `tsc.NewContentMapperHost`). tsrs parses and resolves `contentMappers` and validates `--runExternalCode`, but never
  starts a mapper: no file is content-mapped (`tsrs_compiler/src/program.rs:539`, `:733`; `fileloader.rs:71`), so a
  project that depends on one gets different files and diagnostics from tsgo. 7 `tsc` and 2 `tsbuild` scenarios cover
  it; the tsrs harness skips them. Closed by the content-mapper port (notes/contentmappers.md): `tsc`, `tsc -b` and
  incremental builds run mappers, and the 9 scenarios match Go.
- **`--pprofDir` is silently ignored**, on the command line and in `--lsp`. Go writes CPU and heap profiles. (Go's
  `--generateCpuProfile` is parsed and never read at the pin, so ignoring it matches.)
- **LSP `custom/initializeAPISession`** is not ported. This is how an editor extension gets a Node API session
  attached to the running language server, which is what `getCurrentLanguageServerSnapshot` needs. It matters more
  now: upstream #64647 (2026-10-07) exposes the API client modules from the VS Code extension to other extensions.
  `custom/runGC` is also unported.
- **107 watch scenario baselines** (`tscWatch`, `tsbuildWatch`) are outside the 216 / 190 scenario totals.
- **Windows needs more than a build.** The LSP file watcher has no Windows backend (`fswatch/windows.go` and
  `walkdir_windows.go`, 548 Go lines), `nativepath` realpath / symlink handling for Windows (129 lines) is not
  ported, and Windows builds fall back to plain pointers (notes/mem-pointer-compression.md), a configuration no CI
  job builds or runs.

### Ranked by what a `tsc` user would notice

Sizes are from the Go source at the pin (non-test, non-generated lines). For fully ported packages the Rust is
0.7-1.3x the Go (checker 1.29, parser 1.26, tsoptions 1.31, fswatch 0.73).

| # | gap | who notices, and how | Go source to port | port size | tests that would start passing |
| ---: | --- | --- | --- | --- | --- |
| 1 | `tsc --watch`, `tsc -b --watch` | anyone with a watch dev loop; the command exits with an error | `execute/watcher.go` 706, `execute/watchmanager` 492, the watch half of `build/orchestrator.go` (~420: `Watch`, `updateWatch`, `checkTasksForEventChanges`, `computeDesiredWatches`, `DoCycle`) and `buildtask.go` (~100), `vfs/trackingvfs` 68, the watch status reporter; the OS backend is already ported (`tsrs_fswatch`, used by the LSP) | ~1,800 Go lines, medium; the incremental program and build orchestrator it drives are ported | 107 `tscWatch` / `tsbuildWatch` baselines (needs `mock_watch_backend.go`, 239 lines, in the harness) |
| 2 | Windows (and Intel macOS) binaries | every Windows user: the npm package has no win32 build | release matrix entry; Windows fswatch backend 548, nativepath 129, named-pipe transport (Go 19 lines over go-winio; ~150 Rust over `windows-sys`); a CI job for the plain-pointer build. Intel macOS is a matrix line plus PGO training under Rosetta or a cross-built profile | medium (Windows), small (Intel macOS) | none in the suites; needs a Windows CI lane |
| 3 | `--showConfig` | build tooling and anyone debugging `extends` / `${configDir}`; the command exits with an error | `tsoptions/showconfig.go` 389, 4 lines in `tsc.go`; `tsrs_core::json::marshal_indent` exists | ~450 Rust lines, small | 17 `tsc/showConfig` + 1 `extends` scenario; upstream adds 4 more (#64457) |
| 4 | `--locale` / localized messages | non-English users: `--locale` on the command line, and the language server, which tsgo localizes from the editor's display language | `locale` 40, `diagnostics.Localize` / `getLocalizedMessages` ~70, the embedded tables (13 locales, ~85 KB gzipped each, +1.1 MB binary), a locale threaded through the reporters (~100 call sites take one in Go) | ~400 Rust lines + a generator step, small to medium | `tsc` / `tsbuild` `locale` and `bad-locale` (4) |
| 5 | content mappers (CLI and language server; the CLI half is ported, notes/contentmappers.md) | Vue / Svelte / Astro projects that opt in with `contentMappers` + `--runExternalCode` (TypeScript 7.1); tsrs never runs the mapper, so those files have no TypeScript content | `contentmapper` 1,977, `spanmap` 818, `ipc` 989 (partly covered by `tsrs_api_transport`), ~800 lines of hooks (fileloader, program, emitter `MapSourcePosition`, project, LSP), test harness `contentmappertest` ~400 | ~4,500 Go lines, large | 55 fourslash, 7 `tsc`, 2 `tsbuild` |
| 6 | `--generateTrace` | people profiling slow builds with `@typescript/analyze-trace`; no trace is written | `tracing` 763, `checker/tracer.go` 366, ~60 hook sites (checker, relater, program, file loader, emitter, checker pool) | ~1,300 Rust lines, medium; the hooks sit on hot paths and must cost nothing when off, and trace thread ids must cope with tsrs's work stealing | 2 `tsc/generateTrace` |
| 7 | `--init` | new projects; easy workaround (`npx tsc --init`) | `execute/tsc/init.go` 215 (uses the option table and `json.MarshalIndent`) | ~250 Rust lines, small | 10 `Initialized-TSConfig-*` |
| 8 | `--help` / `--all` text | anyone reading help; tsrs prints two lines | `execute/tsc/help.go` 426 (terminal width, option categories) | ~450 Rust lines, small; port the upstream version, whose text changed (#64457, #63915, #64093) | `help`, `help-all`, 2 `show-help`, `tsbuild/help` |
| 9 | LSP-attached Node API session (`custom/initializeAPISession`, `getCurrentLanguageServerSnapshot`) | VS Code extensions that use the TypeScript 7 API through the language server | `lsp/server.go` `handleInitializeAPISession` ~70, `api.NewLSPSession` and the snapshot handler in `api/session.go` ~150 | ~300 Rust lines, small to medium | upstream API suites' LSP-session tests |
| 10 | automatic type acquisition | JavaScript projects in an editor that rely on downloaded `@types` | `project/ata` 1,443 | medium | ATA project tests |
| 11 | profiling: `--pprofDir`, LSP profile requests and `runGC`, Node API `startCPUProfile` / `stopCPUProfile` / `saveHeapProfile` | people profiling the server; Go-runtime-specific, would need a Rust profiler (pprof-rs or similar) | `pprof` 169 + handlers | small, low value | none |
| 12 | LSP telemetry | nobody directly (Microsoft's extension telemetry) | `project/session.go` performance and project-info telemetry, ~200 | small, low value | none |
| - | harness only: 4 conformance error baselines, 8 fourslash `@tsc` tests | nobody | the error baselines already pass in emit mode; the fourslash `@tsc` tests need `tsctests.GetFileMapWithBuild` wired into the fourslash harness now that emit and `--build` exist | small | 8 fourslash |

## Recommendations

**First: move the pin to upstream main now, porting the 49 commits in a short stack of area PRs.** The delta is
small today (8 days, 95 failing tests, about 30 of the 49 commits are ports of 20 lines or fewer), it grows by about
6 commits a day, and tsrs's promise is "identical to tsgo at the pin": until the pin moves, users who compare tsrs
with the current `typescript@next` see the differences listed above, and some are bugs upstream already fixed. In
order of value per line:

1. Output that is wrong today, each a few lines: `sourceRoot` in source maps (#64544, every `.map` file), the false
   TS4094 that fails `declaration` builds (#64573), the CommonJS export miscompile (#64676), lost expando exports in
   `.d.ts` (#64545), `import A, {} from` (#64578), and the three emit panics (#64574, #64651, #64670), which end a
   whole tsrs run with status 5 (on invalid input only). Language service: document symbols (#64160), the
   `workspace/symbol` panic (#64642), the format misport (#64597).
2. Declaration emit honesty: #64461, then #64558 and #64556 (~120 lines in the node builder). These change real
   `declaration` builds from a silent `any` to TS5088 / TS7056, and move the most baselines after #63915.
3. The checker one-liners (#64525, #64530, #64674, #64646, #64636, #64482, #64243, #64584, #64638, #64617) and
   #64553's inference key and cache, which also removes an exponential case in generic call chains.
4. #64566 needs a decision rather than a port: upstream's rule (report merged-interface errors once, at the
   program's first declaration) is also independent of the checker assignment, so tsrs can adopt it and retire its
   per-file rule from #165; the regression fixture and the default-mode bench output change.
5. The lib regeneration (#64604 DOM, #64093, #64536) with the pin bump itself, since DOM code sees different errors
   until it lands; then #63915 source phase imports with #64640 (~300 lines and six generator re-runs; the scanner
   change is the risky part), and #64457's removal of the watch options with the harness's vary-by rule.
6. Node API and language-service plumbing last, as one PR with the `npm/sdk` resync: the #64159 wire changes, the
   five new methods, #64554's snapshot preferences and the `UserPreferences` restructure, which the regenerated
   fourslash tests need before they compile.

Leave #64649 (one node builder per emit resolver) for a measured perf PR: it is churn with no output change, and
tsrs already memoizes the expensive part.

**Then the gaps, in this order:**

1. `--watch` and `-b --watch`. The most-used missing command, about 1,800 Go lines on top of pieces tsrs already
   has (incremental programs, the build orchestrator, `tsrs_fswatch`), and the 107 watch scenario baselines give it a
   test bed.
2. `--showConfig`, `--init` and `--help` together (about 1,150 Rust lines, all small): they close most of the
   remaining `tsc` scenario failures and remove three of the "not supported" exits. Port them from upstream main,
   whose help text and option definitions changed.
3. Windows binaries, if Windows users are wanted: a CI lane for the plain-pointer build first, then the fswatch and
   nativepath ports and named pipes.
4. `--locale`, mainly for the language server, then `--generateTrace`.
5. Content mappers when a framework ships one people use; it is the largest item (about 4,500 Go lines) and is
   still opt-in upstream. (Since done for `tsc`, `-b` and incremental builds; the language server is phase 2 of
   notes/contentmappers.md.)

Cheap housekeeping found on the way: the 8 fourslash `@tsc` failures give a stale reason ("needs emit"), and the
README should say that the 4 conformance error-baseline failures disappear when the harness runs emit
(`--baselines js`), and that content mappers are a CLI gap too.

## How to reproduce

```sh
git clone --filter=blob:none https://github.com/microsoft/TypeScript $TS          # upstream main
git -C $TS worktree add --detach $TS-pin b85298b6a81f772d080b0455de0ca9d744cd6fd6
cargo build --release --locked -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash
ln -sfn $TS-pin ts-ref   # then $TS for the upstream run
TSRS_TEST_RESULTS=target/test-results-<pin|main> ./target/release/tsrs-test run --suite all --baselines types,symbols,js --jobs 8
TSRS_TEST_RESULTS=target/test-results-<pin|main>-maps ./target/release/tsrs-test run --suite all --baselines jsmap,sourcemap --jobs 8
./target/release/tsrs-fourslash run -j 8
git -C $TS log --format=%h --name-only b85298b6..origin/main -- tsc/testdata   # attribute a failing test to its commit
```
