// Emit through the API, following compiler options (no environment opt-in): noEmit, noEmitOnError,
// emitDeclarationOnly, allowJs with declarations from JS, source and declaration maps, d.ts-only inputs, and the
// string-returning emit methods. Outputs are captured as the API returns them and as written via the host fs.

import { EmitOnly } from "@typescript/typescript/unstable/sync";
import { describe, test } from "node:test";
import { diags, golden, sortedWrites, syncAPI } from "./utils.ts";

const g = golden("emit");

function project(files: Record<string, string>) {
    const ctx = syncAPI(files);
    const program = ctx.api.createSnapshot({ openProject: "/tsconfig.json" }).getConfiguredProject("/tsconfig.json")!.program;
    return { ...ctx, program };
}

function emitResult(r: { diagnostics: readonly any[]; emitSkipped: boolean; emittedFiles?: readonly string[]; fileSystem?: unknown; }) {
    return { ...r, diagnostics: diags(r.diagnostics) };
}

const src = {
    "/src/index.ts": `import { helper } from "./helper";\nexport const value: number = helper(2);\nexport class Box<T> { constructor(public item: T) {} }\n`,
    "/src/helper.ts": `/** doubles */\nexport function helper(n: number) { return n * 2; }\n`,
};

describe("parity: emit", () => {
    test("noEmit produces no outputs", () => {
        const { api, program, writes } = project({ ...src, "/tsconfig.json": `{ "compilerOptions": { "noEmit": true, "outDir": "dist", "rootDir": "src" } }` });
        using _ = api;
        g.check("noEmit.result", emitResult(program.emit()));
        g.check("noEmit.writes", sortedWrites(writes));
    });

    test("noEmitOnError skips emit when there are errors", () => {
        const { api, program, writes } = project({
            ...src,
            "/src/bad.ts": `export const bad: string = 1;\n`,
            "/tsconfig.json": `{ "compilerOptions": { "noEmitOnError": true, "outDir": "dist", "rootDir": "src", "declaration": true } }`,
        });
        using _ = api;
        g.check("noEmitOnError.result", emitResult(program.emit()));
        g.check("noEmitOnError.writes", sortedWrites(writes));
    });

    test("emitDeclarationOnly with declaration maps", () => {
        const { api, program, writes } = project({ ...src, "/tsconfig.json": `{ "compilerOptions": { "declaration": true, "emitDeclarationOnly": true, "declarationMap": true, "outDir": "dist", "rootDir": "src" } }` });
        using _ = api;
        g.check("declOnly.result", emitResult(program.emit()));
        g.check("declOnly.writes", sortedWrites(writes));
    });

    test("allowJs with declarations generated from JavaScript", () => {
        const { api, program, writes } = project({
            "/src/lib.js": `/**\n * @param {string} s\n * @returns {number}\n */\nexport function len(s) { return s.length; }\nexport const answer = 42;\n`,
            "/src/main.ts": `import { len, answer } from "./lib.js";\nexport const n = len("abc") + answer;\n`,
            "/tsconfig.json": `{ "compilerOptions": { "allowJs": true, "checkJs": true, "declaration": true, "outDir": "dist", "rootDir": "src", "module": "esnext", "target": "es2022" } }`,
        });
        using _ = api;
        g.check("allowJs.result", emitResult(program.emit()));
        g.check("allowJs.writes", sortedWrites(writes));
    });

    test("source maps with inline sources and downlevel target", () => {
        const { api, program, writes } = project({ ...src, "/tsconfig.json": `{ "compilerOptions": { "sourceMap": true, "inlineSources": true, "target": "es2015", "module": "commonjs", "outDir": "dist", "rootDir": "src" } }` });
        using _ = api;
        g.check("sourceMap.result", emitResult(program.emit()));
        g.check("sourceMap.writes", sortedWrites(writes));
    });

    test("d.ts inputs produce no outputs", () => {
        const { api, program, writes } = project({
            "/src/types.d.ts": `declare const ambient: number;\n`,
            "/src/use.ts": `export const x = ambient + 1;\n`,
            "/tsconfig.json": `{ "compilerOptions": { "declaration": true, "outDir": "dist", "rootDir": "src" } }`,
        });
        using _ = api;
        g.check("dtsInput.result", emitResult(program.emit()));
        g.check("dtsInput.writes", sortedWrites(writes));
    });

    test("emitOnly variants and string-returning emit", () => {
        const { api, program, writes } = project({ ...src, "/tsconfig.json": `{ "compilerOptions": { "declaration": true, "sourceMap": true, "outDir": "dist", "rootDir": "src" } }` });
        using _ = api;
        g.check("toString.all", program.emitToString());
        g.check("toString.js", program.emitToString(EmitOnly.OnlyJs));
        g.check("toString.dts", program.emitToString(EmitOnly.OnlyDts));
        g.check("getJavaScriptEmit", program.getJavaScriptEmit(["/src/index.ts"]));
        g.check("getDeclarationEmit", program.getDeclarationEmit(["/src/helper.ts"]));
        g.check("toString.writes", sortedWrites(writes));
    });
});
