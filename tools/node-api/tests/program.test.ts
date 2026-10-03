// Programs, configuration and diagnostics through the sync client: config- and root-file-created programs,
// tsconfig option errors, `extends` through node_modules, package exports resolution, missing imports, and
// diagnostic chains/related information with positions after astral-plane and BMP non-ASCII text.

import { describe, test } from "node:test";
import { diags, golden, syncAPI } from "./utils.ts";

const g = golden("program");

describe("parity: programs and diagnostics", () => {
    test("UTF-16 positions after astral and BMP characters, chains and related info", () => {
        const source = [
            `const face = "😀😀 é ü 中文";`,
            `interface Props { cb: (x: string) => void }`,
            `const p: Props = { cb: (x: number) => {} }; // 𝒳`,
            `/* 🧪 */ const n: number = face;`,
            `let 𝑣ar = 1;`,
        ].join("\n");
        const { api } = syncAPI({ "/tsconfig.json": `{ "compilerOptions": { "strict": true } }`, "/src/index.ts": source });
        using _ = api;
        const program = api.createSnapshot({ openProject: "/tsconfig.json" }).getConfiguredProject("/tsconfig.json")!.program;
        g.check("utf16.syntactic", diags(program.getSyntacticDiagnostics("/src/index.ts")));
        g.check("utf16.semantic", diags(program.getSemanticDiagnostics("/src/index.ts")));
        g.check("utf16.text", program.getSourceFile("/src/index.ts")!.text === source);
    });

    test("config parsing errors, global and program diagnostics", () => {
        const { api } = syncAPI({
            "/tsconfig.json": `{
                "compilerOptions": { "target": "es2099", "strict": "yes", "notAnOption": true },
                "files": ["src/a.ts", "src/missing.ts"]
            }`,
            "/src/a.ts": `export const a = 1;`,
        });
        using _ = api;
        const program = api.createSnapshot({ openProject: "/tsconfig.json" }).getConfiguredProject("/tsconfig.json")!.program;
        g.check("config.parsing", diags(program.getConfigFileParsingDiagnostics()));
        g.check("config.program", diags(program.getProgramDiagnostics()));
        g.check("config.global", diags(program.getGlobalDiagnostics()));
        g.check("config.names", program.getSourceFileNames().filter(f => !f.includes("lib.")));
    });

    test("extends chain through node_modules and a relative base", () => {
        const { api } = syncAPI({
            "/node_modules/@cfg/base/package.json": `{ "name": "@cfg/base", "version": "1.0.0" }`,
            "/node_modules/@cfg/base/tsconfig.json": `{ "compilerOptions": { "noImplicitAny": true, "target": "es2020" } }`,
            "/configs/strict.json": `{ "extends": "@cfg/base/tsconfig.json", "compilerOptions": { "strictNullChecks": true } }`,
            "/tsconfig.json": `{ "extends": "./configs/strict.json", "include": ["src"] }`,
            "/src/index.ts": `export function f(x) { return x; }\nexport const s: string = null;`,
        });
        using _ = api;
        const program = api.createSnapshot({ openProject: "/tsconfig.json" }).getConfiguredProject("/tsconfig.json")!.program;
        g.check("extends.semantic", diags(program.getSemanticDiagnostics("/src/index.ts")));
        g.check("extends.configFiles", program.getConfigFileNames());
        g.check("extends.parsing", diags(program.getConfigFileParsingDiagnostics()));
    });

    test("node_modules package exports, types and missing modules", () => {
        const { api } = syncAPI({
            "/tsconfig.json": `{ "compilerOptions": { "module": "nodenext", "strict": true, "noEmit": true } }`,
            "/package.json": `{ "name": "app", "type": "module" }`,
            "/node_modules/pkg/package.json": `{ "name": "pkg", "type": "module", "exports": { ".": { "types": "./dist/index.d.ts", "default": "./dist/index.js" }, "./sub": { "types": "./dist/sub.d.ts" } } }`,
            "/node_modules/pkg/dist/index.d.ts": `export declare const v: { readonly tag: "pkg" };`,
            "/node_modules/pkg/dist/sub.d.ts": `export declare function sub(n: number): string;`,
            "/src/index.ts": [
                `import { v } from "pkg";`,
                `import { sub } from "pkg/sub";`,
                `import { nope } from "pkg/missing";`,
                `import { gone } from "./gone.js";`,
                `export const t: "pkg" = v.tag;`,
                `export const r: number = sub(1);`,
            ].join("\n"),
        });
        using _ = api;
        const program = api.createSnapshot({ openProject: "/tsconfig.json" }).getConfiguredProject("/tsconfig.json")!.program;
        g.check("modules.semantic", diags(program.getSemanticDiagnostics("/src/index.ts")));
        g.check("modules.names", program.getSourceFileNames().filter(f => !f.includes("lib.")).sort());
        g.check("modules.external", program.isSourceFileFromExternalLibrary("/node_modules/pkg/dist/index.d.ts"));
    });

    test("root-file program created without a tsconfig", () => {
        const { api } = syncAPI({
            "/a.ts": `export const a: number = "no";`,
            "/b.ts": `import { a } from "./a"; a.toFixed(); const c = a.nope;`,
        });
        using _ = api;
        const program = api.createProgram(["/b.ts"], { strict: true, noEmit: true });
        g.check("roots.names", program.getSourceFileNames().filter(f => !f.includes("lib.")).sort());
        g.check("roots.semantic.a", diags(program.getSemanticDiagnostics("/a.ts")));
        g.check("roots.semantic.b", diags(program.getSemanticDiagnostics("/b.ts")));
        program.dispose();
    });
});
