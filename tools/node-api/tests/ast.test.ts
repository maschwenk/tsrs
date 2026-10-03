// AST and checker edge cases centred on the wire/JS position and string boundary (UTF-8/WTF-8 on the wire,
// UTF-16 in JS): full forEachChild traversals of TSX with astral identifiers and of string escapes that produce
// lone surrogates, a source file whose text itself contains a lone surrogate, deep nesting, and checker
// queries addressed by UTF-16 positions after astral text, overloads and JSDoc-typed JavaScript.

import type { Node, SourceFile } from "@typescript/typescript/unstable/ast";
import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { golden, syncAPI } from "./utils.ts";

const g = golden("ast");

function walk(file: SourceFile) {
    const out: unknown[] = [];
    const visit = (node: Node, depth: number) => {
        const text = (node as any).text;
        out.push([node.kind, node.pos, node.end, depth, typeof text === "string" && depth > 0 ? text : null]);
        node.forEachChild(child => {
            visit(child, depth + 1);
        });
    };
    visit(file, 0);
    return out;
}

function open(files: Record<string, string>, config = `{ "compilerOptions": { "strict": true, "jsx": "preserve", "allowJs": true, "checkJs": true } }`) {
    const ctx = syncAPI({ "/tsconfig.json": config, ...files });
    const project = ctx.api.createSnapshot({ openProject: "/tsconfig.json" }).getConfiguredProject("/tsconfig.json")!;
    return { ...ctx, program: project.program, checker: project.checker };
}

const tsx = [
    `// 😀 leading comment with astral text`,
    `const 𝑥 = "a😀b" as const;`,
    `const ü = \`t\${𝑥}😀\${1}\` ;`,
    `export function C(props: { 名前: string }) { return <div title="😀">{props.名前}{/* 💬 */}</div>; }`,
    `export const s = "\\uD800 lone \\uDFFF" + '\\u{1F600}';`,
].join("\n");

describe("parity: AST and checker at the UTF-16/WTF-8 boundary", () => {
    test("TSX traversal with astral identifiers, templates, JSX and escapes", () => {
        const { api, program } = open({ "/src/c.tsx": tsx });
        using _ = api;
        const file = program.getSourceFile("/src/c.tsx")!;
        g.check("tsx.textRoundTrip", file.text === tsx);
        g.check("tsx.walk", walk(file));
    });

    // Pinned Go defect (b85298b6): a readFile callback result containing a lone surrogate makes the server panic
    // ("jsontext: invalid surrogate pair") and exit, so there is no oracle value. The test asserts the minimum
    // acceptable behavior instead: the server survives, and the text either round-trips or the request fails
    // with an explicit error. It is todo so neither server is credited unless it actually passes.
    test("source text containing a lone surrogate", { todo: "pinned Go server panics on lone surrogates in callback file text" }, () => {
        const text = `export const raw = "x\uD83Dy";\nexport const after = 1;\n`;
        const { api, program } = open({ "/src/lone.ts": text });
        using _ = api;
        let outcome: string;
        try {
            outcome = program.getSourceFile("/src/lone.ts")!.text === text ? "round-trip" : "altered";
        }
        catch (e) {
            outcome = `error: ${(e as Error).message}`;
        }
        assert.doesNotMatch(outcome, /EOF|child process/, `server died: ${outcome}`);
        const fresh = api.createSnapshot({ openProject: "/tsconfig.json" });
        assert.ok(fresh.getConfiguredProject("/tsconfig.json"), "server no longer serves after the lone-surrogate file");
    });

    test("deeply nested expression encodes and traverses", () => {
        const depth = 400;
        const text = `export const deep = ${"(".repeat(depth)}1${")".repeat(depth)};\n`;
        const { api, program, checker } = open({ "/src/deep.ts": text });
        using _ = api;
        const file = program.getSourceFile("/src/deep.ts")!;
        let max = 0;
        const visit = (n: Node, d: number) => {
            max = Math.max(max, d);
            n.forEachChild(c => {
                visit(c, d + 1);
            });
        };
        visit(file, 0);
        g.check("deep.maxDepth", max);
        g.check("deep.type", checker.typeToString(checker.getTypeAtPosition("/src/deep.ts", text.indexOf("deep"))!));
    });

    test("checker queries by UTF-16 position after astral text", () => {
        const text = `const 😀 = 1; // not an identifier\nconst pre = "😀😀😀"; export const target = { k: pre, 𝑣: 2 as const };\nexport type Lit = "😀" | "é";\n`;
        const { api, checker } = open({ "/src/pos.ts": text });
        using _ = api;
        const at = (needle: string) => {
            const pos = text.indexOf(needle);
            const sym = checker.getSymbolAtPosition("/src/pos.ts", pos);
            const type = checker.getTypeAtPosition("/src/pos.ts", pos);
            return { needle, pos, symbol: sym?.name ?? null, type: type ? checker.typeToString(type) : null };
        };
        g.check("pos.queries", [at("target"), at("𝑣"), at("Lit"), at("pre,")]);
    });

    test("overload resolution and JSDoc-typed JavaScript", () => {
        const ts = `export function over(a: string): "s";\nexport function over(a: number): "n";\nexport function over(a: any): any { return a; }\nexport const r1 = over(1);\nexport const r2 = over("x");\n`;
        const js = `/** @type {Array<{ id: number }>} */\nexport const items = [];\n/** @param {string} s */\nexport function shout(s) { return s.toUpperCase(); }\nexport const loud = shout("a");\n`;
        const { api, checker } = open({ "/src/over.ts": ts, "/src/doc.js": js });
        using _ = api;
        const typeAt = (file: string, text: string, needle: string) => checker.typeToString(checker.getTypeAtPosition(file, text.indexOf(needle))!);
        g.check("overload.types", [typeAt("/src/over.ts", ts, "r1"), typeAt("/src/over.ts", ts, "r2")]);
        const sym = checker.getSymbolAtPosition("/src/over.ts", ts.indexOf("over"))!;
        const fnType = checker.getTypeOfSymbol(sym)!;
        g.check("overload.signatures", checker.getSignaturesOfType(fnType, 0).map(s => checker.typeToString(checker.getReturnTypeOfSignature(s)!)));
        g.check("jsdoc.types", [typeAt("/src/doc.js", js, "items"), typeAt("/src/doc.js", js, "shout"), typeAt("/src/doc.js", js, "loud")]);
    });
});
