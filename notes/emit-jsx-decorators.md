# emit/jsx-decorators (waves E10 + E11)

Branch `emit/jsx-decorators`, based on `emit/core` (merged, not rebased).

## Ported

- `crates/tsrs_transformers/src/jsxtransforms/jsx.rs`: all of jsx.go (classic `React.createElement` with
  `jsxFactory`/`jsxFragmentFactory`/`reactNamespace`, `react-jsx`/`react-jsxdev` with the automatic runtime import or
  `require`, the `createElement` fallback for a `key` after a spread, entity decoding, whitespace fixup). `preserve`
  and `react-native` do not run the transformer (`CompilerOptions.GetJSXTransformEnabled`, emitter.go).
- `crates/tsrs_transformers/src/tstransforms/legacydecorators.rs`, `metadata.rs`, `typeserializer.rs`: all functions.
- `crates/tsrs_transformers/src/utilities.rs`: `IsGeneratedIdentifier`, `MoveRangePastModifiers`,
  `MoveRangePastDecorators` (utilities.go; emit/transforms has the same functions in its `todo_core.rs`, keep one).
- `crates/tsrs_transformers/src/emit_test.rs`: isolated smoke tests (program + lent checker + transforms + printer).
- `tools/oracle/emit/monorepo.sh` + `list-packages.js`: the monorepo emit oracle (see below).

## Decisions / deviations in shape

- Go `tx.Visitor().Visit(node)` (the raw visit callback, no SyntaxList lifting) is a direct call of the
  transformer's own `visit` (it is the callback the visitor holds).
- Go `visit` funcs take nil-able nodes only where Go can pass nil (`JSXTransformer.visit`, for `{}` expressions).
- Go appends possibly-nil visitor results into slices in a few places (`transformDecorators`,
  `transformJsxAttributesToExpression`, the constructor decoration statement); the Rust code skips `None`. In all of
  these Go would print a nil node (panic), and the inputs cannot produce nil there.
- `typeserializer.go` exported/private name pairs (`SerializeTypeOfNode`/`serializeTypeOfNode`): the private one
  gets a trailing `_` (printer convention).
- `metadataSerializer.c` (the context) is a `Cell` of a `Copy` struct; `defer` restores are explicit.
- `collections.GroupBy` (metadata decorators last) is `Iterator::partition` (stable, same order).
- jsx `getSortedSpecifiers` sorts by property name then name, case-sensitive (so `Fragment` < `jsx`), as Go.

## Monorepo oracle

`tools/oracle/emit/monorepo.sh [--only jsx|decorators] [--keep] [pkg...]` lists the packages whose `build` script
runs tsc (`list-packages.js`, effective options from `tsgo --showConfig`), emits each with tsgo and with
`TSRS_EMIT=1 tsrs` into `$OUT/emit-go/<pkg>` / `$OUT/emit-rs/<pkg>` (redirecting `outDir`, `declarationDir` when
the config sets one, and `tsBuildInfoFile`) and compares every file byte for byte. It aborts if anything under the
package directory is newer than its start marker, and checks `git status` before/after. (An early version
mis-parsed empty TSV columns and let one package write `dist/types` into the checkout; the files were removed and the
script now refuses to continue on the first stray write.)

JSX packages (11): apps/guest-app-screenshots (noEmit), dionysus-customizer, icons-react-native, mailer,
marketing-emails, online-ordering, pos-network, sightglass, telemetry-react-native, transport, user-journey-mobile.
Decorator packages (13): adm-entities, api-client, parallel, provider-abstract-pool, service-auth, service-env,
service-feature-flags, service-framework-utils, service-olympus-client, service-utils, temporal-utils,
threat-intel-entities, vulnmgmt-entities.

## Status / blocked on

- End-to-end emit of any TS file needs the type eraser, import elision, runtime syntax (emit/transforms, E3) and the
  ES module transformer (E4); the `.js` baseline harness (`--baselines js`) is E2 on emit/core.
