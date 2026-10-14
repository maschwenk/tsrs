import type * as Monaco from "monaco-editor";
import type { Sandbox } from "@typescript/sandbox";
import lzstring from "lz-string";
import metadata from "./generated/compiler.json";
import { getInitialCode } from "../vendor/packages/sandbox/src/getInitialCode";
import { CompilerClient } from "./compiler-client";
import {
  compilerDefaults,
  diagnosticRange,
  diagnosticText,
  optionsFromURL,
  type CompilerOptions,
} from "./project";

export const unsupported = (feature: string): never => {
  throw new Error(
    `${feature} is unavailable in the tsrs browser compiler. The WASM API provides checking and emit, without a language service.`,
  );
};

import { playCopy } from "../vendor/packages/typescriptlang-org/src/copy/en/playground";
const example = playCopy.play_default_code_sample;

export function createSandbox(monaco: typeof Monaco) {
  const params = new URLSearchParams(location.search);
  const filetype =
    params.get("filetype") === "js" || params.get("useJavaScript")
      ? "js"
      : params.get("filetype") === "d.ts"
        ? "d.ts"
        : "ts";
  const defaults = {
    ...compilerDefaults,
    ...(filetype === "js" ? { allowJs: true, checkJs: true } : {}),
  };
  let options: CompilerOptions = { ...defaults, ...optionsFromURL(params) };
  const fileName = () =>
    "/input." +
    (filetype === "d.ts"
      ? "d.ts"
      : filetype +
        ((typeof options.jsx === "number" && options.jsx !== 0) ||
        (typeof options.jsx === "string" && options.jsx.toLowerCase() !== "none")
          ? "x"
          : ""));
  let text = example;
  try {
    text = getInitialCode(localStorage.getItem("sandbox-history") || example, location) ?? example;
  } catch {
    /* A malformed share link falls back to the example. */
  }
  const model = monaco.editor.createModel(
    text,
    filetype === "js" ? "javascript" : "typescript",
    monaco.Uri.file(fileName()),
  );
  model.updateOptions({ bracketColorizationOptions: { enabled: true } });
  const editor = monaco.editor.create(document.getElementById("monaco-editor-embed")!, {
    model,
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastColumn: 3,
    fontFamily: "var(--code-font)",
    fontLigatures: true,
    bracketPairColorization: { enabled: true },
    quickSuggestions: false,
    parameterHints: { enabled: false },
    suggestOnTriggerCharacters: false,
    wordBasedSuggestions: false,
  });
  const selection = ["ssl", "ssc", "pln", "pc"].map((key) => Number(params.get(key)));
  if (selection.every((value) => value > 0))
    editor.setSelection(
      new monaco.Selection(selection[0], selection[1], selection[2], selection[3]),
    );

  const compiler = new CompilerClient(
    () => new Worker(new URL("./compiler.worker.ts", import.meta.url), { type: "module" }),
  );
  const libraries: Record<string, string> = {};
  let didUpdate = (_options: CompilerOptions) => {};
  const status = document.getElementById("compiler-status")!;
  const compile = (delay = 0) => {
    status.textContent = "Checking…";
    compiler.request(
      {
        source: model.getValue(),
        fileName: fileName(),
        compilerOptions: options,
        libraries: { ...libraries },
      },
      delay,
    );
  };
  const schedule = () => {
    monaco.editor.setModelMarkers(model, "tsrs", []);
    compile(300);
  };
  model.onDidChangeContent(schedule);
  const ready = () => {
    return compiler.ready();
  };
  compiler.subscribe((result) => {
    status.textContent = result.error
      ? "Compiler failed — Retry"
      : `${result.diagnostics.length} diagnostic${result.diagnostics.length === 1 ? "" : "s"}`;
    status.title = result.error ?? result.stderr;
    const severity = [
      monaco.MarkerSeverity.Warning,
      monaco.MarkerSeverity.Error,
      monaco.MarkerSeverity.Hint,
      monaco.MarkerSeverity.Info,
    ];
    monaco.editor.setModelMarkers(
      model,
      "tsrs",
      result.error
        ? []
        : result.diagnostics.flatMap((diagnostic) => {
            const range = diagnosticRange(diagnostic, result.sourceFiles);
            if (!range) return [];
            const start = model.getPositionAt(range.start);
            const end = model.getPositionAt(Math.max(range.start + 1, range.end));
            const relatedInformation = diagnostic.relatedInformation?.flatMap((related) => {
              const relatedRange = diagnosticRange(related, result.sourceFiles);
              if (!relatedRange) return [];
              const relatedStart = model.getPositionAt(relatedRange.start);
              const relatedEnd = model.getPositionAt(relatedRange.end);
              return [
                {
                  resource: model.uri,
                  message: diagnosticText(related),
                  startLineNumber: relatedStart.lineNumber,
                  startColumn: relatedStart.column,
                  endLineNumber: relatedEnd.lineNumber,
                  endColumn: relatedEnd.column,
                },
              ];
            });
            return [
              {
                startLineNumber: start.lineNumber,
                startColumn: start.column,
                endLineNumber: end.lineNumber,
                endColumn: end.column,
                severity: severity[diagnostic.category],
                code: String(diagnostic.code),
                message: diagnosticText(diagnostic),
                source: "tsrs",
                relatedInformation,
                tags: diagnostic.reportsUnnecessary
                  ? [monaco.MarkerTag.Unnecessary]
                  : diagnostic.reportsDeprecated
                    ? [monaco.MarkerTag.Deprecated]
                    : [],
              },
            ];
          }),
    );
  });
  status.onclick = () => compile();

  const setCompilerSettings = (next: CompilerOptions) => {
    options = { ...next };
    schedule();
    didUpdate(options);
  };
  const createURLQueryWithCompilerOptions = (
    _sandbox?: unknown,
    overrides: Record<string, unknown> = {},
  ) => {
    const query = new URLSearchParams(location.search);
    query.delete("install-plugin");
    for (const name of Object.keys(defaults)) query.delete(name);
    for (const [name, value] of Object.entries(options)) {
      if (value !== defaults[name as keyof typeof defaults])
        query.set(name, Array.isArray(value) ? value.join(",") : String(value));
      else query.delete(name);
    }
    if (filetype !== "ts") query.set("filetype", filetype);
    for (const [name, value] of Object.entries(overrides)) query.set(name, String(value));
    const selected = editor.getSelection();
    for (const key of ["ssl", "ssc", "pln", "pc"]) query.delete(key);
    if (selected && !selected.isEmpty()) {
      [
        selected.selectionStartLineNumber,
        selected.selectionStartColumn,
        selected.positionLineNumber,
        selected.positionColumn,
      ].forEach((value, index) => query.set(["ssl", "ssc", "pln", "pc"][index], String(value)));
    }
    return (
      (query.size ? "?" + query.toString() : "") +
      "#code/" +
      lzstring.compressToEncodedURIComponent(model.getValue())
    );
  };

  // Metadata and message formatting satisfy the UI without loading the JS compiler.
  const ts = new Proxy(
    {
      ...metadata,
      flattenDiagnosticMessageText: (message: unknown) =>
        typeof message === "string" ? message : JSON.stringify(message),
    },
    {
      get(target, key) {
        if (key === "then") return undefined;
        if (key in target) return Reflect.get(target, key);
        return unsupported(`TypeScript API ${String(key)}`);
      },
    },
  );
  const adapter = {
    config: { filetype, logger: console, acquireTypes: false },
    ts,
    monaco,
    editor,
    lzstring,
    compilerDefaults: defaults,
    supportedVersions: [],
    language: filetype === "js" ? "javascript" : "typescript",
    get filepath() {
      return fileName().slice(1);
    },
    getModel: () => model,
    getDomNode: () => editor.getDomNode()!,
    getText: () => model.getValue(),
    setText: (value: string) => model.setValue(value),
    getCompilerOptions: () => ({ ...options }),
    setCompilerSettings,
    updateCompilerSettings: (next: CompilerOptions) => setCompilerSettings({ ...options, ...next }),
    updateCompilerSetting: (name: string, value: unknown) =>
      setCompilerSettings({ ...options, [name]: value }),
    setDidUpdateCompilerSettings: (callback: typeof didUpdate) => {
      didUpdate = callback;
    },
    createURLQueryWithCompilerOptions,
    addLibraryToRuntime: (code: string, path: string) => {
      libraries[path] = code;
      schedule();
    },
    removeLibraryFromRuntime: (path: string) => {
      delete libraries[path];
      schedule();
    },
    getEmitResult: async () => ({
      outputFiles: Object.entries((await ready()).files).map(([name, text]) => ({
        name,
        text,
        writeByteOrderMark: text.startsWith("\uFEFF"),
      })),
      emitSkipped: false,
    }),
    getRunnableJS: async () => {
      const result = await ready();
      if (result.error) throw new Error(result.error);
      const js = Object.entries(result.files).find(([name]) => /\.(?:[cm]?js|jsx)$/.test(name));
      if (!js)
        throw new Error("No JavaScript was emitted. Check the compiler settings and errors.");
      return js[1];
    },
    getDTSForCode: async () =>
      Object.entries((await ready()).files).find(([name]) => /\.d\.[cm]?ts$/.test(name))?.[1] ?? "",
    getWorkerProcess: async () => unsupported("Language-service plugins"),
    createTSProgram: async () => unsupported("Type inspection"),
    getAST: async () => unsupported("AST inspection"),
    setupTSVFS: async () => unsupported("TypeScript compiler programs"),
    getTwoSlashCompilerOptions: () => ({}),
    languageServiceDefaults: {
      setCompilerOptions: setCompilerSettings,
      getCompilerOptions: () => options,
    },
    compiler,
    ready,
    retry: compile,
    dispose: () => {
      compiler.dispose();
      editor.dispose();
      model.dispose();
    },
  };
  compile();
  return { sandbox: adapter as unknown as Sandbox, adapter };
}

export type SandboxAdapter = ReturnType<typeof createSandbox>["adapter"];
