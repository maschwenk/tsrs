// Methods no upstream test sends (inventory "unexercised" on the pinned oracle): getExportSymbolOfSymbol,
// getOuterTypeParametersOfType, getConstraintOfType, getCurrentLanguageServerSnapshot and the profiling methods.
// Each is driven through the pinned client's public surface; outcomes are oracle-recorded.

import { API } from "@typescript/typescript/unstable/sync";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, test } from "node:test";
import { createVirtualFileSystem } from "../testUtils.ts";
import { cwd, golden, softGolden } from "./utils.ts";

const g = golden("gaps");
const soft = softGolden("gaps");

const SymbolFlagsValue = 111551; // SymbolFlags.Value at the pin
const source = [
    `export function exported() { return 1; }`,
    `export function outer<T extends object>(t: T) {`,
    `    class Inner<U> { t!: T; u!: U; }`,
    `    return new Inner<number>();`,
    `}`,
    `export type Pick1<T> = T extends string ? { x: T } : never;`,
    `const use = exported();`,
].join("\n") + "\n";

describe("parity: methods without upstream coverage", () => {
    test("getExportSymbolOfSymbol, getOuterTypeParametersOfType, getConstraintOfType", () => {
        using api = new API({ cwd, fs: createVirtualFileSystem({ "/tsconfig.json": `{ "compilerOptions": { "strict": true } }`, "/src/index.ts": source }) });
        const project = api.createSnapshot({ openProject: "/tsconfig.json" }).getConfiguredProject("/tsconfig.json")!;
        const { checker, program } = project;
        const file = program.getSourceFile("/src/index.ts")!;

        // A module-local symbol of an exported function carries an export symbol.
        const useNode = (file as any).statements[3];
        const local = checker.getSymbolsInScope(useNode, SymbolFlagsValue).find(s => s.name === "exported");
        const exportSym = local?.getExportSymbol();
        g.check("exportSymbol", { local: local?.name ?? null, exportName: exportSym?.name ?? null, distinct: exportSym !== undefined && exportSym !== local });

        // Class declared inside a generic function: its declared type has outer type parameter T.
        const innerSym = checker.getSymbolAtPosition("/src/index.ts", source.indexOf("Inner"))!;
        const innerType = checker.getDeclaredTypeOfSymbol(innerSym)!;
        g.check("outerTypeParameters", innerType.getOuterTypeParameters().map(t => checker.typeToString(t)));

        // Inside the true branch of `T extends string ? ...`, T is a substitution type whose constraint is string.
        const xSym = checker.getSymbolAtPosition("/src/index.ts", source.indexOf("x: T"))!;
        const xType = checker.getTypeOfSymbol(xSym)!;
        const constraint = xType.getConstraint();
        g.check("substitutionConstraint", { flags: xType.flags, type: checker.typeToString(xType), constraint: constraint ? checker.typeToString(constraint) : null });
    });

    test("getCurrentLanguageServerSnapshot without a language server connection", t => {
        using api = new API({ cwd, fs: createVirtualFileSystem({ "/tsconfig.json": "{}", "/a.ts": "export {};" }) });
        let threw = false;
        let message: string | null = null;
        try {
            (api as any).getCurrentLanguageServerSnapshot();
        }
        catch (e) {
            threw = true;
            message = String((e as Error).message).split("\n")[0];
        }
        g.check("lsSnapshot.threw", threw);
        soft.check(t, "lsSnapshot.message", message);
        assert.ok(api.parseCommandLine(["--strict"]).options.strict, "server must keep serving");
    });

    test("CPU and heap profiling requests", t => {
        using api = new API({ cwd, fs: createVirtualFileSystem({ "/tsconfig.json": "{}", "/a.ts": "export const a = 1;" }) });
        const dir = fs.mkdtempSync(path.join(os.tmpdir(), "node-api-profile-"));
        const outcome = (fn: () => unknown) => {
            try {
                const value = fn();
                return { ok: true, value: typeof value === "string" ? path.relative(dir, value).replace(/\\/g, "/").replace(/\d+/g, "N") : value ?? null };
            }
            catch (e) {
                return { ok: false, error: String((e as Error).message).split("\n")[0] };
            }
        };
        try {
            const start = outcome(() => api.internal.startCPUProfile(dir));
            api.createSnapshot({ openProject: "/tsconfig.json" });
            const stop = outcome(() => api.internal.stopCPUProfile());
            const heap = outcome(() => api.internal.saveHeapProfile(dir));
            const written = fs.readdirSync(dir).map(f => f.replace(/\d+/g, "N")).sort();
            soft.check(t, "profiling", { start, stop, heap, written });
            g.check("profiling.serverSurvives", api.parseCommandLine(["--noEmit"]).options.noEmit === true);
        }
        finally {
            fs.rmSync(dir, { recursive: true, force: true });
        }
    });
});
