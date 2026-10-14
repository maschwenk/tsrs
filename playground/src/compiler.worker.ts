import { tsc } from "@tsrs/wasm";
import wasmURL from "@tsrs/wasm/tsrs.wasm?url";
import { createProject, type Compilation, type CompileRequest } from "./project";

self.onmessage = async ({ data }: MessageEvent<CompileRequest>) => {
  let sourceFiles: Compilation["sourceFiles"] = [];
  try {
    const project = createProject(data);
    sourceFiles = project.sourceFiles;
    const result = await tsc(["-p", "/tsconfig.json", "--pretty", "false"], {
      cwd: "/",
      files: project.files,
      diagnostics: "json",
      url: wasmURL,
    });
    const response: Compilation = {
      revision: data.revision,
      sourceFiles,
      diagnostics: result.exitCode === 5 ? [] : (result.diagnostics ?? []),
      files: result.exitCode === 5 ? {} : (result.files ?? {}),
      exitCode: result.exitCode,
      stderr: result.stderr,
      ...(result.exitCode === 5
        ? { error: result.stderr || "The compiler stopped unexpectedly." }
        : {}),
    };
    self.postMessage(response);
  } catch (error) {
    self.postMessage({
      revision: data.revision,
      sourceFiles,
      diagnostics: [],
      files: {},
      exitCode: 5,
      stderr: "",
      error: String(error),
    } satisfies Compilation);
  }
};
