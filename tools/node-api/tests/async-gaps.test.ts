// Async-binding coverage for methods that the upstream async suite never answers successfully (found by the
// paired-response comparison): batched position/location/symbol queries, module resolver release, substitution
// and class type sub-properties, export symbols, widened/parameter types, array-likeness, signature-to-declaration
// and references. Values are oracle-recorded; captured runs also pair these responses method by method.

import { API } from "@typescript/typescript/unstable/async";
import { describe, test } from "node:test";
import { createVirtualFileSystem } from "../testUtils.ts";
import { cwd, golden } from "./utils.ts";

const g = golden("async-gaps");
const SymbolFlagsValue = 111551;
const source = [
    `export function exported(n: number, s: string) { return [n, s] as const; }`,
    `export function outer<T extends object>(t: T) {`,
    `    class Inner<U> { t!: T; u!: U; }`,
    `    return new Inner<number>();`,
    `}`,
    `export type Pick1<T> = T extends string ? { x: T } : never;`,
    `const use = exported(1, "a");`,
    `const lit = 1;`,
    `const arr = [1, 2];`,
    `use; exported; lit;`,
].join("\n") + "\n";

describe("parity: async binding coverage", () => {
    test("checker queries, sub-properties and resolver release through the async client", async () => {
        const api = new API({ cwd, fs: createVirtualFileSystem({ "/tsconfig.json": `{ "compilerOptions": { "strict": true } }`, "/src/index.ts": source }) });
        try {
            const snapshot = await api.createSnapshot({ openProject: "/tsconfig.json" });
            const project = snapshot.getConfiguredProject("/tsconfig.json")!;
            const { checker, program } = project;
            const file = (await program.getSourceFile("/src/index.ts"))!;
            const at = (needle: string, from = 0) => source.indexOf(needle, from);
            const name = (s: any) => s?.name ?? null;

            const syms = await checker.getSymbolAtPosition("/src/index.ts", [at("exported"), at("Inner"), at("lit"), at("arr")]);
            g.check("symbolsAtPositions", syms.map(name));
            const statements = (file as any).statements;
            const declNames = [statements[3], statements[4], statements[5]].map((s: any) => s.declarationList.declarations[0].name);
            g.check("symbolsAtLocations", (await checker.getSymbolAtLocation(declNames)).map(name));
            const types = await checker.getTypeOfSymbol(syms.filter(Boolean) as any);
            g.check("typesOfSymbols", await Promise.all(types.map(t => checker.typeToString(t))));
            g.check("typeAtLocations", await Promise.all((await checker.getTypeAtLocation(declNames)).map(t => checker.typeToString(t))));

            const local = (await checker.getSymbolsInScope(statements[3], SymbolFlagsValue)).find(s => s.name === "exported" && (s as any).exportSymbol);
            g.check("exportSymbol", { local: name(local), export: name(await local?.getExportSymbol()) });

            const inner = await checker.getDeclaredTypeOfSymbol(syms[1]!);
            g.check("outerTypeParameters", await Promise.all((await inner.getOuterTypeParameters()).map(t => checker.typeToString(t))));
            g.check("localTypeParameters", await Promise.all((await inner.getLocalTypeParameters()).map(t => checker.typeToString(t))));

            const xSym = (await checker.getSymbolAtPosition("/src/index.ts", at("x: T")))!;
            const subst = await checker.getTypeOfSymbol(xSym);
            g.check("substitution", { type: await checker.typeToString(subst), base: await checker.typeToString(await subst.getBaseType()), constraint: await checker.typeToString((await subst.getConstraint())!) });

            const litType = await checker.getTypeOfSymbol(syms[2]!);
            g.check("widened", await checker.typeToString(await checker.getWidenedType(litType)));
            const arrType = await checker.getTypeOfSymbol(syms[3]!);
            g.check("arrayLike", [await checker.isArrayLikeType(arrType), await checker.isArrayLikeType(litType)]);

            const fnType = await checker.getTypeOfSymbol(syms[0]!);
            const [sig] = await checker.getSignaturesOfType(fnType, 0);
            g.check("parameterTypes", [await checker.typeToString(await checker.getParameterType(sig, 0)), await checker.typeToString(await checker.getParameterType(sig, 1))]);
            const decl = await checker.signatureToSignatureDeclaration(sig, 263 /* SyntaxKind.FunctionDeclaration at the pin */);
            g.check("signatureDeclaration", decl ? { kind: decl.kind, parameters: (decl as any).parameters?.length ?? null } : null);

            g.check("referencesInFile", (await checker.getReferencesToSymbolInFile("/src/index.ts", syms[0]!)).map(h => [h.index, h.kind, h.path]));

            const resolver = await api.createModuleResolver({ moduleResolution: 100 as any });
            await resolver.dispose();
            g.check("resolverReleased", true);
        }
        finally {
            await api.close();
        }
    });
});
