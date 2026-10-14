import { expect, test } from "vite-plus/test";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  readdirSync,
  rmSync,
  existsSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { tsc } from "@ts-rs/wasm";
import { compilerDefaults, createProject } from "./project";

const native =
  process.env.TSRS_NATIVE ?? fileURLToPath(new URL("../../target/fast/tsrs", import.meta.url));
const fixtures = [
  { name: "TS", source: "export const value: string = 1;\n", fileName: "input.ts", options: {} },
  {
    name: "JS",
    source: "/** @type {string} */\nexport const value = 1;\n",
    fileName: "input.js",
    options: { allowJs: true, checkJs: true },
  },
  {
    name: "TSX",
    source:
      "declare namespace JSX { interface IntrinsicElements { div: {}; } }\nexport const element = <div/>;\n",
    fileName: "input.tsx",
    options: { jsx: "preserve" },
  },
  {
    name: "declarations",
    source: "export declare function f(value: string): number;\n",
    fileName: "input.d.ts",
    options: {},
  },
  {
    name: "relative import",
    source:
      '// @filename: a.ts\nexport const value = "😀";\n// @filename: b.ts\nimport { value } from "./a";\nexport const wrong: number = value;\n',
    fileName: "input.ts",
    options: {},
  },
];

test.each(fixtures)(
  "$name diagnostics and emitted bytes equal native tsrs",
  async ({ source, fileName, options }) => {
    const dir = mkdtempSync(path.join(tmpdir(), "tsrs-playground-"));
    try {
      const project = createProject({
        revision: 1,
        source,
        fileName,
        compilerOptions: { ...compilerDefaults, ...options, outDir: "out" },
        libraries: {},
      });
      const files: Record<string, string> = {};
      for (const [name, text] of Object.entries(project.files)) {
        const full = path.join(dir, name.slice(1));
        let content = text;
        if (name === "/tsconfig.json") {
          const config = JSON.parse(content);
          config.files = config.files.map((file: string) => file.slice(1));
          content = JSON.stringify(config);
        }
        files[full] = content;
        mkdirSync(path.dirname(full), { recursive: true });
        writeFileSync(full, content);
      }
      const result = await tsc(["-p", ".", "--pretty", "false"], { files, cwd: dir });
      const reference = spawnSync(native, ["-p", ".", "--singleThreaded", "--pretty", "false"], {
        cwd: dir,
        encoding: "utf8",
      });
      if (reference.error) throw reference.error;
      expect(result.exitCode).toBe(reference.status);
      expect(result.stdout).toBe(reference.stdout);
      const emitted: Record<string, string> = {};
      const collect = (folder: string) => {
        for (const entry of readdirSync(folder, { withFileTypes: true })) {
          const full = path.join(folder, entry.name);
          if (entry.isDirectory()) collect(full);
          else emitted[full] = readFileSync(full, "utf8");
        }
      };
      if (existsSync(path.join(dir, "out"))) collect(path.join(dir, "out"));
      expect(result.files).toEqual(emitted);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  },
);

test("JSON option errors are retained without an editor file location", async () => {
  const result = await tsc(["input.ts", "--strictt"], {
    cwd: "/",
    files: { "/input.ts": "const value = 1;" },
    diagnostics: "json",
  });
  expect(result.diagnostics).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ code: 5025, text: expect.stringContaining("strictt") }),
    ]),
  );
  expect(
    result.diagnostics?.find((diagnostic) => diagnostic.code === 5025)?.fileName,
  ).toBeUndefined();
});
