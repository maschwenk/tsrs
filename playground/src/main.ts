import * as Monaco from "monaco-editor/esm/vs/editor/editor.api";
import "monaco-editor/esm/vs/basic-languages/typescript/typescript.contribution";
import "monaco-editor/esm/vs/basic-languages/javascript/javascript.contribution";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import React from "react";
import ReactDOM from "react-dom";
import metadata from "./generated/compiler.json";
import { setupPlayground } from "../vendor/packages/playground/src";
import { optionsPlugin } from "../vendor/packages/playground/src/sidebar/plugins";
import { runPlugin } from "../vendor/packages/playground/src/sidebar/runtime";
import { sandboxTheme, sandboxThemeDark } from "../vendor/packages/sandbox/src/theme";
import { setEditorTheme } from "../vendor/packages/playground/src/theme";
import { playCopy } from "../vendor/packages/typescriptlang-org/src/copy/en/playground";
import { createSandbox } from "./sandbox";
import { errorsPane, guardPlugin, outputPane } from "./panes";
import type { PluginFactory } from "../vendor/packages/playground/src";

export async function startPlayground(pageContext: { playgroundHandbookTOC: { docs: unknown[] } }) {
  const globals = window as typeof window & Record<string, any>;
  // Load AMD after ESM dependencies: their UMD wrappers must not register anonymous plugins.
  await new Promise<void>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = new URL("require.js", document.baseURI).href;
    script.onload = () => resolve();
    script.onerror = () =>
      reject(new Error("The plugin loader could not be loaded. Reload to retry."));
    document.head.appendChild(script);
  });
  globals.MonacoEnvironment = { getWorker: () => new EditorWorker() };
  const monaco = {
    ...Monaco,
    languages: { ...Monaco.languages, typescript: metadata },
  } as unknown as typeof import("monaco-editor");
  monaco.editor.defineTheme("light", sandboxTheme);
  monaco.editor.defineTheme("dark", sandboxThemeDark);
  monaco.editor.defineTheme("hc", { base: "hc-black", inherit: true, rules: [], colors: {} });
  const localize = (key: string) => (playCopy as Record<string, string>)[key] ?? key;
  globals.i = localize;
  globals.__tsLocalize = localize;
  globals.optionsSummary = metadata.optionDeclarations
    .filter((option) => option.description)
    .map((option) => ({
      id: option.name,
      display: option.name,
      oneliner: option.description!.message,
      categoryID: option.category?.key ?? "",
      categoryDisplay: option.category?.message ?? "",
    }));
  globals.react = React;
  globals.reactDOM = ReactDOM;

  for (const id of ["compiler-status", "plugin-status"]) {
    const status = document.createElement("span");
    status.id = id;
    status.hidden = true;
    status.setAttribute("role", "status");
    document.body.appendChild(status);
  }
  globals.playgroundHandbookTOC = pageContext.playgroundHandbookTOC;
  document.getElementById("loader")!.remove();
  const container = document.getElementById("playground-container")!;
  container.style.display = "flex";
  container.style.height = `${Math.max(window.innerHeight, 600) - Math.round(container.getClientRects()[0].top) - 18}px`;
  const { sandbox, adapter } = createSandbox(monaco);
  setEditorTheme(
    document.documentElement.classList.contains("dark-theme") ? "dark" : "light",
    monaco.editor,
  );
  const plugins: PluginFactory[] = [];
  if (!localStorage.getItem("disable-sidebar-js")) plugins.push(outputPane(adapter, "js"));
  if (!localStorage.getItem("disable-sidebar-dts")) plugins.push(outputPane(adapter, "dts"));
  if (!localStorage.getItem("disable-sidebar-err")) plugins.push(errorsPane(adapter));
  if (!localStorage.getItem("disable-sidebar-run")) plugins.push(runPlugin);
  if (!localStorage.getItem("disable-sidebar-plugins")) plugins.push(optionsPlugin);
  if (!plugins.length) plugins.push(outputPane(adapter, "js"));
  if (localStorage.getItem("enable-sidebar-ast"))
    plugins.push(() => ({
      id: "ast",
      displayName: "AST",
      didMount: (_, container) => {
        container.textContent =
          "AST/type inspection, semantic autocomplete, hover, inlay hints, and automatic package type acquisition are unavailable in the tsrs browser compiler. Checking, JavaScript, and declarations use tsrs WASM.";
      },
    }));

  // RequireJS remains only for the upstream plugin contract, never for compiler loading.
  globals.requirejs.config({
    paths: { unpkg: "https://unpkg.com", local: "http://localhost:5000" },
    waitSeconds: 20,
  });
  for (const [name, value] of Object.entries({
    react: React,
    "react-dom": ReactDOM,
    "monaco-editor": monaco,
    "@typescript/sandbox": { createTypeScriptSandbox: () => sandbox },
    typescript: adapter.ts,
  }))
    globals.define(name, [], () => value);
  globals.requirejs.onError = (error: Error) => {
    document.getElementById("plugin-status")!.textContent =
      "Plugin failed to load: " + error.message;
  };
  const playground = setupPlayground(
    sandbox,
    monaco,
    {
      lang: "en",
      prefix: location.pathname.replace(/\/$/, ""),
      supportCustomPlugins: true,
      plugins,
      enableSemanticFeatures: false,
      compilerVersionLabel: `tsrs ${metadata.tsrsVersion}`,
      wrapPlugin: guardPlugin,
    },
    localize,
    React,
  );
  globals.playground = playground;
  const settingsButton = document.getElementById("playground-settings")!;
  const showSettings = settingsButton.onclick;
  settingsButton.onclick = (event) => {
    showSettings?.call(settingsButton, event);
    const ata = document.getElementById("disable-ata") as HTMLInputElement | null;
    if (ata) {
      ata.checked = true;
      ata.disabled = true;
      ata.title = "Automatic type acquisition is unavailable in the tsrs browser compiler.";
    }
  };

  adapter.compiler.subscribe((result) => {
    const tab = document.getElementById("playground-plugin-tab-errors");
    if (tab) tab.setAttribute("aria-label", `Errors (${result.diagnostics.length})`);
  });
  const requestedVersion = new URLSearchParams(location.search).get("ts");
  if (requestedVersion && requestedVersion !== metadata.version)
    document.getElementById("plugin-status")!.textContent =
      `This link requests TypeScript ${requestedVersion}; this playground uses pinned tsrs ${metadata.tsrsVersion}.`;
  adapter.editor.focus();
  adapter.editor.layout();
  window.addEventListener("beforeunload", () => adapter.dispose());
}
