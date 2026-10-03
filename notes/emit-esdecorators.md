# mfs-cx/emit-esdecorators (wave E6, ES decorators part)

Branch history (no force pushes): `07be672` = the sanitized E10/E11 commit (`mfs-cx/emit-jsx-decorators`),
`eb2491b` = dependency `emit/classfields` at d949ed9 (E5: classfields, namedevaluation, classthis, estransforms
utilities) squashed in, then this port. `using`/disposal (the rest of E6) is not here: it belongs to emit/transforms.

## Ported

`transformers/estransforms/esdecorator.go` (2751 lines) in full, replacing the gate stub:
`estransforms/esdecorator_1.rs` (Go 1–1230: types, constructor and secondary visitors, lexical-entry stack,
`visit`, `createClassInfo`, `transformClassLike`, class declaration/expression visits, constructor) and
`esdecorator_2.rs` (Go 1231–end: `partialTransformClassElement`, member visits, auto-accessor descriptors,
super-property rewriting, destructuring targets, pending expressions, decorator/call binding, descriptor objects and
forwarders, `Symbol.metadata`, `injectClassThisAssignmentIfMissing`).

Decisions:
- `lexicalEntry`, `classInfo`, `memberInfo` are arena `P<..>` structs; fields Go assigns after construction are
  `Cell`/`RefCell`, so shared-pointer mutation (e.g. `mi.memberDescriptorName` after `memberInfos.Set`) matches Go.
- `pendingExpressions` is `RefCell<Option<Vec<..>>>` (Go nil vs empty); saved/restored by value on the stack.
- `createDescriptorFunc` is `fn(&esDecoratorTransformer, P<Node>, Option<P<ModifierList>>) -> P<Node>`.
- `createMethodDescriptorForwarder` builds a get accessor, as Go does.
- Go appends nil visitor results into statement lists in a few places; Rust skips `None` (unreachable there).

## Evidence

Standalone (this branch): conformance errors + `--baselines types,symbols` identical to main in default and
`TSRS_LAZY_MEMBERS=0` modes; fourslash 4066/63 same pass list; `RUSTFLAGS="-D warnings" cargo check --workspace
--locked --all-targets` clean; `emit_gate` 3/3; `--baselines js` 1364 / 15197, 0 fail (the type eraser is a stub on
this branch, so TS inputs crash there).

Scratch integration (local only, never pushed): this branch + emit/transforms (4c40ea2) + emit/async +
emit/es2016-2020 + emit/sourcemaps; reference tsgo built from the pinned ts-ref commit.
- ES decorator tests (every variant under `esDecorators`): 163 / 219 pass, 0 fail, 0 crash, 56 skip.
- decorator group (conformance/decorators, compiler/decorator*, *Metadata*): 144 / 213 pass, 0 fail, 0 crash, 69 skip
  (was 131 with 13 esdecorator crashes).
- whole suite: 13278 / 15197 pass, 15 fail, 98 crash (all `using`), 1 timeout, 1805 skip. The 15 fails are the
  moduleDetection cache-key and commonjs nil-vs-empty issues fixed on emit/transforms f708ab1 (not in this scratch).
- private monorepo emit oracle (read-only, all outputs outside it; full emit with maps): 103/103 packages,
  10,248 files identical, 0 different; per-file rows checked, not just the exit status.

## Remaining blockers

- `using`/`await using` (using.go): 98 crashing variants; owned by emit/transforms.
- The scratch numbers depend on emit/transforms, emit/async, emit/es2016-2020 and emit/sourcemaps landing.
