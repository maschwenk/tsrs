# TypeScript Playground vendoring

Repository: <https://github.com/microsoft/TypeScript-Website>

Commit: `5cba2a5c364d1b27e941f04dd4eac10d61bedf02` (branch `v2`, 2026-10-13).

Copied paths preserve their upstream layout beneath this directory:

- `packages/playground/`: UI source, upstream configuration, and checks.
- `packages/sandbox/src/`: editor helpers and reference sandbox source.
- `packages/playground-worker/`: reference worker and virtual-file splitting implementation.
- `packages/playground-examples/`: English examples, original generators, and metadata.
- `packages/playground-handbook/`: English Help chapters and their original ordering generator.
- `packages/typescriptlang-org/src/templates/play.tsx` and `play.scss`: the original page and playground stylesheet.
- `packages/typescriptlang-org/src/components/layout*`, `ShowExamples*`, `Intl*`, and `HeadSEO.tsx`:
  original navigation, footer, example menu, typography, theme/customization controls, and localization;
  `playgroundHandbook.tsx` is retained as the Help page rendering reference.
- Website English labels, their small library dependencies, Microsoft logo asset, `static/fonts/`,
  DocSearch assets, `src/style/globals.scss`, and `gatsby-ssr.js` (theme initialization reference).
- `LICENSE-CODE` and `LICENSE`: MIT code license and CC BY 4.0 content license, respectively.
- `requirejs/LICENSE`: the plugin loader's MIT license from the RequireJS `2.3.7` tag, omitted by its npm tarball.

The playground page and its website shell are rendered directly from vendored source. Their files and fonts
remain byte-for-byte upstream; `WEBSITE-MANIFEST.json` is verified on every build. No custom stylesheet overrides
the official styles. The Gatsby build application is replaced by a small Vite host. `playground/src/` supplies
the tsrs adapter, worker, virtual-file mapping, and diagnostic/output panes using the upstream design system.
The reference sandbox and TypeScript worker are not
browser entrypoints and are not bundled. Static compiler-option metadata is generated from build-time TypeScript
6.0.2; removed TypeScript 7 options are excluded from dropdowns. TypeScript metadata is not a runtime compiler.

Local modifications to upstream source:

- `playground/src/index.ts`: optional host configuration for a pinned version label, semantic-feature disabling,
  and guarded plugin lifecycles; avoid TypeScript-only ESM setting adjustments; handle a disabled Logs pane.
- `playground/src/createElements.ts`: remove an unused runtime TypeScript import and avoid remounting an already active tab.
- `playground/src/createConfigDropdown.ts`: link the external TSConfig reference.
- `playground/src/exporter.ts`: serialize current compiler options through the tsrs host, including bundler resolution,
  and use the repository's pinned npm TypeScript nightly for external project dependencies.
- `playground/src/sidebar/runtime.ts`: surface rejected compilation promises in Logs.
- `sandbox/src/getInitialCode.ts`: import the ESM-compatible `lz-string` package.

`scripts/upstream-website.mjs` makes hosting adaptations at build time without editing the vendored website:
replace the compiler bootstrap with tsrs, bridge Gatsby URLs and English localization to ESM, use generated
English example metadata, import the footer image as an ESM asset, and replace the brand/title with tsrs.
Help chapters are rendered from the vendored Markdown at build time and served locally in upstream order.
The empty cookie-banner element is retained without loading Microsoft's consent service. Other site links
point to typescriptlang.org. Static asset URLs, including fonts, work under a directory base path.

To update, download the desired upstream revision, replace these copied paths and license files, reapply the listed
adaptations, update this revision, and run the checks in the playground README. Exclude the vendor tree from app
formatting/linting to avoid unrelated upstream rewrites. Vendored package manifests are provenance; only the root
playground package participates in this repository's pnpm workspace.
