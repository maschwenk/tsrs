// getExportSymbolOfSymbol through the async binding. The public async API answers Symbol.getExportSymbol() from
// the client's symbol cache when the export symbol was already delivered (no request is sent), so this test
// records both: (1) the public cached result, and (2) a real wire request with a valid symbol handle sent through
// the same client connection, whose answer must identify the same symbol as the cached one. The client cache is
// not disabled; the request goes through the pinned client's own apiRequest.

import { API } from "@typescript/typescript/unstable/async";
import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { createVirtualFileSystem } from "../testUtils.ts";
import { cwd, golden } from "./utils.ts";

const g = golden("async-export-symbol");
const SymbolFlagsValue = 111551;
const source = `export function exported() { return 1; }\nconst use = exported();\n`;

describe("parity: async getExportSymbolOfSymbol", () => {
    test("public cached result and real wire request agree", async () => {
        const api = new API({ cwd, fs: createVirtualFileSystem({ "/tsconfig.json": `{ "compilerOptions": { "strict": true } }`, "/src/index.ts": source }) });
        try {
            const project = (await api.createSnapshot({ openProject: "/tsconfig.json" })).getConfiguredProject("/tsconfig.json")!;
            const file = (await project.program.getSourceFile("/src/index.ts"))!;
            const local = (await project.checker.getSymbolsInScope((file as any).statements[1], SymbolFlagsValue)).find(s => s.name === "exported" && (s as any).exportSymbol)!;
            assert.ok(local, "module-local symbol with an export symbol");
            const viaPublic = await local.getExportSymbol();
            const raw = await (api as any).client.apiRequest("getExportSymbolOfSymbol", { symbol: local.reference });
            assert.ok(raw, "wire response");
            // Identity: the wire answer is the symbol the client had cached for local.exportSymbol.
            assert.equal(raw.reference.id, viaPublic.reference.id);
            assert.notEqual(raw.reference.id, local.reference.id);
            g.check("public", { name: viaPublic.name, flags: viaPublic.flags, distinctFromLocal: viaPublic !== local });
            g.check("wire", { name: raw.name, flags: raw.flags, checkFlags: raw.checkFlags, declarations: raw.declarations, referenceKind: raw.reference.kind });
        }
        finally {
            await api.close();
        }
    });
});
