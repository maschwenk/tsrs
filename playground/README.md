# tsrs Playground

A standalone browser playground rendering the vendored official TypeScript Playground page, navigation, examples,
styles, fonts, and themes with this checkout's tsrs WebAssembly compiler. There is no custom visual stylesheet.
Checking and emit run locally in a Web Worker. Monaco provides editing and syntax highlighting, without loading
the JavaScript TypeScript compiler or its language service.

## Run locally

Use the repository's Node.js and pnpm versions and the Vite+ CLI (`vp`). From the repository root:

```sh
vp install --frozen-lockfile
rustup target add wasm32-wasip1
tools/wasm/build.sh
cd playground
vp dev
```

For development without Binaryen's `wasm-opt`, use `WASM_OPT=none tools/wasm/build.sh` instead. Rebuild the module
after changing Rust compiler code. The app uses the local `npm/tsrs-wasm` package, rather than an npm release.

`vp build` produces `playground/dist/`, which can be served as a static site, including beneath a directory such as
`/playground/`. Use a URL with a trailing slash for directory hosting. `vp preview` serves that build locally.
Generated option metadata, example assets, the plugin loader, license notices, WASM, and build output are excluded from git.

## Features and limits

- TypeScript, JavaScript, TSX, and declaration input; strict checking by default.
- tsrs diagnostics, including related information and compiler-option errors, with UTF-16 editor positions.
- JavaScript and declaration output with selectors for multiple files, compiler settings, themes, and resizing.
- English upstream examples and compressed `#code/` links, including compiler options and editor selection.
- Multiple virtual files using `// @filename: path/to/file.ts`, including relative imports, JSON, and `package.json`.
  Duplicate normalized paths are rejected. A virtual `/tsconfig.json`, when supplied, defines the project instead of
  the settings dropdown. Otherwise the dropdown generates the config. TypeScript's standard libraries are bundled
  in WASM; imported packages must be provided as virtual files.
- Run and Logs use the upstream evaluator with tsrs-emitted JavaScript. This runs the first emitted JavaScript file;
  it does not bundle imports, install npm dependencies, or provide a module loader. Runtime failures appear in Logs.
- Share and Markdown/project exports retain the upstream controls. External project exports run on their destination's
  compiler, rather than this site's tsrs instance.
- Custom plugins retain upstream loading, settings, persistence, and lifecycle hooks. Plugins using editing, UI,
  diagnostics, or emit can work; TypeScript compiler/language-service API calls report an unsupported error.

AST/type inspection, semantic autocomplete, semantic hover, inlay hints, and automatic package type acquisition
are unavailable. An optional AST pane explains these gaps when enabled in Settings; the ATA control is disabled.
The compiler version is pinned to the repository;
`?ts=` cannot select a historical compiler. See [compatibility](../docs/COMPATIBILITY.md).

## Verification

Build the native reference with `cargo build --profile dev -p tsrs_cli -p tsrs_testrunner` from the repository root.
Then, from `playground/`:

```sh
vp run prepare-assets
vp check
vp test run
vp build
vp exec playwright install chromium
vp run test:browser
```

`TSRS_NATIVE` can select another native reference binary. `PLAYWRIGHT_CHROMIUM_EXECUTABLE` can select an existing
Chromium executable. The browser checks use the production build; integration checks compare diagnostics and emitted
file bytes against native tsrs. The Vite build rejects a bundled JavaScript TypeScript compiler or language-service worker.
It also verifies the vendored website's visual source against the pinned upstream SHA-256 manifest.

## Upstream and licensing

See [vendoring provenance](vendor/UPSTREAM.md) for the pinned source, copied paths, licenses, and adaptations.
The upstream UI code is MIT-licensed, with documentation under CC BY 4.0; tsrs and its default libraries remain
under their existing licenses. Attribution and both upstream license files are retained in `vendor/`.
