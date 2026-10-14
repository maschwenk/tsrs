import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import path from "node:path";
import ts from "typescript";
import JSON5 from "json5";
import { marked } from "marked";
import { createRequire } from "node:module";

const root = fileURLToPath(new URL("../", import.meta.url));
const removed = new Set([
  "noStrictGenericChecks",
  "suppressExcessPropertyErrors",
  "suppressImplicitAnyIndexErrors",
  "keyofStringsOnly",
  "importsNotUsedAsValues",
  "preserveValueImports",
  "charset",
  "out",
  "baseUrl",
]);

/** Only compiler metadata reaches the browser; TypeScript itself is build-time tooling. */
export function prepareAssets() {
  const wasm = path.join(root, "../npm/tsrs-wasm/tsrs.wasm");
  if (!existsSync(wasm))
    throw new Error("Build the playground compiler first: tools/wasm/build.sh");
  const cargo = readFileSync(path.join(root, "../Cargo.toml"), "utf8");
  const version = cargo.match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
  const compilerVersion = cargo.match(
    /\[workspace\.metadata\.typescript\][\s\S]*?version\s*=\s*"([^"]+)"/,
  )[1];
  const options = ts.optionDeclarations.map((option) => ({
    name: option.name,
    type: typeof option.type === "string" ? option.type : Object.fromEntries(option.type),
    isCommandLineOnly: !!option.isCommandLineOnly,
    category: option.category,
    description: option.description,
  }));
  const supportedEnum = (value, unsupported) =>
    Object.fromEntries(
      Object.entries(value).filter(
        ([key, v]) => !unsupported.includes(key) && !unsupported.includes(String(v)),
      ),
    );
  const metadata = {
    version: compilerVersion,
    npmVersion: cargo.match(
      /\[workspace\.metadata\.typescript\][\s\S]*?nightly\s*=\s*"([^"]+)"/,
    )[1],
    tsrsVersion: version,
    versionMajorMinor: compilerVersion.split(".").slice(0, 2).join("."),
    optionDeclarations: options.filter((option) => !removed.has(option.name)),
    allOptions: options,
    ScriptTarget: supportedEnum(ts.ScriptTarget, ["ES3", "ES5"]),
    ModuleKind: supportedEnum(ts.ModuleKind, ["AMD", "UMD", "System"]),
    ModuleResolutionKind: supportedEnum(ts.ModuleResolutionKind, ["Classic", "NodeJs", "Node10"]),
    JsxEmit: ts.JsxEmit,
  };
  const generated = path.join(root, "src/generated");
  mkdirSync(generated, { recursive: true });
  writeFileSync(path.join(generated, "compiler.json"), JSON.stringify(metadata));

  const source = path.join(root, "vendor/packages/playground-examples/copy/en");
  const output = path.join(root, "public/js/examples");
  mkdirSync(output, { recursive: true });
  cpSync(source, path.join(output, "en"), { recursive: true });
  const toc = JSON5.parse(readFileSync(path.join(source, "sections.json"), "utf8"));
  const examples = [];
  const visit = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) =>
      a.name.localeCompare(b.name),
    )) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) visit(full);
      else if (/\.(tsx?|js)$/.test(entry.name)) {
        const contents = readFileSync(full, "utf8");
        const header = contents.startsWith("//// {")
          ? JSON.parse(contents.split(/\r?\n/, 1)[0].slice(5))
          : {};
        const title = entry.name.replace(/\.(tsx?|js)$/, "");
        examples.push({
          name: entry.name,
          title: header.title ?? title,
          lang: "en",
          path: path.relative(source, dir).split(path.sep),
          id: title
            .toLowerCase()
            .replace(/[^\x20-\x7E]/g, "-")
            .replace(/[ /+]/g, "-"),
          sortIndex: header.order ?? 1,
          compilerSettings: header.compiler ?? {},
          hash: createHash("sha512").update(contents).digest("hex"),
        });
      }
    }
  };
  visit(source);
  writeFileSync(path.join(output, "en.json"), JSON.stringify({ ...toc, examples }));
  writeFileSync(path.join(generated, "examples.json"), JSON.stringify({ ...toc, examples }));
  const handbook = path.join(root, "vendor/packages/playground-handbook");
  const generator = readFileSync(path.join(handbook, "scripts/generateJSON.mjs"), "utf8");
  const chapters = [...generator.matchAll(/"([^"\n]+\.md)"/g)].map((match) => match[1]);
  const handbookOutput = path.join(root, "public/_playground-handbook");
  mkdirSync(handbookOutput, { recursive: true });
  const docs = chapters.map((chapter) => {
    const title = chapter.replace(/\.md$/, "");
    const id = title
      .toLowerCase()
      .replace(/[^\p{ASCII}]/gu, "-")
      .replace(/[ /+]/g, "-");
    const html = marked.parse(readFileSync(path.join(handbook, "copy/en", chapter), "utf8"), {
      async: false,
    });
    writeFileSync(
      path.join(handbookOutput, id + ".html"),
      `<div id="___gatsby"><div id="gatsby-focus-wrapper"><div>${html}</div></div></div>`,
    );
    return { type: "href", title, href: "_playground-handbook/" + id + ".html" };
  });
  writeFileSync(path.join(generated, "handbook.json"), JSON.stringify({ docs }));
  const website = path.join(root, "vendor/packages/typescriptlang-org/static");
  cpSync(path.join(website, "js/docsearch.js"), path.join(root, "public/js/docsearch.js"));
  mkdirSync(path.join(root, "public/css"), { recursive: true });
  cpSync(path.join(website, "css/docsearch.css"), path.join(root, "public/css/docsearch.css"));
  cpSync(
    path.join(root, "node_modules/requirejs/require.js"),
    path.join(root, "public/require.js"),
  );
  // Static distributions carry the notices alongside the bundled source and examples.
  const licenses = path.join(root, "public/licenses");
  mkdirSync(licenses, { recursive: true });
  for (const [source, name] of [
    ["vendor/LICENSE-CODE", "TypeScript-Website-MIT.txt"],
    ["vendor/LICENSE", "TypeScript-Website-CC-BY-4.0.txt"],
    ["vendor/UPSTREAM.md", "UPSTREAM.txt"],
    ["../LICENSE", "tsrs-Apache-2.0.txt"],
    ["../NOTICE", "tsrs-NOTICE.txt"],
    ["vendor/requirejs/LICENSE", "requirejs-LICENSE.txt"],
  ])
    cpSync(path.join(root, source), path.join(licenses, name));
  const require = createRequire(path.join(root, "package.json"));
  const dependencies = [
    "monaco-editor",
    "lz-string",
    "react",
    "react-dom",
    "react-helmet",
    "react-intl",
    "requirejs",
    "typescript",
  ];
  const notices = [
    "tsrs Playground",
    "UI: microsoft/TypeScript-Website@5cba2a5c364d1b27e941f04dd4eac10d61bedf02",
    "Upstream code is MIT; upstream documentation is CC BY 4.0. Adaptations are described in UPSTREAM.txt.",
    "tsrs and its bundled TypeScript libraries are covered by tsrs-Apache-2.0.txt and tsrs-NOTICE.txt.",
    "",
  ];
  for (const dependency of dependencies) {
    const directory = path.dirname(require.resolve(dependency + "/package.json"));
    for (const filename of [
      "LICENSE",
      "LICENSE.txt",
      "LICENSE.md",
      "License.txt",
      "ThirdPartyNoticeText.txt",
      "ThirdPartyNotices.txt",
    ]) {
      if (!existsSync(path.join(directory, filename))) continue;
      const name = dependency + "-" + filename;
      cpSync(path.join(directory, filename), path.join(licenses, name));
      notices.push(name);
    }
  }
  const scheduler = path.dirname(
    createRequire(require.resolve("react-dom/package.json")).resolve("scheduler/package.json"),
  );
  cpSync(path.join(scheduler, "LICENSE"), path.join(licenses, "scheduler-LICENSE"));
  notices.push("scheduler-LICENSE");
  writeFileSync(path.join(licenses, "NOTICE.txt"), notices.join("\n") + "\n");
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  prepareAssets();
