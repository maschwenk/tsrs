// Shared helpers for the parity tests. Copied by run-upstream.mjs to packages/typescript/test/parity/ next to the
// pinned upstream tests, so "../testUtils.ts" is upstream's own virtual filesystem.
//
// Goldens: every value passed to check() is compared with tools/node-api/tests/golden/<suite>.json, which is
// recorded from the Go oracle only (run-upstream.mjs --record refuses any other binary). A tsrs mismatch is a
// parity failure to report to the owning lane, never a reason to re-record.

import { API as AsyncAPI } from "@typescript/typescript/unstable/async";
import { API } from "@typescript/typescript/unstable/sync";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
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

/**
 * Like golden().check but for values whose exact text is not part of the parity contract (error wording).
 * Recorded from the oracle; a candidate mismatch is printed as a test diagnostic and summarized by the runner,
 * not failed. Callers must assert the contractual part (that it failed, and how) with check().
 */
export function softGolden(suite: string) {
    const strict = golden(`${suite}.soft`);
    return {
        check(t: { diagnostic(msg: string): void; }, name: string, value: unknown) {
            try {
                strict.check(name, value);
            }
            catch (e) {
                t.diagnostic(`soft-mismatch ${suite}:${name}: ${(e as Error).message.split("\n")[0]} got ${JSON.stringify(value)}`);
            }
        },
    };
}

/** Path of the real --api server binary under test (not the tracing wrapper). */
export function serverBinary(): string {
    const exe = process.env.NODE_API_BINARY;
    if (!exe) throw new Error("NODE_API_BINARY is not set (run through tools/node-api/run-upstream.mjs)");
    return exe;
}

/** Linux: deepest descendant of `pid` whose argv contains --api (the server even behind the tracing proxy). */
export function apiServerPid(pid: number): number | undefined {
    let found: number | undefined;
    const visit = (p: number) => {
        let children: string[] = [];
        try {
            children = fs.readdirSync(`/proc/${p}/task`).flatMap(task => fs.readFileSync(`/proc/${p}/task/${task}/children`, "utf8").trim().split(/\s+/).filter(Boolean));
        }
        catch {
            return;
        }
        for (const c of children.map(Number)) {
            try {
                const argv = fs.readFileSync(`/proc/${c}/cmdline`, "utf8").split("\0");
                if (argv.includes("--api") && !argv.some(a => a.endsWith("proxy.mjs"))) found = c;
            }
            catch {}
            visit(c);
        }
    };
    visit(pid);
    return found;
}

/** Linux: resident set size of a process in KiB, from /proc/<pid>/status. */
export function rssKiB(pid: number): number {
    const m = /VmRSS:\s+(\d+)\s+kB/.exec(fs.readFileSync(`/proc/${pid}/status`, "utf8"));
    if (!m) throw new Error(`no VmRSS for ${pid}`);
    return Number(m[1]);
}

/** Write a JSON evidence file next to the run's results (tools/node-api/.work/<label>/evidence/<name>.json). */
export function evidence(name: string, value: unknown) {
    const dir = process.env.NODE_API_EVIDENCE_DIR;
    if (!dir) return;
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, `${name}.json`), JSON.stringify(value, null, 2) + "\n");
}

export const linuxOnly = process.platform === "linux" ? false : "needs /proc (Linux)";

export interface IsolatedResult {
    status: number | null;
    signal: string | null;
    timedOut: boolean;
    ms: number;
    result: any;
    stderr: string;
}

/**
 * Runs `body` (an async function body using `API`/`AsyncAPI`/`createVirtualFileSystem`, returning JSON) in a
 * separate Node process with a hard timeout, so a hung server or client cannot hang the suite. The outcome
 * (including a timeout) is returned for the caller to assert; a timeout is never treated as success.
 */
export function runIsolated(body: string, timeoutMs = 20_000): IsolatedResult {
    const here = path.dirname(fileURLToPath(import.meta.url));
    const file = path.join(here, `.isolated-${process.pid}-${Math.random().toString(36).slice(2)}.ts`);
    fs.writeFileSync(
        file,
        `import { API } from "@typescript/typescript/unstable/sync";
import { API as AsyncAPI } from "@typescript/typescript/unstable/async";
import { createVirtualFileSystem } from "../testUtils.ts";
const cwd = ${JSON.stringify(cwd)};
void API; void AsyncAPI; void createVirtualFileSystem; void cwd;
const result = await (async () => { ${body} })();
process.stdout.write("\\n@@RESULT@@" + JSON.stringify(result ?? null) + "\\n");
process.exit(0);
`,
    );
    const started = Date.now();
    try {
        const r = spawnSync(process.execPath, ["--conditions", "@typescript/source", file], { encoding: "utf8", timeout: timeoutMs, killSignal: "SIGKILL", env: process.env });
        const line = (r.stdout ?? "").split("\n").find(l => l.startsWith("@@RESULT@@"));
        return {
            status: r.status,
            signal: r.signal,
            timedOut: (r.error as any)?.code === "ETIMEDOUT",
            ms: Date.now() - started,
            result: line ? JSON.parse(line.slice("@@RESULT@@".length)) : undefined,
            stderr: (r.stderr ?? "").slice(0, 2000),
        };
    }
    finally {
        fs.rmSync(file, { force: true });
    }
}
