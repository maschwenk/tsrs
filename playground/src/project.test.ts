import { expect, test } from "vite-plus/test";
import {
  createProject,
  diagnosticRange,
  jsonCompilerOptions,
  optionsFromURL,
  splitSource,
  compilerDefaults,
} from "./project";

test("repeated contents and CRLF preserve exact UTF-16 editor ranges", () => {
  const content = 'const emoji = "😀"; const value: string = 1;\r\n';
  const source = "// @filename: a.ts\r\n" + content + "// @filename: b.ts\r\n" + content;
  const files = splitSource(source, "input.ts");
  expect(files.map((file) => file.name)).toEqual(["/a.ts", "/b.ts"]);
  const pos = content.indexOf("value");
  expect(
    diagnosticRange(
      { fileName: "/b.ts", pos, end: pos + 5, code: 2322, category: 1, text: "error" },
      files,
    ),
  ).toEqual({ start: source.lastIndexOf("value"), end: source.lastIndexOf("value") + 5 });
  expect(
    diagnosticRange({ pos: 0, end: 0, code: 5102, category: 1, text: "option error" }, files),
  ).toBeUndefined();
});

test("duplicate normalized virtual paths cannot silently overwrite files", () => {
  expect(() =>
    splitSource("// @filename: x/../a.ts\nexport {};\n// @filename: a.ts\nexport {};", "input.ts"),
  ).toThrow("Duplicate virtual file");
});

test("shared numeric and string compiler options become valid JSON compiler settings", () => {
  const options = optionsFromURL(
    new URLSearchParams(
      "target=9&module=esnext&moduleResolution=bundler&lib=es2022,dom&strict=false&filetype=ts&ts=next",
    ),
  );
  expect(jsonCompilerOptions({ ...compilerDefaults, ...options })).toMatchObject({
    target: "es2022",
    module: "esnext",
    moduleResolution: "bundler",
    lib: ["es2022", "dom"],
    strict: false,
  });
  expect(options).not.toHaveProperty("filetype");
  expect(options).not.toHaveProperty("ts");
});

test("a preamble does not collide with an explicitly named default file", () => {
  const files = splitSource(
    "// shared comment\n\n// @filename: input.ts\nconst value = 1;",
    "input.ts",
  );
  expect(files.map((file) => file.name)).toEqual(["/global.ts", "/input.ts"]);
  expect(files[0].text).toBe("// shared comment\n\n");
});

test("package and JSON files are available to the compiler but are not TS root files", () => {
  const project = createProject({
    revision: 1,
    source:
      '// @filename: package.json\n{"type":"module"}\n// @filename: data.json\n{"value":1}\n// @filename: index.ts\nexport const value = 1;\n',
    fileName: "input.ts",
    compilerOptions: compilerDefaults,
    libraries: {},
  });
  expect(JSON.parse(project.files["/tsconfig.json"]).files).toEqual(["/index.ts"]);
  expect(project.files["/package.json"]).toContain('"type":"module"');
});
