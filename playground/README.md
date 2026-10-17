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

Build the native reference with `cargo build --profile dev -p tsrs_cli` from the repository root.
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

Set `PLAYWRIGHT_BASE_URL=https://tsrs-playground.void.app` to run the same browser checks against the hosted site
without starting a local preview server. Hosted checks allow longer timeouts for network loading.

## Deployment

[The playground](https://tsrs-playground.void.app) is a static site on Void Cloud. The
[Playground workflow](../.github/workflows/void-deploy.yml) validates pull requests, including the native/WASM
comparison and production browser checks. The [Release workflow](../.github/workflows/release.yml) calls it after
publishing the GitHub release from `oxc-project/tsrs`'s `main`. Dry runs and skipped releases do not deploy.
Production deployments are serialized; there is no separate manual trigger or preview deployment.
Void's OIDC service requires the release commit to remain the current `main` head.

The deploy job uploads the exact validated build artifact using the repo's locked Void CLI, then runs the browser checks
against the hosted URL. GitHub OIDC grants a short-lived project-scoped token; no `VOID_TOKEN` secret is needed.
The platform URL and project slug are public workflow configuration. Dependencies and the compiler are built in
the validation job, which has no deployment permission.

For the initial setup, build and verify the site as above, then run these commands from the repository root:

```sh
vp exec void connect https://api.void.cloud
vp exec void deploy --platform void --project tsrs-playground --dir playground/dist
vp exec void github install
vp exec void github connect tsrs-playground \
  --repo oxc-project/tsrs --branch main --executor github_actions \
  --workflow .github/workflows/void-deploy.yml
vp exec void github status tsrs-playground
```

The first deployment creates the project (with confirmation in an interactive terminal). GitHub App installation
needs repository access; use `github join` for an existing shared installation, or ask the repository owner to install
it. The OIDC connection must authorize exactly the repository, branch, and workflow above before the workflow lands. Local
connection state under `.void/` is ignored by git; fresh CI runners connect with `--no-login` before deployment.

Inspect deployments with `vp exec void project status tsrs-playground`. To restore a previous deployment,
run `vp exec void project rollback <deployId>` from the linked checkout. A hosted browser-check failure marks
the workflow failed after publication; inspect the failure and roll back if needed. Fix the underlying problem
before the next release, which deploys again automatically.

## Upstream and licensing

See [vendoring provenance](vendor/UPSTREAM.md) for the pinned source, copied paths, licenses, and adaptations.
The upstream UI code is MIT-licensed, with documentation under CC BY 4.0; tsrs and its default libraries remain
under their existing licenses. Attribution and both upstream license files are retained in `vendor/`.
