import type { PluginFactory, PlaygroundPlugin } from "../vendor/packages/playground/src";
import type { DesignSystem } from "../vendor/packages/playground/src/ds/createDesignSystem";
import type { SandboxAdapter } from "./sandbox";
import { diagnosticRange, diagnosticText, type Compilation } from "./project";

function compilerFailure(ds: DesignSystem, error: string, adapter: SandboxAdapter) {
  ds.p(error);
  const retry = ds.button({ label: "Retry compiler", onclick: () => adapter.retry() });
  retry.id = "compiler-retry";
}

export function outputPane(adapter: SandboxAdapter, kind: "js" | "dts"): PluginFactory {
  return (i, utils) => {
    let ds: DesignSystem;
    let unsubscribe: (() => void) | undefined;
    let currentFile = "";
    const render = (result: Compilation) => {
      ds.clear();
      if (result.error) {
        compilerFailure(ds, result.error, adapter);
        return;
      }
      const outputs = Object.entries(result.files).filter(([name]) =>
        kind === "js" ? /\.(?:[cm]?js|jsx)$/.test(name) : /\.d\.[cm]?ts$/.test(name),
      );
      if (!outputs.length) {
        ds.showEmptyScreen("No files emitted. Check the compiler settings and errors.");
        return;
      }
      if (!outputs.some(([name]) => name === currentFile)) currentFile = outputs[0][0];
      const code = ds.code("");
      const update = () => {
        const selectedFile = currentFile;
        const text = outputs.find(([name]) => name === currentFile)![1];
        code.textContent = text;
        void adapter.monaco.editor
          .colorize(text, kind === "js" ? "javascript" : "typescript", {})
          .then((html) => {
            if (code.isConnected && currentFile === selectedFile) code.innerHTML = html;
          });
      };
      // Single-file output has the exact upstream pre/code structure.
      if (outputs.length > 1) {
        const select = document.createElement("select");
        select.setAttribute(
          "aria-label",
          kind === "js" ? "JavaScript output file" : "Declaration output file",
        );
        for (const [name] of outputs) select.add(new Option(name, name));
        select.value = currentFile;
        ds.container.insertBefore(select, ds.container.firstChild);
        select.onchange = () => {
          currentFile = select.value;
          update();
        };
      }
      update();
    };
    return {
      id: kind,
      displayName: i(kind === "js" ? "play_sidebar_js" : "play_sidebar_dts"),
      didMount: (_, element) => {
        ds = utils.createDesignSystem(element);
        ds.code("");
        unsubscribe = adapter.compiler.subscribe(render);
      },
      didUnmount: () => unsubscribe?.(),
    };
  };
}

export function errorsPane(adapter: SandboxAdapter): PluginFactory {
  return (i, utils) => {
    let unsubscribe: (() => void) | undefined;
    let ds: DesignSystem;
    return {
      id: "errors",
      displayName: i("play_sidebar_errors"),
      didMount: (_, container) => {
        ds = utils.createDesignSystem(container);
        unsubscribe = adapter.compiler.subscribe((result) => {
          ds.clearDeltaDecorators(true);
          ds.clear();
          if (result.error) {
            compilerFailure(ds, result.error, adapter);
            return;
          }
          if (!result.diagnostics.length) {
            ds.showEmptyScreen(i("play_sidebar_errors_no_errors"));
            return;
          }
          ds.subtitle("Errors in code");
          ds.listDiags(
            adapter.getModel(),
            result.diagnostics.map((diagnostic) => {
              const range = diagnosticRange(diagnostic, result.sourceFiles);
              const multiFile = result.sourceFiles.length > 1;
              return {
                code: diagnostic.code,
                category: diagnostic.category,
                messageText:
                  (multiFile && diagnostic.fileName ? diagnostic.fileName + ": " : "") +
                  diagnosticText(diagnostic),
                start: range?.start,
                length: range ? Math.max(1, range.end - range.start) : undefined,
                file: undefined,
              };
            }),
          );
          for (const diagnostic of result.diagnostics)
            for (const related of diagnostic.relatedInformation ?? [])
              ds.p(`${related.fileName ?? ""}: ${diagnosticText(related)}`);
        });
      },
      didUnmount: () => {
        unsubscribe?.();
        ds.clearDeltaDecorators(true);
      },
    };
  };
}

/** Catch both synchronous and asynchronous failures at every third-party lifecycle boundary. */
export function guardPlugin(plugin: PlaygroundPlugin): PlaygroundPlugin {
  const guarded = { ...plugin };
  for (const key of [
    "willMount",
    "didMount",
    "modelChanged",
    "modelChangedDebounce",
    "willUnmount",
    "didUnmount",
  ] as const) {
    const callback = plugin[key];
    if (!callback) continue;
    const report = (error: unknown) => {
      const container = document.querySelector(".playground-plugin-container");
      const active = document
        .getElementById("playground-plugin-tab-" + plugin.id)
        ?.classList.contains("active");
      const target = active ? container : document.getElementById("plugin-status");
      if (target) target.textContent = `${plugin.displayName}: ${String(error)}`;
    };
    // Upstream lifecycle signatures differ only in their additional arguments.
    Object.assign(guarded, {
      [key]: (...args: unknown[]) => {
        try {
          Promise.resolve(Reflect.apply(callback, plugin, args)).catch(report);
        } catch (error) {
          report(error);
        }
      },
    });
  }
  return guarded;
}
