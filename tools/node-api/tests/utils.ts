// Shared helpers for the parity tests. Copied by run-upstream.mjs to packages/typescript/test/parity/ next to the
// pinned upstream tests, so "../testUtils.ts" is upstream's own virtual filesystem.
//
// Goldens: every value passed to check() is compared with tools/node-api/tests/golden/<suite>.json, which is
// recorded from the Go oracle only (run-upstream.mjs --record refuses any other binary). A tsrs mismatch is a
// parity failure to report to the owning lane, never a reason to re-record.

import { API as AsyncAPI } from "@typescript/typescript/unstable/async";
import { API } from "@typescript/typescript/unstable/sync";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createVirtualFileSystem } from "../testUtils.ts";

export const cwd = fileURLToPath(new URL("../../../../", import.meta.url).toString());

function mapsAsObjects(_key: string, value: unknown) {
    if (value instanceof Map) return Object.fromEntries([...value].sort(([a], [b]) => String(a) < String(b) ? -1 : 1));
    if (value instanceof Set) return [...value];
    return value;
}

export function golden(suite: string) {
    const dir = process.env.NODE_API_GOLDEN_DIR;
    if (!dir) throw new Error("NODE_API_GOLDEN_DIR is not set (run through tools/node-api/run-upstream.mjs)");
    const record = process.env.NODE_API_RECORD === "1";
    const file = path.join(dir, `${suite}.json`);
    const data: Record<string, unknown> = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : {};
    return {
        check(name: string, value: unknown) {
            const normalized = value === undefined ? null : JSON.parse(JSON.stringify(value, mapsAsObjects));
            if (record) {
                data[name] = normalized;
                const sorted = Object.fromEntries(Object.entries(data).sort(([a], [b]) => a < b ? -1 : 1));
                fs.writeFileSync(file, JSON.stringify(sorted, null, 2) + "\n");
                return;
            }
            assert.ok(Object.hasOwn(data, name), `no oracle golden recorded for ${suite}:${name}`);
            assert.deepStrictEqual(normalized, data[name], `${suite}:${name} differs from the Go oracle`);
        },
    };
}

/** Virtual filesystem that also records every file the server writes through the callback host. */
export function recordingFS(files: Record<string, string>) {
    const vfs = createVirtualFileSystem(files);
    const writes: Record<string, string> = {};
    const writeFile = vfs.writeFile.bind(vfs);
    vfs.writeFile = (p: string, data: string) => {
        writes[p] = data;
        writeFile(p, data);
    };
    return { vfs, writes };
}

export function syncAPI(files: Record<string, string>) {
    const { vfs, writes } = recordingFS(files);
    const api = new API({ cwd, fs: vfs });
    return { api, vfs, writes };
}

export function asyncAPI(files: Record<string, string>) {
    const { vfs, writes } = recordingFS(files);
    const api = new AsyncAPI({ cwd, fs: vfs });
    return { api, vfs, writes };
}

/** Diagnostics minus nothing: the full wire shape, sorted deterministically by file then position. */
export function diags<T extends { fileName?: string; pos: number; code: number; }>(list: readonly T[]): T[] {
    return [...list].sort((a, b) => (a.fileName ?? "").localeCompare(b.fileName ?? "") || a.pos - b.pos || a.code - b.code);
}

/** Server-written files, keyed by path, sorted. */
export function sortedWrites(writes: Record<string, string>) {
    return Object.fromEntries(Object.entries(writes).sort(([a], [b]) => a < b ? -1 : 1));
}
