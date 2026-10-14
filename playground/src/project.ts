import metadata from "./generated/compiler.json";
import type { Diagnostic } from "@tsrs/wasm";

export type CompilerOptions = Record<string, unknown>;
export interface SourceFile {
  name: string;
  text: string;
  /** Offset of this virtual file's first character in the editor, in UTF-16 units. */
  offset: number;
}
export interface Compilation {
  revision: number;
  diagnostics: Diagnostic[];
  files: Record<string, string>;
  sourceFiles: SourceFile[];
  exitCode: number;
  stderr: string;
  error?: string;
}
export interface CompileRequest {
  revision: number;
  source: string;
  fileName: string;
  compilerOptions: CompilerOptions;
  libraries: Record<string, string>;
}

export const compilerDefaults: CompilerOptions = {
  strict: true,
  target: metadata.ScriptTarget.ES2022,
  module: metadata.ModuleKind.ESNext,
  moduleResolution: metadata.ModuleResolutionKind.Bundler,
  jsx: metadata.JsxEmit.None,
  declaration: true,
  outDir: "out",
};

export function normalizeFileName(name: string): string {
  const parts: string[] = [];
  for (const part of name.replaceAll("\\", "/").split("/")) {
    if (part === "..") parts.pop();
    else if (part && part !== ".") parts.push(part);
  }
  return "/" + parts.join("/");
}

/** Split by offsets, rather than searching for contents, so identical files map correctly. */
export function splitSource(source: string, fileName: string): SourceFile[] {
  const files: SourceFile[] = [];
  const directives = [...source.matchAll(/^[ \t]*\/\/ @filename: ([^\r\n]+)(?:\r\n|\r|\n|$)/gm)];
  const defaultName = normalizeFileName(fileName);
  let name = directives.some((directive) => normalizeFileName(directive[1].trim()) === defaultName)
    ? "/global.ts"
    : defaultName;
  let offset = 0;
  for (const directive of directives) {
    const end = directive.index;
    if (end > offset && source.slice(offset, end).trim())
      files.push({ name, text: source.slice(offset, end), offset });
    name = normalizeFileName(directive[1].trim());
    offset = end + directive[0].length;
  }
  if (offset < source.length || !files.length)
    files.push({ name, text: source.slice(offset), offset });
  const seen = new Set<string>();
  for (const file of files) {
    if (seen.has(file.name)) throw new Error(`Duplicate virtual file: ${file.name}`);
    seen.add(file.name);
  }
  return files;
}

/** Monaco's numeric enums become the string names expected by a real tsconfig.json. */
export function jsonCompilerOptions(options: CompilerOptions): CompilerOptions {
  const result: CompilerOptions = { ...options };
  for (const [key, value] of Object.entries(result)) {
    const option = metadata.allOptions.find((option) => option.name === key);
    if (typeof value === "number" && option && typeof option.type === "object") {
      result[key] =
        Object.entries(option.type).find(([, number]) => number === value)?.[0] ?? value;
    }
  }
  // JSX None is a Monaco editor setting, not a valid compiler setting.
  if (result.jsx === 0) delete result.jsx;
  return result;
}

export function createProject(request: CompileRequest) {
  const sourceFiles = splitSource(request.source, request.fileName);
  const files: Record<string, string> = { ...request.libraries };
  for (const file of sourceFiles) files[file.name] = file.text;
  const rootNames = sourceFiles
    .filter((file) => /\.(?:[cm]?[jt]sx?)$/.test(file.name))
    .map((file) => file.name);
  // Node-style emit follows the virtual package.json, just as a normal compiler invocation does.
  const moduleName = String(jsonCompilerOptions(request.compilerOptions).module).toLowerCase();
  if (!files["/package.json"] && /^(node16|node18|node20|nodenext)$/.test(moduleName))
    files["/package.json"] = '{"type":"module"}';
  // A user-supplied tsconfig is honored; otherwise the UI settings define the project.
  files["/tsconfig.json"] ??= JSON.stringify({
    compilerOptions: jsonCompilerOptions(request.compilerOptions),
    files: rootNames,
  });
  return { files, sourceFiles };
}

export function diagnosticRange(diagnostic: Diagnostic, files: SourceFile[]) {
  const file = files.find(
    (file) => diagnostic.fileName && file.name === normalizeFileName(diagnostic.fileName),
  );
  if (!file) return undefined;
  const start = Math.max(0, Math.min(diagnostic.pos, file.text.length));
  const end = Math.max(start, Math.min(diagnostic.end, file.text.length));
  return { start: file.offset + start, end: file.offset + end };
}

export function diagnosticText(diagnostic: Diagnostic): string {
  const children = diagnostic.messageChain?.map(diagnosticText) ?? [];
  return [diagnostic.text, ...children.map((text) => "  " + text)].filter(Boolean).join("\n");
}

export function optionsFromURL(params: URLSearchParams): CompilerOptions {
  const options: CompilerOptions = {};
  for (const [name, value] of params) {
    const option = metadata.allOptions.find((option) => option.name === name);
    if (!option) continue;
    if (option.type === "boolean") options[name] = value === "true";
    else if (typeof option.type === "object")
      options[name] = /^\d+$/.test(value)
        ? Number(value)
        : (Object.entries(option.type).find(([key]) => key === value.toLowerCase())?.[1] ?? value);
    else if (option.type === "list") options[name] = value.split(",");
    else if (option.type === "number") options[name] = Number(value);
    else options[name] = value;
  }
  return options;
}
